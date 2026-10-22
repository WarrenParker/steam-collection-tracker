//! Talking to Steam.
//!
//! Every request goes through one paced client. Steam limits how often an
//! internet connection may call the Community and Market pages, and going over
//! can lock you out for hours, so each kind of request has its own minimum gap
//! and the client waits and retries when Steam says to slow down.

pub mod inventory;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

pub const WEBAPI: &str = "https://api.steampowered.com";
pub const COMMUNITY: &str = "https://steamcommunity.com";

/// Called with short, plain-language status messages (for example when waiting).
pub type Notify<'a> = &'a (dyn Fn(String) + Send + Sync);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Bucket {
    /// api.steampowered.com: generous limits.
    WebApi,
    /// steamcommunity.com pages and inventory: strict per-connection limits.
    Community,
    /// steamcommunity.com/market: the strictest.
    Market,
}

impl Bucket {
    fn min_interval(self) -> Duration {
        match self {
            Bucket::WebApi => Duration::from_millis(300),
            Bucket::Community => Duration::from_millis(3500),
            Bucket::Market => Duration::from_millis(4000),
        }
    }

    /// Seconds to wait before each retry after Steam answers "too many requests".
    fn retry_waits(self) -> &'static [u64] {
        match self {
            Bucket::WebApi => &[5, 15],
            _ => &[20, 60],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub body: String,
    pub elapsed_ms: u64,
    pub final_url: String,
}

impl Fetched {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProfileView {
    pub steam_id: String,
    pub persona_name: String,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    pub is_public: bool,
}

#[derive(Debug, Clone, Default)]
pub struct OwnedGames {
    /// False when the profile's game details are private.
    pub visible: bool,
    pub games: Vec<OwnedGame>,
}

#[derive(Debug, Clone)]
pub struct OwnedGame {
    pub appid: u64,
    pub name: String,
    pub playtime_minutes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Badges {
    pub visible: bool,
    pub player_level: Option<u64>,
    pub list: Vec<Badge>,
}

#[derive(Debug, Clone)]
pub struct Badge {
    pub badge_id: Option<u64>,
    /// Set for game badges; absent for Steam's own badges (events, years of service…).
    pub appid: Option<u64>,
    pub level: u64,
    pub foil: bool,
}

pub struct SteamClient {
    http: reqwest::Client,
    last: Mutex<HashMap<Bucket, Instant>>,
    requests: AtomicU32,
}

impl SteamClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "SteamCollectionTracker/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .gzip(true)
            .build()
            .expect("failed to build the HTTP client");
        Self {
            http,
            last: Mutex::new(HashMap::new()),
            requests: AtomicU32::new(0),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Total requests sent to Steam since the app started.
    pub fn requests(&self) -> u32 {
        self.requests.load(Ordering::Relaxed)
    }

    async fn pace(&self, bucket: Bucket) {
        let mut last = self.last.lock().await;
        if let Some(prev) = last.get(&bucket) {
            let elapsed = prev.elapsed();
            let min = bucket.min_interval();
            if elapsed < min {
                tokio::time::sleep(min - elapsed).await;
            }
        }
        last.insert(bucket, Instant::now());
    }

    /// Sends a paced GET request. Non-success HTTP codes are returned as data so
    /// callers can explain them; only network failures become errors.
    pub async fn fetch(
        &self,
        bucket: Bucket,
        url: &Url,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let waits = bucket.retry_waits();
        let mut attempt = 0usize;
        let mut retried_network = false;
        loop {
            self.pace(bucket).await;
            self.requests.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            match self.http.get(url.clone()).send().await {
                Err(err) => {
                    if !retried_network && (err.is_timeout() || err.is_connect()) {
                        retried_network = true;
                        notify("Steam didn't answer in time. Trying again…".to_string());
                        continue;
                    }
                    return Err(network_message(&err));
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 429 && attempt < waits.len() {
                        let wait = waits[attempt];
                        attempt += 1;
                        notify(format!(
                            "Steam asked the app to slow down. Waiting {wait} seconds before trying again…"
                        ));
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        continue;
                    }
                    let final_url = resp.url().to_string();
                    let body = resp.text().await.map_err(|e| network_message(&e))?;
                    return Ok(Fetched {
                        status,
                        body,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        final_url,
                    });
                }
            }
        }
    }

    pub async fn check_api_key(&self, key: &str, notify: Notify<'_>) -> Result<(), String> {
        let url = webapi_url("/ISteamWebAPIUtil/GetSupportedAPIList/v1/", &[("key", key)]);
        let f = self.fetch(Bucket::WebApi, &url, notify).await?;
        match f.status {
            200 => Ok(()),
            401 | 403 => Err(
                "Steam didn't accept this key. Make sure you copied all 32 characters from the key page."
                    .to_string(),
            ),
            s => Err(format!(
                "Steam gave an unexpected answer (code {s}). Wait a minute and try again."
            )),
        }
    }

    pub async fn resolve_vanity(
        &self,
        key: &str,
        name: &str,
        notify: Notify<'_>,
    ) -> Result<String, String> {
        let url = webapi_url(
            "/ISteamUser/ResolveVanityURL/v1/",
            &[("key", key), ("vanityurl", name)],
        );
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, notify).await?)?;
        let resp = &v["response"];
        match resp["success"].as_u64() {
            Some(1) => resp["steamid"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "Steam's answer was missing the profile ID. Try again.".to_string()),
            _ => Err(format!(
                "No Steam profile uses the custom URL \"{name}\". Check the spelling, or paste your full profile link."
            )),
        }
    }

    pub async fn player_summary(
        &self,
        key: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<ProfileView, String> {
        let url = webapi_url(
            "/ISteamUser/GetPlayerSummaries/v2/",
            &[("key", key), ("steamids", steam_id)],
        );
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, notify).await?)?;
        parse_player_summary(&v, steam_id).ok_or_else(|| {
            "Steam couldn't find that profile. Check the link and try again.".to_string()
        })
    }

    pub async fn owned_games(
        &self,
        key: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<OwnedGames, String> {
        let url = webapi_url(
            "/IPlayerService/GetOwnedGames/v1/",
            &[
                ("key", key),
                ("steamid", steam_id),
                ("include_appinfo", "true"),
                ("include_played_free_games", "true"),
                ("skip_unvetted_apps", "false"),
            ],
        );
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, notify).await?)?;
        Ok(parse_owned_games(&v))
    }

    pub async fn badges(
        &self,
        key: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<Badges, String> {
        let url = webapi_url(
            "/IPlayerService/GetBadges/v1/",
            &[("key", key), ("steamid", steam_id)],
        );
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, notify).await?)?;
        Ok(parse_badges(&v))
    }

    /// One page (up to 2,000 items) of the Steam Community inventory, which holds
    /// cards, emoticons, backgrounds, boosters and gems.
    pub async fn inventory_page(
        &self,
        steam_id: &str,
        start: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let mut params = vec![("l", "english"), ("count", "2000")];
        if let Some(s) = start {
            params.push(("start_assetid", s));
        }
        let url = community_url(&format!("/inventory/{steam_id}/753/6"), &params);
        self.fetch(Bucket::Community, &url, notify).await
    }
}

pub fn webapi_url(path: &str, params: &[(&str, &str)]) -> Url {
    build_url(WEBAPI, path, params)
}

pub fn community_url(path: &str, params: &[(&str, &str)]) -> Url {
    build_url(COMMUNITY, path, params)
}

fn build_url(base: &str, path: &str, params: &[(&str, &str)]) -> Url {
    let mut url = Url::parse(base).expect("valid base url");
    url.set_path(path);
    if !params.is_empty() {
        let mut query = url.query_pairs_mut();
        for (k, v) in params {
            query.append_pair(k, v);
        }
    }
    url
}

fn expect_json(f: &Fetched) -> Result<Value, String> {
    if f.status == 401 || f.status == 403 {
        return Err("Steam refused the request. Check your API key in Settings.".to_string());
    }
    if !f.is_success() {
        return Err(format!(
            "Steam gave an unexpected answer (code {}). Wait a minute and try again.",
            f.status
        ));
    }
    f.json().ok_or_else(|| {
        "Steam sent an answer the app couldn't read. Try again in a minute.".to_string()
    })
}

/// Plain-language network errors. Deliberately never includes the URL,
/// because Web API URLs contain the API key.
fn network_message(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "Steam took too long to answer. Check your internet connection and try again.".to_string()
    } else if err.is_connect() {
        "Couldn't connect to Steam. Check your internet connection and try again.".to_string()
    } else if err.is_body() || err.is_decode() {
        "Steam's answer was cut off. Try again in a minute.".to_string()
    } else {
        "Something went wrong talking to Steam. Try again in a minute.".to_string()
    }
}

pub fn parse_player_summary(v: &Value, steam_id: &str) -> Option<ProfileView> {
    let p = v["response"]["players"].as_array()?.first()?;
    Some(ProfileView {
        steam_id: p["steamid"].as_str().unwrap_or(steam_id).to_string(),
        persona_name: p["personaname"]
            .as_str()
            .unwrap_or("Steam user")
            .to_string(),
        avatar_url: p["avatarfull"]
            .as_str()
            .or_else(|| p["avatarmedium"].as_str())
            .map(str::to_string),
        profile_url: p["profileurl"].as_str().map(str::to_string),
        is_public: p["communityvisibilitystate"].as_u64() == Some(3),
    })
}

pub fn parse_owned_games(v: &Value) -> OwnedGames {
    let resp = &v["response"];
    let Some(list) = resp["games"].as_array() else {
        // A private library comes back as an empty object; a public empty one has game_count 0.
        return OwnedGames {
            visible: resp.get("game_count").is_some(),
            games: Vec::new(),
        };
    };
    let games = list
        .iter()
        .filter_map(|g| {
            Some(OwnedGame {
                appid: g["appid"].as_u64()?,
                name: g["name"].as_str().unwrap_or_default().to_string(),
                playtime_minutes: g["playtime_forever"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    OwnedGames {
        visible: true,
        games,
    }
}

pub fn parse_badges(v: &Value) -> Badges {
    let resp = &v["response"];
    let list = resp["badges"]
        .as_array()
        .map(|badges| {
            badges
                .iter()
                .map(|b| Badge {
                    badge_id: b["badgeid"].as_u64(),
                    appid: b["appid"].as_u64(),
                    level: b["level"].as_u64().unwrap_or(0),
                    foil: b["border_color"].as_u64() == Some(1),
                })
                .collect()
        })
        .unwrap_or_default();
    Badges {
        visible: resp.get("badges").is_some() || resp.get("player_level").is_some(),
        player_level: resp["player_level"].as_u64(),
        list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_are_encoded() {
        let url = community_url(
            "/market/search/render/",
            &[("category_753_Game[]", "tag_app_440"), ("query", "")],
        );
        assert_eq!(
            url.as_str(),
            "https://steamcommunity.com/market/search/render/?category_753_Game%5B%5D=tag_app_440&query="
        );
        let url = webapi_url("/ISteamUser/ResolveVanityURL/v1/", &[("vanityurl", "a b")]);
        assert!(url.as_str().ends_with("vanityurl=a+b"));
    }

    #[test]
    fn player_summary() {
        let v = json!({"response": {"players": [{
            "steamid": "76561197960287930", "personaname": "Wolf",
            "avatarfull": "https://avatars.steamstatic.com/x_full.jpg",
            "profileurl": "https://steamcommunity.com/id/wolf/", "communityvisibilitystate": 3
        }]}});
        let p = parse_player_summary(&v, "x").unwrap();
        assert_eq!(p.persona_name, "Wolf");
        assert!(p.is_public);
        assert!(parse_player_summary(&json!({"response": {"players": []}}), "x").is_none());
    }

    #[test]
    fn owned_games_visibility() {
        assert!(!parse_owned_games(&json!({"response": {}})).visible);
        let empty = parse_owned_games(&json!({"response": {"game_count": 0}}));
        assert!(empty.visible && empty.games.is_empty());
        let some = parse_owned_games(&json!({"response": {"game_count": 2, "games": [
            {"appid": 440, "name": "Team Fortress 2", "playtime_forever": 90},
            {"appid": 570, "name": "Dota 2"}
        ]}}));
        assert_eq!(some.games.len(), 2);
        assert_eq!(some.games[0].playtime_minutes, 90);
    }

    #[test]
    fn badges() {
        let b = parse_badges(&json!({"response": {"badges": [
            {"badgeid": 1, "appid": 440, "level": 5, "border_color": 0},
            {"badgeid": 1, "appid": 440, "level": 1, "border_color": 1},
            {"badgeid": 13, "level": 3}
        ], "player_level": 42}}));
        assert!(b.visible);
        assert_eq!(b.player_level, Some(42));
        assert_eq!(b.list.iter().filter(|x| x.appid.is_some()).count(), 2);
        assert_eq!(b.list.iter().filter(|x| x.foil).count(), 1);
    }
}
