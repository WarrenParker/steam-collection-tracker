//! What changed between scans, for the dashboard: items gained or lost, and
//! the account's figures over time, one row a day.
//!
//! After a scan reads the inventory, each item's copies are compared with
//! what the previous scan saw (`owned_seen`), and every difference is added
//! to `changes`. The first scan only sets the starting point. Badge levels
//! aren't compared on their own, so a game whose item list is read for the
//! first time doesn't look like a pile of new items.

use std::collections::HashMap;

use rusqlite::{params, Connection, Transaction};
use serde::Serialize;

use crate::completion::{self, AccountCompletion, Snapshot};
use crate::db;
use crate::state::AppState;

const DAY: i64 = 86_400;
/// Changes older than this are dropped.
const KEEP_DAYS: i64 = 365;

/// One item's copies going up or down between two scans.
#[derive(Serialize, Clone, Debug)]
pub struct Change {
    pub at: i64,
    pub key: String,
    pub appid: u64,
    pub name: String,
    pub before: u64,
    pub after: u64,
}

/// The account's figures on one day.
#[derive(Serialize, Clone, Debug)]
pub struct ProgressPoint {
    /// Unix time of the day's last scan.
    pub at: i64,
    pub percent: Option<f64>,
    pub complete: u64,
    pub total: u64,
    pub game_average: Option<f64>,
    pub currency: String,
    pub value: Option<i64>,
    pub cost: Option<i64>,
    pub cost_points: Option<i64>,
}

/// Records what changed since the last scan (when this one read the
/// inventory) and today's figures.
pub fn record(state: &AppState, inventory_read: bool) -> Result<(), String> {
    let rules = completion::Rules::from_settings(&state.settings());
    let snap = state.snapshot(rules)?;
    let now = db::now();
    state.db.write(|tx| {
        if inventory_read {
            record_changes(tx, &snap, now)?;
        }
        record_progress(tx, &snap.account, now)
    })
}

fn record_changes(tx: &Transaction<'_>, snap: &Snapshot, now: i64) -> rusqlite::Result<()> {
    let mut seen: HashMap<String, (u64, String, u64)> = HashMap::new();
    {
        let mut st = tx.prepare("SELECT key, appid, name, held FROM owned_seen")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as u64,
                ),
            ))
        })?;
        for row in rows {
            let (key, v) = row?;
            seen.insert(key, v);
        }
    }
    // Nothing held at all after holding things looks like a read that went
    // wrong, so keep the last good starting point.
    if held_now_is_empty(snap) && !seen.is_empty() {
        return Ok(());
    }
    let first = seen.is_empty();
    let held_now: HashMap<&str, (u64, &str, u64)> = snap
        .items
        .iter()
        .filter(|i| i.held > 0)
        .map(|i| (i.key.as_str(), (i.appid, i.name.as_str(), i.held)))
        .collect();

    if !first {
        let mut add = tx.prepare(
            "INSERT INTO changes (at, key, appid, name, held_before, held_after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (key, (appid, name, held)) in &held_now {
            let before = seen.get(*key).map_or(0, |s| s.2);
            if before != *held {
                add.execute(params![
                    now,
                    key,
                    *appid as i64,
                    name,
                    before as i64,
                    *held as i64
                ])?;
            }
        }
        // Items held before and not now: sold, traded, used up, or crafted
        // into a badge.
        for (key, (appid, name, before)) in &seen {
            if !held_now.contains_key(key.as_str()) && *before > 0 {
                add.execute(params![now, key, *appid as i64, name, *before as i64, 0i64])?;
            }
        }
    }

    tx.execute("DELETE FROM owned_seen", [])?;
    let mut put =
        tx.prepare("INSERT INTO owned_seen (key, appid, name, held) VALUES (?1, ?2, ?3, ?4)")?;
    for (key, (appid, name, held)) in &held_now {
        put.execute(params![key, *appid as i64, name, *held as i64])?;
    }
    tx.execute("DELETE FROM changes WHERE at < ?1", [now - KEEP_DAYS * DAY])?;
    Ok(())
}

fn held_now_is_empty(snap: &Snapshot) -> bool {
    snap.items.iter().all(|i| i.held == 0)
}

/// Today's number in the computer's own time zone, so an evening scan counts
/// for that evening rather than the next day.
fn local_day(now: i64) -> i64 {
    let offset = i64::from(chrono::Local::now().offset().local_minus_utc());
    (now + offset).div_euclid(DAY)
}

fn record_progress(tx: &Transaction<'_>, a: &AccountCompletion, now: i64) -> rusqlite::Result<()> {
    if a.total == 0 {
        return Ok(());
    }
    let priced = a.prices_at.is_some();
    tx.execute(
        "INSERT OR REPLACE INTO progress
             (day, at, percent, owned, complete, total, game_average, currency, value, cost, cost_points)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            local_day(now),
            now,
            a.percent,
            a.owned,
            a.complete as i64,
            a.total as i64,
            a.game_average,
            a.currency,
            priced.then_some(a.prices.value),
            priced.then_some(a.prices.cost),
            priced.then_some(a.prices.cost_points),
        ],
    )?;
    Ok(())
}

/// The most recent changes, newest first.
pub fn recent_changes(c: &Connection, limit: u32) -> rusqlite::Result<Vec<Change>> {
    let mut st = c.prepare(
        "SELECT at, key, appid, name, held_before, held_after FROM changes
         ORDER BY at DESC, id DESC LIMIT ?1",
    )?;
    let rows = st.query_map([limit], |r| {
        Ok(Change {
            at: r.get(0)?,
            key: r.get(1)?,
            appid: r.get::<_, i64>(2)? as u64,
            name: r.get(3)?,
            before: r.get::<_, i64>(4)? as u64,
            after: r.get::<_, i64>(5)? as u64,
        })
    })?;
    rows.collect()
}

/// The account's figures, one point a day, oldest first, since `since`.
pub fn progress(c: &Connection, since: i64) -> rusqlite::Result<Vec<ProgressPoint>> {
    let mut st = c.prepare(
        "SELECT at, percent, complete, total, game_average, currency, value, cost, cost_points
         FROM progress WHERE at >= ?1 ORDER BY day",
    )?;
    let rows = st.query_map([since], |r| {
        Ok(ProgressPoint {
            at: r.get(0)?,
            percent: r.get(1)?,
            complete: r.get::<_, i64>(2)? as u64,
            total: r.get::<_, i64>(3)? as u64,
            game_average: r.get(4)?,
            currency: r.get(5)?,
            value: r.get(6)?,
            cost: r.get(7)?,
            cost_points: r.get(8)?,
        })
    })?;
    rows.collect()
}
