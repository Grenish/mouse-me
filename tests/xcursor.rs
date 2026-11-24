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

#[test]
fn multi_frame_roundtrip_keeps_frames_hotspots_and_delays() {
    use mouse_me::core::xcursor::{parse_xcursor_frames, write_xcursor_frames, XcursorFrame};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait");
    let frame = |size: u32, shade: u8, delay_ms: u32| XcursorFrame {
        image: common::solid_cursor(size, size, [shade, 0, 0, 255]),
        xhot: size / 2,
        yhot: 1,
        delay_ms,
    };
    let frames = vec![frame(24, 10, 60), frame(24, 20, 70), frame(48, 30, 60)];
    write_xcursor_frames(&path, &frames).unwrap();

    let small = parse_xcursor_frames(&path, 24).unwrap();
    assert_eq!(small.len(), 2);
    assert_eq!(small[1].delay_ms, 70);
    assert_eq!(small[1].image.rgba[0], 20);
    assert_eq!((small[0].xhot, small[0].yhot), (12, 1));

    let large = parse_xcursor_frames(&path, 64).unwrap();
    assert_eq!(large.len(), 1);
    assert_eq!(large[0].image.width, 48);
    assert!(parse_xcursor_file(&path, 48).is_some());
}
