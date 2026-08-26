use std::fs;
use std::path::Path;

use image::ImageReader;

use super::types::CursorImage;

pub const MAX_IMAGE_EDGE: u32 = 512;
pub const MAX_IMAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub fn load_bounded_rgba(path: &Path) -> Result<CursorImage, String> {
    let meta = fs::metadata(path).map_err(|error| format!("Could not read image: {error}"))?;
    if meta.len() > MAX_IMAGE_FILE_BYTES {
        return Err(format!(
            "Image is too large (maximum is {} MiB)",
            MAX_IMAGE_FILE_BYTES / (1024 * 1024)
        ));
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_IMAGE_FILE_BYTES);
    let mut reader =
        ImageReader::open(path).map_err(|error| format!("Could not read image: {error}"))?;
    reader.limits(limits);

    let dyn_img = reader
        .decode()
        .map_err(|error| format!("Could not decode image (max {MAX_IMAGE_EDGE}px): {error}"))?;
    if dyn_img.width() > MAX_IMAGE_EDGE || dyn_img.height() > MAX_IMAGE_EDGE {
        return Err(format!(
            "Image is too large (maximum is {MAX_IMAGE_EDGE}×{MAX_IMAGE_EDGE})"
        ));
    }
    let rgba = dyn_img.to_rgba8();
    Ok(CursorImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::{load_bounded_rgba, MAX_IMAGE_EDGE};
    use image::RgbaImage;

    #[test]
    fn rejects_oversized_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        let image = RgbaImage::new(MAX_IMAGE_EDGE + 1, 32);
        image.save(&path).unwrap();
        let err = load_bounded_rgba(&path).unwrap_err();
        assert!(err.contains("too large") || err.contains("max"));
    }
}
