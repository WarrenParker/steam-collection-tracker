//! The store check (`IStoreBrowseService/GetItems`): for a batch of games,
//! their names, what kind of product each is, and whether it has Steam
//! Trading Cards. Games without cards don't need a catalog request.

use serde_json::{json, Value};

use crate::jsonutil::{int, text};

/// The store's "Steam Trading Cards" category.
pub const TRADING_CARDS_CATEGORY: i64 = 29;
/// How many games one request asks about.
pub const APPS_PER_REQUEST: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct StoreApp {
    pub appid: u64,
    /// False when the store doesn't know the game (removed from sale, for example).
    pub found: bool,
    pub name: Option<String>,
    /// Steam's product type: 0 game, 1 demo, 4 DLC, 6 software and so on.
    pub kind: Option<i64>,
    /// `None` when the answer had no categories to tell from.
    pub has_cards: Option<bool>,
}

/// The `input_json` value for a request.
pub fn request_json(appids: &[u64]) -> String {
    let ids: Vec<Value> = appids.iter().map(|a| json!({ "appid": a })).collect();
    json!({
        "ids": ids,
        "context": { "language": "english", "country_code": "US" },
        "data_request": { "include_basic_info": true }
    })
    .to_string()
}

pub fn parse(v: &Value) -> Option<Vec<StoreApp>> {
    let list = v.get("response")?.get("store_items")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|item| {
                let appid = int(&item["appid"])
                    .or_else(|| int(&item["id"]))
                    .filter(|a| *a > 0)? as u64;
                let found = int(&item["success"]).map_or(true, |s| s == 1);
                let cats = &item["categories"];
                let has_cards = if cats.is_object() {
                    let mut ids = [
                        "feature_categoryids",
                        "supported_player_categoryids",
                        "controller_categoryids",
                    ]
                    .iter()
                    .filter_map(|k| cats[*k].as_array())
                    .flatten()
                    .filter_map(int);
                    Some(ids.any(|c| c == TRADING_CARDS_CATEGORY))
                } else {
                    None
                };
                Some(StoreApp {
                    appid,
                    found,
                    name: text(&item["name"]),
                    kind: int(&item["type"]),
                    has_cards: if found { has_cards } else { None },
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_request() {
        let v: Value = serde_json::from_str(&request_json(&[440, 570])).unwrap();
        assert_eq!(v["ids"][1]["appid"], json!(570));
        assert_eq!(v["data_request"]["include_basic_info"], json!(true));
    }

    #[test]
    fn reads_items() {
        let v = json!({"response": {"store_items": [
            {"item_type": 0, "id": 440, "success": 1, "visible": true, "name": "Team Fortress 2", "appid": 440, "type": 0,
             "categories": {"supported_player_categoryids": [1, 49], "feature_categoryids": [22, 29, 30]}},
            {"item_type": 0, "id": 570, "success": 1, "name": "Dota 2", "appid": 570, "type": 0,
             "categories": {"feature_categoryids": [22]}},
            {"item_type": 0, "id": 999, "success": 2},
            {"item_type": 0, "id": 12, "success": 1, "name": "No categories", "appid": 12}
        ]}});
        let apps = parse(&v).unwrap();
        assert_eq!(apps.len(), 4);
        assert_eq!(apps[0].has_cards, Some(true));
        assert_eq!(apps[0].name.as_deref(), Some("Team Fortress 2"));
        assert_eq!(apps[1].has_cards, Some(false));
        assert!(!apps[2].found);
        assert_eq!(apps[2].has_cards, None);
        assert_eq!(apps[3].has_cards, None);
        assert!(parse(&json!({"response": {}})).is_none());
    }
}
