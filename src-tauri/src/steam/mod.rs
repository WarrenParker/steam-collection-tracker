//! Talking to Steam.
//!
//! Every request goes through one paced client. Steam limits how often an
//! internet connection may call the Community pages, inventory and Market, and
//! asking again after a refusal can stretch a block out for hours. So each kind
//! of request has its own minimum gap, and when Steam refuses one (HTTP 429) the
//! client stops sending that kind of request for a while instead of retrying.

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
pub const STORE: &str = "https://store.steampowered.com";

/// Used for Steam's web pages until the interface reports WebView2's own identity.
const FALLBACK_BROWSER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";

/// How long the app leaves a kind of request alone after Steam refuses it.
pub const BLOCK_MINUTES: u64 = 30;

/// Called with short, plain-language status messages (for example when waiting).
pub type Notify<'a> = &'a (dyn Fn(String) + Send + Sync);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Bucket {
    /// api.steampowered.com: generous limits.
    WebApi,
    /// steamcommunity.com pages (badges, card sets).
    Community,
    /// steamcommunity.com/inventory: strict, and refused to anonymous apps on some connections.
    Inventory,
    /// steamcommunity.com/market: the strictest.
    Market,
    /// store.steampowered.com.
    Store,
}

impl Bucket {
    fn min_interval(self) -> Duration {
        match self {
            Bucket::WebApi => Duration::from_millis(300),
            Bucket::Community => Duration::from_millis(3500),
            Bucket::Inventory => Duration::from_millis(4000),
            Bucket::Market => Duration::from_millis(4000),
            Bucket::Store => Duration::from_millis(1000),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub body: String,
    pub elapsed_ms: u64,
    pub final_url: String,
    /// True when the request wasn't sent because Steam refused this kind of
    /// request recently. `status` is then 429.
    pub skipped: bool,
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
    pub xp: Option<i64>,
    /// Unix time the badge was crafted or earned.
    pub completion_time: Option<i64>,
}

/// How a Web API call identifies itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth<'a> {
    /// No key or token: public calls.
    None,
    /// The user's Web API key.
    Key(&'a str),
    /// The signed-in user's token.
    Token(&'a str),
}

impl Auth<'_> {
    pub fn name(self) -> &'static str {
        match self {
            Auth::None => "none",
            Auth::Key(_) => "key",
            Auth::Token(_) => "token",
        }
    }
}

pub struct SteamClient {
    http: reqwest::Client,
    last: Mutex<HashMap<Bucket, Instant>>,
    /// (kind of request, signed in) -> refused until.
    blocked: std::sync::Mutex<HashMap<(Bucket, bool), Instant>>,
    requests: AtomicU32,
    browser_agent: std::sync::Mutex<Option<String>>,
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
            blocked: std::sync::Mutex::new(HashMap::new()),
            requests: AtomicU32::new(0),
            browser_agent: std::sync::Mutex::new(None),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Total requests sent to Steam since the app started.
    pub fn requests(&self) -> u32 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Whether Steam refused this kind of request recently.
    pub fn is_blocked(&self, bucket: Bucket, signed_in: bool) -> bool {
        let blocked = self.blocked.lock().unwrap_or_else(|e| e.into_inner());
        blocked
            .get(&(bucket, signed_in))
            .is_some_and(|until| Instant::now() < *until)
    }

    fn block(&self, bucket: Bucket, signed_in: bool) {
        let mut blocked = self.blocked.lock().unwrap_or_else(|e| e.into_inner());
        blocked.insert(
            (bucket, signed_in),
            Instant::now() + Duration::from_secs(BLOCK_MINUTES * 60),
        );
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

    /// Sends a paced GET request, optionally with Steam sign-in cookies.
    /// Non-success HTTP codes are returned as data so callers can explain them;
    /// only network failures become errors.
    pub async fn fetch(
        &self,
        bucket: Bucket,
        url: &Url,
        cookie: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        self.send(bucket, url, None, cookie, notify).await
    }

    /// Sends a paced POST with a form body. Some Web API methods only accept POST.
    pub async fn post_form(
        &self,
        bucket: Bucket,
        url: &Url,
        form: &[(&str, &str)],
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        self.send(bucket, url, Some(form), None, notify).await
    }

    /// The browser identity used for Steam's web pages and Market, which turn
    /// away requests that don't look like they come from a browser. The app
    /// embeds a browser (WebView2), and the interface reports its real identity
    /// at startup; this is the fallback until it does.
    pub fn set_browser_agent(&self, agent: &str) {
        let agent = agent.trim();
        if agent.starts_with("Mozilla/") && agent.len() < 400 {
            *self.browser_agent.lock().unwrap_or_else(|e| e.into_inner()) = Some(agent.to_string());
        }
    }

    fn browser_agent(&self) -> String {
        self.browser_agent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| FALLBACK_BROWSER_AGENT.to_string())
    }

    async fn send(
        &self,
        bucket: Bucket,
        url: &Url,
        form: Option<&[(&str, &str)]>,
        cookie: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let signed_in = cookie.is_some();
        if bucket != Bucket::WebApi && self.is_blocked(bucket, signed_in) {
            return Ok(Fetched {
                status: 429,
                body: String::new(),
                elapsed_ms: 0,
                final_url: url.to_string(),
                skipped: true,
            });
        }

        // Only the Web API is retried after a refusal; the Community, inventory
        // and Market limits get longer if you keep asking.
        let api_waits: &[u64] = &[5, 15];
        let mut attempt = 0usize;
        let mut retried_network = false;
        loop {
            self.pace(bucket).await;
            self.requests.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let mut request = match form {
                Some(fields) => self.http.post(url.clone()).form(fields),
                None => self.http.get(url.clone()),
            };
            request = self.browser_headers(bucket, request);
            if let Some(c) = cookie {
                request = request.header(reqwest::header::COOKIE, c);
            }
            match request.send().await {
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
                    if status == 429 {
                        if bucket == Bucket::WebApi && attempt < api_waits.len() {
                            let wait = api_waits[attempt];
                            attempt += 1;
                            notify(format!(
                                "Steam asked the app to slow down. Waiting {wait} seconds…"
                            ));
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                            continue;
                        }
                        if bucket != Bucket::WebApi {
                            self.block(bucket, signed_in);
                        }
                    }
                    let final_url = resp.url().to_string();
                    let body = resp.text().await.map_err(|e| network_message(&e))?;
                    return Ok(Fetched {
                        status,
                        body,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        final_url,
                        skipped: false,
                    });
                }
            }
        }
    }

    /// Steam's web pages and Market get the headers a browser would send.
    fn browser_headers(
        &self,
        bucket: Bucket,
        request: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, REFERER, USER_AGENT};
        match bucket {
            Bucket::WebApi => request,
            Bucket::Market => request
                .header(USER_AGENT, self.browser_agent())
                .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
                .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
                .header(
                    REFERER,
                    "https://steamcommunity.com/market/search?appid=753",
                )
                .header("X-Requested-With", "XMLHttpRequest"),
            Bucket::Community | Bucket::Inventory | Bucket::Store => request
                .header(USER_AGENT, self.browser_agent())
                .header(
                    ACCEPT,
                    "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8",
                )
                .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9"),
        }
    }

    pub async fn check_api_key(&self, key: &str, notify: Notify<'_>) -> Result<(), String> {
        let url = webapi_url("/ISteamWebAPIUtil/GetSupportedAPIList/v1/", &[("key", key)]);
        let f = self.fetch(Bucket::WebApi, &url, None, notify).await?;
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
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, None, notify).await?)?;
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
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, None, notify).await?)?;
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
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, None, notify).await?)?;
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
        let v = expect_json(&self.fetch(Bucket::WebApi, &url, None, notify).await?)?;
        Ok(parse_badges(&v))
    }

    /// One page (up to 2,000 items) of the Steam Community inventory, which holds
    /// cards, emoticons, backgrounds, boosters and gems. Pass the sign-in cookie
    /// to read your own inventory as yourself.
    pub async fn inventory_page(
        &self,
        steam_id: &str,
        start: Option<&str>,
        cookie: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let mut params = vec![("l", "english"), ("count", "2000")];
        if let Some(s) = start {
            params.push(("start_assetid", s));
        }
        let url = community_url(&format!("/inventory/{steam_id}/753/6"), &params);
        self.fetch(Bucket::Inventory, &url, cookie, notify).await
    }

    /// The same inventory through Steam's newer Web API, which needs a sign-in
    /// token but has far more generous limits. The answer's `response` object has
    /// the same assets/descriptions layout as the community inventory.
    pub async fn inventory_api_page(
        &self,
        access_token: &str,
        steam_id: &str,
        start: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let mut params = vec![
            ("access_token", access_token),
            ("steamid", steam_id),
            ("appid", "753"),
            ("contextid", "6"),
            ("get_descriptions", "true"),
            ("language", "english"),
            ("count", "2000"),
        ];
        if let Some(s) = start {
            params.push(("start_assetid", s));
        }
        let url = webapi_url(
            "/IEconService/GetInventoryItemsWithDescriptions/v1/",
            &params,
        );
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }
}

impl SteamClient {
    /// Store details for a batch of games, including whether each has trading cards.
    pub async fn store_items(
        &self,
        appids: &[u64],
        key: Option<&str>,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let input = crate::catalog::store::request_json(appids);
        let mut params = vec![("input_json", input.as_str())];
        if let Some(k) = key {
            params.push(("key", k));
        }
        let url = webapi_url("/IStoreBrowseService/GetItems/v1/", &params);
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    /// One page of the Points Shop items for a batch of games.
    /// With `include_unbuyable`, items that can't be bought directly (sale
    /// rewards, retired items) are listed too.
    pub async fn reward_items(
        &self,
        appids: &[u64],
        cursor: Option<&str>,
        include_unbuyable: bool,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let ids: Vec<(String, String)> = appids
            .iter()
            .enumerate()
            .map(|(i, a)| (format!("appids[{i}]"), a.to_string()))
            .collect();
        let count = crate::catalog::points::PAGE_SIZE.to_string();
        let mut params: Vec<(&str, &str)> =
            ids.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        params.push(("count", count.as_str()));
        params.push(("language", "english"));
        if include_unbuyable {
            params.push(("include_direct_purchase_disabled", "true"));
        }
        if let Some(c) = cursor {
            params.push(("cursor", c));
        }
        let url = webapi_url("/ILoyaltyRewardsService/QueryRewardItems/v1/", &params);
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    /// Every game with a Points Shop page, flagging Steam sale events.
    pub async fn eligible_apps(&self, notify: Notify<'_>) -> Result<Fetched, String> {
        let url = webapi_url("/ILoyaltyRewardsService/GetEligibleApps/v1/", &[]);
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    /// The signed-in user's profile items: backgrounds, frames, avatars and so on.
    pub async fn profile_items_owned(
        &self,
        token: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let url = webapi_url(
            "/IPlayerService/GetProfileItemsOwned/v1/",
            &[
                ("access_token", token),
                ("steamid", steam_id),
                ("language", "english"),
            ],
        );
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    pub async fn family_group(
        &self,
        token: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let url = webapi_url(
            "/IFamilyGroupsService/GetFamilyGroupForUser/v1/",
            &[("access_token", token), ("steamid", steam_id)],
        );
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    /// Games other family members share, without the user's own.
    pub async fn family_library(
        &self,
        token: &str,
        group_id: &str,
        steam_id: &str,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let url = webapi_url(
            "/IFamilyGroupsService/GetSharedLibraryApps/v1/",
            &[
                ("access_token", token),
                ("family_groupid", group_id),
                ("steamid", steam_id),
                ("include_own", "false"),
                ("include_excluded", "false"),
                ("include_free", "false"),
                ("language", "english"),
            ],
        );
        self.fetch(Bucket::WebApi, &url, None, notify).await
    }

    /// Achievement progress (unlocked and total) for a batch of games. This
    /// method only accepts POST.
    pub async fn achievements_progress(
        &self,
        auth: Auth<'_>,
        steam_id: &str,
        appids: &[u64],
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let ids: Vec<(String, String)> = appids
            .iter()
            .enumerate()
            .map(|(i, a)| (format!("appids[{i}]"), a.to_string()))
            .collect();
        let mut form: Vec<(&str, &str)> = vec![("steamid", steam_id), ("language", "english")];
        match auth {
            Auth::None => {}
            Auth::Key(k) => form.push(("key", k)),
            Auth::Token(t) => form.push(("access_token", t)),
        }
        form.extend(ids.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let url = webapi_url("/IPlayerService/GetAchievementsProgress/v1/", &[]);
        self.post_form(Bucket::WebApi, &url, &form, notify).await
    }

    /// A game's card page, read without sign-in.
    pub async fn card_page(
        &self,
        steam_id: &str,
        appid: u64,
        notify: Notify<'_>,
    ) -> Result<Fetched, String> {
        let url = community_url(
            &format!("/profiles/{steam_id}/gamecards/{appid}/"),
            &[("l", "english")],
        );
        self.fetch(Bucket::Community, &url, None, notify).await
    }
}

pub fn store_url(path: &str, params: &[(&str, &str)]) -> Url {
    build_url(STORE, path, params)
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

/// (game, unlocked, total) from a `GetAchievementsProgress` answer. `None`
/// means it wasn't an achievements answer.
pub fn parse_achievements(v: &Value) -> Option<Vec<(u64, u64, u64)>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let list = resp["achievement_progress"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    Some(
        list.iter()
            .filter_map(|a| {
                let n = |k: &str| inventory::as_u64(&a[k]);
                Some((
                    n("appid")?,
                    n("unlocked").unwrap_or(0),
                    n("total").unwrap_or(0),
                ))
            })
            .collect(),
    )
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
                    xp: b["xp"].as_i64(),
                    completion_time: b["completion_time"].as_i64(),
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
    fn achievements() {
        let v = json!({"response": {"achievement_progress": [
            {"appid": 440, "unlocked": 12, "total": 520, "percentage": 2.3, "all_unlocked": false},
            {"appid": 570, "unlocked": 0, "total": 0},
            {"unlocked": 3}
        ]}});
        assert_eq!(
            parse_achievements(&v).unwrap(),
            vec![(440, 12, 520), (570, 0, 0)]
        );
        assert_eq!(
            parse_achievements(&json!({"response": {}})).unwrap(),
            vec![]
        );
        assert!(parse_achievements(&json!({})).is_none());
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
