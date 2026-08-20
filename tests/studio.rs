mod common;

use std::collections::HashMap;

use mouse_me::core::studio::{
    export_theme, export_theme_into, load_png_as_cursor, sanitize_theme_name, scale_cursor,
    STUDIO_ROLES,
};
use mouse_me::core::xcursor::parse_xcursor_file;

#[test]
fn sanitize_replaces_spaces_and_unsafe_characters() {
    assert_eq!(sanitize_theme_name("My Pointer"), "My-Pointer");
    assert_eq!(sanitize_theme_name("cool/theme"), "cool_theme");
    assert_eq!(sanitize_theme_name("  "), "custom-pointer");
    assert_eq!(sanitize_theme_name(""), "custom-pointer");
    assert_eq!(sanitize_theme_name("keep_this-01"), "keep_this-01");
}

#[test]
fn studio_roles_cover_core_pointer_names() {
    let ids: Vec<&str> = STUDIO_ROLES.iter().map(|role| role.id).collect();
    assert!(ids.contains(&"left_ptr"));
    assert!(ids.contains(&"pointer"));
    assert!(ids.contains(&"text"));
    assert!(ids.contains(&"wait"));
    assert_eq!(STUDIO_ROLES.len(), 14);
}

#[test]
fn scale_cursor_clamps_size_and_hotspot() {
    let image = common::solid_cursor(64, 64, [10, 20, 30, 255]);
    let (scaled, xhot, yhot) = scale_cursor(&image, 8, 80, 80);
    assert_eq!(scaled.width, 16);
    assert_eq!(scaled.height, 16);
    assert_eq!(xhot, 15);
    assert_eq!(yhot, 15);
}

#[test]
fn scale_cursor_keeps_matching_size() {
    let image = common::solid_cursor(32, 32, [1, 2, 3, 255]);
    let (scaled, xhot, yhot) = scale_cursor(&image, 32, 4, 5);
    assert_eq!(scaled.width, 32);
    assert_eq!(xhot, 4);
    assert_eq!(yhot, 5);
}

#[test]
fn load_png_as_cursor_reads_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    common::write_png(&path, 12, 10, [9, 8, 7, 255]);
    let image = load_png_as_cursor(&path).unwrap();
    assert_eq!(image.width, 12);
    assert_eq!(image.height, 10);
    assert_eq!(image.rgba.len(), 12 * 10 * 4);
}

#[test]
fn load_png_as_cursor_rejects_missing_file() {
    let err = load_png_as_cursor(std::path::Path::new("/no/such.png")).unwrap_err();
    assert!(err.contains("Could not read image"));
}

#[test]
fn export_theme_requires_at_least_one_image() {
    let err = export_theme("demo", "", 24, 0, 0, &HashMap::new()).unwrap_err();
    assert!(err.contains("at least one PNG"));
}

#[test]
fn export_theme_into_writes_xcursor_files_and_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut images = HashMap::new();
    images.insert(
        "left_ptr".into(),
        common::solid_cursor(32, 32, [255, 0, 0, 255]),
    );

    let name = export_theme_into(
        "Test Studio",
        "From tests",
        24,
        1,
        1,
        &images,
        dir.path(),
    )
    .unwrap();

    assert_eq!(name, "Test-Studio");
    let theme = dir.path().join(&name);
    assert!(theme.join("index.theme").exists());
    let index = std::fs::read_to_string(theme.join("index.theme")).unwrap();
    assert!(index.contains("Name=Test Studio"));
    assert!(index.contains("Comment=From tests"));

    let left_ptr = theme.join("cursors").join("left_ptr");
    assert!(parse_xcursor_file(&left_ptr, 24).is_some());
    assert!(theme.join("cursors").join("default").exists());
    assert!(theme.join("cursors").join("pointer").exists());
}
