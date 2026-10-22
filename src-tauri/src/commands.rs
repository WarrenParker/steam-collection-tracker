//! Everything the interface can ask the engine to do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::diagnostics::{self, DiagStep, DiagnosticsResult};
use crate::profile_input::{self, ProfileRef};
use crate::scan::{self, ScanProgress, ScanSummary, PRIVATE_GAMES, PRIVATE_PROFILE};
use crate::settings::{SettingsView, Theme};
use crate::state::AppState;
use crate::steam::ProfileView;
use crate::updater::{self, UpdateState};

const BUSY: &str = "The app is already talking to Steam. Wait for that to finish, then try again.";

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
    portable: bool,
    updates_enabled: bool,
}

#[derive(Deserialize)]
pub struct PreferencesPatch {
    theme: Option<Theme>,
    auto_update: Option<bool>,
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

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.paths.root.display().to_string(),
        portable: state.paths.portable,
        updates_enabled: updater::enabled(),
    }
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

#[tauri::command]
pub async fn run_scan(app: AppHandle, state: State<'_, AppState>) -> Result<ScanSummary, String> {
    let _busy = state.try_begin().ok_or_else(|| BUSY.to_string())?;
    let note_app = app.clone();
    let notify = move |message: String| {
        let _ = note_app.emit("scan-note", message);
    };
    let progress = move |p: ScanProgress| {
        let _ = app.emit("scan-progress", p);
    };
    scan::run(state.inner(), &notify, &progress).await
}

#[tauri::command]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanSummary> {
    scan::load_last(&state.paths)
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
    let _busy = state.try_begin().ok_or_else(|| BUSY.to_string())?;
    let note_app = app.clone();
    let notify = move |message: String| {
        let _ = note_app.emit("diag-note", message);
    };
    let emit_step = move |step: DiagStep| {
        let _ = app.emit("diag-progress", step);
    };
    diagnostics::run(state.inner(), &notify, &emit_step).await
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

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install_and_restart(&app).await
}
