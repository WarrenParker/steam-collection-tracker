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
//!
//! Prices come from the Market tables: each item's cheapest listing, its
//! average and lowest prices, and its order book. Cost to complete is in
//! money wherever the Market sells an item, walking up the order book when
//! several copies are needed, and in points for items only the Points Shop
//! sells. Account value counts only copies the user can sell.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::catalog::cardpage::base_card_name;
use crate::catalog::{class, decode_entities, kind_label};
use crate::db::MarketRow;
use crate::market::{self, book, series::DAY};
use crate::settings::{PriceWindow, Settings, SPIKE_MIN_CENTS, SPIKE_PERCENT};

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
    pub prices: PriceRules,
}

impl Rules {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            keep_card_set: s.keep_card_set,
            include_family: s.include_family,
            prices: PriceRules {
                window: s.price_window,
                spike_percent: s.spike_percent.clamp(1, 1000),
                spike_min_cents: s.spike_min_cents,
                currency: market::currency_id(&s.currency).unwrap_or(1),
            },
        }
    }
}

/// How prices are shown and judged.
#[derive(Clone, Copy, Debug)]
pub struct PriceRules {
    /// The span the average price covers.
    pub window: PriceWindow,
    /// How far from the average, in percent, today's price must be to count
    /// as a spike or a dip...
    pub spike_percent: u32,
    /// ...and by at least this many cents.
    pub spike_min_cents: u32,
    /// Steam's number for the currency prices are shown in.
    pub currency: i64,
}

impl Default for PriceRules {
    fn default() -> Self {
        Self {
            window: PriceWindow::Month,
            spike_percent: SPIKE_PERCENT,
            spike_min_cents: SPIKE_MIN_CENTS,
            currency: 1,
        }
    }
}

/// An order book older than this isn't used for the cost of several copies.
const BOOK_FRESH_DAYS: i64 = 7;

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
    pub const PRICE_SPIKE: &str = "Price spike";
    pub const PRICE_DIP: &str = "Price dip";
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
    /// The item's Market prices, when it's on the Market.
    pub price: Option<ItemPrice>,
    /// Copies the user holds that they could sell.
    pub sellable: u64,
    /// What those copies are worth at today's price, before Steam's fees.
    pub value: Option<i64>,
    /// Money still needed to finish this item, buying on the Market.
    pub cost: Option<i64>,
    /// True when that cost guesses at the price of later copies.
    pub cost_estimated: bool,
    /// Points needed, for items only the Points Shop sells.
    pub cost_points: Option<i64>,
    /// What the Points Shop would charge instead, for items sold both ways.
    pub alt_points: Option<i64>,
    /// Still needed, but nothing sells it right now.
    pub unpriced: bool,
}

/// An item's Market prices, in mills of the shown currency.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ItemPrice {
    pub hash_name: String,
    /// Listed for sale in the latest Market search.
    pub on_market: bool,
    /// The cheapest listing, what a buyer pays.
    pub lowest: Option<i64>,
    pub listings: i64,
    /// The average sale price over the chosen span.
    pub average: Option<i64>,
    /// How far today's price is from that average, in percent.
    pub change: Option<f64>,
    pub avg_30: Option<i64>,
    pub avg_90: Option<i64>,
    pub avg_365: Option<i64>,
    pub avg_all: Option<i64>,
    /// The lowest and highest daily median ever, and when (Unix time).
    pub low: Option<i64>,
    pub low_at: Option<i64>,
    pub high: Option<i64>,
    pub high_at: Option<i64>,
    /// Copies sold in the last 30 days.
    pub sales_30: Option<i64>,
    /// The best offer to buy.
    pub buy_max: Option<i64>,
    /// When the search, history and order book were read (Unix time).
    pub seen_at: i64,
    pub history_at: Option<i64>,
    pub book_at: Option<i64>,
}

/// Money and points for a game or the account. Money is in mills of the
/// shown currency.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct PriceTotals {
    /// Copies the user could sell, at today's prices, before Steam's fees.
    pub value: i64,
    pub value_copies: u64,
    /// Sellable copies with no price to go by.
    pub value_unpriced: u64,
    /// Items the user holds that can't be sold, at their Points Shop price.
    pub points_value: i64,
    /// Money to buy everything still needed from the Market.
    pub cost: i64,
    pub cost_items: u64,
    /// Items whose cost guesses at the price of later copies.
    pub cost_estimated: u64,
    /// Points for items only the Points Shop sells.
    pub cost_points: i64,
    pub cost_points_items: u64,
    /// Items counted in money that the Points Shop also sells: what they'd
    /// cost in points instead, and the money that would save.
    pub alt_points: i64,
    pub alt_money: i64,
    pub alt_items: u64,
    /// Still needed, but nothing sells them right now.
    pub unpriced: u64,
    pub spikes: u64,
    pub dips: u64,
    /// Items on the Market, and how many have a price history so far.
    pub marketable: u64,
    pub with_history: u64,
}

impl PriceTotals {
    fn add_item(&mut self, item: &ItemView) {
        if let Some(v) = item.value {
            self.value += v;
            self.value_copies += item.sellable;
        } else if item.sellable > 0 {
            self.value_unpriced += item.sellable;
        }
        if let Some(cost) = item.points_cost {
            let unsellable = item.held.saturating_sub(item.sellable) as i64;
            self.points_value += unsellable * cost;
        }
        self.add_needed(item);
    }

    /// The parts that only count for games that count toward the account.
    fn add_needed(&mut self, item: &ItemView) {
        if let Some(c) = item.cost {
            self.cost += c;
            self.cost_items += 1;
            if item.cost_estimated {
                self.cost_estimated += 1;
            }
            if let Some(p) = item.alt_points {
                self.alt_points += p;
                self.alt_money += c;
                self.alt_items += 1;
            }
        }
        if let Some(p) = item.cost_points {
            self.cost_points += p;
            self.cost_points_items += 1;
        }
        if item.unpriced {
            self.unpriced += 1;
        }
        if item.tags.contains(&tag::PRICE_SPIKE) {
            self.spikes += 1;
        }
        if item.tags.contains(&tag::PRICE_DIP) {
            self.dips += 1;
        }
        if item.counted && item.marketable && item.price.is_some() {
            self.marketable += 1;
            if item.price.as_ref().is_some_and(|p| p.history_at.is_some()) {
                self.with_history += 1;
            }
        }
    }

    fn merge(&mut self, o: &PriceTotals, needed_too: bool) {
        self.value += o.value;
        self.value_copies += o.value_copies;
        self.value_unpriced += o.value_unpriced;
        self.points_value += o.points_value;
        if needed_too {
            self.cost += o.cost;
            self.cost_items += o.cost_items;
            self.cost_estimated += o.cost_estimated;
            self.cost_points += o.cost_points;
            self.cost_points_items += o.cost_points_items;
            self.alt_points += o.alt_points;
            self.alt_money += o.alt_money;
            self.alt_items += o.alt_items;
            self.unpriced += o.unpriced;
            self.spikes += o.spikes;
            self.dips += o.dips;
            self.marketable += o.marketable;
            self.with_history += o.with_history;
        }
    }
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
    pub prices: PriceTotals,
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
    /// Value and points cover every game listed and Steam's own items; cost
    /// to complete covers the games that count.
    pub prices: PriceTotals,
    /// The currency prices are shown in.
    pub currency: String,
    /// Set when prices can't be shown in the chosen currency yet.
    pub currency_note: Option<String>,
    /// When the latest Market search finished (Unix time).
    pub prices_at: Option<i64>,
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
const NOTE_RETIRED_ON_MARKET: &str =
    "Steam doesn't hand this out any more, but it's for sale on the Market.";
const NOTE_RETIRED_OFF_MARKET: &str =
    "Steam doesn't hand this out any more, and nobody is selling it on the Market right now.";

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
    /// When the game's latest Market search started.
    market_checked_at: Option<i64>,
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
    /// The Market name, from an owned copy.
    hash_name: Option<String>,
    /// The item's Market row.
    market: Option<usize>,
    /// Known only from the Market: Steam lists it nowhere else.
    market_only: bool,
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
             points_checked_at IS NOT NULL, market_checked_at
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
                market_checked_at: r.get(8)?,
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
    let rows = crate::db::market_rows(c)?;
    let fx = Fx::load(c, rules.prices.currency)?;

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
            if e.hash_name.is_none() {
                e.hash_name = mhn.clone();
            }
            if exact {
                e.exact_hit = true;
            } else {
                e.name_hit = true;
            }
        }
    }

    match_market(&mut entries, &rows, |a| listed(a).is_some());

    // Each item's view, in a steady order: game, kind, card series, item number.
    let ctx = ViewCtx {
        rules,
        badges: &badges,
        overrides: &overrides,
        now,
        rows: &rows,
        fx: &fx,
    };
    let mut keyed: Vec<((u64, Group, i64, i64), ItemView)> = Vec::with_capacity(entries.len());
    for e in &entries {
        let Some(app) = listed(e.appid) else { continue };
        let v = view(e, app, &ctx);
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
        g.prices.add_item(item);
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
        currency: market::currency_code(fx.shown).unwrap_or("USD").to_string(),
        currency_note: fx.note.clone(),
        ..AccountCompletion::default()
    };
    // Value covers every listed game; what's still needed only the ones that count.
    for g in games.values() {
        account.prices.merge(&g.prices, g.counts_for_account);
    }
    let (steam_value, steam_copies) = steam_items_value(c, &rows, &fx)?;
    account.prices.value += steam_value;
    account.prices.value_copies += steam_copies;
    account.prices_at = apps
        .values()
        .filter(|a| a.counts_for_account(rules))
        .filter_map(|a| a.market_checked_at)
        .max();
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

/// Whether the item can be sold on the Market. Once a game's Market search
/// has run, that settles it: what the Market lists is marketable, and what it
/// doesn't isn't (cards and booster packs always are, even with no listings).
/// Before then, cards, booster packs and a card set's emoticons and
/// backgrounds are taken to be. A copy the user can sell settles it too.
fn is_marketable(e: &Entry, app: App) -> bool {
    if e.market.is_some() || e.marketable_copies > 0 {
        return true;
    }
    match e.class {
        class::CARD | class::BOOSTER => true,
        _ if app.market_checked_at.is_some() => false,
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

fn auto_availability(
    e: &Entry,
    app: App,
    on_market: bool,
    now: i64,
) -> (Availability, Option<String>) {
    if e.unlisted {
        if (e.class == class::CARD || e.class == class::BOOSTER) && !e.market_only {
            let note = if app.catalog_read {
                NOTE_CARD_MISMATCH
            } else {
                NOTE_NOT_READ
            };
            return (Availability::Available, Some(note.to_string()));
        }
        // Steam hands it out nowhere, but the Market may still sell it.
        if on_market {
            return (
                Availability::Available,
                Some(NOTE_RETIRED_ON_MARKET.to_string()),
            );
        }
        if e.market_only {
            return (
                Availability::Unobtainable,
                Some(NOTE_RETIRED_OFF_MARKET.to_string()),
            );
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

struct ViewCtx<'a> {
    rules: Rules,
    badges: &'a HashMap<(u64, bool), Vec<(i64, u32)>>,
    overrides: &'a HashMap<String, Availability>,
    now: i64,
    rows: &'a [MarketRow],
    fx: &'a Fx,
}

fn view(e: &Entry, app: App, ctx: &ViewCtx<'_>) -> ItemView {
    let rules = ctx.rules;
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
        let level = badge_level(ctx.badges, e.appid, e.foil, e.series).min(levels);
        let have = (level as u64 + held).min(needed as u64) as u32;
        (have, needed)
    } else {
        (u32::from(held > 0), 1)
    };

    let row = e.market.and_then(|i| ctx.rows.get(i));
    let price = row.map(|r| item_price(r, app, ctx));
    let on_market = price.as_ref().is_some_and(|p| p.on_market);
    let (auto, note) = auto_availability(e, app, on_market, ctx.now);
    let overridden = ctx.overrides.get(&key).copied();
    let availability = overridden.unwrap_or(auto);
    let marketable = is_marketable(e, app);
    let counted = e.class != class::BOOSTER && availability != Availability::Unobtainable;

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
    if let Some(t) = price.as_ref().and_then(|p| trend(p, rules.prices)) {
        tags.push(t);
    }
    let unmatched_card =
        e.unlisted && !e.market_only && (e.class == class::CARD || e.class == class::BOOSTER);
    if (e.name_hit && !e.exact_hit) || unmatched_card || note.as_deref() == Some(NOTE_NOT_READ) {
        tags.push(tag::UNVERIFIED);
    }

    // What the copies the user can sell are worth: today's cheapest listing,
    // or the average when nobody is selling.
    let sellable = e.marketable_copies;
    let unit = price.as_ref().and_then(|p| p.lowest.or(p.average));
    let value = unit.filter(|_| sellable > 0).map(|u| u * sellable as i64);

    // What's still needed: in money when the Market sells it, otherwise in
    // points when the Points Shop does.
    let copies = i64::from(needed.saturating_sub(have));
    let points_ok = points_buyable(e, ctx.now);
    let mut cost = None;
    let mut cost_estimated = false;
    let mut cost_points = None;
    let mut alt_points = None;
    let mut unpriced = false;
    if counted && copies > 0 {
        let money = match (row, price.as_ref()) {
            (Some(r), Some(p)) if marketable => money_for(r, p, copies, ctx),
            _ => None,
        };
        if let Some((m, exact)) = money {
            cost = Some(m);
            cost_estimated = !exact;
            if points_ok {
                alt_points = e.points_cost.map(|c| c * copies);
            }
        } else if points_ok {
            cost_points = e.points_cost.map(|c| c * copies);
        } else {
            unpriced = true;
        }
    }

    let image = e.image.clone().or_else(|| {
        row.and_then(|r| r.icon.as_deref())
            .map(market::economy_image)
    });

    ItemView {
        key,
        appid: e.appid,
        class: e.class,
        item_type: e.item_type,
        foil: e.foil,
        group,
        kind: kind_label(e.class, e.foil, e.animated),
        name,
        image,
        held,
        have,
        needed,
        progress: f64::from(have) / f64::from(needed.max(1)),
        counted,
        points_cost: e.points_cost.filter(|_| e.in_points),
        marketable,
        availability,
        auto_availability: auto,
        note,
        overridden: overridden.is_some(),
        tags,
        price,
        sellable,
        value,
        cost,
        cost_estimated,
        cost_points,
        alt_points,
        unpriced,
    }
}

/// Sold in the Points Shop right now, on its own or in a bundle.
fn points_buyable(e: &Entry, now: i64) -> bool {
    e.in_points
        && e.points_cost.is_some_and(|c| c > 0)
        && e.points_active != Some(false)
        && e.points_until.is_none_or(|u| u > now)
        && e.points_from.is_none_or(|f| f <= now)
        && e.points_buyable != Some(0)
}

/// Listed for sale in the game's latest Market search.
fn on_market(r: &MarketRow, app: App) -> bool {
    r.listings > 0 && r.lowest.is_some() && app.market_checked_at.is_none_or(|t| r.seen_at >= t)
}

fn item_price(r: &MarketRow, app: App, ctx: &ViewCtx<'_>) -> ItemPrice {
    let fx = ctx.fx;
    let hc = r.history_currency.unwrap_or(r.currency);
    let conv = |v: Option<i64>, from: i64| v.and_then(|m| fx.convert(m, from));
    let listed = on_market(r, app);
    let lowest = if listed {
        conv(r.lowest, r.currency)
    } else {
        None
    };
    let (avg_30, avg_90, avg_365, avg_all) = (
        conv(r.avg_30, hc),
        conv(r.avg_90, hc),
        conv(r.avg_365, hc),
        conv(r.avg_all, hc),
    );
    let average = match ctx.rules.prices.window {
        PriceWindow::Month => avg_30,
        PriceWindow::Quarter => avg_90,
        PriceWindow::Year => avg_365,
        PriceWindow::All => avg_all,
    };
    let change = match (lowest, average) {
        (Some(l), Some(a)) if a > 0 => Some(round1(100.0 * (l - a) as f64 / a as f64)),
        _ => None,
    };
    ItemPrice {
        hash_name: r.hash_name.clone(),
        on_market: listed,
        lowest,
        listings: if listed { r.listings } else { 0 },
        average,
        change,
        avg_30,
        avg_90,
        avg_365,
        avg_all,
        low: conv(r.low_price, hc),
        low_at: r.low_day.map(|d| d * DAY),
        high: conv(r.high_price, hc),
        high_at: r.high_day.map(|d| d * DAY),
        sales_30: r.sales_30,
        buy_max: conv(r.buy_max, r.book_currency.unwrap_or(r.currency)),
        seen_at: r.seen_at,
        history_at: r.history_at,
        book_at: r.book_at,
    }
}

/// A spike or a dip: today's cheapest listing far enough from the average,
/// in percent and in cents.
fn trend(p: &ItemPrice, rules: PriceRules) -> Option<&'static str> {
    let (l, a) = (p.lowest?, p.average?);
    if a <= 0 {
        return None;
    }
    let diff = l - a;
    let share = f64::from(rules.spike_percent) / 100.0;
    let min = i64::from(rules.spike_min_cents) * 10;
    if diff >= min && diff as f64 >= share * a as f64 {
        Some(tag::PRICE_SPIKE)
    } else if -diff >= min && (-diff) as f64 >= share * a as f64 {
        Some(tag::PRICE_DIP)
    } else {
        None
    }
}

/// The cost of `copies` copies from the Market, cheapest first. A fresh order
/// book prices each copy; without one, every copy is priced at today's
/// cheapest listing and the cost is marked as estimated.
fn money_for(r: &MarketRow, p: &ItemPrice, copies: i64, ctx: &ViewCtx<'_>) -> Option<(i64, bool)> {
    let lowest = p.lowest?;
    if copies <= 1 {
        return Some((lowest * copies.max(0), true));
    }
    if let (Some(at), Some(text)) = (r.book_at, r.book.as_deref()) {
        if ctx.now - at <= BOOK_FRESH_DAYS * DAY {
            let from = r.book_currency.unwrap_or(r.currency);
            let levels: Vec<(i64, i64)> = book::from_text(text)
                .into_iter()
                .filter_map(|(price, qty)| Some((ctx.fx.convert(price, from)?, qty)))
                .collect();
            if let Some(found) = book::cost(&levels, copies) {
                return Some(found);
            }
        }
    }
    Some((lowest * copies, false))
}

/// Ties Market items to catalog items: by the Market name of a copy the user
/// owns, then by kind and name (each game has one booster pack). Emoticons
/// and backgrounds only the Market knows about join their game's list, so
/// retired items that can still be bought count.
fn match_market(entries: &mut Vec<Entry>, rows: &[MarketRow], listed: impl Fn(u64) -> bool) {
    let by_hash: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.hash_name.as_str(), i))
        .collect();
    let mut used = vec![false; rows.len()];
    for e in entries.iter_mut() {
        if let Some(&i) = e.hash_name.as_deref().and_then(|h| by_hash.get(h)) {
            if rows[i].appid == e.appid {
                e.market = Some(i);
                used[i] = true;
            }
        }
    }

    // Items that share a name get Steam's item number on the end of their
    // Market name ("…(Profile Background)-13"), which matches exactly.
    let mut typed: HashMap<(u64, i64, i64, bool), usize> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if !used[i] {
            if let Some(t) = market::type_suffix(&r.hash_name, r.appid, &r.name) {
                typed.entry((r.appid, r.class, t, r.foil)).or_insert(i);
            }
        }
    }
    for e in entries.iter_mut().filter(|e| e.market.is_none()) {
        let hit = e
            .item_type
            .and_then(|t| typed.get(&(e.appid, e.class, t, e.foil)).copied());
        if let Some(i) = hit.filter(|i| !used[*i]) {
            e.market = Some(i);
            used[i] = true;
        }
    }

    let mut names: HashMap<(u64, i64, bool, String), usize> = HashMap::new();
    let mut boosters: HashMap<u64, usize> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if used[i] || r.class == 0 || !listed(r.appid) {
            continue;
        }
        // A numbered duplicate that matched nothing above isn't matched by name.
        if market::type_suffix(&r.hash_name, r.appid, &r.name).is_some() {
            continue;
        }
        if r.class == class::BOOSTER {
            boosters.entry(r.appid).or_insert(i);
        } else {
            names
                .entry((r.appid, r.class, r.foil, norm(market::plain_name(&r.name))))
                .or_insert(i);
        }
    }
    for e in entries.iter_mut().filter(|e| e.market.is_none()) {
        let hit = if e.class == class::BOOSTER {
            boosters.get(&e.appid).copied()
        } else {
            [e.title.as_deref(), e.name.as_deref()]
                .into_iter()
                .flatten()
                .find_map(|n| names.get(&(e.appid, e.class, e.foil, norm(n))).copied())
        };
        if let Some(i) = hit.filter(|i| !used[*i]) {
            e.market = Some(i);
            used[i] = true;
        }
    }

    for (i, r) in rows.iter().enumerate() {
        if used[i] || !listed(r.appid) || !matches!(r.class, class::EMOTICON | class::BACKGROUND) {
            continue;
        }
        let name = market::plain_name(&r.name).to_string();
        entries.push(Entry {
            appid: r.appid,
            class: r.class,
            item_type: None,
            foil: r.foil,
            name: Some(name.clone()),
            title: Some(name),
            unlisted: true,
            market_only: true,
            market: Some(i),
            ..Entry::default()
        });
    }
}

/// Sellable copies of Steam's own items, like sacks of gems: (value, copies).
fn steam_items_value(c: &Connection, rows: &[MarketRow], fx: &Fx) -> rusqlite::Result<(i64, u64)> {
    let by_hash: HashMap<&str, &MarketRow> =
        rows.iter().map(|r| (r.hash_name.as_str(), r)).collect();
    let mut value = 0i64;
    let mut copies = 0u64;
    for (hash, qty) in crate::db::steam_item_copies(c)? {
        let Some(r) = by_hash.get(hash.as_str()) else {
            continue;
        };
        if let Some(unit) = r
            .lowest
            .filter(|_| r.listings > 0)
            .and_then(|m| fx.convert(m, r.currency))
        {
            value += unit * qty as i64;
            copies += qty;
        }
    }
    Ok((value, copies))
}

/// Converts prices into the currency they're shown in, for callers outside
/// completion (the price history view).
pub struct Converter(Fx);

impl Converter {
    pub fn load(c: &Connection, wanted: i64) -> rusqlite::Result<Self> {
        Ok(Self(Fx::load(c, wanted)?))
    }

    pub fn shown(&self) -> i64 {
        self.0.shown
    }

    pub fn convert(&self, mills: i64, from: i64) -> Option<i64> {
        self.0.convert(mills, from)
    }
}

/// Converts Market prices into the currency they're shown in.
struct Fx {
    /// The currency prices are shown in.
    shown: i64,
    /// The currency Steam answers in for this account.
    main: i64,
    base: Option<i64>,
    /// Units of each currency per one of `base`.
    rates: HashMap<i64, f64>,
    note: Option<String>,
}

impl Fx {
    fn load(c: &Connection, wanted: i64) -> rusqlite::Result<Fx> {
        let main = crate::db::get_meta(c, market::META_CURRENCY)?
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|id| *id > 0)
            .unwrap_or(wanted);
        let mut fx = Fx {
            shown: main,
            main,
            base: None,
            rates: HashMap::new(),
            note: None,
        };
        if let Some(text) = crate::db::get_meta(c, market::META_RATES)? {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                fx.base = v["base"].as_str().and_then(market::currency_id);
                if let Some(map) = v["rates"].as_object() {
                    for (code, rate) in map {
                        if let (Some(id), Some(r)) = (market::currency_id(code), rate.as_f64()) {
                            if r > 0.0 {
                                fx.rates.insert(id, r);
                            }
                        }
                    }
                }
            }
        }
        if wanted == main || (fx.rate(main).is_some() && fx.rate(wanted).is_some()) {
            fx.shown = wanted;
        } else {
            let code = |id| market::currency_code(id).unwrap_or("?");
            fx.note = Some(format!(
                "Prices are shown in {} until exchange rates for {} are read. Scan again to read them.",
                code(main),
                code(wanted)
            ));
        }
        Ok(fx)
    }

    fn rate(&self, id: i64) -> Option<f64> {
        if Some(id) == self.base {
            Some(1.0)
        } else {
            self.rates.get(&id).copied()
        }
    }

    /// Mills in `from` (0 when unknown, meaning Steam's usual currency) to
    /// mills in the shown currency.
    fn convert(&self, mills: i64, from: i64) -> Option<i64> {
        let from = if from <= 0 { self.main } else { from };
        if from == self.shown {
            return Some(mills);
        }
        let ratio = self.rate(self.shown)? / self.rate(from)?;
        Some((mills as f64 * ratio).round() as i64)
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
            prices: PriceRules::default(),
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
    fn prices_value_and_cost() {
        let (db, dir) = setup("prices");
        db.write(|tx| {
            tx.execute_batch(
                "UPDATE apps SET market_checked_at = 50 WHERE appid = 10;
                 INSERT INTO meta (key, value) VALUES ('market_currency', '1');
                 INSERT INTO market (hash_name, appid, class, foil, name, currency, lowest, listings, seen_at,
                                     avg_30, sales_30, history_at, book_at, book) VALUES
                     ('10-Alpha', 10, 2, 0, 'Alpha', 1, 100, 5, 60, 100, 3, 60, NULL, NULL),
                     ('10-Beta (Trading Card)', 10, 2, 0, 'Beta (Trading Card)', 1, 200, 9, 60, 100, 5, 60, 95, '200:1 300:5'),
                     ('10-Beta (Foil)', 10, 2, 1, 'Beta (Foil)', 1, 900, 1, 60, NULL, NULL, NULL, NULL, NULL),
                     ('10-:smile:', 10, 4, 0, ':smile:', 1, 30, 4, 60, NULL, NULL, NULL, NULL, NULL),
                     ('10-:retired:', 10, 4, 0, ':retired:', 1, 50, 2, 60, NULL, NULL, NULL, NULL, NULL),
                     ('10-:gone:', 10, 4, 0, ':gone:', 1, NULL, 0, 40, NULL, NULL, NULL, NULL, NULL),
                     ('10-Card Game Booster Pack', 10, 5, 0, 'Card Game Booster Pack', 1, 400, 3, 60,
                      NULL, NULL, NULL, NULL, NULL);",
            )
        })
        .unwrap();
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        let items = snap.game_items(10);
        let find = |name: &str, foil: bool| {
            items
                .iter()
                .find(|i| i.name == name && i.foil == foil)
                .unwrap_or_else(|| panic!("{name} missing"))
                .clone()
        };

        let alpha = find("Alpha", false);
        assert_eq!(alpha.cost, None, "complete");
        assert_eq!((alpha.sellable, alpha.value), (4, Some(400)));

        // Two more copies walk up the order book: 200 + 300.
        let beta = find("Beta", false);
        assert_eq!((beta.cost, beta.cost_estimated), (Some(500), false));
        assert!(beta.tags.contains(&tag::PRICE_SPIKE), "{:?}", beta.tags);
        assert_eq!(beta.price.as_ref().unwrap().change, Some(100.0));

        let beta_foil = find("Beta", true);
        assert_eq!(beta_foil.value, Some(900));
        let alpha_foil = find("Alpha", true);
        assert!(alpha_foil.unpriced, "no listing for it");

        let smile = find(":smile:", false);
        assert!(smile.marketable && smile.price.is_some());
        let frown = find(":frown:", false);
        assert!(!frown.marketable, "the Market search didn't list it");
        assert!(frown.tags.contains(&tag::POINTS) && frown.unpriced);
        let sky = find("Sky", false);
        assert_eq!(sky.cost_points, Some(500));

        // Retired items: counted while someone sells them.
        let retired = find(":retired:", false);
        assert_eq!(retired.availability, Availability::Available);
        assert!(retired.counted && retired.marketable);
        assert_eq!(retired.cost, Some(50));
        let gone = find(":gone:", false);
        assert_eq!(gone.availability, Availability::Unobtainable);
        assert!(!gone.counted);

        let g = &snap.games[&10];
        assert_eq!(g.total, 9, "the retired emoticon joins the count");
        let p = &g.prices;
        assert_eq!((p.cost, p.cost_items, p.cost_points), (550, 2, 500));
        assert_eq!(p.unpriced, 2);
        assert_eq!((p.value, p.value_copies), (1300, 5));
        assert_eq!(
            p.points_value,
            100 + 3000,
            ":smile: and the animated avatar"
        );
        assert_eq!(p.spikes, 1);
        assert_eq!(snap.account.prices.cost, 550);
        assert_eq!(snap.account.currency, "USD");
        assert_eq!(snap.account.prices_at, Some(50));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_sharing_a_name_match_by_number() {
        let (db, dir) = setup("numbered");
        db.write(|tx| {
            tx.execute_batch(
                "UPDATE apps SET market_checked_at = 50 WHERE appid = 10;
                 INSERT INTO items (appid, class, item_type, foil, name, title, in_points, points_cost, points_active, points_buyable, updated_at)
                     VALUES (10, 3, 16, 0, 'Sky', 'Sky', 1, 500, 1, 1, 0);
                 INSERT INTO market (hash_name, appid, class, foil, name, currency, lowest, listings, seen_at) VALUES
                     ('10-Sky (Profile Background)-16', 10, 3, 0, 'Sky (Profile Background)', 1, 90, 3, 60),
                     ('10-Sky (Profile Background)-6', 10, 3, 0, 'Sky (Profile Background)', 1, 40, 3, 60);",
            )
        })
        .unwrap();
        let snap = db.read(|c| compute(c, rules(false, false), 100)).unwrap();
        let skies: Vec<ItemView> = snap
            .game_items(10)
            .into_iter()
            .filter(|i| i.name == "Sky")
            .collect();
        assert_eq!(skies.len(), 2, "no extra Market-only copy");
        let cost = |t: i64| skies.iter().find(|i| i.item_type == Some(t)).unwrap().cost;
        assert_eq!((cost(6), cost(16)), (Some(40), Some(90)));
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
