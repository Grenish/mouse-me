mod common;

use mouse_me::core::types::{CursorImage, CursorType};

#[test]
fn cursor_type_display_names() {
    assert_eq!(CursorType::XCursor.to_string(), "XCursor");
    assert_eq!(CursorType::Hyprcursor.to_string(), "Hyprcursor");
}

#[test]
fn centered_glyph_returns_empty_image_unchanged() {
    let empty = CursorImage {
        width: 0,
        height: 0,
        rgba: vec![],
    };
    let centered = empty.centered_glyph();
    assert_eq!(centered.width, 0);
    assert_eq!(centered.height, 0);
}

#[test]
fn centered_glyph_returns_fully_transparent_image_unchanged() {
    let image = common::solid_cursor(8, 8, [0, 0, 0, 0]);
    let centered = image.centered_glyph();
    assert_eq!(centered.width, 8);
    assert_eq!(centered.height, 8);
    assert_eq!(centered.rgba, image.rgba);
}

#[test]
fn centered_glyph_places_opaque_blob_on_square_canvas() {
    let mut rgba = vec![0u8; 8 * 8 * 4];
    for y in 0..2 {
        for x in 0..2 {
            let i = (y * 8 + x) * 4;
            rgba[i] = 255;
            rgba[i + 3] = 255;
        }
    }
    let image = CursorImage {
        width: 8,
        height: 8,
        rgba,
    };
    let centered = image.centered_glyph();
    assert_eq!(centered.width, centered.height);
    assert!(centered.width >= 24);

    let mut opaque = 0u32;
    for pixel in centered.rgba.chunks_exact(4) {
        if pixel[3] > 24 {
            opaque += 1;
        }
    }
    assert_eq!(opaque, 4);
}
