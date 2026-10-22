//! An item's order book: who's selling at what price, and the best offer to buy.
//!
//! A card needs up to five copies (six when keeping a set), and the cheapest
//! listing rarely has that many, so the cost of several copies walks up the
//! sell side: the cheapest copies first, then the next price, and so on.

/// Price levels kept per item, counted in copies. Enough for any card.
pub const KEEP_COPIES: i64 = 12;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    /// Sell side from cheapest up: (price in mills, copies at that price).
    pub sell: Vec<(i64, i64)>,
    /// The best offer to buy, in mills.
    pub buy_max: Option<i64>,
    pub sell_count: i64,
    pub buy_count: i64,
    /// Steam's currency number.
    pub currency: i64,
}

/// Reads Steam's compact lists, which alternate price (in cents) and copies.
pub fn levels_from_compact(compact: &[i64]) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = compact
        .chunks_exact(2)
        .filter(|pair| pair[0] > 0 && pair[1] > 0)
        .map(|pair| (pair[0] * 10, pair[1]))
        .collect();
    out.sort_by_key(|l| l.0);
    out
}

/// The cheapest levels, enough for `KEEP_COPIES` copies, as "price:copies"
/// pairs separated by spaces.
pub fn to_text(levels: &[(i64, i64)]) -> String {
    let mut out = Vec::new();
    let mut copies = 0i64;
    for (price, qty) in levels {
        out.push(format!("{price}:{qty}"));
        copies += qty;
        if copies >= KEEP_COPIES {
            break;
        }
    }
    out.join(" ")
}

pub fn from_text(text: &str) -> Vec<(i64, i64)> {
    text.split_whitespace()
        .filter_map(|pair| {
            let (p, q) = pair.split_once(':')?;
            Some((p.parse().ok()?, q.parse().ok()?))
        })
        .filter(|(p, q): &(i64, i64)| *p > 0 && *q > 0)
        .collect()
}

/// What `copies` copies cost, buying the cheapest first. The flag is false
/// when the book ran out and the rest were priced at its dearest level.
pub fn cost(levels: &[(i64, i64)], copies: i64) -> Option<(i64, bool)> {
    if copies <= 0 {
        return Some((0, true));
    }
    let mut left = copies;
    let mut total = 0i64;
    let mut last = None;
    for (price, qty) in levels {
        let take = left.min(*qty);
        total += take * price;
        left -= take;
        last = Some(*price);
        if left == 0 {
            return Some((total, true));
        }
    }
    last.map(|p| (total + left * p, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steams_compact_lists() {
        // From a real answer: 11 cents x1, 12 cents x12, 13 cents x9, ...
        let sell = [11, 1, 12, 12, 13, 9, 14, 18, 37, 2];
        let levels = levels_from_compact(&sell);
        assert_eq!(levels[0], (110, 1));
        assert_eq!(levels.len(), 5);
        assert_eq!(levels_from_compact(&[5, 1, 9]), vec![(50, 1)]);
        let text = to_text(&levels);
        assert_eq!(text, "110:1 120:12");
        assert_eq!(from_text(&text), vec![(110, 1), (120, 12)]);
        assert_eq!(from_text("bad 3:x 40:2"), vec![(40, 2)]);
    }

    #[test]
    fn several_copies_walk_up_the_book() {
        let levels = vec![(110, 1), (120, 2), (130, 9)];
        assert_eq!(cost(&levels, 1), Some((110, true)));
        assert_eq!(cost(&levels, 5), Some((110 + 240 + 260, true)));
        assert_eq!(cost(&levels, 0), Some((0, true)));
        assert_eq!(cost(&levels, 13), Some((110 + 240 + 1170 + 130, false)));
        assert_eq!(cost(&[], 2), None);
    }
}
