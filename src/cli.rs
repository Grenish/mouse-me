use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use mouse_me::core::applier::{apply_hypr_cursor_prefs, apply_system_wide, apply_with_targets};
use mouse_me::core::auth::{format_joined, format_published, AuthStore};
use mouse_me::core::catalog::{
    download_and_import, list_packs, looks_like_filesystem_source, pack_spec, resolve_pack,
};
use mouse_me::core::device_info::{collect_device_info, copy_to_clipboard};
use mouse_me::core::importer::{import_cursor_pack, is_safe_theme_name};
use mouse_me::core::scanner::{get_active_cursor, scan_cursor_themes, theme_matches};
use mouse_me::core::settings::{AppSettings, SettingEffect};
use mouse_me::core::updater;

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
    /// List installed cursor themes
    List {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
        /// Only user-installed themes
        #[arg(long)]
        user: bool,
        /// Only system themes
        #[arg(long)]
        system: bool,
        /// Filter by cursor type
        #[arg(long = "type", value_enum, value_name = "TYPE")]
        cursor_type: Option<TypeFilter>,
        /// Case-insensitive search of name, display name, and comment
        #[arg(short, long)]
        search: Option<String>,
    },
    /// Apply a cursor theme using saved Settings targets
    Set {
        /// Name of the cursor theme
        theme: String,
        /// Cursor size in pixels (defaults to the saved preferred size)
        size: Option<u32>,
        /// Write every apply target instead of the saved Settings selection
        #[arg(long)]
        all: bool,
    },
    /// Get the current active cursor theme and size
    Get {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Install a catalog pack or import a local archive
    Add {
        /// Pack name, slug, owner/name, catalog URL, or local archive path
        source: String,
        /// Apply the first imported theme using saved Settings
        #[arg(long)]
        apply: bool,
    },
    /// Remove a user-installed cursor theme
    Remove {
        /// Name of the cursor theme to delete
        theme: String,
    },
    /// Sign in to mouse-me-web
    #[command(visible_alias = "account")]
    Auth {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
        #[command(subcommand)]
        action: Option<AuthAction>,
    },
    /// Print the signed-in username
    Whoami {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Check GitHub for a newer Mouse Me build
    Update {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
        /// Download and install now
        #[arg(long, conflicts_with = "stage")]
        install: bool,
        /// Download for the next launch
        #[arg(long, conflicts_with = "install")]
        stage: bool,
    },
    /// Print diagnostics for this machine
    Doctor {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
        /// Copy the full report to the clipboard
        #[arg(long)]
        copy: bool,
    },
    /// Show or change saved Settings
    Settings {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
    /// Launch the interactive GUI
    Gui,
}

#[derive(Subcommand)]
pub enum AuthAction {
    /// Sign in with email and password
    #[command(visible_aliases = ["signin", "sign-in"])]
    Login {
        #[arg(short, long)]
        email: Option<String>,
        /// Prefer a prompt or MOUSE_ME_PASSWORD
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Create a mouse-me-web account
    #[command(visible_alias = "sign-up")]
    Signup {
        #[arg(long)]
        name: Option<String>,
        #[arg(short, long)]
        username: Option<String>,
        #[arg(short, long)]
        email: Option<String>,
        /// Prefer a prompt or MOUSE_ME_PASSWORD
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Sign out and clear the local session
    #[command(visible_aliases = ["signout", "sign-out"])]
    Logout,
    /// Refresh the saved session and profile
    Refresh,
}

#[derive(Subcommand)]
pub enum SettingsAction {
    /// Print one setting
    Get { key: String },
    /// Change one setting
    Set { key: String, value: String },
    /// Apply saved Hyprland cursor preferences
    ApplyHypr,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum TypeFilter {
    Xcursor,
    Hyprcursor,
}

impl TypeFilter {
    fn as_filter_str(self) -> &'static str {
        match self {
            TypeFilter::Xcursor => "xcursor",
            TypeFilter::Hyprcursor => "hyprcursor",
        }
    }
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

#[derive(Serialize)]
struct JsonAccount {
    signed_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    joined: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_url: Option<String>,
}

#[derive(Serialize)]
struct JsonDoctor {
    version: String,
    os: String,
    kernel: String,
    desktop: String,
    session: String,
    cursor: String,
    gtk: String,
    qt: String,
    env_vars: String,
    full_report: String,
}

#[derive(Serialize)]
struct JsonUpdate {
    installed: String,
    latest: String,
    available: bool,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
}

pub fn handle_cli(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(error) = updater::apply_pending_update() {
        eprintln!("mouse-me: pending update failed: {error}");
    }

    match cli.command {
        Some(Commands::List {
            json,
            user,
            system,
            cursor_type,
            search,
        }) => cmd_list(json, user, system, cursor_type, search)?,
        Some(Commands::Set { theme, size, all }) => cmd_set(&theme, size, all)?,
        Some(Commands::Get { json }) => cmd_get(json)?,
        Some(Commands::Add { source, apply }) => cmd_add(&source, apply)?,
        Some(Commands::Remove { theme }) => cmd_remove(&theme)?,
        Some(Commands::Auth { json, action }) => cmd_auth(json, action)?,
        Some(Commands::Whoami { json }) => cmd_whoami(json)?,
        Some(Commands::Update {
            json,
            install,
            stage,
        }) => cmd_update(json, install, stage)?,
        Some(Commands::Doctor { json, copy }) => cmd_doctor(json, copy)?,
        Some(Commands::Settings { json, action }) => cmd_settings(json, action)?,
        Some(Commands::Gui) | None => crate::gui::run_gui()?,
    }

    Ok(())
}

fn cmd_list(
    json: bool,
    user: bool,
    system: bool,
    cursor_type: Option<TypeFilter>,
    search: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let active = get_active_cursor();
    let themes = scan_cursor_themes();
    let type_filter = cursor_type.map(TypeFilter::as_filter_str);
    let search = search.unwrap_or_default();
    let filtered: Vec<_> = themes
        .iter()
        .filter(|theme| theme_matches(theme, user, system, type_filter, &search))
        .collect();

    if json {
        let items: Vec<JsonThemeItem> = filtered
            .iter()
            .map(|theme| JsonThemeItem {
                name: theme.name.clone(),
                display_name: theme.display_name.clone(),
                cursor_type: theme.cursor_type.to_string(),
                is_active: theme.name.eq_ignore_ascii_case(&active.theme_name),
                is_user: theme.is_user,
                path: theme.path.to_string_lossy().to_string(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    println!("Installed cursor themes ({})", filtered.len());
    if filtered.is_empty() {
        println!("No cursor themes matched.");
        return Ok(());
    }

    for theme in &filtered {
        let is_active = theme.name.eq_ignore_ascii_case(&active.theme_name);
        let marker = if is_active { "*" } else { " " };
        let scope = if theme.is_user { "user" } else { "system" };
        println!(
            "  {} {:<24} {:<12} {:<8} {}",
            marker,
            theme.name,
            theme.cursor_type,
            scope,
            theme.path.display()
        );
    }
    println!("Active: {} ({}px)", active.theme_name, active.size);
    Ok(())
}

fn cmd_set(theme: &str, size: Option<u32>, all: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut settings = AppSettings::load();
    let size = size.unwrap_or(settings.preferred_size);
    let warnings = if all {
        println!("Applying '{theme}' ({size}px) to every target...");
        apply_system_wide(theme, size).map_err(fail)?
    } else {
        println!("Applying '{theme}' ({size}px) using saved apply targets...");
        apply_with_targets(theme, size, &settings.apply_targets()).map_err(fail)?
    };

    settings.preferred_size = size;
    if let Err(error) = settings.save() {
        eprintln!("warning: could not save preferred size: {error}");
    }

    if all {
        println!("Applied '{theme}' ({size}px) to Hyprland, GTK, Qt, and X11.");
    } else {
        println!("Applied '{theme}' ({size}px).");
    }
    print_warnings(&warnings);
    Ok(())
}

fn cmd_get(json: bool) -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}

fn cmd_add(source: &str, apply: bool) -> Result<(), Box<dyn std::error::Error>> {
    let source = source.trim();
    if source.is_empty() {
        return Err(fail("Enter a pack name or a local archive path"));
    }

    let imported = if looks_like_filesystem_source(source) {
        let path = expand_home(source);
        println!("Importing cursor pack from '{}'...", path.display());
        import_cursor_pack(&path).map_err(fail)?
    } else {
        let store = AuthStore::load().map_err(fail)?;
        println!("Looking up '{source}' in the Mouse Me catalog...");
        let packs = list_packs(&store).map_err(fail)?;
        let pack = resolve_pack(&packs, source).map_err(fail)?.clone();
        println!("Downloading {} ({})...", pack.name, pack_spec(&pack));
        download_and_import(&store, &pack).map_err(fail)?
    };

    println!("Installed cursor theme(s):");
    for name in &imported {
        println!("  {name}");
    }

    let settings = AppSettings::load();
    let should_apply = apply || settings.auto_apply_on_import;
    if should_apply {
        if let Some(name) = imported.first() {
            let warnings =
                apply_with_targets(name, settings.preferred_size, &settings.apply_targets())
                    .map_err(fail)?;
            println!("Applied '{name}' ({}px).", settings.preferred_size);
            print_warnings(&warnings);
        }
    } else if let Some(name) = imported.first() {
        println!("Apply with: mouse-me set {name}");
    }
    Ok(())
}

fn cmd_remove(theme: &str) -> Result<(), Box<dyn std::error::Error>> {
    let theme_name = theme.trim();
    if theme_name != theme || !is_safe_theme_name(theme_name) {
        return Err(fail("Theme name must be a single safe directory name"));
    }

    let home = dirs::home_dir().ok_or_else(|| fail("Could not locate home directory"))?;
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
        if fs::symlink_metadata(&path).is_ok() {
            found = true;
            if let Err(error) = fs::remove_dir_all(&path) {
                errors.push(format!("{}: {}", path.display(), error));
            }
        }
    }

    if !errors.is_empty() {
        return Err(fail(format!(
            "Could not remove theme '{theme_name}': {}",
            errors.join(", ")
        )));
    }
    if !found {
        return Err(fail(format!(
            "Could not find user theme '{theme_name}' to remove (system themes cannot be deleted without root)"
        )));
    }
    println!("Removed cursor theme '{theme_name}'");
    Ok(())
}

fn cmd_auth(json: bool, action: Option<AuthAction>) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        None => print_account(json)?,
        Some(AuthAction::Login { email, password }) => {
            let email = require_text(email, "--email", "Email")?;
            let password = require_password(password)?;
            let mut store = AuthStore::load().map_err(fail)?;
            let user = store.sign_in(&email, &password).map_err(fail)?;
            println!(
                "Signed in as {} ({})",
                display_name(&user.name, &user.email),
                user.email
            );
        }
        Some(AuthAction::Signup {
            name,
            username,
            email,
            password,
        }) => {
            let name = require_text(name, "--name", "Name")?;
            let username = require_text(username, "--username", "Username")?;
            let email = require_text(email, "--email", "Email")?;
            let (password, confirm) = require_password_confirmed(password)?;
            let mut store = AuthStore::load().map_err(fail)?;
            let user = store
                .create_account(&name, &username, &email, &password, &confirm)
                .map_err(fail)?;
            println!(
                "Account created. Signed in as {} ({})",
                display_name(&user.name, &user.email),
                user.email
            );
        }
        Some(AuthAction::Logout) => {
            let mut store = AuthStore::load().map_err(fail)?;
            store.sign_out().map_err(fail)?;
            println!("Signed out.");
        }
        Some(AuthAction::Refresh) => {
            let mut store = AuthStore::load().map_err(fail)?;
            match store.refresh().map_err(fail)? {
                Some(user) => {
                    println!(
                        "Session refreshed. Signed in as {} ({})",
                        display_name(&user.name, &user.email),
                        user.email
                    );
                }
                None => println!("Not signed in."),
            }
        }
    }
    Ok(())
}

fn cmd_whoami(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut store = AuthStore::load().map_err(fail)?;
    let user = match store.refresh() {
        Ok(user) => user,
        Err(_) => store.user().cloned(),
    };
    let Some(user) = user else {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&JsonAccount {
                    signed_in: false,
                    name: None,
                    username: None,
                    email: None,
                    joined: None,
                    published: None,
                    profile_url: None,
                })?
            );
            return Ok(());
        }
        return Err(fail("Not signed in."));
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonAccount {
                signed_in: true,
                name: Some(user.name.clone()),
                username: Some(user.username.clone()),
                email: Some(user.email.clone()),
                joined: user.created_at.as_deref().map(format_joined),
                published: Some(format_published(user.published_count)),
                profile_url: store.profile_url(),
            })?
        );
        return Ok(());
    }

    let label = if !user.username.is_empty() {
        user.username
    } else if !user.email.is_empty() {
        user.email
    } else {
        user.name
    };
    println!("{label}");
    Ok(())
}

fn print_account(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut store = AuthStore::load().map_err(fail)?;
    let user = match store.refresh() {
        Ok(user) => user,
        Err(_) => store.user().cloned(),
    };

    if json {
        let payload = match user {
            Some(user) => JsonAccount {
                signed_in: true,
                name: Some(user.name.clone()),
                username: Some(user.username.clone()),
                email: Some(user.email.clone()),
                joined: user.created_at.as_deref().map(format_joined),
                published: Some(format_published(user.published_count)),
                profile_url: store.profile_url(),
            },
            None => JsonAccount {
                signed_in: false,
                name: None,
                username: None,
                email: None,
                joined: None,
                published: None,
                profile_url: None,
            },
        };
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let Some(user) = user else {
        println!("Not signed in.");
        return Ok(());
    };

    println!("Signed in");
    print_kv("Name", &user.name);
    print_kv("Username", &user.username);
    print_kv("Email", &user.email);
    if let Some(joined) = user.created_at.as_deref() {
        print_kv("Joined", &format_joined(joined));
    }
    print_kv("Published", &format_published(user.published_count));
    if let Some(url) = store.profile_url() {
        print_kv("Profile", &url);
    }
    Ok(())
}

fn cmd_update(json: bool, install: bool, stage: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current = updater::current_version();
    let release = updater::latest_release().map_err(fail)?;
    let available = updater::is_newer(&release.version, current);

    let mut action = None;
    let mut installed_path = None;
    if available && install {
        let path = updater::install_update(&release).map_err(fail)?;
        installed_path = Some(path);
        action = Some("install".to_string());
    } else if available && stage {
        updater::stage_update(&release).map_err(fail)?;
        action = Some("stage".to_string());
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonUpdate {
                installed: current.to_string(),
                latest: release.version,
                available,
                tag: release.tag,
                action,
            })?
        );
        return Ok(());
    }

    println!("Installed: {current}");
    println!("Latest:    {}", release.version);
    if !available {
        println!("You're on {current}. That's the latest published build.");
        return Ok(());
    }
    match action.as_deref() {
        Some("install") => {
            if let Some(path) = installed_path {
                println!("Installed {} to {}.", release.tag, path.display());
            }
            println!("Restart Mouse Me to use it.");
        }
        Some("stage") => {
            println!(
                "{} will install the next time you open Mouse Me.",
                release.tag
            );
        }
        _ => {
            println!("Update available: {}", release.tag);
            println!(
                "Run `mouse-me update --install` to apply it now, or `--stage` for the next launch."
            );
        }
    }
    Ok(())
}

fn cmd_doctor(json: bool, copy: bool) -> Result<(), Box<dyn std::error::Error>> {
    let info = collect_device_info();
    let version = env!("CARGO_PKG_VERSION").to_string();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonDoctor {
                version: version.clone(),
                os: info.os.clone(),
                kernel: info.kernel.clone(),
                desktop: info.desktop.clone(),
                session: info.session.clone(),
                cursor: info.cursor.clone(),
                gtk: info.gtk.clone(),
                qt: info.qt.clone(),
                env_vars: info.env_vars.clone(),
                full_report: info.full_report.clone(),
            })?
        );
    } else {
        println!("Mouse Me {version}");
        println!();
        println!("Session");
        print_kv("OS", &info.os);
        print_kv("Kernel", &info.kernel);
        print_kv("Desktop", &info.desktop);
        print_kv("Session", &info.session);
        println!();
        println!("Pointer");
        print_kv("Active", &info.cursor);
        print_kv("GTK", &info.gtk);
        print_kv("Qt", &info.qt);
        print_kv("Environment", &info.env_vars);
    }

    if copy {
        copy_to_clipboard(&info.full_report).map_err(fail)?;
        eprintln!("Copied diagnostics to the clipboard.");
    }
    Ok(())
}

fn cmd_settings(
    json: bool,
    action: Option<SettingsAction>,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        None => {
            let settings = AppSettings::load();
            if json {
                println!("{}", serde_json::to_string_pretty(&settings)?);
                return Ok(());
            }
            println!("Apply");
            for key in [
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
            ] {
                print_setting(&settings, key)?;
            }
            println!("Library");
            for key in [
                "show-user-themes",
                "show-system-themes",
                "library-filter",
                "library-type",
                "preferred-size",
            ] {
                print_setting(&settings, key)?;
            }
            println!("Hyprland");
            for key in [
                "enable-hyprcursor",
                "hide-on-key-press",
                "hide-on-touch",
                "no-hardware-cursors",
                "inactive-timeout",
            ] {
                print_setting(&settings, key)?;
            }
            println!("Updates");
            for key in ["auto-update", "auto-update-when"] {
                print_setting(&settings, key)?;
            }
        }
        Some(SettingsAction::Get { key }) => {
            let settings = AppSettings::load();
            println!("{}", settings.get_key(&key).map_err(fail)?);
        }
        Some(SettingsAction::Set { key, value }) => {
            let mut settings = AppSettings::load();
            let effect = settings.set_key(&key, &value).map_err(fail)?;
            settings.save().map_err(fail)?;
            println!(
                "{} = {}",
                normalize_printed_key(&key),
                settings.get_key(&key).map_err(fail)?
            );
            apply_setting_effect(&settings, effect)?;
        }
        Some(SettingsAction::ApplyHypr) => {
            let settings = AppSettings::load();
            apply_hypr_from_settings(&settings)?;
            println!("Hyprland cursor preferences applied.");
        }
    }
    Ok(())
}

fn apply_setting_effect(
    settings: &AppSettings,
    effect: SettingEffect,
) -> Result<(), Box<dyn std::error::Error>> {
    match effect {
        SettingEffect::None => Ok(()),
        SettingEffect::HyprPrefs => apply_hypr_from_settings(settings),
        SettingEffect::PreferredSize => {
            if !settings.apply_size_immediately {
                return Ok(());
            }
            let active = get_active_cursor();
            if active.theme_name.is_empty() || active.theme_name == "default" {
                return Ok(());
            }
            let warnings = apply_with_targets(
                &active.theme_name,
                settings.preferred_size,
                &settings.apply_targets(),
            )
            .map_err(fail)?;
            println!(
                "Applied '{}' ({}px).",
                active.theme_name, settings.preferred_size
            );
            print_warnings(&warnings);
            Ok(())
        }
    }
}

fn apply_hypr_from_settings(settings: &AppSettings) -> Result<(), Box<dyn std::error::Error>> {
    apply_hypr_cursor_prefs(
        settings.enable_hyprcursor,
        settings.hide_on_key_press,
        settings.hide_on_touch,
        settings.no_hardware_cursors,
        settings.inactive_timeout,
    )
    .map_err(fail)
}

fn print_setting(settings: &AppSettings, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let value = settings.get_key(key).map_err(fail)?;
    println!("  {key:<24} {value}");
    Ok(())
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("  warning: {warning}");
    }
}

fn print_kv(label: &str, value: &str) {
    println!("  {label:<12} {value}");
}

fn display_name(name: &str, email: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        email.to_string()
    } else {
        name.to_string()
    }
}

fn normalize_printed_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace('_', "-")
}

fn require_text(
    value: Option<String>,
    flag: &str,
    label: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(value) = value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if io::stdin().is_terminal() {
        let entered = prompt(label, false)?;
        if entered.is_empty() {
            return Err(fail(format!("{flag} is required")));
        }
        return Ok(entered);
    }
    Err(fail(format!("Pass {flag} (no TTY to prompt)")))
}

fn require_password(explicit: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(value) = explicit {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    if let Ok(value) = std::env::var("MOUSE_ME_PASSWORD") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    if io::stdin().is_terminal() {
        let entered = prompt("Password", true)?;
        if entered.is_empty() {
            return Err(fail("Password is required"));
        }
        return Ok(entered);
    }
    let entered = read_stdin_line()?;
    if entered.is_empty() {
        return Err(fail("Password is required"));
    }
    Ok(entered)
}

fn require_password_confirmed(
    explicit: Option<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let interactive = explicit
        .as_deref()
        .map(|value| value.is_empty())
        .unwrap_or(true)
        && std::env::var("MOUSE_ME_PASSWORD")
            .map(|value| value.is_empty())
            .unwrap_or(true)
        && io::stdin().is_terminal();
    let password = require_password(explicit)?;
    if interactive {
        let confirm = prompt("Confirm password", true)?;
        if confirm != password {
            return Err(fail("Passwords do not match."));
        }
        return Ok((password, confirm));
    }
    let confirm = password.clone();
    Ok((password, confirm))
}

fn prompt(label: &str, secret: bool) -> Result<String, Box<dyn std::error::Error>> {
    let mut stderr = io::stderr();
    write!(stderr, "{label}: ")?;
    stderr.flush()?;

    if secret {
        stty_echo(false)?;
        let _restore = EchoRestore;
        let line = read_tty_line().or_else(|_| read_stdin_line())?;
        drop(_restore);
        writeln!(stderr)?;
        return Ok(line);
    }

    read_tty_line()
        .or_else(|_| read_stdin_line())
        .map_err(|error| error.into())
}

struct EchoRestore;

impl Drop for EchoRestore {
    fn drop(&mut self) {
        let _ = stty_echo(true);
    }
}

fn stty_echo(enable: bool) -> io::Result<()> {
    let tty = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    let arg = if enable { "echo" } else { "-echo" };
    let status = Command::new("stty")
        .arg(arg)
        .stdin(tty)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("Could not change terminal echo"))
    }
}

fn read_tty_line() -> io::Result<String> {
    let tty = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    let mut reader = io::BufReader::new(tty);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(trim_line(line))
}

fn read_stdin_line() -> io::Result<String> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(trim_line(line))
}

fn trim_line(line: String) -> String {
    line.trim_end_matches(['\n', '\r']).to_string()
}

#[derive(Debug)]
struct CliError(String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

fn expand_home(source: &str) -> PathBuf {
    if let Some(rest) = source.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(source)
}

fn fail(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(CliError(message.into()))
}
