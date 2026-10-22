//! Signing in to Steam.
//!
//! Signing in happens on Steam's own sign-in page, shown in a small app window
//! (it offers a QR code for the Steam mobile app as well as a password form).
//! Once Steam sets its sign-in cookie, the app reads that cookie from the
//! window's cookie store and closes the window. The cookies stay in the
//! WebView's profile inside the data folder, encrypted for the current Windows
//! user, like a browser's. The app never reads or stores the password.
//!
//! Steam renews a web sign-in by itself whenever one of its pages loads, so when
//! the cookie is about to expire the app quietly loads a Steam page in a hidden
//! window and reads the renewed cookie.

use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::Value;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::state::AppState;
use crate::steam::{community_url, store_url, Bucket};

const LOGIN_LABEL: &str = "steam-login";
const REFRESH_LABEL: &str = "steam-refresh";
const LOGOUT_LABEL: &str = "steam-logout";
const COMMUNITY_HOME: &str = "https://steamcommunity.com/";
const STORE_HOME: &str = "https://store.steampowered.com/";
const LOGIN_PAGE: &str = "https://steamcommunity.com/login/home/?goto=";
const LOGOUT_SCRIPT: &str = r#"(function () {
  if (window.__sctLogout) return;
  window.__sctLogout = true;
  var body = new URLSearchParams();
  body.append('sessionid', window.g_sessionID || '');
  fetch('/login/logout/', { method: 'POST', body: body, credentials: 'include' }).catch(function () {});
})();"#;

#[derive(Clone, Debug)]
pub struct Session {
    pub steam_id: String,
    /// Value of the steamcommunity.com `steamLoginSecure` cookie.
    pub community_cookie: String,
    /// Value of the store.steampowered.com `steamLoginSecure` cookie, if Steam set one.
    pub store_cookie: Option<String>,
    pub session_id: Option<String>,
    /// Token for Web API calls made as the signed-in user.
    pub webapi_token: Option<String>,
    /// Where the token came from, for the connection test report.
    pub token_source: String,
    /// When the sign-in cookie expires (Unix seconds).
    pub expires_at: Option<i64>,
}

impl Session {
    pub fn valid_for(&self, seconds: i64) -> bool {
        self.expires_at.map_or(true, |exp| exp > now() + seconds)
    }

    /// Cookie header for steamcommunity.com requests.
    pub fn community_header(&self) -> String {
        match &self.session_id {
            Some(id) => format!("steamLoginSecure={}; sessionid={id}", self.community_cookie),
            None => format!("steamLoginSecure={}", self.community_cookie),
        }
    }

    /// Token for api.steampowered.com calls made as the signed-in user.
    pub fn access_token(&self) -> Option<String> {
        self.webapi_token
            .clone()
            .or_else(|| login_token(&self.community_cookie))
    }

    /// Everything that must be blanked out of a shared report.
    pub fn secrets(&self) -> Vec<String> {
        let mut out = vec![self.community_cookie.clone()];
        out.extend(self.store_cookie.clone());
        out.extend(self.webapi_token.clone());
        out.extend(login_token(&self.community_cookie));
        out.extend(self.store_cookie.as_deref().and_then(login_token));
        out.extend(self.session_id.clone());
        out.retain(|s| s.len() >= 8);
        out
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `steamLoginSecure` holds "<steamid>||<access token>", URL-encoded.
pub fn parse_login_cookie(value: &str) -> Option<(String, Option<i64>)> {
    let decoded = value.replace("%7C", "|").replace("%7c", "|");
    let (steam_id, token) = decoded.split_once("||")?;
    if steam_id.len() != 17 || !steam_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((steam_id.to_string(), jwt_expiry(token)))
}

pub fn login_token(value: &str) -> Option<String> {
    let decoded = value.replace("%7C", "|").replace("%7c", "|");
    decoded
        .split_once("||")
        .map(|(_, token)| token.to_string())
        .filter(|t| !t.is_empty())
}

/// Reads the expiry time from a JWT access token.
pub fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims["exp"].as_i64()
}

/// Reads one cookie from the WebView's cookie store. Blocks briefly, so it must
/// be called from an async command or task, never on the main thread.
fn read_cookie(window: &WebviewWindow, url: &str, name: &str) -> Option<String> {
    let url: tauri::Url = url.parse().ok()?;
    let cookies = window.cookies_for_url(url).ok()?;
    cookies
        .iter()
        .find(|c| c.name() == name)
        .map(|c| c.value().to_string())
}

/// The sign-in cookie for a site, if it's present and valid for at least `seconds`.
fn login_cookie(window: &WebviewWindow, url: &str, seconds: i64) -> Option<String> {
    let value = read_cookie(window, url, "steamLoginSecure")?;
    let (_, expiry) = parse_login_cookie(&value)?;
    if expiry.is_some_and(|exp| exp <= now() + seconds) {
        return None;
    }
    Some(value)
}

async fn build_session(app: &AppHandle, window: &WebviewWindow) -> Option<Session> {
    let community_cookie = login_cookie(window, COMMUNITY_HOME, 60)?;
    let (steam_id, expires_at) = parse_login_cookie(&community_cookie)?;
    let session_id = read_cookie(window, COMMUNITY_HOME, "sessionid");
    let store_cookie = login_cookie(window, STORE_HOME, 60);
    let state = app.state::<AppState>();
    let (webapi_token, token_source) =
        fetch_webapi_token(&state, store_cookie.as_deref(), &community_cookie).await;
    Some(Session {
        steam_id,
        community_cookie,
        store_cookie,
        session_id,
        webapi_token,
        token_source,
        expires_at,
    })
}

/// Steam's own pages get a Web API token for the signed-in user from this
/// endpoint. Tries the store first, then the community site; falls back to the
/// token inside the sign-in cookie.
async fn fetch_webapi_token(
    state: &AppState,
    store_cookie: Option<&str>,
    community_cookie: &str,
) -> (Option<String>, String) {
    let quiet = |_: String| {};
    let mut attempts = Vec::new();
    if let Some(cookie) = store_cookie {
        attempts.push((
            Bucket::Store,
            store_url("/pointssummary/ajaxgetasyncconfig", &[]),
            format!("steamLoginSecure={cookie}"),
            "the Steam store",
        ));
    }
    attempts.push((
        Bucket::Community,
        community_url("/pointssummary/ajaxgetasyncconfig", &[]),
        format!("steamLoginSecure={community_cookie}"),
        "the Steam Community site",
    ));
    for (bucket, url, cookie, source) in attempts {
        let Ok(f) = state
            .steam
            .fetch(bucket, &url, Some(cookie.as_str()), &quiet)
            .await
        else {
            continue;
        };
        let token = f
            .json()
            .and_then(|v| v["data"]["webapi_token"].as_str().map(str::to_string))
            .filter(|t| !t.is_empty());
        if let Some(token) = token {
            return (Some(token), source.to_string());
        }
    }
    (None, "the sign-in cookie".to_string())
}

/// Opens Steam's sign-in page and waits for the user to sign in or close it.
pub async fn sign_in(app: &AppHandle) -> Result<Session, String> {
    if let Some(existing) = app.get_webview_window(LOGIN_LABEL) {
        let _ = existing.set_focus();
        return Err("The Steam sign-in window is already open.".to_string());
    }
    let state = app.state::<AppState>();
    let url: tauri::Url = LOGIN_PAGE
        .parse()
        .map_err(|_| "Couldn't open the sign-in page.".to_string())?;
    let window = WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Sign in to Steam")
        .inner_size(540.0, 780.0)
        .min_inner_size(420.0, 560.0)
        .center()
        .data_directory(state.paths.webview_dir())
        .build()
        .map_err(|e| format!("Couldn't open the sign-in window: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(20 * 60);
    loop {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if app.get_webview_window(LOGIN_LABEL).is_none() {
            return Err("Sign-in was cancelled.".to_string());
        }
        if Instant::now() > deadline {
            let _ = window.destroy();
            return Err("Sign-in timed out. Try again when you're ready.".to_string());
        }
        if login_cookie(&window, COMMUNITY_HOME, 60).is_some() {
            // Steam sets the store's cookie a moment after the community one.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let session = build_session(app, &window).await;
            let _ = window.destroy();
            return session.ok_or_else(|| "Steam didn't finish signing in. Try again.".to_string());
        }
    }
}

/// The current sign-in, renewed if it's about to expire. `Ok(None)` means the
/// user hasn't signed in; `Err` means the sign-in has expired.
pub async fn current(app: &AppHandle) -> Result<Option<Session>, String> {
    let state = app.state::<AppState>();
    let Some(signed_in_id) = state.settings().signed_in_steam_id else {
        return Ok(None);
    };

    {
        let cached = state.session.lock().await;
        if let Some(s) = cached.as_ref() {
            if s.valid_for(600) && s.steam_id == signed_in_id {
                return Ok(Some(s.clone()));
            }
        }
    }

    // The cookie store may already hold a fresh sign-in.
    if let Some(main) = app.get_webview_window("main") {
        if login_cookie(&main, COMMUNITY_HOME, 600).is_some() {
            if let Some(s) = build_session(app, &main).await {
                *state.session.lock().await = Some(s.clone());
                return Ok(Some(s));
            }
        }
    }

    match refresh(app).await {
        Some(s) => {
            *state.session.lock().await = Some(s.clone());
            Ok(Some(s))
        }
        None => {
            *state.session.lock().await = None;
            Err("Your Steam sign-in has expired. Sign in again from Settings.".to_string())
        }
    }
}

/// Loads a Steam page in a hidden window so Steam renews the sign-in cookie.
async fn refresh(app: &AppHandle) -> Option<Session> {
    if let Some(old) = app.get_webview_window(REFRESH_LABEL) {
        let _ = old.destroy();
    }
    let state = app.state::<AppState>();
    let url: tauri::Url = "https://steamcommunity.com/my/".parse().ok()?;
    let window = WebviewWindowBuilder::new(app, REFRESH_LABEL, WebviewUrl::External(url))
        .title("Steam")
        .visible(false)
        .data_directory(state.paths.webview_dir())
        .build()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut session = None;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if login_cookie(&window, COMMUNITY_HOME, 60).is_some() {
            session = build_session(app, &window).await;
            break;
        }
    }
    let _ = window.destroy();
    session
}

/// Signs out on Steam's side too, so the session can't be reused.
pub async fn sign_out(app: &AppHandle) {
    let state = app.state::<AppState>();
    *state.session.lock().await = None;
    if let Some(old) = app.get_webview_window(LOGOUT_LABEL) {
        let _ = old.destroy();
    }
    let Ok(url) = COMMUNITY_HOME.parse::<tauri::Url>() else {
        return;
    };
    let built = WebviewWindowBuilder::new(app, LOGOUT_LABEL, WebviewUrl::External(url))
        .title("Steam")
        .visible(false)
        .data_directory(state.paths.webview_dir())
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished
                && payload.url().host_str() == Some("steamcommunity.com")
            {
                let _ = window.eval(LOGOUT_SCRIPT);
            }
        })
        .build();
    if let Ok(window) = built {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let _ = window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(exp: i64) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"typ":"JWT","alg":"EdDSA"}"#);
        let claims = engine.encode(format!(
            r#"{{"iss":"steam","sub":"76561199220451769","exp":{exp}}}"#
        ));
        format!("{header}.{claims}.signature")
    }

    #[test]
    fn reads_the_sign_in_cookie() {
        let token = jwt(1_900_000_000);
        let cookie = format!("76561199220451769%7C%7C{token}");
        let (steam_id, exp) = parse_login_cookie(&cookie).unwrap();
        assert_eq!(steam_id, "76561199220451769");
        assert_eq!(exp, Some(1_900_000_000));
        assert_eq!(login_token(&cookie).as_deref(), Some(token.as_str()));
        assert!(parse_login_cookie("garbage").is_none());
        assert!(parse_login_cookie("123%7C%7Cabc").is_none());
    }

    #[test]
    fn session_headers_and_secrets() {
        let token = jwt(now() + 3600);
        let s = Session {
            steam_id: "76561199220451769".into(),
            community_cookie: format!("76561199220451769%7C%7C{token}"),
            store_cookie: None,
            session_id: Some("abcdef0123456789abcdef01".into()),
            webapi_token: None,
            token_source: "the sign-in cookie".into(),
            expires_at: jwt_expiry(&token),
        };
        assert!(s.valid_for(600));
        assert!(!s.valid_for(7200));
        assert!(s
            .community_header()
            .ends_with("; sessionid=abcdef0123456789abcdef01"));
        assert_eq!(s.access_token().as_deref(), Some(token.as_str()));
        assert!(s.secrets().contains(&token));
    }
}
