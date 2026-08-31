use mouse_me::core::bar::{
    apply_profile, detect_host, empty_bar, layout_preview, list_profiles, live_bar, new_empty,
    remove_profile, restore_backup, save_from_live, BarHost, BarPaths,
};
use serde_json::json;
use std::fs;
use std::path::Path;

fn paths(root: &Path, omarchy: bool, waybar: bool) -> BarPaths {
    let live = root.join("config/omarchy/shell.json");
    let default = root.join("default/shell.json");
    let waybar_path = root.join("config/waybar/config.jsonc");
    fs::create_dir_all(live.parent().unwrap()).unwrap();
    fs::create_dir_all(default.parent().unwrap()).unwrap();
    if waybar {
        fs::create_dir_all(waybar_path.parent().unwrap()).unwrap();
        fs::write(&waybar_path, "{}").unwrap();
    }
    BarPaths {
        bars_dir: root.join("bars"),
        live_shell: live,
        default_shell: Some(default),
        waybar_config: waybar.then_some(waybar_path),
        omarchy_on_path: omarchy,
    }
}

fn write_live(paths: &BarPaths) {
    fs::write(
        &paths.live_shell,
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "idle": { "lock": 300 },
            "plugins": [{ "id": "community.weather-extra" }],
            "extra": "keep-me",
            "bar": {
                "position": "top",
                "layout": {
                    "left": [{ "id": "omarchy.menu" }],
                    "center": [{ "id": "omarchy.clock" }],
                    "right": [{ "id": "omarchy.audio" }]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn detect_omarchy_when_binary_and_config_exist() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), true, false);
    fs::write(
        paths.default_shell.as_ref().unwrap(),
        r#"{"version":1,"bar":{}}"#,
    )
    .unwrap();
    assert_eq!(detect_host(&paths), BarHost::OmarchyShell);
}

#[test]
fn detect_waybar_without_omarchy() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), false, true);
    assert_eq!(detect_host(&paths), BarHost::WaybarLegacy);
}

#[test]
fn detect_unsupported_without_either() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), false, false);
    assert_eq!(detect_host(&paths), BarHost::Unsupported);
}

#[test]
fn empty_canvas_has_empty_sections() {
    let (left, center, right) = layout_preview(&empty_bar());
    assert!(left.is_empty());
    assert!(center.is_empty());
    assert!(right.is_empty());
}

#[test]
fn save_writes_bar_only_and_apply_keeps_idle() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), true, false);
    write_live(&paths);

    let id = save_from_live(&paths, "Work").unwrap();
    assert_eq!(id, "Work");
    let saved = fs::read_to_string(paths.bars_dir.join("Work").join("shell.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert!(saved.get("idle").is_none());
    assert!(saved.get("plugins").is_none());
    assert!(saved.get("bar").is_some());

    let empty_id = new_empty(&paths, "Empty").unwrap();
    apply_profile(&paths, &empty_id).unwrap();
    let live = live_bar(&paths).unwrap();
    let (left, _, _) = layout_preview(&live);
    assert!(left.is_empty());

    let document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.live_shell).unwrap()).unwrap();
    assert_eq!(document["idle"]["lock"], 300);
    assert_eq!(document["plugins"][0]["id"], "community.weather-extra");
    assert_eq!(document["extra"], "keep-me");

    restore_backup(&paths).unwrap();
    let restored = live_bar(&paths).unwrap();
    let (left, center, right) = layout_preview(&restored);
    assert_eq!(left, "menu");
    assert_eq!(center, "clock");
    assert_eq!(right, "audio");
}

#[test]
fn list_includes_live_row_and_saved_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), true, false);
    write_live(&paths);
    save_from_live(&paths, "Work").unwrap();
    let rows = list_profiles(&paths).unwrap();
    assert_eq!(rows[0].id, "_live");
    assert!(rows[0].is_live);
    assert!(!rows[0].is_deletable);
    assert!(rows.iter().any(|row| row.id == "Work" && row.is_applied));
}

#[test]
fn remove_rejects_live_and_deletes_saved() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), true, false);
    write_live(&paths);
    save_from_live(&paths, "Work").unwrap();
    assert!(remove_profile(&paths, "_live").is_err());
    remove_profile(&paths, "Work").unwrap();
    assert!(!paths.bars_dir.join("Work").exists());
}

#[test]
fn unsafe_names_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), true, false);
    write_live(&paths);
    assert!(save_from_live(&paths, "..").is_err());
    assert!(save_from_live(&paths, ".").is_err());
    assert!(new_empty(&paths, "nested/theme").is_err());
}

#[test]
fn waybar_host_cannot_apply() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path(), false, true);
    assert!(new_empty(&paths, "Empty").is_err());
}

#[test]
fn unknown_widget_ids_round_trip() {
    let bar = json!({
        "layout": {
            "left": [{ "id": "community.vpn", "color": "#6fcf82" }],
            "center": [],
            "right": []
        }
    });
    let (left, _, _) = layout_preview(&bar);
    assert_eq!(left, "community.vpn");
}
