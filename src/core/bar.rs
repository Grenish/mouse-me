use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::importer::is_safe_theme_name;

const LIVE_ID: &str = "_live";
const BACKUP_DIR: &str = "_live-backup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarHost {
    OmarchyShell,
    WaybarLegacy,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct BarPaths {
    pub bars_dir: PathBuf,
    pub live_shell: PathBuf,
    pub default_shell: Option<PathBuf>,
    pub waybar_config: Option<PathBuf>,
    pub omarchy_on_path: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarManifest {
    pub id: String,
    pub display_name: String,
    pub created_at: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct BarProfileSummary {
    pub id: String,
    pub display_name: String,
    pub source: String,
    pub is_live: bool,
    pub is_applied: bool,
    pub is_deletable: bool,
    pub left: String,
    pub center: String,
    pub right: String,
}

impl BarPaths {
    pub fn from_env() -> Self {
        let config = dirs::config_dir();
        let data = dirs::data_local_dir();
        let live_shell = config
            .as_ref()
            .map(|dir| dir.join("omarchy").join("shell.json"))
            .unwrap_or_else(|| PathBuf::from("/tmp/omarchy-shell.json"));
        let default_shell = omarchy_default_shell();
        let waybar_config = config.as_ref().and_then(|dir| first_waybar_config(dir));
        let bars_dir = data
            .map(|dir| dir.join("mouse-me").join("bars"))
            .unwrap_or_else(|| PathBuf::from("/tmp/mouse-me-bars"));
        Self {
            bars_dir,
            live_shell,
            default_shell,
            waybar_config,
            omarchy_on_path: command_on_path("omarchy"),
        }
    }
}

pub fn detect_host(paths: &BarPaths) -> BarHost {
    let default_exists = paths
        .default_shell
        .as_ref()
        .map(|path| path.exists())
        .unwrap_or(false);
    if paths.live_shell.exists() || (paths.omarchy_on_path && default_exists) {
        return BarHost::OmarchyShell;
    }
    if paths
        .waybar_config
        .as_ref()
        .map(|path| path.exists())
        .unwrap_or(false)
    {
        return BarHost::WaybarLegacy;
    }
    BarHost::Unsupported
}

pub fn host_message(host: &BarHost) -> String {
    match host {
        BarHost::OmarchyShell => {
            "Omarchy menubar. Save drafts here; Apply writes ~/.config/omarchy/shell.json.".into()
        }
        BarHost::WaybarLegacy => "This machine still uses Waybar. Bar profiles target Omarchy's shell.json, so this page is read-only for now.".into(),
        BarHost::Unsupported => {
            "No Omarchy shell was found. Bar profiles are available on Omarchy Quattro.".into()
        }
    }
}

pub fn empty_bar() -> Value {
    json!({
        "position": "top",
        "transparent": false,
        "centerAnchor": "",
        "layout": {
            "left": [],
            "center": [],
            "right": []
        }
    })
}

pub fn live_bar(paths: &BarPaths) -> Result<Value, String> {
    if paths.live_shell.exists() {
        return bar_object(&read_json(&paths.live_shell)?);
    }
    if let Some(default) = &paths.default_shell {
        if default.exists() {
            return bar_object(&read_json(default)?);
        }
    }
    Ok(empty_bar())
}

pub fn list_profiles(paths: &BarPaths) -> Result<Vec<BarProfileSummary>, String> {
    fs::create_dir_all(&paths.bars_dir).map_err(|error| error.to_string())?;
    let live = live_bar(paths).unwrap_or_else(|_| empty_bar());
    let mut rows = vec![summary_from_bar(
        LIVE_ID, "Live", "live", true, true, false, &live,
    )];

    let mut saved = Vec::new();
    let entries = fs::read_dir(&paths.bars_dir).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('_') || !entry.path().is_dir() {
            continue;
        }
        match load_profile(paths, name.as_ref()) {
            Ok((manifest, bar)) => {
                let applied = bar == live;
                saved.push(summary_from_bar(
                    &manifest.id,
                    &manifest.display_name,
                    &manifest.source,
                    false,
                    applied,
                    true,
                    &bar,
                ));
            }
            Err(_) => continue,
        }
    }
    saved.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
    });
    rows.extend(saved);
    Ok(rows)
}

pub fn save_from_live(paths: &BarPaths, name: &str) -> Result<String, String> {
    require_omarchy(paths)?;
    let bar = live_bar(paths)?;
    write_profile(paths, name, bar, "live")
}

pub fn new_empty(paths: &BarPaths, name: &str) -> Result<String, String> {
    require_omarchy(paths)?;
    write_profile(paths, name, empty_bar(), "empty")
}

pub fn duplicate_profile(paths: &BarPaths, id: &str, name: &str) -> Result<String, String> {
    require_omarchy(paths)?;
    let (_, bar) = load_profile(paths, id)?;
    write_profile(paths, name, bar, "duplicate")
}

pub fn apply_profile(paths: &BarPaths, id: &str) -> Result<(), String> {
    require_omarchy(paths)?;
    if id == LIVE_ID {
        return Ok(());
    }
    let (_, bar) = load_profile(paths, id)?;
    apply_bar(paths, bar)
}

pub fn backup_exists(paths: &BarPaths) -> bool {
    backup_path(paths).exists()
}

pub fn restore_backup(paths: &BarPaths) -> Result<(), String> {
    require_omarchy(paths)?;
    let backup = backup_path(paths);
    if !backup.exists() {
        return Err("No previous bar to restore.".into());
    }
    let document = read_json(&backup)?;
    write_atomic(&paths.live_shell, &document)
}

pub fn remove_profile(paths: &BarPaths, id: &str) -> Result<(), String> {
    require_omarchy(paths)?;
    if id == LIVE_ID || id == BACKUP_DIR {
        return Err("That bar cannot be removed.".into());
    }
    let dir = profile_dir(paths, id);
    if !dir.exists() {
        return Err(format!("Bar profile '{id}' was not found"));
    }
    fs::remove_dir_all(&dir).map_err(|error| error.to_string())
}

fn apply_bar(paths: &BarPaths, bar: Value) -> Result<(), String> {
    if !bar.is_object() {
        return Err("Bar profile is missing a bar object.".into());
    }
    let mut document = if paths.live_shell.exists() {
        read_json(&paths.live_shell)?
    } else if let Some(default) = &paths.default_shell {
        if default.exists() {
            read_json(default)?
        } else {
            json!({ "version": 1 })
        }
    } else {
        json!({ "version": 1 })
    };
    if paths.live_shell.exists() {
        fs::create_dir_all(backup_dir(paths)).map_err(|error| error.to_string())?;
        fs::copy(&paths.live_shell, backup_path(paths)).map_err(|error| error.to_string())?;
    }
    document["bar"] = bar;
    if document.get("version").is_none() {
        document["version"] = json!(1);
    }
    write_atomic(&paths.live_shell, &document)
}

fn write_profile(paths: &BarPaths, name: &str, bar: Value, source: &str) -> Result<String, String> {
    let id = unique_profile_id(paths, name)?;
    let dir = profile_dir(paths, &id);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let manifest = BarManifest {
        id: id.clone(),
        display_name: name.trim().to_string(),
        created_at: now_stamp(),
        source: source.to_string(),
    };
    write_atomic(
        &dir.join("manifest.json"),
        &serde_json::to_value(&manifest).map_err(|e| e.to_string())?,
    )?;
    write_atomic(
        &dir.join("shell.json"),
        &json!({
            "version": 1,
            "bar": bar
        }),
    )?;
    Ok(id)
}

fn load_profile(paths: &BarPaths, id: &str) -> Result<(BarManifest, Value), String> {
    if !is_safe_theme_name(id) || id.starts_with('_') {
        return Err(format!("Bar profile '{id}' is not a safe name"));
    }
    let dir = profile_dir(paths, id);
    let manifest: BarManifest = serde_json::from_value(read_json(&dir.join("manifest.json"))?)
        .map_err(|error| format!("Could not read bar profile '{id}': {error}"))?;
    let document = read_json(&dir.join("shell.json"))?;
    Ok((manifest, bar_object(&document)?))
}

fn unique_profile_id(paths: &BarPaths, name: &str) -> Result<String, String> {
    let base = profile_id_from_name(name)?;
    if !profile_dir(paths, &base).exists() {
        return Ok(base);
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if !profile_dir(paths, &candidate).exists() {
            return Ok(candidate);
        }
    }
    Err("Could not find a free bar profile name".into())
}

fn profile_id_from_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.contains('/') || name.contains('\\') {
        return Err("Bar name is not a safe profile name".into());
    }
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else if character.is_whitespace() {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    if !is_safe_theme_name(&cleaned) {
        return Err("Bar name is not a safe profile name".into());
    }
    Ok(cleaned)
}

fn bar_object(document: &Value) -> Result<Value, String> {
    document
        .get("bar")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| "Shell config is missing a bar object.".into())
}

fn summary_from_bar(
    id: &str,
    display_name: &str,
    source: &str,
    is_live: bool,
    is_applied: bool,
    is_deletable: bool,
    bar: &Value,
) -> BarProfileSummary {
    let (left, center, right) = layout_preview(bar);
    BarProfileSummary {
        id: id.to_string(),
        display_name: display_name.to_string(),
        source: source.to_string(),
        is_live,
        is_applied,
        is_deletable,
        left,
        center,
        right,
    }
}

pub fn layout_preview(bar: &Value) -> (String, String, String) {
    (
        section_preview(bar, "left"),
        section_preview(bar, "center"),
        section_preview(bar, "right"),
    )
}

fn section_preview(bar: &Value, section: &str) -> String {
    let Some(items) = bar
        .get("layout")
        .and_then(|layout| layout.get(section))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(short_widget_id)
        .collect::<Vec<_>>()
        .join("  ")
}

fn short_widget_id(id: &str) -> &str {
    id.strip_prefix("omarchy.").unwrap_or(id)
}

fn require_omarchy(paths: &BarPaths) -> Result<(), String> {
    match detect_host(paths) {
        BarHost::OmarchyShell => Ok(()),
        BarHost::WaybarLegacy => Err(host_message(&BarHost::WaybarLegacy)),
        BarHost::Unsupported => Err(host_message(&BarHost::Unsupported)),
    }
}

fn profile_dir(paths: &BarPaths, id: &str) -> PathBuf {
    paths.bars_dir.join(id)
}

fn backup_dir(paths: &BarPaths) -> PathBuf {
    paths.bars_dir.join(BACKUP_DIR)
}

fn backup_path(paths: &BarPaths) -> PathBuf {
    backup_dir(paths).join("shell.json")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))
}

fn write_atomic(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let parent = path.parent().ok_or("Invalid bar path")?;
    let raw = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temporary
        .write_all(raw.as_bytes())
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

fn omarchy_default_shell() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("OMARCHY_PATH") {
        let path = PathBuf::from(root)
            .join("config")
            .join("omarchy")
            .join("shell.json");
        if path.exists() {
            return Some(path);
        }
    }
    let packaged = PathBuf::from("/usr/share/omarchy/config/omarchy/shell.json");
    packaged.exists().then_some(packaged)
}

fn first_waybar_config(config_dir: &Path) -> Option<PathBuf> {
    let dir = config_dir.join("waybar");
    for name in ["config.jsonc", "config.json", "config"] {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn command_on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    path.split(':')
        .map(|dir| Path::new(dir).join(name))
        .any(|candidate| candidate.is_file())
}

fn now_stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
