//! The collection scan. It runs in the background in six stages:
//!
//! 1. **Library:** profile, owned games, Steam Family library, badges,
//!    inventory, achievements and profile items. A handful of requests.
//! 2. **Store check:** which games have trading cards, and names for games the
//!    user doesn't own. Web API, 100 games per request.
//! 3. **Points Shop:** every item each game sells for points. Web API, batched.
//! 4. **Item catalog:** each card game's card set, from the game's card page
//!    (Community pages, paced at one every 3.5 seconds).
//! 5. **Market prices** and 6. **order books and price history**, in
//!    `prices.rs`.
//!
//! Everything is saved as it arrives, so a scan can be paused, or the app
//! closed, and picked up later. Rescans only fetch what's missing or out of
//! date. When Steam refuses card page requests, the scan stops and resumes by
//! itself once the block has had time to clear.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::catalog::{cardpage, library, points, store};
use crate::completion;
use crate::db::{self, CatalogCounts, CatalogTarget, LibraryUpdate};
use crate::paths::AppPaths;
use crate::session::{self, Session};
use crate::state::AppState;
use crate::steam::{
    inventory, parse_achievements, Auth, Bucket, Notify, ProfileView, BLOCK_MINUTES,
};

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
/// Typical time per game, until real timings take over.
const SECONDS_PER_GAME: f64 = 4.2;
const MAX_POINTS_PAGES: usize = 40;
/// Games per achievement progress request.
const ACH_PER_REQUEST: usize = 100;
const MAX_INVENTORY_PAGES: u32 = 200;

const META_LIBRARY_READ: &str = "library_read_at";
const META_ELIGIBLE_READ: &str = "eligible_apps_read_at";

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
    /// When a held-back scan carries on by itself (Unix seconds).
    pub resume_at: Option<i64>,
    /// Items on the Market whose price history hasn't been read yet, once
    /// reading them has started, and a time estimate.
    pub prices_remaining: u64,
    pub prices_seconds: Option<u64>,
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
        let (remaining, seconds) = backlog(&state);
        let (prices, price_seconds) = price_backlog(&state);
        let mut s = state.scan.status.lock().unwrap_or_else(|e| e.into_inner());
        s.remaining = remaining;
        s.remaining_seconds = seconds;
        s.prices_remaining = prices;
        s.prices_seconds = price_seconds;
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
            prices_remaining: s.prices_remaining,
            prices_seconds: s.prices_seconds,
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
        let _ = state.db.write(|tx| db::pause_waiting_scan(tx));
        state.scan.set(app, |s| {
            s.state = ScanState::Paused;
            s.resume_at = None;
            s.message = Some("Scan paused. Continue it whenever you like.".to_string());
        });
    }
    state.scan.status()
}

pub(crate) enum RunEnd {
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
    let (remaining, remaining_seconds) = backlog(&state);
    let (prices_remaining, prices_seconds) = price_backlog(&state);
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
                "Your library is read. Cataloging every game's items takes about {}. You can keep using the app while it runs, and pause it anytime.",
                duration_text(remaining_seconds.unwrap_or(0))
            )),
            None,
        ),
        Ok(RunEnd::Waiting(at)) => (
            ScanState::Waiting,
            Some(format!(
                "Steam asked the app to slow down, so the scan is taking a break for {}. It carries on by itself if the app stays open.",
                duration_text((at - db::now()).max(60) as u64)
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
        s.prices_remaining = prices_remaining;
        s.prices_seconds = prices_seconds;
        s.message = message;
        s.resume_at = resume_at;
    });
    state.scan.running.store(false, Ordering::SeqCst);

    if let Some(at) = resume_at {
        schedule_resume(app, at);
    }
}

/// Carries on a waiting scan at `at`, unless it was paused or restarted by then.
fn schedule_resume(app: &AppHandle, at: i64) {
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

/// At startup, carries on a scan that was waiting for Steam when the app
/// closed, once the saved wait is over (or in a minute).
pub fn resume_after_restart(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state
        .db
        .read(db::last_scan_status)
        .ok()
        .flatten()
        .as_deref()
        != Some("waiting")
    {
        return;
    }
    let at = crate::prices::saved_wait_until(&state).max(db::now() + 60);
    state.scan.set(app, |s| {
        s.state = ScanState::Waiting;
        s.resume_at = Some(at);
        s.message = Some(format!(
            "Steam asked the app to slow down, so the scan is taking a break for {}. It carries on by itself if the app stays open.",
            duration_text((at - db::now()).max(60) as u64)
        ));
    });
    schedule_resume(app, at);
}

/// Games still waiting for their catalog, and a time estimate.
fn backlog(state: &AppState) -> (u64, Option<u64>) {
    let family = state.settings().include_family;
    let targets = state
        .db
        .read(|c| db::catalog_targets(c, family, db::now() - CATALOG_STALE_DAYS * DAY))
        .unwrap_or_default();
    let remaining = targets.iter().filter(|t| t.wanted()).count() as u64;
    let seconds = (remaining > 0).then(|| (remaining as f64 * SECONDS_PER_GAME).ceil() as u64);
    (remaining, seconds)
}

/// Price history is read when an item is opened, so scans never leave any
/// behind.
fn price_backlog(_state: &AppState) -> (u64, Option<u64>) {
    (0, None)
}

// ---- The run itself -----------------------------------------------------------

pub(crate) struct Ctx<'a> {
    pub(crate) app: &'a AppHandle,
    pub(crate) state: &'a AppState,
    pub(crate) notify: Notify<'a>,
}

impl Ctx<'_> {
    pub(crate) fn stage(&self, stage: u8, label: &str, total: u64) {
        self.state.scan.set(self.app, |s| {
            s.stage = stage;
            s.label = label.to_string();
            s.done = 0;
            s.total = total;
            s.seconds_left = None;
        });
    }

    pub(crate) fn progress(&self, done: u64, seconds_left: Option<u64>) {
        self.state.scan.set(self.app, |s| {
            s.done = done;
            s.seconds_left = seconds_left;
        });
    }

    pub(crate) fn label(&self, label: &str) {
        self.state
            .scan
            .set(self.app, |s| s.label = label.to_string());
    }

    pub(crate) fn pause_requested(&self) -> bool {
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

    let inventory_read = summary.is_none();
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

    let outcome = catalog_stages(&ctx, &key, &steam_id, family_on, resume).await;
    // Prices come last, once every game's item list is read.
    let outcome = match outcome {
        Ok(RunEnd::Done { .. }) => {
            let cookie = session.as_ref().map(|s| s.community_header());
            crate::prices::stages(&ctx, family_on, cookie.as_deref()).await
        }
        other => other,
    };

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
    // What changed since the last scan, and today's figures, for the dashboard.
    let _ = crate::history::record(state, inventory_read && summary.inventory_visible);
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
        warnings.push("Your Steam sign-in didn't include a Web API token, so Steam Family games and profile items were skipped. Sign out and in again from Settings.".to_string());
    }

    let mut family: Option<Vec<library::FamilyApp>> = None;
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

        ctx.label("Reading your profile items…");
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
            db::refresh_mismatched_catalogs(tx)?;
        }
        if badges.visible {
            db::replace_badges(tx, &badges.list)?;
        }
        if let Some(p) = &profile_items {
            db::replace_profile_items(tx, "profile_items", p)?;
        }
        db::set_meta(tx, META_LIBRARY_READ, &db::now().to_string())
    })?;

    // Games where the user owns an item no list includes have their Points
    // Shop read again, at most once a day, in case the item is new.
    let rules = completion::Rules {
        keep_card_set: false,
        ..completion::Rules::from_settings(&state.settings())
    };
    if let Ok(apps) = state
        .db
        .read(|c| completion::compute(c, rules, db::now()).map(|s| s.apps_with_unlisted()))
    {
        let _ = state
            .db
            .write(|tx| db::expire_points(tx, &apps, db::now() - DAY));
    }

    // Achievement progress for every owned game: a couple of quick requests.
    ctx.label("Reading your achievements…");
    let ach_apps = state
        .db
        .read(|c| db::apps_needing_achievements(c, db::now()))?;
    let mut auths: Vec<Auth> = vec![Auth::Key(key)];
    if let Some(t) = token {
        auths.push(Auth::Token(t));
    }
    let mut auth_index = 0usize;
    'achievements: for batch in ach_apps.chunks(ACH_PER_REQUEST) {
        loop {
            let Some(auth) = auths.get(auth_index).copied() else {
                break 'achievements;
            };
            let f = state
                .steam
                .achievements_progress(auth, steam_id, batch, notify)
                .await?;
            if f.status == 401 || f.status == 403 {
                auth_index += 1;
                continue;
            }
            match f.json().as_ref().and_then(parse_achievements) {
                Some(progress) => {
                    state
                        .db
                        .write(|tx| db::save_achievements(tx, batch, &progress))?;
                    break;
                }
                None => break 'achievements,
            }
        }
    }

    let owned: HashSet<u64> = games.games.iter().map(|g| g.appid).collect();
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
    let eligible_read = data
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
    for batch in pending.chunks(points::APPS_PER_REQUEST) {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        // Everything, including items that can't be bought on their own, then
        // only what can be bought, to tell the two apart.
        let Some((mut items, mut bundled)) = read_points(state, batch, true, notify).await? else {
            break;
        };
        let buyable = read_points(state, batch, false, notify).await?;
        if let Some((sold, more_bundled)) = &buyable {
            let sold: HashSet<i64> = sold.iter().map(|i| i.defid).collect();
            bundled.extend(more_bundled);
            for item in &mut items {
                item.buyable = Some(if sold.contains(&item.defid) {
                    1
                } else if bundled.contains(&item.defid) {
                    2
                } else {
                    0
                });
            }
        }
        data.write(|tx| db::save_points(tx, batch, &items))?;
        done += batch.len() as u64;
        ctx.progress(done, None);
    }

    // Stage 4: each card game's card set, from its card page.
    let targets: Vec<CatalogTarget> = data
        .read(|c| db::catalog_targets(c, family_on, db::now() - CATALOG_STALE_DAYS * DAY))?
        .into_iter()
        .filter(|t| t.wanted())
        .collect();
    let first_catalog = data
        .read(|c| db::catalog_counts(c, family_on))?
        .games_cataloged
        == 0;
    if !resume && first_catalog && targets.len() as f64 * SECONDS_PER_GAME > CONFIRM_SECONDS {
        return Ok(RunEnd::NeedsConfirm);
    }

    ctx.stage(4, "Cataloging every game's items…", targets.len() as u64);
    let mut timed = 0u32;
    let mut timed_secs = 0f64;
    for (i, target) in targets.iter().enumerate() {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        let game_started = Instant::now();
        if state.steam.is_blocked(Bucket::Community, false) {
            return Ok(community_waiting(state));
        }
        let f = state
            .steam
            .card_page(steam_id, target.appid, notify)
            .await?;
        if f.status == 429 {
            return Ok(community_waiting(state));
        }
        if f.is_success() {
            let page = cardpage::parse(&f.body);
            let mut items = cardpage::to_items(&page);
            let known = data.read(|c| db::owned_card_types(c, target.appid))?;
            cardpage::apply_known_types(&mut items, &known);
            data.write(|tx| db::save_catalog(tx, target.appid, &items, "card_page"))?;
        } else {
            let error = format!("The card page answered with code {}.", f.status);
            data.write(|tx| db::save_catalog_error(tx, target.appid, &error))?;
        }

        timed += 1;
        timed_secs += game_started.elapsed().as_secs_f64();
        let left = (targets.len() - i - 1) as f64;
        let per_game = if timed >= 3 {
            timed_secs / f64::from(timed)
        } else {
            SECONDS_PER_GAME
        };
        ctx.progress(i as u64 + 1, Some((left * per_game).ceil() as u64));
    }

    Ok(RunEnd::Done { seconds: 0 })
}

/// Every Points Shop item for a batch of games, and the items sold inside
/// bundles. `None` when Steam's answer wasn't a Points Shop answer.
async fn read_points(
    state: &AppState,
    batch: &[u64],
    include_unbuyable: bool,
    notify: Notify<'_>,
) -> Result<Option<(Vec<points::PointsItem>, HashSet<i64>)>, String> {
    let mut items = Vec::new();
    let mut bundled = HashSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_POINTS_PAGES {
        let f = state
            .steam
            .reward_items(batch, cursor.as_deref(), include_unbuyable, notify)
            .await?;
        let Some(page) = f.json().as_ref().and_then(points::parse_page) else {
            return Ok(None);
        };
        let last = page.definitions < points::PAGE_SIZE as usize
            || page.next_cursor.is_none()
            || page.next_cursor == cursor;
        bundled.extend(page.bundled.iter().copied());
        items.extend(page.items.into_iter().filter(|i| batch.contains(&i.appid)));
        if last {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(Some((items, bundled)))
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

/// Waits for as long as Steam's card pages are held back.
fn community_waiting(state: &AppState) -> RunEnd {
    let wait = state.steam.wait_seconds(Bucket::Community, false).max(60);
    RunEnd::Waiting(db::now() + wait as i64)
}

pub(crate) fn duration_text(seconds: u64) -> String {
    let plural = |n: u64, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    if seconds < 60 {
        return plural(seconds.max(1), "second");
    }
    // Round up to whole minutes first, so 1:59:59 reads "2 hours".
    let total = seconds.div_ceil(60);
    let (hours, minutes) = (total / 60, total % 60);
    match (hours, minutes) {
        (0, m) => plural(m, "minute"),
        (h, 0) => plural(h, "hour"),
        (h, m) => format!("{} {}", plural(h, "hour"), plural(m, "minute")),
    }
}

#[cfg(test)]
mod duration_tests {
    use super::duration_text;

    #[test]
    fn rounds_up_to_whole_minutes_before_splitting_hours() {
        assert_eq!(duration_text(30), "30 seconds");
        assert_eq!(duration_text(61), "2 minutes");
        assert_eq!(duration_text(3599), "1 hour");
        assert_eq!(duration_text(7199), "2 hours");
        assert_eq!(duration_text(7260), "2 hours 1 minute");
        assert_eq!(duration_text(5400), "1 hour 30 minutes");
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
                "Steam is limiting inventory requests from your connection right now, even signed in. The app will wait at least {BLOCK_MINUTES} minutes before asking again, and longer if Steam keeps refusing."
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
