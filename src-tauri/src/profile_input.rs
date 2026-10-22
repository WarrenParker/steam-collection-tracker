//! Turns whatever the user pastes (a profile link, a custom URL name or a
//! 17-digit Steam ID) into something the Steam API can look up.

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileRef {
    SteamId(String),
    Vanity(String),
}

const NOT_A_PROFILE: &str =
    "That doesn't look like a Steam profile. Paste a link like steamcommunity.com/id/yourname, your custom URL name, or your 17-digit Steam ID.";

pub fn parse(input: &str) -> Result<ProfileRef, String> {
    let original = input.trim();
    if original.is_empty() {
        return Err("Enter your Steam profile link or custom URL name.".to_string());
    }

    // Lowercasing ASCII keeps byte positions, so we can match on the lowercase
    // copy and slice the original (custom URL names keep their case).
    let lower = original.to_ascii_lowercase();
    let mut start = 0;
    for prefix in ["https://", "http://"] {
        if lower[start..].starts_with(prefix) {
            start += prefix.len();
            break;
        }
    }
    if lower[start..].starts_with("www.") {
        start += 4;
    }

    if lower[start..].starts_with("steamcommunity.com/") {
        let rest = &original[start + "steamcommunity.com/".len()..];
        let mut parts = rest.split(['/', '?', '#']).filter(|p| !p.is_empty());
        return match (parts.next(), parts.next()) {
            (Some(kind), Some(id)) if kind.eq_ignore_ascii_case("profiles") && is_steam_id(id) => {
                Ok(ProfileRef::SteamId(id.to_string()))
            }
            (Some(kind), Some(name)) if kind.eq_ignore_ascii_case("id") && is_vanity(name) => {
                Ok(ProfileRef::Vanity(name.to_string()))
            }
            _ => Err(NOT_A_PROFILE.to_string()),
        };
    }

    if is_steam_id(original) {
        return Ok(ProfileRef::SteamId(original.to_string()));
    }
    if is_vanity(original) {
        return Ok(ProfileRef::Vanity(original.to_string()));
    }
    Err(NOT_A_PROFILE.to_string())
}

fn is_steam_id(s: &str) -> bool {
    s.len() == 17 && s.starts_with("7656119") && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_vanity(s: &str) -> bool {
    (2..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        assert_eq!(
            parse("https://steamcommunity.com/id/WolfCheevos/").unwrap(),
            ProfileRef::Vanity("WolfCheevos".into())
        );
        assert_eq!(
            parse("steamcommunity.com/id/wolfcheevos").unwrap(),
            ProfileRef::Vanity("wolfcheevos".into())
        );
        assert_eq!(
            parse("http://www.steamcommunity.com/profiles/76561197960287930/?l=english").unwrap(),
            ProfileRef::SteamId("76561197960287930".into())
        );
        assert_eq!(
            parse("  https://steamcommunity.com/id/some_name/badges/ ").unwrap(),
            ProfileRef::Vanity("some_name".into())
        );
    }

    #[test]
    fn bare_values() {
        assert_eq!(
            parse("76561197960287930").unwrap(),
            ProfileRef::SteamId("76561197960287930".into())
        );
        assert_eq!(
            parse("gabelogannewell").unwrap(),
            ProfileRef::Vanity("gabelogannewell".into())
        );
    }

    #[test]
    fn rejects() {
        assert!(parse("").is_err());
        assert!(parse("https://store.steampowered.com/app/440").is_err());
        assert!(parse("steamcommunity.com/profiles/123").is_err());
        assert!(parse("has spaces in it").is_err());
    }
}
