use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output};

use super::importer::is_safe_theme_name;

#[derive(Debug, Clone)]
pub struct ApplyTargets {
    pub hyprland: bool,
    pub gsettings: bool,
    pub gtk: bool,
    pub qt: bool,
    pub environment: bool,
    pub xresources: bool,
    pub default_index: bool,
    pub flatpak: bool,
}

impl ApplyTargets {
    pub fn all() -> Self {
        Self {
            hyprland: true,
            gsettings: true,
            gtk: true,
            qt: true,
            environment: true,
            xresources: true,
            default_index: true,
            flatpak: true,
        }
    }
}

pub fn validate_apply_input(theme: &str, size: u32) -> Result<(), String> {
    if theme.trim().is_empty() {
        return Err("Cursor theme name cannot be empty".into());
    }
    if theme.chars().any(|character| character.is_control()) {
        return Err("Cursor theme name cannot contain control characters".into());
    }
    if !is_safe_theme_name(theme.trim()) {
        return Err("Cursor theme name is not a safe directory name".into());
    }
    if size == 0 {
        return Err("Cursor size must be greater than zero".into());
    }
    Ok(())
}

fn checked_output(output: Output, operation: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!(
            "{} failed with status {}",
            operation, output.status
        ))
    } else {
        Err(format!("{} failed: {}", operation, stderr))
    }
}

fn quote_environment_value(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '$' => quoted.push_str("\\$"),
            '`' => quoted.push_str("\\`"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn quote_gtk2_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Applies the given cursor theme and size universally across all Linux subsystems.
/// Returns skipped-target warnings on success.
pub fn apply_system_wide(theme_name: &str, size: u32) -> Result<Vec<String>, String> {
    apply_with_targets(theme_name, size, &ApplyTargets::all())
}

pub fn apply_with_targets(
    theme_name: &str,
    size: u32,
    targets: &ApplyTargets,
) -> Result<Vec<String>, String> {
    validate_apply_input(theme_name, size)?;
    let mut warnings = Vec::new();
    let mut attempted = 0usize;
    let mut failed = 0usize;

    if targets.hyprland {
        attempted += 1;
        if let Err(e) = apply_hyprctl(theme_name, size) {
            failed += 1;
            warnings.push(format!("Hyprctl: {}", e));
        }
    }

    if targets.gsettings {
        attempted += 1;
        if let Err(e) = apply_gsettings(theme_name, size) {
            failed += 1;
            warnings.push(format!("GSettings: {}", e));
        }
    }

    if targets.gtk {
        attempted += 1;
        if let Err(e) = apply_gtk_configs(theme_name, size) {
            failed += 1;
            warnings.push(format!("GTK configs: {}", e));
        }
    }

    if targets.qt {
        attempted += 1;
        if let Err(e) = apply_qt_config(theme_name, size) {
            failed += 1;
            warnings.push(format!("Qt config: {}", e));
        }
    }

    if targets.environment {
        attempted += 1;
        if let Err(e) = apply_environment_d(theme_name, size) {
            failed += 1;
            warnings.push(format!("Environment.d: {}", e));
        }
    }

    if targets.xresources {
        attempted += 1;
        if let Err(e) = apply_xresources(theme_name, size) {
            failed += 1;
            warnings.push(format!("Xresources: {}", e));
        }
    }

    if targets.default_index {
        attempted += 1;
        if let Err(e) = apply_default_index_theme(theme_name) {
            failed += 1;
            warnings.push(format!("Default index.theme: {}", e));
        }
    }

    if targets.flatpak {
        attempted += 1;
        if let Err(e) = apply_flatpak_overrides() {
            failed += 1;
            warnings.push(format!("Flatpak: {}", e));
        }
    }

    if attempted > 0 && failed == attempted {
        Err(format!("Could not apply: {}", warnings.join(", ")))
    } else {
        Ok(warnings)
    }
}

pub fn apply_hypr_cursor_prefs(
    enable_hyprcursor: bool,
    hide_on_key_press: bool,
    hide_on_touch: bool,
    no_hardware_cursors: bool,
    inactive_timeout: i32,
) -> Result<(), String> {
    let mut errors = Vec::new();
    let pairs = [
        (
            "cursor:enable_hyprcursor",
            if enable_hyprcursor { "true" } else { "false" },
        ),
        (
            "cursor:hide_on_key_press",
            if hide_on_key_press { "true" } else { "false" },
        ),
        (
            "cursor:hide_on_touch",
            if hide_on_touch { "true" } else { "false" },
        ),
        (
            "cursor:no_hardware_cursors",
            if no_hardware_cursors { "true" } else { "false" },
        ),
    ];

    for (key, value) in pairs {
        if let Err(e) = hypr_keyword(key, value) {
            errors.push(e);
        }
    }

    let timeout = inactive_timeout.clamp(0, 60).to_string();
    if let Err(e) = hypr_keyword("cursor:inactive_timeout", &timeout) {
        errors.push(e);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(", "))
    }
}

fn hypr_keyword(key: &str, value: &str) -> Result<(), String> {
    let output = Command::new("hyprctl")
        .args(["keyword", key, value])
        .output()
        .map_err(|e| format!("{}: {}", key, e))?;
    checked_output(output, &format!("{} {}", key, value))
}

fn apply_hyprctl(theme: &str, size: u32) -> Result<(), String> {
    let output = Command::new("hyprctl")
        .args(["setcursor", theme, &size.to_string()])
        .output()
        .map_err(|e| e.to_string())?;

    checked_output(output, "hyprctl setcursor")
}

fn apply_gsettings(theme: &str, size: u32) -> Result<(), String> {
    let size = size.to_string();
    let theme_output = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "cursor-theme", theme])
        .output()
        .map_err(|e| e.to_string())?;
    checked_output(theme_output, "gsettings cursor-theme")?;

    let size_output = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "cursor-size", &size])
        .output()
        .map_err(|e| e.to_string())?;
    checked_output(size_output, "gsettings cursor-size")
}

fn apply_gtk_configs(theme: &str, size: u32) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("No home directory")?;

    // ~/.config/gtk-3.0/settings.ini
    let gtk3_dir = home.join(".config").join("gtk-3.0");
    fs::create_dir_all(&gtk3_dir).map_err(|e| e.to_string())?;
    update_ini_setting(
        &gtk3_dir.join("settings.ini"),
        "Settings",
        &[
            ("gtk-cursor-theme-name", theme),
            ("gtk-cursor-theme-size", &size.to_string()),
        ],
    )?;

    // ~/.config/gtk-4.0/settings.ini
    let gtk4_dir = home.join(".config").join("gtk-4.0");
    fs::create_dir_all(&gtk4_dir).map_err(|e| e.to_string())?;
    update_ini_setting(
        &gtk4_dir.join("settings.ini"),
        "Settings",
        &[
            ("gtk-cursor-theme-name", theme),
            ("gtk-cursor-theme-size", &size.to_string()),
        ],
    )?;

    // ~/.gtkrc-2.0
    let gtk2_file = home.join(".gtkrc-2.0");
    update_gtk2_rc(&gtk2_file, theme, size)?;

    Ok(())
}

fn update_ini_setting<const N: usize>(
    path: &Path,
    section_name: &str,
    kv_pairs: &[(&str, &str); N],
) -> Result<(), String> {
    let mut lines = Vec::new();
    if path.exists() {
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            for line_res in reader.lines() {
                lines.push(line_res.map_err(|e| e.to_string())?);
            }
        }
    }

    let section_header = format!("[{}]", section_name);
    let mut section_idx = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().eq_ignore_ascii_case(&section_header) {
            section_idx = Some(i);
            break;
        }
    }

    let mut remaining_keys: std::collections::HashMap<&str, &str> =
        kv_pairs.iter().cloned().collect();

    if let Some(idx) = section_idx {
        let mut i = idx + 1;
        while i < lines.len() {
            let line_trimmed = lines[i].trim().to_string();
            if line_trimmed.starts_with('[') && line_trimmed.ends_with(']') {
                break;
            }
            for (key, val) in kv_pairs {
                let prefix = format!("{}=", key);
                if line_trimmed.starts_with(&prefix) {
                    lines[i] = format!("{}={}", key, val);
                    remaining_keys.remove(key);
                }
            }
            i += 1;
        }
        for (key, val) in remaining_keys {
            lines.insert(idx + 1, format!("{}={}", key, val));
        }
    } else {
        lines.push(section_header);
        for (key, val) in kv_pairs {
            lines.push(format!("{}={}", key, val));
        }
    }

    fs::write(path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(())
}

fn update_gtk2_rc(path: &Path, theme: &str, size: u32) -> Result<(), String> {
    let mut lines = Vec::new();
    if path.exists() {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let reader = BufReader::new(file);
        for line_res in reader.lines() {
            let line = line_res.map_err(|e| e.to_string())?;
            let trimmed = line.trim();
            if !trimmed.starts_with("gtk-cursor-theme-name")
                && !trimmed.starts_with("gtk-cursor-theme-size")
            {
                lines.push(line);
            }
        }
    }

    lines.push(format!(
        "gtk-cursor-theme-name=\"{}\"",
        quote_gtk2_value(theme)
    ));
    lines.push(format!("gtk-cursor-theme-size={}", size));

    fs::write(path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_qt_config(theme: &str, size: u32) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("No home directory")?;
    let kdeglobals = home.join(".config").join("kdeglobals");
    fs::create_dir_all(kdeglobals.parent().ok_or("Invalid KDE config path")?)
        .map_err(|e| e.to_string())?;

    update_ini_setting(
        &kdeglobals,
        "Mouse",
        &[("cursorTheme", theme), ("cursorSize", &size.to_string())],
    )
}

fn apply_environment_d(theme: &str, size: u32) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("No home directory")?;
    let env_dir = home.join(".config").join("environment.d");
    fs::create_dir_all(&env_dir).map_err(|e| e.to_string())?;

    let env_file = env_dir.join("10-cursor.conf");
    let quoted_theme = quote_environment_value(theme);
    let content = format!(
        "# Generated by mouse-me cursor manager\n\
         XCURSOR_THEME={theme}\n\
         XCURSOR_SIZE={size}\n\
         HYPRCURSOR_THEME={theme}\n\
         HYPRCURSOR_SIZE={size}\n",
        theme = quoted_theme,
        size = size
    );

    fs::write(&env_file, content).map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_xresources(theme: &str, size: u32) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("No home directory")?;
    let xresources_file = home.join(".Xresources");

    let mut lines = Vec::new();
    if xresources_file.exists() {
        let file = File::open(&xresources_file).map_err(|e| e.to_string())?;
        let reader = BufReader::new(file);
        for line_res in reader.lines() {
            let line = line_res.map_err(|e| e.to_string())?.trim().to_string();
            if !line.starts_with("Xcursor.theme:") && !line.starts_with("Xcursor.size:") {
                lines.push(line);
            }
        }
    }

    lines.push(format!("Xcursor.theme: {}", theme));
    lines.push(format!("Xcursor.size: {}", size));

    fs::write(&xresources_file, lines.join("\n") + "\n").map_err(|e| e.to_string())?;

    // Refresh X resources database if xrdb exists.
    let output = Command::new("xrdb")
        .args(["-merge"])
        .arg(&xresources_file)
        .output()
        .map_err(|e| e.to_string())?;
    checked_output(output, "xrdb -merge")
}

fn apply_default_index_theme(theme: &str) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("No home directory")?;

    let paths = [
        home.join(".icons").join("default").join("index.theme"),
        home.join(".local")
            .join("share")
            .join("icons")
            .join("default")
            .join("index.theme"),
    ];

    let content = format!(
        "[Icon Theme]\nName=Default\nComment=Default Cursor Theme\nInherits={}\n",
        theme
    );

    for p in &paths {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(p, &content).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn apply_flatpak_overrides() -> Result<(), String> {
    let output = Command::new("flatpak")
        .args([
            "override",
            "--user",
            "--filesystem=xdg-data/icons:ro",
            "--filesystem=~/.icons:ro",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    checked_output(output, "flatpak override")
}
