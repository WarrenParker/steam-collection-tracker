//! The Steam Community Market: current prices, price history and order books,
//! and the currencies Steam answers in.
//!
//! Three routes are used, all paced like every other Market request:
//!
//! - **Search** (`/market/search/render/?norender=1`) lists a game's items ten
//!   at a time with the cheapest listing and how many are listed. One request
//!   covers ten items, so it's how current prices are read.
//! - **Price history** (`/market/actions?q=QueryPriceHistory`) gives an item's
//!   whole sale history. It works without sign-in. The older
//!   `/market/pricehistory/` is used if it doesn't answer, when signed in.
//! - **Order book** (`/market/orderbook?q=Load`) lists every price level on
//!   both sides in a few hundred bytes, so the cost of several copies is known.
//!
//! Money is kept in thousandths of the currency's main unit ("mills").

pub mod book;
pub mod series;

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::class;
use crate::jsonutil;

/// Steam answers 10 search results per request, whatever `count` asks for.
pub const SEARCH_PAGE: u64 = 10;

/// Where the exchange rates are kept in the database's `meta` table.
pub const META_RATES: &str = "exchange_rates";
/// The currency Steam answers in for this account (Steam's number).
pub const META_CURRENCY: &str = "market_currency";

/// A Steam item image from its image id.
pub fn economy_image(icon: &str) -> String {
    format!("https://community.akamai.steamstatic.com/economy/image/{icon}/96fx96f")
}

/// Steam's currency numbers (`ECurrencyCode`) and their ISO codes.
pub const CURRENCIES: &[(i64, &str)] = &[
    (1, "USD"),
    (2, "GBP"),
    (3, "EUR"),
    (4, "CHF"),
    (5, "RUB"),
    (6, "PLN"),
    (7, "BRL"),
    (8, "JPY"),
    (9, "NOK"),
    (10, "IDR"),
    (11, "MYR"),
    (12, "PHP"),
    (13, "SGD"),
    (14, "THB"),
    (15, "VND"),
    (16, "KRW"),
    (17, "TRY"),
    (18, "UAH"),
    (19, "MXN"),
    (20, "CAD"),
    (21, "AUD"),
    (22, "NZD"),
    (23, "CNY"),
    (24, "INR"),
    (25, "CLP"),
    (26, "PEN"),
    (27, "COP"),
    (28, "ZAR"),
    (29, "HKD"),
    (30, "TWD"),
    (31, "SAR"),
    (32, "AED"),
    (33, "SEK"),
    (34, "ARS"),
    (35, "ILS"),
    (36, "BYN"),
    (37, "KZT"),
    (38, "KWD"),
    (39, "QAR"),
    (40, "CRC"),
    (41, "UYU"),
    (42, "BGN"),
    (43, "HRK"),
    (44, "CZK"),
    (45, "DKK"),
    (46, "HUF"),
    (47, "RON"),
];

pub fn currency_code(id: i64) -> Option<&'static str> {
    CURRENCIES.iter().find(|(n, _)| *n == id).map(|(_, c)| *c)
}

pub fn currency_id(code: &str) -> Option<i64> {
    CURRENCIES
        .iter()
        .find(|(_, c)| c.eq_ignore_ascii_case(code))
        .map(|(n, _)| *n)
}

/// One item from a Market search.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    /// Steam's name for the item on the Market, like "308040-:b2b:".
    pub hash_name: String,
    pub appid: u64,
    /// The item's class, or 0 for kinds the catalog doesn't track (gems).
    pub class: i64,
    pub foil: bool,
    /// The name without the game's number.
    pub name: String,
    pub type_label: String,
    /// The cheapest listing, what a buyer pays, in mills.
    pub lowest: Option<i64>,
    pub listings: i64,
    /// Steam's image id, for items the catalog doesn't picture.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    pub total: u64,
    pub listings: Vec<Listing>,
}

/// The kind of item from the Market's type line, like "Back to Bed Rare
/// Emoticon" or "Kingdom Rush Foil Trading Card".
pub fn class_of(type_label: &str) -> (i64, bool) {
    let t = type_label.trim();
    if t.ends_with("Foil Trading Card") {
        (class::CARD, true)
    } else if t.ends_with("Trading Card") {
        (class::CARD, false)
    } else if t.ends_with("Emoticon") {
        (class::EMOTICON, false)
    } else if t.ends_with("Profile Background") {
        (class::BACKGROUND, false)
    } else if t.ends_with("Booster Pack") {
        (class::BOOSTER, false)
    } else {
        (0, false)
    }
}

/// The game number at the front of a Market name ("308040-:b2b:" is 308040).
pub fn app_of(hash_name: &str) -> Option<u64> {
    let (app, rest) = hash_name.split_once('-')?;
    (!rest.is_empty()).then(|| app.parse().ok()).flatten()
}

/// A Market name as the catalog would write it: the suffixes Steam adds to
/// tell items with the same name apart are dropped.
pub fn plain_name(name: &str) -> &str {
    let mut n = name.trim();
    for suffix in [
        " (Profile Background)",
        " (Foil Trading Card)",
        " (Trading Card)",
        " (Foil)",
        " (Emoticon)",
    ] {
        if let Some(s) = n.strip_suffix(suffix) {
            n = s.trim_end();
        }
    }
    n
}

pub fn parse_search(v: &Value) -> Option<SearchPage> {
    if v["success"].as_bool() != Some(true) {
        return None;
    }
    let results = v["results"].as_array()?;
    let listings = results
        .iter()
        .filter_map(|r| {
            let hash_name = r["hash_name"].as_str()?.to_string();
            let appid = app_of(&hash_name)?;
            let desc = &r["asset_description"];
            let type_label = desc["type"].as_str().unwrap_or_default().to_string();
            let (class_id, foil) = class_of(&type_label);
            let prefix = format!("{appid}-");
            let name = desc["market_name"]
                .as_str()
                .or_else(|| r["name"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| hash_name.trim_start_matches(&prefix).to_string());
            let listings = jsonutil::int(&r["sell_listings"]).unwrap_or(0);
            let lowest = jsonutil::int(&r["sell_price"])
                .filter(|p| *p > 0 && listings > 0)
                .map(|cents| cents * 10);
            Some(Listing {
                hash_name,
                appid,
                class: class_id,
                foil,
                name,
                type_label,
                lowest,
                listings,
                icon: jsonutil::text(&desc["icon_url"]).filter(|i| i.len() < 400),
            })
        })
        .collect();
    Some(SearchPage {
        total: v["total_count"].as_u64().unwrap_or(0),
        listings,
    })
}

/// Sale points from either price history: (Unix time, median in mills, sold),
/// with Steam's currency number when the answer says (0 when it doesn't).
pub fn parse_history(v: &Value) -> Option<(i64, Vec<(i64, i64, i64)>)> {
    let to_mills = |x: f64| (x * 1000.0).round() as i64;
    // The newer route: {"data": {"ecurrency": 1, "prices": [{time, price_median, purchases}]}}
    if let Some(data) = v.get("data").filter(|d| d.get("prices").is_some()) {
        let currency = jsonutil::int(&data["ecurrency"]).unwrap_or(0);
        let points = data["prices"]
            .as_array()?
            .iter()
            .filter_map(|p| {
                Some((
                    jsonutil::int(&p["time"])?,
                    to_mills(p["price_median"].as_f64()?),
                    jsonutil::int(&p["purchases"]).unwrap_or(0),
                ))
            })
            .collect();
        return Some((currency, points));
    }
    // The older one: {"success": true, "prices": [["May 16 2013 01: +0", 5.127, "3"]]}
    if v["success"].as_bool() == Some(true) {
        let points = match v["prices"].as_array() {
            Some(list) => list
                .iter()
                .filter_map(|p| {
                    Some((
                        series::parse_old_date(p[0].as_str()?)?,
                        to_mills(p[1].as_f64()?),
                        jsonutil::int(&p[2]).unwrap_or(0),
                    ))
                })
                .collect(),
            // An item that has never sold answers with `"prices": false`.
            None => Vec::new(),
        };
        return Some((0, points));
    }
    None
}

/// The order book answer: {"data": {"success": true, "data": {...}}}.
pub fn parse_book(v: &Value) -> Option<book::Book> {
    let outer = v.get("data").unwrap_or(v);
    if outer["success"].as_bool() == Some(false) {
        return None;
    }
    let d = outer.get("data").unwrap_or(outer);
    d.get("rgCompactSellOrders")?;
    let list = |key: &str| -> Vec<i64> {
        d[key]
            .as_array()
            .map(|a| a.iter().filter_map(jsonutil::int).collect())
            .unwrap_or_default()
    };
    Some(book::Book {
        sell: book::levels_from_compact(&list("rgCompactSellOrders")),
        buy_max: jsonutil::int(&d["amtMaxBuyOrder"])
            .filter(|p| *p > 0)
            .map(|c| c * 10),
        sell_count: jsonutil::int(&d["cSellOrders"]).unwrap_or(0),
        buy_count: jsonutil::int(&d["cBuyOrders"]).unwrap_or(0),
        currency: jsonutil::int(&d["eCurrency"]).unwrap_or(0),
    })
}

/// Exchange rates from api.frankfurter.dev: units of each currency per one of
/// the base currency.
pub fn parse_rates(v: &Value) -> Option<HashMap<String, f64>> {
    let list = v.as_array()?;
    let rates: HashMap<String, f64> = list
        .iter()
        .filter_map(|r| {
            let rate = r["rate"].as_f64().filter(|x| *x > 0.0)?;
            Some((r["quote"].as_str()?.to_ascii_uppercase(), rate))
        })
        .collect();
    (!rates.is_empty()).then_some(rates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_results() {
        let v = json!({"success": true, "start": 0, "pagesize": 10, "total_count": 23, "results": [
            {"name": ":b2b:", "hash_name": "308040-:b2b:", "sell_listings": 68, "sell_price": 3,
             "sell_price_text": "$0.03", "asset_description": {"appid": 753, "type": "Back to Bed Emoticon", "market_name": ":b2b:"}},
            {"name": "Out the window (Foil)", "hash_name": "308040-Out the window (Foil)", "sell_listings": 2, "sell_price": 45,
             "asset_description": {"type": "Back to Bed Foil Trading Card", "market_name": "Out the window (Foil)"}},
            {"name": "Team (Profile Background)", "hash_name": "399670-Team (Profile Background)", "sell_listings": 0, "sell_price": 0,
             "asset_description": {"type": "Game Corp DX Rare Profile Background"}},
            {"name": "bad", "hash_name": "nope"}
        ]});
        let page = parse_search(&v).unwrap();
        assert_eq!(page.total, 23);
        assert_eq!(page.listings.len(), 3);
        let b2b = &page.listings[0];
        assert_eq!(
            (b2b.appid, b2b.class, b2b.foil, b2b.lowest),
            (308040, 4, false, Some(30))
        );
        let foil = &page.listings[1];
        assert_eq!((foil.class, foil.foil, foil.lowest), (2, true, Some(450)));
        assert_eq!(plain_name(&foil.name), "Out the window");
        let bg = &page.listings[2];
        assert_eq!((bg.class, bg.lowest, bg.listings), (3, None, 0));
        assert_eq!(plain_name(&bg.name), "Team");
        assert!(parse_search(&json!({"success": false})).is_none());
        assert_eq!(class_of("Steam Gems"), (0, false));
        assert_eq!(class_of("Back to Bed Booster Pack"), (5, false));
    }

    #[test]
    fn both_price_histories() {
        let new = json!({"data": {"ecurrency": 1, "prices": [
            {"price_median": 0.06707478, "purchases": 3, "time": 1789776000},
            {"price_median": 1.8630980253, "purchases": 1, "time": 1406678400}
        ]}});
        let (cur, pts) = parse_history(&new).unwrap();
        assert_eq!(cur, 1);
        assert_eq!(pts[0], (1789776000, 67, 3));
        assert_eq!(pts[1].1, 1863);
        let old = json!({"success": true, "price_prefix": "$", "prices": [["May 16 2013 01: +0", 5.127, "3"]]});
        let (cur, pts) = parse_history(&old).unwrap();
        assert_eq!((cur, pts[0].1, pts[0].2), (0, 5127, 3));
        let never = json!({"success": true, "prices": false});
        assert_eq!(parse_history(&never).unwrap().1, vec![]);
        assert!(parse_history(&json!({"success": false})).is_none());
    }

    #[test]
    fn order_book() {
        let v = json!({"data": {"success": true, "data": {"amtMaxBuyOrder": 7, "amtMinSellOrder": 11,
            "eCurrency": 1, "cBuyOrders": 177, "cSellOrders": 56,
            "rgCompactBuyOrders": [7, 45, 5, 9], "rgCompactSellOrders": [11, 1, 12, 12, 13, 9]}}});
        let b = parse_book(&v).unwrap();
        assert_eq!(b.sell, vec![(110, 1), (120, 12), (130, 9)]);
        assert_eq!(
            (b.buy_max, b.sell_count, b.buy_count, b.currency),
            (Some(70), 56, 177, 1)
        );
        assert!(parse_book(&json!({"data": {"success": false}})).is_none());
    }

    #[test]
    fn currencies_and_rates() {
        assert_eq!(currency_code(1), Some("USD"));
        assert_eq!(currency_id("eur"), Some(3));
        assert_eq!(currency_code(99), None);
        let v = json!([{"date": "2026-10-21", "base": "USD", "quote": "EUR", "rate": 0.89},
                       {"quote": "XXX", "rate": 0}]);
        let rates = parse_rates(&v).unwrap();
        assert_eq!(rates.get("EUR"), Some(&0.89));
        assert!(!rates.contains_key("XXX"));
        assert_eq!(app_of("753-Sack of Gems"), Some(753));
        assert_eq!(app_of("abc"), None);
    }
}
