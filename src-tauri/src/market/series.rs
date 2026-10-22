//! Price history: one point per day, the figures worked out from it, and a
//! compact form for storing it.
//!
//! Steam sends an item's whole sale history every time: hourly medians for the
//! last month and daily ones before that, back to the first sale. The app
//! turns that into one point per day, works out the averages and the lowest
//! price, and keeps only a recent stretch (daily for 90 days, then weekly for
//! the rest of the year). The full history is fetched again only when someone
//! asks to see it.
//!
//! Prices are in thousandths of the currency's main unit ("mills"), so a
//! median of $0.0735 is 74 and a listing of 7 cents is 70.

/// Seconds in a day.
pub const DAY: i64 = 86_400;
/// Days kept one point per day in the stored history.
pub const DAILY_DAYS: i64 = 90;
/// Days kept in the stored history altogether.
pub const KEPT_DAYS: i64 = 365;

/// One day (or week, or month) of sales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    /// Days since 1 January 1970 (UTC). For a grouped point, its first day.
    pub day: i64,
    /// Median sale price, in mills.
    pub price: i64,
    /// Items sold.
    pub volume: i64,
}

/// Figures from a price history, worked out when it's read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub avg_30: Option<i64>,
    pub avg_90: Option<i64>,
    pub avg_365: Option<i64>,
    pub avg_all: Option<i64>,
    /// Lowest and highest daily median, and the day each happened.
    pub low: Option<(i64, i64)>,
    pub high: Option<(i64, i64)>,
    /// Items sold in the last 30 days.
    pub sales_30: i64,
    pub first_day: Option<i64>,
    pub last_day: Option<i64>,
}

/// Turns sale points (Unix time, price in mills, items sold) into one point
/// per day. A day with several points gets the average of their prices,
/// weighted by how many sold at each, and the total sold.
pub fn daily(raw: &[(i64, i64, i64)]) -> Vec<Point> {
    let mut sorted: Vec<(i64, i64, i64)> = raw
        .iter()
        .copied()
        .filter(|(_, price, volume)| *price >= 0 && *volume >= 0)
        .collect();
    sorted.sort_by_key(|p| p.0);
    let mut out: Vec<Point> = Vec::new();
    let mut acc: Option<(i64, Acc)> = None;
    for (time, price, volume) in sorted {
        let day = time.div_euclid(DAY);
        match &mut acc {
            Some((d, a)) if *d == day => a.add(price, volume),
            _ => {
                if let Some((d, a)) = acc.take() {
                    out.push(a.point(d));
                }
                let mut a = Acc::default();
                a.add(price, volume);
                acc = Some((day, a));
            }
        }
    }
    if let Some((d, a)) = acc {
        out.push(a.point(d));
    }
    out
}

/// Adds up prices weighted by volume. Points with no recorded volume still
/// count, with a weight of one, so a price is never lost.
#[derive(Default)]
struct Acc {
    weighted: i128,
    weight: i128,
    volume: i64,
}

impl Acc {
    fn add(&mut self, price: i64, volume: i64) {
        let w = volume.max(1) as i128;
        self.weighted += price as i128 * w;
        self.weight += w;
        self.volume += volume;
    }

    fn point(&self, day: i64) -> Point {
        Point {
            day,
            price: div_round(self.weighted, self.weight),
            volume: self.volume,
        }
    }
}

fn div_round(n: i128, d: i128) -> i64 {
    if d == 0 {
        return 0;
    }
    ((n + d / 2) / d) as i64
}

/// Averages over the last 30, 90 and 365 days and all time, weighted by
/// items sold, plus the lowest and highest days. `today` is days since 1970.
pub fn summarize(points: &[Point], today: i64) -> Summary {
    let avg = |since: i64| -> Option<i64> {
        let mut acc = Acc::default();
        let mut any = false;
        for p in points.iter().filter(|p| p.day > today - since) {
            acc.add(p.price, p.volume);
            any = true;
        }
        any.then(|| acc.point(0).price)
    };
    let low = points
        .iter()
        .min_by_key(|p| (p.price, -p.day))
        .map(|p| (p.price, p.day));
    let high = points
        .iter()
        .max_by_key(|p| (p.price, p.day))
        .map(|p| (p.price, p.day));
    Summary {
        avg_30: avg(30),
        avg_90: avg(90),
        avg_365: avg(365),
        avg_all: avg(i64::MAX / 4),
        low,
        high,
        sales_30: points
            .iter()
            .filter(|p| p.day > today - 30)
            .map(|p| p.volume)
            .sum(),
        first_day: points.first().map(|p| p.day),
        last_day: points.last().map(|p| p.day),
    }
}

/// Groups points into buckets of `days` days (weeks, months), counted from
/// 1970, each with the volume-weighted price and the total sold.
pub fn grouped(points: &[Point], days: i64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    let mut acc: Option<(i64, Acc)> = None;
    for p in points {
        let bucket = p.day.div_euclid(days) * days;
        match &mut acc {
            Some((b, a)) if *b == bucket => a.add(p.price, p.volume),
            _ => {
                if let Some((b, a)) = acc.take() {
                    out.push(a.point(b));
                }
                let mut a = Acc::default();
                a.add(p.price, p.volume);
                acc = Some((bucket, a));
            }
        }
    }
    if let Some((b, a)) = acc {
        out.push(a.point(b));
    }
    out
}

/// The stretch the app keeps: every day of the last 90, then weekly points
/// back to a year ago.
pub fn recent(points: &[Point], today: i64) -> Vec<Point> {
    let daily_from = today - DAILY_DAYS + 1;
    let kept_from = today - KEPT_DAYS + 1;
    let older: Vec<Point> = points
        .iter()
        .copied()
        .filter(|p| p.day >= kept_from && p.day < daily_from)
        .collect();
    let mut out = grouped(&older, 7);
    // A week that straddles the daily stretch keeps only its older days.
    out.retain(|p| p.day < daily_from);
    out.extend(points.iter().copied().filter(|p| p.day >= daily_from));
    out
}

/// A whole history shaped for a chart: daily for 90 days, weekly for two
/// years, then one point per 30 days. A ten-year history is about 300 points.
pub fn for_chart(points: &[Point], today: i64) -> Vec<Point> {
    let daily_from = today - DAILY_DAYS + 1;
    let weekly_from = today - 2 * 365 + 1;
    let monthly: Vec<Point> = points
        .iter()
        .copied()
        .filter(|p| p.day < weekly_from)
        .collect();
    let weekly: Vec<Point> = points
        .iter()
        .copied()
        .filter(|p| p.day >= weekly_from && p.day < daily_from)
        .collect();
    let mut out = grouped(&monthly, 30);
    out.retain(|p| p.day < weekly_from);
    let mut w = grouped(&weekly, 7);
    w.retain(|p| p.day < daily_from);
    out.extend(w);
    out.extend(points.iter().copied().filter(|p| p.day >= daily_from));
    out
}

// ---- Storage ------------------------------------------------------------------

const FORMAT: u8 = 1;

/// Packs points into a few bytes each: the gap in days from the previous
/// point, the price and the volume, as variable-length numbers.
pub fn encode(points: &[Point]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + points.len() * 4);
    out.push(FORMAT);
    put(&mut out, points.len() as u64);
    let mut prev = 0i64;
    for (i, p) in points.iter().enumerate() {
        let gap = if i == 0 { p.day } else { p.day - prev };
        put(&mut out, zigzag(gap));
        put(&mut out, p.price.max(0) as u64);
        put(&mut out, p.volume.max(0) as u64);
        prev = p.day;
    }
    out
}

/// Unpacks what `encode` made. `None` for anything it didn't make.
pub fn decode(bytes: &[u8]) -> Option<Vec<Point>> {
    let (&format, mut rest) = bytes.split_first()?;
    if format != FORMAT {
        return None;
    }
    let count = take(&mut rest)? as usize;
    if count > 100_000 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut day = 0i64;
    for _ in 0..count {
        let gap = unzigzag(take(&mut rest)?);
        day = day.checked_add(gap)?;
        let price = i64::try_from(take(&mut rest)?).ok()?;
        let volume = i64::try_from(take(&mut rest)?).ok()?;
        out.push(Point { day, price, volume });
    }
    rest.is_empty().then_some(out)
}

fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn unzigzag(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

fn put(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn take(input: &mut &[u8]) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = input.split_first()?;
        *input = rest;
        n |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

// ---- Dates ----------------------------------------------------------------------

/// Days since 1970 for a calendar date (proleptic Gregorian).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Reads the older price history's dates, like "May 16 2013 01: +0", as Unix time.
pub fn parse_old_date(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let day: u32 = parts.next()?.parse().ok()?;
    let year: i64 = parts.next()?.parse().ok()?;
    let hour: i64 = parts.next()?.trim_end_matches(':').parse().ok()?;
    if !(1..=31).contains(&day) || !(0..24).contains(&hour) {
        return None;
    }
    Some(days_from_civil(year, month, day) * DAY + hour * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(day: i64, price: i64, volume: i64) -> Point {
        Point { day, price, volume }
    }

    #[test]
    fn hourly_points_become_one_per_day() {
        let base = 20_000 * DAY;
        let raw = [
            (base + 3600, 100, 1),
            (base + 7200, 200, 3),
            (base + DAY, 50, 2),
            (base - DAY + 10, 80, 0),
        ];
        assert_eq!(
            daily(&raw),
            vec![p(19_999, 80, 0), p(20_000, 175, 4), p(20_001, 50, 2)]
        );
    }

    #[test]
    fn averages_and_extremes() {
        let today = 1000;
        let pts = vec![
            p(500, 400, 1),
            p(900, 100, 2),
            p(980, 200, 1),
            p(1000, 300, 1),
        ];
        let s = summarize(&pts, today);
        assert_eq!(s.avg_30, Some(250)); // (200 + 300) / 2
        assert_eq!(s.avg_90, Some(250));
        assert_eq!(s.avg_365, Some(175)); // (100*2 + 200 + 300) / 4
        assert_eq!(s.avg_all, Some(220)); // (400 + 200 + 200 + 300) / 5
        assert_eq!(s.low, Some((100, 900)));
        assert_eq!(s.high, Some((400, 500)));
        assert_eq!(s.sales_30, 2);
        assert_eq!((s.first_day, s.last_day), (Some(500), Some(1000)));
        assert_eq!(summarize(&[], today), Summary::default());
    }

    #[test]
    fn recent_keeps_ninety_days_and_weeks_before() {
        let today = 20_000;
        let pts: Vec<Point> = (today - 400..=today).map(|d| p(d, 100, 1)).collect();
        let r = recent(&pts, today);
        let daily_from = today - DAILY_DAYS + 1;
        assert_eq!(r.iter().filter(|x| x.day >= daily_from).count(), 90);
        assert!(r.iter().all(|x| x.day >= today - KEPT_DAYS - 6));
        let weeks = r.iter().filter(|x| x.day < daily_from).count();
        assert!((38..=40).contains(&weeks), "{weeks} weeks");
        assert!(r.windows(2).all(|w| w[0].day < w[1].day));
        // Volume is kept: every day sold one.
        let total: i64 = r.iter().map(|x| x.volume).sum();
        assert!(total <= 365 && total >= 358, "{total}");
    }

    #[test]
    fn chart_shape_stays_small() {
        let today = 20_000;
        let pts: Vec<Point> = (today - 3650..=today)
            .map(|d| p(d, 100 + d % 7, 2))
            .collect();
        let c = for_chart(&pts, today);
        assert!(c.len() < 320, "{} points", c.len());
        assert!(c.windows(2).all(|w| w[0].day < w[1].day));
        assert_eq!(c.last().unwrap().day, today);
    }

    #[test]
    fn storage_round_trip() {
        let pts = vec![p(19_000, 70, 3), p(19_007, 1_234_567, 0), p(19_008, 0, 250)];
        let bytes = encode(&pts);
        assert!(bytes.len() < 20, "{} bytes", bytes.len());
        assert_eq!(decode(&bytes).unwrap(), pts);
        assert_eq!(decode(&encode(&[])).unwrap(), vec![]);
        assert!(decode(&[9, 0]).is_none());
        assert!(decode(&bytes[..bytes.len() - 1]).is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn old_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(
            parse_old_date("May 16 2013 01: +0"),
            Some(days_from_civil(2013, 5, 16) * DAY + 3600)
        );
        assert_eq!(
            parse_old_date("Oct 22 2026 15: +0").map(|t| t % DAY),
            Some(15 * 3600)
        );
        assert!(parse_old_date("Foo 1 2013 01: +0").is_none());
        assert!(parse_old_date("May 40 2013 01: +0").is_none());
    }
}
