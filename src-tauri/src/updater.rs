//! Self-updating from GitHub Releases.
//!
//! Each release carries `SteamCollectionTracker.exe` and a `.sha256` file. The
//! app downloads the new exe, checks it against the published hash, and swaps
//! it in place of the running one (Windows allows renaming a running exe). The
//! next launch runs the new version; "Restart now" just relaunches early.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::hexutil;
use crate::paths::AppPaths;
use crate::state::AppState;

pub const ASSET_NAME: &str = "SteamCollectionTracker.exe";

/// Set by the GitHub build ("owner/repo"). Local builds have no updater.
const REPO: Option<&str> = option_env!("UPDATE_REPO");

#[derive(Serialize, Clone, Debug, Default)]
pub struct UpdateState {
    pub enabled: bool,
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    pub ready: bool,
    pub checking: bool,
    pub notes: Option<String>,
    pub error: Option<String>,
}

pub fn enabled() -> bool {
    REPO.is_some() && !cfg!(debug_assertions)
}

pub fn snapshot(state: &AppState) -> UpdateState {
    let mut s = state
        .update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    s.enabled = enabled();
    s.current = env!("CARGO_PKG_VERSION").to_string();
    s
}

fn modify(app: &AppHandle, change: impl FnOnce(&mut UpdateState)) -> UpdateState {
    let state = app.state::<AppState>();
    {
        let mut guard = state.update.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut guard);
    }
    let snap = snapshot(&state);
    let _ = app.emit("update-status", snap.clone());
    snap
}

struct Release {
    version: semver::Version,
    notes: String,
    exe_url: String,
    sha_url: String,
}

pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

fn parse_release(v: &Value) -> Option<Release> {
    let version = parse_tag(v["tag_name"].as_str()?)?;
    let assets = v["assets"].as_array()?;
    let find = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_string)
    };
    Some(Release {
        version,
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        exe_url: find(ASSET_NAME)?,
        sha_url: find(&format!("{ASSET_NAME}.sha256"))?,
    })
}

async fn latest_release(http: &reqwest::Client, repo: &str) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = http
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| {
            "Couldn't reach the update server. Check your internet connection.".to_string()
        })?;
    let status = resp.status().as_u16();
    if status == 404 {
        return Err("No updates have been published yet.".to_string());
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "The update server answered with code {status}. Try again later."
        ));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|_| "The update information couldn't be read.".to_string())?;
    parse_release(&v).ok_or_else(|| "The newest release is missing the app file.".to_string())
}

async fn download_and_stage(
    http: &reqwest::Client,
    paths: &AppPaths,
    release: &Release,
) -> Result<(), String> {
    let failed = |_: reqwest::Error| {
        "Couldn't download the update. The app will try again the next time it opens.".to_string()
    };
    let bytes = http
        .get(&release.exe_url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(failed)?
        .error_for_status()
        .map_err(failed)?
        .bytes()
        .await
        .map_err(failed)?;
    let expected_text = http
        .get(&release.sha_url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(failed)?
        .error_for_status()
        .map_err(failed)?
        .text()
        .await
        .map_err(failed)?;

    let expected = expected_text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actual = hexutil::encode(&Sha256::digest(&bytes));
    if expected.len() != 64 || expected != actual {
        return Err(
            "The downloaded update didn't pass its safety check, so it wasn't installed."
                .to_string(),
        );
    }

    let file = paths.update_dir().join("SteamCollectionTracker.new.exe");
    std::fs::write(&file, &bytes).map_err(|e| format!("Couldn't save the update: {e}"))?;
    let result =
        self_replace::self_replace(&file).map_err(|e| format!("Couldn't install the update: {e}"));
    let _ = std::fs::remove_file(&file);
    result
}

/// Checks GitHub for a newer version. Downloads it when automatic updates are
/// on, or when `force_download` is set (the user pressed "Update now").
pub async fn check(app: &AppHandle, force_download: bool) -> UpdateState {
    let Some(repo) = REPO else {
        return modify(app, |s| {
            s.error = Some("Updates are turned off in this copy of the app.".to_string())
        });
    };
    if cfg!(debug_assertions) {
        return modify(app, |s| {
            s.error = Some("Updates are turned off in development builds.".to_string())
        });
    }

    modify(app, |s| {
        s.checking = true;
        s.error = None;
    });
    let state = app.state::<AppState>();
    let http = state.steam.http().clone();
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0));

    let release = match latest_release(&http, repo).await {
        Ok(r) => r,
        Err(e) => {
            return modify(app, |s| {
                s.checking = false;
                s.error = Some(e);
            })
        }
    };

    let newer = release.version > current;
    let latest = release.version.to_string();
    let notes = release.notes.clone();
    modify(app, |s| {
        s.latest = Some(latest);
        s.notes = Some(notes);
        s.available = newer;
    });

    let auto = state.settings().auto_update;
    let already_ready = state.update.lock().unwrap_or_else(|e| e.into_inner()).ready;
    if newer && !already_ready && (force_download || auto) {
        return match download_and_stage(&http, &state.paths, &release).await {
            Ok(()) => modify(app, |s| {
                s.checking = false;
                s.ready = true;
            }),
            Err(e) => modify(app, |s| {
                s.checking = false;
                s.error = Some(e);
            }),
        };
    }
    modify(app, |s| s.checking = false)
}

pub async fn install_and_restart(app: &AppHandle) -> Result<(), String> {
    let ready = {
        let state = app.state::<AppState>();
        let ready = state.update.lock().unwrap_or_else(|e| e.into_inner()).ready;
        ready
    };
    if !ready {
        let snap = check(app, true).await;
        if !snap.ready {
            return Err(snap
                .error
                .unwrap_or_else(|| "You're already on the newest version.".to_string()));
        }
    }
    let state = app.state::<AppState>();
    let exe = state.exe_path.clone().ok_or_else(|| {
        "Couldn't find the app's file to restart it. Close and reopen the app to finish updating."
            .to_string()
    })?;
    std::process::Command::new(&exe).spawn().map_err(|e| {
        format!(
            "Couldn't restart automatically ({e}). Close and reopen the app to finish updating."
        )
    })?;
    app.exit(0);
    Ok(())
}

pub fn spawn_startup_check(app: AppHandle) {
    if !enabled() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Let the window open first.
        tokio::time::sleep(Duration::from_secs(4)).await;
        let _ = check(&app, false).await;
    });
}

/// Removes any half-finished download from a previous run.
pub fn cleanup_leftovers(paths: &AppPaths) {
    if let Ok(entries) = std::fs::read_dir(paths.root.join("update")) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tags() {
        assert_eq!(parse_tag("v0.2.1").unwrap(), semver::Version::new(0, 2, 1));
        assert_eq!(parse_tag("1.0.0").unwrap(), semver::Version::new(1, 0, 0));
        assert!(parse_tag("latest").is_none());
        assert!(parse_tag("v0.10.0").unwrap() > parse_tag("v0.9.3").unwrap());
    }

    #[test]
    fn release_needs_both_files() {
        let full = json!({
            "tag_name": "v0.2.0", "body": "Notes",
            "assets": [
                {"name": "SteamCollectionTracker.exe", "browser_download_url": "https://example.com/a.exe"},
                {"name": "SteamCollectionTracker.exe.sha256", "browser_download_url": "https://example.com/a.sha256"}
            ]
        });
        let r = parse_release(&full).unwrap();
        assert_eq!(r.version, semver::Version::new(0, 2, 0));
        assert_eq!(r.exe_url, "https://example.com/a.exe");

        let missing_hash = json!({
            "tag_name": "v0.2.0",
            "assets": [{"name": "SteamCollectionTracker.exe", "browser_download_url": "https://example.com/a.exe"}]
        });
        assert!(parse_release(&missing_hash).is_none());
    }
}
