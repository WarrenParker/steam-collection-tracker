//! The banner: an image of the user's progress to share. The interface
//! draws it on a canvas; the engine supplies the pictures (so the canvas can
//! be saved, which pictures loaded straight from another site would block)
//! and saves the finished PNG.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;

/// Largest picture taken for the banner, and largest banner saved.
const MAX_IMAGE: usize = 4 << 20;
const MAX_PNG: usize = 32 << 20;

/// Steam's image servers, the only places banner pictures come from.
fn allowed_image(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && (host.ends_with(".steamstatic.com") || host == "steamcdn-a.akamaihd.net")
}

/// The picture at `url` as a data URL, or `None` when it can't be had.
pub async fn image(http: &reqwest::Client, url: &str) -> Option<String> {
    if !allowed_image(url) {
        return None;
    }
    let mut resp = http
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() || !allowed_image(resp.url().as_str()) {
        return None;
    }
    let kind = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })?;
    if !matches!(
        kind.as_str(),
        "image/jpeg" | "image/png" | "image/webp" | "image/gif"
    ) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_IMAGE {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(body)
    ))
}

/// A game's wide store picture.
pub fn game_art_url(appid: u64) -> String {
    format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{appid}/header.jpg")
}

/// Reads the PNG the interface drew, checking it is one.
pub fn decode_png(data: &str) -> Result<Vec<u8>, String> {
    let bad = || "The banner couldn't be saved. Try again.".to_string();
    let b64 = data.strip_prefix("data:image/png;base64,").unwrap_or(data);
    if b64.len() > MAX_PNG / 3 * 4 + 4 {
        return Err(bad());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| bad())?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Err(bad());
    }
    Ok(bytes)
}

/// Where banners are saved: a folder in the user's Pictures.
pub fn folder(pictures: Option<PathBuf>, fallback: &Path) -> PathBuf {
    pictures
        .map(|p| p.join("Steam Collection Tracker"))
        .unwrap_or_else(|| fallback.join("banners"))
}

/// "steam-collection-2026-10-28-wide.png".
pub fn file_name(date: &str, shape: &str) -> String {
    let shape: String = shape
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let date: String = date
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-')
        .take(10)
        .collect();
    format!("steam-collection-{date}-{shape}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_steam_pictures() {
        assert!(allowed_image(
            "https://avatars.fastly.steamstatic.com/abc_full.jpg"
        ));
        assert!(allowed_image(&game_art_url(440)));
        for bad in [
            "http://avatars.fastly.steamstatic.com/a.jpg",
            "https://evil.com/a.jpg",
            "https://steamstatic.com.evil.com/a.jpg",
            "https://user@avatars.fastly.steamstatic.com/a.jpg",
            "https://avatars.fastly.steamstatic.com:8443/a.jpg",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!allowed_image(bad), "{bad}");
        }
    }

    #[test]
    fn png_check() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0];
        let data = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        assert_eq!(decode_png(&data).unwrap(), png);
        assert!(decode_png("data:image/png;base64,aGVsbG8=").is_err());
        assert!(decode_png("not base64!").is_err());
    }

    #[test]
    fn names_stay_tidy() {
        assert_eq!(
            file_name("2026-10-28", "wide"),
            "steam-collection-2026-10-28-wide.png"
        );
        assert_eq!(
            file_name("../../x", "..\\square"),
            "steam-collection--square.png"
        );
    }
}
