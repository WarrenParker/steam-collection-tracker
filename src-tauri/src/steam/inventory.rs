//! Reading the Steam Community inventory (app 753, context 6).
//!
//! Each page has a list of `assets` (the items) and `descriptions` (what each
//! kind of item is). They're joined on classid + instanceid. An item's type
//! comes from its `item_class` tag, its game from its `Game` tag.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct Tally {
    /// Number of item entries (a stack of gems counts once).
    pub items: u64,
    /// Type name -> (entries, total quantity).
    pub by_type: BTreeMap<String, (u64, u64)>,
    pub foil_cards: u64,
    /// Games the items belong to.
    pub apps: BTreeSet<u64>,
    /// Game names from the items' `Game` tags, for games the user doesn't own.
    pub app_names: BTreeMap<u64, String>,
    /// Every kind of item held, keyed by classid and instanceid.
    pub rows: BTreeMap<(String, String), InvRow>,
    /// The first description seen for each item class, for the connection test.
    pub samples: BTreeMap<i64, Value>,
}

/// One kind of item in the inventory, with how many the user has.
#[derive(Debug, Clone, PartialEq)]
pub struct InvRow {
    pub classid: String,
    pub instanceid: String,
    pub appid: Option<u64>,
    /// The `item_class` number: 2 card, 3 background, 4 emoticon, 5 booster…
    pub class: Option<i64>,
    /// The item's type number within its game, when Steam reveals it.
    pub item_type: Option<i64>,
    pub foil: bool,
    pub name: Option<String>,
    pub market_hash_name: Option<String>,
    pub type_label: Option<String>,
    pub marketable: Option<bool>,
    pub tradable: Option<bool>,
    pub icon: Option<String>,
    /// Separate entries (stacks) of this item.
    pub stacks: u64,
    /// Total quantity across those entries.
    pub quantity: u64,
}

/// "Turn into Gems" links name the item's game, type number and border:
/// `GetGooValue( '%contextid%', '%assetid%', 440, 3, 0 )`.
static GOO_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"GetGooValue\(\s*'[^']*'\s*,\s*'[^']*'\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)")
        .unwrap()
});

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PageInfo {
    pub more: bool,
    pub last_assetid: Option<String>,
    pub total: Option<u64>,
    pub assets: usize,
}

pub fn tally_page(v: &Value, tally: &mut Tally) -> PageInfo {
    let mut descriptions: HashMap<String, &Value> = HashMap::new();
    if let Some(list) = v["descriptions"].as_array() {
        for d in list {
            descriptions.insert(join_key(d), d);
        }
    }

    let assets: &[Value] = v["assets"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    for asset in assets {
        let desc = descriptions.get(&join_key(asset)).copied();
        let amount = as_u64(&asset["amount"]).unwrap_or(1).max(1);
        let label = desc.map(type_label).unwrap_or_else(|| "Other".to_string());
        tally.items += 1;
        let entry = tally.by_type.entry(label).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += amount;
        if let Some(d) = desc {
            if is_foil_card(d) {
                tally.foil_cards += 1;
            }
            if let Some(app) = app_of(d) {
                tally.apps.insert(app);
                if let Some(name) = tag(d, "Game")
                    .and_then(|t| t["localized_tag_name"].as_str())
                    .filter(|n| !n.is_empty())
                {
                    tally
                        .app_names
                        .entry(app)
                        .or_insert_with(|| name.to_string());
                }
            }
        }
        let key = (text_of(&asset["classid"]), text_of(&asset["instanceid"]));
        let row = tally
            .rows
            .entry(key.clone())
            .or_insert_with(|| new_row(key.0, key.1, desc));
        row.stacks += 1;
        row.quantity += amount;
        if let (Some(d), Some(class_id)) = (desc, row.class) {
            tally.samples.entry(class_id).or_insert_with(|| d.clone());
        }
    }

    let more = as_u64(&v["more_items"]).unwrap_or(0) == 1;
    PageInfo {
        more,
        last_assetid: v["last_assetid"].as_str().map(str::to_string),
        total: as_u64(&v["total_inventory_count"]),
        assets: assets.len(),
    }
}

fn new_row(classid: String, instanceid: String, desc: Option<&Value>) -> InvRow {
    let Some(d) = desc else {
        return InvRow {
            classid,
            instanceid,
            appid: None,
            class: None,
            item_type: None,
            foil: false,
            name: None,
            market_hash_name: None,
            type_label: None,
            marketable: None,
            tradable: None,
            icon: None,
            stacks: 0,
            quantity: 0,
        };
    };
    let goo = goo_value(d);
    InvRow {
        classid,
        instanceid,
        appid: app_of(d).or(goo.map(|g| g.0)),
        class: item_class(d),
        item_type: goo.map(|g| g.1 as i64),
        foil: tag(d, "cardborder").and_then(|t| t["internal_name"].as_str())
            == Some("cardborder_1")
            || goo.is_some_and(|g| g.2 == 1),
        name: non_empty(&d["name"]),
        market_hash_name: non_empty(&d["market_hash_name"]),
        type_label: non_empty(&d["type"]),
        marketable: as_u64(&d["marketable"]).map(|m| m != 0),
        tradable: as_u64(&d["tradable"]).map(|t| t != 0),
        icon: non_empty(&d["icon_url"]),
        stacks: 0,
        quantity: 0,
    }
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The item's class number from its `item_class_N` tag.
pub fn item_class(d: &Value) -> Option<i64> {
    tag(d, "item_class")
        .and_then(|t| t["internal_name"].as_str())
        .and_then(|n| n.strip_prefix("item_class_"))
        .and_then(|n| n.parse().ok())
}

/// (game, type number, border) from the item's "Turn into Gems" action.
fn goo_value(d: &Value) -> Option<(u64, u64, u64)> {
    let actions = d["owner_actions"].as_array()?;
    actions.iter().find_map(|a| {
        let link = a["link"].as_str()?;
        let c = GOO_VALUE.captures(link)?;
        Some((c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?))
    })
}

fn join_key(v: &Value) -> String {
    format!("{}_{}", text_of(&v["classid"]), text_of(&v["instanceid"]))
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => "0".to_string(),
    }
}

/// Steam mixes numbers and numeric strings, and uses 1/0 for booleans.
pub fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(u64::from(*b)),
        _ => None,
    }
}

fn tag<'a>(d: &'a Value, category: &str) -> Option<&'a Value> {
    d["tags"]
        .as_array()?
        .iter()
        .find(|t| t["category"].as_str() == Some(category))
}

/// "Trading Card", "Emoticon", "Profile Background", "Steam Gems"…
pub fn type_label(d: &Value) -> String {
    tag(d, "item_class")
        .and_then(|t| t["localized_tag_name"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Other")
        .to_string()
}

pub fn is_foil_card(d: &Value) -> bool {
    tag(d, "item_class").and_then(|t| t["internal_name"].as_str()) == Some("item_class_2")
        && tag(d, "cardborder").and_then(|t| t["internal_name"].as_str()) == Some("cardborder_1")
}

pub fn app_of(d: &Value) -> Option<u64> {
    tag(d, "Game")
        .and_then(|t| t["internal_name"].as_str())
        .and_then(|n| n.strip_prefix("app_"))
        .and_then(|n| n.parse().ok())
        .or_else(|| as_u64(&d["market_fee_app"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page() -> Value {
        json!({
            "assets": [
                {"appid": 753, "contextid": "6", "assetid": "1", "classid": "100", "instanceid": "0", "amount": "1"},
                {"appid": 753, "contextid": "6", "assetid": "2", "classid": "101", "instanceid": "0", "amount": "1"},
                {"appid": 753, "contextid": "6", "assetid": "3", "classid": "102", "instanceid": "0", "amount": "1500"},
                {"appid": 753, "contextid": "6", "assetid": "4", "classid": "999", "instanceid": "0", "amount": "1"}
            ],
            "descriptions": [
                {"classid": "100", "instanceid": "0", "market_fee_app": 440, "type": "Team Fortress 2 Trading Card",
                 "name": "Pyro", "market_hash_name": "440-Pyro", "marketable": 1, "tradable": 1,
                 "owner_actions": [{"link": "javascript:GetGooValue( '%contextid%', '%assetid%', 440, 3, 0 )", "name": "Turn into Gems..."}],
                 "tags": [
                    {"category": "Game", "internal_name": "app_440", "localized_tag_name": "Team Fortress 2"},
                    {"category": "item_class", "internal_name": "item_class_2", "localized_tag_name": "Trading Card"},
                    {"category": "cardborder", "internal_name": "cardborder_0", "localized_tag_name": "Normal"}
                 ]},
                {"classid": "101", "instanceid": "0", "market_fee_app": "570",
                 "tags": [
                    {"category": "item_class", "internal_name": "item_class_2", "localized_tag_name": "Trading Card"},
                    {"category": "cardborder", "internal_name": "cardborder_1", "localized_tag_name": "Foil"}
                 ]},
                {"classid": "102", "instanceid": "0",
                 "tags": [
                    {"category": "Game", "internal_name": "app_753", "localized_tag_name": "Steam"},
                    {"category": "item_class", "internal_name": "item_class_7", "localized_tag_name": "Steam Gems"}
                 ]}
            ],
            "more_items": 1,
            "last_assetid": "4",
            "total_inventory_count": 2400,
            "success": 1
        })
    }

    #[test]
    fn tallies_a_page() {
        let mut t = Tally::default();
        let info = tally_page(&page(), &mut t);
        assert_eq!(info.assets, 4);
        assert!(info.more);
        assert_eq!(info.last_assetid.as_deref(), Some("4"));
        assert_eq!(info.total, Some(2400));
        assert_eq!(t.items, 4);
        assert_eq!(t.by_type["Trading Card"], (2, 2));
        assert_eq!(t.by_type["Steam Gems"], (1, 1500));
        assert_eq!(t.by_type["Other"], (1, 1));
        assert_eq!(t.foil_cards, 1);
        assert_eq!(
            t.apps.iter().copied().collect::<Vec<_>>(),
            vec![440, 570, 753]
        );
        assert_eq!(
            t.app_names.get(&440).map(String::as_str),
            Some("Team Fortress 2")
        );
        assert_eq!(t.rows.len(), 4);
        let pyro = &t.rows[&("100".to_string(), "0".to_string())];
        assert_eq!(pyro.class, Some(2));
        assert_eq!(pyro.item_type, Some(3));
        assert_eq!(pyro.appid, Some(440));
        assert_eq!(pyro.market_hash_name.as_deref(), Some("440-Pyro"));
        assert_eq!(pyro.marketable, Some(true));
        assert!(!pyro.foil);
        let foil = &t.rows[&("101".to_string(), "0".to_string())];
        assert!(foil.foil);
        assert_eq!(foil.item_type, None);
        let gems = &t.rows[&("102".to_string(), "0".to_string())];
        assert_eq!((gems.stacks, gems.quantity), (1, 1500));
        assert_eq!(gems.class, Some(7));
        assert!(t.rows[&("999".to_string(), "0".to_string())]
            .class
            .is_none());
        assert_eq!(t.samples.keys().copied().collect::<Vec<_>>(), vec![2, 7]);
    }

    #[test]
    fn rows_add_up_across_stacks() {
        let page = json!({
            "assets": [
                {"assetid": "1", "classid": "5", "instanceid": "0", "amount": "1"},
                {"assetid": "2", "classid": "5", "instanceid": "0", "amount": "1"}
            ],
            "descriptions": [
                {"classid": "5", "instanceid": "0", "marketable": true,
                 "tags": [{"category": "item_class", "internal_name": "item_class_4", "localized_tag_name": "Emoticon"}]}
            ]
        });
        let mut t = Tally::default();
        tally_page(&page, &mut t);
        let row = &t.rows[&("5".to_string(), "0".to_string())];
        assert_eq!((row.stacks, row.quantity), (2, 2));
        assert_eq!(row.class, Some(4));
        assert_eq!(row.marketable, Some(true));
    }

    #[test]
    fn empty_inventory() {
        let mut t = Tally::default();
        let info = tally_page(&json!({"total_inventory_count": 0, "success": 1}), &mut t);
        assert_eq!(info.assets, 0);
        assert!(!info.more);
        assert_eq!(t.items, 0);
    }
}
