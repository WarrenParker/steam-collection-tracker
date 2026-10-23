//! Everything the interface can ask the engine to do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::completion::{self, AccountCompletion, Availability, GameCompletion, ItemView};
use crate::db::{self, GameRow};
use crate::diagnostics::{self, DiagStep, DiagnosticsResult};
use crate::housekeeping;
use crate::prices::{self, PriceHistory};
use crate::profile_input::{self, ProfileRef};
use crate::scan::{self, ScanStatus, ScanSummary, PRIVATE_GAMES, PRIVATE_PROFILE};
use crate::session;
use crate::settings::{PriceWindow, SettingsView, Theme};
use crate::state::AppState;
use crate::steam::ProfileView;
use crate::updater::{self, UpdateState, VersionList};

const BUSY: &str = "The app is already talking to Steam. Wait for that to finish, then try again.";
const SCANNING: &str = "A scan is running. Pause it first, then run the test.";

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    build: Option<String>,
    data_dir: String,
    data_size_bytes: u64,
    portable: bool,
    updates_enabled: bool,
}

#[derive(Deserialize)]
pub struct PreferencesPatch {
    theme: Option<Theme>,
    auto_update: Option<bool>,
    include_family: Option<bool>,
    keep_card_set: Option<bool>,
}

#[derive(Serialize)]
pub struct ProfileCheck {
    profile: ProfileView,
    games_visible: bool,
    warnings: Vec<String>,
}

/// The interface calls this once it has drawn, so the window never flashes white.
#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// The interface reports WebView2's browser identity, used for Steam's web pages.
#[tauri::command]
pub fn report_browser_agent(state: State<'_, AppState>, agent: String) {
    state.steam.set_browser_agent(&agent);
}

#[tauri::command]
pub async fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: updater::build_id(),
        data_dir: state.paths.root.display().to_string(),
        data_size_bytes: housekeeping::folder_size(&state.paths.root),
        portable: state.paths.portable,
        updates_enabled: updater::enabled(),
    })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    state.settings().view()
}

#[tauri::command]
pub fn update_preferences(
    window: WebviewWindow,
    state: State<'_, AppState>,
    patch: PreferencesPatch,
) -> Result<SettingsView, String> {
    let updated = state.update_settings(|s| {
        if let Some(theme) = patch.theme {
            s.theme = theme;
        }
        if let Some(auto) = patch.auto_update {
            s.auto_update = auto;
        }
        if let Some(family) = patch.include_family {
            s.include_family = family;
        }
        if let Some(keep) = patch.keep_card_set {
            s.keep_card_set = keep;
        }
        Ok(())
    })?;
    let _ = window.set_theme(Some(updated.theme.window_theme()));
    Ok(updated.view())
}

#[tauri::command]
pub async fn save_api_key(state: State<'_, AppState>, key: String) -> Result<SettingsView, String> {
    let key: String = key
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    if key.len() != 32 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "That doesn't look like a Steam API key. A key is 32 letters and numbers, like 1A2B3C4D…"
                .to_string(),
        );
    }
    let quiet = |_: String| {};
    state.steam.check_api_key(&key, &quiet).await?;
    let updated = state.update_settings(|s| s.set_api_key(&key))?;
    Ok(updated.view())
}

#[tauri::command]
pub async fn set_profile(
    state: State<'_, AppState>,
    input: String,
) -> Result<ProfileCheck, String> {
    let key = state.api_key()?;
    let quiet = |_: String| {};
    let steam_id = match profile_input::parse(&input)? {
        ProfileRef::SteamId(id) => id,
        ProfileRef::Vanity(name) => state.steam.resolve_vanity(&key, &name, &quiet).await?,
    };
    let profile = state.steam.player_summary(&key, &steam_id, &quiet).await?;
    let games = state.steam.owned_games(&key, &steam_id, &quiet).await?;

    let mut warnings = Vec::new();
    if !profile.is_public {
        warnings.push(PRIVATE_PROFILE.to_string());
    } else if !games.visible {
        warnings.push(PRIVATE_GAMES.to_string());
    }

    let saved = profile.clone();
    state.update_settings(move |s| {
        s.profile = Some(saved);
        Ok(())
    })?;
    Ok(ProfileCheck {
        profile,
        games_visible: games.visible,
        warnings,
    })
}

/// Starts a scan, or continues a paused one. Returns right away; progress
/// arrives as `scan-status` events.
#[tauri::command]
pub fn start_scan(app: AppHandle, resume: Option<bool>) -> ScanStatus {
    scan::start(&app, resume.unwrap_or(false))
}

#[tauri::command]
pub fn pause_scan(app: AppHandle) -> ScanStatus {
    scan::pause(&app)
}

#[tauri::command]
pub fn get_scan_status(app: AppHandle) -> ScanStatus {
    scan::status(&app)
}

#[tauri::command]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanSummary> {
    scan::last_summary(&state)
}

fn rules(state: &AppState) -> completion::Rules {
    completion::Rules::from_settings(&state.settings())
}

/// Every listed game: owned, unowned with items, and Steam Family games when
/// that setting is on, with how complete each one is.
#[tauri::command]
pub fn get_games(state: State<'_, AppState>) -> Result<Vec<GameRow>, String> {
    let rules = rules(&state);
    let snap = state.snapshot(rules)?;
    state.db.read(|c| {
        let mut games = db::games(c, rules.include_family)?;
        for g in &mut games {
            if let Some(done) = snap.games.get(&g.appid) {
                g.percent = done.percent;
                g.complete_items = done.complete;
                g.counted_items = done.total;
                g.cards_complete = done.cards_complete;
                let p = &done.prices;
                g.value = (p.value_copies > 0).then_some(p.value);
                g.cost = (p.cost_items > 0 || (done.total > 0 && done.complete == done.total))
                    .then_some(p.cost);
                g.cost_points = p.cost_points;
                g.currency = snap.account.currency.clone();
            }
        }
        Ok(games)
    })
}

/// Completion for the whole account.
#[tauri::command]
pub fn get_completion(state: State<'_, AppState>) -> Result<AccountCompletion, String> {
    let rules = rules(&state);
    Ok(state.snapshot(rules)?.account.clone())
}

#[derive(Serialize)]
pub struct GameDetail {
    completion: Option<GameCompletion>,
    items: Vec<ItemView>,
    /// The currency prices are shown in.
    currency: String,
}

fn game_detail(state: &AppState, appid: u64) -> Result<GameDetail, String> {
    let snap = state.snapshot(rules(state))?;
    Ok(GameDetail {
        completion: snap.games.get(&appid).cloned(),
        items: snap.game_items(appid),
        currency: snap.account.currency.clone(),
    })
}

/// One game's items, with what the user owns and each item's tags.
#[tauri::command]
pub fn get_game_detail(state: State<'_, AppState>, appid: u64) -> Result<GameDetail, String> {
    game_detail(&state, appid)
}

/// Sets an item's availability by hand, or hands it back to the app with `None`.
#[tauri::command]
pub fn set_item_availability(
    state: State<'_, AppState>,
    appid: u64,
    key: String,
    availability: Option<Availability>,
) -> Result<GameDetail, String> {
    if !key.starts_with(&format!("{appid}-")) {
        return Err("That item isn't part of this game.".to_string());
    }
    state
        .db
        .write(|tx| completion::set_override(tx, &key, availability, db::now()))?;
    game_detail(&state, appid)
}

#[derive(Deserialize)]
pub struct PriceSettingsPatch {
    currency: Option<String>,
    price_window: Option<PriceWindow>,
    spike_percent: Option<u32>,
    spike_min_cents: Option<u32>,
}

/// Changes how prices are shown. A new currency reads exchange rates straight
/// away, so prices switch over without a scan.
#[tauri::command]
pub async fn update_price_settings(
    state: State<'_, AppState>,
    patch: PriceSettingsPatch,
) -> Result<SettingsView, String> {
    if let Some(code) = &patch.currency {
        if crate::market::currency_id(code).is_none() {
            return Err("That currency isn't one Steam uses.".to_string());
        }
    }
    let updated = state.update_settings(|s| {
        if let Some(code) = &patch.currency {
            s.currency = code.to_ascii_uppercase();
        }
        if let Some(w) = patch.price_window {
            s.price_window = w;
        }
        if let Some(p) = patch.spike_percent {
            s.spike_percent = p.clamp(5, 500);
        }
        if let Some(c) = patch.spike_min_cents {
            s.spike_min_cents = c.min(10_000);
        }
        Ok(())
    })?;
    if patch.currency.is_some() {
        // The overview explains it if the rates couldn't be read.
        let _ = prices::refresh_rates(&state).await;
    }
    Ok(updated.view())
}

/// Reads the order books of a game's cards that still need several copies,
/// when the game is opened, and returns the game with its updated cost.
/// Progress goes out as "book-progress" events.
#[tauri::command]
pub async fn read_game_books(
    app: AppHandle,
    state: State<'_, AppState>,
    appid: u64,
) -> Result<GameDetail, String> {
    let cookie = match session::current(&app).await {
        Ok(Some(s)) => Some(s.community_header()),
        _ => None,
    };
    let emit = |p: prices::BookProgress| {
        let _ = app.emit("book-progress", p);
    };
    prices::game_books(&state, appid, cookie.as_deref(), &emit).await?;
    game_detail(&state, appid)
}

/// An item's price history: the stored last year (read from Steam the first
/// time), or the whole history read fresh with `full`.
#[tauri::command]
pub async fn get_price_history(
    app: AppHandle,
    state: State<'_, AppState>,
    hash_name: String,
    full: Option<bool>,
) -> Result<PriceHistory, String> {
    let cookie = match session::current(&app).await {
        Ok(Some(s)) => Some(s.community_header()),
        _ => None,
    };
    prices::item_history(&state, &hash_name, full.unwrap_or(false), cookie.as_deref()).await
}

#[tauri::command]
pub fn get_diagnostics_plan() -> Vec<DiagStep> {
    diagnostics::plan()
}

#[tauri::command]
pub async fn run_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticsResult, String> {
    if state.scan.is_running() {
        return Err(SCANNING.to_string());
    }
    let _busy = state.try_begin().ok_or_else(|| BUSY.to_string())?;
    let note_app = app.clone();
    let notify = move |message: String| {
        let _ = note_app.emit("diag-note", message);
    };
    let step_app = app.clone();
    let emit_step = move |step: DiagStep| {
        let _ = step_app.emit("diag-progress", step);
    };
    diagnostics::run(&app, &notify, &emit_step).await
}

/// Only files inside the reports folder can be read or shown.
fn report_path(state: &AppState, path: &str) -> Result<PathBuf, String> {
    let dir = state
        .paths
        .reports_dir()
        .canonicalize()
        .map_err(|_| "The reports folder is missing.".to_string())?;
    let requested = Path::new(path)
        .canonicalize()
        .map_err(|_| "That report file no longer exists.".to_string())?;
    if !requested.starts_with(&dir) {
        return Err("Only connection test reports can be opened here.".to_string());
    }
    Ok(PathBuf::from(path))
}

#[tauri::command]
pub fn read_report(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let file = report_path(&state, &path)?;
    std::fs::read_to_string(file).map_err(|e| format!("Couldn't read the report: {e}"))
}

#[tauri::command]
pub fn reveal_report(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let file = report_path(&state, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(file)
        .map_err(|e| format!("Couldn't open the folder: {e}"))
}

const ALLOWED_LINKS: &[&str] = &[
    "https://steamcommunity.com/",
    "https://store.steampowered.com/",
    "https://help.steampowered.com/",
    "https://github.com/",
];

/// Opens a Steam or GitHub page in the user's normal web browser.
#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    if !ALLOWED_LINKS.iter().any(|prefix| url.starts_with(prefix)) {
        return Err("That link can't be opened from the app.".to_string());
    }
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("Couldn't open the link: {e}"))
}

#[tauri::command]
pub fn get_update_info(state: State<'_, AppState>) -> UpdateState {
    updater::snapshot(&state)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateState, String> {
    Ok(updater::check(&app, false).await)
}

/// Installs the newest version and restarts the app.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install_latest_and_restart(&app).await
}

/// Every published version, for the version history.
#[tauri::command]
pub async fn get_versions(app: AppHandle) -> Result<VersionList, String> {
    updater::versions(&app).await
}

/// Installs a specific version (newer or older) and restarts the app.
#[tauri::command]
pub async fn install_version(app: AppHandle, version: String) -> Result<(), String> {
    updater::install_version_and_restart(&app, &version).await
}

#[derive(Serialize)]
pub struct SignInResult {
    settings: SettingsView,
    /// True when signing in switched the tracked profile to a different account.
    switched_profile: bool,
}

/// Opens Steam's sign-in window and waits for the user to sign in.
#[tauri::command]
pub async fn sign_in(app: AppHandle, state: State<'_, AppState>) -> Result<SignInResult, String> {
    let signed = session::sign_in(&app).await?;
    let previous = state.settings().profile.map(|p| p.steam_id);
    let switched_profile =
        previous.is_some() && previous.as_deref() != Some(signed.steam_id.as_str());

    // The app tracks the signed-in account.
    let quiet = |_: String| {};
    let profile = match state.api_key() {
        Ok(key) => state
            .steam
            .player_summary(&key, &signed.steam_id, &quiet)
            .await
            .ok(),
        Err(_) => None,
    };
    let steam_id = signed.steam_id.clone();
    let updated = state.update_settings(move |s| {
        let keep_current = s.profile.as_ref().is_some_and(|p| p.steam_id == steam_id);
        if let Some(p) = profile {
            s.profile = Some(p);
        } else if !keep_current {
            s.profile = Some(ProfileView {
                steam_id: steam_id.clone(),
                persona_name: "Steam user".to_string(),
                avatar_url: None,
                profile_url: None,
                is_public: true,
            });
        }
        s.signed_in_steam_id = Some(steam_id);
        Ok(())
    })?;
    *state.session.lock().await = Some(signed);
    Ok(SignInResult {
        settings: updated.view(),
        switched_profile,
    })
}

/// Signs out of Steam, on Steam's side too.
#[tauri::command]
pub async fn sign_out(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsView, String> {
    let updated = state.update_settings(|s| {
        s.signed_in_steam_id = None;
        Ok(())
    })?;
    session::sign_out(&app).await;
    Ok(updated.view())
}
