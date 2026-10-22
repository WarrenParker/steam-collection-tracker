//! The item catalog: what exists for each game, read from Steam's sources and
//! turned into one shape before it's saved.
//!
//! Every Steam community item is identified by its game (`appid`), its class
//! (card, background, emoticon…) and its type number within that game and
//! class. Cards also come in a foil version, which shares the type number.

pub mod cardpage;
pub mod library;
pub mod points;
pub mod store;

/// Steam's community item classes (`ECommunityItemClass`). The inventory uses
/// the same numbers in its `item_class_N` tags.
pub mod class {
    pub const BADGE: i64 = 1;
    pub const CARD: i64 = 2;
    pub const BACKGROUND: i64 = 3;
    pub const EMOTICON: i64 = 4;
    pub const BOOSTER: i64 = 5;
    pub const GEMS: i64 = 7;
    pub const MINI_BACKGROUND: i64 = 13;
    pub const AVATAR_FRAME: i64 = 14;
    pub const ANIMATED_AVATAR: i64 = 15;
}

/// Plain-language name for a kind of item.
pub fn kind_label(class_id: i64, foil: bool, animated: bool) -> &'static str {
    match class_id {
        class::BADGE => "Badges",
        class::CARD if foil => "Foil cards",
        class::CARD => "Cards",
        class::BACKGROUND if animated => "Animated backgrounds",
        class::BACKGROUND => "Backgrounds",
        class::EMOTICON => "Emoticons",
        class::BOOSTER => "Booster packs",
        6 => "Consumables",
        class::GEMS => "Gems",
        8 => "Profile modifiers",
        9 => "Scenes",
        10 => "Saliens items",
        11 => "Stickers",
        12 => "Chat effects",
        class::MINI_BACKGROUND => "Mini profile backgrounds",
        class::AVATAR_FRAME => "Avatar frames",
        class::ANIMATED_AVATAR => "Animated avatars",
        16 => "Steam Deck keyboards",
        17 => "Steam Deck startup movies",
        _ => "Other items",
    }
}

/// One item in a game's catalog, from whichever source described it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CatalogItem {
    pub class: i64,
    pub item_type: i64,
    pub foil: bool,
    /// Steam's short name (an emoticon's code, for example).
    pub name: Option<String>,
    /// The name people see.
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub image_large: Option<String>,
    pub animated: bool,
    pub series: Option<i64>,
    pub active: Option<bool>,
}

const ITEM_IMAGES: &str = "https://cdn.akamai.steamstatic.com/steamcommunity/public/images/items";

/// Steam's item sources give images either as full links or as file names
/// inside the game's item image folder.
pub fn image_url(appid: u64, image: &str) -> String {
    let image = image.trim();
    if image.starts_with("https://") || image.starts_with("http://") {
        image.to_string()
    } else {
        format!("{ITEM_IMAGES}/{appid}/{}", image.trim_start_matches('/'))
    }
}

/// Turns the few HTML entities Steam uses in names back into characters.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(end) = after.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(kind_label(class::CARD, false, false), "Cards");
        assert_eq!(kind_label(class::CARD, true, false), "Foil cards");
        assert_eq!(
            kind_label(class::BACKGROUND, false, true),
            "Animated backgrounds"
        );
        assert_eq!(kind_label(99, false, false), "Other items");
    }

    #[test]
    fn images() {
        assert_eq!(
            image_url(440, "abc.png"),
            "https://cdn.akamai.steamstatic.com/steamcommunity/public/images/items/440/abc.png"
        );
        assert_eq!(
            image_url(440, "https://x.test/a.png"),
            "https://x.test/a.png"
        );
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(
            decode_entities("&quot;Hi&quot; &#39;x&#39; &#x41;"),
            "\"Hi\" 'x' A"
        );
        assert_eq!(decode_entities("a & b"), "a & b");
        assert_eq!(decode_entities("&bogus; ok"), "&bogus; ok");
    }
}
