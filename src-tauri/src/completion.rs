//! Completion: how much of each game's catalog the user owns, and every
//! item's tags and availability.
//!
//! Owned items are matched to the catalog by game, item class, Steam's item
//! number and foil. Some inventory items don't reveal their number, so those
//! fall back to matching by name. Items the user owns that no Steam list
//! includes any more are added to their game's list, so they only appear for
//! people who own them.
//!
//! Cards count by copies: each badge level uses one copy of every card, so a
//! level 3 badge covers three of the five copies each card needs. Spare copies
//! in the inventory cover the rest. The foil badge has one level, so each foil
//! card needs one copy. With the "keep a set" setting, every card also needs
//! one more copy held on top of the badge.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::catalog::cardpage::base_card_name;
use crate::catalog::{class, decode_entities, kind_label};

/// Badge levels a regular card set has, and copies each card needs for them.
pub const BADGE_LEVELS: u32 = 5;

/// Steam's own "game" for account-wide items such as gems.
const STEAM_APP: u64 = 753;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rules {
    /// Cards need one copy held in the inventory on top of the badge.
    pub keep_card_set: bool,
    /// Steam Family games are listed and count toward the account figure.
    pub include_family: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Availability {
    Available,
    Limited,
    Unobtainable,
}

impl Availability {
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Available => "available",
            Availability::Limited => "limited",
            Availability::Unobtainable => "unobtainable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "available" => Some(Availability::Available),
            "limited" => Some(Availability::Limited),
            "unobtainable" => Some(Availability::Unobtainable),
            _ => None,
        }
    }
}

/// Tags shown on items. Price tags arrive with prices.
pub mod tag {
    pub const UNOWNED: &str = "Unowned";
    pub const FAMILY: &str = "Family shared";
    pub const POINTS: &str = "Points";
    pub const UNOBTAINABLE: &str = "Unobtainable";
    pub const LIMITED: &str = "Limited/Event";
    pub const NOT_MARKETABLE: &str = "Not marketable";
    pub const UNVERIFIED: &str = "Unverified";
}

/// The kinds a game's completion is broken down into.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Cards,
    FoilCards,
    Emoticons,
    Backgrounds,
    PointsShop,
    Boosters,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Cards => "Cards",
            Group::FoilCards => "Foil cards",
            Group::Emoticons => "Emoticons",
            Group::Backgrounds => "Backgrounds",
            Group::PointsShop => "Points Shop items",
            Group::Boosters => "Booster packs",
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ItemView {
    /// Stable name for the item, used for tag overrides.
    pub key: String,
    pub appid: u64,
    pub class: i64,
    pub item_type: Option<i64>,
    pub foil: bool,
    pub group: Group,
    pub kind: &'static str,
    pub name: String,
    pub image: Option<String>,
    /// Copies the user holds.
    pub held: u64,
    /// Copies' worth the user has, out of `needed`. For cards this includes
    /// the copies their badge used up.
    pub have: u32,
    pub needed: u32,
    /// 0 to 1.
    pub progress: f64,
    /// Whether it counts toward the game's percentage.
    pub counted: bool,
    pub points_cost: Option<i64>,
    pub marketable: bool,
    pub availability: Availability,
    /// What the app worked out by itself, before any override.
    pub auto_availability: Availability,
    /// Why it has that availability, in plain words.
    pub note: Option<String>,
    pub overridden: bool,
    pub tags: Vec<&'static str>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GroupProgress {
    pub group: Group,
    pub label: &'static str,
    /// Items owned, with cards counted by the share of copies they have.
    pub owned: f64,
    pub complete: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct GameCompletion {
    pub appid: u64,
    /// `None` when nothing for the game counts.
    pub percent: Option<f64>,
    pub owned: f64,
    pub complete: u64,
    pub total: u64,
    pub groups: Vec<GroupProgress>,
    /// Items left out of the percentage because they can't be had.
    pub unobtainable: u64,
    pub unobtainable_owned: u64,
    pub limited: u64,
    pub badge_level: u32,
    pub foil_badge: bool,
    pub has_cards: bool,
    /// Level 5 badge and the foil badge.
    pub cards_complete: bool,
    /// Whether the game counts toward the account-wide figure.
    pub counts_for_account: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct AccountCompletion {
    /// Every counted item weighs the same.
    pub percent: Option<f64>,
    pub owned: f64,
    pub complete: u64,
    pub total: u64,
    /// Every counted game weighs the same.
    pub game_average: Option<f64>,
    /// Counted games that have any items.
    pub games: u64,
    pub games_started: u64,
    pub games_complete: u64,
    pub card_games: u64,
    pub card_sets_complete: u64,
    pub groups: Vec<GroupProgress>,
    pub unobtainable: u64,
    pub unobtainable_owned: u64,
    pub limited: u64,
    pub keep_card_set: bool,
    pub include_family: bool,
}

pub struct Snapshot {
    pub items: Vec<ItemView>,
    pub games: BTreeMap<u64, GameCompletion>,
    pub account: AccountCompletion,
}

impl Snapshot {
    pub fn game_items(&self, appid: u64) -> Vec<ItemView> {
        self.items
            .iter()
            .filter(|i| i.appid == appid)
            .cloned()
            .collect()
    }

    /// Games where the user owns something no list the app read includes.
    pub fn apps_with_unlisted(&self) -> Vec<u64> {
        let set: HashSet<u64> = self
            .items
            .iter()
            .filter(|i| i.held > 0 && i.note.as_deref() == Some(NOTE_UNLISTED))
            .map(|i| i.appid)
            .collect();
        let mut v: Vec<u64> = set.into_iter().collect();
        v.sort_unstable();
        v
    }
}

const NOTE_UNLISTED: &str =
    "Steam no longer lists this item anywhere, so it only shows up because you own it.";
const NOTE_NOT_READ: &str = "This game's item list hasn't been read yet. Scan again to check it.";
const NOTE_CARD_MISMATCH: &str =
    "This card doesn't match the card list yet. Its card page is read again on the next scan.";

// ---- Loading ------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct App {
    owned: bool,
    family: bool,
    has_items: bool,
    event: bool,
    has_cards: bool,
    catalog_read: bool,
    points_read: bool,
}

impl App {
    /// The same games the Games tab lists.
    fn listed(&self, rules: Rules) -> bool {
        self.owned || self.has_items || (self.family && rules.include_family)
    }

    fn counts_for_account(&self, rules: Rules) -> bool {
        self.owned || (self.family && rules.include_family)
    }
}

#[derive(Default, Clone)]
struct Entry {
    appid: u64,
    class: i64,
    item_type: Option<i64>,
    foil: bool,
    name: Option<String>,
    title: Option<String>,
    image: Option<String>,
    animated: bool,
    series: Option<i64>,
    in_points: bool,
    points_cost: Option<i64>,
    points_active: Option<bool>,
    points_from: Option<i64>,
    points_until: Option<i64>,
    points_buyable: Option<i64>,
    /// Not in any catalog; added because the user owns it.
    unlisted: bool,
    inv_held: u64,
    profile_held: u64,
    marketable_copies: u64,
    exact_hit: bool,
    name_hit: bool,
}

impl Entry {
    fn display_name(&self) -> String {
        let raw = self
            .title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Unnamed item");
        decode_entities(raw)
    }
}

fn norm(s: &str) -> String {
    let decoded = decode_entities(s);
    base_card_name(&decoded.split_whitespace().collect::<Vec<_>>().join(" "))
}

struct Index {
    exact: HashMap<(u64, i64, i64, bool), usize>,
    names: HashMap<(u64, i64, bool, String), usize>,
}

impl Index {
    fn add(&mut self, e: &Entry, i: usize) {
        if let Some(t) = e.item_type {
            self.exact.entry((e.appid, e.class, t, e.foil)).or_insert(i);
        }
        for n in [e.title.as_deref(), e.name.as_deref()]
            .into_iter()
            .flatten()
        {
            let n = norm(n);
            if !n.is_empty() {
                self.names.entry((e.appid, e.class, e.foil, n)).or_insert(i);
            }
        }
    }

    fn by_type(&self, appid: u64, class_id: i64, t: Option<i64>, foil: bool) -> Option<usize> {
        if class_id == class::BOOSTER {
            // Each game has one booster pack, whatever number Steam gives it.
            return self.exact.get(&(appid, class_id, 0, false)).copied();
        }
        t.and_then(|t| self.exact.get(&(appid, class_id, t, foil)).copied())
    }

    fn by_name<'a>(
        &self,
        appid: u64,
        class_id: i64,
        foil: bool,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Option<usize> {
        names
            .into_iter()
            .find_map(|n| self.names.get(&(appid, class_id, foil, norm(n))).copied())
    }
}

fn load_apps(c: &Connection) -> rusqlite::Result<HashMap<u64, App>> {
    let mut st = c.prepare(
        "SELECT appid, owned, family, has_items, event_app, COALESCE(has_cards, 0),
             catalog_checked_at IS NOT NULL AND catalog_error IS NULL,
             points_checked_at IS NOT NULL
         FROM apps WHERE appid <> 753",
    )?;
    let rows = st.query_map((), |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            App {
                owned: r.get::<_, i64>(1)? != 0,
                family: r.get::<_, i64>(2)? != 0,
                has_items: r.get::<_, i64>(3)? != 0,
                event: r.get::<_, i64>(4)? != 0,
                has_cards: r.get::<_, i64>(5)? != 0,
                catalog_read: r.get::<_, i64>(6)? != 0,
                points_read: r.get::<_, i64>(7)? != 0,
            },
        ))
    })?;
    rows.collect()
}

fn load_overrides(c: &Connection) -> rusqlite::Result<HashMap<String, Availability>> {
    let mut st = c.prepare("SELECT key, availability FROM item_overrides")?;
    let rows = st.query_map((), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (key, value) = row?;
        if let Some(a) = Availability::parse(&value) {
            out.insert(key, a);
        }
    }
    Ok(out)
}

/// Badge levels by game and foil: (badge number, level).
fn load_badges(c: &Connection) -> rusqlite::Result<HashMap<(u64, bool), Vec<(i64, u32)>>> {
    let mut st = c.prepare("SELECT appid, foil, badgeid, level FROM badges WHERE appid > 0")?;
    let rows = st.query_map((), |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            r.get::<_, i64>(1)? != 0,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?.max(0) as u32,
        ))
    })?;
    let mut out: HashMap<(u64, bool), Vec<(i64, u32)>> = HashMap::new();
    for row in rows {
        let (appid, foil, badgeid, level) = row?;
        out.entry((appid, foil)).or_default().push((badgeid, level));
    }
    Ok(out)
}

fn badge_level(
    badges: &HashMap<(u64, bool), Vec<(i64, u32)>>,
    appid: u64,
    foil: bool,
    series: Option<i64>,
) -> u32 {
    let Some(list) = badges.get(&(appid, foil)) else {
        return 0;
    };
    // Games with more than one card series have a badge per series.
    if let Some(s) = series {
        if let Some((_, level)) = list.iter().find(|(id, _)| *id == s) {
            return *level;
        }
    }
    list.iter().map(|(_, l)| *l).max().unwrap_or(0)
}

// ---- Working it out -------------------------------------------------------------

pub fn compute(c: &Connection, rules: Rules, now: i64) -> rusqlite::Result<Snapshot> {
    let apps = load_apps(c)?;
    let listed = |appid: u64| -> Option<App> {
        apps.get(&appid)
            .copied()
            .filter(|a| a.listed(rules))
            .filter(|_| appid != STEAM_APP)
    };
    let overrides = load_overrides(c)?;
    let badges = load_badges(c)?;

    let mut entries: Vec<Entry> = Vec::new();
    {
        let mut st = c.prepare(
            "SELECT appid, class, item_type, foil, name, title, COALESCE(image, image_large),
                 animated, series, in_points, points_cost, points_active, points_from,
                 points_until, points_buyable
             FROM items WHERE class <> 1
             ORDER BY appid, class, foil, COALESCE(series, 0), item_type",
        )?;
        let rows = st.query_map((), |r| {
            Ok(Entry {
                appid: r.get::<_, i64>(0)? as u64,
                class: r.get(1)?,
                item_type: Some(r.get(2)?),
                foil: r.get::<_, i64>(3)? != 0,
                name: r.get(4)?,
                title: r.get(5)?,
                image: r.get(6)?,
                animated: r.get::<_, i64>(7)? != 0,
                series: r.get(8)?,
                in_points: r.get::<_, i64>(9)? != 0,
                points_cost: r.get(10)?,
                points_active: r.get::<_, Option<i64>>(11)?.map(|v| v != 0),
                points_from: r.get(12)?,
                points_until: r.get(13)?,
                points_buyable: r.get(14)?,
                ..Entry::default()
            })
        })?;
        for row in rows {
            let e = row?;
            if listed(e.appid).is_some() {
                entries.push(e);
            }
        }
    }

    // The card page only pictures the regular card, so foils borrow its image.
    let card_images: HashMap<(u64, i64), String> = entries
        .iter()
        .filter(|e| e.class == class::CARD && !e.foil)
        .filter_map(|e| Some(((e.appid, e.item_type?), e.image.clone()?)))
        .collect();
    for e in entries
        .iter_mut()
        .filter(|e| e.class == class::CARD && e.foil && e.image.is_none())
    {
        if let Some(t) = e.item_type {
            e.image = card_images.get(&(e.appid, t)).cloned();
        }
    }

    let mut index = Index {
        exact: HashMap::new(),
        names: HashMap::new(),
    };
    for (i, e) in entries.iter().enumerate() {
        index.add(e, i);
    }

    // Profile items first: they always carry Steam's item number, so items
    // only they know about get the right number before the inventory is matched.
    {
        let mut st = c.prepare(
            "SELECT appid, class, item_type, name, title, SUM(quantity)
             FROM profile_items GROUP BY appid, class, item_type, name, title",
        )?;
        let rows = st.query_map((), |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?.max(0) as u64,
            ))
        })?;
        for row in rows {
            let (appid, class_id, item_type, name, title, qty) = row?;
            if listed(appid).is_none() || class_id <= class::BADGE {
                continue;
            }
            let names = [title.as_deref(), Some(name.as_str())];
            let (idx, exact) = match index.by_type(appid, class_id, item_type, false) {
                Some(i) => (Some(i), true),
                None => (
                    index.by_name(appid, class_id, false, names.into_iter().flatten()),
                    false,
                ),
            };
            let (i, exact) = match idx {
                Some(i) => (i, exact),
                None => {
                    let e = Entry {
                        appid,
                        class: class_id,
                        item_type,
                        name: Some(name.clone()),
                        title: title.clone(),
                        unlisted: true,
                        ..Entry::default()
                    };
                    entries.push(e);
                    let i = entries.len() - 1;
                    index.add(&entries[i], i);
                    (i, item_type.is_some())
                }
            };
            let e = &mut entries[i];
            e.profile_held += qty;
            if exact {
                e.exact_hit = true;
            } else {
                e.name_hit = true;
            }
        }
    }

    {
        let mut st = c.prepare(
            "SELECT appid, class, item_type, foil, name, market_hash_name, marketable, icon,
                 SUM(quantity)
             FROM inventory
             WHERE appid IS NOT NULL AND class IS NOT NULL
             GROUP BY appid, class, item_type, foil, name, market_hash_name, marketable, icon",
        )?;
        let rows = st.query_map((), |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)? != 0,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<i64>>(6)?.map(|m| m != 0),
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?.max(0) as u64,
            ))
        })?;
        for row in rows {
            let (appid, class_id, item_type, foil, name, mhn, marketable, icon, qty) = row?;
            if listed(appid).is_none() || class_id <= class::BADGE || class_id == class::GEMS {
                continue;
            }
            let prefix = format!("{appid}-");
            let market_name = mhn
                .as_deref()
                .map(|m| m.strip_prefix(prefix.as_str()).unwrap_or(m).to_string());
            let names = [name.as_deref(), market_name.as_deref()];
            let (idx, exact) = match index.by_type(appid, class_id, item_type, foil) {
                Some(i) => (Some(i), true),
                None => (
                    index.by_name(appid, class_id, foil, names.into_iter().flatten()),
                    false,
                ),
            };
            let (i, exact) = match idx {
                Some(i) => (i, exact),
                None => {
                    let e = Entry {
                        appid,
                        class: class_id,
                        item_type,
                        foil,
                        name: market_name.clone(),
                        title: name.clone(),
                        image: icon.map(|i| {
                            format!("https://community.akamai.steamstatic.com/economy/image/{i}/96fx96f")
                        }),
                        unlisted: true,
                        ..Entry::default()
                    };
                    entries.push(e);
                    let i = entries.len() - 1;
                    index.add(&entries[i], i);
                    (i, item_type.is_some())
                }
            };
            let e = &mut entries[i];
            e.inv_held += qty;
            if marketable == Some(true) {
                e.marketable_copies += qty;
            }
            if exact {
                e.exact_hit = true;
            } else {
                e.name_hit = true;
            }
        }
    }

    // Each item's view, in a steady order: game, kind, card series, item number.
    let mut keyed: Vec<((u64, Group, i64, i64), ItemView)> = Vec::with_capacity(entries.len());
    for e in &entries {
        let Some(app) = listed(e.appid) else { continue };
        let v = view(e, app, rules, &badges, &overrides, now);
        let order = (
            e.appid,
            v.group,
            e.series.unwrap_or(0),
            e.item_type.unwrap_or(i64::MAX),
        );
        keyed.push((order, v));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let items: Vec<ItemView> = keyed.into_iter().map(|(_, v)| v).collect();

    // Per game, then the account.
    let mut games: BTreeMap<u64, GameCompletion> = BTreeMap::new();
    for (&appid, app) in apps
        .iter()
        .filter(|(a, app)| **a != STEAM_APP && app.listed(rules))
    {
        games.insert(
            appid,
            GameCompletion {
                appid,
                badge_level: badge_level(&badges, appid, false, None),
                foil_badge: badge_level(&badges, appid, true, None) > 0,
                has_cards: app.has_cards,
                counts_for_account: app.counts_for_account(rules),
                ..GameCompletion::default()
            },
        );
    }
    let mut game_groups: HashMap<u64, BTreeMap<Group, GroupProgress>> = HashMap::new();
    for item in &items {
        let Some(g) = games.get_mut(&item.appid) else {
            continue;
        };
        if item.group == Group::Cards || item.group == Group::FoilCards {
            g.has_cards = true;
        }
        match item.availability {
            Availability::Unobtainable => {
                g.unobtainable += 1;
                if item.held > 0 {
                    g.unobtainable_owned += 1;
                }
            }
            Availability::Limited => g.limited += 1,
            Availability::Available => {}
        }
        let groups = game_groups.entry(item.appid).or_default();
        let gp = groups.entry(item.group).or_insert_with(|| GroupProgress {
            group: item.group,
            label: item.group.label(),
            owned: 0.0,
            complete: 0,
            total: 0,
        });
        if item.counted || item.group == Group::Boosters {
            gp.total += 1;
            gp.owned += item.progress;
            if item.have >= item.needed {
                gp.complete += 1;
            }
        }
        if item.counted {
            g.total += 1;
            g.owned += item.progress;
            if item.have >= item.needed {
                g.complete += 1;
            }
        }
    }
    for (appid, g) in games.iter_mut() {
        g.percent = (g.total > 0).then(|| round1(100.0 * g.owned / g.total as f64));
        g.owned = round3(g.owned);
        g.cards_complete = g.has_cards && g.badge_level >= BADGE_LEVELS && g.foil_badge;
        if let Some(groups) = game_groups.remove(appid) {
            g.groups = groups
                .into_values()
                .filter(|gp| gp.total > 0)
                .map(|mut gp| {
                    gp.owned = round3(gp.owned);
                    gp
                })
                .collect();
        }
    }

    let mut account = AccountCompletion {
        keep_card_set: rules.keep_card_set,
        include_family: rules.include_family,
        ..AccountCompletion::default()
    };
    let mut account_groups: BTreeMap<Group, GroupProgress> = BTreeMap::new();
    let mut percents: Vec<f64> = Vec::new();
    for g in games.values().filter(|g| g.counts_for_account) {
        account.unobtainable += g.unobtainable;
        account.unobtainable_owned += g.unobtainable_owned;
        account.limited += g.limited;
        if g.has_cards {
            account.card_games += 1;
            if g.cards_complete {
                account.card_sets_complete += 1;
            }
        }
        if g.total == 0 {
            continue;
        }
        account.games += 1;
        account.total += g.total;
        account.owned += g.owned;
        account.complete += g.complete;
        if g.owned > 0.0 {
            account.games_started += 1;
        }
        if g.complete == g.total {
            account.games_complete += 1;
        }
        percents.push(100.0 * g.owned / g.total as f64);
        for gp in &g.groups {
            let a = account_groups
                .entry(gp.group)
                .or_insert_with(|| GroupProgress {
                    group: gp.group,
                    label: gp.label,
                    owned: 0.0,
                    complete: 0,
                    total: 0,
                });
            a.owned += gp.owned;
            a.complete += gp.complete;
            a.total += gp.total;
        }
    }
    account.percent =
        (account.total > 0).then(|| round1(100.0 * account.owned / account.total as f64));
    account.game_average = (!percents.is_empty())
        .then(|| round1(percents.iter().sum::<f64>() / percents.len() as f64));
    account.owned = round3(account.owned);
    account.groups = account_groups
        .into_values()
        .map(|mut gp| {
            gp.owned = round3(gp.owned);
            gp
        })
        .collect();

    Ok(Snapshot {
        items,
        games,
        account,
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn date_text(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.format("%b %-d, %Y").to_string())
        .unwrap_or_else(|| "an unknown date".to_string())
}

fn group_of(e: &Entry) -> Group {
    match e.class {
        class::CARD if e.foil => Group::FoilCards,
        class::CARD => Group::Cards,
        class::EMOTICON => Group::Emoticons,
        class::BACKGROUND if !e.animated => Group::Backgrounds,
        class::BOOSTER => Group::Boosters,
        _ => Group::PointsShop,
    }
}

/// Whether the item can be sold on the Market. Cards, booster packs and the
/// emoticons and backgrounds from a card set can; items only sold for points
/// can't. A copy the user can sell settles it either way. Prices will confirm
/// this against the Market itself.
fn is_marketable(e: &Entry, app: App) -> bool {
    if e.marketable_copies > 0 {
        return true;
    }
    match e.class {
        class::CARD | class::BOOSTER => true,
        _ => is_card_set_item(e, app),
    }
}

/// An emoticon or background from a game's trading card set. In card games
/// every static emoticon and background is one; Points Shop exclusives are
/// animated or other kinds of item.
fn is_card_set_item(e: &Entry, app: App) -> bool {
    matches!(e.class, class::EMOTICON | class::BACKGROUND)
        && app.has_cards
        && !e.animated
        && !e.unlisted
}

fn auto_availability(e: &Entry, app: App, now: i64) -> (Availability, Option<String>) {
    if e.unlisted {
        if e.class == class::CARD || e.class == class::BOOSTER {
            let note = if app.catalog_read {
                NOTE_CARD_MISMATCH
            } else {
                NOTE_NOT_READ
            };
            return (Availability::Available, Some(note.to_string()));
        }
        if !app.points_read {
            return (Availability::Available, Some(NOTE_NOT_READ.to_string()));
        }
        return (Availability::Unobtainable, Some(NOTE_UNLISTED.to_string()));
    }
    if e.class == class::CARD || e.class == class::BOOSTER || !e.in_points {
        return (Availability::Available, None);
    }
    if is_card_set_item(e, app) {
        // Crafting the badge hands these out, and they trade on the Market,
        // whatever the Points Shop does.
        let sold = e.points_active != Some(false)
            && e.points_until.is_none_or(|u| u > now)
            && e.points_buyable != Some(0);
        let note = (!sold).then(|| {
            "Not sold in the Points Shop. Crafting the badge can give it, and it's on the Market."
                .to_string()
        });
        return (Availability::Available, note);
    }
    if e.points_active == Some(false) {
        return (
            Availability::Unobtainable,
            Some("No longer sold in the Points Shop.".to_string()),
        );
    }
    if let Some(until) = e.points_until {
        return if until <= now {
            (
                Availability::Unobtainable,
                Some(format!("Left the Points Shop on {}.", date_text(until))),
            )
        } else {
            (
                Availability::Limited,
                Some(format!("In the Points Shop until {}.", date_text(until))),
            )
        };
    }
    if let Some(from) = e.points_from.filter(|f| *f > now) {
        return (
            Availability::Limited,
            Some(format!("Comes to the Points Shop on {}.", date_text(from))),
        );
    }
    match e.points_buyable {
        Some(0) => {
            return (
                Availability::Limited,
                Some(
                    "Not sold on its own. It's handed out during sales, events or as a reward."
                        .to_string(),
                ),
            )
        }
        Some(2) => {
            return (
                Availability::Available,
                Some("Sold as part of a Points Shop bundle.".to_string()),
            )
        }
        _ => {}
    }
    if app.event {
        return (
            Availability::Limited,
            Some("From a Steam sale event.".to_string()),
        );
    }
    (Availability::Available, None)
}

pub fn item_key(
    appid: u64,
    class_id: i64,
    item_type: Option<i64>,
    name: &str,
    foil: bool,
) -> String {
    let id = match item_type {
        Some(t) => t.to_string(),
        None => format!("n:{}", norm(name)),
    };
    format!("{appid}-{class_id}-{id}-{}", u8::from(foil))
}

fn view(
    e: &Entry,
    app: App,
    rules: Rules,
    badges: &HashMap<(u64, bool), Vec<(i64, u32)>>,
    overrides: &HashMap<String, Availability>,
    now: i64,
) -> ItemView {
    let name = e.display_name();
    let key = item_key(
        e.appid,
        e.class,
        e.item_type,
        e.title.as_deref().or(e.name.as_deref()).unwrap_or(""),
        e.foil,
    );
    let group = group_of(e);
    let held = e.inv_held.max(e.profile_held);

    let (have, needed) = if e.class == class::CARD {
        let levels = if e.foil { 1 } else { BADGE_LEVELS };
        let needed = levels + u32::from(rules.keep_card_set);
        let level = badge_level(badges, e.appid, e.foil, e.series).min(levels);
        let have = (level as u64 + held).min(needed as u64) as u32;
        (have, needed)
    } else {
        (u32::from(held > 0), 1)
    };

    let (auto, note) = auto_availability(e, app, now);
    let overridden = overrides.get(&key).copied();
    let availability = overridden.unwrap_or(auto);
    let marketable = is_marketable(e, app);

    let mut tags = Vec::new();
    if !app.owned && app.family {
        tags.push(tag::FAMILY);
    } else if !app.owned && !app.event {
        tags.push(tag::UNOWNED);
    }
    if e.in_points && !marketable {
        tags.push(tag::POINTS);
    }
    match availability {
        Availability::Unobtainable => tags.push(tag::UNOBTAINABLE),
        Availability::Limited => tags.push(tag::LIMITED),
        Availability::Available => {}
    }
    if !marketable {
        tags.push(tag::NOT_MARKETABLE);
    }
    let unmatched_card = e.unlisted && (e.class == class::CARD || e.class == class::BOOSTER);
    if (e.name_hit && !e.exact_hit) || unmatched_card || note.as_deref() == Some(NOTE_NOT_READ) {
        tags.push(tag::UNVERIFIED);
    }

    ItemView {
        key,
        appid: e.appid,
        class: e.class,
        item_type: e.item_type,
        foil: e.foil,
        group,
        kind: kind_label(e.class, e.foil, e.animated),
        name,
        image: e.image.clone(),
        held,
        have,
        needed,
        progress: f64::from(have) / f64::from(needed.max(1)),
        counted: e.class != class::BOOSTER && availability != Availability::Unobtainable,
        points_cost: e.points_cost.filter(|_| e.in_points),
        marketable,
        availability,
        auto_availability: auto,
        note,
        overridden: overridden.is_some(),
        tags,
    }
}

/// Saves or clears the user's own availability for an item.
pub fn set_override(
    c: &Connection,
    key: &str,
    availability: Option<Availability>,
    now: i64,
) -> rusqlite::Result<()> {
    match availability {
        Some(a) => c.execute(
            "INSERT INTO item_overrides (key, availability, set_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET availability = excluded.availability, set_at = excluded.set_at",
            params![key, a.as_str(), now],
        )?,
        None => c.execute("DELETE FROM item_overrides WHERE key = ?1", [key])?,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, Db};

    fn setup(name: &str) -> (Db, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("sct-completion-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join(db::FILE_NAME)).unwrap();
        db.write(|tx| {
            tx.execute_batch(
                "INSERT INTO apps (appid, name, owned, has_items, has_cards, catalog_checked_at, points_checked_at)
                     VALUES (10, 'Card Game', 1, 1, 1, 1, 1),
                            (20, 'Sale Event', 0, 1, 0, NULL, 1),
                            (30, 'Shared Game', 0, 0, 1, 1, 1);
                 UPDATE apps SET event_app = 1 WHERE appid = 20;
                 UPDATE apps SET family = 1 WHERE appid = 30;
                 INSERT INTO items (appid, class, item_type, foil, title, in_catalog, updated_at) VALUES
                     (10, 2, 3, 0, 'Alpha', 1, 0), (10, 2, 3, 1, 'Alpha', 1, 0),
                     (10, 2, 4, 0, 'Beta', 1, 0),  (10, 2, 4, 1, 'Beta', 1, 0),
                     (10, 5, 0, 0, 'Card Game Booster Pack', 1, 0),
                     (30, 2, 1, 0, 'Gamma', 1, 0), (30, 2, 1, 1, 'Gamma', 1, 0);
                 INSERT INTO items (appid, class, item_type, foil, name, title, in_points, points_cost, points_active, points_buyable, updated_at) VALUES
                     (10, 4, 5, 0, ':smile:', ':smile:', 1, 100, 1, 1, 0),
                     (10, 3, 6, 0, 'Sky', 'Sky', 1, 500, 1, 1, 0),
                     (10, 15, 7, 0, 'Spin', 'Spin', 1, 3000, 1, 0, 0),
                     (10, 4, 9, 0, ':frown:', ':frown:', 1, 100, 1, 0, 0),
                     (10, 14, 8, 0, 'Gone', 'Gone', 1, 2000, 0, 1, 0),
                     (20, 11, 1, 0, 'Wave', 'Wave', 1, 1000, 1, 1, 0);
                 UPDATE items SET image = 'alpha.png' WHERE appid = 10 AND class = 2 AND item_type = 3 AND foil = 0;
                 INSERT INTO badges (appid, badgeid, foil, level) VALUES (10, 1, 0, 3);
                 INSERT INTO inventory (classid, instanceid, appid, class, item_type, foil, name, marketable, stacks, quantity) VALUES
                     ('a', '0', 10, 2, 3, 0, 'Alpha', 1, 1, 4),
                     ('b', '0', 10, 2, 4, 1, 'Beta (Foil)', 1, 1, 1),
                     ('c', '0', 10, 4, NULL, 0, ':smile:', 0, 1, 1),
                     ('d', '0', 20, 11, NULL, 0, 'Wave', 0, 1, 1),
                     ('e', '0', 10, 11, NULL, 0, 'Old Sticker', 0, 1, 1);
                 INSERT INTO profile_items (source, appid, class, item_type, name, title, quantity) VALUES
                     ('profile_items', 10, 15, 7, 'spin', 'Spin', 1);",
            )
        })
        .unwrap();
        (db, dir)
    }

    fn rules(keep: bool, family: bool) -> Rules {
        Rules {
            keep_card_set: keep,
            include_family: family,
        }
    }

    #[test]
    fn cards_count_by_badge_level_and_copies() {
        let (db, dir) = setup("cards");
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        let items = snap.game_items(10);
        let alpha = items.iter().find(|i| i.name == "Alpha" && !i.foil).unwrap();
        assert_eq!(
            (alpha.have, alpha.needed),
            (5, 5),
            "level 3 plus 4 copies, capped at 5"
        );
        let beta = items.iter().find(|i| i.name == "Beta" && !i.foil).unwrap();
        assert_eq!((beta.have, beta.needed), (3, 5), "the level 3 badge alone");
        let beta_foil = items.iter().find(|i| i.name == "Beta" && i.foil).unwrap();
        assert_eq!((beta_foil.have, beta_foil.needed), (1, 1));
        let alpha_foil = items.iter().find(|i| i.name == "Alpha" && i.foil).unwrap();
        assert_eq!(alpha_foil.have, 0);
        assert_eq!(
            alpha_foil.image.as_deref(),
            Some("alpha.png"),
            "foils borrow the card's image"
        );

        // Keeping a set: one more copy per card on top of the badge.
        let snap = db.read(|c| compute(c, rules(true, false), 100)).unwrap();
        let items = snap.game_items(10);
        let alpha = items.iter().find(|i| i.name == "Alpha" && !i.foil).unwrap();
        assert_eq!((alpha.have, alpha.needed), (6, 6));
        let beta = items.iter().find(|i| i.name == "Beta" && !i.foil).unwrap();
        assert_eq!((beta.have, beta.needed), (3, 6));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_availability_and_tags() {
        let (db, dir) = setup("tags");
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        let items = snap.game_items(10);
        let smile = items.iter().find(|i| i.name == ":smile:").unwrap();
        assert_eq!(smile.held, 1, "matched by name");
        assert!(smile.tags.contains(&tag::UNVERIFIED));
        assert!(
            smile.marketable,
            "a card game's emoticon is from its card set"
        );
        assert!(!smile.tags.contains(&tag::POINTS));

        let spin = items.iter().find(|i| i.name == "Spin").unwrap();
        assert_eq!(spin.held, 1, "found through profile items");
        assert_eq!(
            spin.availability,
            Availability::Limited,
            "can't be bought on its own"
        );
        assert!(spin.counted);
        assert!(spin.tags.contains(&tag::POINTS) && spin.tags.contains(&tag::NOT_MARKETABLE));
        assert!(!spin.tags.contains(&tag::UNVERIFIED));

        let frown = items.iter().find(|i| i.name == ":frown:").unwrap();
        assert_eq!(
            frown.availability,
            Availability::Available,
            "crafting still gives card-set items"
        );
        assert!(frown.note.is_some());

        let gone = items.iter().find(|i| i.name == "Gone").unwrap();
        assert_eq!(gone.availability, Availability::Unobtainable);
        assert!(!gone.counted);

        let old = items.iter().find(|i| i.name == "Old Sticker").unwrap();
        assert_eq!(
            old.availability,
            Availability::Unobtainable,
            "no list has it"
        );
        assert_eq!(old.held, 1);

        let booster = items.iter().find(|i| i.group == Group::Boosters).unwrap();
        assert!(!booster.counted);

        let wave = snap.game_items(20);
        assert_eq!(wave.len(), 1);
        assert_eq!(
            wave[0].availability,
            Availability::Limited,
            "sale event item"
        );
        assert!(!wave[0].tags.contains(&tag::UNOWNED));

        // Game 10 counts 4 cards, 2 emoticons, 1 background, 1 avatar: 8 items.
        let g = &snap.games[&10];
        assert_eq!(g.total, 8);
        assert_eq!(g.unobtainable, 2);
        assert_eq!(g.unobtainable_owned, 1);
        // Alpha 1 + Beta 0.6 + Beta foil 1 + emoticon 1 + avatar 1 = 4.6 of 8.
        assert_eq!(g.owned, 4.6);
        assert_eq!(g.percent, Some(57.5));
        assert_eq!(g.badge_level, 3);
        assert!(!g.cards_complete);

        // Only owned games count for the account; the sale event doesn't.
        assert_eq!(snap.account.games, 1);
        assert_eq!(snap.account.total, 8);
        assert!(
            !snap.games.contains_key(&30),
            "family games hidden while the setting is off"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn family_and_overrides() {
        let (db, dir) = setup("family");
        let snap = db.read(|c| compute(c, rules(false, true), 100)).unwrap();
        assert!(snap.games[&30].counts_for_account);
        assert_eq!(snap.account.games, 2);
        let gamma = snap.game_items(30);
        assert!(gamma[0].tags.contains(&tag::FAMILY));

        // Marking the avatar Unobtainable takes it out of the percentage.
        let spin_key = snap
            .game_items(10)
            .into_iter()
            .find(|i| i.name == "Spin")
            .unwrap()
            .key;
        db.write(|tx| set_override(tx, &spin_key, Some(Availability::Unobtainable), 1))
            .unwrap();
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        let spin = snap
            .game_items(10)
            .into_iter()
            .find(|i| i.name == "Spin")
            .unwrap();
        assert!(spin.overridden && !spin.counted);
        assert_eq!(spin.auto_availability, Availability::Limited);
        assert_eq!(snap.games[&10].total, 7);
        db.write(|tx| set_override(tx, &spin_key, None, 2)).unwrap();
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        assert_eq!(snap.games[&10].total, 8);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys() {
        assert_eq!(item_key(10, 2, Some(3), "x", true), "10-2-3-1");
        assert_eq!(
            item_key(10, 11, None, "Old  Sticker", false),
            "10-11-n:old sticker-0"
        );
    }
}
