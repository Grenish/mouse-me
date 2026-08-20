#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use mouse_me::core::types::CursorImage;

pub fn solid_cursor(width: u32, height: u32, rgba: [u8; 4]) -> CursorImage {
    let count = (width * height) as usize;
    let mut pixels = Vec::with_capacity(count * 4);
    for _ in 0..count {
        pixels.extend_from_slice(&rgba);
    }
    CursorImage {
        width,
        height,
        rgba: pixels,
    }
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: [u8; 4]) {
    let mut image = image::RgbaImage::new(width, height);
    for pixel in image.pixels_mut() {
        *pixel = image::Rgba(rgba);
    }
    image.save(path).expect("write png");
}

pub fn make_theme_dir(root: &Path, name: &str, display: &str) -> PathBuf {
    let theme = root.join(name);
    fs::create_dir_all(theme.join("cursors")).unwrap();
    fs::write(
        theme.join("index.theme"),
        format!("[Icon Theme]\nName={display}\nComment=Test theme\nInherits=core\n"),
    )
    .unwrap();
    theme
}

pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        if name.ends_with('/') {
            zip.add_directory(*name, options).unwrap();
        } else {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
    }
    zip.finish().unwrap();
}
