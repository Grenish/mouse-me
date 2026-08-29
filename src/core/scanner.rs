use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::types::{ActiveCursorState, CursorTheme, CursorType};
use crate::core::xcursor::parse_xcursor_file;

/// Returns standard icon search directories in order of priority.
pub fn get_icon_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = dirs::home_dir() {
        // ~/.local/share/icons
        let local_icons = home.join(".local").join("share").join("icons");
        if local_icons.exists() {
            dirs.push(local_icons);
        }

        // ~/.icons (legacy, but widely used)
        let home_icons = home.join(".icons");
        if home_icons.exists() {
            dirs.push(home_icons);
        }
    }

    // XDG_DATA_DIRS
    if let Ok(xdg_data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for p in xdg_data_dirs.split(':') {
            let icons_path = Path::new(p).join("icons");
            if icons_path.exists() && !dirs.contains(&icons_path) {
                dirs.push(icons_path);
            }
        }
    }

    // System directories
    let sys1 = PathBuf::from("/usr/local/share/icons");
    if sys1.exists() && !dirs.contains(&sys1) {
        dirs.push(sys1);
    }

    let sys2 = PathBuf::from("/usr/share/icons");
    if sys2.exists() && !dirs.contains(&sys2) {
        dirs.push(sys2);
    }

    dirs
}

/// Discovers all available cursor themes installed on the system.
pub fn scan_cursor_themes() -> Vec<CursorTheme> {
    scan_cursor_themes_from(&get_icon_directories())
}

/// CLI/GUI library filter: scope, type, and case-insensitive search.
pub fn theme_matches(
    theme: &CursorTheme,
    user: bool,
    system: bool,
    type_filter: Option<&str>,
    search: &str,
) -> bool {
    let scope_ok = match (user, system) {
        (false, false) | (true, true) => true,
        (true, false) => theme.is_user,
        (false, true) => !theme.is_user,
    };
    if !scope_ok {
        return false;
    }

    if let Some(type_filter) = type_filter {
        if theme.cursor_type.to_string().to_lowercase() != type_filter.to_lowercase() {
            return false;
        }
    }

    let query = search.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    theme.name.to_lowercase().contains(&query)
        || theme.display_name.to_lowercase().contains(&query)
        || theme.comment.to_lowercase().contains(&query)
}

/// Discovers cursor themes under the given icon directories.
pub fn scan_cursor_themes_from(search_dirs: &[PathBuf]) -> Vec<CursorTheme> {
    let mut themes = Vec::new();
    let mut seen_names = HashSet::new();

    for dir in search_dirs {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Skip default virtual symlink folder
            if folder_name == "default" {
                continue;
            }

            // Deduplicate if already loaded from higher priority user directory
            if seen_names.contains(&folder_name) {
                continue;
            }

            if let Some(theme) = inspect_cursor_directory(&path, &folder_name) {
                seen_names.insert(folder_name);
                themes.push(theme);
            }
        }
    }

    // Sort alphabetically by display name
    themes.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    themes
}

/// Inspects a directory to see if it's a valid cursor theme.
fn inspect_cursor_directory(path: &Path, folder_name: &str) -> Option<CursorTheme> {
    let cursors_dir = path.join("cursors");
    let hyprcursors_dir = path.join("hyprcursors");
    let manifest_hl = path.join("manifest.hl");

    let has_xcursor = cursors_dir.is_dir();
    let has_hyprcursor = hyprcursors_dir.is_dir() || manifest_hl.is_file();

    if !has_xcursor && !has_hyprcursor {
        return None;
    }

    let cursor_type = if has_hyprcursor {
        CursorType::Hyprcursor
    } else {
        CursorType::XCursor
    };

    let (display_name, comment) = parse_theme_metadata(path, folder_name);

    let is_user = if let Some(home) = dirs::home_dir() {
        path.starts_with(home)
    } else {
        false
    };

    // Load previews
    let preview_default =
        find_cursor_preview(path, &["default", "left_ptr", "arrow", "top_left_arrow"]);
    let preview_pointer = find_cursor_preview(
        path,
        &["pointer", "pointing_hand", "hand2", "hand1", "hand"],
    );
    let preview_wait =
        find_cursor_preview(path, &["wait", "watch", "progress", "half-busy", "circle"]);
    let preview_text = find_cursor_preview(path, &["text", "xterm", "ibeam"]);

    Some(CursorTheme {
        name: folder_name.to_string(),
        display_name,
        comment,
        cursor_type,
        path: path.to_path_buf(),
        is_user,
        preview_default,
        preview_pointer,
        preview_wait,
        preview_text,
    })
}

/// Extracts metadata (Name, Comment) from index.theme or manifest.hl
fn parse_theme_metadata(path: &Path, fallback_name: &str) -> (String, String) {
    let mut display_name = fallback_name.to_string();
    let mut comment = String::new();

    // Check index.theme
    let index_path = path.join("index.theme");
    if let Ok(file) = File::open(&index_path) {
        let reader = BufReader::new(file);
        let mut in_icon_theme_section = false;

        for line_res in reader.lines() {
            let line = match line_res {
                Ok(l) => l.trim().to_string(),
                Err(_) => break,
            };

            if line.starts_with('[') && line.ends_with(']') {
                in_icon_theme_section = line.eq_ignore_ascii_case("[Icon Theme]")
                    || line.eq_ignore_ascii_case("[cursor.theme]");
                continue;
            }

            if in_icon_theme_section {
                if let Some(val) = line.strip_prefix("Name=") {
                    let v = val.trim();
                    if !v.is_empty() {
                        display_name = v.to_string();
                    }
                } else if let Some(val) = line.strip_prefix("Comment=") {
                    comment = val.trim().to_string();
                }
            }
        }
    }

    // Check cursor.theme if comment/display_name wasn't found
    if display_name == fallback_name {
        let cursor_theme_path = path.join("cursor.theme");
        if let Ok(file) = File::open(&cursor_theme_path) {
            let reader = BufReader::new(file);
            for line_res in reader.lines().flatten() {
                let line = line_res.trim();
                if let Some(val) = line.strip_prefix("Name=") {
                    let v = val.trim();
                    if !v.is_empty() {
                        display_name = v.to_string();
                    }
                }
            }
        }
    }

    (display_name, comment)
}

/// Looks for a cursor file matching candidates and parses its image.
fn find_cursor_preview<const N: usize>(
    theme_path: &Path,
    candidates: &[&str; N],
) -> Option<crate::core::types::CursorImage> {
    let cursors_dir = theme_path.join("cursors");
    if cursors_dir.is_dir() {
        for candidate in candidates {
            let file_path = cursors_dir.join(candidate);
            if file_path.exists() {
                if let Some(img) = parse_xcursor_file(&file_path, 24) {
                    return Some(img.centered_glyph());
                }
            }
        }
    }

    // Fallback: check hyprcursors directory for PNG images
    let hypr_dir = theme_path.join("hyprcursors");
    if hypr_dir.is_dir() {
        for candidate in candidates {
            let cand_dir = hypr_dir.join(candidate);
            if cand_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&cand_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                            if ext.eq_ignore_ascii_case("png") {
                                if let Ok(img) = crate::core::images::load_bounded_rgba(&p) {
                                    return Some(img.centered_glyph());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Detects the currently active cursor theme and size on the system.
pub fn get_active_cursor() -> ActiveCursorState {
    let mut theme_name = String::new();
    let mut size = 24u32;

    // 1. Try GSettings
    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-theme"])
        .output()
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let s = raw
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .unwrap_or(&raw);
            if !s.is_empty() {
                theme_name = s.to_string();
            }
        }
    }

    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-size"])
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(sz) = s.parse::<u32>() {
                if sz > 0 {
                    size = sz;
                }
            }
        }
    }

    // 2. Try GTK-3 settings.ini if gsettings empty
    if theme_name.is_empty() {
        if let Some(home) = dirs::home_dir() {
            let gtk3_ini = home.join(".config").join("gtk-3.0").join("settings.ini");
            if let Ok(file) = File::open(&gtk3_ini) {
                let reader = BufReader::new(file);
                for line_res in reader.lines().flatten() {
                    let line = line_res.trim();
                    if let Some(val) = line.strip_prefix("gtk-cursor-theme-name=") {
                        theme_name = val.trim().to_string();
                    } else if let Some(val) = line.strip_prefix("gtk-cursor-theme-size=") {
                        if let Ok(sz) = val.trim().parse::<u32>() {
                            size = sz;
                        }
                    }
                }
            }
        }
    }

    // 3. Try Environment Variables
    if theme_name.is_empty() {
        if let Ok(env_theme) = std::env::var("HYPRCURSOR_THEME") {
            if !env_theme.is_empty() {
                theme_name = env_theme;
            }
        }
        if theme_name.is_empty() {
            if let Ok(env_theme) = std::env::var("XCURSOR_THEME") {
                if !env_theme.is_empty() {
                    theme_name = env_theme;
                }
            }
        }
    }

    if let Ok(env_sz) = std::env::var("XCURSOR_SIZE") {
        if let Ok(sz) = env_sz.parse::<u32>() {
            if sz > 0 {
                size = sz;
            }
        }
    }

    // Fallback default
    if theme_name.is_empty() {
        theme_name = "default".to_string();
    }

    ActiveCursorState { theme_name, size }
}
