use crate::core::types::CursorImage;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const XCURSOR_MAGIC: u32 = 0x72756358; // "Xcur"
const XCURSOR_IMAGE_TYPE: u32 = 0xfffd0002;

/// Parses an XCursor binary file and extracts the best nominal size image.
pub fn parse_xcursor_file(path: &Path, target_size: u32) -> Option<CursorImage> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header).ok()?;

    let magic = read_u32(&header, 0)?;
    if magic != XCURSOR_MAGIC {
        return None;
    }

    let ntoc = read_u32(&header, 12)? as usize;
    if ntoc == 0 || ntoc > 1000 {
        return None;
    }

    // Read all TOC entries
    let mut toc_entries = Vec::with_capacity(ntoc);
    for _ in 0..ntoc {
        let mut toc_buf = [0u8; 12];
        if file.read_exact(&mut toc_buf).is_err() {
            break;
        }
        let chunk_type = read_u32(&toc_buf, 0)?;
        let subtype = read_u32(&toc_buf, 4)?;
        let position = read_u32(&toc_buf, 8)?;

        if chunk_type == XCURSOR_IMAGE_TYPE {
            toc_entries.push((subtype, position));
        }
    }

    if toc_entries.is_empty() {
        return None;
    }

    // Pick entry closest to target_size (default 24 or 32)
    toc_entries.sort_by_key(|(size, _)| (*size as i64 - target_size as i64).abs());
    let (_, best_pos) = toc_entries[0];

    // Seek to image chunk
    file.seek(SeekFrom::Start(best_pos as u64)).ok()?;

    let mut chunk_hdr = [0u8; 36];
    file.read_exact(&mut chunk_hdr).ok()?;

    let width = read_u32(&chunk_hdr, 16)?;
    let height = read_u32(&chunk_hdr, 20)?;

    if width == 0 || height == 0 || width > 512 || height > 512 {
        return None;
    }

    let pixel_count = (width * height) as usize;
    let mut pixel_data = vec![0u8; pixel_count * 4];
    file.read_exact(&mut pixel_data).ok()?;

    // Convert ARGB/BGRA little-endian to RGBA
    let mut rgba = vec![0u8; pixel_count * 4];
    for i in 0..pixel_count {
        let src_idx = i * 4;
        let b = pixel_data[src_idx];
        let g = pixel_data[src_idx + 1];
        let r = pixel_data[src_idx + 2];
        let a = pixel_data[src_idx + 3];

        let dst_idx = i * 4;
        if a == 0 {
            rgba[dst_idx] = 0;
            rgba[dst_idx + 1] = 0;
            rgba[dst_idx + 2] = 0;
            rgba[dst_idx + 3] = 0;
        } else {
            // Un-premultiply alpha if needed
            let alpha_f = a as f32 / 255.0;
            let un_r = ((r as f32 / alpha_f).min(255.0)) as u8;
            let un_g = ((g as f32 / alpha_f).min(255.0)) as u8;
            let un_b = ((b as f32 / alpha_f).min(255.0)) as u8;

            rgba[dst_idx] = un_r;
            rgba[dst_idx + 1] = un_g;
            rgba[dst_idx + 2] = un_b;
            rgba[dst_idx + 3] = a;
        }
    }

    Some(CursorImage {
        width,
        height,
        rgba,
    })
}

/// Writes a single-frame XCursor file (Xcur) with a BGRA premultiplied payload.
pub fn write_xcursor_file(
    path: &Path,
    image: &CursorImage,
    xhot: u32,
    yhot: u32,
    delay_ms: u32,
) -> Result<(), String> {
    if image.width == 0 || image.height == 0 {
        return Err("Cursor image is empty".into());
    }
    if image.rgba.len() != (image.width as usize) * (image.height as usize) * 4 {
        return Err("Cursor image pixel buffer does not match dimensions".into());
    }

    let xhot = xhot.min(image.width.saturating_sub(1));
    let yhot = yhot.min(image.height.saturating_sub(1));
    let nominal = image.width.max(image.height);

    let mut buf = Vec::new();
    write_u32(&mut buf, XCURSOR_MAGIC);
    write_u32(&mut buf, 16); // header bytes
    write_u32(&mut buf, 0x0001_0000); // version 1.0
    write_u32(&mut buf, 1); // ntoc

    let chunk_pos = 16 + 12;
    write_u32(&mut buf, XCURSOR_IMAGE_TYPE);
    write_u32(&mut buf, nominal);
    write_u32(&mut buf, chunk_pos);

    write_u32(&mut buf, 36); // chunk header size
    write_u32(&mut buf, XCURSOR_IMAGE_TYPE);
    write_u32(&mut buf, nominal);
    write_u32(&mut buf, 1); // chunk version
    write_u32(&mut buf, image.width);
    write_u32(&mut buf, image.height);
    write_u32(&mut buf, xhot);
    write_u32(&mut buf, yhot);
    write_u32(&mut buf, delay_ms.max(1));

    let pixel_count = (image.width * image.height) as usize;
    for i in 0..pixel_count {
        let src = i * 4;
        let r = image.rgba[src] as u16;
        let g = image.rgba[src + 1] as u16;
        let b = image.rgba[src + 2] as u16;
        let a = image.rgba[src + 3] as u16;
        let (pr, pg, pb) = if a == 0 {
            (0, 0, 0)
        } else {
            ((r * a) / 255, (g * a) / 255, (b * a) / 255)
        };
        buf.push(pb as u8);
        buf.push(pg as u8);
        buf.push(pr as u8);
        buf.push(a as u8);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, buf).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn write_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(value.try_into().ok()?))
}
