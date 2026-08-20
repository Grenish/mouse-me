use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use image::imageops::FilterType;

use super::types::CursorImage;
use super::xcursor::write_xcursor_file;

pub struct RoleDef {
    pub id: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str; 5],
}

static LEFT_PTR_ALIASES: [&str; 5] = ["default", "arrow", "top_left_arrow", "left_arrow", ""];
static POINTER_ALIASES: [&str; 5] = ["pointing_hand", "hand1", "hand2", "hand", ""];
static TEXT_ALIASES: [&str; 5] = ["xterm", "ibeam", "vertical-text", "", ""];
static WAIT_ALIASES: [&str; 5] = ["watch", "", "", "", ""];
static PROGRESS_ALIASES: [&str; 5] = ["left_ptr_watch", "half-busy", "", "", ""];
static MOVE_ALIASES: [&str; 5] = ["fleur", "size_all", "grabbing", "grab", "all-scroll"];
static NS_RESIZE_ALIASES: [&str; 5] = [
    "sb_v_double_arrow",
    "size_ver",
    "v_double_arrow",
    "row-resize",
    "",
];
static EW_RESIZE_ALIASES: [&str; 5] = [
    "sb_h_double_arrow",
    "size_hor",
    "h_double_arrow",
    "col-resize",
    "",
];
static NESW_RESIZE_ALIASES: [&str; 5] = ["size_bdiag", "fd_double_arrow", "", "", ""];
static NWSE_RESIZE_ALIASES: [&str; 5] = ["size_fdiag", "bd_double_arrow", "", "", ""];
static NOT_ALLOWED_ALIASES: [&str; 5] = ["crossed_circle", "dnd-no-drop", "circle", "", ""];
static CROSSHAIR_ALIASES: [&str; 5] = ["cross", "tcross", "plus", "cell", ""];
static HELP_ALIASES: [&str; 5] = ["question_arrow", "whats_this", "", "", ""];
static PENCIL_ALIASES: [&str; 5] = ["draft", "", "", "", ""];

pub const STUDIO_ROLES: &[RoleDef; 14] = &[
    RoleDef {
        id: "left_ptr",
        label: "Default",
        aliases: &LEFT_PTR_ALIASES,
    },
    RoleDef {
        id: "pointer",
        label: "Link",
        aliases: &POINTER_ALIASES,
    },
    RoleDef {
        id: "text",
        label: "Text",
        aliases: &TEXT_ALIASES,
    },
    RoleDef {
        id: "wait",
        label: "Busy",
        aliases: &WAIT_ALIASES,
    },
    RoleDef {
        id: "progress",
        label: "Progress",
        aliases: &PROGRESS_ALIASES,
    },
    RoleDef {
        id: "move",
        label: "Move",
        aliases: &MOVE_ALIASES,
    },
    RoleDef {
        id: "ns-resize",
        label: "Resize NS",
        aliases: &NS_RESIZE_ALIASES,
    },
    RoleDef {
        id: "ew-resize",
        label: "Resize EW",
        aliases: &EW_RESIZE_ALIASES,
    },
    RoleDef {
        id: "nesw-resize",
        label: "Resize NESW",
        aliases: &NESW_RESIZE_ALIASES,
    },
    RoleDef {
        id: "nwse-resize",
        label: "Resize NWSE",
        aliases: &NWSE_RESIZE_ALIASES,
    },
    RoleDef {
        id: "not-allowed",
        label: "Forbidden",
        aliases: &NOT_ALLOWED_ALIASES,
    },
    RoleDef {
        id: "crosshair",
        label: "Crosshair",
        aliases: &CROSSHAIR_ALIASES,
    },
    RoleDef {
        id: "help",
        label: "Help",
        aliases: &HELP_ALIASES,
    },
    RoleDef {
        id: "pencil",
        label: "Precision",
        aliases: &PENCIL_ALIASES,
    },
];

pub fn sanitize_theme_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "custom-pointer".into();
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "custom-pointer".into()
    } else {
        cleaned
    }
}

pub fn load_png_as_cursor(path: &Path) -> Result<CursorImage, String> {
    let dyn_img = image::open(path).map_err(|e| format!("Could not read image: {}", e))?;
    let rgba = dyn_img.to_rgba8();
    Ok(CursorImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

pub fn scale_cursor(
    image: &CursorImage,
    size: u32,
    xhot: u32,
    yhot: u32,
) -> (CursorImage, u32, u32) {
    let size = size.clamp(16, 128);
    if image.width == size && image.height == size {
        return (image.clone(), xhot.min(size - 1), yhot.min(size - 1));
    }

    let src = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .unwrap_or_else(|| image::RgbaImage::new(image.width.max(1), image.height.max(1)));
    let resized = image::imageops::resize(&src, size, size, FilterType::Lanczos3);
    let sx = if image.width == 0 {
        0.0
    } else {
        size as f32 / image.width as f32
    };
    let sy = if image.height == 0 {
        0.0
    } else {
        size as f32 / image.height as f32
    };
    let hx = ((xhot as f32) * sx).round() as u32;
    let hy = ((yhot as f32) * sy).round() as u32;
    (
        CursorImage {
            width: size,
            height: size,
            rgba: resized.into_raw(),
        },
        hx.min(size - 1),
        hy.min(size - 1),
    )
}

pub fn export_theme(
    display_name: &str,
    comment: &str,
    size: u32,
    xhot: u32,
    yhot: u32,
    images: &HashMap<String, CursorImage>,
) -> Result<String, String> {
    let home = dirs::home_dir().ok_or("Could not locate home directory")?;
    let dest_root = home.join(".local").join("share").join("icons");
    export_theme_into(display_name, comment, size, xhot, yhot, images, &dest_root)
}

/// Exports an XCursor theme into `dest_root/<sanitized-name>`.
pub fn export_theme_into(
    display_name: &str,
    comment: &str,
    size: u32,
    xhot: u32,
    yhot: u32,
    images: &HashMap<String, CursorImage>,
    dest_root: &Path,
) -> Result<String, String> {
    if images.is_empty() {
        return Err("Assign at least one PNG to a role before exporting.".into());
    }

    let folder = sanitize_theme_name(display_name);
    let dest = dest_root.join(&folder);

    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("Could not replace existing theme: {}", e))?;
    }

    let cursors_dir = dest.join("cursors");
    fs::create_dir_all(&cursors_dir).map_err(|e| e.to_string())?;

    let fallback = images
        .get("left_ptr")
        .or_else(|| images.values().next())
        .cloned()
        .ok_or("No cursor image available")?;

    for role in STUDIO_ROLES {
        let source = images.get(role.id).unwrap_or(&fallback);
        let delay = 50;
        let (scaled, hx, hy) = scale_cursor(source, size, xhot, yhot);
        let file_path = cursors_dir.join(role.id);
        write_xcursor_file(&file_path, &scaled, hx, hy, delay)?;

        for alias in role.aliases {
            if alias.is_empty() {
                continue;
            }
            let alias_path = cursors_dir.join(alias);
            if alias_path.exists() {
                continue;
            }
            symlink(role.id, &alias_path)
                .map_err(|error| format!("Could not create alias '{}': {}", alias, error))?;
        }
    }

    let comment_line = if comment.trim().is_empty() {
        "Created in Mouse Me Studio"
    } else {
        comment.trim()
    };

    let index = format!(
        "[Icon Theme]\nName={name}\nComment={comment}\nInherits=core\n",
        name = display_name.trim(),
        comment = comment_line
    );
    fs::write(dest.join("index.theme"), index).map_err(|e| e.to_string())?;

    Ok(folder)
}
