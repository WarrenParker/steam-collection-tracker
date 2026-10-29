//! Shared prices. The owner's copy of the app publishes the Market prices it
//! has read as one small file on a GitHub release, at most once a day, and
//! every other copy downloads that file before its own Market stage. Games
//! the file covers with prices newer than the copy's own are taken from it,
//! so Steam is only asked about the rest.
//!
//! The release lives in a repository of its own (the app's repository with
//! `-prices` added), so the token that publishes it can't touch the app's
//! code. The file's format is in `market/shared.rs`.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::time::Duration;

use rusqlite::{params_from_iter, Connection};
use serde::Serialize;
use serde_json::{json, Value as Json};

use crate::db;
use crate::market::{
    series,
    shared::{self, File, Game, Kind, Value, COLUMNS},
};
use crate::state::AppState;
use crate::steam::Notify;

/// The release tag and the file on it.
pub const TAG: &str = "prices";
pub const FILE_NAME: &str = "prices.txt.gz";
/// A new file is uploaded under this name, then renamed, so the published
/// file is only missing for a moment.
const UPLOAD_NAME: &str = "prices.next.txt.gz";

/// Size limits for a downloaded file, compressed and not.
const MAX_DOWNLOAD: usize = 32 << 20;
const MAX_TEXT: u64 = 256 << 20;

/// How often a copy looks for a newer file.
const CHECK_EVERY: i64 = 6 * 3600;
/// How often the owner's copy publishes.
const PUBLISH_EVERY: i64 = 20 * 3600;

const META_CHECKED_AT: &str = "shared_checked_at";
const META_USED: &str = "shared_used";
const META_PUBLISHED: &str = "shared_published";
const META_ERROR: &str = "shared_error";

/// The repository holding the "prices" release, or `None` in local builds.
pub fn repo() -> Option<String> {
    crate::updater::repo().map(|r| format!("{r}-prices"))
}

fn download_url(repo: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{TAG}/{FILE_NAME}")
}

// ---- Reading the file ---------------------------------------------------------------

fn gunzip(bytes: &[u8]) -> Result<String, String> {
    let unreadable = || "The shared price file couldn't be read.".to_string();
    // A server may already have unpacked it.
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return String::from_utf8(bytes.to_vec()).map_err(|_| unreadable());
    }
    let mut out = String::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_TEXT + 1)
        .read_to_string(&mut out)
        .map_err(|_| unreadable())?;
    if out.len() as u64 > MAX_TEXT {
        return Err("The shared price file is far larger than expected, so it wasn't used.".into());
    }
    Ok(out)
}

/// Downloads and reads the published file. `None` when nothing is published yet.
async fn download(http: &reqwest::Client, repo: &str) -> Result<Option<File>, String> {
    let offline = || {
        "The shared prices couldn't be downloaded. Prices come from Steam this time.".to_string()
    };
    let too_big =
        || "The shared price file is far larger than expected, so it wasn't used.".to_string();
    let mut resp = http
        .get(download_url(repo))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| offline())?;
    let status = resp.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(offline());
    }
    // GitHub hands release files out from its own servers.
    let host = resp.url().host_str().unwrap_or("").to_ascii_lowercase();
    if resp.url().scheme() != "https"
        || !(host == "github.com" || host.ends_with(".githubusercontent.com"))
    {
        return Err(offline());
    }
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_DOWNLOAD as u64)
    {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| offline())? {
        if body.len() + chunk.len() > MAX_DOWNLOAD {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    let text = gunzip(&body)?;
    shared::decode(&text).map(Some).map_err(|_| {
        "The shared price file is from a newer version of the app. Update the app to use it."
            .to_string()
    })
}

// ---- Saving it -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Key,
    /// The current price, kept when it's newer than ours.
    Current,
    Icon,
    History,
    Book,
}

fn part(column: &str) -> Part {
    match column {
        "hash_name" => Part::Key,
        "icon" => Part::Icon,
        "history_at" | "history_due" | "history_currency" | "avg_30" | "avg_90" | "avg_365"
        | "avg_all" | "low_price" | "low_day" | "high_price" | "high_day" | "sales_30"
        | "first_day" | "last_day" | "recent" => Part::History,
        "book_at" | "book_currency" | "book" | "buy_max" | "sell_count" | "buy_count" => Part::Book,
        _ => Part::Current,
    }
}

/// Adds or updates a Market row, keeping whichever of the current price,
/// price history and order book is newer, ours or the file's.
fn upsert_sql() -> String {
    let names: Vec<&str> = COLUMNS.iter().map(|c| c.0).collect();
    let holes: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
    let sets: Vec<String> = names
        .iter()
        .filter_map(|&n| {
            let newer = match part(n) {
                Part::Key => return None,
                Part::Icon => return Some("icon = COALESCE(market.icon, excluded.icon)".into()),
                Part::Current => "excluded.seen_at > market.seen_at",
                Part::History => {
                    "COALESCE(excluded.history_at, 0) > COALESCE(market.history_at, 0)"
                }
                Part::Book => "COALESCE(excluded.book_at, 0) > COALESCE(market.book_at, 0)",
            };
            Some(format!(
                "{n} = CASE WHEN {newer} THEN excluded.{n} ELSE market.{n} END"
            ))
        })
        .collect();
    format!(
        "INSERT INTO market ({}) VALUES ({}) ON CONFLICT (hash_name) DO UPDATE SET {}",
        names.join(", "),
        holes.join(", "),
        sets.join(", ")
    )
}

fn to_sql(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match v {
        Value::Null => Sql::Null,
        Value::Int(i) => Sql::Integer(*i),
        Value::Text(t) => Sql::Text(t.clone()),
        Value::Blob(b) => Sql::Blob(b.clone()),
    }
}

fn at(name: &str) -> usize {
    shared::column(name).expect("a shared column")
}

/// Clears a part of a row, for when it's in another currency or unreadable.
fn clear(row: &mut [Value], which: Part) {
    for (i, (name, _)) in COLUMNS.iter().enumerate() {
        if part(name) == which {
            row[i] = Value::Null;
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Used {
    /// When the file was made.
    pub made_at: i64,
    /// Games whose prices came from the file.
    pub games: usize,
    pub items: usize,
}

/// Brings the file's prices into the database for the games in `wanted`
/// (the games this user's scan prices). Only rows in `currency` are used.
pub fn import(
    c: &Connection,
    file: &File,
    wanted: &[u64],
    currency: i64,
) -> rusqlite::Result<Used> {
    let wanted: HashSet<u64> = wanted.iter().copied().collect();
    let checked: HashMap<u64, i64> = file
        .games
        .iter()
        .filter(|g| wanted.contains(&g.appid))
        .map(|g| (g.appid, g.checked_at))
        .collect();
    let mut ours: HashMap<u64, i64> = HashMap::new();
    for appid in checked.keys() {
        let t: Option<i64> = c
            .query_row(
                "SELECT market_checked_at FROM apps WHERE appid = ?1",
                [*appid as i64],
                |r| r.get(0),
            )
            .unwrap_or(None);
        ours.insert(*appid, t.unwrap_or(0));
    }
    // Games whose Market search in the file is newer than ours.
    let newer: HashSet<u64> = checked
        .iter()
        .filter(|(appid, t)| **t > ours.get(appid).copied().unwrap_or(0))
        .map(|(appid, _)| *appid)
        .collect();

    let mut st = c.prepare(&upsert_sql())?;
    let mut items = 0usize;
    for row in &file.rows {
        let Some(appid) = row[at("appid")].int().map(|a| a as u64) else {
            continue;
        };
        if !wanted.contains(&appid) || row[at("currency")].int() != Some(currency) {
            continue;
        }
        let mut row = row.clone();
        let history_ok = row[at("history_currency")]
            .int()
            .is_none_or(|c| c == currency)
            && match &row[at("recent")] {
                Value::Blob(b) => series::decode(b).is_some(),
                _ => true,
            };
        if !history_ok {
            clear(&mut row, Part::History);
        }
        if row[at("book_currency")]
            .int()
            .is_some_and(|c| c != currency)
        {
            clear(&mut row, Part::Book);
        }
        st.execute(params_from_iter(row.iter().map(to_sql)))?;
        if newer.contains(&appid) {
            items += 1;
        }
    }
    for appid in &newer {
        c.execute(
            "INSERT INTO apps (appid, market_checked_at) VALUES (?1, ?2)
             ON CONFLICT (appid) DO UPDATE SET market_checked_at = excluded.market_checked_at",
            rusqlite::params![*appid as i64, checked[appid]],
        )?;
    }
    Ok(Used {
        made_at: file.made_at,
        games: newer.len(),
        items,
    })
}

/// Before the Market stage: downloads the shared file when it's due and
/// takes the prices it covers. Never fails the scan; problems are reported.
pub async fn before_market(state: &AppState, family_on: bool, currency: i64, notify: Notify<'_>) {
    let settings = state.settings();
    if !settings.shared_prices || settings.has_github_token() {
        return;
    }
    let Some(repo) = repo() else { return };
    let now = db::now();
    let last = state
        .db
        .read(|c| db::get_meta(c, META_CHECKED_AT))
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    if now - last < CHECK_EVERY {
        return;
    }
    let file = match download(state.steam.http(), &repo).await {
        Ok(file) => file,
        Err(e) => {
            let _ = state.db.write(|tx| db::set_meta(tx, META_ERROR, &e));
            notify(e);
            return;
        }
    };
    let result = state.db.write(|tx| {
        let mut used = Used::default();
        if let Some(file) = &file {
            used.made_at = file.made_at;
            if file.currency() == currency {
                let wanted = db::market_targets(tx, family_on, i64::MAX)?;
                used = import(tx, file, &wanted, currency)?;
            }
        }
        db::set_meta(tx, META_CHECKED_AT, &now.to_string())?;
        db::set_meta(tx, META_USED, &json!(used).to_string())?;
        db::set_meta(tx, META_ERROR, "")
    });
    if let Err(e) = result {
        notify(e);
    }
}

// ---- Publishing ---------------------------------------------------------------------

/// The file for everything this copy has read from the Market.
pub fn build(c: &Connection, now: i64) -> rusqlite::Result<File> {
    let mut games = Vec::new();
    {
        let mut st = c.prepare(
            "SELECT appid, market_checked_at FROM apps
             WHERE market_checked_at IS NOT NULL
               AND appid IN (SELECT DISTINCT appid FROM market)
             ORDER BY appid",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (appid, checked_at) = row?;
            if appid > 0 {
                games.push(Game {
                    appid: appid as u64,
                    checked_at,
                });
            }
        }
    }
    let names: Vec<&str> = COLUMNS.iter().map(|c| c.0).collect();
    let mut st = c.prepare(&format!(
        "SELECT {} FROM market ORDER BY appid, hash_name",
        names.join(", ")
    ))?;
    let rows = st.query_map([], |r| {
        let mut out = Vec::with_capacity(COLUMNS.len());
        for (i, (_, kind)) in COLUMNS.iter().enumerate() {
            out.push(match kind {
                Kind::Text => r
                    .get::<_, Option<String>>(i)?
                    .map_or(Value::Null, Value::Text),
                Kind::Int(..) => r.get::<_, Option<i64>>(i)?.map_or(Value::Null, Value::Int),
                Kind::Blob => r
                    .get::<_, Option<Vec<u8>>>(i)?
                    .map_or(Value::Null, Value::Blob),
            });
        }
        Ok(out)
    })?;
    Ok(File {
        made_at: now,
        games,
        rows: rows.collect::<rusqlite::Result<_>>()?,
    })
}

fn gzip(text: &str) -> Result<Vec<u8>, String> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(text.as_bytes())
        .and_then(|_| enc.finish())
        .map_err(|_| "The shared price file couldn't be packed.".to_string())
}

/// One call to GitHub's API with the owner's token.
async fn github(
    http: &reqwest::Client,
    token: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<(Vec<u8>, &str)>,
) -> Result<(u16, Json), String> {
    let mut req = http
        .request(method, url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(120));
    if let Some((bytes, content_type)) = body {
        req = req
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes);
    }
    let resp = req
        .send()
        .await
        .map_err(|_| "Couldn't reach GitHub. Check your internet connection.".to_string())?;
    let status = resp.status().as_u16();
    let json = resp.json::<Json>().await.unwrap_or(Json::Null);
    Ok((status, json))
}

fn token_problem(status: u16, repo: &str) -> String {
    match status {
        401 => "GitHub didn't accept the token. It may have expired: create a new one and enter it again.".into(),
        403 | 404 => format!(
            "The token can't publish to {repo}. Make sure that repository exists and the token has Contents: Read and write on it."
        ),
        422 => format!("GitHub couldn't make the release. Add a README to {repo} on github.com first, so it has something to tag."),
        429 => "GitHub is busy. Try again in a few minutes.".into(),
        s => format!("GitHub answered with code {s}. Try again later."),
    }
}

/// The "prices" release's id and its files (name, id), made if missing.
async fn release(
    http: &reqwest::Client,
    token: &str,
    repo: &str,
) -> Result<(u64, Vec<(String, u64)>), String> {
    let api = format!("https://api.github.com/repos/{repo}/releases");
    let (status, mut json) = github(
        http,
        token,
        reqwest::Method::GET,
        &format!("{api}/tags/{TAG}"),
        None,
    )
    .await?;
    if status == 404 {
        let body = json!({
            "tag_name": TAG,
            "name": "Shared prices",
            "body": "Steam Market prices read by Steam Collection Tracker, replaced once a day. The app downloads this file by itself.",
        })
        .to_string()
        .into_bytes();
        let (status, made) = github(
            http,
            token,
            reqwest::Method::POST,
            &api,
            Some((body, "application/json")),
        )
        .await?;
        if status != 201 {
            return Err(token_problem(status, repo));
        }
        json = made;
    } else if !(200..300).contains(&status) {
        return Err(token_problem(status, repo));
    }
    let id = json["id"].as_u64().ok_or_else(|| token_problem(0, repo))?;
    let assets = json["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["id"].as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    Ok((id, assets))
}

/// Checks a token can publish, making the release if it's missing.
pub async fn check_token(http: &reqwest::Client, token: &str) -> Result<(), String> {
    let repo = repo()
        .ok_or_else(|| "Sharing prices is turned off in this copy of the app.".to_string())?;
    release(http, token, &repo).await.map(|_| ())
}

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct Published {
    pub at: i64,
    pub games: usize,
    pub items: usize,
    pub bytes: usize,
}

/// Publishes the file now.
async fn publish(state: &AppState, token: &str) -> Result<Published, String> {
    let repo = repo()
        .ok_or_else(|| "Sharing prices is turned off in this copy of the app.".to_string())?;
    let now = db::now();
    let file = state.db.read(|c| build(c, now))?;
    if file.rows.is_empty() {
        return Err("There are no prices to share yet. Scan first.".into());
    }
    let bytes = gzip(&shared::encode(&file))?;
    let info = Published {
        at: now,
        games: file.games.len(),
        items: file.rows.len(),
        bytes: bytes.len(),
    };
    let http = state.steam.http();
    let (id, assets) = release(http, token, &repo).await?;
    let asset_url =
        |asset: u64| format!("https://api.github.com/repos/{repo}/releases/assets/{asset}");
    for (name, asset) in &assets {
        if name == UPLOAD_NAME {
            github(
                http,
                token,
                reqwest::Method::DELETE,
                &asset_url(*asset),
                None,
            )
            .await?;
        }
    }
    let upload =
        format!("https://uploads.github.com/repos/{repo}/releases/{id}/assets?name={UPLOAD_NAME}");
    let (status, made) = github(
        http,
        token,
        reqwest::Method::POST,
        &upload,
        Some((bytes, "application/gzip")),
    )
    .await?;
    if status != 201 {
        return Err(token_problem(status, &repo));
    }
    let new_id = made["id"].as_u64().ok_or_else(|| token_problem(0, &repo))?;
    for (name, asset) in &assets {
        if name == FILE_NAME {
            github(
                http,
                token,
                reqwest::Method::DELETE,
                &asset_url(*asset),
                None,
            )
            .await?;
        }
    }
    let rename = json!({ "name": FILE_NAME }).to_string().into_bytes();
    let (status, _) = github(
        http,
        token,
        reqwest::Method::PATCH,
        &asset_url(new_id),
        Some((rename, "application/json")),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(token_problem(status, &repo));
    }
    state.db.write(|tx| {
        db::set_meta(tx, META_PUBLISHED, &json!(info).to_string())?;
        db::set_meta(tx, META_ERROR, "")
    })?;
    Ok(info)
}

fn published(state: &AppState) -> Option<Published> {
    let text = state
        .db
        .read(|c| db::get_meta(c, META_PUBLISHED))
        .ok()
        .flatten()?;
    let v: Json = serde_json::from_str(&text).ok()?;
    Some(Published {
        at: v["at"].as_i64()?,
        games: v["games"].as_u64()? as usize,
        items: v["items"].as_u64()? as usize,
        bytes: v["bytes"].as_u64()? as usize,
    })
}

/// After a scan's Market stage: publishes when the owner's copy is set up and
/// a day has passed. Never fails the scan; problems are reported.
pub async fn after_market(state: &AppState, notify: Notify<'_>) {
    let Ok(Some(token)) = state.settings().github_token() else {
        return;
    };
    if published(state).is_some_and(|p| db::now() - p.at < PUBLISH_EVERY) {
        return;
    }
    if let Err(e) = publish(state, &token).await {
        let _ = state.db.write(|tx| db::set_meta(tx, META_ERROR, &e));
        notify(format!("Prices weren't shared: {e}"));
    }
}

/// Publishes straight away, from Settings.
pub async fn publish_now(state: &AppState) -> Result<Status, String> {
    let token = state
        .settings()
        .github_token()?
        .ok_or_else(|| "Enter a GitHub token first.".to_string())?;
    if let Err(e) = publish(state, &token).await {
        let _ = state.db.write(|tx| db::set_meta(tx, META_ERROR, &e));
        return Err(e);
    }
    Ok(status(state))
}

/// What Settings shows about shared prices.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    pub repo: Option<String>,
    pub enabled: bool,
    pub publishing: bool,
    pub checked_at: Option<i64>,
    pub used: Option<Used>,
    pub published: Option<Published>,
    pub error: Option<String>,
}

pub fn status(state: &AppState) -> Status {
    let settings = state.settings();
    let meta = |key: &str| state.db.read(|c| db::get_meta(c, key)).ok().flatten();
    let used = meta(META_USED)
        .and_then(|t| serde_json::from_str::<Json>(&t).ok())
        .and_then(|v| {
            Some(Used {
                made_at: v["made_at"].as_i64()?,
                games: v["games"].as_u64()? as usize,
                items: v["items"].as_u64()? as usize,
            })
        });
    Status {
        repo: repo(),
        enabled: settings.shared_prices,
        publishing: settings.has_github_token(),
        checked_at: meta(META_CHECKED_AT).and_then(|v| v.parse().ok()),
        used,
        published: published(state),
        error: meta(META_ERROR).filter(|e| !e.is_empty()),
    }
}

/// Lets the next scan look for a newer file straight away, for when the
/// setting is turned back on.
pub fn check_again(state: &AppState) {
    let _ = state.db.write(|tx| db::set_meta(tx, META_CHECKED_AT, "0"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_db(f: impl FnOnce(&Connection)) {
        let d = db::Db::in_memory();
        d.read(|c| {
            f(c);
            Ok(())
        })
        .unwrap();
    }

    fn row(hash: &str, appid: i64, lowest: i64, seen_at: i64) -> Vec<Value> {
        let mut r = vec![Value::Null; COLUMNS.len()];
        r[at("hash_name")] = Value::Text(hash.into());
        r[at("appid")] = Value::Int(appid);
        r[at("class")] = Value::Int(2);
        r[at("foil")] = Value::Int(0);
        r[at("name")] = Value::Text(hash.into());
        r[at("currency")] = Value::Int(1);
        r[at("lowest")] = Value::Int(lowest);
        r[at("listings")] = Value::Int(3);
        r[at("seen_at")] = Value::Int(seen_at);
        r
    }

    fn lowest(c: &Connection, hash: &str) -> Option<i64> {
        c.query_row(
            "SELECT lowest FROM market WHERE hash_name = ?1",
            [hash],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn newer_prices_win_and_wanted_games_only() {
        with_db(|c| {
            c.execute_batch(
                "INSERT INTO apps (appid, market_checked_at) VALUES (10, 1792000000), (20, 1792900000);
                 INSERT INTO market (hash_name, appid, class, foil, name, currency, lowest, listings, seen_at, icon)
                 VALUES ('10-A', 10, 2, 0, 'A', 1, 500, 1, 1792000100, 'ours'),
                        ('20-B', 20, 2, 0, 'B', 1, 700, 1, 1792900100, NULL);",
            )
            .unwrap();
            let mut file = File {
                made_at: 1_792_866_504,
                games: vec![
                    Game {
                        appid: 10,
                        checked_at: 1_792_800_000,
                    },
                    Game {
                        appid: 20,
                        checked_at: 1_792_800_000,
                    },
                    Game {
                        appid: 30,
                        checked_at: 1_792_800_000,
                    },
                ],
                rows: vec![
                    row("10-A", 10, 450, 1_792_800_100),
                    row("10-C", 10, 90, 1_792_800_100),
                    row("20-B", 20, 650, 1_792_800_100),
                    row("30-D", 30, 10, 1_792_800_100),
                ],
            };
            file.rows[0][at("icon")] = Value::Text("theirs".into());
            file.rows[2][at("icon")] = Value::Text("theirs".into());
            let used = import(&c, &file, &[10, 20], 1).unwrap();
            assert_eq!(
                used,
                Used {
                    made_at: 1_792_866_504,
                    games: 1,
                    items: 2
                }
            );
            assert_eq!(lowest(&c, "10-A"), Some(450), "the file's price is newer");
            assert_eq!(lowest(&c, "10-C"), Some(90), "a new item is added");
            assert_eq!(lowest(&c, "20-B"), Some(700), "our price is newer");
            let n: i64 = c
                .query_row("SELECT COUNT(*) FROM market WHERE appid = 30", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(n, 0, "games the user doesn't price are left out");
            let icons: Vec<Option<String>> = c
                .prepare("SELECT icon FROM market WHERE hash_name IN ('10-A', '20-B') ORDER BY hash_name")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(icons, vec![Some("ours".into()), Some("theirs".into())]);
            let checked: Vec<i64> = c
                .prepare(
                    "SELECT market_checked_at FROM apps WHERE appid IN (10, 20) ORDER BY appid",
                )
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(checked, vec![1_792_800_000, 1_792_900_000]);
        });
    }

    #[test]
    fn history_and_books_keep_the_newer_side() {
        with_db(|c| {
            c.execute_batch(
                "INSERT INTO market (hash_name, appid, class, foil, name, currency, lowest, listings, seen_at,
                                     history_at, avg_30, book_at, book)
                 VALUES ('10-A', 10, 2, 0, 'A', 1, 500, 1, 1792900000, 1792000000, 111, 1792950000, 'ours');",
            )
            .unwrap();
            let mut r = row("10-A", 10, 450, 1_792_800_000);
            r[at("history_at")] = Value::Int(1_792_800_000);
            r[at("avg_30")] = Value::Int(222);
            r[at("recent")] = Value::Blob(series::encode(&[]));
            r[at("book_at")] = Value::Int(1_792_800_000);
            r[at("book")] = Value::Text("theirs".into());
            let file = File {
                made_at: 1,
                games: vec![],
                rows: vec![r],
            };
            import(&c, &file, &[10], 1).unwrap();
            let got: (i64, i64, String) = c
                .query_row(
                    "SELECT lowest, avg_30, book FROM market WHERE hash_name = '10-A'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(got, (500, 222, "ours".into()));
        });
    }

    #[test]
    fn other_currencies_and_broken_history_are_skipped() {
        with_db(|c| {
            let mut euro = row("10-E", 10, 300, 1_792_800_000);
            euro[at("currency")] = Value::Int(3);
            let mut broken = row("10-H", 10, 300, 1_792_800_000);
            broken[at("history_at")] = Value::Int(1_792_800_000);
            broken[at("avg_30")] = Value::Int(5);
            broken[at("recent")] = Value::Blob(vec![99, 1, 2]);
            let file = File {
                made_at: 1,
                games: vec![],
                rows: vec![euro, broken],
            };
            import(&c, &file, &[10], 1).unwrap();
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM market WHERE hash_name = '10-E'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let avg: Option<i64> = c
                .query_row(
                    "SELECT avg_30 FROM market WHERE hash_name = '10-H'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(avg, None, "unreadable history isn't kept");
        });
    }

    #[test]
    fn build_and_read_back() {
        with_db(|c| {
            c.execute_batch(
                "INSERT INTO apps (appid, market_checked_at) VALUES (10, 1792800000), (11, 1792800000);
                 INSERT INTO market (hash_name, appid, class, foil, name, currency, lowest, listings, seen_at)
                 VALUES ('10-A', 10, 2, 0, 'A\tB', 1, 500, 1, 1792800100);",
            )
            .unwrap();
            let file = build(&c, 1_792_866_504).unwrap();
            assert_eq!(
                file.games,
                vec![Game {
                    appid: 10,
                    checked_at: 1_792_800_000
                }]
            );
            let text = gunzip(&gzip(&shared::encode(&file)).unwrap()).unwrap();
            assert_eq!(shared::decode(&text).unwrap(), file);
        });
    }

    #[test]
    fn unpacking_is_capped() {
        assert!(gunzip(b"plain text").is_ok());
        assert!(gunzip(&[0x1f, 0x8b, 1, 2, 3]).is_err());
    }
}
