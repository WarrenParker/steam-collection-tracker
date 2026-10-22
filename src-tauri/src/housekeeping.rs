//! Keeps the app's folder tidy. Runs at every launch, before any window opens,
//! and only ever touches files the app itself creates.

use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::AppPaths;

/// How many connection test reports to keep.
pub const REPORTS_TO_KEEP: usize = 5;

pub fn tidy_up(paths: &AppPaths) {
    // Half-finished downloads from an interrupted update.
    clear_dir_files(&paths.root.join("update"));

    // Leftovers from swapping in a new exe. Windows can't delete a running exe,
    // so the old one is renamed and removed once it has closed; this catches any
    // that a restart left behind.
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        if let Ok(entries) = fs::read_dir(&exe_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".exe")
                    && (name.contains("__relocated__") || name.contains("__selfdelete__"))
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    prune_reports(paths);

    // Web caches from the Steam sign-in window. Sign-in cookies live elsewhere
    // in the same folder and are kept.
    let profile = paths.webview_dir().join("EBWebView");
    for cache in [
        "Default/Cache",
        "Default/Code Cache",
        "Default/GPUCache",
        "Default/Service Worker/CacheStorage",
        "ShaderCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "Crashpad/reports",
    ] {
        let _ = fs::remove_dir_all(profile.join(cache));
    }
}

/// Keeps only the newest few connection test reports.
pub fn prune_reports(paths: &AppPaths) {
    let Ok(entries) = fs::read_dir(paths.root.join("reports")) else {
        return;
    };
    let mut reports: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("connection-test-") && name.ends_with(".txt")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    reports.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in reports.into_iter().skip(REPORTS_TO_KEEP) {
        let _ = fs::remove_file(path);
    }
}

fn clear_dir_files(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Total size of the data folder, for the settings panel.
pub fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_newest_reports() {
        let root = std::env::temp_dir().join(format!("sct-housekeeping-{}", std::process::id()));
        let paths = AppPaths {
            root: root.clone(),
            portable: true,
        };
        let reports = paths.reports_dir();
        for i in 0..8 {
            fs::write(reports.join(format!("connection-test-{i}.txt")), "x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        fs::write(reports.join("keep-me.txt"), "x").unwrap();
        prune_reports(&paths);
        let mut left: Vec<String> = fs::read_dir(&reports)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left.len(), REPORTS_TO_KEEP + 1);
        assert!(left.contains(&"keep-me.txt".to_string()));
        assert!(left.contains(&"connection-test-7.txt".to_string()));
        assert!(!left.contains(&"connection-test-0.txt".to_string()));
        assert!(folder_size(&root) >= 6);
        let _ = fs::remove_dir_all(&root);
    }
}
