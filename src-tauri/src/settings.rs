//! User settings, saved as JSON in the data folder and edited from the app's
//! settings panel.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::steam::ProfileView;
use crate::{hexutil, secret};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Greyscale,
    Dark,
    Light,
}

impl Theme {
    /// The Windows title bar only knows light and dark.
    pub fn window_theme(self) -> tauri::Theme {
        match self {
            Theme::Light => tauri::Theme::Light,
            _ => tauri::Theme::Dark,
        }
    }
}

/// How far back the average price looks.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PriceWindow {
    #[default]
    Month,
    Quarter,
    Year,
    All,
}

/// Spike and dip sensitivity, used when the settings file has none.
pub const SPIKE_PERCENT: u32 = 25;
pub const SPIKE_MIN_CENTS: u32 = 5;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub auto_update: bool,
    pub currency: String,
    pub profile: Option<ProfileView>,
    /// The API key, encrypted for the current Windows user and hex-encoded.
    pub api_key_protected: Option<String>,
    /// The last four characters of the key, so the user can recognise it.
    pub api_key_hint: Option<String>,
    /// The account the user signed in to Steam with, if they did. The sign-in
    /// cookies themselves stay in the WebView's encrypted cookie store.
    pub signed_in_steam_id: Option<String>,
    /// List and catalog games shared through the user's Steam Family.
    pub include_family: bool,
    /// Cards also need a copy held in the inventory on top of the badge.
    pub keep_card_set: bool,
    /// The span the average price covers.
    pub price_window: PriceWindow,
    /// How far, in percent, today's price must be from the average to be
    /// tagged a spike or a dip.
    pub spike_percent: u32,
    /// And by at least this many cents, so a 3-cent card moving a cent isn't flagged.
    pub spike_min_cents: u32,
    /// Take prices from the shared price file before asking Steam.
    pub shared_prices: bool,
    /// A GitHub token that publishes the shared price file, encrypted like
    /// the API key. Only the owner's copy has one.
    pub github_token_protected: Option<String>,
    pub github_token_hint: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Greyscale,
            auto_update: true,
            currency: "USD".to_string(),
            profile: None,
            api_key_protected: None,
            api_key_hint: None,
            signed_in_steam_id: None,
            include_family: false,
            keep_card_set: false,
            price_window: PriceWindow::Month,
            spike_percent: SPIKE_PERCENT,
            spike_min_cents: SPIKE_MIN_CENTS,
            shared_prices: true,
            github_token_protected: None,
            github_token_hint: None,
        }
    }
}

/// What the interface is allowed to see. The key itself never leaves Rust.
#[derive(Serialize, Clone, Debug)]
pub struct SettingsView {
    pub theme: Theme,
    pub auto_update: bool,
    pub currency: String,
    pub profile: Option<ProfileView>,
    pub has_api_key: bool,
    pub api_key_hint: Option<String>,
    pub signed_in: bool,
    pub include_family: bool,
    pub keep_card_set: bool,
    pub price_window: PriceWindow,
    pub spike_percent: u32,
    pub spike_min_cents: u32,
    pub shared_prices: bool,
    pub publishing_prices: bool,
    pub github_token_hint: Option<String>,
}

/// Encrypts a secret for the current Windows user, with its last four
/// characters so the user can recognise it.
fn seal(secret_text: &str) -> Result<(String, String), String> {
    let sealed = secret::protect(secret_text.as_bytes())?;
    let hint: String = secret_text
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok((hexutil::encode(&sealed), hint))
}

fn unseal(hex: Option<&str>, damaged: &str) -> Result<Option<String>, String> {
    let Some(hex) = hex else {
        return Ok(None);
    };
    let sealed = hexutil::decode(hex).ok_or_else(|| damaged.to_string())?;
    let plain = secret::unprotect(&sealed)?;
    String::from_utf8(plain)
        .map(Some)
        .map_err(|_| damaged.to_string())
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => settings,
                Err(_) => {
                    // Keep the unreadable file for troubleshooting and start fresh.
                    let _ = fs::rename(path, path.with_extension("broken.json"));
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }

    pub fn set_api_key(&mut self, key: &str) -> Result<(), String> {
        let (sealed, hint) = seal(key)?;
        self.api_key_protected = Some(sealed);
        self.api_key_hint = Some(hint);
        Ok(())
    }

    pub fn api_key(&self) -> Result<Option<String>, String> {
        unseal(
            self.api_key_protected.as_deref(),
            "Your saved API key is damaged. Enter it again in Settings.",
        )
    }

    pub fn set_github_token(&mut self, token: &str) -> Result<(), String> {
        let (sealed, hint) = seal(token)?;
        self.github_token_protected = Some(sealed);
        self.github_token_hint = Some(hint);
        Ok(())
    }

    pub fn clear_github_token(&mut self) {
        self.github_token_protected = None;
        self.github_token_hint = None;
    }

    pub fn github_token(&self) -> Result<Option<String>, String> {
        unseal(
            self.github_token_protected.as_deref(),
            "Your saved GitHub token is damaged. Enter it again in Settings.",
        )
    }

    pub fn has_github_token(&self) -> bool {
        matches!(self.github_token(), Ok(Some(_)))
    }

    pub fn view(&self) -> SettingsView {
        SettingsView {
            theme: self.theme,
            auto_update: self.auto_update,
            currency: self.currency.clone(),
            profile: self.profile.clone(),
            has_api_key: matches!(self.api_key(), Ok(Some(_))),
            api_key_hint: self.api_key_hint.clone(),
            signed_in: self.signed_in_steam_id.is_some(),
            include_family: self.include_family,
            keep_card_set: self.keep_card_set,
            price_window: self.price_window,
            spike_percent: self.spike_percent,
            spike_min_cents: self.spike_min_cents,
            shared_prices: self.shared_prices,
            publishing_prices: self.has_github_token(),
            github_token_hint: self.github_token_hint.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(s.theme, Theme::Light);
        assert!(s.auto_update);
        assert_eq!(s.currency, "USD");
        assert!(!s.include_family);
        assert!(!s.keep_card_set);
        assert_eq!(s.price_window, PriceWindow::Month);
        assert_eq!((s.spike_percent, s.spike_min_cents), (25, 5));
        assert!(s.shared_prices);
        assert!(!s.has_github_token());
    }

    #[test]
    fn github_token_round_trip() {
        let mut s = Settings::default();
        s.set_github_token("github_pat_11AAAAexample0123").unwrap();
        assert_eq!(
            s.github_token().unwrap().as_deref(),
            Some("github_pat_11AAAAexample0123")
        );
        assert_eq!(s.github_token_hint.as_deref(), Some("0123"));
        assert!(s.view().publishing_prices);
        s.clear_github_token();
        assert!(!s.view().publishing_prices);
        assert!(s.github_token_hint.is_none());
    }

    #[test]
    fn api_key_round_trip_and_hint() {
        let mut s = Settings::default();
        s.set_api_key("0123456789ABCDEF0123456789ABCDEF").unwrap();
        assert_eq!(
            s.api_key().unwrap().as_deref(),
            Some("0123456789ABCDEF0123456789ABCDEF")
        );
        assert_eq!(s.api_key_hint.as_deref(), Some("CDEF"));
        assert!(s.view().has_api_key);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("0123456789ABCDEF0123456789ABCDEF") || cfg!(not(windows)));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("sct-settings-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.theme = Theme::Dark;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).theme, Theme::Dark);
        fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load(&path).theme, Theme::Greyscale);
        let _ = fs::remove_dir_all(&dir);
    }
}
