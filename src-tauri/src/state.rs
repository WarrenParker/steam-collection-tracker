use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::paths::AppPaths;
use crate::settings::Settings;
use crate::steam::SteamClient;
use crate::updater::UpdateState;

pub struct AppState {
    pub paths: AppPaths,
    pub settings: Mutex<Settings>,
    pub steam: SteamClient,
    /// The exe's path at launch, used to restart after an update.
    pub exe_path: Option<PathBuf>,
    /// True while a scan or diagnostics run is talking to Steam.
    busy: AtomicBool,
    pub update: Mutex<UpdateState>,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings) -> Self {
        Self {
            paths,
            settings: Mutex::new(settings),
            steam: SteamClient::new(),
            exe_path: std::env::current_exe().ok(),
            busy: AtomicBool::new(false),
            update: Mutex::new(UpdateState::default()),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Changes settings and saves them straight away.
    pub fn update_settings<F>(&self, change: F) -> Result<Settings, String>
    where
        F: FnOnce(&mut Settings) -> Result<(), String>,
    {
        let mut guard = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = guard.clone();
        change(&mut next)?;
        next.save(&self.paths.settings_file())
            .map_err(|e| format!("Couldn't save your settings: {e}"))?;
        *guard = next.clone();
        Ok(next)
    }

    pub fn api_key(&self) -> Result<String, String> {
        self.settings()
            .api_key()?
            .ok_or_else(|| "Add your Steam API key in Settings first.".to_string())
    }

    /// Only one job talks to Steam at a time, so the pacing stays safe.
    pub fn try_begin(&self) -> Option<BusyGuard<'_>> {
        self.busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| BusyGuard(&self.busy))
    }
}

pub struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
