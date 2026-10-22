//! Steam's item definitions for one game (`IQuestService/GetCommunityItemDefinitions`):
//! every community item the game has, including its cards with their framed
//! and foil artwork, emoticons, backgrounds and badges. A single Web API call
//! per game, which makes it the fast route for the catalog.

use serde_json::Value;

use super::{class, image_url, CatalogItem};
use crate::jsonutil::{flag, int, text};

/// Reads the answer. `None` means it wasn't an item definitions answer at all.
pub fn parse(v: &Value) -> Option<Vec<CatalogItem>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let Some(list) = resp["item_definitions"].as_array() else {
        // A game without community items answers with an empty response.
        return Some(Vec::new());
    };
    let mut items = Vec::new();
    for d in list {
        if flag(&d["deleted"]) == Some(true) {
            continue;
        }
        let (Some(class_id), Some(item_type)) = (int(&d["item_class"]), int(&d["item_type"]))
        else {
            continue;
        };
        let appid = int(&d["appid"]).unwrap_or(0).max(0) as u64;
        let image = |key: &str| text(&d[key]).map(|i| image_url(appid, &i));
        let small = image("item_image_small");
        let large = image("item_image_large");
        let composed = image("item_image_composed");
        let base = CatalogItem {
            class: class_id,
            item_type,
            foil: false,
            name: text(&d["item_name"]),
            title: text(&d["item_title"]),
            description: text(&d["item_description"]),
            image: composed.clone().or_else(|| small.clone()),
            image_large: large.clone(),
            animated: text(&d["item_movie_webm"]).is_some() || text(&d["item_movie_mp4"]).is_some(),
            series: int(&d["item_series"]),
            active: flag(&d["active"]),
        };
        if class_id == class::CARD {
            let foil = CatalogItem {
                foil: true,
                image: image("item_image_composed_foil")
                    .or_else(|| composed.clone())
                    .or_else(|| small.clone()),
                ..base.clone()
            };
            items.push(base);
            items.push(foil);
        } else {
            items.push(base);
        }
    }
    Some(items)
}

/// How many cards (not counting foils) a list holds.
pub fn card_count(items: &[CatalogItem]) -> usize {
    items
        .iter()
        .filter(|i| i.class == class::CARD && !i.foil)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_definitions() {
        let v = json!({"response": {"item_definitions": [
            {"item_type": 1, "appid": 440, "item_name": "pyro", "item_title": "Pyro",
             "item_image_small": "a.png", "item_image_large": "b.jpg",
             "item_image_composed": "c.png", "item_image_composed_foil": "d.png",
             "item_series": 1, "item_class": 2, "active": true},
            {"item_type": "2", "appid": 440, "item_name": "spy", "item_title": "Spy",
             "item_image_small": "e.png", "item_class": "2"},
            {"item_type": 10, "appid": 440, "item_name": "tf2pyro", "item_class": 4,
             "item_image_small": "f.png"},
            {"item_type": 11, "appid": 440, "item_title": "Mann Co.", "item_class": 3,
             "item_movie_webm": "g.webm"},
            {"item_type": 12, "appid": 440, "item_class": 3, "deleted": true},
            {"appid": 440, "item_title": "No type"}
        ]}});
        let items = parse(&v).unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(card_count(&items), 2);
        assert_eq!(items[0].image.as_deref(), Some("c.png"));
        assert!(items[1].foil);
        assert_eq!(items[1].image.as_deref(), Some("d.png"));
        assert_eq!(
            items[3].image.as_deref(),
            Some("e.png"),
            "foil falls back to the normal art"
        );
        assert_eq!(items[4].name.as_deref(), Some("tf2pyro"));
        assert!(items[5].animated);
    }

    #[test]
    fn empty_and_wrong_answers() {
        assert_eq!(parse(&json!({"response": {}})), Some(Vec::new()));
        assert_eq!(parse(&json!({"error": "x"})), None);
        assert_eq!(parse(&json!({"response": []})), None);
    }
}
