//! The shared price file: the Market prices one copy of the app has read, in
//! a small text form that other copies read before asking Steam.
//!
//! The file is plain text, one record per line, with tab-separated fields,
//! and is gzip-compressed on the way out. The first line names the format and
//! its version. Then come:
//!
//! - `made <unix time>`: when the file was built.
//! - `game <appid> <checked at>`: a game whose Market search finished, and
//!   when it started. Items of that game the file lists with an older
//!   `seen_at` weren't for sale at that time.
//! - `columns <name>...`: the item columns, in the order rows give them.
//! - `row <value>...`: one Market item.
//!
//! Readers match columns by name, ignore ones they don't know and treat
//! missing ones as empty, so later versions can add columns. `\N` is an empty
//! value; text escapes `\\`, tabs and line breaks; blobs are hex.
//!
//! Everything read is checked against strict limits, because the file comes
//! from the internet: a damaged or hostile file can't make the app store
//! nonsense or run out of memory.

pub const MAGIC: &str = "steam-collection-tracker prices";
pub const VERSION: u32 = 1;

/// Limits on what a file may hold.
pub const MAX_GAMES: usize = 100_000;
pub const MAX_ROWS: usize = 1_000_000;
const MAX_TEXT: usize = 2_048;
const MAX_BLOB: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// An integer from `min` to `max`.
    Int(i64, i64),
    Blob,
}

/// Times from 2003 (Steam's start) to 2100.
const TIME: Kind = Kind::Int(1_041_379_200, 4_102_444_800);
/// Days since 1970, over the same span.
const DAY: Kind = Kind::Int(12_053, 47_482);
/// Up to a billion in mills.
const MONEY: Kind = Kind::Int(0, 1_000_000_000_000);
const COUNT: Kind = Kind::Int(0, 1_000_000_000);
const CURRENCY: Kind = Kind::Int(0, 1_000);

/// The `market` table's columns that are shared, with what each may hold.
pub const COLUMNS: &[(&str, Kind)] = &[
    ("hash_name", Kind::Text),
    ("appid", Kind::Int(1, 100_000_000)),
    ("class", Kind::Int(0, 64)),
    ("foil", Kind::Int(0, 1)),
    ("name", Kind::Text),
    ("type_label", Kind::Text),
    ("icon", Kind::Text),
    ("currency", CURRENCY),
    ("lowest", MONEY),
    ("listings", COUNT),
    ("seen_at", TIME),
    ("history_at", TIME),
    ("history_due", TIME),
    ("history_currency", CURRENCY),
    ("avg_30", MONEY),
    ("avg_90", MONEY),
    ("avg_365", MONEY),
    ("avg_all", MONEY),
    ("low_price", MONEY),
    ("low_day", DAY),
    ("high_price", MONEY),
    ("high_day", DAY),
    ("sales_30", COUNT),
    ("first_day", DAY),
    ("last_day", DAY),
    ("recent", Kind::Blob),
    ("book_at", TIME),
    ("book_currency", CURRENCY),
    ("book", Kind::Text),
    ("buy_max", MONEY),
    ("sell_count", COUNT),
    ("buy_count", COUNT),
];

/// Columns a row can't do without.
const REQUIRED: &[&str] = &["hash_name", "appid", "class", "name", "currency", "seen_at"];

/// Where a column sits in a row.
pub fn column(name: &str) -> Option<usize> {
    COLUMNS.iter().position(|(n, _)| *n == name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Game {
    pub appid: u64,
    pub checked_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub made_at: i64,
    pub games: Vec<Game>,
    /// Each row holds one value per entry in `COLUMNS`, in that order.
    pub rows: Vec<Vec<Value>>,
}

impl File {
    /// The currency most rows are in (Steam's number), or 0 when empty.
    pub fn currency(&self) -> i64 {
        let at = column("currency").unwrap_or(0);
        let mut counts: Vec<(i64, usize)> = Vec::new();
        for row in &self.rows {
            if let Some(c) = row.get(at).and_then(Value::int) {
                match counts.iter_mut().find(|(id, _)| *id == c) {
                    Some(entry) => entry.1 += 1,
                    None => counts.push((c, 1)),
                }
            }
        }
        counts.into_iter().max_by_key(|e| e.1).map_or(0, |e| e.0)
    }
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

fn unescape(field: &str) -> Option<String> {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

fn hex(bytes: &[u8], out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}

/// Writes a file. Rows must hold one value per entry in `COLUMNS`.
pub fn encode(file: &File) -> String {
    let mut out = String::with_capacity(file.rows.len() * 160 + file.games.len() * 24 + 256);
    out.push_str(&format!("{MAGIC} {VERSION}\nmade\t{}\n", file.made_at));
    for g in &file.games {
        out.push_str(&format!("game\t{}\t{}\n", g.appid, g.checked_at));
    }
    out.push_str("columns");
    for (name, _) in COLUMNS {
        out.push('\t');
        out.push_str(name);
    }
    out.push('\n');
    for row in &file.rows {
        out.push_str("row");
        for value in row.iter().take(COLUMNS.len()) {
            out.push('\t');
            match value {
                Value::Null => out.push_str("\\N"),
                Value::Int(v) => out.push_str(&v.to_string()),
                Value::Text(t) => escape(t, &mut out),
                Value::Blob(b) => hex(b, &mut out),
            }
        }
        out.push('\n');
    }
    out
}

fn parse_value(field: &str, kind: Kind) -> Option<Value> {
    if field == "\\N" {
        return Some(Value::Null);
    }
    match kind {
        Kind::Int(min, max) => {
            let v: i64 = field.parse().ok()?;
            (min..=max).contains(&v).then_some(Value::Int(v))
        }
        Kind::Text => {
            let t = unescape(field)?;
            (t.len() <= MAX_TEXT && !t.chars().any(|c| c.is_control() && c != '\t' && c != '\n'))
                .then_some(Value::Text(t))
        }
        Kind::Blob => {
            let b = unhex(field)?;
            (b.len() <= MAX_BLOB).then_some(Value::Blob(b))
        }
    }
}

/// Why a file couldn't be read.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Not a price file, or one from a newer version of the app.
    NotPrices,
    TooBig,
}

/// Reads a file. Rows with an unreadable or out-of-range value are dropped
/// whole, so a damaged line can't half-update an item.
pub fn decode(text: &str) -> Result<File, ReadError> {
    decode_limited(text, MAX_ROWS)
}

fn decode_limited(text: &str, max_rows: usize) -> Result<File, ReadError> {
    let mut lines = text.lines();
    let first = lines.next().ok_or(ReadError::NotPrices)?;
    let version: u32 = first
        .strip_prefix(MAGIC)
        .and_then(|rest| rest.trim().parse().ok())
        .ok_or(ReadError::NotPrices)?;
    if version == 0 || version > VERSION {
        return Err(ReadError::NotPrices);
    }
    let mut file = File::default();
    // For each column in the file, where it goes in our rows.
    let mut layout: Option<Vec<Option<usize>>> = None;
    let required: Vec<usize> = REQUIRED.iter().filter_map(|n| column(n)).collect();
    for line in lines {
        let mut fields = line.split('\t');
        match fields.next() {
            Some("made") => {
                if let Some(Value::Int(t)) = fields.next().and_then(|f| parse_value(f, TIME)) {
                    file.made_at = t;
                }
            }
            Some("game") => {
                let appid = fields.next().and_then(|f| f.parse::<u64>().ok());
                let at = fields.next().and_then(|f| parse_value(f, TIME));
                if let (Some(appid), Some(Value::Int(checked_at))) = (appid, at) {
                    if appid > 0 && appid <= 100_000_000 {
                        if file.games.len() >= MAX_GAMES {
                            return Err(ReadError::TooBig);
                        }
                        file.games.push(Game { appid, checked_at });
                    }
                }
            }
            Some("columns") => {
                layout = Some(fields.map(column).collect());
            }
            Some("row") => {
                let Some(layout) = &layout else { continue };
                if file.rows.len() >= max_rows {
                    return Err(ReadError::TooBig);
                }
                let mut row = vec![Value::Null; COLUMNS.len()];
                let mut ok = true;
                for (i, field) in fields.enumerate() {
                    let Some(Some(at)) = layout.get(i) else {
                        continue;
                    };
                    match parse_value(field, COLUMNS[*at].1) {
                        Some(v) => row[*at] = v,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok && required.iter().all(|&at| row[at] != Value::Null) {
                    file.rows.push(row);
                }
            }
            _ => {}
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(hash_name: &str, appid: i64, lowest: Option<i64>) -> Vec<Value> {
        let mut r = vec![Value::Null; COLUMNS.len()];
        let set = |r: &mut Vec<Value>, name: &str, v: Value| r[column(name).unwrap()] = v;
        set(&mut r, "hash_name", Value::Text(hash_name.to_string()));
        set(&mut r, "appid", Value::Int(appid));
        set(&mut r, "class", Value::Int(2));
        set(&mut r, "foil", Value::Int(0));
        set(&mut r, "name", Value::Text(hash_name.to_string()));
        set(&mut r, "currency", Value::Int(1));
        set(&mut r, "listings", Value::Int(12));
        set(&mut r, "seen_at", Value::Int(1_792_800_000));
        if let Some(l) = lowest {
            set(&mut r, "lowest", Value::Int(l));
        }
        r
    }

    #[test]
    fn round_trip() {
        let mut odd = row("220-\"Rise\tand\\shine\"\nMr", 220, Some(90));
        odd[column("recent").unwrap()] = Value::Blob(vec![0, 1, 254, 255]);
        odd[column("book").unwrap()] = Value::Text(String::new());
        let file = File {
            made_at: 1_792_866_504,
            games: vec![
                Game {
                    appid: 220,
                    checked_at: 1_792_820_000,
                },
                Game {
                    appid: 550,
                    checked_at: 1_792_820_500,
                },
            ],
            rows: vec![odd, row("550-Witch (Foil)", 550, None)],
        };
        let text = encode(&file);
        assert_eq!(text.lines().count(), 1 + 1 + 2 + 1 + 2);
        assert_eq!(decode(&text).unwrap(), file);
        assert_eq!(file.currency(), 1);
    }

    #[test]
    fn rejects_other_files_and_newer_versions() {
        assert_eq!(decode(""), Err(ReadError::NotPrices));
        assert_eq!(decode("<html>"), Err(ReadError::NotPrices));
        assert_eq!(
            decode(&format!("{MAGIC} {}\n", VERSION + 1)),
            Err(ReadError::NotPrices)
        );
        assert_eq!(decode(&format!("{MAGIC} 0\n")), Err(ReadError::NotPrices));
        assert!(decode(&format!("{MAGIC} {VERSION}\n"))
            .unwrap()
            .rows
            .is_empty());
    }

    #[test]
    fn columns_match_by_name() {
        // Columns in another order, one unknown, several missing.
        let text = format!(
            "{MAGIC} 1\ncolumns\tseen_at\tfuture_column\tname\tappid\tclass\tcurrency\thash_name\tlowest\n\
             row\t1792800000\twhatever\tBoomer\t550\t2\t1\t550-Boomer\t30\n"
        );
        let file = decode(&text).unwrap();
        assert_eq!(file.rows.len(), 1);
        let r = &file.rows[0];
        assert_eq!(r[column("lowest").unwrap()], Value::Int(30));
        assert_eq!(
            r[column("hash_name").unwrap()],
            Value::Text("550-Boomer".into())
        );
        assert_eq!(r[column("listings").unwrap()], Value::Null);
    }

    #[test]
    fn drops_bad_rows_whole() {
        let good = encode(&File {
            made_at: 1_792_866_504,
            games: vec![],
            rows: vec![row("550-Boomer", 550, Some(30))],
        });
        let header: String = good.lines().take(3).map(|l| format!("{l}\n")).collect();
        let line = good.lines().nth(3).unwrap();
        let bad_lines = [
            line.replace("\t30\t", "\t-5\t"),              // negative price
            line.replace("\t30\t", "\t1e9\t"),             // not an integer
            line.replace("\t550\t", "\t0\t"),              // no such game
            line.replace("\t1792800000", "\t99"),          // a time before Steam
            line.replace("550-Boomer", "550-Boo\\xmer"),   // bad escape
            line.replace("550-Boomer", "\\N"),             // missing name
            line.replace("550-Boomer", "550-Boo\u{7}mer"), // control character
            line.replace("550-Boomer", &"x".repeat(3000)), // too long
        ];
        for bad in &bad_lines {
            let file = decode(&format!("{header}{bad}\n{line}\n")).unwrap();
            assert_eq!(file.rows.len(), 1, "{bad}");
        }
        // Rows before any columns line are ignored.
        let file = decode(&format!("{MAGIC} 1\n{line}\n")).unwrap();
        assert!(file.rows.is_empty());
    }

    #[test]
    fn bad_games_are_skipped() {
        let text = format!(
            "{MAGIC} 1\nmade\t1792866504\ngame\t220\t1792820000\ngame\t0\t1792820000\n\
             game\tabc\t1792820000\ngame\t440\t5\n"
        );
        let file = decode(&text).unwrap();
        assert_eq!(file.made_at, 1_792_866_504);
        assert_eq!(
            file.games,
            vec![Game {
                appid: 220,
                checked_at: 1_792_820_000
            }]
        );
    }

    #[test]
    fn limits_rows() {
        let r = encode(&File {
            made_at: 1_792_866_504,
            games: vec![],
            rows: vec![row("550-Boomer", 550, Some(30))],
        });
        let header: String = r.lines().take(3).map(|l| format!("{l}\n")).collect();
        let line = format!("{}\n", r.lines().nth(3).unwrap());
        let text = format!("{header}{}", line.repeat(11));
        assert_eq!(decode_limited(&text, 10), Err(ReadError::TooBig));
        assert_eq!(decode_limited(&text, 11).unwrap().rows.len(), 11);
    }
}
