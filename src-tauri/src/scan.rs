//! Version 0.1 scan: counts games, badges and inventory items. Later versions
//! grow this into the full item catalog and completion math.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::paths::AppPaths;
use crate::state::AppState;
use crate::steam::{inventory, Notify, ProfileView};

pub const PRIVATE_PROFILE: &str = "Your Steam profile is private, so the app can't see your games, badges or inventory. In Steam's privacy settings, set My profile, Game details and Inventory to Public.";
pub const PRIVATE_GAMES: &str = "Your game details are private, so owned games can't be counted. In Steam's privacy settings, set Game details to Public.";
const PRIVATE_INVENTORY: &str = "Your inventory is private, so items can't be counted. In Steam's privacy settings, set Inventory to Public.";
const RATE_LIMITED: &str = "Steam is limiting requests from your connection right now, so the inventory count may be incomplete. Scan again in a few minutes.";

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
    state: &AppState,
    notify: Notify<'_>,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
) -> Result<ScanSummary, String> {
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
    let mut tally = inventory::Tally::default();
    let mut inventory_visible = true;
    let mut start: Option<String> = None;
    let mut page = 0u32;
    loop {
        page += 1;
        let f = state
            .steam
            .inventory_page(&steam_id, start.as_deref(), notify)
            .await?;
        if f.status == 401 || f.status == 403 {
            inventory_visible = false;
            if profile.is_public {
                warnings.push(PRIVATE_INVENTORY.to_string());
            }
            break;
        }
        if f.status == 429 {
            warnings.push(RATE_LIMITED.to_string());
            break;
        }
        if !f.is_success() {
            warnings.push(format!(
                "Steam couldn't return your inventory (code {}). Scan again later.",
                f.status
            ));
            break;
        }
        let Some(v) = f.json().filter(|v| !v.is_null()) else {
            inventory_visible = false;
            if profile.is_public {
                warnings.push(PRIVATE_INVENTORY.to_string());
            }
            break;
        };
        let info = inventory::tally_page(&v, &mut tally);
        if let Some(total_items) = info.total {
            let pages = total_items.div_ceil(2000).max(1);
            if pages > 1 {
                step(
                    4,
                    &format!("Reading your inventory (page {page} of {pages})…"),
                );
            }
        }
        if !info.more || info.last_assetid.is_none() || page >= MAX_INVENTORY_PAGES {
            break;
        }
        start = info.last_assetid;
    }

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
        inventory_visible,
        inventory_items: tally.items,
        foil_cards: tally.foil_cards,
        by_type,
        apps_with_items: item_apps.len() as u64,
        apps_not_owned,
        warnings,
    };
    save_last(&state.paths, &summary);
    Ok(summary)
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
