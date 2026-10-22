//! A game's card page (`steamcommunity.com/profiles/{id}/gamecards/{appid}`),
//! the slower fallback for the card catalog. It lists every card in the set
//! with its artwork and how many you have. Community pages are rate limited
//! per connection, so the scanner only reads these when Steam's item
//! definitions aren't available.

use std::sync::LazyLock;

use regex::Regex;

use super::{class, decode_entities, CatalogItem};

#[derive(Debug, Clone, PartialEq)]
pub struct PageCard {
    /// The card's place in the set, from "3 of 8, Series 1".
    pub position: i64,
    pub name: String,
    pub image: Option<String>,
    pub owned: i64,
    pub series: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CardPage {
    pub cards: Vec<PageCard>,
    /// The set size the page states, if it states one.
    pub set_size: Option<i64>,
}

static CARD_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="badge_card_set_card[ "]"#).unwrap());
static IMG_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<img\b[^>]*>").unwrap());
static SRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bsrc="([^"]+)""#).unwrap());
static QTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div class="badge_card_set_text_qty">\s*\((\d+)\)\s*</div>"#).unwrap()
});
static NAME_BOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="badge_card_set_text(?:\s[^"]*)?"\s*>"#).unwrap());
static POSITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s+of\s+(\d+)(?:,\s*Series\s+(\d+))?").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Reads every card on the page. A game without cards gives an empty list.
pub fn parse(html: &str) -> CardPage {
    let starts: Vec<usize> = CARD_START.find_iter(html).map(|m| m.start()).collect();
    let mut page = CardPage::default();
    for (i, &start) in starts.iter().enumerate() {
        let mut end = starts
            .get(i + 1)
            .copied()
            .unwrap_or_else(|| (start + 4000).min(html.len()));
        while !html.is_char_boundary(end) {
            end -= 1;
        }
        let Some(block) = html.get(start..end) else {
            continue;
        };

        let image = IMG_TAG
            .find_iter(block)
            .map(|m| m.as_str())
            .find(|tag| tag.contains("gamecard"))
            .and_then(|tag| SRC.captures(tag))
            .map(|c| decode_entities(&c[1]));

        let owned = QTY
            .captures(block)
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(0);

        let name = NAME_BOX.find(block).map(|m| {
            let rest = QTY.replace(&block[m.end()..], "");
            let text = rest.split("</div>").next().unwrap_or_default();
            let text = TAGS.replace_all(text, " ");
            SPACES
                .replace_all(&decode_entities(&text), " ")
                .trim()
                .to_string()
        });
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            continue;
        };

        let position = POSITION.captures(block);
        if let Some(total) = position.as_ref().and_then(|c| c[2].parse::<i64>().ok()) {
            page.set_size = Some(total);
        }
        page.cards.push(PageCard {
            position: position
                .as_ref()
                .and_then(|c| c[1].parse().ok())
                .unwrap_or(i as i64 + 1),
            name,
            image,
            owned,
            series: position
                .as_ref()
                .and_then(|c| c.get(3))
                .and_then(|m| m.as_str().parse().ok()),
        });
    }
    page
}

/// The page's cards as catalog items, foil versions included. The card's
/// place in the set stands in for its type number.
pub fn to_items(page: &CardPage) -> Vec<CatalogItem> {
    page.cards
        .iter()
        .flat_map(|c| {
            let normal = CatalogItem {
                class: class::CARD,
                item_type: c.position,
                foil: false,
                name: None,
                title: Some(c.name.clone()),
                description: None,
                image: c.image.clone(),
                image_large: None,
                animated: false,
                series: c.series,
                active: None,
            };
            let foil = CatalogItem {
                foil: true,
                image: None,
                ..normal.clone()
            };
            [normal, foil]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
<div class="badge_card_set_cards">
  <div class="badge_card_set_card owned">
    <div class="game_card_ctn with_zoom" onclick="">
      <img class="gamecard " src="https://community.akamai.steamstatic.com/economy/image/AAA/96fx96f">
    </div>
    <div class="badge_card_set_text ellipsis">
      <div class="badge_card_set_text_qty">(2)</div>
      Tom &amp; Jerry
    </div>
    <div style="clear: right"></div>
    <div class="badge_card_set_text game_card_unowned_border">1 of 3, Series 1</div>
  </div>
  <div class="badge_card_set_card unowned">
    <div class="game_card_ctn with_zoom"><img src="https://x.test/b.png" class="gamecard"></div>
    <div class="badge_card_set_text ellipsis">
      The <b>Spy</b>
    </div>
    <div class="badge_card_set_text">2 of 3, Series 1</div>
  </div>
  <div class="badge_card_set_card unowned">
    <div class="badge_card_set_text ellipsis">Engineer</div>
  </div>
</div>"#;

    #[test]
    fn reads_cards() {
        let page = parse(PAGE);
        assert_eq!(page.cards.len(), 3);
        assert_eq!(page.set_size, Some(3));
        let first = &page.cards[0];
        assert_eq!(first.name, "Tom & Jerry");
        assert_eq!(first.owned, 2);
        assert_eq!(first.position, 1);
        assert_eq!(first.series, Some(1));
        assert!(first.image.as_deref().unwrap().contains("/AAA/"));
        let second = &page.cards[1];
        assert_eq!(second.name, "The Spy");
        assert_eq!(second.owned, 0);
        assert_eq!(second.image.as_deref(), Some("https://x.test/b.png"));
        let third = &page.cards[2];
        assert_eq!(third.position, 3, "falls back to the card's order");
        assert_eq!(third.image, None);
        let items = to_items(&page);
        assert_eq!(items.len(), 6);
        assert!(items[1].foil && items[1].image.is_none());
    }

    #[test]
    fn page_without_cards() {
        assert!(parse("<html><body>No cards here</body></html>")
            .cards
            .is_empty());
    }
}
