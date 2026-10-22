//! The collection scan. It runs in the background in four stages:
//!
//! 1. **Library:** profile, owned games, Steam Family library, badges,
//!    inventory, emoticons and profile items. A handful of requests.
//! 2. **Store check:** which games have trading cards, and names for games the
//!    user doesn't own. Web API, 100 games per request.
//! 3. **Points Shop:** every item each game sells for points. Web API, batched.
//! 4. **Item catalog:** each card game's full item list, from Steam's item
//!    definitions (Web API, fast) or, when those aren't available, the game's
//!    card page (Community, slow).
//!
//! Everything is saved as it arrives, so a scan can be paused, or the app
//! closed, and picked up later. Rescans only fetch what's missing or out of
//! date. When Steam refuses card page requests, the scan stops and resumes by
//! itself once the block has had time to clear.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::catalog::{cardpage, class, defs, library, points, store, CatalogItem};
use crate::db::{self, CatalogCounts, CatalogTarget, LibraryUpdate};
use crate::paths::AppPaths;
use crate::session::{self, Session};
use crate::state::AppState;
use crate::steam::{inventory, Auth, Bucket, Fetched, Notify, ProfileView, BLOCK_MINUTES};

pub const PRIVATE_PROFILE: &str = "Your Steam profile is private, so the app can't see your games, badges or inventory. In Steam's privacy settings, set My profile, Game details and Inventory to Public.";
pub const PRIVATE_GAMES: &str = "Your game details are private, so owned games can't be counted. In Steam's privacy settings, set Game details to Public.";
const PRIVATE_INVENTORY: &str = "Your inventory is private, so items can't be counted. Set Inventory to Public in Steam's privacy settings, or sign in with Steam.";
const BUSY_TEST: &str =
    "The connection test is running. Start the scan again when it has finished.";

const DAY: i64 = 24 * 60 * 60;
/// How long each kind of data is trusted before a rescan fetches it again.
const STORE_STALE_DAYS: i64 = 30;
const POINTS_STALE_DAYS: i64 = 7;
const CATALOG_STALE_DAYS: i64 = 30;
/// A resumed scan skips the library stage if it ran this recently.
const LIBRARY_FRESH_HOURS: i64 = 6;
/// A first catalog expected to take longer than this waits for a go-ahead.
const CONFIRM_SECONDS: f64 = 10.0 * 60.0;
/// Typical time per game for each route, until real timings take over.
const FAST_SECONDS: f64 = 0.5;
const SLOW_SECONDS: f64 = 4.2;
/// After this many games where the item definitions came back without cards
/// for a game that has them, the scan switches to card pages.
const EMPTY_DEFS_LIMIT: u32 = 5;
const MAX_POINTS_PAGES: usize = 40;
const MAX_INVENTORY_PAGES: u32 = 200;

const META_LIBRARY_READ: &str = "library_read_at";
const META_ELIGIBLE_READ: &str = "eligible_apps_read_at";
const META_ROUTE: &str = "catalog_route";

// ---- The overview summary ---------------------------------------------------

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
    /// Games shared through the user's Steam Family, when signed in.
    #[serde(default)]
    pub family_games: Option<u64>,
    /// The item catalog, once a scan has built one.
    #[serde(default)]
    pub catalog: Option<CatalogCounts>,
}

// ---- Scan status --------------------------------------------------------------

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScanState {
    #[default]
    Idle,
    Running,
    Paused,
    /// Held back after Steam refused requests; resumes at `resume_at`.
    Waiting,
    Done,
    Failed,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ScanStatus {
    pub state: ScanState,
    /// 1 to 4 while running.
    pub stage: u8,
    /// What's happening, in plain words.
    pub label: String,
    pub done: u64,
    pub total: u64,
    pub seconds_left: Option<u64>,
    /// Games whose item catalog hasn't been read yet.
    pub remaining: u64,
    /// Rough time to read those.
    pub remaining_seconds: Option<u64>,
    /// The catalog is coming from card pages, the slow route.
    pub slow_route: bool,
    /// When a held-back scan carries on by itself (Unix seconds).
    pub resume_at: Option<i64>,
    /// How the last run ended, or why it stopped.
    pub message: Option<String>,
    pub pause_requested: bool,
}

pub struct ScanControl {
    status: Mutex<ScanStatus>,
    pause: AtomicBool,
    running: AtomicBool,
}

impl Default for ScanControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanControl {
    pub fn new() -> Self {
        Self {
            status: Mutex::new(ScanStatus::default()),
            pause: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> ScanStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Changes the status and tells the interface.
    fn set(&self, app: &AppHandle, change: impl FnOnce(&mut ScanStatus)) {
        let copy = {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut s);
            s.clone()
        };
        let _ = app.emit("scan-status", copy);
    }
}

/// The current status, with the catalog backlog refreshed while idle.
pub fn status(app: &AppHandle) -> ScanStatus {
    let state = app.state::<AppState>();
    if !state.scan.is_running() {
        let (remaining, seconds, slow) = backlog(&state);
        let mut s = state.scan.status.lock().unwrap_or_else(|e| e.into_inner());
        s.remaining = remaining;
        s.remaining_seconds = seconds;
        s.slow_route = slow;
        if s.message.is_none() {
            if let Some(problem) = &state.db_problem {
                s.message = Some(format!(
                    "{problem} Scans still work, but their results are kept only until the app closes."
                ));
            }
        }
    }
    state.scan.status()
}

/// Starts a scan, or carries on a paused one when `resume` is set.
pub fn start(app: &AppHandle, resume: bool) -> ScanStatus {
    let state = app.state::<AppState>();
    if state.scan.running.swap(true, Ordering::SeqCst) {
        return state.scan.status();
    }
    let was = state.scan.status().state;
    let resume = resume || matches!(was, ScanState::Paused | ScanState::Waiting);
    state.scan.pause.store(false, Ordering::SeqCst);
    state.scan.set(app, |s| {
        *s = ScanStatus {
            state: ScanState::Running,
            label: "Starting…".to_string(),
            remaining: s.remaining,
            remaining_seconds: s.remaining_seconds,
            slow_route: s.slow_route,
            ..ScanStatus::default()
        };
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = run(&handle, resume).await;
        finish(&handle, outcome);
    });
    state.scan.status()
}

/// Asks a running scan to stop after the current game, or cancels a scan
/// that's waiting to carry on.
pub fn pause(app: &AppHandle) -> ScanStatus {
    let state = app.state::<AppState>();
    if state.scan.is_running() {
        state.scan.pause.store(true, Ordering::SeqCst);
        state.scan.set(app, |s| {
            s.pause_requested = true;
            s.label = "Pausing…".to_string();
        });
    } else if state.scan.status().state == ScanState::Waiting {
        state.scan.set(app, |s| {
            s.state = ScanState::Paused;
            s.resume_at = None;
            s.message = Some("Scan paused. Continue it whenever you like.".to_string());
        });
    }
    state.scan.status()
}

enum RunEnd {
    Done {
        seconds: u64,
    },
    Paused,
    /// Stopped before a long first catalog, waiting for a go-ahead.
    NeedsConfirm,
    /// Steam refused card page requests; carry on at this time.
    Waiting(i64),
}

fn finish(app: &AppHandle, outcome: Result<RunEnd, String>) {
    let state = app.state::<AppState>();
    let (remaining, remaining_seconds, slow) = backlog(&state);
    let (next, message, resume_at) = match outcome {
        Ok(RunEnd::Done { seconds }) => (
            ScanState::Done,
            Some(format!("Scan complete. It took {}.", duration_text(seconds))),
            None,
        ),
        Ok(RunEnd::Paused) => (
            ScanState::Paused,
            Some("Scan paused. Everything so far is saved; continue whenever you like.".to_string()),
            None,
        ),
        Ok(RunEnd::NeedsConfirm) => (
            ScanState::Paused,
            Some(format!(
                "Your library is read. Cataloging every game's items takes about {}{}. You can keep using the app while it runs, and pause it anytime.",
                duration_text(remaining_seconds.unwrap_or(0)),
                if slow {
                    " because Steam's quick item list isn't available on your connection"
                } else {
                    ""
                }
            )),
            None,
        ),
        Ok(RunEnd::Waiting(at)) => (
            ScanState::Waiting,
            Some(format!(
                "Steam asked the app to slow down, so the scan is taking a {BLOCK_MINUTES}-minute break. It carries on by itself if the app stays open."
            )),
            Some(at),
        ),
        Err(e) => (ScanState::Failed, Some(e), None),
    };
    state.scan.set(app, |s| {
        s.state = next;
        s.stage = 0;
        s.label = String::new();
        s.done = 0;
        s.total = 0;
        s.seconds_left = None;
        s.pause_requested = false;
        s.remaining = remaining;
        s.remaining_seconds = remaining_seconds;
        s.slow_route = slow;
        s.message = message;
        s.resume_at = resume_at;
    });
    state.scan.running.store(false, Ordering::SeqCst);

    if let Some(at) = resume_at {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let wait = (at - db::now()).max(1) as u64;
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let state = handle.state::<AppState>();
            let s = state.scan.status();
            if s.state == ScanState::Waiting && s.resume_at == Some(at) {
                start(&handle, true);
            }
        });
    }
}

/// Games still waiting for their catalog, a time estimate, and whether the
/// slow route is in use.
fn backlog(state: &AppState) -> (u64, Option<u64>, bool) {
    let family = state.settings().include_family;
    let route = state
        .db
        .read(|c| db::get_meta(c, META_ROUTE))
        .ok()
        .flatten();
    let slow = route.as_deref() == Some(Route::CARD_PAGES);
    let targets = state
        .db
        .read(|c| db::catalog_targets(c, family, db::now() - CATALOG_STALE_DAYS * DAY))
        .unwrap_or_default();
    let remaining = targets.iter().filter(|t| t.wanted(!slow)).count() as u64;
    let per_game = if slow { SLOW_SECONDS } else { FAST_SECONDS };
    let seconds = (remaining > 0).then(|| (remaining as f64 * per_game).ceil() as u64);
    (remaining, seconds, slow)
}

// ---- The run itself -----------------------------------------------------------

struct Route;

impl Route {
    const CARD_PAGES: &'static str = "card_page";
}

struct Ctx<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    notify: Notify<'a>,
}

impl Ctx<'_> {
    fn stage(&self, stage: u8, label: &str, total: u64) {
        self.state.scan.set(self.app, |s| {
            s.stage = stage;
            s.label = label.to_string();
            s.done = 0;
            s.total = total;
            s.seconds_left = None;
        });
    }

    fn progress(&self, done: u64, seconds_left: Option<u64>) {
        self.state.scan.set(self.app, |s| {
            s.done = done;
            s.seconds_left = seconds_left;
        });
    }

    fn label(&self, label: &str) {
        self.state
            .scan
            .set(self.app, |s| s.label = label.to_string());
    }

    fn pause_requested(&self) -> bool {
        self.state.scan.pause.load(Ordering::SeqCst)
    }
}

async fn run(app: &AppHandle, resume: bool) -> Result<RunEnd, String> {
    let state = app.state::<AppState>();
    let state: &AppState = &state;
    let _busy = state.try_begin().ok_or_else(|| BUSY_TEST.to_string())?;
    let key = state.api_key()?;
    let settings = state.settings();
    let steam_id = settings
        .profile
        .as_ref()
        .map(|p| p.steam_id.clone())
        .ok_or_else(|| "Add your Steam profile in Settings first.".to_string())?;
    let family_on = settings.include_family;
    let started = Instant::now();
    let requests_before = state.steam.requests();
    let note = |message: String| {
        state.scan.set(app, |s| s.label = message);
    };
    let ctx = Ctx {
        app,
        state,
        notify: &note,
    };
    let scan_id = state.db.write(|tx| db::begin_scan(tx))?;
    let requests = || state.steam.requests().saturating_sub(requests_before);

    let session = match session::current(app).await {
        Ok(s) => s.filter(|s| s.steam_id == steam_id),
        Err(_) => None,
    };
    let token = session.as_ref().and_then(|s| s.access_token());

    // A resumed scan whose library was read recently goes straight to the catalog.
    let library_read_at = state
        .db
        .read(|c| db::get_meta(c, META_LIBRARY_READ))?
        .and_then(|v| v.parse::<i64>().ok());
    let library_fresh = library_read_at.is_some_and(|t| db::now() - t < LIBRARY_FRESH_HOURS * 3600);
    let mut summary = if resume && library_fresh {
        load_summary(state)
    } else {
        None
    };

    if summary.is_none() {
        let result = read_library(&ctx, &key, &steam_id, session.as_ref(), token.as_deref()).await;
        let mut s = match result {
            Ok(s) => s,
            Err(e) => {
                let _ = state
                    .db
                    .write(|tx| db::end_scan(tx, scan_id, "failed", requests(), None));
                return Err(e);
            }
        };
        s.requests = requests();
        s.duration_ms = started.elapsed().as_millis() as u64;
        s.catalog = state.db.read(|c| db::catalog_counts(c, family_on)).ok();
        save_summary(state, scan_id, &s);
        let _ = app.emit("scan-summary", s.clone());
        summary = Some(s);
    }

    let outcome = catalog_stages(&ctx, &key, &steam_id, token.as_deref(), family_on, resume).await;

    // Whatever happened, keep the overview's catalog figures current.
    let status = match &outcome {
        Ok(RunEnd::Done { .. }) => "done",
        Ok(RunEnd::Paused | RunEnd::NeedsConfirm) => "paused",
        Ok(RunEnd::Waiting(_)) => "waiting",
        Err(_) => "failed",
    };
    let mut summary = summary.unwrap_or_else(|| empty_summary(state));
    summary.catalog = state.db.read(|c| db::catalog_counts(c, family_on)).ok();
    summary.requests = summary.requests.saturating_add(requests());
    let text = serde_json::to_string(&summary).ok();
    let _ = state
        .db
        .write(|tx| db::end_scan(tx, scan_id, status, requests(), text.as_deref()));
    let _ = app.emit("scan-summary", summary);

    match outcome {
        Ok(RunEnd::Done { .. }) => Ok(RunEnd::Done {
            seconds: started.elapsed().as_secs(),
        }),
        other => other,
    }
}

/// Stage 1: everything about the user's own account.
async fn read_library(
    ctx: &Ctx<'_>,
    key: &str,
    steam_id: &str,
    session: Option<&Session>,
    token: Option<&str>,
) -> Result<ScanSummary, String> {
    let state = ctx.state;
    let notify = ctx.notify;
    let mut warnings: Vec<String> = Vec::new();

    ctx.stage(1, "Reading your profile…", 0);
    let profile = state.steam.player_summary(key, steam_id, notify).await?;
    let saved = profile.clone();
    state.update_settings(move |s| {
        s.profile = Some(saved);
        Ok(())
    })?;
    if !profile.is_public {
        warnings.push(PRIVATE_PROFILE.to_string());
    }

    ctx.label("Reading your games…");
    let games = state.steam.owned_games(key, steam_id, notify).await?;
    if profile.is_public && !games.visible {
        warnings.push(PRIVATE_GAMES.to_string());
    }

    ctx.label("Reading your badges…");
    let badges = state.steam.badges(key, steam_id, notify).await?;

    ctx.label("Reading your inventory…");
    let on_page = |page: u32, pages: u64| {
        ctx.label(&format!("Reading your inventory (page {page} of {pages})…"));
    };
    let outcome = read_inventory(state, steam_id, session, notify, &on_page).await?;
    warnings.extend(outcome.warning.clone());
    if session.is_some() && token.is_none() {
        warnings.push("Your Steam sign-in didn't include a Web API token, so Steam Family games, emoticons and profile items were skipped. Sign out and in again from Settings.".to_string());
    }

    let mut family: Option<Vec<library::FamilyApp>> = None;
    let mut emoticons: Option<Vec<library::OwnedProfileItem>> = None;
    let mut profile_items: Option<Vec<library::OwnedProfileItem>> = None;
    if let Some(token) = token {
        ctx.label("Reading your Steam Family library…");
        let group = state.steam.family_group(token, steam_id, notify).await?;
        match group.json().as_ref().and_then(library::parse_family_group) {
            Some(Some(group_id)) => {
                let f = state
                    .steam
                    .family_library(token, &group_id, steam_id, notify)
                    .await?;
                family = f.json().as_ref().and_then(library::parse_family_library);
            }
            Some(None) => family = Some(Vec::new()),
            None => {}
        }

        ctx.label("Reading your emoticons and profile items…");
        let e = state.steam.emoticons(token, notify).await?;
        emoticons = e.json().as_ref().and_then(library::parse_emoticons);
        let p = state
            .steam
            .profile_items_owned(token, steam_id, notify)
            .await?;
        profile_items = p.json().as_ref().and_then(library::parse_profile_items);
    }

    // Games the user has items or badges for, owned or not.
    let tally = &outcome.tally;
    let mut item_apps: BTreeMap<u64, Option<String>> = BTreeMap::new();
    for app in &tally.apps {
        if *app as i64 != db::STEAM_APP {
            item_apps.insert(*app, tally.app_names.get(app).cloned());
        }
    }
    for b in &badges.list {
        if let Some(app) = b.appid {
            item_apps.entry(app).or_insert(None);
        }
    }
    let reset_items = outcome.visible && badges.visible;
    state.db.write(|tx| {
        db::save_library(
            tx,
            &LibraryUpdate {
                owned: games.visible.then_some(&games.games[..]),
                family: family.as_deref(),
                item_apps: &item_apps,
                reset_items,
            },
        )?;
        if outcome.visible {
            db::replace_inventory(tx, tally.rows.values())?;
        }
        if badges.visible {
            db::replace_badges(tx, &badges.list)?;
        }
        if let Some(e) = &emoticons {
            db::replace_profile_items(tx, "emoticons", e)?;
        }
        if let Some(p) = &profile_items {
            db::replace_profile_items(tx, "profile_items", p)?;
        }
        db::set_meta(tx, META_LIBRARY_READ, &db::now().to_string())
    })?;

    let owned: std::collections::HashSet<u64> = games.games.iter().map(|g| g.appid).collect();
    let apps_not_owned = if games.visible {
        item_apps.keys().filter(|a| !owned.contains(a)).count() as u64
    } else {
        0
    };
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

    Ok(ScanSummary {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: 0,
        requests: 0,
        profile,
        games_visible: games.visible,
        games_owned: games.games.len() as u64,
        game_badges: badges
            .list
            .iter()
            .filter(|b| b.appid.is_some() && !b.foil)
            .count() as u64,
        foil_badges: badges
            .list
            .iter()
            .filter(|b| b.appid.is_some() && b.foil)
            .count() as u64,
        other_badges: badges.list.iter().filter(|b| b.appid.is_none()).count() as u64,
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
        family_games: family.as_ref().map(|f| f.len() as u64),
        catalog: None,
    })
}

/// Stages 2 to 4: the store check, the Points Shop and the item catalog.
async fn catalog_stages(
    ctx: &Ctx<'_>,
    key: &str,
    steam_id: &str,
    token: Option<&str>,
    family_on: bool,
    resume: bool,
) -> Result<RunEnd, String> {
    let state = ctx.state;
    let notify = ctx.notify;
    let data = &state.db;

    // Stage 2: which games have trading cards.
    let pending =
        data.read(|c| db::apps_needing_store(c, family_on, db::now() - STORE_STALE_DAYS * DAY))?;
    ctx.stage(
        2,
        "Checking which games have trading cards…",
        pending.len() as u64,
    );
    let mut use_key = false;
    let mut done = 0u64;
    for batch in pending.chunks(store::APPS_PER_REQUEST) {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        let mut f = state
            .steam
            .store_items(batch, use_key.then_some(key), notify)
            .await?;
        if (f.status == 401 || f.status == 403) && !use_key {
            use_key = true;
            f = state.steam.store_items(batch, Some(key), notify).await?;
        }
        match f.json().as_ref().and_then(store::parse) {
            Some(apps) => data.write(|tx| db::save_store(tx, batch, &apps))?,
            // The check is a shortcut, not a requirement: without it, the
            // catalog stage checks each game itself.
            None => break,
        }
        done += batch.len() as u64;
        ctx.progress(done, None);
    }

    // Stage 3: the Points Shop, and which apps are sale events.
    let eligible_read = db
        .read(|c| db::get_meta(c, META_ELIGIBLE_READ))?
        .and_then(|v| v.parse::<i64>().ok());
    if eligible_read.is_none_or(|t| db::now() - t > POINTS_STALE_DAYS * DAY) {
        ctx.stage(3, "Reading the Points Shop…", 0);
        let f = state.steam.eligible_apps(notify).await?;
        if let Some(apps) = f.json().as_ref().and_then(points::parse_eligible_apps) {
            data.write(|tx| {
                db::save_event_apps(tx, &apps)?;
                db::set_meta(tx, META_ELIGIBLE_READ, &db::now().to_string())
            })?;
        }
    }
    let pending =
        data.read(|c| db::apps_needing_points(c, family_on, db::now() - POINTS_STALE_DAYS * DAY))?;
    ctx.stage(3, "Reading the Points Shop…", pending.len() as u64);
    let mut done = 0u64;
    'batches: for batch in pending.chunks(points::APPS_PER_REQUEST) {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_POINTS_PAGES {
            let f = state
                .steam
                .reward_items(batch, cursor.as_deref(), notify)
                .await?;
            let Some(page) = f.json().as_ref().and_then(points::parse_page) else {
                break 'batches;
            };
            let last = page.definitions < points::PAGE_SIZE as usize
                || page.next_cursor.is_none()
                || page.next_cursor == cursor;
            items.extend(page.items.into_iter().filter(|i| batch.contains(&i.appid)));
            if last {
                break;
            }
            cursor = page.next_cursor;
        }
        data.write(|tx| db::save_points(tx, batch, &items))?;
        done += batch.len() as u64;
        ctx.progress(done, None);
    }

    // Stage 4: each game's item catalog.
    let mut slow =
        data.read(|c| db::get_meta(c, META_ROUTE))?.as_deref() == Some(Route::CARD_PAGES);
    let targets: Vec<CatalogTarget> = db
        .read(|c| db::catalog_targets(c, family_on, db::now() - CATALOG_STALE_DAYS * DAY))?
        .into_iter()
        .filter(|t| t.wanted(!slow))
        .collect();
    let first_catalog = db
        .read(|c| db::catalog_counts(c, family_on))?
        .games_cataloged
        == 0;
    let estimate =
        |left: usize, slow: bool| left as f64 * if slow { SLOW_SECONDS } else { FAST_SECONDS };
    if !resume && first_catalog && estimate(targets.len(), slow) > CONFIRM_SECONDS {
        return Ok(RunEnd::NeedsConfirm);
    }

    ctx.stage(4, "Cataloging every game's items…", targets.len() as u64);
    ctx.state.scan.set(ctx.app, |s| s.slow_route = slow);
    let mut auths: Vec<Auth> = vec![Auth::None, Auth::Key(key)];
    if let Some(t) = token {
        auths.push(Auth::Token(t));
    }
    let mut auth_index = 0usize;
    let mut empty_defs = 0u32;
    let mut timed = 0u32;
    let mut timed_secs = 0f64;

    for (i, target) in targets.iter().enumerate() {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        let game_started = Instant::now();
        let has_cards = target.has_cards == Some(true) || target.evidence;
        let mut found: Option<(Vec<CatalogItem>, &str)> = None;
        let mut extra: Vec<CatalogItem> = Vec::new();

        if !slow {
            match read_defs(state, target.appid, &auths, &mut auth_index, notify).await? {
                DefsResult::Items(items) => {
                    if defs::card_count(&items) > 0 || !has_cards {
                        empty_defs = 0;
                        found = Some((items, "defs"));
                    } else {
                        // The definitions left out the cards this game has.
                        empty_defs += 1;
                        extra = items;
                    }
                }
                DefsResult::Skip(error) => {
                    data.write(|tx| db::save_catalog_error(tx, target.appid, &error))?;
                    ctx.progress(i as u64 + 1, None);
                    continue;
                }
                DefsResult::Unavailable => {
                    empty_defs = EMPTY_DEFS_LIMIT;
                }
            }
            if empty_defs >= EMPTY_DEFS_LIMIT {
                slow = true;
                data.write(|tx| db::set_meta(tx, META_ROUTE, Route::CARD_PAGES))?;
                ctx.state.scan.set(ctx.app, |s| s.slow_route = true);
                let left = targets.len() - i;
                if !resume && first_catalog && estimate(left, true) > CONFIRM_SECONDS {
                    return Ok(RunEnd::NeedsConfirm);
                }
            }
        }

        if found.is_none() && has_cards {
            if state.steam.is_blocked(Bucket::Community, false) {
                return Ok(RunEnd::Waiting(db::now() + BLOCK_MINUTES as i64 * 60));
            }
            let f = state
                .steam
                .card_page(steam_id, target.appid, notify)
                .await?;
            if f.status == 429 {
                return Ok(RunEnd::Waiting(db::now() + BLOCK_MINUTES as i64 * 60));
            }
            if !f.is_success() {
                let error = format!("The card page answered with code {}.", f.status);
                data.write(|tx| db::save_catalog_error(tx, target.appid, &error))?;
                ctx.progress(i as u64 + 1, None);
                continue;
            }
            let page = cardpage::parse(&f.body);
            let mut items = cardpage::to_items(&page);
            items.extend(extra.drain(..).filter(|x| x.class != class::CARD));
            found = Some((items, Route::CARD_PAGES));
        }

        if let Some((items, source)) = found {
            data.write(|tx| db::save_catalog(tx, target.appid, &items, source))?;
        }

        timed += 1;
        timed_secs += game_started.elapsed().as_secs_f64();
        let left = (targets.len() - i - 1) as f64;
        let per_game = if timed >= 3 {
            timed_secs / f64::from(timed)
        } else if slow {
            SLOW_SECONDS
        } else {
            FAST_SECONDS
        };
        ctx.progress(i as u64 + 1, Some((left * per_game).ceil() as u64));
    }

    Ok(RunEnd::Done { seconds: 0 })
}

enum DefsResult {
    Items(Vec<CatalogItem>),
    /// This game's answer was unusable; the next game may be fine.
    Skip(String),
    /// The item definitions can't be read on this connection at all.
    Unavailable,
}

/// Reads one game's item definitions, trying each way of identifying the app
/// until one is accepted. The one that works is kept for the following games.
async fn read_defs(
    state: &AppState,
    appid: u64,
    auths: &[Auth<'_>],
    auth_index: &mut usize,
    notify: Notify<'_>,
) -> Result<DefsResult, String> {
    while *auth_index < auths.len() {
        let f: Fetched = state
            .steam
            .item_definitions(appid, auths[*auth_index], notify)
            .await?;
        if f.status == 401 || f.status == 403 {
            *auth_index += 1;
            continue;
        }
        if f.status == 429 {
            return Ok(DefsResult::Skip(
                "Steam asked the app to slow down. This game is read again on the next scan."
                    .to_string(),
            ));
        }
        if f.status >= 500 {
            return Ok(DefsResult::Skip(format!(
                "Steam's item list answered with code {}.",
                f.status
            )));
        }
        if !f.is_success() {
            return Ok(DefsResult::Unavailable);
        }
        return Ok(match f.json().as_ref().and_then(defs::parse) {
            Some(items) => DefsResult::Items(items),
            None => DefsResult::Unavailable,
        });
    }
    Ok(DefsResult::Unavailable)
}

fn save_summary(state: &AppState, scan_id: i64, summary: &ScanSummary) {
    if let Ok(text) = serde_json::to_string(summary) {
        let _ = state
            .db
            .write(|tx| db::save_scan_summary(tx, scan_id, &text));
    }
}

fn load_summary(state: &AppState) -> Option<ScanSummary> {
    let text = state.db.read(db::last_summary).ok().flatten()?;
    serde_json::from_str(&text).ok()
}

/// The overview from the most recent scan.
pub fn last_summary(state: &AppState) -> Option<ScanSummary> {
    let mut summary = load_summary(state)?;
    // The family setting may have changed since; the counts follow it.
    summary.catalog = state
        .db
        .read(|c| db::catalog_counts(c, state.settings().include_family))
        .ok()
        .filter(|c| c.games_listed > 0)
        .or(summary.catalog);
    Some(summary)
}

fn empty_summary(state: &AppState) -> ScanSummary {
    let profile = state.settings().profile.unwrap_or(ProfileView {
        steam_id: String::new(),
        persona_name: "Steam user".to_string(),
        avatar_url: None,
        profile_url: None,
        is_public: true,
    });
    ScanSummary {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: 0,
        requests: 0,
        profile,
        games_visible: false,
        games_owned: 0,
        game_badges: 0,
        foil_badges: 0,
        other_badges: 0,
        player_level: None,
        inventory_visible: false,
        inventory_items: 0,
        foil_cards: 0,
        by_type: Vec::new(),
        apps_with_items: 0,
        apps_not_owned: 0,
        warnings: Vec::new(),
        needs_sign_in: false,
        inventory_route: None,
        family_games: None,
        catalog: None,
    }
}

/// Version 0.2.0 kept the last scan in `cache/last_scan.json`. Its figures
/// move into the database, and the file and its folder go away.
pub fn import_legacy_summary(paths: &AppPaths, state: &AppState) {
    let file = paths.root.join("cache").join("last_scan.json");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return;
    };
    let has_summary = state.db.read(db::last_summary).ok().flatten().is_some();
    if !has_summary {
        if let Ok(summary) = serde_json::from_str::<ScanSummary>(&text) {
            let at = chrono::DateTime::parse_from_rfc3339(&summary.scanned_at)
                .map(|t| t.timestamp())
                .unwrap_or_else(|_| db::now());
            if let Ok(json) = serde_json::to_string(&summary) {
                let _ = state.db.write(|tx| db::import_summary(tx, at, &json));
            }
        }
    }
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(paths.root.join("cache"));
}

fn duration_text(seconds: u64) -> String {
    let plural = |n: u64, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    if seconds < 60 {
        plural(seconds.max(1), "second")
    } else if seconds < 3600 {
        plural(seconds.div_ceil(60), "minute")
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600).div_ceil(60);
        if minutes == 0 {
            plural(hours, "hour")
        } else {
            format!("{} {}", plural(hours, "hour"), plural(minutes, "minute"))
        }
    }
}

// ---- Inventory ------------------------------------------------------------------

pub struct InventoryOutcome {
    pub tally: inventory::Tally,
    pub visible: bool,
    pub route: Option<&'static str>,
    pub warning: Option<String>,
    pub needs_sign_in: bool,
}

enum InvRoute<'a> {
    /// Steam's Web API with the signed-in user's token: generous limits.
    Api(&'a str),
    /// The community inventory, as the signed-in user.
    Signed(String),
    /// The community inventory, anonymously.
    Anonymous,
}

impl InvRoute<'_> {
    fn name(&self) -> &'static str {
        match self {
            InvRoute::Api(_) => "Steam Web API, signed in",
            InvRoute::Signed(_) => "Steam Community, signed in",
            InvRoute::Anonymous => "Steam Community, not signed in",
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
    let mut routes: Vec<InvRoute> = Vec::new();
    if let Some(t) = token.as_deref() {
        routes.push(InvRoute::Api(t));
    }
    match session {
        Some(s) => routes.push(InvRoute::Signed(s.community_header())),
        None => routes.push(InvRoute::Anonymous),
    }

    let mut errors: Vec<PageError> = Vec::new();
    for route in &routes {
        let api = matches!(route, InvRoute::Api(_));
        let mut tally = inventory::Tally::default();
        let mut start: Option<String> = None;
        let mut page = 0u32;
        let result: Result<(), PageError> = loop {
            page += 1;
            let f = match route {
                InvRoute::Api(t) => {
                    state
                        .steam
                        .inventory_api_page(t, steam_id, start.as_deref(), notify)
                        .await?
                }
                InvRoute::Signed(cookie) => {
                    state
                        .steam
                        .inventory_page(steam_id, start.as_deref(), Some(cookie.as_str()), notify)
                        .await?
                }
                InvRoute::Anonymous => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(0), "1 second");
        assert_eq!(duration_text(45), "45 seconds");
        assert_eq!(duration_text(61), "2 minutes");
        assert_eq!(duration_text(3600), "1 hour");
        assert_eq!(duration_text(3720), "1 hour 2 minutes");
    }

    #[test]
    fn old_summaries_still_load() {
        let old = r#"{"scanned_at":"2026-10-21T10:00:00Z","duration_ms":1000,"requests":4,
            "profile":{"steam_id":"1","persona_name":"Wolf","avatar_url":null,"profile_url":null,"is_public":true},
            "games_visible":true,"games_owned":3,"game_badges":1,"foil_badges":0,"other_badges":2,
            "player_level":10,"inventory_visible":false,"inventory_items":0,"foil_cards":0,"by_type":[],
            "apps_with_items":1,"apps_not_owned":0,"warnings":[]}"#;
        let s: ScanSummary = serde_json::from_str(old).unwrap();
        assert_eq!(s.games_owned, 3);
        assert!(s.catalog.is_none() && s.family_games.is_none());
    }
}
