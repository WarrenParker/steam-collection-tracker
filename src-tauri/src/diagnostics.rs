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

use crate::housekeeping;
use crate::session::{self, Session};
use crate::state::AppState;
use crate::steam::{
    community_url, inventory, parse_badges, parse_owned_games, parse_player_summary, webapi_url,
    Bucket, Fetched, Notify, BLOCK_MINUTES,
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
    ("card_page", "A game's card set page"),
    ("market_search", "Market items for one game"),
    ("price_history", "Price history for one item"),
    ("points_catalog", "Points Shop items for one game"),
    ("points_owned", "Points Shop items you own"),
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
    match steam.fetch(Bucket::WebApi, &url, None, notify).await {
        Ok(f) => {
            let owned = parse_owned_games(&f.json().unwrap_or(Value::Null));
            owned_count = owned.games.len();
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
    let mut inventory_total: Option<u64> = None;
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
            inventory_total = info.total;
            inventory_apps = tally.apps.iter().copied().collect();
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

    // 6. The game's card page
    let t = run.start("card_page");
    let url = community_url(
        &format!("/profiles/{sid}/gamecards/{sample_app}/"),
        &[("l", "english")],
    );
    match steam.fetch(Bucket::Community, &url, None, notify).await {
        Ok(f) if f.skipped => run.finish(
            "card_page",
            t,
            StepStatus::Skipped,
            skipped_text(),
            Value::Null,
        ),
        Ok(f) => {
            let cards = count_regex(&f.body, r#"class="badge_card_set_card[ "]"#);
            let detail = html_detail(
                &f,
                &[
                    "badge_card_set_card",
                    "badge_card_set_text_cardname",
                    "badge_card_set_text_qty",
                    "badge_card_set_title",
                    "gamecard_badge_progress",
                    "badge_title_stats_drops",
                    "badge_info_title",
                    "badge_current",
                ],
            );
            let (status, text) = if !f.is_success() {
                (
                    StepStatus::Fail,
                    format!("Steam answered with code {}.", f.status),
                )
            } else if cards > 0 {
                (
                    StepStatus::Ok,
                    format!("Found {cards} cards in the set for app {sample_app}."),
                )
            } else {
                (
                    StepStatus::Warn,
                    format!("The page loaded, but no cards were found for app {sample_app}."),
                )
            };
            run.finish("card_page", t, status, text, detail);
        }
        Err(e) => run.finish("card_page", t, StepStatus::Fail, e, Value::Null),
    }

    // 7. Every market item for the game, in one request
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

    // 8. Price history: the price history API when signed in, otherwise the
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

    // 9. Points Shop catalog for the game
    let t = run.start("points_catalog");
    let app_text = sample_app.to_string();
    let url = webapi_url(
        "/ILoyaltyRewardsService/QueryRewardItems/v1/",
        &[
            ("appids[0]", app_text.as_str()),
            ("count", "100"),
            ("language", "english"),
        ],
    );
    match steam.fetch(Bucket::WebApi, &url, None, notify).await {
        Ok(f) => {
            let count = f
                .json()
                .and_then(|v| v["response"]["definitions"].as_array().map(|a| a.len()));
            let (status, text) = match count {
                Some(n) if n > 0 => (
                    StepStatus::Ok,
                    format!("Found {n} Points Shop items for app {sample_app}."),
                ),
                Some(_) => (
                    StepStatus::Warn,
                    format!("The Points Shop lists no items for app {sample_app}."),
                ),
                None => (
                    StepStatus::Fail,
                    format!(
                        "The Points Shop catalog couldn't be read (code {}).",
                        f.status
                    ),
                ),
            };
            run.finish("points_catalog", t, status, text, http_detail(&f, 2));
        }
        Err(e) => run.finish("points_catalog", t, StepStatus::Fail, e, Value::Null),
    }

    // 10. Points Shop items the user owns
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
                    let (status, text) = if f.is_success() {
                        (
                            StepStatus::Ok,
                            format!(
                                "Steam listed {} Points Shop items you own.",
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

    // 11. Steam Family library
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

    // 12. The badges page (lists card drops remaining and badge progress)
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

    // Estimate how long a full item scan would take at this pacing.
    let mut apps: BTreeSet<u64> = badge_apps.iter().map(|(app_id, _)| *app_id).collect();
    apps.extend(inventory_apps.iter().copied());
    apps.remove(&753);
    let inventory_pages = inventory_total
        .map(|n| n.div_ceil(2000))
        .unwrap_or(1)
        .max(1);
    let seconds = inventory_pages as f64 * 4.0 + apps.len() as f64 * (3.5 + 4.0) + 5.0;
    let minutes = (seconds / 60.0).ceil() as u64;
    let estimate = if apps.is_empty() {
        "The full scan time couldn't be estimated because no games with items were found yet."
            .to_string()
    } else {
        format!(
            "A full item scan would take about {} for the {} games found with items so far. Price history would then fill in gradually in the background.",
            plural(minutes, "minute"),
            fmt_num(apps.len() as u64)
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

fn result_detail(r: &Result<Fetched, String>, depth: usize) -> Value {
    match r {
        Ok(f) => http_detail(f, depth),
        Err(e) => json!({ "error": e }),
    }
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

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
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
        assert_eq!(tail("héllo", 4), "…llo");
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
