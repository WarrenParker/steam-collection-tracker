//! "Test the Steam connection": a short, safe check of every Steam data source
//! the app relies on. It runs on the user's own PC (Steam can't be reached from
//! where the app is built), times each request, records the shape of each
//! answer, and writes a report the user can send back. Requests use the same
//! pacing as real scans, and once Steam refuses a kind of request the test
//! doesn't ask again, so it can't make a block longer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Instant;

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};

use crate::catalog::{cardpage, defs, kind_label, library, points, store, CatalogItem};
use crate::housekeeping;
use crate::jsonutil::int;
use crate::session::{self, Session};
use crate::state::AppState;
use crate::steam::{
    community_url, inventory, parse_badges, parse_owned_games, parse_player_summary, webapi_url,
    Auth, Bucket, Fetched, Notify, BLOCK_MINUTES,
};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Running,
    Ok,
    Warn,
    Fail,
    Skipped,
}

impl StepStatus {
    fn tag(self) -> &'static str {
        match self {
            StepStatus::Pending => "[    ]",
            StepStatus::Running => "[ .. ]",
            StepStatus::Ok => "[ OK ]",
            StepStatus::Warn => "[WARN]",
            StepStatus::Fail => "[FAIL]",
            StepStatus::Skipped => "[SKIP]",
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct DiagStep {
    pub key: String,
    pub label: String,
    pub status: StepStatus,
    pub summary: String,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct DiagnosticsResult {
    pub report_path: String,
    pub steps: Vec<DiagStep>,
    pub estimate: String,
    pub total_seconds: u64,
}

const PLAN: &[(&str, &str)] = &[
    ("api_key", "Steam Web API key"),
    ("sign_in", "Steam sign-in"),
    ("owned_games", "Owned games"),
    ("badges", "Badges"),
    ("inventory", "Inventory"),
    ("market_search", "Market items for one game"),
    ("price_history", "Price history for one item"),
    ("item_defs", "Steam's item list for one game"),
    ("store_cards", "Which games have trading cards"),
    ("card_page", "A game's card set page"),
    ("points_catalog", "Points Shop items for a few games"),
    ("points_owned", "Points Shop items you own"),
    ("emoticons", "Emoticons you own"),
    ("family", "Steam Family library"),
    ("badge_page", "Badges page"),
];

const NEEDS_SIGN_IN: &str = "Needs Steam sign-in.";

pub fn plan() -> Vec<DiagStep> {
    PLAN.iter()
        .map(|(key, label)| DiagStep {
            key: key.to_string(),
            label: label.to_string(),
            status: StepStatus::Pending,
            summary: String::new(),
            elapsed_ms: 0,
        })
        .collect()
}

struct Run<'a> {
    steps: Vec<DiagStep>,
    details: Map<String, Value>,
    emit: &'a (dyn Fn(DiagStep) + Send + Sync),
}

impl Run<'_> {
    fn start(&mut self, key: &str) -> Instant {
        if let Some(step) = self.steps.iter_mut().find(|s| s.key == key) {
            step.status = StepStatus::Running;
            let copy = step.clone();
            (self.emit)(copy);
        }
        Instant::now()
    }

    fn finish(
        &mut self,
        key: &str,
        started: Instant,
        status: StepStatus,
        summary: impl Into<String>,
        detail: Value,
    ) {
        let elapsed = started.elapsed().as_millis() as u64;
        let summary = summary.into();
        if let Some(step) = self.steps.iter_mut().find(|s| s.key == key) {
            step.status = status;
            step.summary = summary;
            step.elapsed_ms = elapsed;
            let copy = step.clone();
            (self.emit)(copy);
        }
        self.details.insert(key.to_string(), detail);
    }
}

fn skipped_text() -> String {
    format!(
        "Skipped: Steam refused this kind of request in the last {BLOCK_MINUTES} minutes, and asking again could make the block longer."
    )
}

fn local_time(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Reads the first page of an inventory answer, if it is one.
fn first_inventory_page(f: &Fetched, api: bool) -> Option<(inventory::Tally, inventory::PageInfo)> {
    if !f.is_success() {
        return None;
    }
    let v = f.json()?;
    let v = if api { v.get("response")?.clone() } else { v };
    if v.get("assets").is_none() && v.get("total_inventory_count").is_none() {
        return None;
    }
    let mut tally = inventory::Tally::default();
    let info = inventory::tally_page(&v, &mut tally);
    Some((tally, info))
}

pub async fn run(
    app: &AppHandle,
    notify: Notify<'_>,
    emit: &(dyn Fn(DiagStep) + Send + Sync),
) -> Result<DiagnosticsResult, String> {
    let state = app.state::<AppState>();
    let key = state.api_key()?;
    let sid = state
        .settings()
        .profile
        .map(|p| p.steam_id)
        .ok_or_else(|| "Add your Steam profile in Settings first.".to_string())?;
    let steam = &state.steam;
    let started_all = Instant::now();
    let requests_before = steam.requests();
    let mut secrets: Vec<String> = vec![key.clone()];
    let mut run = Run {
        steps: plan(),
        details: Map::new(),
        emit,
    };

    // 1. API key and profile
    let t = run.start("api_key");
    let url = webapi_url(
        "/ISteamUser/GetPlayerSummaries/v2/",
        &[("key", key.as_str()), ("steamids", sid.as_str())],
    );
    match steam.fetch(Bucket::WebApi, &url, None, notify).await {
        Ok(f) => {
            let v = f.json().unwrap_or(Value::Null);
            let (status, text) = match (f.status, parse_player_summary(&v, &sid)) {
                (200, Some(p)) if p.is_public => (
                    StepStatus::Ok,
                    format!("Key works. Profile \"{}\" is public.", p.persona_name),
                ),
                (200, Some(p)) => (
                    StepStatus::Warn,
                    format!("Key works, but profile \"{}\" is private.", p.persona_name),
                ),
                (200, None) => (
                    StepStatus::Warn,
                    "Key works, but Steam didn't return the profile.".to_string(),
                ),
                (code, _) => (
                    StepStatus::Fail,
                    format!("Steam answered with code {code}."),
                ),
            };
            run.finish("api_key", t, status, text, http_detail(&f, 4));
        }
        Err(e) => run.finish("api_key", t, StepStatus::Fail, e, Value::Null),
    }

    // 2. Steam sign-in
    let t = run.start("sign_in");
    let mut session: Option<Session> = None;
    if state.settings().signed_in_steam_id.is_none() {
        run.finish(
            "sign_in",
            t,
            StepStatus::Warn,
            "Not signed in. Steam needs a sign-in for your inventory, prices and Points Shop items.",
            Value::Null,
        );
    } else {
        match session::current(app).await {
            Ok(Some(s)) => {
                secrets.extend(s.secrets());
                let same_account = s.steam_id == sid;
                let until = s
                    .expires_at
                    .map(|e| format!(" The sign-in is valid until {}.", local_time(e)))
                    .unwrap_or_default();
                let text = if same_account {
                    format!("Signed in. Web API token from {}.{until}", s.token_source)
                } else {
                    "Signed in, but as a different account than this profile.".to_string()
                };
                let detail = json!({
                    "same_account": same_account,
                    "token_source": s.token_source,
                    "has_webapi_token": s.webapi_token.is_some(),
                    "has_store_cookie": s.store_cookie.is_some(),
                    "has_session_id": s.session_id.is_some(),
                    "expires_at": s.expires_at,
                });
                run.finish(
                    "sign_in",
                    t,
                    if same_account {
                        StepStatus::Ok
                    } else {
                        StepStatus::Warn
                    },
                    text,
                    detail,
                );
                if same_account {
                    session = Some(s);
                }
            }
            Ok(None) => run.finish(
                "sign_in",
                t,
                StepStatus::Warn,
                "Not signed in.",
                Value::Null,
            ),
            Err(e) => run.finish("sign_in", t, StepStatus::Fail, e, Value::Null),
        }
    }
    let token = session.as_ref().and_then(|s| s.access_token());
    let cookie = session.as_ref().map(|s| s.community_header());

    // 3. Owned games
    let t = run.start("owned_games");
    let url = webapi_url(
        "/IPlayerService/GetOwnedGames/v1/",
        &[
            ("key", key.as_str()),
            ("steamid", sid.as_str()),
            ("include_appinfo", "true"),
            ("include_played_free_games", "true"),
            ("skip_unvetted_apps", "false"),
        ],
    );
    let mut owned_count = 0usize;
    let mut owned_apps: Vec<u64> = Vec::new();
    match steam.fetch(Bucket::WebApi, &url, None, notify).await {
        Ok(f) => {
            let owned = parse_owned_games(&f.json().unwrap_or(Value::Null));
            owned_count = owned.games.len();
            owned_apps = owned.games.iter().map(|g| g.appid).collect();
            let (status, text) = if !f.is_success() {
                (
                    StepStatus::Fail,
                    format!("Steam answered with code {}.", f.status),
                )
            } else if !owned.visible {
                (
                    StepStatus::Warn,
                    "Game details are private, so owned games can't be read.".to_string(),
                )
            } else {
                (
                    StepStatus::Ok,
                    format!("{} games found.", fmt_num(owned_count as u64)),
                )
            };
            run.finish("owned_games", t, status, text, http_detail(&f, 3));
        }
        Err(e) => run.finish("owned_games", t, StepStatus::Fail, e, Value::Null),
    }

    // 4. Badges
    let t = run.start("badges");
    let url = webapi_url(
        "/IPlayerService/GetBadges/v1/",
        &[("key", key.as_str()), ("steamid", sid.as_str())],
    );
    let mut badge_apps: Vec<(u64, u64)> = Vec::new();
    match steam.fetch(Bucket::WebApi, &url, None, notify).await {
        Ok(f) => {
            let badges = parse_badges(&f.json().unwrap_or(Value::Null));
            for b in &badges.list {
                if let (Some(app_id), false) = (b.appid, b.foil) {
                    badge_apps.push((app_id, b.level));
                }
            }
            let foil = badges
                .list
                .iter()
                .filter(|b| b.appid.is_some() && b.foil)
                .count();
            let (status, text) = if !f.is_success() {
                (
                    StepStatus::Fail,
                    format!("Steam answered with code {}.", f.status),
                )
            } else if !badges.visible {
                (
                    StepStatus::Warn,
                    "Badges are hidden on this profile.".to_string(),
                )
            } else {
                (
                    StepStatus::Ok,
                    format!(
                        "{} badges: {} game badges and {} foil badges.",
                        fmt_num(badges.list.len() as u64),
                        fmt_num(badge_apps.len() as u64),
                        fmt_num(foil as u64)
                    ),
                )
            };
            run.finish("badges", t, status, text, http_detail(&f, 4));
        }
        Err(e) => run.finish("badges", t, StepStatus::Fail, e, Value::Null),
    }

    // 5. Inventory: the signed-in Web API first, then the community inventory
    // (signed in if possible). Only the first page is read.
    let t = run.start("inventory");
    let mut attempts = Map::new();
    let mut inventory_apps: Vec<u64> = Vec::new();
    let mut read_by: Option<(&str, inventory::Tally, inventory::PageInfo)> = None;
    let mut last_status: Option<(u16, bool)> = None;
    if let Some(tok) = token.as_deref() {
        match steam.inventory_api_page(tok, &sid, None, notify).await {
            Ok(f) => {
                attempts.insert("web_api".into(), http_detail(&f, 4));
                last_status = Some((f.status, f.skipped));
                if let Some((tally, info)) = first_inventory_page(&f, true) {
                    read_by = Some(("Steam's Web API (signed in)", tally, info));
                }
            }
            Err(e) => {
                attempts.insert("web_api".into(), json!({ "error": e }));
            }
        }
    }
    if read_by.is_none() {
        let route = if cookie.is_some() {
            "community_signed_in"
        } else {
            "community"
        };
        match steam
            .inventory_page(&sid, None, cookie.as_deref(), notify)
            .await
        {
            Ok(f) => {
                attempts.insert(route.into(), http_detail(&f, 4));
                last_status = Some((f.status, f.skipped));
                if let Some((tally, info)) = first_inventory_page(&f, false) {
                    let name = if cookie.is_some() {
                        "the community inventory (signed in)"
                    } else {
                        "the community inventory"
                    };
                    read_by = Some((name, tally, info));
                }
            }
            Err(e) => {
                attempts.insert(route.into(), json!({ "error": e }));
            }
        }
    }
    match read_by {
        Some((name, tally, info)) => {
            inventory_apps = tally.apps.iter().copied().collect();
            let samples: Map<String, Value> = tally
                .samples
                .iter()
                .map(|(class_id, d)| (format!("item_class_{class_id}"), shape(d, 4)))
                .collect();
            attempts.insert("item_samples".into(), Value::Object(samples));
            let types: Vec<String> = tally
                .by_type
                .iter()
                .map(|(label, (count, _))| format!("{label} {count}"))
                .collect();
            run.finish(
                "inventory",
                t,
                StepStatus::Ok,
                format!(
                    "Read through {name}: {} items in total. The first page had {} items from {} games ({}).",
                    fmt_num(info.total.unwrap_or(info.assets as u64)),
                    fmt_num(info.assets as u64),
                    fmt_num(tally.apps.len() as u64),
                    types.join(", ")
                ),
                Value::Object(attempts),
            );
        }
        None => {
            let (status, text) = match last_status {
                Some((_, true)) => (StepStatus::Skipped, skipped_text()),
                Some((429, false)) if session.is_none() => (
                    StepStatus::Warn,
                    "Steam refused to share the inventory without a sign-in (code 429). Sign in with Steam to read it.".to_string(),
                ),
                Some((429, false)) => (
                    StepStatus::Fail,
                    "Steam refused the inventory even signed in (code 429).".to_string(),
                ),
                Some((403, false)) if session.is_none() => (
                    StepStatus::Warn,
                    "The inventory is private (code 403). Sign in, or set Inventory to Public in Steam's privacy settings.".to_string(),
                ),
                Some((code, _)) => (
                    StepStatus::Fail,
                    format!("The inventory couldn't be read (code {code})."),
                ),
                None => (StepStatus::Fail, "The inventory couldn't be reached.".to_string()),
            };
            run.finish("inventory", t, status, text, Value::Object(attempts));
        }
    }

    // A game to test the per-game sources with: the highest badge, else a game from the inventory.
    let sample_app = badge_apps
        .iter()
        .max_by_key(|(_, level)| *level)
        .map(|(app_id, _)| *app_id)
        .or_else(|| inventory_apps.iter().copied().find(|a| *a != 753))
        .unwrap_or(440);
    run.details
        .insert("sample_app".to_string(), json!(sample_app));

    // 6. Every market item for the game, in one request. This goes first among
    // the per-game checks: whether the market answers again is the open question.
    let t = run.start("market_search");
    let game_tag = format!("tag_app_{sample_app}");
    let url = community_url(
        "/market/search/render/",
        &[
            ("query", ""),
            ("start", "0"),
            ("count", "100"),
            ("search_descriptions", "0"),
            ("sort_column", "name"),
            ("sort_dir", "asc"),
            ("appid", "753"),
            ("category_753_Game[]", game_tag.as_str()),
            ("norender", "1"),
            ("currency", "1"),
        ],
    );
    let mut sample_hash: Option<String> = None;
    let mut market_ms: Option<u64> = None;
    let signed_note = if cookie.is_some() {
        " (signed in)"
    } else {
        " (not signed in)"
    };
    match steam
        .fetch(Bucket::Market, &url, cookie.as_deref(), notify)
        .await
    {
        Ok(f) if f.skipped => run.finish(
            "market_search",
            t,
            StepStatus::Skipped,
            skipped_text(),
            Value::Null,
        ),
        Ok(f) => {
            market_ms = Some(f.elapsed_ms);
            let detail = http_detail(&f, 4);
            match f.json() {
                Some(v) if f.is_success() && v["success"].as_bool() == Some(true) => {
                    let results = v["results"].as_array().cloned().unwrap_or_default();
                    let total = v["total_count"].as_u64().unwrap_or(results.len() as u64);
                    let mut types: BTreeMap<String, usize> = BTreeMap::new();
                    for r in &results {
                        let kind = r["asset_description"]["type"].as_str().unwrap_or("?");
                        *types.entry(kind.to_string()).or_default() += 1;
                    }
                    sample_hash = results
                        .iter()
                        .find(|r| {
                            r["asset_description"]["type"]
                                .as_str()
                                .is_some_and(|k| k.ends_with("Trading Card"))
                        })
                        .or(results.first())
                        .and_then(|r| r["hash_name"].as_str())
                        .map(str::to_string);
                    let price = results
                        .first()
                        .and_then(|r| r["sell_price_text"].as_str())
                        .unwrap_or("unknown");
                    let kinds: Vec<String> =
                        types.iter().map(|(k, n)| format!("{k} {n}")).collect();
                    run.finish(
                        "market_search",
                        t,
                        StepStatus::Ok,
                        format!(
                            "{total} market items for app {sample_app}{signed_note}, for example priced at {price}. Types: {}.",
                            kinds.join(", ")
                        ),
                        detail,
                    );
                }
                _ if f.status == 429 => run.finish(
                    "market_search",
                    t,
                    if cookie.is_some() {
                        StepStatus::Fail
                    } else {
                        StepStatus::Warn
                    },
                    format!("Steam refused the market search{signed_note} (code 429)."),
                    detail,
                ),
                _ => run.finish(
                    "market_search",
                    t,
                    StepStatus::Fail,
                    format!(
                        "The market search didn't work{signed_note} (code {}).",
                        f.status
                    ),
                    detail,
                ),
            }
        }
        Err(e) => run.finish("market_search", t, StepStatus::Fail, e, Value::Null),
    }

    // 7. Price history: the price history API when signed in, otherwise the
    // graph data embedded in the item's market page.
    let t = run.start("price_history");
    match sample_hash.as_deref() {
        None => run.finish(
            "price_history",
            t,
            StepStatus::Skipped,
            "Skipped because the market search didn't return an item.",
            Value::Null,
        ),
        Some(hash) if cookie.is_some() => {
            let url = community_url(
                "/market/pricehistory/",
                &[
                    ("appid", "753"),
                    ("market_hash_name", hash),
                    ("currency", "1"),
                ],
            );
            match steam
                .fetch(Bucket::Market, &url, cookie.as_deref(), notify)
                .await
            {
                Ok(f) if f.skipped => run.finish(
                    "price_history",
                    t,
                    StepStatus::Skipped,
                    skipped_text(),
                    Value::Null,
                ),
                Ok(f) => {
                    let v = f.json();
                    let points = v
                        .as_ref()
                        .and_then(|v| v["prices"].as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    let mut detail = http_detail(&f, 2);
                    if let (Value::Object(map), Some(v)) = (&mut detail, &v) {
                        let prices = v["prices"].as_array();
                        map.insert("item".into(), json!(hash));
                        map.insert("price_prefix".into(), v["price_prefix"].clone());
                        map.insert(
                            "first_point".into(),
                            prices
                                .and_then(|p| p.first())
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        map.insert(
                            "last_point".into(),
                            prices
                                .and_then(|p| p.last())
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                    let (status, text) = if points > 0 {
                        (
                            StepStatus::Ok,
                            format!(
                                "Price history works signed in: {} data points for \"{hash}\".",
                                fmt_num(points as u64)
                            ),
                        )
                    } else {
                        (
                            StepStatus::Fail,
                            format!(
                                "The price history API didn't return prices (code {}).",
                                f.status
                            ),
                        )
                    };
                    run.finish("price_history", t, status, text, detail);
                }
                Err(e) => run.finish("price_history", t, StepStatus::Fail, e, Value::Null),
            }
        }
        Some(hash) => {
            let mut url = community_url("/market/listings/753", &[]);
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.push(hash);
            }
            match steam.fetch(Bucket::Market, &url, None, notify).await {
                Ok(f) if f.skipped => run.finish(
                    "price_history",
                    t,
                    StepStatus::Skipped,
                    skipped_text(),
                    Value::Null,
                ),
                Ok(f) => {
                    let line1 = extract_between(&f.body, "var line1=", "];");
                    let points = line1.map(|s| s.matches("+0\"").count()).unwrap_or(0);
                    let mut detail = html_detail(&f, &["var line1=", "Market_LoadOrderSpread"]);
                    if let Value::Object(map) = &mut detail {
                        map.insert("item".into(), json!(hash));
                        map.insert("price_points".into(), json!(points));
                        map.insert(
                            "first_points".into(),
                            json!(line1.map(|s| truncate(s, 200))),
                        );
                    }
                    let (status, text) = if points > 0 {
                        (
                            StepStatus::Ok,
                            format!("Price history works without sign-in: {} data points for \"{hash}\".", fmt_num(points as u64)),
                        )
                    } else {
                        (
                            StepStatus::Warn,
                            format!(
                                "No price history was found on the item's page (code {}).",
                                f.status
                            ),
                        )
                    };
                    run.finish("price_history", t, status, text, detail);
                }
                Err(e) => run.finish("price_history", t, StepStatus::Fail, e, Value::Null),
            }
        }
    }

    // 8. Steam's item definitions for the game: the fast route for the catalog.
    let t = run.start("item_defs");
    let mut auths: Vec<Auth> = vec![Auth::None, Auth::Key(key.as_str())];
    if let Some(tok) = token.as_deref() {
        auths.push(Auth::Token(tok));
    }
    let mut attempts = Map::new();
    let mut defs_result: Option<(Vec<CatalogItem>, &str)> = None;
    let mut defs_samples = Map::new();
    let mut defs_status: Option<u16> = None;
    for auth in &auths {
        match steam.item_definitions(sample_app, *auth, notify).await {
            Ok(f) => {
                attempts.insert(auth.name().into(), http_detail(&f, 3));
                defs_status = Some(f.status);
                if f.status == 401 || f.status == 403 {
                    continue;
                }
                if let Some(v) = f.json() {
                    if let Some(items) = defs::parse(&v) {
                        if let Some(list) = v["response"]["item_definitions"].as_array() {
                            for d in list {
                                let key =
                                    format!("item_class_{}", int(&d["item_class"]).unwrap_or(0));
                                defs_samples.entry(key).or_insert_with(|| shape(d, 2));
                            }
                        }
                        defs_result = Some((items, auth.name()));
                    }
                }
                break;
            }
            Err(e) => {
                attempts.insert(auth.name().into(), json!({ "error": e }));
                break;
            }
        }
    }
    attempts.insert("samples".into(), Value::Object(defs_samples));
    let mut defs_cards: Option<usize> = None;
    match &defs_result {
        Some((items, auth)) if !items.is_empty() => {
            defs_cards = Some(defs::card_count(items));
            let way = match *auth {
                "none" => "without a key",
                "key" => "with your API key",
                _ => "with your sign-in",
            };
            run.finish(
                "item_defs",
                t,
                StepStatus::Ok,
                format!(
                    "Works {way}: {} for app {sample_app}. This is the fast route for the catalog.",
                    kinds_text(items)
                ),
                Value::Object(attempts),
            );
        }
        Some(_) => run.finish(
            "item_defs",
            t,
            StepStatus::Warn,
            format!("Steam answered, but listed no items for app {sample_app}. The catalog would use card pages instead, which is slower."),
            Value::Object(attempts),
        ),
        None => run.finish(
            "item_defs",
            t,
            StepStatus::Warn,
            format!(
                "Steam's item list isn't available{}. The catalog would use card pages instead, which is slower.",
                defs_status
                    .map(|c| format!(" (code {c})"))
                    .unwrap_or_default()
            ),
            Value::Object(attempts),
        ),
    }

    // 9. The store check: which games have trading cards.
    let t = run.start("store_cards");
    let mut check: Vec<u64> = vec![sample_app];
    check.extend(
        owned_apps
            .iter()
            .copied()
            .filter(|a| *a != sample_app)
            .take(39),
    );
    let mut cards_ratio: Option<f64> = None;
    let mut f = steam.store_items(&check, None, notify).await;
    let mut store_auth = "without a key";
    if matches!(&f, Ok(r) if r.status == 401 || r.status == 403) {
        store_auth = "with your API key";
        f = steam.store_items(&check, Some(key.as_str()), notify).await;
    }
    match f {
        Ok(f) => {
            let mut detail = http_detail(&f, 2);
            if let (Value::Object(map), Some(v)) = (&mut detail, f.json()) {
                map.insert(
                    "first_item".into(),
                    v["response"]["store_items"]
                        .as_array()
                        .and_then(|a| a.first())
                        .map(|i| shape(i, 3))
                        .unwrap_or(Value::Null),
                );
            }
            match f.json().as_ref().and_then(store::parse) {
                Some(apps) if !apps.is_empty() => {
                    let yes = apps.iter().filter(|a| a.has_cards == Some(true)).count();
                    let no = apps.iter().filter(|a| a.has_cards == Some(false)).count();
                    let unknown = apps.len() - yes - no;
                    if yes + no > 0 {
                        cards_ratio = Some(yes as f64 / (yes + no) as f64);
                    }
                    let (status, text) = if yes + no == 0 {
                        (
                            StepStatus::Warn,
                            format!("The store answered {store_auth}, but without the trading card category for any of the {} games checked.", apps.len()),
                        )
                    } else {
                        (
                            StepStatus::Ok,
                            format!(
                                "Works {store_auth}: {yes} of {} games checked have trading cards{}.",
                                apps.len(),
                                if unknown > 0 { format!(", and {unknown} couldn't be told") } else { String::new() }
                            ),
                        )
                    };
                    run.finish("store_cards", t, status, text, detail);
                }
                _ => run.finish(
                    "store_cards",
                    t,
                    StepStatus::Warn,
                    format!("The store check didn't work (code {}). The scan would check each game's item list instead.", f.status),
                    detail,
                ),
            }
        }
        Err(e) => run.finish("store_cards", t, StepStatus::Fail, e, Value::Null),
    }

    // 10. The game's card page: the slower catalog route.
    let t = run.start("card_page");
    match steam.card_page(&sid, sample_app, notify).await {
        Ok(f) if f.skipped => run.finish(
            "card_page",
            t,
            StepStatus::Skipped,
            skipped_text(),
            Value::Null,
        ),
        Ok(f) => {
            let page = cardpage::parse(&f.body);
            let mut detail = html_detail(
                &f,
                &[
                    "badge_card_set_card",
                    "badge_card_set_text_qty",
                    "badge_card_set_text",
                    "gamecard",
                ],
            );
            if let Value::Object(map) = &mut detail {
                map.insert(
                    "parsed".into(),
                    json!(page
                        .cards
                        .iter()
                        .take(15)
                        .map(|c| json!({
                            "position": c.position,
                            "name": c.name,
                            "owned": c.owned,
                            "series": c.series,
                            "image": c.image.as_deref().map(|i| truncate(i, 90)),
                        }))
                        .collect::<Vec<_>>()),
                );
                map.insert("set_size".into(), json!(page.set_size));
                map.insert(
                    "first_card_html".into(),
                    json!(f.body.find("badge_card_set_card").map(|i| around(
                        &f.body,
                        i + 1200,
                        1200
                    ))),
                );
            }
            let with_images = page.cards.iter().filter(|c| c.image.is_some()).count();
            let names: Vec<&str> = page.cards.iter().take(4).map(|c| c.name.as_str()).collect();
            let agrees = match defs_cards {
                Some(n) if n == page.cards.len() => {
                    " It agrees with Steam's item list.".to_string()
                }
                Some(n) => format!(" Steam's item list had {n}."),
                None => String::new(),
            };
            let (status, text) = if !f.is_success() {
                (
                    StepStatus::Fail,
                    format!("Steam answered with code {}.", f.status),
                )
            } else if !page.cards.is_empty() {
                (
                    StepStatus::Ok,
                    format!(
                        "Read {} cards for app {sample_app} ({}{}), {with_images} with artwork.{agrees}",
                        page.cards.len(),
                        names.join(", "),
                        if page.cards.len() > names.len() { ", …" } else { "" }
                    ),
                )
            } else {
                (
                    StepStatus::Warn,
                    format!("The page loaded, but no cards could be read for app {sample_app}."),
                )
            };
            run.finish("card_page", t, status, text, detail);
        }
        Err(e) => run.finish("card_page", t, StepStatus::Fail, e, Value::Null),
    }

    // 11. The Points Shop catalog for the sample game and a few others in one
    // request, and the list of games and sale events with a Points Shop page.
    let t = run.start("points_catalog");
    let mut batch: Vec<u64> = vec![sample_app];
    batch.extend(
        badge_apps
            .iter()
            .map(|(app_id, _)| *app_id)
            .filter(|a| *a != sample_app)
            .take(4),
    );
    let mut detail = Map::new();
    // The same game asked both ways: does Steam also list the items that
    // can't be bought directly (sale rewards, retired items)?
    let mut unbuyable_extra: Option<i64> = None;
    if let (Ok(plain), Ok(all)) = (
        steam.reward_items(&[sample_app], None, false, notify).await,
        steam.reward_items(&[sample_app], None, true, notify).await,
    ) {
        let count = |f: &Fetched| {
            f.json()
                .as_ref()
                .and_then(points::parse_page)
                .map(|p| p.items.len() as i64)
        };
        if let (Some(a), Some(b)) = (count(&plain), count(&all)) {
            unbuyable_extra = Some(b - a);
            detail.insert(
                "unbuyable_check".into(),
                json!({ "listed_plain": a, "listed_with_unbuyable": b }),
            );
        }
    }
    let page = match steam.reward_items(&batch, None, true, notify).await {
        Ok(f) => {
            detail.insert("query".into(), http_detail(&f, 2));
            if let Some(v) = f.json() {
                let mut samples = Map::new();
                for d in v["response"]["definitions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    let key = format!(
                        "item_class_{}",
                        int(&d["community_item_class"]).unwrap_or(0)
                    );
                    samples.entry(key).or_insert_with(|| shape(d, 3));
                }
                detail.insert("samples".into(), Value::Object(samples));
            }
            f.json().as_ref().and_then(points::parse_page)
        }
        Err(e) => {
            detail.insert("query".into(), json!({ "error": e }));
            None
        }
    };
    let events = match steam.eligible_apps(notify).await {
        Ok(f) => {
            detail.insert("eligible_apps".into(), http_detail(&f, 1));
            f.json().as_ref().and_then(points::parse_eligible_apps)
        }
        Err(e) => {
            detail.insert("eligible_apps".into(), json!({ "error": e }));
            None
        }
    };
    match page {
        Some(page) => {
            let mut per_app: BTreeMap<u64, Vec<CatalogItem>> = BTreeMap::new();
            for p in &page.items {
                per_app.entry(p.appid).or_default().push(p.item.clone());
            }
            let sample_items = per_app.get(&sample_app).cloned().unwrap_or_default();
            detail.insert(
                "items_per_game".into(),
                json!(per_app
                    .iter()
                    .map(|(a, l)| (a.to_string(), l.len()))
                    .collect::<BTreeMap<_, _>>()),
            );
            detail.insert("next_cursor".into(), json!(page.next_cursor));
            detail.insert("total".into(), json!(page.total));
            let unbuyable_text = match unbuyable_extra {
                Some(n) if n > 0 => {
                    format!(" Asking for items that can't be bought directly added {n} more.")
                }
                Some(_) => {
                    " Asking for items that can't be bought directly added none.".to_string()
                }
                None => String::new(),
            };
            let events_text = events
                .as_ref()
                .map(|e| {
                    format!(
                        " Steam lists {} games and {} sale events with a Points Shop page.",
                        fmt_num(e.len() as u64),
                        fmt_num(e.iter().filter(|(_, ev)| *ev).count() as u64)
                    )
                })
                .unwrap_or_default();
            let text = if sample_items.is_empty() {
                format!(
                    "The Points Shop lists no items for app {sample_app}; {} items across the {} games asked about.{events_text}",
                    page.items.len(),
                    batch.len()
                )
            } else {
                format!(
                    "App {sample_app}: {}. One request covered {} games with {} items.{unbuyable_text}{events_text}",
                    kinds_text(&sample_items),
                    per_app.len(),
                    page.items.len()
                )
            };
            run.finish(
                "points_catalog",
                t,
                StepStatus::Ok,
                text,
                Value::Object(detail),
            );
        }
        None => run.finish(
            "points_catalog",
            t,
            StepStatus::Fail,
            "The Points Shop catalog couldn't be read.",
            Value::Object(detail),
        ),
    }

    // 12. Profile items the user owns, including Points Shop purchases
    let t = run.start("points_owned");
    match token.as_deref() {
        None => run.finish(
            "points_owned",
            t,
            StepStatus::Skipped,
            NEEDS_SIGN_IN,
            Value::Null,
        ),
        Some(tok) => {
            let url = webapi_url(
                "/IPlayerService/GetProfileItemsOwned/v1/",
                &[
                    ("access_token", tok),
                    ("steamid", sid.as_str()),
                    ("language", "english"),
                ],
            );
            match steam.fetch(Bucket::WebApi, &url, None, notify).await {
                Ok(f) => {
                    let owned: usize = f
                        .json()
                        .and_then(|v| {
                            v["response"].as_object().map(|o| {
                                o.values()
                                    .filter_map(|x| x.as_array())
                                    .map(|a| a.len())
                                    .sum()
                            })
                        })
                        .unwrap_or(0);
                    let kinds = f
                        .json()
                        .as_ref()
                        .and_then(library::parse_profile_items)
                        .map(|items| owned_kinds_text(&items))
                        .unwrap_or_default();
                    let (status, text) = if f.is_success() {
                        (
                            StepStatus::Ok,
                            format!(
                                "Steam listed {} profile items you own{kinds}.",
                                fmt_num(owned as u64)
                            ),
                        )
                    } else {
                        (
                            StepStatus::Fail,
                            format!("Steam didn't accept the sign-in token (code {}).", f.status),
                        )
                    };
                    run.finish("points_owned", t, status, text, http_detail(&f, 3));
                }
                Err(e) => run.finish("points_owned", t, StepStatus::Fail, e, Value::Null),
            }
        }
    }

    // 13. Every emoticon the user can use
    let t = run.start("emoticons");
    match token.as_deref() {
        None => run.finish(
            "emoticons",
            t,
            StepStatus::Skipped,
            NEEDS_SIGN_IN,
            Value::Null,
        ),
        Some(tok) => match steam.emoticons(tok, notify).await {
            Ok(f) => {
                let list = f.json().as_ref().and_then(library::parse_emoticons);
                let detail = http_detail(&f, 3);
                match list {
                    Some(list) => {
                        let games: BTreeSet<u64> = list.iter().map(|e| e.appid).collect();
                        run.finish(
                            "emoticons",
                            t,
                            StepStatus::Ok,
                            format!(
                                "You can use {} emoticons from {} games.",
                                fmt_num(list.len() as u64),
                                fmt_num(games.len() as u64)
                            ),
                            detail,
                        );
                    }
                    None => run.finish(
                        "emoticons",
                        t,
                        StepStatus::Fail,
                        format!("Steam didn't return your emoticons (code {}).", f.status),
                        detail,
                    ),
                }
            }
            Err(e) => run.finish("emoticons", t, StepStatus::Fail, e, Value::Null),
        },
    }

    // 14. Steam Family library
    let t = run.start("family");
    match token.as_deref() {
        None => run.finish("family", t, StepStatus::Skipped, NEEDS_SIGN_IN, Value::Null),
        Some(tok) => {
            let url = webapi_url(
                "/IFamilyGroupsService/GetFamilyGroupForUser/v1/",
                &[("access_token", tok), ("steamid", sid.as_str())],
            );
            match steam.fetch(Bucket::WebApi, &url, None, notify).await {
                Ok(f) => {
                    let v = f.json().unwrap_or(Value::Null);
                    let resp = &v["response"];
                    let group = resp["family_groupid"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| resp["family_groupid"].as_u64().map(|n| n.to_string()))
                        .filter(|g| g != "0");
                    let mut detail = json!({ "group": http_detail(&f, 3) });
                    let (status, text) = if !f.is_success() {
                        (
                            StepStatus::Fail,
                            format!("Steam didn't accept the sign-in token (code {}).", f.status),
                        )
                    } else if let Some(group_id) = group {
                        let url = webapi_url(
                            "/IFamilyGroupsService/GetSharedLibraryApps/v1/",
                            &[
                                ("access_token", tok),
                                ("family_groupid", group_id.as_str()),
                                ("steamid", sid.as_str()),
                                ("include_own", "false"),
                                ("include_excluded", "false"),
                                ("include_free", "false"),
                                ("language", "english"),
                            ],
                        );
                        match steam.fetch(Bucket::WebApi, &url, None, notify).await {
                            Ok(g) => {
                                let apps = g.json().and_then(|v| {
                                    v["response"]["apps"].as_array().map(|a| a.len())
                                });
                                detail["library"] = http_detail(&g, 3);
                                match apps {
                                    Some(n) => (
                                        StepStatus::Ok,
                                        format!("In a Steam Family with {} shared games.", fmt_num(n as u64)),
                                    ),
                                    None => (
                                        StepStatus::Warn,
                                        format!("In a Steam Family, but the shared library couldn't be read (code {}).", g.status),
                                    ),
                                }
                            }
                            Err(e) => (StepStatus::Fail, e),
                        }
                    } else {
                        (StepStatus::Ok, "Not in a Steam Family.".to_string())
                    };
                    run.finish("family", t, status, text, detail);
                }
                Err(e) => run.finish("family", t, StepStatus::Fail, e, Value::Null),
            }
        }
    }

    // 15. The badges page (lists card drops remaining and badge progress)
    let t = run.start("badge_page");
    let url = community_url(&format!("/profiles/{sid}/badges/"), &[("l", "english")]);
    match steam.fetch(Bucket::Community, &url, None, notify).await {
        Ok(f) if f.skipped => run.finish(
            "badge_page",
            t,
            StepStatus::Skipped,
            skipped_text(),
            Value::Null,
        ),
        Ok(f) => {
            let rows = count_regex(&f.body, r#"class="badge_row[ "]"#);
            let detail = html_detail(
                &f,
                &[
                    "badge_row",
                    "progress_info_bold",
                    "badge_title_stats_drops",
                    "pagebtn",
                    "pagelink",
                    "badge_progress_info",
                ],
            );
            let (status, text) = if !f.is_success() {
                (
                    StepStatus::Fail,
                    format!("Steam answered with code {}.", f.status),
                )
            } else if rows > 0 {
                (
                    StepStatus::Ok,
                    format!("The first page lists {rows} badges."),
                )
            } else {
                (
                    StepStatus::Warn,
                    "The page loaded, but no badges were found in it.".to_string(),
                )
            };
            run.finish("badge_page", t, status, text, detail);
        }
        Err(e) => run.finish("badge_page", t, StepStatus::Fail, e, Value::Null),
    }

    // Estimate how long a first full scan would take at this pacing.
    let mut apps: BTreeSet<u64> = badge_apps.iter().map(|(app_id, _)| *app_id).collect();
    apps.extend(inventory_apps.iter().copied());
    apps.remove(&753);
    let card_games = match cards_ratio {
        Some(ratio) => apps
            .len()
            .max((owned_count as f64 * ratio).round() as usize),
        None => apps.len(),
    };
    let fast = defs_cards.is_some();
    let per_game = if fast { 0.5 } else { 4.2 };
    let seconds = card_games as f64 * per_game + 30.0;
    let minutes = (seconds / 60.0).ceil() as u64;
    let estimate = if card_games == 0 {
        "The full scan time couldn't be estimated because no games with trading cards were found yet."
            .to_string()
    } else {
        format!(
            "A first full scan would take about {} for roughly {} games with trading cards, using {}. Later scans only read what's new.",
            plural(minutes, "minute"),
            fmt_num(card_games as u64),
            if fast { "Steam's item list" } else { "card pages" }
        )
    };

    let total_seconds = started_all.elapsed().as_secs();
    let requests = steam.requests().saturating_sub(requests_before);

    // Write the report: a readable summary first, then the raw details.
    let mut text = String::new();
    let _ = writeln!(text, "Steam Collection Tracker: connection test");
    let _ = writeln!(
        text,
        "App version: {}{}",
        env!("CARGO_PKG_VERSION"),
        crate::updater::build_id()
            .map(|b| format!(" (build {b})"))
            .unwrap_or_default()
    );
    let _ = writeln!(
        text,
        "Run at: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let _ = writeln!(text, "Steam ID: {sid}");
    let _ = writeln!(
        text,
        "Signed in: {}",
        if session.is_some() { "yes" } else { "no" }
    );
    let _ = writeln!(text, "Owned games: {owned_count}");
    let _ = writeln!(text, "Sample game: {sample_app}");
    let _ = writeln!(
        text,
        "Took {total_seconds} seconds and {requests} requests."
    );
    if let Some(ms) = market_ms {
        let _ = writeln!(text, "Market search answered in {ms} ms.");
    }
    let _ = writeln!(text);
    for step in &run.steps {
        let _ = writeln!(
            text,
            "{} {} ({:.1} s): {}",
            step.status.tag(),
            step.label,
            step.elapsed_ms as f64 / 1000.0,
            step.summary
        );
    }
    let _ = writeln!(text);
    let _ = writeln!(text, "{estimate}");
    let _ = writeln!(text);
    let _ = writeln!(text, "---- Details for troubleshooting ----");
    let _ = writeln!(
        text,
        "{}",
        serde_json::to_string_pretty(&Value::Object(run.details.clone())).unwrap_or_default()
    );

    // The API key and sign-in tokens must never end up in a file meant to be shared.
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut text = text;
    for secret in &secrets {
        if secret.len() >= 8 {
            text = text.replace(secret.as_str(), "<HIDDEN>");
        }
    }

    let file = state.paths.reports_dir().join(format!(
        "connection-test-{}.txt",
        chrono::Local::now().format("%Y-%m-%d-%H%M%S")
    ));
    std::fs::write(&file, text).map_err(|e| format!("Couldn't save the report: {e}"))?;
    housekeeping::prune_reports(&state.paths);

    Ok(DiagnosticsResult {
        report_path: file.display().to_string(),
        steps: run.steps,
        estimate,
        total_seconds,
    })
}

/// "8 cards, 8 foil cards, 5 emoticons…" for a list of catalog items.
fn kinds_text(items: &[CatalogItem]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for i in items {
        *counts
            .entry(kind_label(i.class, i.foil, i.animated))
            .or_default() += 1;
    }
    let mut list: Vec<(&str, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    list.iter()
        .map(|(label, n)| format!("{n} {}", label.to_lowercase()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// " (12 backgrounds, 3 avatar frames…)" for owned profile items.
fn owned_kinds_text(items: &[library::OwnedProfileItem]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for i in items {
        *counts.entry(kind_label(i.class, false, false)).or_default() += 1;
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(label, n)| format!("{n} {}", label.to_lowercase()))
        .collect();
    format!(": {}", parts.join(", "))
}

fn http_detail(f: &Fetched, depth: usize) -> Value {
    let mut map = Map::new();
    map.insert("status".to_string(), json!(f.status));
    map.insert("ms".to_string(), json!(f.elapsed_ms));
    map.insert("bytes".to_string(), json!(f.body.len()));
    match f.json() {
        Some(v) => {
            map.insert("shape".to_string(), shape(&v, depth));
        }
        None => {
            map.insert("snippet".to_string(), json!(truncate(&f.body, 400)));
        }
    }
    Value::Object(map)
}

fn html_detail(f: &Fetched, markers: &[&str]) -> Value {
    let found: Map<String, Value> = markers
        .iter()
        .map(|m| (m.to_string(), json!(f.body.matches(m).count())))
        .collect();
    let snippet = markers
        .iter()
        .find_map(|m| f.body.find(m))
        .map(|i| around(&f.body, i, 300));
    json!({
        "status": f.status,
        "ms": f.elapsed_ms,
        "bytes": f.body.len(),
        "final_url": f.final_url,
        "title": extract_between(&f.body, "<title>", "</title>").map(|t| t.trim().to_string()),
        "markers": found,
        "snippet": snippet,
    })
}

/// A compact outline of a JSON answer: its keys, array lengths and a sample of
/// each value, so the next version can be written against the real format.
fn shape(v: &Value, depth: usize) -> Value {
    match v {
        Value::Object(map) => {
            if depth == 0 {
                return json!(format!("{{object with {} keys}}", map.len()));
            }
            Value::Object(
                map.iter()
                    .take(40)
                    .map(|(k, x)| (k.clone(), shape(x, depth - 1)))
                    .collect(),
            )
        }
        Value::Array(items) => {
            if depth == 0 {
                return json!(format!("[array of {}]", items.len()));
            }
            json!({
                "_len": items.len(),
                "_first": items.first().map(|x| shape(x, depth - 1)).unwrap_or(Value::Null),
            })
        }
        Value::String(s) => json!(truncate(s, 80)),
        other => other.clone(),
    }
}

fn count_regex(haystack: &str, pattern: &str) -> usize {
    Regex::new(pattern)
        .map(|re| re.find_iter(haystack).count())
        .unwrap_or(0)
}

fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let to = s[from..].find(end)? + from;
    Some(&s[from..to])
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn around(s: &str, index: usize, radius: usize) -> String {
    let mut from = index.saturating_sub(radius);
    while !s.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (index + radius).min(s.len());
    while !s.is_char_boundary(to) {
        to += 1;
    }
    s[from..to].to_string()
}

fn fmt_num(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(999), "999");
        assert_eq!(fmt_num(1000), "1,000");
        assert_eq!(fmt_num(1234567), "1,234,567");
        assert_eq!(plural(1, "minute"), "1 minute");
        assert_eq!(plural(3, "minute"), "3 minutes");
    }

    #[test]
    fn text_helpers() {
        assert_eq!(
            extract_between("a var line1=[[1]]; b", "var line1=", "];"),
            Some("[[1]")
        );
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(around("abcdef", 3, 1), "cd");
        assert_eq!(
            count_regex(
                r#"<div class="badge_row is_link"> <div class="badge_row">"#,
                r#"class="badge_row[ "]"#
            ),
            2
        );
    }

    #[test]
    fn shapes_are_compact() {
        let v = json!({"response": {"games": [{"appid": 1, "name": "x"}, {"appid": 2}]}});
        let s = shape(&v, 4);
        assert_eq!(s["response"]["games"]["_len"], json!(2));
        assert_eq!(s["response"]["games"]["_first"]["appid"], json!(1));
        assert_eq!(shape(&v, 0), json!("{object with 1 keys}"));
    }

    #[test]
    fn plan_has_unique_keys() {
        let keys: BTreeSet<String> = plan().into_iter().map(|s| s.key).collect();
        assert_eq!(keys.len(), PLAN.len());
    }
}
