//! Small helpers for Steam's JSON, which mixes numbers, numeric strings and
//! 1/0 for booleans, sometimes within the same answer.

use serde_json::Value;

/// A whole number, whether Steam sent it as a number, a string or a boolean.
pub fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// A non-empty, trimmed string.
pub fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A yes/no value, from `true`, `1` or `"1"` and their opposites.
pub fn flag(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.trim() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_mixed_values() {
        assert_eq!(int(&json!(5)), Some(5));
        assert_eq!(int(&json!("42")), Some(42));
        assert_eq!(int(&json!(true)), Some(1));
        assert_eq!(int(&json!(null)), None);
        assert_eq!(text(&json!("  hi ")), Some("hi".to_string()));
        assert_eq!(text(&json!("")), None);
        assert_eq!(flag(&json!(1)), Some(true));
        assert_eq!(flag(&json!("0")), Some(false));
        assert_eq!(flag(&json!(false)), Some(false));
        assert_eq!(flag(&json!("maybe")), None);
    }
}
