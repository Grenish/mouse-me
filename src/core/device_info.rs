use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crate::core::scanner::get_active_cursor;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub os: String,
    pub kernel: String,
    pub desktop: String,
    pub session: String,
    pub cursor: String,
    pub gtk: String,
    pub qt: String,
    pub env_vars: String,
    pub full_report: String,
}

pub fn collect_device_info() -> DeviceInfo {
    let os = get_os_pretty_name();
    let kernel_full = get_kernel_info();
    let kernel = shorten_kernel(&kernel_full);
    let desktop_full = get_desktop_environment();
    let desktop = shorten_desktop(&desktop_full);
    let session = get_session_info();
    let active_cursor = get_active_cursor();
    let cursor = format!("{} ({}px)", active_cursor.theme_name, active_cursor.size);
    let gtk = get_gtk_cursor_setting();
    let qt = get_qt_cursor_setting();
    let env_vars = get_cursor_env_vars();

    let app_version = env!("CARGO_PKG_VERSION");

    let full_report = format!(
        "### Mouse Me Debug & Device Information\n\
         - **App Version:** v{}\n\
         - **OS / Distro:** {}\n\
         - **Kernel & Arch:** {}\n\
         - **Desktop Environment:** {}\n\
         - **Session Type:** {}\n\
         - **Active Cursor:** {}\n\
         - **GTK / GSettings:** {}\n\
         - **Qt / KDE Config:** {}\n\
         - **Environment Variables:** {}\n",
        app_version, os, kernel_full, desktop_full, session, cursor, gtk, qt, env_vars
    );

    DeviceInfo {
        os,
        kernel,
        desktop,
        session,
        cursor,
        gtk,
        qt,
        env_vars,
        full_report,
    }
}

fn get_os_pretty_name() -> String {
    if let Ok(file) = File::open("/etc/os-release") {
        let reader = BufReader::new(file);
        let mut pretty_name = String::new();
        let mut name = String::new();
        let mut version = String::new();

        for line_res in reader.lines().flatten() {
            let line = line_res.trim();
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                pretty_name = val.trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(val) = line.strip_prefix("NAME=") {
                name = val.trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(val) = line.strip_prefix("VERSION_ID=") {
                version = val.trim_matches('"').trim_matches('\'').to_string();
            }
        }

        if !pretty_name.is_empty() {
            return pretty_name;
        }
        if !name.is_empty() {
            if !version.is_empty() {
                return format!("{} {}", name, version);
            }
            return name;
        }
    }

    format!("Linux ({})", std::env::consts::OS)
}

fn get_kernel_info() -> String {
    if let Ok(output) = Command::new("uname").args(["-srm"]).output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    format!("Linux ({})", std::env::consts::ARCH)
}

fn get_desktop_environment() -> String {
    let xdg_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let session = std::env::var("DESKTOP_SESSION").unwrap_or_default();

    // Check for Hyprland version specifically if applicable
    if xdg_desktop.eq_ignore_ascii_case("hyprland")
        || session.eq_ignore_ascii_case("hyprland")
        || std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
    {
        if let Ok(output) = Command::new("hyprctl").args(["version"]).output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = s.lines().next() {
                    let trimmed = first_line.trim();
                    if trimmed.starts_with("Hyprland") {
                        return trimmed.to_string();
                    }
                }
            }
        }
        return "Hyprland".to_string();
    }

    if !xdg_desktop.is_empty() {
        if !session.is_empty() && !session.eq_ignore_ascii_case(&xdg_desktop) {
            return format!("{} ({})", xdg_desktop, session);
        }
        return xdg_desktop;
    }

    if !session.is_empty() {
        return session;
    }

    "Unknown Desktop".to_string()
}

fn get_session_info() -> String {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let x11_display = std::env::var("DISPLAY").unwrap_or_default();

    if !wayland_display.is_empty() {
        return format!("Wayland ({})", wayland_display);
    }

    if session_type.eq_ignore_ascii_case("wayland") {
        return "Wayland".to_string();
    }

    if !x11_display.is_empty() {
        return format!("X11 ({})", x11_display);
    }

    if !session_type.is_empty() {
        return session_type;
    }

    "Unknown".to_string()
}

fn get_gtk_cursor_setting() -> String {
    let mut gsettings_val = String::new();
    let mut gsettings_sz = String::new();

    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-theme"])
        .output()
    {
        if output.status.success() {
            gsettings_val = String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace('\'', "");
        }
    }

    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-size"])
        .output()
    {
        if output.status.success() {
            gsettings_sz = String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }

    if !gsettings_val.is_empty() {
        if !gsettings_sz.is_empty() {
            return format!("{} ({}px)", gsettings_val, gsettings_sz);
        }
        return gsettings_val;
    }

    // Try ~/.config/gtk-3.0/settings.ini
    if let Some(home) = dirs::home_dir() {
        let gtk3_ini = home.join(".config").join("gtk-3.0").join("settings.ini");
        if let Ok(file) = File::open(&gtk3_ini) {
            let reader = BufReader::new(file);
            for line_res in reader.lines().flatten() {
                let line = line_res.trim();
                if let Some(val) = line.strip_prefix("gtk-cursor-theme-name=") {
                    return val.trim().to_string();
                }
            }
        }
    }

    "Not set".to_string()
}

fn get_qt_cursor_setting() -> String {
    if let Some(home) = dirs::home_dir() {
        let kdeglobals = home.join(".config").join("kdeglobals");
        if let Ok(file) = File::open(&kdeglobals) {
            let reader = BufReader::new(file);
            let mut in_mouse = false;
            let mut theme = String::new();
            let mut size = String::new();

            for line_res in reader.lines().flatten() {
                let line = line_res.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    in_mouse = line.eq_ignore_ascii_case("[Mouse]");
                    continue;
                }
                if in_mouse {
                    if let Some(val) = line.strip_prefix("cursorTheme=") {
                        theme = val.trim().to_string();
                    } else if let Some(val) = line.strip_prefix("cursorSize=") {
                        size = val.trim().to_string();
                    }
                }
            }

            if !theme.is_empty() {
                if !size.is_empty() {
                    return format!("{} ({}px)", theme, size);
                }
                return theme;
            }
        }
    }

    "Not set".to_string()
}

fn get_cursor_env_vars() -> String {
    let xcursor = std::env::var("XCURSOR_THEME").unwrap_or_default();
    let hypr = std::env::var("HYPRCURSOR_THEME").unwrap_or_default();
    let size = std::env::var("XCURSOR_SIZE").unwrap_or_default();
    format_env_vars(&xcursor, &hypr, &size)
}

fn format_env_vars(xcursor: &str, hypr: &str, size: &str) -> String {
    let xcursor = xcursor.trim();
    let hypr = hypr.trim();
    let size = size.trim();
    if xcursor.is_empty() && hypr.is_empty() && size.is_empty() {
        return "Not set".into();
    }
    let size_part = if size.is_empty() {
        String::new()
    } else if size.ends_with("px") {
        format!(" · {size}")
    } else {
        format!(" · {size}px")
    };
    if !xcursor.is_empty() && (hypr.is_empty() || hypr == xcursor) {
        return format!("{xcursor}{size_part}");
    }
    if xcursor.is_empty() && !hypr.is_empty() {
        return format!("{hypr}{size_part}");
    }
    format!("XCURSOR {xcursor} · HYPRCURSOR {hypr}{size_part}")
}

fn shorten_desktop(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or(raw).trim();
    let mut parts = line.split_whitespace();
    let Some(name) = parts.next() else {
        return raw.to_string();
    };
    if name.eq_ignore_ascii_case("Hyprland") {
        if let Some(version) = parts.next() {
            if version.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                return format!("Hyprland {version}");
            }
        }
        return "Hyprland".into();
    }
    line.to_string()
}

fn shorten_kernel(raw: &str) -> String {
    let mut parts = raw.split_whitespace();
    let _sys = parts.next();
    let release = parts.next().unwrap_or("");
    let arch = parts.next().unwrap_or("");
    if !release.is_empty() && !arch.is_empty() {
        return format!("{release} · {arch}");
    }
    raw.to_string()
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // 1. Try wl-copy (standard Wayland clipboard)
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait() {
            if status.success() {
                return Ok(());
            }
        }
    }

    // 2. Try xclip (standard X11 / XWayland clipboard)
    if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait() {
            if status.success() {
                return Ok(());
            }
        }
    }

    // 3. Try xsel
    if let Ok(mut child) = Command::new("xsel")
        .args(["-b", "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait() {
            if status.success() {
                return Ok(());
            }
        }
    }

    Err("Clipboard tool not found (please install wl-copy or xclip)".to_string())
}

#[cfg(test)]
mod tests {
    use super::{format_env_vars, shorten_desktop, shorten_kernel};

    #[test]
    fn hyprland_version_drops_commit_noise() {
        let raw = "Hyprland 0.56.2 built from branch v0.56.2 at commit efb50993780079460b0cbed1363e2166a2de1d9f clean ([gha] Nix)";
        assert_eq!(shorten_desktop(raw), "Hyprland 0.56.2");
        assert_eq!(shorten_desktop("GNOME"), "GNOME");
    }

    #[test]
    fn kernel_line_keeps_release_and_arch() {
        assert_eq!(
            shorten_kernel("Linux 7.1.8-arch1-3 x86_64"),
            "7.1.8-arch1-3 · x86_64"
        );
    }

    #[test]
    fn env_line_collapses_matching_themes() {
        assert_eq!(
            format_env_vars("modest-light", "modest-light", "24"),
            "modest-light · 24px"
        );
        assert_eq!(
            format_env_vars("one", "two", "32"),
            "XCURSOR one · HYPRCURSOR two · 32px"
        );
        assert_eq!(format_env_vars("", "", ""), "Not set");
    }
}
