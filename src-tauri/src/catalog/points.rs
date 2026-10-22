//! The Points Shop catalog (`ILoyaltyRewardsService/QueryRewardItems`): every
//! item a game sells for Steam Points, with its price in points and when it's
//! available. It includes the emoticons and backgrounds from each game's
//! trading card set, plus items sold only in the Points Shop.

use serde_json::Value;

use super::CatalogItem;
use crate::jsonutil::{flag, int, text};

/// Items per page asked for. Steam pages the answer with a cursor.
pub const PAGE_SIZE: u32 = 100;
/// How many games one request asks about.
pub const APPS_PER_REQUEST: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct PointsItem {
    pub appid: u64,
    pub defid: i64,
    pub cost: Option<i64>,
    pub active: Option<bool>,
    /// Unix time the item became available; `None` means no limit.
    pub available_from: Option<i64>,
    /// Unix time the item stops being available; `None` means no limit.
    pub available_until: Option<i64>,
    pub item: CatalogItem,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PointsPage {
    pub items: Vec<PointsItem>,
    pub next_cursor: Option<String>,
    pub total: Option<i64>,
    /// Definitions on the page, including bundles and others that were skipped.
    pub definitions: usize,
}

/// Reads one page. `None` means it wasn't a Points Shop answer.
pub fn parse_page(v: &Value) -> Option<PointsPage> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let list: &[Value] = resp["definitions"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut items = Vec::new();
    for d in list {
        // Bundles group other items, and some definitions aren't items at all.
        let is_bundle = d["bundle_defids"].as_array().is_some_and(|b| !b.is_empty());
        let (Some(appid), Some(defid), Some(class_id), Some(item_type)) = (
            int(&d["appid"]).filter(|a| *a > 0),
            int(&d["defid"]),
            int(&d["community_item_class"]).filter(|c| *c > 0),
            int(&d["community_item_type"]),
        ) else {
            continue;
        };
        if is_bundle {
            continue;
        }
        let data = &d["community_item_data"];
        let animated = flag(&data["animated"]).unwrap_or(false)
            || text(&data["item_movie_webm"]).is_some()
            || text(&data["item_movie_mp4"]).is_some();
        items.push(PointsItem {
            appid: appid as u64,
            defid,
            cost: int(&d["point_cost"]),
            active: flag(&d["active"]),
            available_from: int(&d["timestamp_available"]).filter(|t| *t > 0),
            available_until: int(&d["timestamp_available_end"]).filter(|t| *t > 0),
            item: CatalogItem {
                class: class_id,
                item_type,
                foil: false,
                name: text(&data["item_name"]),
                title: text(&data["item_title"]),
                description: text(&data["item_description"]),
                image: text(&data["item_image_small"]),
                image_large: text(&data["item_image_large"]),
                animated,
                series: None,
                active: None,
            },
        });
    }
    Some(PointsPage {
        items,
        next_cursor: text(&resp["next_cursor"]),
        total: int(&resp["total_count"]),
        definitions: list.len(),
    })
}

/// Games that have a Points Shop page (`ILoyaltyRewardsService/GetEligibleApps`),
/// and which of them are Steam sale events rather than games.
pub fn parse_eligible_apps(v: &Value) -> Option<Vec<(u64, bool)>> {
    let list = v.get("response")?.get("apps")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|a| {
                let appid = int(&a["appid"]).filter(|x| *x > 0)? as u64;
                Some((appid, flag(&a["event_app"]).unwrap_or(false)))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_page() {
        let v = json!({"response": {"definitions": [
            {"appid": 440, "defid": 100, "type": 1, "community_item_class": 4, "community_item_type": 10,
             "point_cost": "100", "active": true, "timestamp_available": 0, "timestamp_available_end": 0,
             "community_item_data": {"item_name": "tf2pyro", "item_title": ":tf2pyro:", "item_image_small": "s.png"}},
            {"appid": 440, "defid": 101, "community_item_class": 15, "community_item_type": 30,
             "point_cost": "3000", "active": false, "timestamp_available": 1700000000, "timestamp_available_end": 1700600000,
             "community_item_data": {"item_title": "Hat", "animated": true}},
            {"appid": 440, "defid": 102, "community_item_class": 0, "bundle_defids": [100, 101]},
            {"appid": 440, "defid": 103, "community_item_class": 3, "community_item_type": 5, "bundle_defids": [1]}
        ], "total_count": 4, "count": 4, "next_cursor": "abc"}});
        let page = parse_page(&v).unwrap();
        assert_eq!(page.definitions, 4);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_cursor.as_deref(), Some("abc"));
        assert_eq!(page.total, Some(4));
        let first = &page.items[0];
        assert_eq!(
            (first.appid, first.defid, first.cost),
            (440, 100, Some(100))
        );
        assert_eq!(first.available_from, None);
        assert_eq!(first.item.name.as_deref(), Some("tf2pyro"));
        let second = &page.items[1];
        assert!(second.item.animated);
        assert_eq!(second.active, Some(false));
        assert_eq!(second.available_until, Some(1700600000));
    }

    #[test]
    fn last_page_and_wrong_answers() {
        let page = parse_page(&json!({"response": {"definitions": []}})).unwrap();
        assert!(page.items.is_empty() && page.next_cursor.is_none());
        assert!(parse_page(&json!({})).is_none());
    }

    #[test]
    fn eligible_apps() {
        let v = json!({"response": {"apps": [
            {"appid": 440, "has_items_anyone_can_purchase": true},
            {"appid": 2861690, "event_app": true},
            {"appid": 0}
        ]}});
        assert_eq!(
            parse_eligible_apps(&v).unwrap(),
            vec![(440, false), (2861690, true)]
        );
    }
}
