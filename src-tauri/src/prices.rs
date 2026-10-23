//! Reading prices from the Steam Community Market, as the last two stages of
//! a scan:
//!
//! 5. **Market prices:** each game's items on the Market, ten per request,
//!    with the cheapest listing. Read again when older than a day.
//! 6. **Price history:** each item's price history, items that change cost
//!    to complete first, then items the user owns, then the rest. A history
//!    is read once and then again about a month later, staggered so only a
//!    few items fall due each day.
//!
//! Order books, for cards that still need two or more copies (so the cost
//! walks up the listings), are read when the user opens a game rather than
//! during scans: there are over a thousand of them on a large account.
//!
//! Exchange rates come from api.frankfurter.dev, once a day, and only when
//! prices are shown in a currency other than the one Steam answers in.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::completion::{self, Group};
use crate::db::{self, HistorySave};
use crate::market::{self, series};
use crate::scan::{Ctx, RunEnd};
use crate::state::AppState;
use crate::steam::{Bucket, Notify};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;
/// Market prices older than this are read again.
pub const PRICES_STALE_HOURS: i64 = 20;
/// Order books older than this are read again (completion trusts them for 7).
const BOOK_STALE_DAYS: i64 = 6;
/// A price history is read again 23 to 37 days later.
const HISTORY_DAYS: i64 = 30;
const HISTORY_SPREAD_DAYS: i64 = 7;
/// After an answer that couldn't be read, try that item again a day later.
const RETRY_DAYS: i64 = 1;
/// A first price history fill expected to take longer than this waits for a go-ahead.
const CONFIRM_SECONDS: f64 = 10.0 * 60.0;
/// Seconds per Market request on long reads, which the request budget holds
/// to about one every 7.5 seconds.
pub const SECONDS_PER_REQUEST: f64 = 7.5;
/// A game with more items than this on the Market is cut short.
const MAX_SEARCH_ITEMS: u64 = 3000;
/// Market searches in a row that fail before the stage gives up for now.
const MAX_FAILURES: u32 = 5;

const META_CURRENCY: &str = market::META_CURRENCY;
const META_CURRENCY_AT: &str = "market_currency_at";
/// The Market pace, kept slower after Steam refused a long read.
const META_MARKET_PERIOD: &str = "market_period_ms";
/// Market refusals in a row, and when the app may ask again (Unix time),
/// signed in and not, kept so a restart doesn't ask too soon.
const META_MARKET_WAIT: [(bool, &str, &str); 2] = [
    (false, "market_refusals_anon", "market_blocked_until_anon"),
    (
        true,
        "market_refusals_signed_in",
        "market_blocked_until_signed_in",
    ),
];
/// Something always on the Market, used to learn which currency Steam answers in.
const PROBE_ITEM: &str = "753-Sack of Gems";
const RATES_URL: &str = "https://api.frankfurter.dev/v2/rates";

/// The currency Steam answers in for this account, once known.
pub fn market_currency(c: &rusqlite::Connection) -> rusqlite::Result<Option<i64>> {
    Ok(db::get_meta(c, META_CURRENCY)?.and_then(|v| v.parse().ok()))
}

fn remember_currency(state: &AppState, id: i64) {
    if id > 0 {
        let _ = state.db.write(|tx| {
            db::set_meta(tx, META_CURRENCY, &id.to_string())?;
            db::set_meta(tx, META_CURRENCY_AT, &db::now().to_string())
        });
    }
}

/// When an item's history is next due: about a month away, spread by a
/// steady hash of its name so items don't all fall due on the same day.
fn next_due(hash_name: &str, now: i64) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in hash_name.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let spread = (h % (2 * HISTORY_SPREAD_DAYS as u64 + 1)) as i64 - HISTORY_SPREAD_DAYS;
    now + (HISTORY_DAYS + spread) * DAY
}

/// Carries on once the Market is allowed again.
fn waiting(state: &AppState, signed_in: bool) -> RunEnd {
    let wait = state.steam.wait_seconds(Bucket::Market, signed_in).max(60);
    RunEnd::Waiting(db::now() + wait as i64)
}

/// The message when Steam is refusing the Market right now.
fn slow_down_text(state: &AppState, signed_in: bool) -> String {
    let wait = state.steam.wait_seconds(Bucket::Market, signed_in).max(60);
    format!(
        "Steam asked the app to slow down. Try again in about {}.",
        crate::scan::duration_text(wait)
    )
}

static MARKET_RESTORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Restores the Market pace, refusals in a row and any wait still running,
/// once per app start.
fn restore_market(state: &AppState) {
    if MARKET_RESTORED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let meta = |key: &str| -> Option<i64> {
        state
            .db
            .read(|c| db::get_meta(c, key))
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
    };
    if let Some(ms) = meta(META_MARKET_PERIOD) {
        state.steam.set_market_period_ms(ms.max(0) as u64);
    }
    for (signed_in, refusals_key, until_key) in META_MARKET_WAIT {
        let refusals = meta(refusals_key).unwrap_or(0).clamp(0, 100) as u32;
        let left = meta(until_key).map_or(0, |t| (t - db::now()).max(0) as u64);
        if refusals > 0 || left > 0 {
            state
                .steam
                .restore_block(Bucket::Market, signed_in, refusals, left);
        }
    }
}

/// Seconds per Market request on long reads at the current pace, which is
/// slower after refusals.
pub fn seconds_per_request(state: &AppState) -> f64 {
    restore_market(state);
    (state.steam.market_period_ms() as f64 / 1000.0).max(SECONDS_PER_REQUEST)
}

/// Saves the Market pace, refusals in a row and any wait, for the next run
/// and the next app start.
fn save_market(state: &AppState) {
    let ms = state.steam.market_period_ms().to_string();
    let _ = state.db.write(|tx| {
        db::set_meta(tx, META_MARKET_PERIOD, &ms)?;
        for (signed_in, refusals_key, until_key) in META_MARKET_WAIT {
            let refusals = state.steam.refusals(Bucket::Market, signed_in);
            let wait = state.steam.wait_seconds(Bucket::Market, signed_in);
            let until = if wait > 0 { db::now() + wait as i64 } else { 0 };
            db::set_meta(tx, refusals_key, &refusals.to_string())?;
            db::set_meta(tx, until_key, &until.to_string())?;
        }
        Ok(())
    });
}

/// Stages 5 and 6.
pub(crate) async fn stages(
    ctx: &Ctx<'_>,
    family_on: bool,
    resume: bool,
    cookie: Option<&str>,
) -> Result<RunEnd, String> {
    let state = ctx.state;
    restore_market(state);
    let outcome = run_stages(ctx, family_on, resume, cookie).await;
    save_market(state);
    outcome
}

async fn run_stages(
    ctx: &Ctx<'_>,
    family_on: bool,
    resume: bool,
    cookie: Option<&str>,
) -> Result<RunEnd, String> {
    let period = || ctx.state.steam.market_period_ms() as f64 / 1000.0;
    let state = ctx.state;
    let notify = ctx.notify;
    let data = &state.db;
    let signed_in = cookie.is_some();

    // Which currency Steam answers in, checked once a week.
    let known_at = data
        .read(|c| db::get_meta(c, META_CURRENCY_AT))?
        .and_then(|v| v.parse::<i64>().ok());
    if known_at.is_none_or(|t| db::now() - t > 7 * DAY) {
        ctx.stage(5, "Checking Market prices…", 0);
        let f = state.steam.market_book(PROBE_ITEM, cookie, notify).await?;
        if f.status == 429 {
            return Ok(waiting(state, signed_in));
        }
        if let Some(b) = f.json().as_ref().and_then(market::parse_book) {
            remember_currency(state, b.currency);
        }
    }
    if let Err(e) = refresh_rates(state).await {
        notify(e);
    }

    // Stage 5: current prices, game by game.
    data.write(|tx| db::ensure_inventory_apps(tx))?;
    let targets =
        data.read(|c| db::market_targets(c, family_on, db::now() - PRICES_STALE_HOURS * HOUR))?;
    let currency = data.read(market_currency)?.unwrap_or(1);
    let mut pages_left: u64 = targets
        .iter()
        .map(|a| {
            data.read(|c| db::market_item_estimate(c, *a))
                .unwrap_or(10)
                .div_ceil(market::SEARCH_PAGE)
        })
        .sum();
    ctx.stage(5, "Reading Market prices…", targets.len() as u64);
    let timer = Timer::new();
    let mut failures = 0u32;
    for (i, appid) in targets.iter().enumerate() {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        if state.steam.is_blocked(Bucket::Market, signed_in) {
            return Ok(waiting(state, signed_in));
        }
        let started = db::now();
        let expected = data
            .read(|c| db::market_item_estimate(c, *appid))
            .unwrap_or(10)
            .div_ceil(market::SEARCH_PAGE);
        let mut start = 0u64;
        let mut finished = false;
        loop {
            let t = Instant::now();
            let f = state
                .steam
                .market_search(*appid, start, cookie, notify)
                .await?;
            timer.add(t);
            if f.status == 429 {
                return Ok(waiting(state, signed_in));
            }
            let Some(page) = f
                .json()
                .as_ref()
                .filter(|_| f.is_success())
                .and_then(market::parse_search)
            else {
                failures += 1;
                if failures >= MAX_FAILURES {
                    return Err(
                        "Steam's Market search isn't answering right now. Prices will be read on the next scan."
                            .to_string(),
                    );
                }
                break;
            };
            failures = 0;
            data.write(|tx| db::save_market_page(tx, &page.listings, currency, db::now()))?;
            start += market::SEARCH_PAGE;
            if start >= page.total || page.listings.is_empty() || start >= MAX_SEARCH_ITEMS {
                finished = true;
                break;
            }
            if ctx.pause_requested() {
                return Ok(RunEnd::Paused);
            }
        }
        if finished {
            data.write(|tx| db::finish_market_app(tx, *appid, started))?;
        }
        pages_left = pages_left.saturating_sub(expected);
        ctx.progress(i as u64 + 1, Some(timer.estimate(pages_left, period())));
    }

    // Stage 6: price histories that are missing or due. Items that change
    // cost to complete come first (their usual price decides whether an
    // outlying listing counts), then items the user owns (value), then the
    // rest. New histories come before monthly refreshes.
    let now = db::now();
    let rules = completion::Rules::from_settings(&state.settings());
    let snap = state.snapshot(rules)?;
    let mut rank: HashMap<&str, u8> = HashMap::new();
    for i in &snap.items {
        let Some(p) = i.price.as_ref() else { continue };
        let counts = snap
            .games
            .get(&i.appid)
            .is_some_and(|g| g.counts_for_account);
        let r = if i.counted && counts && i.have < i.needed {
            0
        } else if i.held > 0 {
            1
        } else {
            2
        };
        let e = rank.entry(p.hash_name.as_str()).or_insert(r);
        *e = (*e).min(r);
    }
    let mut histories = data.read(|c| db::history_targets(c, family_on, now))?;
    histories.sort_by_key(|(hash, read)| (*read, rank.get(hash.as_str()).copied().unwrap_or(2)));
    drop(rank);
    drop(snap);
    let total = histories.len() as u64;
    if total == 0 {
        return Ok(RunEnd::Done { seconds: 0 });
    }
    let first_fill = !data.read(db::has_any_history)?;
    if !resume && first_fill && total as f64 * SECONDS_PER_REQUEST > CONFIRM_SECONDS {
        return Ok(RunEnd::NeedsPriceConfirm { items: total });
    }

    // Price history is read without sign-in (see `fetch_history`).
    ctx.stage(6, "Reading price history…", total);
    let timer = Timer::new();
    let mut done = 0u64;
    for (hash, _) in &histories {
        if ctx.pause_requested() {
            return Ok(RunEnd::Paused);
        }
        if state.steam.is_blocked(Bucket::Market, false) {
            return Ok(waiting(state, false));
        }
        let t = Instant::now();
        let outcome = fetch_history(state, hash, cookie, notify).await?;
        timer.add(t);
        match outcome {
            History::Refused => return Ok(waiting(state, false)),
            History::Read(h) => save_history(state, hash, &h)?,
            History::Unreadable => {
                data.write(|tx| db::postpone_history(tx, hash, db::now() + RETRY_DAYS * DAY))?
            }
        }
        done += 1;
        ctx.progress(done, Some(timer.estimate(total - done, period())));
    }

    Ok(RunEnd::Done { seconds: 0 })
}

/// Average time per request so far, for the time left.
struct Timer {
    spent: std::sync::Mutex<(u32, f64)>,
}

impl Timer {
    fn new() -> Self {
        Self {
            spent: std::sync::Mutex::new((0, 0.0)),
        }
    }

    fn add(&self, started: Instant) {
        let mut s = self.spent.lock().unwrap_or_else(|e| e.into_inner());
        s.0 += 1;
        s.1 += started.elapsed().as_secs_f64();
    }

    /// Time left for `requests_left` more requests. Past the budget's short
    /// burst, requests go out no faster than the long-read pace.
    fn estimate(&self, requests_left: u64, period_secs: f64) -> u64 {
        let s = self.spent.lock().unwrap_or_else(|e| e.into_inner());
        let measured = if s.0 >= 3 {
            s.1 / f64::from(s.0)
        } else {
            SECONDS_PER_REQUEST
        };
        let per = if requests_left as f64 > crate::steam::budget::BURST {
            measured.max(period_secs)
        } else {
            measured
        };
        (requests_left as f64 * per).ceil() as u64
    }
}

// ---- Price history ------------------------------------------------------------------

pub(crate) struct ReadHistory {
    currency: i64,
    daily: Vec<series::Point>,
}

pub(crate) enum History {
    Read(ReadHistory),
    /// Steam asked the app to slow down.
    Refused,
    Unreadable,
}

/// One item's whole history, from the newer route without sign-in. Steam
/// refused signed-in price history after a day of reading (one request every
/// half hour for hours) while answering the same request without sign-in, so
/// it seems to limit it per account. The older route, which needs sign-in, is
/// only tried when the newer one answers with something unreadable.
async fn fetch_history(
    state: &AppState,
    hash: &str,
    cookie: Option<&str>,
    notify: Notify<'_>,
) -> Result<History, String> {
    let f = state.steam.market_history(hash, None, notify).await?;
    if f.status == 429 {
        return Ok(History::Refused);
    }
    let mut parsed = f
        .json()
        .as_ref()
        .filter(|_| f.is_success())
        .and_then(market::parse_history);
    if parsed.is_none() {
        if let Some(c) = cookie.filter(|_| !state.steam.is_blocked(Bucket::Market, true)) {
            let f = state.steam.market_history_old(hash, c, notify).await?;
            // A refusal here is the signed-in route's own; the item is tried
            // again later and the read carries on without sign-in.
            if f.status == 429 {
                return Ok(History::Unreadable);
            }
            parsed = f
                .json()
                .as_ref()
                .filter(|_| f.is_success())
                .and_then(market::parse_history);
        }
    }
    Ok(match parsed {
        Some((currency, raw)) => {
            if currency > 0 {
                remember_currency(state, currency);
            }
            History::Read(ReadHistory {
                currency,
                daily: series::daily(&raw),
            })
        }
        None => History::Unreadable,
    })
}

fn today() -> i64 {
    db::now().div_euclid(DAY)
}

fn save_history(state: &AppState, hash: &str, h: &ReadHistory) -> Result<(), String> {
    let now = db::now();
    let summary = series::summarize(&h.daily, today());
    let recent = series::encode(&series::recent(&h.daily, today()));
    let currency = if h.currency > 0 {
        h.currency
    } else {
        state.db.read(market_currency)?.unwrap_or(1)
    };
    state.db.write(|tx| {
        db::save_history(
            tx,
            hash,
            &HistorySave {
                currency,
                summary: &summary,
                recent: &recent,
                due: next_due(hash, now),
            },
            now,
        )
    })
}

#[derive(Serialize, Clone, Debug)]
pub struct ChartPoint {
    /// Unix time of the point's first day.
    pub t: i64,
    /// Median sale price, in mills of the shown currency.
    pub price: i64,
    pub volume: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct PriceHistory {
    pub points: Vec<ChartPoint>,
    /// The whole history rather than the last year.
    pub full: bool,
    pub currency: String,
    pub read_at: Option<i64>,
}

/// An item's price history for its detail view: the stored last year, read
/// from Steam first if the app doesn't have it yet, or the whole history,
/// always read fresh, when `full` is set.
pub async fn item_history(
    state: &AppState,
    hash: &str,
    full: bool,
    cookie: Option<&str>,
) -> Result<PriceHistory, String> {
    let rules = completion::Rules::from_settings(&state.settings());
    let stored = if full {
        None
    } else {
        state.db.read(|c| db::recent_history(c, hash))?
    };
    let (points, currency) =
        match stored {
            Some((blob, currency)) => (series::decode(&blob).unwrap_or_default(), currency),
            None => {
                if !state.db.read(|c| db::market_has(c, hash))? {
                    return Err("That item isn't on the Market.".to_string());
                }
                let quiet = |_: String| {};
                restore_market(state);
                let outcome = fetch_history(state, hash, cookie, &quiet).await;
                save_market(state);
                match outcome? {
                    History::Refused => return Err(slow_down_text(state, false)),
                    History::Unreadable => return Err(
                        "Steam's price history for this item couldn't be read. Try again later."
                            .to_string(),
                    ),
                    History::Read(h) => {
                        save_history(state, hash, &h)?;
                        let currency = if h.currency > 0 {
                            h.currency
                        } else {
                            state.db.read(market_currency)?.unwrap_or(1)
                        };
                        let shaped = if full {
                            series::for_chart(&h.daily, today())
                        } else {
                            series::recent(&h.daily, today())
                        };
                        (shaped, currency)
                    }
                }
            }
        };
    let conv = state
        .db
        .read(|c| completion::Converter::load(c, rules.prices.currency))?;
    Ok(PriceHistory {
        points: points
            .iter()
            .filter_map(|p| {
                Some(ChartPoint {
                    t: p.day * DAY,
                    price: conv.convert(p.price, currency)?,
                    volume: p.volume,
                })
            })
            .collect(),
        full,
        currency: market::currency_code(conv.shown())
            .unwrap_or("USD")
            .to_string(),
        read_at: Some(db::now()),
    })
}

// ---- Order books ----------------------------------------------------------------------

/// One game's order books are read at a time, so opening several games
/// doesn't send their requests side by side.
static BOOKS: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Serialize, Clone, Debug)]
pub struct BookProgress {
    pub appid: u64,
    pub done: u64,
    pub total: u64,
}

/// Reads the order books of a game's cards that still need two or more
/// copies, when the user opens the game, so its cost to complete walks up
/// the listings. Books read in the last six days are kept. `progress` is
/// called before each request. Returns how many books were read.
pub async fn game_books(
    state: &AppState,
    appid: u64,
    cookie: Option<&str>,
    progress: &(dyn Fn(BookProgress) + Send + Sync),
) -> Result<u64, String> {
    let _one_at_a_time = BOOKS.lock().await;
    restore_market(state);
    let now = db::now();
    let rules = completion::Rules::from_settings(&state.settings());
    let books: Vec<String> = state
        .snapshot(rules)?
        .items
        .iter()
        .filter(|i| {
            i.appid == appid
                && i.counted
                && i.group == Group::Cards
                && i.needed.saturating_sub(i.have) >= 2
        })
        .filter_map(|i| i.price.as_ref())
        .filter(|p| p.on_market && p.book_at.is_none_or(|t| now - t > BOOK_STALE_DAYS * DAY))
        .map(|p| p.hash_name.clone())
        .collect();
    let signed_in = cookie.is_some();
    let total = books.len() as u64;
    let quiet = |_: String| {};
    let mut read = 0u64;
    let mut outcome = Ok(());
    for (i, hash) in books.iter().enumerate() {
        progress(BookProgress {
            appid,
            done: i as u64,
            total,
        });
        if state.steam.is_blocked(Bucket::Market, signed_in) {
            outcome = Err(slow_down_text(state, signed_in));
            break;
        }
        let f = match state.steam.market_book(hash, cookie, &quiet).await {
            Ok(f) => f,
            Err(e) => {
                outcome = Err(e);
                break;
            }
        };
        if f.status == 429 {
            outcome = Err(slow_down_text(state, signed_in));
            break;
        }
        if let Some(b) = f
            .json()
            .as_ref()
            .filter(|_| f.is_success())
            .and_then(market::parse_book)
        {
            state
                .db
                .write(|tx| db::save_book(tx, hash, &b, db::now()))?;
            read += 1;
        }
    }
    if total > 0 {
        save_market(state);
        progress(BookProgress {
            appid,
            done: total,
            total,
        });
    }
    outcome.map(|()| read)
}

// ---- Exchange rates -------------------------------------------------------------------

/// Reads exchange rates when prices are shown in a currency other than
/// Steam's, at most once a day.
pub async fn refresh_rates(state: &AppState) -> Result<(), String> {
    let wanted = market::currency_id(&state.settings().currency).unwrap_or(1);
    let main = state.db.read(market_currency)?.unwrap_or(1);
    if wanted == main {
        return Ok(());
    }
    let Some(base) = market::currency_code(main) else {
        return Ok(());
    };
    let saved = state
        .db
        .read(|c| db::get_meta(c, market::META_RATES))?
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let Some(v) = &saved {
        let fresh = v["fetched_at"]
            .as_i64()
            .is_some_and(|t| db::now() - t < DAY);
        if fresh && v["base"].as_str() == Some(base) {
            return Ok(());
        }
    }
    let url = format!("{RATES_URL}?base={}", base.to_ascii_lowercase());
    let failed = || {
        "Exchange rates couldn't be read, so prices stay in Steam's currency for now.".to_string()
    };
    let resp = state
        .steam
        .http()
        .get(&url)
        .send()
        .await
        .map_err(|_| failed())?;
    if !resp.status().is_success() {
        return Err(failed());
    }
    let v: Value = resp.json().await.map_err(|_| failed())?;
    let rates = market::parse_rates(&v).ok_or_else(failed)?;
    let text = json!({"base": base, "fetched_at": db::now(), "rates": rates}).to_string();
    state
        .db
        .write(|tx| db::set_meta(tx, market::META_RATES, &text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histories_fall_due_spread_over_two_weeks() {
        let now = 1_000_000;
        let dues: Vec<i64> = (0..200)
            .map(|i| next_due(&format!("730-item {i}"), now))
            .collect();
        assert!(dues
            .iter()
            .all(|d| (23 * DAY..=37 * DAY).contains(&(d - now))));
        let distinct: std::collections::HashSet<i64> = dues.iter().map(|d| d / DAY).collect();
        assert!(distinct.len() > 10, "{} different days", distinct.len());
        assert_eq!(next_due("a", now), next_due("a", now));
    }
}
