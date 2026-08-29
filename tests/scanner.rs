mod common;

use mouse_me::core::scanner::{
    get_icon_directories, scan_cursor_themes, scan_cursor_themes_from, theme_matches,
};
use mouse_me::core::types::{CursorTheme, CursorType};
use std::path::PathBuf;

#[test]
fn scan_skips_non_theme_folders_and_default() {
    let root = tempfile::tempdir().unwrap();
    common::make_theme_dir(root.path(), "alpha", "Alpha Pointers");
    common::make_theme_dir(root.path(), "default", "Should Skip");
    std::fs::create_dir_all(root.path().join("not-a-theme")).unwrap();

    let themes = scan_cursor_themes_from(&[root.path().to_path_buf()]);
    let names: Vec<String> = themes.iter().map(|theme| theme.name.clone()).collect();
    assert_eq!(names, vec!["alpha".to_string()]);
    assert_eq!(themes[0].display_name, "Alpha Pointers");
    assert_eq!(themes[0].cursor_type, CursorType::XCursor);
}

#[test]
fn scan_sorts_by_display_name() {
    let root = tempfile::tempdir().unwrap();
    common::make_theme_dir(root.path(), "zulu", "Zulu");
    common::make_theme_dir(root.path(), "alpha", "Alpha");

    let themes = scan_cursor_themes_from(&[root.path().to_path_buf()]);
    let displays: Vec<_> = themes
        .iter()
        .map(|theme| theme.display_name.as_str())
        .collect();
    assert_eq!(displays, ["Alpha", "Zulu"]);
}

#[test]
fn scan_prefers_first_directory_when_names_collide() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    common::make_theme_dir(first.path(), "shared", "From First");
    common::make_theme_dir(second.path(), "shared", "From Second");

    let themes =
        scan_cursor_themes_from(&[first.path().to_path_buf(), second.path().to_path_buf()]);
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].display_name, "From First");
}

#[test]
fn get_icon_directories_has_no_duplicates() {
    let dirs = get_icon_directories();
    let mut unique = dirs.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(dirs.len(), unique.len());
}

#[test]
fn scan_cursor_themes_does_not_panic() {
    let _ = scan_cursor_themes();
}

fn sample_theme(name: &str, user: bool, cursor_type: CursorType) -> CursorTheme {
    CursorTheme {
        name: name.into(),
        display_name: format!("{name} Display"),
        comment: "a comment".into(),
        cursor_type,
        path: PathBuf::from("/tmp"),
        is_user: user,
        preview_default: None,
        preview_pointer: None,
        preview_wait: None,
        preview_text: None,
    }
}

#[test]
fn theme_matches_filters_scope_type_and_search() {
    let user = sample_theme("modest-light", true, CursorType::XCursor);
    let system = sample_theme("Adwaita", false, CursorType::Hyprcursor);

    assert!(theme_matches(&user, true, false, None, ""));
    assert!(!theme_matches(&system, true, false, None, ""));
    assert!(theme_matches(&system, false, true, None, ""));
    assert!(!theme_matches(&user, false, true, None, ""));
    assert!(theme_matches(
        &user,
        false,
        false,
        Some("xcursor"),
        "modest"
    ));
    assert!(theme_matches(&user, false, false, Some("XCursor"), ""));
    assert!(!theme_matches(&user, false, false, Some("hyprcursor"), ""));
    assert!(theme_matches(&user, false, false, None, "DISPLAY"));
    assert!(!theme_matches(&user, false, false, None, "nope"));
}
