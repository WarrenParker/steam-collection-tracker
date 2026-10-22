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
        let sealed = secret::protect(key.as_bytes())?;
        self.api_key_protected = Some(hexutil::encode(&sealed));
        let hint: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.api_key_hint = Some(hint);
        Ok(())
    }

    pub fn api_key(&self) -> Result<Option<String>, String> {
        let Some(hex) = &self.api_key_protected else {
            return Ok(None);
        };
        let sealed = hexutil::decode(hex).ok_or_else(|| {
            "Your saved API key is damaged. Enter it again in Settings.".to_string()
        })?;
        let plain = secret::unprotect(&sealed)?;
        String::from_utf8(plain)
            .map(Some)
            .map_err(|_| "Your saved API key is damaged. Enter it again in Settings.".to_string())
    }

    pub fn view(&self) -> SettingsView {
        SettingsView {
            theme: self.theme,
            auto_update: self.auto_update,
            currency: self.currency.clone(),
            profile: self.profile.clone(),
            has_api_key: matches!(self.api_key(), Ok(Some(_))),
            api_key_hint: self.api_key_hint.clone(),
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
