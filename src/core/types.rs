use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorType {
    XCursor,
    Hyprcursor,
}

impl std::fmt::Display for CursorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CursorType::XCursor => write!(f, "XCursor"),
            CursorType::Hyprcursor => write!(f, "Hyprcursor"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CursorImage {
    /// Crops transparent padding and places the glyph in the center of a square.
    /// Library previews then share a common visual baseline.
    pub fn centered_glyph(&self) -> CursorImage {
        let w = self.width as usize;
        let h = self.height as usize;
        if w == 0 || h == 0 || self.rgba.len() < w * h * 4 {
            return self.clone();
        }

        let mut min_x = w;
        let mut min_y = h;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        let mut found = false;

        for y in 0..h {
            for x in 0..w {
                let a = self.rgba[(y * w + x) * 4 + 3];
                if a > 24 {
                    found = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }

        if !found {
            return self.clone();
        }

        min_x = min_x.saturating_sub(1);
        min_y = min_y.saturating_sub(1);
        max_x = (max_x + 1).min(w.saturating_sub(1));
        max_y = (max_y + 1).min(h.saturating_sub(1));

        let crop_w = max_x - min_x + 1;
        let crop_h = max_y - min_y + 1;
        let side = crop_w.max(crop_h).max(24);
        let ox = (side - crop_w) / 2;
        let oy = (side - crop_h) / 2;

        let mut rgba = vec![0u8; side * side * 4];
        for y in 0..crop_h {
            for x in 0..crop_w {
                let src = ((min_y + y) * w + (min_x + x)) * 4;
                let dst = ((oy + y) * side + (ox + x)) * 4;
                rgba[dst..dst + 4].copy_from_slice(&self.rgba[src..src + 4]);
            }
        }

        CursorImage {
            width: side as u32,
            height: side as u32,
            rgba,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CursorTheme {
    pub name: String,
    pub display_name: String,
    pub comment: String,
    pub cursor_type: CursorType,
    pub path: PathBuf,
    pub is_user: bool,
    pub preview_default: Option<CursorImage>,
    pub preview_pointer: Option<CursorImage>,
    pub preview_wait: Option<CursorImage>,
    pub preview_text: Option<CursorImage>,
}

#[derive(Debug, Clone)]
pub struct ActiveCursorState {
    pub theme_name: String,
    pub size: u32,
}
