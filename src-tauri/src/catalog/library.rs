//! What the signed-in user has beyond the inventory: their Steam Family's
//! shared library, every emoticon they can use, and their profile items
//! (backgrounds, frames, avatars and so on, including Points Shop purchases).

use serde_json::Value;

use super::class;
use crate::jsonutil::{int, text};

#[derive(Debug, Clone, PartialEq)]
pub struct FamilyApp {
    pub appid: u64,
    pub name: Option<String>,
}

/// The user's family group from `IFamilyGroupsService/GetFamilyGroupForUser`.
/// `Some(None)` means they aren't in a Steam Family; `None` that the answer
/// couldn't be read.
pub fn parse_family_group(v: &Value) -> Option<Option<String>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let group = text(&resp["family_groupid"])
        .or_else(|| int(&resp["family_groupid"]).map(|n| n.to_string()))
        .filter(|g| g != "0");
    Some(group)
}

/// Games shared with the user (`IFamilyGroupsService/GetSharedLibraryApps`).
/// Games the family has excluded from sharing are left out.
pub fn parse_family_library(v: &Value) -> Option<Vec<FamilyApp>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let Some(list) = resp["apps"].as_array() else {
        return Some(Vec::new());
    };
    Some(
        list.iter()
            .filter(|a| int(&a["exclude_reason"]).unwrap_or(0) == 0)
            .filter_map(|a| {
                Some(FamilyApp {
                    appid: int(&a["appid"]).filter(|x| *x > 0)? as u64,
                    name: text(&a["name"]),
                })
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct OwnedProfileItem {
    pub appid: u64,
    pub class: i64,
    pub item_type: Option<i64>,
    pub name: String,
    pub title: Option<String>,
    pub quantity: i64,
}

/// Emoticons the user can use (`IPlayerService/GetEmoticonList`).
pub fn parse_emoticons(v: &Value) -> Option<Vec<OwnedProfileItem>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let Some(list) = resp["emoticons"].as_array() else {
        return Some(Vec::new());
    };
    Some(
        list.iter()
            .filter_map(|e| {
                Some(OwnedProfileItem {
                    appid: int(&e["appid"]).unwrap_or(0).max(0) as u64,
                    class: class::EMOTICON,
                    item_type: None,
                    name: text(&e["name"])?,
                    title: None,
                    quantity: int(&e["count"]).unwrap_or(1).max(1),
                })
            })
            .collect(),
    )
}

/// The groups in a `IPlayerService/GetProfileItemsOwned` answer, with the item
/// class each holds when the item doesn't say.
const PROFILE_GROUPS: &[(&str, i64)] = &[
    ("profile_backgrounds", class::BACKGROUND),
    ("mini_profile_backgrounds", class::MINI_BACKGROUND),
    ("avatar_frames", class::AVATAR_FRAME),
    ("animated_avatars", class::ANIMATED_AVATAR),
    ("profile_modifiers", 8),
    ("steam_deck_keyboard_skins", 16),
    ("steam_deck_startup_movies", 17),
];

pub fn parse_profile_items(v: &Value) -> Option<Vec<OwnedProfileItem>> {
    let resp = v.get("response")?;
    if !resp.is_object() {
        return None;
    }
    let mut out = Vec::new();
    for (group, default_class) in PROFILE_GROUPS {
        let Some(list) = resp[*group].as_array() else {
            continue;
        };
        for item in list {
            let name = text(&item["name"])
                .or_else(|| text(&item["item_title"]))
                .or_else(|| int(&item["communityitemid"]).map(|n| n.to_string()))
                .or_else(|| text(&item["communityitemid"]));
            let Some(name) = name else {
                continue;
            };
            out.push(OwnedProfileItem {
                appid: int(&item["appid"]).unwrap_or(0).max(0) as u64,
                class: int(&item["item_class"])
                    .filter(|c| *c > 0)
                    .unwrap_or(*default_class),
                item_type: int(&item["item_type"]),
                name,
                title: text(&item["item_title"]),
                quantity: 1,
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn family() {
        assert_eq!(
            parse_family_group(&json!({"response": {"family_groupid": "123"}})),
            Some(Some("123".to_string()))
        );
        assert_eq!(
            parse_family_group(&json!({"response": {"is_not_member_of_any_group": true}})),
            Some(None)
        );
        assert_eq!(parse_family_group(&json!({})), None);
        let apps = parse_family_library(&json!({"response": {"apps": [
            {"appid": 10, "name": "Counter-Strike", "owner_steamids": ["1"]},
            {"appid": 20, "name": "Excluded", "exclude_reason": 2},
            {"appid": 30}
        ]}}))
        .unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name.as_deref(), Some("Counter-Strike"));
        assert_eq!(apps[1].name, None);
    }

    #[test]
    fn emoticons_and_profile_items() {
        let e = parse_emoticons(&json!({"response": {"emoticons": [
            {"name": ":steamhappy:", "count": 3, "appid": 753},
            {"name": ":tf2pyro:", "appid": 440},
            {"count": 1}
        ]}}))
        .unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].quantity, 3);
        assert_eq!(e[1].appid, 440);

        let p = parse_profile_items(&json!({"response": {
            "profile_backgrounds": [{"communityitemid": "55", "appid": 440, "item_type": 11, "item_class": 3, "name": "mannco", "item_title": "Mann Co."}],
            "avatar_frames": [{"communityitemid": "56", "appid": 2861690, "item_type": 4, "item_title": "Frame"}]
        }}))
        .unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "mannco");
        assert_eq!(p[1].class, class::AVATAR_FRAME);
        assert_eq!(p[1].name, "Frame");
    }
}
