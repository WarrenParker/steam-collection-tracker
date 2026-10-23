//! The collection database: every game the app knows about, each game's item
//! catalog, what the user owns, and a record of every scan. It's a single
//! SQLite file in the data folder. Almost everything in it comes from Steam
//! and can be rebuilt by scanning again.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::catalog::library::{FamilyApp, OwnedProfileItem};
use crate::catalog::points::PointsItem;
use crate::catalog::store::StoreApp;
use crate::catalog::{class, kind_label, CatalogItem};
use crate::steam::inventory::InvRow;
use crate::steam::{Badge, OwnedGame};

pub const FILE_NAME: &str = "collection.db";
const SCHEMA_VERSION: i64 = 4;

/// The app's own "game" for Steam-wide items such as gems.
pub const STEAM_APP: i64 = 753;

const SCHEMA_V1: &str = r#"
BEGIN;

CREATE TABLE apps (
    appid              INTEGER PRIMARY KEY,
    name               TEXT,
    owned              INTEGER NOT NULL DEFAULT 0,
    family             INTEGER NOT NULL DEFAULT 0,
    has_items          INTEGER NOT NULL DEFAULT 0,
    playtime_minutes   INTEGER NOT NULL DEFAULT 0,
    store_found        INTEGER,
    store_type         INTEGER,
    has_cards          INTEGER,
    event_app          INTEGER NOT NULL DEFAULT 0,
    store_checked_at   INTEGER,
    points_checked_at  INTEGER,
    catalog_checked_at INTEGER,
    catalog_source     TEXT,
    catalog_error      TEXT
);

CREATE TABLE items (
    appid          INTEGER NOT NULL,
    class          INTEGER NOT NULL,
    item_type      INTEGER NOT NULL,
    foil           INTEGER NOT NULL DEFAULT 0,
    name           TEXT,
    title          TEXT,
    description    TEXT,
    image          TEXT,
    image_large    TEXT,
    animated       INTEGER NOT NULL DEFAULT 0,
    series         INTEGER,
    active         INTEGER,
    in_catalog     INTEGER NOT NULL DEFAULT 0,
    catalog_source TEXT,
    in_points      INTEGER NOT NULL DEFAULT 0,
    points_defid   INTEGER,
    points_cost    INTEGER,
    points_active  INTEGER,
    points_from    INTEGER,
    points_until   INTEGER,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (appid, class, item_type, foil)
) WITHOUT ROWID;

CREATE TABLE inventory (
    classid          TEXT NOT NULL,
    instanceid       TEXT NOT NULL,
    appid            INTEGER,
    class            INTEGER,
    item_type        INTEGER,
    foil             INTEGER NOT NULL DEFAULT 0,
    name             TEXT,
    market_hash_name TEXT,
    type_label       TEXT,
    marketable       INTEGER,
    tradable         INTEGER,
    icon             TEXT,
    stacks           INTEGER NOT NULL,
    quantity         INTEGER NOT NULL,
    PRIMARY KEY (classid, instanceid)
) WITHOUT ROWID;
CREATE INDEX inventory_by_app ON inventory (appid, class);

CREATE TABLE profile_items (
    source    TEXT NOT NULL,
    appid     INTEGER NOT NULL,
    class     INTEGER NOT NULL,
    item_type INTEGER,
    name      TEXT NOT NULL,
    title     TEXT,
    quantity  INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (source, appid, class, name)
) WITHOUT ROWID;

CREATE TABLE badges (
    appid           INTEGER NOT NULL,
    badgeid         INTEGER NOT NULL,
    foil            INTEGER NOT NULL,
    level           INTEGER NOT NULL,
    xp              INTEGER,
    completion_time INTEGER,
    PRIMARY KEY (appid, badgeid, foil)
) WITHOUT ROWID;

CREATE TABLE scans (
    id          INTEGER PRIMARY KEY,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    status      TEXT NOT NULL,
    requests    INTEGER NOT NULL DEFAULT 0,
    summary     TEXT
);

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

PRAGMA user_version = 1;
COMMIT;
"#;

/// 0.3.1: catalogs read from card pages in 0.3.0 numbered cards by their place
/// in the set, so they're read again with Steam's numbers. Games also get
/// achievement progress.
const MIGRATE_V2: &str = "
BEGIN;
UPDATE apps SET catalog_checked_at = NULL WHERE catalog_source = 'card_page';
ALTER TABLE apps ADD COLUMN ach_unlocked INTEGER;
ALTER TABLE apps ADD COLUMN ach_total INTEGER;
ALTER TABLE apps ADD COLUMN ach_checked_at INTEGER;
PRAGMA user_version = 2;
COMMIT;
";

/// 0.5.0: Points Shop items record whether they can be bought on their own,
/// so every game's Points Shop is read again. Users can set an item's
/// availability themselves.
const MIGRATE_V3: &str = "
BEGIN;
ALTER TABLE items ADD COLUMN points_buyable INTEGER;
CREATE TABLE item_overrides (
    key          TEXT PRIMARY KEY,
    availability TEXT NOT NULL,
    set_at       INTEGER NOT NULL
) WITHOUT ROWID;
UPDATE apps SET points_checked_at = NULL;
PRAGMA user_version = 3;
COMMIT;
";

/// 0.6.0: Market prices. One row per item Steam lists on the Market, with its
/// cheapest listing, a summary of its price history and a compact recent
/// stretch of it, and its order book. Games record when their Market search
/// last finished.
const MIGRATE_V4: &str = "
BEGIN;
ALTER TABLE apps ADD COLUMN market_checked_at INTEGER;
CREATE TABLE market (
    hash_name        TEXT PRIMARY KEY,
    appid            INTEGER NOT NULL,
    class            INTEGER NOT NULL,
    foil             INTEGER NOT NULL DEFAULT 0,
    name             TEXT NOT NULL,
    type_label       TEXT,
    icon             TEXT,
    currency         INTEGER NOT NULL DEFAULT 0,
    lowest           INTEGER,
    listings         INTEGER NOT NULL DEFAULT 0,
    seen_at          INTEGER NOT NULL,
    history_at       INTEGER,
    history_due      INTEGER,
    history_currency INTEGER,
    avg_30           INTEGER,
    avg_90           INTEGER,
    avg_365          INTEGER,
    avg_all          INTEGER,
    low_price        INTEGER,
    low_day          INTEGER,
    high_price       INTEGER,
    high_day         INTEGER,
    sales_30         INTEGER,
    first_day        INTEGER,
    last_day         INTEGER,
    recent           BLOB,
    book_at          INTEGER,
    book_currency    INTEGER,
    book             TEXT,
    buy_max          INTEGER,
    sell_count       INTEGER,
    buy_count        INTEGER
);
CREATE INDEX market_by_app ON market (appid, class);
PRAGMA user_version = 4;
COMMIT;
";

pub struct Db {
    conn: Mutex<Connection>,
    /// Goes up with every saved change, so cached figures know when to refresh.
    generation: std::sync::atomic::AtomicU64,
}

impl Db {
    /// Opens the database, creating or upgrading it as needed. An unreadable
    /// file, or one written by a newer version of the app, is set aside as
    /// `collection.old.db` and a fresh one is started; scanning refills it.
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = match try_open(path) {
            Ok(conn) => conn,
            Err(_) => {
                let aside = path.with_extension("old.db");
                let _ = std::fs::remove_file(&aside);
                std::fs::rename(path, &aside).map_err(|e| {
                    format!("The collection database couldn't be opened or replaced: {e}")
                })?;
                try_open(path)
                    .map_err(|e| format!("Couldn't create the collection database: {e}"))?
            }
        };
        Ok(Self {
            conn: Mutex::new(conn),
            generation: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// A database that lives only while the app is open, for when the file
    /// can't be created at all.
    pub fn in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("an in-memory database always opens");
        conn.execute_batch(SCHEMA_V1).expect("the schema is valid");
        conn.execute_batch(MIGRATE_V2)
            .expect("the migration is valid");
        conn.execute_batch(MIGRATE_V3)
            .expect("the migration is valid");
        conn.execute_batch(MIGRATE_V4)
            .expect("the migration is valid");
        Self {
            conn: Mutex::new(conn),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Runs read-only work.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard).map_err(db_error)
    }

    /// Runs changes in one transaction: all of them are saved, or none.
    pub fn write<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = guard.transaction().map_err(db_error)?;
        let out = f(&tx).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(out)
    }

    /// Counts saved changes since the app started.
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn try_open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", (), |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version > SCHEMA_VERSION {
        return Err(format!("written by a newer version (schema {version})"));
    }
    if version < 1 {
        conn.execute_batch(SCHEMA_V1).map_err(|e| e.to_string())?;
    }
    if version < 2 {
        conn.execute_batch(MIGRATE_V2).map_err(|e| e.to_string())?;
    }
    if version < 3 {
        conn.execute_batch(MIGRATE_V3).map_err(|e| e.to_string())?;
    }
    if version < 4 {
        conn.execute_batch(MIGRATE_V4).map_err(|e| e.to_string())?;
    }
    Ok(conn)
}

fn db_error(e: rusqlite::Error) -> String {
    format!("The collection database had a problem: {e}")
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn opt_i64(v: Option<u64>) -> Option<i64> {
    v.map(|n| n as i64)
}

fn opt_bool(v: Option<bool>) -> Option<i64> {
    v.map(i64::from)
}

// ---- Library ---------------------------------------------------------------

pub struct LibraryUpdate<'a> {
    /// `None` when the user's game details are private.
    pub owned: Option<&'a [OwnedGame]>,
    /// `None` when the family library wasn't read this time.
    pub family: Option<&'a [FamilyApp]>,
    /// Games with inventory items or badges, with a name when one is known.
    pub item_apps: &'a BTreeMap<u64, Option<String>>,
    /// True when the inventory and badges were both read in full, so games
    /// missing from `item_apps` no longer have items.
    pub reset_items: bool,
}

pub fn save_library(c: &Connection, u: &LibraryUpdate<'_>) -> rusqlite::Result<()> {
    if let Some(owned) = u.owned {
        c.execute("UPDATE apps SET owned = 0, playtime_minutes = 0", ())?;
        let mut st = c.prepare(
            "INSERT INTO apps (appid, name, owned, playtime_minutes) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (appid) DO UPDATE SET owned = 1,
                 playtime_minutes = excluded.playtime_minutes,
                 name = COALESCE(excluded.name, apps.name)",
        )?;
        for g in owned {
            let name = Some(g.name.trim()).filter(|n| !n.is_empty());
            st.execute(params![g.appid as i64, name, g.playtime_minutes as i64])?;
        }
    }
    if let Some(family) = u.family {
        c.execute("UPDATE apps SET family = 0", ())?;
        let mut st = c.prepare(
            "INSERT INTO apps (appid, name, family) VALUES (?1, ?2, 1)
             ON CONFLICT (appid) DO UPDATE SET family = 1, name = COALESCE(apps.name, excluded.name)",
        )?;
        for a in family {
            st.execute(params![a.appid as i64, a.name])?;
        }
    }
    if u.reset_items {
        c.execute("UPDATE apps SET has_items = 0", ())?;
    }
    let mut st = c.prepare(
        "INSERT INTO apps (appid, name, has_items) VALUES (?1, ?2, 1)
         ON CONFLICT (appid) DO UPDATE SET has_items = 1, name = COALESCE(apps.name, excluded.name)",
    )?;
    for (appid, name) in u.item_apps {
        st.execute(params![*appid as i64, name])?;
    }
    Ok(())
}

pub fn replace_inventory<'a>(
    c: &Connection,
    rows: impl IntoIterator<Item = &'a InvRow>,
) -> rusqlite::Result<()> {
    c.execute("DELETE FROM inventory", ())?;
    let mut st = c.prepare(
        "INSERT INTO inventory (classid, instanceid, appid, class, item_type, foil, name,
             market_hash_name, type_label, marketable, tradable, icon, stacks, quantity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?;
    for r in rows {
        st.execute(params![
            r.classid,
            r.instanceid,
            opt_i64(r.appid),
            r.class,
            r.item_type,
            i64::from(r.foil),
            r.name,
            r.market_hash_name,
            r.type_label,
            opt_bool(r.marketable),
            opt_bool(r.tradable),
            r.icon,
            r.stacks as i64,
            r.quantity as i64,
        ])?;
    }
    Ok(())
}

pub fn replace_badges(c: &Connection, badges: &[Badge]) -> rusqlite::Result<()> {
    c.execute("DELETE FROM badges", ())?;
    let mut st = c.prepare(
        "INSERT OR REPLACE INTO badges (appid, badgeid, foil, level, xp, completion_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for b in badges {
        st.execute(params![
            b.appid.unwrap_or(0) as i64,
            b.badge_id.unwrap_or(0) as i64,
            i64::from(b.foil),
            b.level as i64,
            b.xp,
            b.completion_time,
        ])?;
    }
    Ok(())
}

/// Replaces one source's profile items: "emoticons" or "profile_items".
pub fn replace_profile_items(
    c: &Connection,
    source: &str,
    items: &[OwnedProfileItem],
) -> rusqlite::Result<()> {
    c.execute("DELETE FROM profile_items WHERE source = ?1", [source])?;
    let mut st = c.prepare(
        "INSERT INTO profile_items (source, appid, class, item_type, name, title, quantity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (source, appid, class, name) DO UPDATE SET quantity = quantity + excluded.quantity",
    )?;
    for i in items {
        st.execute(params![
            source,
            i.appid as i64,
            i.class,
            i.item_type,
            i.name,
            i.title,
            i.quantity,
        ])?;
    }
    Ok(())
}

// ---- Which games need what --------------------------------------------------

/// The games the app lists: owned, unowned with items, and (when the setting
/// is on) Steam Family games.
const LISTED: &str = "a.appid <> 753 AND (a.owned = 1 OR a.has_items = 1 OR (a.family = 1 AND ?1))";

fn appids(c: &Connection, sql: &str, family: bool, before: i64) -> rusqlite::Result<Vec<u64>> {
    let mut st = c.prepare(sql)?;
    let rows = st.query_map(params![family, before], |r| r.get::<_, i64>(0))?;
    rows.map(|r| r.map(|a| a as u64)).collect()
}

/// Listed games whose store details are missing or older than `before`.
pub fn apps_needing_store(c: &Connection, family: bool, before: i64) -> rusqlite::Result<Vec<u64>> {
    appids(
        c,
        &format!(
            "SELECT a.appid FROM apps a WHERE {LISTED}
             AND (a.store_checked_at IS NULL OR a.store_checked_at < ?2) ORDER BY a.appid"
        ),
        family,
        before,
    )
}

pub fn save_store(c: &Connection, checked: &[u64], found: &[StoreApp]) -> rusqlite::Result<()> {
    let t = now();
    let mut mark = c.prepare("UPDATE apps SET store_checked_at = ?2 WHERE appid = ?1")?;
    for appid in checked {
        mark.execute(params![*appid as i64, t])?;
    }
    let mut st = c.prepare(
        "UPDATE apps SET store_found = ?2, store_type = ?3,
             name = COALESCE(name, ?4),
             has_cards = CASE WHEN ?5 IS NULL THEN has_cards ELSE ?5 END
         WHERE appid = ?1",
    )?;
    for s in found {
        st.execute(params![
            s.appid as i64,
            i64::from(s.found),
            s.kind,
            s.name,
            opt_bool(s.has_cards),
        ])?;
    }
    Ok(())
}

/// Listed games whose Points Shop items are missing or older than `before`.
pub fn apps_needing_points(
    c: &Connection,
    family: bool,
    before: i64,
) -> rusqlite::Result<Vec<u64>> {
    appids(
        c,
        &format!(
            "SELECT a.appid FROM apps a WHERE {LISTED}
             AND (a.points_checked_at IS NULL OR a.points_checked_at < ?2) ORDER BY a.appid"
        ),
        family,
        before,
    )
}

/// Saves the Points Shop items for a batch of games, replacing what was saved before.
pub fn save_points(c: &Connection, appids: &[u64], items: &[PointsItem]) -> rusqlite::Result<()> {
    let t = now();
    let mut clear = c.prepare(
        "UPDATE items SET in_points = 0, points_defid = NULL, points_cost = NULL,
             points_active = NULL, points_from = NULL, points_until = NULL, points_buyable = NULL
         WHERE appid = ?1",
    )?;
    for appid in appids {
        clear.execute([*appid as i64])?;
    }
    let mut st = c.prepare(
        "INSERT INTO items (appid, class, item_type, foil, name, title, description, image, image_large,
             animated, in_points, points_defid, points_cost, points_active, points_from, points_until,
             points_buyable, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11, ?12, ?13, ?14, ?16, ?15)
         ON CONFLICT (appid, class, item_type, foil) DO UPDATE SET
             in_points = 1,
             points_buyable = excluded.points_buyable,
             points_defid = excluded.points_defid,
             points_cost = excluded.points_cost,
             points_active = excluded.points_active,
             points_from = excluded.points_from,
             points_until = excluded.points_until,
             name = COALESCE(items.name, excluded.name),
             title = COALESCE(items.title, excluded.title),
             description = COALESCE(items.description, excluded.description),
             image = COALESCE(items.image, excluded.image),
             image_large = COALESCE(items.image_large, excluded.image_large),
             animated = MAX(items.animated, excluded.animated),
             updated_at = excluded.updated_at",
    )?;
    for p in items {
        let i = &p.item;
        st.execute(params![
            p.appid as i64,
            i.class,
            i.item_type,
            i.name,
            i.title,
            i.description,
            i.image,
            i.image_large,
            i64::from(i.animated),
            p.defid,
            p.cost,
            opt_bool(p.active),
            p.available_from,
            p.available_until,
            t,
            p.buyable,
        ])?;
    }
    let mut tidy =
        c.prepare("DELETE FROM items WHERE appid = ?1 AND in_catalog = 0 AND in_points = 0")?;
    let mut mark = c.prepare("UPDATE apps SET points_checked_at = ?2 WHERE appid = ?1")?;
    for appid in appids {
        tidy.execute([*appid as i64])?;
        mark.execute(params![*appid as i64, t])?;
    }
    Ok(())
}

/// Marks the games that are Steam sale events rather than games.
pub fn save_event_apps(c: &Connection, apps: &[(u64, bool)]) -> rusqlite::Result<()> {
    c.execute("UPDATE apps SET event_app = 0", ())?;
    let mut st = c.prepare("UPDATE apps SET event_app = 1 WHERE appid = ?1")?;
    for (appid, event) in apps {
        if *event {
            st.execute([*appid as i64])?;
        }
    }
    Ok(())
}

/// Games whose Points Shop is read again on this scan, unless it was read
/// after `before`: used for games where the user owns an item no list has.
pub fn expire_points(c: &Connection, appids: &[u64], before: i64) -> rusqlite::Result<()> {
    let mut st = c.prepare(
        "UPDATE apps SET points_checked_at = NULL WHERE appid = ?1 AND points_checked_at < ?2",
    )?;
    for appid in appids {
        st.execute(params![*appid as i64, before])?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CatalogTarget {
    pub appid: u64,
    /// What the store said; `None` when it couldn't tell.
    pub has_cards: Option<bool>,
    /// The user has a badge or card-set items for the game, so it has cards.
    pub evidence: bool,
}

impl CatalogTarget {
    /// Whether the game needs its card page read: the store says it has cards,
    /// or the user has cards, badges or card-set items for it.
    pub fn wanted(&self) -> bool {
        self.has_cards == Some(true) || self.evidence
    }
}

/// Listed games whose catalog is missing or older than `before`.
pub fn catalog_targets(
    c: &Connection,
    family: bool,
    before: i64,
) -> rusqlite::Result<Vec<CatalogTarget>> {
    let sql = format!(
        "SELECT a.appid, a.has_cards,
             EXISTS (SELECT 1 FROM badges b WHERE b.appid = a.appid)
             OR EXISTS (SELECT 1 FROM inventory i WHERE i.appid = a.appid AND i.class IN (2, 3, 4, 5))
         FROM apps a
         WHERE {LISTED} AND a.event_app = 0
           AND (a.catalog_checked_at IS NULL OR a.catalog_checked_at < ?2)
         ORDER BY a.owned DESC, a.has_items DESC, a.appid"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params![family, before], |r| {
        Ok(CatalogTarget {
            appid: r.get::<_, i64>(0)? as u64,
            has_cards: r.get::<_, Option<i64>>(1)?.map(|v| v != 0),
            evidence: r.get::<_, i64>(2)? != 0,
        })
    })?;
    rows.collect()
}

/// Saves one game's catalog from `source` ("defs" or "card_page"),
/// replacing what that game's catalog held before.
pub fn save_catalog(
    c: &Connection,
    appid: u64,
    items: &[CatalogItem],
    source: &str,
) -> rusqlite::Result<()> {
    let t = now();
    let app = appid as i64;
    c.execute("UPDATE items SET in_catalog = 0 WHERE appid = ?1", [app])?;
    let mut st = c.prepare(
        "INSERT INTO items (appid, class, item_type, foil, name, title, description, image, image_large,
             animated, series, active, in_catalog, catalog_source, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?14)
         ON CONFLICT (appid, class, item_type, foil) DO UPDATE SET
             in_catalog = 1,
             catalog_source = excluded.catalog_source,
             name = COALESCE(excluded.name, items.name),
             title = COALESCE(excluded.title, items.title),
             description = COALESCE(excluded.description, items.description),
             image = COALESCE(excluded.image, items.image),
             image_large = COALESCE(excluded.image_large, items.image_large),
             animated = MAX(items.animated, excluded.animated),
             series = COALESCE(excluded.series, items.series),
             active = COALESCE(excluded.active, items.active),
             updated_at = excluded.updated_at",
    )?;
    for i in items {
        st.execute(params![
            app,
            i.class,
            i.item_type,
            i64::from(i.foil),
            i.name,
            i.title,
            i.description,
            i.image,
            i.image_large,
            i64::from(i.animated),
            i.series,
            opt_bool(i.active),
            source,
            t,
        ])?;
    }
    let has_cards = items.iter().any(|i| i.class == class::CARD);
    if has_cards && !items.iter().any(|i| i.class == class::BOOSTER) {
        // Every game with cards has one booster pack. Steam's item lists don't
        // include it, so it's added here.
        let name: Option<String> = c
            .query_row("SELECT name FROM apps WHERE appid = ?1", [app], |r| {
                r.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten();
        let title = match name {
            Some(n) => format!("{n} Booster Pack"),
            None => "Booster Pack".to_string(),
        };
        c.execute(
            "INSERT INTO items (appid, class, item_type, foil, title, in_catalog, catalog_source, updated_at)
             VALUES (?1, ?2, 0, 0, ?3, 1, 'derived', ?4)
             ON CONFLICT (appid, class, item_type, foil) DO UPDATE SET in_catalog = 1, updated_at = excluded.updated_at",
            params![app, class::BOOSTER, title, t],
        )?;
    }
    c.execute(
        "DELETE FROM items WHERE appid = ?1 AND in_catalog = 0 AND in_points = 0",
        [app],
    )?;
    c.execute(
        "UPDATE apps SET catalog_checked_at = ?2, catalog_source = ?3, catalog_error = NULL,
             has_cards = CASE WHEN ?4 THEN 1 WHEN has_cards IS NULL THEN 0 ELSE has_cards END
         WHERE appid = ?1",
        params![app, t, source, has_cards],
    )?;
    Ok(())
}

/// Owned games whose achievement progress is missing or older than `before`.
pub fn apps_needing_achievements(c: &Connection, before: i64) -> rusqlite::Result<Vec<u64>> {
    let mut st = c.prepare(
        "SELECT appid FROM apps WHERE owned = 1 AND appid <> 753
         AND (ach_checked_at IS NULL OR ach_checked_at < ?1) ORDER BY appid",
    )?;
    let rows = st.query_map([before], |r| r.get::<_, i64>(0))?;
    rows.map(|r| r.map(|a| a as u64)).collect()
}

/// Saves achievement progress: (game, unlocked, total) for each game asked about.
pub fn save_achievements(
    c: &Connection,
    asked: &[u64],
    progress: &[(u64, u64, u64)],
) -> rusqlite::Result<()> {
    let t = now();
    let mut mark = c.prepare(
        "UPDATE apps SET ach_checked_at = ?2, ach_unlocked = COALESCE(ach_unlocked, 0),
             ach_total = COALESCE(ach_total, 0)
         WHERE appid = ?1",
    )?;
    for appid in asked {
        mark.execute(params![*appid as i64, t])?;
    }
    let mut st = c.prepare("UPDATE apps SET ach_unlocked = ?2, ach_total = ?3 WHERE appid = ?1")?;
    for (appid, unlocked, total) in progress {
        st.execute(params![*appid as i64, *unlocked as i64, *total as i64])?;
    }
    Ok(())
}

/// Games where the user owns a card the catalog doesn't have under that
/// number are read again on this scan, so the catalog picks up the real number.
pub fn refresh_mismatched_catalogs(c: &Connection) -> rusqlite::Result<usize> {
    c.execute(
        "UPDATE apps SET catalog_checked_at = NULL
         WHERE catalog_checked_at IS NOT NULL AND appid IN (
             SELECT v.appid FROM inventory v
             WHERE v.class = 2 AND v.item_type IS NOT NULL AND NOT EXISTS (
                 SELECT 1 FROM items i
                 WHERE i.appid = v.appid AND i.class = 2
                   AND i.item_type = v.item_type AND i.foil = v.foil))",
        (),
    )
}

/// Names and Steam item numbers of the cards the user owns for a game.
pub fn owned_card_types(c: &Connection, appid: u64) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st = c.prepare(
        "SELECT name, item_type FROM inventory
         WHERE appid = ?1 AND class = 2 AND name IS NOT NULL AND item_type IS NOT NULL",
    )?;
    let rows = st.query_map([appid as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

pub fn save_catalog_error(c: &Connection, appid: u64, error: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE apps SET catalog_error = ?2 WHERE appid = ?1",
        params![appid as i64, error],
    )?;
    Ok(())
}

// ---- Market prices ------------------------------------------------------------

/// Kinds of item the Market search records against the catalog.
const MARKET_CLASSES: &str = "(2, 3, 4, 5)";

/// Games whose Market prices are missing or older than `before`: card games
/// that count for the account, and any game the user holds an item they could
/// sell from (so account value covers the whole inventory). Owned games first.
pub fn market_targets(c: &Connection, family: bool, before: i64) -> rusqlite::Result<Vec<u64>> {
    appids(
        c,
        "SELECT a.appid FROM apps a
         WHERE (a.market_checked_at IS NULL OR a.market_checked_at < ?2)
           AND ((a.appid <> 753 AND COALESCE(a.has_cards, 0) = 1
                 AND (a.owned = 1 OR (a.family = 1 AND ?1)))
             OR a.appid IN (SELECT v.appid FROM inventory v
                            WHERE v.marketable = 1 AND v.appid IS NOT NULL))
         ORDER BY a.owned DESC, a.appid",
        family,
        before,
    )
}

/// Makes sure every game the user holds a sellable item from has a row, so
/// its Market search can be recorded (Steam's own items use game 753).
pub fn ensure_inventory_apps(c: &Connection) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO apps (appid)
         SELECT DISTINCT appid FROM inventory WHERE marketable = 1 AND appid IS NOT NULL",
        (),
    )?;
    Ok(())
}

/// Roughly how many Market items a game has, for time estimates.
pub fn market_item_estimate(c: &Connection, appid: u64) -> rusqlite::Result<u64> {
    let n: i64 = c.query_row(
        &format!(
            "SELECT MAX(
                 (SELECT COUNT(*) FROM market m WHERE m.appid = ?1),
                 (SELECT COUNT(*) FROM items i WHERE i.appid = ?1 AND i.class IN {MARKET_CLASSES}
                      AND (i.class IN (2, 5) OR i.in_points = 0 OR i.animated = 0)))"
        ),
        [appid as i64],
        |r| r.get(0),
    )?;
    Ok(n.max(1) as u64)
}

/// Saves one page of a Market search.
pub fn save_market_page(
    c: &Connection,
    listings: &[crate::market::Listing],
    currency: i64,
    seen_at: i64,
) -> rusqlite::Result<()> {
    let mut st = c.prepare(
        "INSERT INTO market (hash_name, appid, class, foil, name, type_label, icon, currency, lowest, listings, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (hash_name) DO UPDATE SET
             appid = excluded.appid, class = excluded.class, foil = excluded.foil,
             name = excluded.name, type_label = excluded.type_label,
             icon = COALESCE(excluded.icon, icon), currency = excluded.currency,
             lowest = excluded.lowest, listings = excluded.listings, seen_at = excluded.seen_at",
    )?;
    for l in listings {
        st.execute(params![
            l.hash_name,
            l.appid as i64,
            l.class,
            i64::from(l.foil),
            l.name,
            l.type_label,
            l.icon,
            currency,
            l.lowest,
            l.listings,
            seen_at,
        ])?;
    }
    Ok(())
}

/// Records that a game's Market search finished. Items it didn't list since
/// `started` aren't for sale right now.
pub fn finish_market_app(c: &Connection, appid: u64, started: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO apps (appid, market_checked_at) VALUES (?1, ?2)
         ON CONFLICT (appid) DO UPDATE SET market_checked_at = excluded.market_checked_at",
        params![appid as i64, started],
    )?;
    Ok(())
}

/// Market items in games that count for the account whose price history is
/// missing or due, owned games first, and whether each has one already.
pub fn history_targets(
    c: &Connection,
    family: bool,
    now: i64,
) -> rusqlite::Result<Vec<(String, bool)>> {
    let mut st = c.prepare(&format!(
        "SELECT m.hash_name, m.history_at IS NOT NULL FROM market m JOIN apps a ON a.appid = m.appid
         WHERE m.class IN {MARKET_CLASSES}
           AND (a.owned = 1 OR (a.family = 1 AND ?1))
           AND (m.history_due IS NULL OR m.history_due <= ?2)
         ORDER BY a.owned DESC, m.history_at IS NOT NULL, m.appid, m.hash_name"
    ))?;
    let rows = st.query_map(params![family, now], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
    })?;
    rows.collect()
}

/// Whether the Market lists an item by this name.
pub fn market_has(c: &Connection, hash_name: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS (SELECT 1 FROM market WHERE hash_name = ?1)",
        [hash_name],
        |r| r.get(0),
    )
}

/// Items in games that count whose price history hasn't been read yet and
/// isn't waiting for a retry.
pub fn history_missing(c: &Connection, family: bool, now: i64) -> rusqlite::Result<u64> {
    let n: i64 = c.query_row(
        &format!(
            "SELECT COUNT(*) FROM market m JOIN apps a ON a.appid = m.appid
             WHERE m.class IN {MARKET_CLASSES} AND m.history_at IS NULL
               AND (m.history_due IS NULL OR m.history_due <= ?2)
               AND (a.owned = 1 OR (a.family = 1 AND ?1))"
        ),
        params![family, now],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

/// Whether any item has a price history yet.
pub fn has_any_history(c: &Connection) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS (SELECT 1 FROM market WHERE history_at IS NOT NULL)",
        (),
        |r| r.get(0),
    )
}

pub struct HistorySave<'a> {
    pub currency: i64,
    pub summary: &'a crate::market::series::Summary,
    pub recent: &'a [u8],
    pub due: i64,
}

pub fn save_history(
    c: &Connection,
    hash_name: &str,
    h: &HistorySave<'_>,
    now: i64,
) -> rusqlite::Result<()> {
    let s = h.summary;
    c.execute(
        "UPDATE market SET history_at = ?2, history_due = ?3, history_currency = ?4,
             avg_30 = ?5, avg_90 = ?6, avg_365 = ?7, avg_all = ?8,
             low_price = ?9, low_day = ?10, high_price = ?11, high_day = ?12,
             sales_30 = ?13, first_day = ?14, last_day = ?15, recent = ?16
         WHERE hash_name = ?1",
        params![
            hash_name,
            now,
            h.due,
            h.currency,
            s.avg_30,
            s.avg_90,
            s.avg_365,
            s.avg_all,
            s.low.map(|l| l.0),
            s.low.map(|l| l.1),
            s.high.map(|l| l.0),
            s.high.map(|l| l.1),
            s.sales_30,
            s.first_day,
            s.last_day,
            h.recent,
        ],
    )?;
    Ok(())
}

/// Tries an item's history again later, when Steam's answer couldn't be read.
pub fn postpone_history(c: &Connection, hash_name: &str, due: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE market SET history_due = ?2 WHERE hash_name = ?1",
        params![hash_name, due],
    )?;
    Ok(())
}

/// The stored recent history and its currency.
pub fn recent_history(c: &Connection, hash_name: &str) -> rusqlite::Result<Option<(Vec<u8>, i64)>> {
    c.query_row(
        "SELECT recent, COALESCE(history_currency, currency) FROM market
         WHERE hash_name = ?1 AND recent IS NOT NULL",
        [hash_name],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub fn save_book(
    c: &Connection,
    hash_name: &str,
    b: &crate::market::book::Book,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE market SET book_at = ?2, book_currency = ?3, book = ?4, buy_max = ?5,
             sell_count = ?6, buy_count = ?7
         WHERE hash_name = ?1",
        params![
            hash_name,
            now,
            b.currency,
            crate::market::book::to_text(&b.sell),
            b.buy_max,
            b.sell_count,
            b.buy_count,
        ],
    )?;
    Ok(())
}

/// A Market item as completion reads it. Prices are in mills of `currency`.
#[derive(Clone, Debug, Default)]
pub struct MarketRow {
    pub hash_name: String,
    pub appid: u64,
    pub class: i64,
    pub foil: bool,
    pub name: String,
    pub icon: Option<String>,
    pub currency: i64,
    pub lowest: Option<i64>,
    pub listings: i64,
    pub seen_at: i64,
    pub history_at: Option<i64>,
    pub history_currency: Option<i64>,
    pub avg_30: Option<i64>,
    pub avg_90: Option<i64>,
    pub avg_365: Option<i64>,
    pub avg_all: Option<i64>,
    pub low_price: Option<i64>,
    pub low_day: Option<i64>,
    pub high_price: Option<i64>,
    pub high_day: Option<i64>,
    pub sales_30: Option<i64>,
    pub book_at: Option<i64>,
    pub book_currency: Option<i64>,
    pub book: Option<String>,
    pub buy_max: Option<i64>,
}

pub fn market_rows(c: &Connection) -> rusqlite::Result<Vec<MarketRow>> {
    let mut st = c.prepare(
        "SELECT hash_name, appid, class, foil, name, icon, currency, lowest, listings, seen_at,
             history_at, history_currency, avg_30, avg_90, avg_365, avg_all,
             low_price, low_day, high_price, high_day, sales_30,
             book_at, book_currency, book, buy_max
         FROM market",
    )?;
    let rows = st.query_map((), |r| {
        Ok(MarketRow {
            hash_name: r.get(0)?,
            appid: r.get::<_, i64>(1)? as u64,
            class: r.get(2)?,
            foil: r.get::<_, i64>(3)? != 0,
            name: r.get(4)?,
            icon: r.get(5)?,
            currency: r.get(6)?,
            lowest: r.get(7)?,
            listings: r.get(8)?,
            seen_at: r.get(9)?,
            history_at: r.get(10)?,
            history_currency: r.get(11)?,
            avg_30: r.get(12)?,
            avg_90: r.get(13)?,
            avg_365: r.get(14)?,
            avg_all: r.get(15)?,
            low_price: r.get(16)?,
            low_day: r.get(17)?,
            high_price: r.get(18)?,
            high_day: r.get(19)?,
            sales_30: r.get(20)?,
            book_at: r.get(21)?,
            book_currency: r.get(22)?,
            book: r.get(23)?,
            buy_max: r.get(24)?,
        })
    })?;
    rows.collect()
}

/// Sellable copies of Steam's own items (game 753, like sacks of gems), by
/// Market name, for the account's value.
pub fn steam_item_copies(c: &Connection) -> rusqlite::Result<Vec<(String, u64)>> {
    let mut st = c.prepare(
        "SELECT market_hash_name, SUM(quantity) FROM inventory
         WHERE appid = 753 AND marketable = 1 AND market_hash_name IS NOT NULL
         GROUP BY market_hash_name",
    )?;
    let rows = st.query_map((), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
    })?;
    rows.collect()
}

// ---- Meta values ------------------------------------------------------------

pub fn get_meta(c: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

pub fn set_meta(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---- Scans -------------------------------------------------------------------

pub fn begin_scan(c: &Connection) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO scans (started_at, status) VALUES (?1, 'running')",
        [now()],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn end_scan(
    c: &Connection,
    id: i64,
    status: &str,
    requests: u32,
    summary: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE scans SET finished_at = ?2, status = ?3, requests = requests + ?4,
             summary = COALESCE(?5, summary)
         WHERE id = ?1",
        params![id, now(), status, requests, summary],
    )?;
    Ok(())
}

/// Saves the overview summary partway through a scan.
pub fn save_scan_summary(c: &Connection, id: i64, summary: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE scans SET summary = ?2 WHERE id = ?1",
        params![id, summary],
    )?;
    Ok(())
}

pub fn last_summary(c: &Connection) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT summary FROM scans WHERE summary IS NOT NULL ORDER BY id DESC LIMIT 1",
        (),
        |r| r.get(0),
    )
    .optional()
}

/// Records a summary from before the database existed (version 0.2.0 kept it
/// in a file), so the overview has something to show until the next scan.
pub fn import_summary(c: &Connection, scanned_at: i64, summary: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO scans (started_at, finished_at, status, summary) VALUES (?1, ?1, 'done', ?2)",
        params![scanned_at, summary],
    )?;
    Ok(())
}

// ---- What the interface shows -------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct KindCount {
    pub label: String,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CatalogCounts {
    pub games_listed: u64,
    pub games_owned: u64,
    pub games_unowned: u64,
    pub games_family: u64,
    pub games_with_cards: u64,
    pub games_cataloged: u64,
    /// Catalog items across the listed games, badges not included.
    pub items: u64,
    pub points_items: u64,
    pub by_kind: Vec<KindCount>,
    /// Owned games that have any collectible items.
    #[serde(default)]
    pub owned_with_items: u64,
    /// Owned games with achievements, and how many of those are started.
    #[serde(default)]
    pub ach_games: u64,
    #[serde(default)]
    pub ach_started: u64,
    #[serde(default)]
    pub ach_perfect: u64,
    /// Average completion of started games, 0 to 100, as Steam counts it.
    #[serde(default)]
    pub ach_average: Option<f64>,
    #[serde(default)]
    pub ach_unlocked: u64,
    #[serde(default)]
    pub ach_total: u64,
}

pub fn catalog_counts(c: &Connection, family: bool) -> rusqlite::Result<CatalogCounts> {
    let mut counts = c.query_row(
        &format!(
            "SELECT COUNT(*),
                 SUM(a.owned = 1),
                 SUM(a.owned = 0 AND NOT (a.family = 1 AND ?1)),
                 SUM(a.owned = 0 AND a.family = 1 AND ?1),
                 SUM(a.has_cards = 1),
                 SUM(a.catalog_checked_at IS NOT NULL),
                 SUM(a.owned = 1 AND EXISTS (SELECT 1 FROM items i WHERE i.appid = a.appid AND i.class <> 1)),
                 SUM(a.owned = 1 AND a.ach_total > 0),
                 SUM(a.owned = 1 AND a.ach_unlocked > 0),
                 SUM(a.owned = 1 AND a.ach_total > 0 AND a.ach_unlocked >= a.ach_total),
                 AVG(CASE WHEN a.owned = 1 AND a.ach_unlocked > 0 AND a.ach_total > 0
                     THEN 100.0 * a.ach_unlocked / a.ach_total END),
                 SUM(CASE WHEN a.owned = 1 THEN a.ach_unlocked END),
                 SUM(CASE WHEN a.owned = 1 THEN a.ach_total END)
             FROM apps a WHERE {LISTED}"
        ),
        params![family],
        |r| {
            let n = |i: usize| -> rusqlite::Result<u64> {
                Ok(r.get::<_, Option<i64>>(i)?.unwrap_or(0).max(0) as u64)
            };
            Ok(CatalogCounts {
                games_listed: n(0)?,
                games_owned: n(1)?,
                games_unowned: n(2)?,
                games_family: n(3)?,
                games_with_cards: n(4)?,
                games_cataloged: n(5)?,
                owned_with_items: n(6)?,
                ach_games: n(7)?,
                ach_started: n(8)?,
                ach_perfect: n(9)?,
                ach_average: r.get::<_, Option<f64>>(10)?,
                ach_unlocked: n(11)?,
                ach_total: n(12)?,
                ..CatalogCounts::default()
            })
        },
    )?;

    let mut st = c.prepare(&format!(
        "SELECT i.class, i.foil, i.animated, COUNT(*), SUM(i.in_points)
         FROM items i JOIN apps a ON a.appid = i.appid
         WHERE {LISTED} AND i.class <> 1
         GROUP BY i.class, i.foil, i.animated"
    ))?;
    let mut by_label: BTreeMap<&'static str, u64> = BTreeMap::new();
    let rows = st.query_map(params![family], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)? != 0,
            r.get::<_, i64>(2)? != 0,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?.unwrap_or(0),
        ))
    })?;
    for row in rows {
        let (class_id, foil, animated, count, points) = row?;
        *by_label
            .entry(kind_label(class_id, foil, animated))
            .or_default() += count as u64;
        counts.items += count as u64;
        counts.points_items += points as u64;
    }
    let mut by_kind: Vec<KindCount> = by_label
        .into_iter()
        .map(|(label, count)| KindCount {
            label: label.to_string(),
            count,
        })
        .collect();
    by_kind.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    counts.by_kind = by_kind;
    Ok(counts)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GameRow {
    pub appid: u64,
    pub name: String,
    pub owned: bool,
    pub family: bool,
    pub event: bool,
    pub has_cards: Option<bool>,
    pub cataloged: bool,
    /// Cards in the set, not counting foils.
    pub cards: u64,
    /// Every item in the game's catalog, badges not included.
    pub items: u64,
    /// Of those, items sold in the Points Shop.
    pub points_items: u64,
    /// Items in the user's inventory for this game, counting each copy.
    pub your_items: u64,
    pub badge_level: Option<u64>,
    pub foil_badge: bool,
    pub playtime_minutes: u64,
    pub ach_unlocked: Option<u64>,
    pub ach_total: Option<u64>,
    /// Completion, filled in from the completion figures.
    pub percent: Option<f64>,
    pub complete_items: u64,
    pub counted_items: u64,
    pub cards_complete: bool,
    /// What the copies the user can sell are worth, in mills of the shown currency.
    pub value: Option<i64>,
    /// Money and points still needed to finish the game.
    pub cost: Option<i64>,
    pub cost_points: i64,
    /// The currency `value` and `cost` are in.
    pub currency: String,
}

pub fn games(c: &Connection, family: bool) -> rusqlite::Result<Vec<GameRow>> {
    let mut st = c.prepare(&format!(
        "SELECT a.appid, a.name, a.owned, a.family, a.event_app, a.has_cards,
             a.catalog_checked_at IS NOT NULL, a.playtime_minutes,
             (SELECT COUNT(*) FROM items i WHERE i.appid = a.appid AND i.class = 2 AND i.foil = 0),
             (SELECT COUNT(*) FROM items i WHERE i.appid = a.appid AND i.class <> 1),
             (SELECT COUNT(*) FROM items i WHERE i.appid = a.appid AND i.class <> 1 AND i.in_points = 1),
             (SELECT COALESCE(SUM(v.quantity), 0) FROM inventory v WHERE v.appid = a.appid),
             (SELECT MAX(b.level) FROM badges b WHERE b.appid = a.appid AND b.foil = 0),
             EXISTS (SELECT 1 FROM badges b WHERE b.appid = a.appid AND b.foil = 1),
             a.ach_unlocked, a.ach_total
         FROM apps a WHERE {LISTED}
         ORDER BY a.name COLLATE NOCASE, a.appid"
    ))?;
    let rows = st.query_map(params![family], |r| {
        let appid = r.get::<_, i64>(0)?;
        let count = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(i)?.max(0) as u64) };
        Ok(GameRow {
            appid: appid as u64,
            name: r
                .get::<_, Option<String>>(1)?
                .unwrap_or_else(|| format!("App {appid}")),
            owned: r.get::<_, i64>(2)? != 0,
            family: r.get::<_, i64>(3)? != 0,
            event: r.get::<_, i64>(4)? != 0,
            has_cards: r.get::<_, Option<i64>>(5)?.map(|v| v != 0),
            cataloged: r.get::<_, i64>(6)? != 0,
            playtime_minutes: count(7)?,
            cards: count(8)?,
            items: count(9)?,
            points_items: count(10)?,
            your_items: count(11)?,
            badge_level: r.get::<_, Option<i64>>(12)?.map(|l| l.max(0) as u64),
            foil_badge: r.get::<_, i64>(13)? != 0,
            ach_unlocked: r.get::<_, Option<i64>>(14)?.map(|v| v.max(0) as u64),
            ach_total: r.get::<_, Option<i64>>(15)?.map(|v| v.max(0) as u64),
            percent: None,
            complete_items: 0,
            counted_items: 0,
            cards_complete: false,
            value: None,
            cost: None,
            cost_points: 0,
            currency: String::new(),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::Badge;

    fn temp_db(name: &str) -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sct-db-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        (Db::open(&path).unwrap(), dir)
    }

    fn card(item_type: i64, foil: bool) -> CatalogItem {
        CatalogItem {
            class: class::CARD,
            item_type,
            foil,
            title: Some(format!("Card {item_type}")),
            ..CatalogItem::default()
        }
    }

    #[test]
    fn library_catalog_and_lists() {
        let (db, dir) = temp_db("lists");
        let owned = vec![
            OwnedGame {
                appid: 440,
                name: "Team Fortress 2".into(),
                playtime_minutes: 90,
            },
            OwnedGame {
                appid: 570,
                name: "Dota 2".into(),
                playtime_minutes: 0,
            },
        ];
        let family = vec![FamilyApp {
            appid: 10,
            name: Some("Counter-Strike".into()),
        }];
        let mut item_apps = BTreeMap::new();
        item_apps.insert(620u64, Some("Portal 2".to_string()));
        item_apps.insert(440u64, None);
        db.write(|tx| {
            save_library(
                tx,
                &LibraryUpdate {
                    owned: Some(&owned),
                    family: Some(&family),
                    item_apps: &item_apps,
                    reset_items: true,
                },
            )?;
            replace_badges(
                tx,
                &[Badge {
                    badge_id: Some(1),
                    appid: Some(620),
                    level: 2,
                    foil: false,
                    xp: Some(200),
                    completion_time: None,
                }],
            )
        })
        .unwrap();

        // Without the family setting: two owned games and one unowned with items.
        let games_list = db.read(|c| games(c, false)).unwrap();
        assert_eq!(games_list.len(), 3);
        assert_eq!(db.read(|c| games(c, true)).unwrap().len(), 4);
        let portal = games_list.iter().find(|g| g.appid == 620).unwrap();
        assert!(!portal.owned);
        assert_eq!(portal.badge_level, Some(2));

        // The store says TF2 has cards and Dota 2 doesn't; Portal 2 is unknown.
        db.write(|tx| {
            save_store(
                tx,
                &[440, 570, 620],
                &[
                    StoreApp {
                        appid: 440,
                        found: true,
                        name: None,
                        kind: Some(0),
                        has_cards: Some(true),
                    },
                    StoreApp {
                        appid: 570,
                        found: true,
                        name: None,
                        kind: Some(0),
                        has_cards: Some(false),
                    },
                ],
            )
        })
        .unwrap();
        let targets = db.read(|c| catalog_targets(c, false, now())).unwrap();
        let wanted: Vec<u64> = targets
            .iter()
            .filter(|t| t.wanted())
            .map(|t| t.appid)
            .collect();
        assert_eq!(
            wanted,
            vec![440, 620],
            "TF2 by the store, Portal 2 by its badge"
        );

        db.write(|tx| {
            save_catalog(
                tx,
                440,
                &[card(1, false), card(1, true), card(2, false), card(2, true)],
                "defs",
            )
        })
        .unwrap();
        let tf2 = db
            .read(|c| games(c, false))
            .unwrap()
            .into_iter()
            .find(|g| g.appid == 440)
            .unwrap();
        assert_eq!(tf2.cards, 2);
        assert_eq!(tf2.items, 5, "four cards and a booster pack");
        assert!(tf2.cataloged);

        // Points Shop items merge into the same catalog.
        let emoticon = PointsItem {
            appid: 440,
            defid: 7,
            cost: Some(100),
            active: Some(true),
            available_from: None,
            available_until: None,
            buyable: Some(1),
            item: CatalogItem {
                class: class::EMOTICON,
                item_type: 9,
                title: Some(":pyro:".into()),
                ..CatalogItem::default()
            },
        };
        db.write(|tx| save_points(tx, &[440, 570], &[emoticon]))
            .unwrap();
        let counts = db.read(|c| catalog_counts(c, false)).unwrap();
        assert_eq!(counts.games_listed, 3);
        assert_eq!(counts.games_unowned, 1);
        assert_eq!(counts.games_with_cards, 1);
        assert_eq!(counts.games_cataloged, 1);
        assert_eq!(counts.items, 6);
        assert_eq!(counts.points_items, 1);
        assert!(counts.by_kind.contains(&KindCount {
            label: "Foil cards".into(),
            count: 2
        }));

        // A rescan that no longer lists card 2 removes it.
        db.write(|tx| save_catalog(tx, 440, &[card(1, false), card(1, true)], "defs"))
            .unwrap();
        let counts = db.read(|c| catalog_counts(c, false)).unwrap();
        assert_eq!(counts.items, 4);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn achievement_progress() {
        let (db, dir) = temp_db("ach");
        let owned = vec![
            OwnedGame {
                appid: 10,
                name: "A".into(),
                playtime_minutes: 1,
            },
            OwnedGame {
                appid: 20,
                name: "B".into(),
                playtime_minutes: 1,
            },
            OwnedGame {
                appid: 30,
                name: "C".into(),
                playtime_minutes: 1,
            },
        ];
        let item_apps = BTreeMap::new();
        db.write(|tx| {
            save_library(
                tx,
                &LibraryUpdate {
                    owned: Some(&owned),
                    family: None,
                    item_apps: &item_apps,
                    reset_items: true,
                },
            )?;
            save_achievements(tx, &[10, 20, 30], &[(10, 10, 10), (20, 5, 20)])
        })
        .unwrap();
        let counts = db.read(|c| catalog_counts(c, false)).unwrap();
        assert_eq!(
            (counts.ach_games, counts.ach_started, counts.ach_perfect),
            (2, 2, 1)
        );
        assert_eq!(counts.ach_average, Some(62.5), "average of 100% and 25%");
        assert_eq!((counts.ach_unlocked, counts.ach_total), (15, 30));
        assert!(db
            .read(|c| apps_needing_achievements(c, now() - 60))
            .unwrap()
            .is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_and_meta() {
        let (db, dir) = temp_db("scans");
        let id = db.write(|tx| begin_scan(tx)).unwrap();
        db.write(|tx| save_scan_summary(tx, id, "{\"a\":1}"))
            .unwrap();
        db.write(|tx| end_scan(tx, id, "done", 12, None)).unwrap();
        assert_eq!(db.read(last_summary).unwrap().as_deref(), Some("{\"a\":1}"));
        db.write(|tx| set_meta(tx, "route", "defs")).unwrap();
        assert_eq!(
            db.read(|c| get_meta(c, "route")).unwrap().as_deref(),
            Some("defs")
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_file_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("sct-db-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            b"this is not a database, just some text that is long enough",
        )
        .unwrap();
        let db = Db::open(&path).unwrap();
        assert!(db.read(|c| games(c, false)).unwrap().is_empty());
        assert!(dir.join("collection.old.db").exists());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
