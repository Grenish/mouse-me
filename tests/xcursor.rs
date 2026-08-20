mod common;

use mouse_me::core::xcursor::{parse_xcursor_file, write_xcursor_file};

#[test]
fn write_rejects_empty_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    let image = common::solid_cursor(0, 0, [0, 0, 0, 0]);
    let err = write_xcursor_file(&path, &image, 0, 0, 50).unwrap_err();
    assert!(err.contains("empty"));
}

#[test]
fn write_rejects_mismatched_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad");
    let image = mouse_me::core::types::CursorImage {
        width: 4,
        height: 4,
        rgba: vec![0; 8],
    };
    let err = write_xcursor_file(&path, &image, 0, 0, 50).unwrap_err();
    assert!(err.contains("does not match"));
}

#[test]
fn write_then_parse_roundtrip_preserves_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("left_ptr");
    let image = common::solid_cursor(32, 32, [40, 180, 90, 255]);
    write_xcursor_file(&path, &image, 2, 3, 50).unwrap();

    let parsed = parse_xcursor_file(&path, 32).expect("parse written cursor");
    assert_eq!(parsed.width, 32);
    assert_eq!(parsed.height, 32);
    assert_eq!(parsed.rgba.len(), 32 * 32 * 4);
}

#[test]
fn parse_missing_file_returns_none() {
    assert!(parse_xcursor_file(std::path::Path::new("/no/such/cursor"), 24).is_none());
}

#[test]
fn parse_non_xcursor_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-cursor");
    std::fs::write(&path, b"hello").unwrap();
    assert!(parse_xcursor_file(&path, 24).is_none());
}
