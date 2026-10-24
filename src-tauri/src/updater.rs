//! Updates and version switching, from GitHub Releases.
//!
//! Versions follow semantic versioning (MAJOR.MINOR.PATCH). Each release carries
//! `SteamCollectionTracker.exe` and a `.sha256` file. The app downloads the exe,
//! checks it against the published hash, and swaps it in place of the running
//! one (Windows allows renaming a running exe). The next launch runs the new
//! version; "Restart now" relaunches straight away.
//!
//! Any published version can be installed, including older ones. Installing an
//! older version turns automatic updates off so the app stays on it.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::hexutil;
use crate::paths::AppPaths;
use crate::state::AppState;

pub const ASSET_NAME: &str = "SteamCollectionTracker.exe";
/// Far larger than the app (about 9 MB), so a bad answer can't fill memory.
const MAX_UPDATE_BYTES: usize = 100 * 1024 * 1024;

/// Set by the GitHub build ("owner/repo"). Local builds have no updater.
const REPO: Option<&str> = option_env!("UPDATE_REPO");
/// The commit this copy was built from, set by the GitHub build.
const BUILD_SHA: Option<&str> = option_env!("GITHUB_SHA");

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

/// One published version, for the version history list.
#[derive(Serialize, Clone, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: String,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub installable: bool,
    pub current: bool,
    pub newer: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct VersionList {
    pub current: String,
    pub build: Option<String>,
    pub releases: Vec<ReleaseInfo>,
}

pub fn enabled() -> bool {
    REPO.is_some() && !cfg!(debug_assertions)
}

pub fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

/// Short commit id of this build, like "394d2dd".
pub fn build_id() -> Option<String> {
    BUILD_SHA.map(|sha| sha.chars().take(7).collect())
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

#[derive(Clone, Debug)]
struct Release {
    version: semver::Version,
    notes: String,
    published_at: Option<String>,
    prerelease: bool,
    exe_url: Option<String>,
    sha_url: Option<String>,
}

impl Release {
    fn installable(&self) -> bool {
        self.exe_url.is_some() && self.sha_url.is_some()
    }
}

pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

fn parse_release(v: &Value) -> Option<Release> {
    let version = parse_tag(v["tag_name"].as_str()?)?;
    let assets = v["assets"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let find = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_string)
    };
    Some(Release {
        version,
        notes: v["body"].as_str().unwrap_or_default().trim().to_string(),
        published_at: v["published_at"].as_str().map(str::to_string),
        prerelease: v["prerelease"].as_bool().unwrap_or(false),
        exe_url: find(ASSET_NAME),
        sha_url: find(&format!("{ASSET_NAME}.sha256")),
    })
}

async fn github_get(http: &reqwest::Client, path: &str, not_found: &str) -> Result<Value, String> {
    let repo = REPO.ok_or_else(|| "Updates are turned off in this copy of the app.".to_string())?;
    let url = format!("https://api.github.com/repos/{repo}{path}");
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
        return Err(not_found.to_string());
    }
    if status == 403 || status == 429 {
        return Err("The update server is busy. Try again in a few minutes.".to_string());
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "The update server answered with code {status}. Try again later."
        ));
    }
    resp.json()
        .await
        .map_err(|_| "The update information couldn't be read.".to_string())
}

async fn latest_release(http: &reqwest::Client) -> Result<Release, String> {
    let v = github_get(
        http,
        "/releases/latest",
        "No versions have been published yet.",
    )
    .await?;
    parse_release(&v)
        .ok_or_else(|| "The newest version's information couldn't be read.".to_string())
}

async fn all_releases(http: &reqwest::Client) -> Result<Vec<Release>, String> {
    let v = github_get(
        http,
        "/releases?per_page=50",
        "GitHub couldn't find the app's releases. Try again later.",
    )
    .await?;
    let unreadable = || "GitHub's list of versions couldn't be read. Try again later.".to_string();
    let list = v.as_array().ok_or_else(unreadable)?;
    let mut releases: Vec<Release> = list.iter().filter_map(parse_release).collect();
    if releases.is_empty() && !list.is_empty() {
        return Err(unreadable());
    }
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(releases)
}

/// Update files must come from this repository's own GitHub releases.
fn trusted_download(url: &str, repo: &str) -> bool {
    if repo.is_empty() || url.contains("..") || url.contains('@') {
        return false;
    }
    let prefix = format!("https://github.com/{repo}/releases/download/").to_ascii_lowercase();
    url.to_ascii_lowercase().starts_with(&prefix)
}

async fn download_and_stage(
    http: &reqwest::Client,
    paths: &AppPaths,
    release: &Release,
) -> Result<(), String> {
    let (Some(exe_url), Some(sha_url)) = (&release.exe_url, &release.sha_url) else {
        return Err(format!(
            "Version {} has no app file to install.",
            release.version
        ));
    };
    let repo = REPO.unwrap_or_default();
    if !trusted_download(exe_url, repo) || !trusted_download(sha_url, repo) {
        return Err(
            "The update isn't from this app's own releases, so it wasn't installed.".to_string(),
        );
    }
    let failed = |_: reqwest::Error| {
        "Couldn't download the update. Check your internet connection and try again.".to_string()
    };
    let bytes = http
        .get(exe_url)
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
        .get(sha_url)
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
    if bytes.len() > MAX_UPDATE_BYTES {
        return Err(
            "The download didn't pass its safety check, so it wasn't installed.".to_string(),
        );
    }
    let actual = hexutil::encode(&Sha256::digest(&bytes));
    if expected.len() != 64 || expected != actual {
        return Err(
            "The download didn't pass its safety check, so it wasn't installed.".to_string(),
        );
    }

    let file = paths.update_dir().join("SteamCollectionTracker.new.exe");
    std::fs::write(&file, &bytes).map_err(|e| format!("Couldn't save the download: {e}"))?;
    let result = self_replace::self_replace(&file).map_err(|e| format!("Couldn't install it: {e}"));
    let _ = std::fs::remove_file(&file);
    result
}

/// Checks GitHub for a newer version. Downloads it when automatic updates are
/// on, or when `force_download` is set (the user pressed "Update now").
pub async fn check(app: &AppHandle, force_download: bool) -> UpdateState {
    if !enabled() {
        return modify(app, |s| {
            s.error = Some("Updates are turned off in this copy of the app.".to_string())
        });
    }

    modify(app, |s| {
        s.checking = true;
        s.error = None;
    });
    let state = app.state::<AppState>();
    let http = state.steam.http().clone();

    let release = match latest_release(&http).await {
        Ok(r) => r,
        Err(e) => {
            return modify(app, |s| {
                s.checking = false;
                s.error = Some(e);
            })
        }
    };

    let newer = release.version > current_version();
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

/// Every published version, newest first.
pub async fn versions(app: &AppHandle) -> Result<VersionList, String> {
    let state = app.state::<AppState>();
    let current = current_version();
    if !enabled() {
        return Err(
            "Updates are turned off in this copy of the app, so there's no version list."
                .to_string(),
        );
    }
    let releases = all_releases(state.steam.http()).await?;
    Ok(VersionList {
        current: current.to_string(),
        build: build_id(),
        releases: releases
            .into_iter()
            .map(|r| ReleaseInfo {
                version: r.version.to_string(),
                installable: r.installable(),
                current: r.version == current,
                newer: r.version > current,
                notes: r.notes,
                published_at: r.published_at,
                prerelease: r.prerelease,
            })
            .collect(),
    })
}

fn restart(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let exe = state.exe_path.clone().ok_or_else(|| {
        "Couldn't restart automatically. Close the app and open it again to finish.".to_string()
    })?;
    std::process::Command::new(&exe).spawn().map_err(|e| {
        format!("Couldn't restart automatically ({e}). Close the app and open it again to finish.")
    })?;
    app.exit(0);
    Ok(())
}

/// Installs the newest version (if not already downloaded) and restarts.
pub async fn install_latest_and_restart(app: &AppHandle) -> Result<(), String> {
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
    restart(app)
}

/// Installs a specific version (newer or older) and restarts.
pub async fn install_version_and_restart(app: &AppHandle, version: &str) -> Result<(), String> {
    if !enabled() {
        return Err("Updates are turned off in this copy of the app.".to_string());
    }
    let wanted =
        parse_tag(version).ok_or_else(|| format!("\"{version}\" isn't a version number."))?;
    let current = current_version();
    if wanted == current {
        return Err(format!("You're already on version {current}."));
    }
    let state = app.state::<AppState>();
    let release = all_releases(state.steam.http())
        .await?
        .into_iter()
        .find(|r| r.version == wanted)
        .ok_or_else(|| format!("Version {wanted} isn't available to download."))?;
    download_and_stage(state.steam.http(), &state.paths, &release).await?;
    if wanted < current {
        // Otherwise the next launch would update straight back to the newest version.
        state.update_settings(|s| {
            s.auto_update = false;
            Ok(())
        })?;
    }
    restart(app)
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
    fn releases() {
        let full = json!({
            "tag_name": "v0.2.0", "body": " Notes ", "published_at": "2026-10-22T07:00:00Z",
            "assets": [
                {"name": "SteamCollectionTracker.exe", "browser_download_url": "https://example.com/a.exe"},
                {"name": "SteamCollectionTracker.exe.sha256", "browser_download_url": "https://example.com/a.sha256"}
            ]
        });
        let r = parse_release(&full).unwrap();
        assert_eq!(r.version, semver::Version::new(0, 2, 0));
        assert_eq!(r.notes, "Notes");
        assert!(r.installable());

        let missing_hash = json!({
            "tag_name": "v0.2.0",
            "assets": [{"name": "SteamCollectionTracker.exe", "browser_download_url": "https://example.com/a.exe"}]
        });
        assert!(!parse_release(&missing_hash).unwrap().installable());
        assert!(parse_release(&json!({"tag_name": "nightly"})).is_none());
    }

    #[test]
    fn update_files_come_from_this_repos_releases() {
        let repo = "WarrenParker/steam-collection-tracker";
        let good = "https://github.com/WarrenParker/steam-collection-tracker/releases/download/v0.8.2/SteamCollectionTracker.exe";
        assert!(trusted_download(good, repo));
        assert!(trusted_download(&good.to_lowercase(), repo));
        for bad in [
            "https://example.com/a.exe",
            "http://github.com/WarrenParker/steam-collection-tracker/releases/download/v1/a.exe",
            "https://github.com/someone/else/releases/download/v1/a.exe",
            "https://github.com/WarrenParker/steam-collection-tracker/releases/download/../../x/a.exe",
            "https://github.com/WarrenParker/steam-collection-tracker/releases/download@evil.com/a.exe",
        ] {
            assert!(!trusted_download(bad, repo), "{bad}");
        }
        assert!(!trusted_download(good, ""));
    }
}
