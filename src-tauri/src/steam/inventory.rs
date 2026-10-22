//! Reading the Steam Community inventory (app 753, context 6).
//!
//! Each page has a list of `assets` (the items) and `descriptions` (what each
//! kind of item is). They're joined on classid + instanceid. An item's type
//! comes from its `item_class` tag, its game from its `Game` tag.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
}

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
            }
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
