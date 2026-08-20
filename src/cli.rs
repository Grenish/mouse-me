use clap::{Parser, Subcommand};
use serde::Serialize;
use std::path::PathBuf;

use mouse_me::core::applier::apply_system_wide;
use mouse_me::core::importer::{import_cursor_pack, is_safe_theme_name};
use mouse_me::core::scanner::{get_active_cursor, scan_cursor_themes};

#[derive(Parser)]
#[command(
    name = "mouse-me",
    about = "Universal Linux cursor manager with Slint GUI & fast CLI",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Launch GUI directly
    #[arg(short, long)]
    pub gui: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List all installed cursor themes
    List {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Set system-wide cursor theme and size
    Set {
        /// Name of the cursor theme
        theme: String,
        /// Cursor size in pixels (e.g. 16, 24, 32, 48, 64)
        #[arg(default_value_t = 24)]
        size: u32,
    },
    /// Get current active cursor theme and size
    Get {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Import a custom cursor pack (.zip, .tar.gz, .tar.xz, folder)
    Add {
        /// Path to cursor archive or folder
        path: PathBuf,
    },
    /// Remove a user-installed cursor theme
    Remove {
        /// Name of the cursor theme to delete
        theme: String,
    },
    /// Launch the interactive GUI
    Gui,
}

#[derive(Serialize)]
struct JsonThemeItem {
    name: String,
    display_name: String,
    cursor_type: String,
    is_active: bool,
    is_user: bool,
    path: String,
}

#[derive(Serialize)]
struct JsonActiveItem {
    theme: String,
    size: u32,
}

pub fn handle_cli(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Some(Commands::List { json }) => {
            let active = get_active_cursor();
            let themes = scan_cursor_themes();

            if json {
                let items: Vec<JsonThemeItem> = themes
                    .iter()
                    .map(|t| JsonThemeItem {
                        name: t.name.clone(),
                        display_name: t.display_name.clone(),
                        cursor_type: t.cursor_type.to_string(),
                        is_active: t.name.eq_ignore_ascii_case(&active.theme_name),
                        is_user: t.is_user,
                        path: t.path.to_string_lossy().to_string(),
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                println!("\n  🐭 Installed Cursor Themes ({})", themes.len());
                println!("  ─────────────────────────────────────────────────────────");
                for t in &themes {
                    let is_active = t.name.eq_ignore_ascii_case(&active.theme_name);
                    let marker = if is_active { "● [ACTIVE]" } else { " " };
                    let tag = format!("[{}]", t.cursor_type);
                    let scope = if t.is_user { "(user)" } else { "(system)" };

                    println!(
                        "  {:10} {:<24} {:<12} {:<8} {}",
                        marker,
                        t.name,
                        tag,
                        scope,
                        t.path.display()
                    );
                }
                println!("  ─────────────────────────────────────────────────────────");
                println!(
                    "  Current active: {} ({}px)\n",
                    active.theme_name, active.size
                );
            }
        }
        Some(Commands::Set { theme, size }) => {
            println!("Applying cursor '{}' ({}px) system-wide...", theme, size);
            let warnings = apply_system_wide(&theme, size)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            println!(
                "✓ Successfully applied '{}' ({}px) to Hyprland, GTK, Qt, and X11!",
                theme, size
            );
            for warning in warnings {
                eprintln!("  warning: {warning}");
            }
        }
        Some(Commands::Get { json }) => {
            let active = get_active_cursor();
            if json {
                let item = JsonActiveItem {
                    theme: active.theme_name,
                    size: active.size,
                };
                println!("{}", serde_json::to_string_pretty(&item)?);
            } else {
                println!("Theme: {}", active.theme_name);
                println!("Size:  {}px", active.size);
            }
        }
        Some(Commands::Add { path }) => {
            println!("Importing cursor pack from '{}'...", path.display());
            match import_cursor_pack(&path) {
                Ok(imported) => {
                    println!("✓ Successfully installed cursor theme(s):");
                    for name in imported {
                        println!("  - {}", name);
                    }
                    println!("\nYou can now apply it using: mouse-me set <name>");
                }
                Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::Other, e).into()),
            }
        }
        Some(Commands::Remove { theme }) => {
            let theme_name = theme.trim();
            if theme_name != theme || !is_safe_theme_name(theme_name) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Theme name must be a single safe directory name",
                )
                .into());
            }

            let home = dirs::home_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not locate home directory",
                )
            })?;
            let paths = [
                home.join(".local")
                    .join("share")
                    .join("icons")
                    .join(theme_name),
                home.join(".icons").join(theme_name),
            ];
            let mut found = false;
            let mut errors = Vec::new();

            for path in paths {
                if std::fs::symlink_metadata(&path).is_ok() {
                    found = true;
                    if let Err(error) = std::fs::remove_dir_all(&path) {
                        errors.push(format!("{}: {}", path.display(), error));
                    }
                }
            }

            if !errors.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "Could not remove theme '{}': {}",
                        theme_name,
                        errors.join(", ")
                    ),
                )
                .into());
            }
            if !found {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "Could not find user theme '{}' to remove (system themes cannot be deleted without root)",
                        theme_name
                    ),
                )
                .into());
            }
            println!("✓ Removed cursor theme '{}'", theme_name);
        }
        Some(Commands::Gui) => {
            crate::gui::run_gui()?;
        }
        None => {
            // Default: launch GUI if no subcommands passed
            crate::gui::run_gui()?;
        }
    }

    Ok(())
}
