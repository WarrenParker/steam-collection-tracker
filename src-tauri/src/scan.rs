//! The scan: counts games, badges and inventory items. Later versions grow this
//! into the full item catalog and completion math.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::paths::AppPaths;
use crate::session::{self, Session};
use crate::state::AppState;
use crate::steam::{inventory, Notify, ProfileView, BLOCK_MINUTES};

pub const PRIVATE_PROFILE: &str = "Your Steam profile is private, so the app can't see your games, badges or inventory. In Steam's privacy settings, set My profile, Game details and Inventory to Public.";
pub const PRIVATE_GAMES: &str = "Your game details are private, so owned games can't be counted. In Steam's privacy settings, set Game details to Public.";
const PRIVATE_INVENTORY: &str = "Your inventory is private, so items can't be counted. Set Inventory to Public in Steam's privacy settings, or sign in with Steam.";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TypeCount {
    pub label: String,
    pub count: u64,
    pub quantity: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScanSummary {
    pub scanned_at: String,
    pub duration_ms: u64,
    pub requests: u32,
    pub profile: ProfileView,
    pub games_visible: bool,
    pub games_owned: u64,
    pub game_badges: u64,
    pub foil_badges: u64,
    pub other_badges: u64,
    pub player_level: Option<u64>,
    pub inventory_visible: bool,
    pub inventory_items: u64,
    pub foil_cards: u64,
    pub by_type: Vec<TypeCount>,
    pub apps_with_items: u64,
    pub apps_not_owned: u64,
    pub warnings: Vec<String>,
    /// Steam refused the inventory because the app isn't signed in.
    #[serde(default)]
    pub needs_sign_in: bool,
    /// Which way the inventory was read, for the curious.
    #[serde(default)]
    pub inventory_route: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanProgress {
    pub step: u32,
    pub total: u32,
    pub message: String,
}

const STEAM_APP: u64 = 753;
const MAX_INVENTORY_PAGES: u32 = 200;

pub async fn run(
    app: &AppHandle,
    notify: Notify<'_>,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
) -> Result<ScanSummary, String> {
    let state = app.state::<AppState>();
    let key = state.api_key()?;
    let steam_id = state
        .settings()
        .profile
        .map(|p| p.steam_id)
        .ok_or_else(|| "Add your Steam profile in Settings first.".to_string())?;
    let started = Instant::now();
    let requests_before = state.steam.requests();
    let mut warnings: Vec<String> = Vec::new();
    let total = 4;
    let step = |n: u32, message: &str| {
        progress(ScanProgress {
            step: n,
            total,
            message: message.to_string(),
        })
    };

    step(1, "Checking your profile…");
    let profile = state.steam.player_summary(&key, &steam_id, notify).await?;
    let saved = profile.clone();
    state.update_settings(move |s| {
        s.profile = Some(saved);
        Ok(())
    })?;
    if !profile.is_public {
        warnings.push(PRIVATE_PROFILE.to_string());
    }

    step(2, "Counting your games…");
    let games = state.steam.owned_games(&key, &steam_id, notify).await?;
    if profile.is_public && !games.visible {
        warnings.push(PRIVATE_GAMES.to_string());
    }

    step(3, "Reading your badges…");
    let badges = state.steam.badges(&key, &steam_id, notify).await?;

    step(4, "Reading your inventory…");
    let session = match session::current(app).await {
        Ok(s) => s.filter(|s| s.steam_id == steam_id),
        Err(e) => {
            warnings.push(e);
            None
        }
    };
    let on_page = |page: u32, pages: u64| {
        step(
            4,
            &format!("Reading your inventory (page {page} of {pages})…"),
        );
    };
    let outcome = read_inventory(&state, &steam_id, session.as_ref(), notify, &on_page).await?;
    warnings.extend(outcome.warning.clone());
    let tally = outcome.tally;

    let owned: HashSet<u64> = games.games.iter().map(|g| g.appid).collect();
    let mut item_apps: BTreeSet<u64> = tally.apps.clone();
    item_apps.extend(badges.list.iter().filter_map(|b| b.appid));
    item_apps.remove(&STEAM_APP);
    let apps_not_owned = if games.visible {
        item_apps.iter().filter(|a| !owned.contains(a)).count() as u64
    } else {
        0
    };

    let game_badges = badges
        .list
        .iter()
        .filter(|b| b.appid.is_some() && !b.foil)
        .count() as u64;
    let foil_badges = badges
        .list
        .iter()
        .filter(|b| b.appid.is_some() && b.foil)
        .count() as u64;
    let other_badges = badges.list.iter().filter(|b| b.appid.is_none()).count() as u64;

    let mut by_type: Vec<TypeCount> = tally
        .by_type
        .iter()
        .map(|(label, (count, quantity))| TypeCount {
            label: label.clone(),
            count: *count,
            quantity: *quantity,
        })
        .collect();
    by_type.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));

    let summary = ScanSummary {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: started.elapsed().as_millis() as u64,
        requests: state.steam.requests().saturating_sub(requests_before),
        profile,
        games_visible: games.visible,
        games_owned: games.games.len() as u64,
        game_badges,
        foil_badges,
        other_badges,
        player_level: badges.player_level,
        inventory_visible: outcome.visible,
        inventory_items: tally.items,
        foil_cards: tally.foil_cards,
        by_type,
        apps_with_items: item_apps.len() as u64,
        apps_not_owned,
        warnings,
        needs_sign_in: outcome.needs_sign_in,
        inventory_route: outcome.route.map(str::to_string),
    };
    save_last(&state.paths, &summary);
    Ok(summary)
}

pub struct InventoryOutcome {
    pub tally: inventory::Tally,
    pub visible: bool,
    pub route: Option<&'static str>,
    pub warning: Option<String>,
    pub needs_sign_in: bool,
}

enum Route<'a> {
    /// Steam's Web API with the signed-in user's token: generous limits.
    Api(&'a str),
    /// The community inventory, as the signed-in user.
    Signed(String),
    /// The community inventory, anonymously.
    Anonymous,
}

impl Route<'_> {
    fn name(&self) -> &'static str {
        match self {
            Route::Api(_) => "Steam Web API, signed in",
            Route::Signed(_) => "Steam Community, signed in",
            Route::Anonymous => "Steam Community, not signed in",
        }
    }
}

#[derive(Debug)]
enum PageError {
    Refused,
    Private,
    Status(u16),
    Unreadable,
}

/// Reads the whole Steam inventory, trying the signed-in routes first.
pub async fn read_inventory(
    state: &AppState,
    steam_id: &str,
    session: Option<&Session>,
    notify: Notify<'_>,
    on_page: &(dyn Fn(u32, u64) + Send + Sync),
) -> Result<InventoryOutcome, String> {
    let token = session.and_then(|s| s.access_token());
    let mut routes: Vec<Route> = Vec::new();
    if let Some(t) = token.as_deref() {
        routes.push(Route::Api(t));
    }
    match session {
        Some(s) => routes.push(Route::Signed(s.community_header())),
        None => routes.push(Route::Anonymous),
    }

    let mut errors: Vec<PageError> = Vec::new();
    for route in &routes {
        let api = matches!(route, Route::Api(_));
        let mut tally = inventory::Tally::default();
        let mut start: Option<String> = None;
        let mut page = 0u32;
        let result: Result<(), PageError> = loop {
            page += 1;
            let f = match route {
                Route::Api(t) => {
                    state
                        .steam
                        .inventory_api_page(t, steam_id, start.as_deref(), notify)
                        .await?
                }
                Route::Signed(cookie) => {
                    state
                        .steam
                        .inventory_page(steam_id, start.as_deref(), Some(cookie.as_str()), notify)
                        .await?
                }
                Route::Anonymous => {
                    state
                        .steam
                        .inventory_page(steam_id, start.as_deref(), None, notify)
                        .await?
                }
            };
            if f.status == 429 {
                break Err(PageError::Refused);
            }
            if f.status == 401 || f.status == 403 {
                break Err(if api {
                    PageError::Status(f.status)
                } else {
                    PageError::Private
                });
            }
            if !f.is_success() {
                break Err(PageError::Status(f.status));
            }
            let Some(v) = f.json() else {
                break Err(PageError::Unreadable);
            };
            let v = if api {
                v.get("response").cloned().unwrap_or(Value::Null)
            } else {
                v
            };
            if v.is_null() {
                break Err(PageError::Private);
            }
            if v.get("assets").is_none() && v.get("total_inventory_count").is_none() {
                break Err(PageError::Unreadable);
            }
            let info = inventory::tally_page(&v, &mut tally);
            if let Some(total) = info.total {
                let pages = total.div_ceil(2000).max(1);
                if pages > 1 {
                    on_page(page, pages);
                }
            }
            if !info.more || info.last_assetid.is_none() || page >= MAX_INVENTORY_PAGES {
                break Ok(());
            }
            start = info.last_assetid;
        };
        match result {
            Ok(()) => {
                return Ok(InventoryOutcome {
                    tally,
                    visible: true,
                    route: Some(route.name()),
                    warning: None,
                    needs_sign_in: false,
                })
            }
            Err(e) => errors.push(e),
        }
    }

    // Every route failed: explain the most useful reason.
    let (warning, needs_sign_in) = if session.is_none() {
        match errors.last() {
            Some(PageError::Refused) => (None, true),
            Some(PageError::Private) => (Some(PRIVATE_INVENTORY.to_string()), false),
            Some(PageError::Status(code)) => (
                Some(format!(
                    "Steam couldn't return your inventory (code {code}). Scan again later."
                )),
                false,
            ),
            _ => (
                Some("Steam's inventory answer couldn't be read. Scan again later.".to_string()),
                false,
            ),
        }
    } else if errors.iter().all(|e| matches!(e, PageError::Refused)) {
        (
            Some(format!(
                "Steam is limiting inventory requests from your connection right now, even signed in. The app will wait {BLOCK_MINUTES} minutes before asking again."
            )),
            false,
        )
    } else {
        let codes: Vec<String> = errors
            .iter()
            .map(|e| match e {
                PageError::Refused => "refused".to_string(),
                PageError::Private => "private".to_string(),
                PageError::Status(c) => format!("code {c}"),
                PageError::Unreadable => "unreadable".to_string(),
            })
            .collect();
        (
            Some(format!(
                "Steam wouldn't return your inventory ({}). Scan again later, and run the connection test if it keeps happening.",
                codes.join(", ")
            )),
            false,
        )
    };
    Ok(InventoryOutcome {
        tally: inventory::Tally::default(),
        visible: false,
        route: None,
        warning,
        needs_sign_in,
    })
}

fn last_scan_file(paths: &AppPaths) -> std::path::PathBuf {
    paths.cache_dir().join("last_scan.json")
}

fn save_last(paths: &AppPaths, summary: &ScanSummary) {
    if let Ok(text) = serde_json::to_string_pretty(summary) {
        let _ = fs::write(last_scan_file(paths), text);
    }
}

pub fn load_last(paths: &AppPaths) -> Option<ScanSummary> {
    let text = fs::read_to_string(last_scan_file(paths)).ok()?;
    serde_json::from_str(&text).ok()
}
