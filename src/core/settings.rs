use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use super::applier::ApplyTargets;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub apply_hyprland: bool,
    pub apply_gsettings: bool,
    pub apply_gtk: bool,
    pub apply_qt: bool,
    pub apply_environment: bool,
    pub apply_xresources: bool,
    pub apply_default_index: bool,
    pub apply_flatpak: bool,
    pub auto_apply_on_import: bool,
    pub apply_size_immediately: bool,
    pub show_user_themes: bool,
    pub show_system_themes: bool,
    pub library_filter: String,
    pub library_type: String,
    pub last_page: i32,
    pub preferred_size: u32,
    pub enable_hyprcursor: bool,
    pub hide_on_key_press: bool,
    pub hide_on_touch: bool,
    pub no_hardware_cursors: bool,
    pub inactive_timeout: i32,
    pub auto_update: bool,
    pub auto_update_when: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            apply_hyprland: true,
            apply_gsettings: true,
            apply_gtk: true,
            apply_qt: true,
            apply_environment: true,
            apply_xresources: true,
            apply_default_index: true,
            apply_flatpak: true,
            auto_apply_on_import: false,
            apply_size_immediately: true,
            show_user_themes: true,
            show_system_themes: true,
            library_filter: "all".into(),
            library_type: "all".into(),
            last_page: 0,
            preferred_size: 24,
            enable_hyprcursor: true,
            hide_on_key_press: false,
            hide_on_touch: true,
            no_hardware_cursors: false,
            inactive_timeout: 0,
            auto_update: false,
            auto_update_when: "next-launch".into(),
        }
    }
}

impl AppSettings {
    pub fn auto_update_when_index(&self) -> i32 {
        match self.auto_update_when.as_str() {
            "background" => 1,
            "instantly" => 2,
            _ => 0,
        }
    }

    pub fn auto_update_when_from_index(index: i32) -> String {
        match index {
            1 => "background".into(),
            2 => "instantly".into(),
            _ => "next-launch".into(),
        }
    }

    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("mouse-me").join("settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("Could not locate config directory")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let parent = path.parent().ok_or("Invalid config path")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary
            .write_all(raw.as_bytes())
            .map_err(|e| e.to_string())?;
        temporary
            .persist(&path)
            .map_err(|error| error.error.to_string())?;
        Ok(())
    }

    pub fn apply_targets(&self) -> ApplyTargets {
        ApplyTargets {
            hyprland: self.apply_hyprland,
            gsettings: self.apply_gsettings,
            gtk: self.apply_gtk,
            qt: self.apply_qt,
            environment: self.apply_environment,
            xresources: self.apply_xresources,
            default_index: self.apply_default_index,
            flatpak: self.apply_flatpak,
        }
    }

    pub fn keys() -> &'static [&'static str] {
        &[
            "apply-hyprland",
            "apply-gsettings",
            "apply-gtk",
            "apply-qt",
            "apply-environment",
            "apply-xresources",
            "apply-default-index",
            "apply-flatpak",
            "auto-apply-on-import",
            "apply-size-immediately",
            "show-user-themes",
            "show-system-themes",
            "library-filter",
            "library-type",
            "preferred-size",
            "enable-hyprcursor",
            "hide-on-key-press",
            "hide-on-touch",
            "no-hardware-cursors",
            "inactive-timeout",
            "auto-update",
            "auto-update-when",
        ]
    }

    pub fn get_key(&self, key: &str) -> Result<String, String> {
        match normalize_key(key).as_str() {
            "apply-hyprland" => Ok(bool_str(self.apply_hyprland)),
            "apply-gsettings" => Ok(bool_str(self.apply_gsettings)),
            "apply-gtk" => Ok(bool_str(self.apply_gtk)),
            "apply-qt" => Ok(bool_str(self.apply_qt)),
            "apply-environment" => Ok(bool_str(self.apply_environment)),
            "apply-xresources" => Ok(bool_str(self.apply_xresources)),
            "apply-default-index" => Ok(bool_str(self.apply_default_index)),
            "apply-flatpak" => Ok(bool_str(self.apply_flatpak)),
            "auto-apply-on-import" => Ok(bool_str(self.auto_apply_on_import)),
            "apply-size-immediately" => Ok(bool_str(self.apply_size_immediately)),
            "show-user-themes" => Ok(bool_str(self.show_user_themes)),
            "show-system-themes" => Ok(bool_str(self.show_system_themes)),
            "library-filter" => Ok(self.library_filter.clone()),
            "library-type" => Ok(self.library_type.clone()),
            "preferred-size" => Ok(self.preferred_size.to_string()),
            "enable-hyprcursor" => Ok(bool_str(self.enable_hyprcursor)),
            "hide-on-key-press" => Ok(bool_str(self.hide_on_key_press)),
            "hide-on-touch" => Ok(bool_str(self.hide_on_touch)),
            "no-hardware-cursors" => Ok(bool_str(self.no_hardware_cursors)),
            "inactive-timeout" => Ok(self.inactive_timeout.to_string()),
            "auto-update" => Ok(bool_str(self.auto_update)),
            "auto-update-when" => Ok(self.auto_update_when.clone()),
            other => Err(unknown_setting(other)),
        }
    }

    pub fn set_key(&mut self, key: &str, value: &str) -> Result<SettingEffect, String> {
        match normalize_key(key).as_str() {
            "apply-hyprland" => {
                self.apply_hyprland = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-gsettings" => {
                self.apply_gsettings = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-gtk" => {
                self.apply_gtk = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-qt" => {
                self.apply_qt = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-environment" => {
                self.apply_environment = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-xresources" => {
                self.apply_xresources = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-default-index" => {
                self.apply_default_index = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-flatpak" => {
                self.apply_flatpak = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "auto-apply-on-import" => {
                self.auto_apply_on_import = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "apply-size-immediately" => {
                self.apply_size_immediately = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "show-user-themes" => {
                self.show_user_themes = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "show-system-themes" => {
                self.show_system_themes = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "library-filter" => {
                self.library_filter = parse_choice(value, &["all", "user", "system"], key)?;
                Ok(SettingEffect::None)
            }
            "library-type" => {
                self.library_type = parse_choice(value, &["all", "xcursor", "hyprcursor"], key)?;
                Ok(SettingEffect::None)
            }
            "preferred-size" => {
                self.preferred_size = parse_preferred_size(value)?;
                Ok(SettingEffect::PreferredSize)
            }
            "enable-hyprcursor" => {
                self.enable_hyprcursor = parse_bool(value)?;
                Ok(SettingEffect::HyprPrefs)
            }
            "hide-on-key-press" => {
                self.hide_on_key_press = parse_bool(value)?;
                Ok(SettingEffect::HyprPrefs)
            }
            "hide-on-touch" => {
                self.hide_on_touch = parse_bool(value)?;
                Ok(SettingEffect::HyprPrefs)
            }
            "no-hardware-cursors" => {
                self.no_hardware_cursors = parse_bool(value)?;
                Ok(SettingEffect::HyprPrefs)
            }
            "inactive-timeout" => {
                self.inactive_timeout = parse_inactive_timeout(value)?;
                Ok(SettingEffect::HyprPrefs)
            }
            "auto-update" => {
                self.auto_update = parse_bool(value)?;
                Ok(SettingEffect::None)
            }
            "auto-update-when" => {
                self.auto_update_when =
                    parse_choice(value, &["next-launch", "background", "instantly"], key)?;
                Ok(SettingEffect::None)
            }
            other => Err(unknown_setting(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingEffect {
    None,
    HyprPrefs,
    PreferredSize,
}

fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace('_', "-")
}

fn bool_str(value: bool) -> String {
    if value {
        "true".into()
    } else {
        "false".into()
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("Expected true or false, got '{}'", value.trim())),
    }
}

fn parse_choice(value: &str, allowed: &[&str], key: &str) -> Result<String, String> {
    let normalized = normalize_key(value);
    if allowed.iter().any(|item| *item == normalized) {
        Ok(normalized)
    } else {
        Err(format!(
            "{} must be one of: {}",
            normalize_key(key),
            allowed.join(", ")
        ))
    }
}

fn parse_preferred_size(value: &str) -> Result<u32, String> {
    let size = value
        .trim()
        .parse::<u32>()
        .map_err(|_| "preferred-size must be a number".to_string())?;
    if !(1..=512).contains(&size) {
        return Err("preferred-size must be between 1 and 512".into());
    }
    Ok(size)
}

fn parse_inactive_timeout(value: &str) -> Result<i32, String> {
    let timeout = value
        .trim()
        .parse::<i32>()
        .map_err(|_| "inactive-timeout must be a number".to_string())?;
    if !(0..=60).contains(&timeout) {
        return Err("inactive-timeout must be between 0 and 60".into());
    }
    Ok(timeout)
}

fn unknown_setting(key: &str) -> String {
    format!("Unknown setting '{key}'. Run mouse-me settings to list keys.")
}
