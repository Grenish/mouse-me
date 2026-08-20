use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use super::applier::ApplyTargets;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

impl AppSettings {
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
}
