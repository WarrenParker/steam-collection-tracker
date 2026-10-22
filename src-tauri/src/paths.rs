//! Where the app keeps its files.
//!
//! The app is portable: it keeps everything in a `data` folder next to the exe.
//! If that folder can't be written to (for example, the exe was put in
//! Program Files), it falls back to the user's local app data folder.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub portable: bool,
}

impl AppPaths {
    pub fn detect() -> Self {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("data")))
        {
            if is_writable(&dir) {
                return Self {
                    root: dir,
                    portable: true,
                };
            }
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join("SteamCollectionTracker");
        let _ = fs::create_dir_all(&root);
        Self {
            root,
            portable: false,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.sub("reports")
    }

    pub fn update_dir(&self) -> PathBuf {
        self.sub("update")
    }

    pub fn webview_dir(&self) -> PathBuf {
        self.root.join("webview")
    }

    fn sub(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        let _ = fs::create_dir_all(&dir);
        dir
    }
}

fn is_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-test");
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
