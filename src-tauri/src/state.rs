use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::completion::{self, Rules, Snapshot};
use crate::db::{self, Db};
use crate::paths::AppPaths;
use crate::scan::ScanControl;
use crate::session::Session;
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
    /// The Steam sign-in, cached while it's valid.
    pub session: tokio::sync::Mutex<Option<Session>>,
    /// The collection database.
    pub db: Db,
    /// Set when the database file couldn't be opened and a temporary one is in use.
    pub db_problem: Option<String>,
    /// The background scan.
    pub scan: ScanControl,
    /// The latest completion figures, reused until the database or the
    /// rules change, or a minute passes.
    snapshot: Mutex<Option<(SnapshotKey, Arc<Snapshot>)>>,
}

#[derive(Clone, Copy, PartialEq)]
struct SnapshotKey {
    generation: u64,
    rules: Rules,
    minute: i64,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings) -> Self {
        let (db, db_problem) = match Db::open(&paths.root.join(db::FILE_NAME)) {
            Ok(db) => (db, None),
            Err(e) => (Db::in_memory(), Some(e)),
        };
        Self {
            paths,
            settings: Mutex::new(settings),
            steam: SteamClient::new(),
            exe_path: std::env::current_exe().ok(),
            busy: AtomicBool::new(false),
            update: Mutex::new(UpdateState::default()),
            session: tokio::sync::Mutex::new(None),
            db,
            db_problem,
            scan: ScanControl::new(),
            snapshot: Mutex::new(None),
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

    /// Completion and prices for the whole collection. Working them out reads
    /// every item, so the result is kept and reused while nothing has changed.
    /// Items whose availability depends on a date are checked again each minute.
    pub fn snapshot(&self, rules: Rules) -> Result<Arc<Snapshot>, String> {
        let now = db::now();
        let key = SnapshotKey {
            generation: self.db.generation(),
            rules,
            minute: now / 60,
        };
        let mut cached = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, snap)) = cached.as_ref() {
            if *k == key {
                return Ok(snap.clone());
            }
        }
        let snap = Arc::new(self.db.read(|c| completion::compute(c, rules, now))?);
        *cached = Some((key, snap.clone()));
        Ok(snap)
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
