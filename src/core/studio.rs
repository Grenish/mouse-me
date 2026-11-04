use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use serde::{Deserialize, Serialize};

use super::types::CursorImage;
use super::xcursor::{parse_xcursor_frames, write_xcursor_frames, XcursorFrame};

/// File dropped into every exported theme so Studio knows it may replace it.
pub const STUDIO_MARKER: &str = ".mouse-me-studio";
pub const SIZE_CHOICES: [u32; 4] = [24, 32, 48, 64];
pub const MAX_FRAMES: usize = 64;
const DEFAULT_DELAY_MS: u32 = 80;
const OPAQUE: u8 = 32;

/// Where a role's hotspot usually sits, used to guess one for a new frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    Center,
    BottomLeft,
}

pub struct RoleDef {
    pub id: &'static str,
    pub label: &'static str,
    /// The family a role belongs to, used to group the role list.
    pub group: &'static str,
    pub hint: &'static str,
    pub anchor: Anchor,
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
        group: "Pointing",
        hint: "The everyday arrow. Its tip is the hotspot.",
        anchor: Anchor::TopLeft,
        aliases: &LEFT_PTR_ALIASES,
    },
    RoleDef {
        id: "pointer",
        label: "Link",
        group: "Pointing",
        hint: "Shown over links and buttons. Point at the fingertip.",
        anchor: Anchor::Top,
        aliases: &POINTER_ALIASES,
    },
    RoleDef {
        id: "help",
        label: "Help",
        group: "Pointing",
        hint: "Arrow with a question mark. Its tip is the hotspot.",
        anchor: Anchor::TopLeft,
        aliases: &HELP_ALIASES,
    },
    RoleDef {
        id: "progress",
        label: "Progress",
        group: "Pointing",
        hint: "Working in the background, still clickable. Usually animated.",
        anchor: Anchor::TopLeft,
        aliases: &PROGRESS_ALIASES,
    },
    RoleDef {
        id: "text",
        label: "Text",
        group: "Text and precision",
        hint: "The I-beam over editable text. Centre it.",
        anchor: Anchor::Center,
        aliases: &TEXT_ALIASES,
    },
    RoleDef {
        id: "crosshair",
        label: "Crosshair",
        group: "Text and precision",
        hint: "Precise selection, like screenshots. Centre it exactly.",
        anchor: Anchor::Center,
        aliases: &CROSSHAIR_ALIASES,
    },
    RoleDef {
        id: "pencil",
        label: "Pencil",
        group: "Text and precision",
        hint: "Drawing and handwriting. Put the hotspot at the nib.",
        anchor: Anchor::BottomLeft,
        aliases: &PENCIL_ALIASES,
    },
    RoleDef {
        id: "wait",
        label: "Busy",
        group: "Status",
        hint: "The app is not responding. Usually animated.",
        anchor: Anchor::Center,
        aliases: &WAIT_ALIASES,
    },
    RoleDef {
        id: "not-allowed",
        label: "Forbidden",
        group: "Status",
        hint: "The action under the pointer is not allowed.",
        anchor: Anchor::Center,
        aliases: &NOT_ALLOWED_ALIASES,
    },
    RoleDef {
        id: "move",
        label: "Move",
        group: "Move and resize",
        hint: "Dragging or grabbing something.",
        anchor: Anchor::Center,
        aliases: &MOVE_ALIASES,
    },
    RoleDef {
        id: "ns-resize",
        label: "Vertical",
        group: "Move and resize",
        hint: "Resizing a top or bottom edge.",
        anchor: Anchor::Center,
        aliases: &NS_RESIZE_ALIASES,
    },
    RoleDef {
        id: "ew-resize",
        label: "Horizontal",
        group: "Move and resize",
        hint: "Resizing a left or right edge.",
        anchor: Anchor::Center,
        aliases: &EW_RESIZE_ALIASES,
    },
    RoleDef {
        id: "nesw-resize",
        label: "Diagonal NE",
        group: "Move and resize",
        hint: "Resizing a top-right or bottom-left corner.",
        anchor: Anchor::Center,
        aliases: &NESW_RESIZE_ALIASES,
    },
    RoleDef {
        id: "nwse-resize",
        label: "Diagonal NW",
        group: "Move and resize",
        hint: "Resizing a top-left or bottom-right corner.",
        anchor: Anchor::Center,
        aliases: &NWSE_RESIZE_ALIASES,
    },
];

pub fn role_def(id: &str) -> Option<&'static RoleDef> {
    STUDIO_ROLES.iter().find(|role| role.id == id)
}

/// Every frame of a role, padded to one square edge, plus the role's hotspot
/// in that square's pixel space.
#[derive(Debug, Clone)]
pub struct RoleAsset {
    pub frames: Vec<CursorImage>,
    pub xhot: u32,
    pub yhot: u32,
    pub delay_ms: u32,
}

impl RoleAsset {
    pub fn edge(&self) -> u32 {
        self.frames.first().map(|frame| frame.width).unwrap_or(1)
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }
}

#[derive(Debug, Clone)]
pub struct StudioProject {
    pub name: String,
    pub comment: String,
    pub sizes: Vec<u32>,
    pub fill_missing: bool,
    pub roles: HashMap<String, RoleAsset>,
}

impl Default for StudioProject {
    fn default() -> Self {
        Self {
            name: "Custom Pointer".into(),
            comment: String::new(),
            sizes: vec![24, 32, 48],
            fill_missing: true,
            roles: HashMap::new(),
        }
    }
}

impl StudioProject {
    /// Adds frames to a role. A new role (or `replace`) takes its square edge
    /// from the first image and gets a guessed hotspot; later frames are
    /// fitted to that edge so an animation never jumps.
    pub fn add_frames(
        &mut self,
        role: &str,
        images: Vec<CursorImage>,
        replace: bool,
    ) -> Result<usize, String> {
        let def = role_def(role).ok_or_else(|| format!("Unknown role '{role}'"))?;
        let mut images = images.into_iter().map(|image| square_pad(&image).0);
        let existing = if replace {
            None
        } else {
            self.roles.get_mut(role)
        };
        let asset = match existing {
            Some(asset) => asset,
            None => {
                let first = images.next().ok_or("No image to add")?;
                let (xhot, yhot) = suggest_hotspot(def.anchor, &first);
                self.roles.insert(
                    role.to_string(),
                    RoleAsset {
                        frames: vec![first],
                        xhot,
                        yhot,
                        delay_ms: DEFAULT_DELAY_MS,
                    },
                );
                self.roles.get_mut(role).expect("role was just inserted")
            }
        };
        let edge = asset.edge();
        for image in images {
            if asset.frames.len() >= MAX_FRAMES {
                return Err(format!("A role can hold at most {MAX_FRAMES} frames"));
            }
            asset.frames.push(resize_square(&image, edge));
        }
        Ok(asset.frames.len())
    }

    pub fn remove_frame(&mut self, role: &str, index: usize) {
        let Some(asset) = self.roles.get_mut(role) else {
            return;
        };
        if index < asset.frames.len() {
            asset.frames.remove(index);
        }
        if asset.frames.is_empty() {
            self.roles.remove(role);
        }
    }

    /// Moves a frame one step earlier (`-1`) or later (`1`) in the animation.
    pub fn move_frame(&mut self, role: &str, index: usize, delta: i32) -> usize {
        let Some(asset) = self.roles.get_mut(role) else {
            return index;
        };
        let target = index as i64 + delta as i64;
        if index >= asset.frames.len() || target < 0 || target >= asset.frames.len() as i64 {
            return index;
        }
        asset.frames.swap(index, target as usize);
        target as usize
    }

    pub fn set_hotspot(&mut self, role: &str, x: i64, y: i64) {
        if let Some(asset) = self.roles.get_mut(role) {
            let last = asset.edge() as i64 - 1;
            asset.xhot = x.clamp(0, last) as u32;
            asset.yhot = y.clamp(0, last) as u32;
        }
    }

    pub fn auto_hotspot(&mut self, role: &str) {
        let Some(def) = role_def(role) else { return };
        if let Some(asset) = self.roles.get_mut(role) {
            let (x, y) = suggest_hotspot(def.anchor, &asset.frames[0]);
            asset.xhot = x;
            asset.yhot = y;
        }
    }

    pub fn set_delay(&mut self, role: &str, delay_ms: u32) {
        if let Some(asset) = self.roles.get_mut(role) {
            asset.delay_ms = delay_ms.clamp(10, 2000);
        }
    }

    /// Copies a role's frames into every role that has none yet. The hotspot
    /// is re-guessed for each role because a text cursor and an arrow anchor
    /// differently even when they share a picture.
    pub fn fill_empty_from(&mut self, role: &str) -> usize {
        let Some(source) = self.roles.get(role).cloned() else {
            return 0;
        };
        let mut filled = 0;
        for def in STUDIO_ROLES {
            if self.roles.contains_key(def.id) {
                continue;
            }
            let mut copy = source.clone();
            let (x, y) = suggest_hotspot(def.anchor, &copy.frames[0]);
            copy.xhot = x;
            copy.yhot = y;
            self.roles.insert(def.id.to_string(), copy);
            filled += 1;
        }
        filled
    }

    pub fn export_sizes(&self) -> Vec<u32> {
        let mut sizes: Vec<u32> = self
            .sizes
            .iter()
            .map(|size| (*size).clamp(16, 256))
            .collect();
        sizes.sort_unstable();
        sizes.dedup();
        sizes
    }

    /// The role whose art fills empty roles: Default when it has art,
    /// otherwise the first role that does.
    pub fn fallback_role(&self) -> Option<&'static str> {
        if self.roles.contains_key("left_ptr") {
            return Some("left_ptr");
        }
        STUDIO_ROLES
            .iter()
            .map(|def| def.id)
            .find(|id| self.roles.contains_key(*id))
    }

    /// The asset exported for a role: its own, or the fallback when
    /// `fill_missing` is on. `None` leaves the role to the inherited theme.
    pub fn asset_for_export(&self, role: &str) -> Option<&RoleAsset> {
        if let Some(asset) = self.roles.get(role) {
            return Some(asset);
        }
        if !self.fill_missing {
            return None;
        }
        self.roles.get(self.fallback_role()?)
    }
}

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
    super::images::load_bounded_rgba(path)
}

/// Centres an image on a transparent square so scaling never stretches it.
/// Returns the padded image and the offset the original was placed at.
pub fn square_pad(image: &CursorImage) -> (CursorImage, u32, u32) {
    let edge = image.width.max(image.height).max(1);
    if image.width == edge && image.height == edge {
        return (image.clone(), 0, 0);
    }
    let ox = (edge - image.width) / 2;
    let oy = (edge - image.height) / 2;
    let mut rgba = vec![0u8; (edge * edge * 4) as usize];
    let row = image.width as usize * 4;
    for y in 0..image.height as usize {
        let src = y * row;
        let dst = ((y + oy as usize) * edge as usize + ox as usize) * 4;
        rgba[dst..dst + row].copy_from_slice(&image.rgba[src..src + row]);
    }
    (
        CursorImage {
            width: edge,
            height: edge,
            rgba,
        },
        ox,
        oy,
    )
}

fn resize_square(image: &CursorImage, edge: u32) -> CursorImage {
    let (square, _, _) = square_pad(image);
    if square.width == edge {
        return square;
    }
    let source = image::RgbaImage::from_raw(square.width, square.height, square.rgba)
        .unwrap_or_else(|| image::RgbaImage::new(edge, edge));
    let resized = image::imageops::resize(&source, edge, edge, FilterType::Lanczos3);
    CursorImage {
        width: edge,
        height: edge,
        rgba: resized.into_raw(),
    }
}

/// Guesses the hotspot from the opaque pixels: the arrow tip for top-left
/// roles, the fingertip for the hand, the nib for the pencil, and the middle
/// of the glyph for everything else.
pub fn suggest_hotspot(anchor: Anchor, image: &CursorImage) -> (u32, u32) {
    let w = image.width as usize;
    let h = image.height as usize;
    let opaque = |x: usize, y: usize| image.rgba[(y * w + x) * 4 + 3] > OPAQUE;
    let row_span = |y: usize| {
        let mut xs = (0..w).filter(|&x| opaque(x, y));
        let first = xs.next()?;
        Some((first, xs.next_back().unwrap_or(first)))
    };
    let rows: Vec<usize> = (0..h).filter(|&y| row_span(y).is_some()).collect();
    let (Some(&top), Some(&bottom)) = (rows.first(), rows.last()) else {
        return (0, 0);
    };
    match anchor {
        Anchor::TopLeft => (
            row_span(top).map(|span| span.0).unwrap_or(0) as u32,
            top as u32,
        ),
        Anchor::Top => {
            let (left, right) = row_span(top).unwrap_or((0, 0));
            (((left + right) / 2) as u32, top as u32)
        }
        Anchor::BottomLeft => (
            row_span(bottom).map(|span| span.0).unwrap_or(0) as u32,
            bottom as u32,
        ),
        Anchor::Center => {
            let left = rows.iter().filter_map(|&y| row_span(y)).map(|s| s.0).min();
            let right = rows.iter().filter_map(|&y| row_span(y)).map(|s| s.1).max();
            let x = (left.unwrap_or(0) + right.unwrap_or(0)) / 2;
            (x as u32, ((top + bottom) / 2) as u32)
        }
    }
}

/// Scales an image to a `size`×`size` cursor, padding non-square art first so
/// it keeps its proportions. The hotspot follows the image.
pub fn scale_cursor(
    image: &CursorImage,
    size: u32,
    xhot: u32,
    yhot: u32,
) -> (CursorImage, u32, u32) {
    let size = size.clamp(16, 256);
    let (square, ox, oy) = square_pad(image);
    let (xhot, yhot) = (xhot + ox, yhot + oy);
    if square.width == size {
        return (square, xhot.min(size - 1), yhot.min(size - 1));
    }
    let scale = size as f32 / square.width.max(1) as f32;
    let scaled = resize_square(&square, size);
    let hx = ((xhot as f32) * scale).round() as u32;
    let hy = ((yhot as f32) * scale).round() as u32;
    (scaled, hx.min(size - 1), hy.min(size - 1))
}

pub fn user_icons_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not locate home directory")?;
    Ok(home.join(".local").join("share").join("icons"))
}

pub fn export_project(project: &StudioProject) -> Result<String, String> {
    export_project_into(project, &user_icons_dir()?)
}

/// Exports an XCursor theme into `dest_root/<sanitized-name>`. Every role is
/// written at every chosen size; animated roles keep all their frames. The
/// theme is built in a staging folder and swapped in, and a theme that Studio
/// did not create is never replaced.
pub fn export_project_into(project: &StudioProject, dest_root: &Path) -> Result<String, String> {
    if project.roles.is_empty() {
        return Err("Add a frame to at least one role before exporting.".into());
    }
    let sizes = project.export_sizes();
    if sizes.is_empty() {
        return Err("Pick at least one size to export.".into());
    }

    let folder = sanitize_theme_name(&project.name);
    let dest = dest_root.join(&folder);
    if dest.exists() && !dest.join(STUDIO_MARKER).exists() {
        return Err(format!(
            "A theme called '{folder}' is already installed and was not made in Studio. Rename yours to keep both."
        ));
    }

    fs::create_dir_all(dest_root).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".mouse-me-studio-")
        .tempdir_in(dest_root)
        .map_err(|e| format!("Could not create a staging folder: {e}"))?;
    let cursors_dir = staging.path().join("cursors");
    fs::create_dir_all(&cursors_dir).map_err(|e| e.to_string())?;

    let written: Vec<&RoleDef> = STUDIO_ROLES
        .iter()
        .filter(|def| project.asset_for_export(def.id).is_some())
        .collect();
    for def in &written {
        let asset = project.asset_for_export(def.id).expect("filtered above");
        let mut frames = Vec::with_capacity(sizes.len() * asset.frames.len());
        for &size in &sizes {
            for frame in &asset.frames {
                let (image, xhot, yhot) = scale_cursor(frame, size, asset.xhot, asset.yhot);
                frames.push(XcursorFrame {
                    image,
                    xhot,
                    yhot,
                    delay_ms: asset.delay_ms,
                });
            }
        }
        write_xcursor_frames(&cursors_dir.join(def.id), &frames)?;
    }
    for def in &written {
        for alias in def.aliases.iter().filter(|alias| !alias.is_empty()) {
            let alias_path = cursors_dir.join(alias);
            if alias_path.symlink_metadata().is_ok() {
                continue;
            }
            symlink(def.id, &alias_path)
                .map_err(|error| format!("Could not create alias '{alias}': {error}"))?;
        }
    }

    let comment = project.comment.trim();
    let comment = if comment.is_empty() {
        "Created in Mouse Me Studio"
    } else {
        comment
    };
    let display_name = project.name.trim();
    let display_name = if display_name.is_empty() {
        folder.as_str()
    } else {
        display_name
    };
    let index = format!(
        "[Icon Theme]\nName={}\nComment={}\nInherits=Adwaita,core\n",
        single_line(display_name),
        single_line(comment)
    );
    fs::write(staging.path().join("index.theme"), index).map_err(|e| e.to_string())?;
    fs::write(
        staging.path().join(STUDIO_MARKER),
        "Created by Mouse Me Studio\n",
    )
    .map_err(|e| e.to_string())?;

    let staged = staging.keep();
    let retired = dest_root.join(format!(".{folder}.mouse-me-old"));
    if dest.exists() {
        let _ = fs::remove_dir_all(&retired);
        if let Err(error) = fs::rename(&dest, &retired) {
            let _ = fs::remove_dir_all(&staged);
            return Err(format!("Could not replace the installed theme: {error}"));
        }
    }
    if let Err(error) = fs::rename(&staged, &dest) {
        if retired.exists() {
            let _ = fs::rename(&retired, &dest);
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(format!("Could not install the theme: {error}"));
    }
    let _ = fs::remove_dir_all(&retired);
    Ok(folder)
}

fn single_line(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

/// Opens an installed XCursor theme as a Studio project so it can be remixed.
pub fn load_theme_project(theme_dir: &Path) -> Result<StudioProject, String> {
    let cursors = theme_dir.join("cursors");
    if !cursors.is_dir() {
        return Err("That theme has no XCursor files to open.".into());
    }
    let index = fs::read_to_string(theme_dir.join("index.theme")).unwrap_or_default();
    let field = |key: &str| {
        index.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().to_string())
        })
    };
    let folder = theme_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = field("Name")
        .filter(|name| !name.is_empty())
        .unwrap_or(folder);
    let ours = theme_dir.join(STUDIO_MARKER).exists();

    let mut project = StudioProject {
        name: if ours { name } else { format!("{name} Remix") },
        comment: field("Comment").unwrap_or_default(),
        ..StudioProject::default()
    };
    for def in STUDIO_ROLES {
        let candidates = std::iter::once(def.id).chain(def.aliases.iter().copied());
        let Some(frames) = candidates
            .filter(|name| !name.is_empty())
            .find_map(|name| parse_xcursor_frames(&cursors.join(name), 48))
        else {
            continue;
        };
        let (_, ox, oy) = square_pad(&frames[0].image);
        let xhot = frames[0].xhot + ox;
        let yhot = frames[0].yhot + oy;
        let delay_ms = frames[0].delay_ms.clamp(10, 2000);
        let images: Vec<CursorImage> = frames
            .into_iter()
            .take(MAX_FRAMES)
            .map(|frame| frame.image)
            .collect();
        project.add_frames(def.id, images, true)?;
        project.set_hotspot(def.id, xhot as i64, yhot as i64);
        project.set_delay(def.id, delay_ms);
    }
    if project.roles.is_empty() {
        return Err("No cursors in that theme could be read.".into());
    }
    Ok(project)
}

#[derive(Serialize, Deserialize)]
struct DraftManifest {
    version: u32,
    name: String,
    comment: String,
    sizes: Vec<u32>,
    fill_missing: bool,
    roles: Vec<DraftRole>,
}

#[derive(Serialize, Deserialize)]
struct DraftRole {
    id: String,
    xhot: u32,
    yhot: u32,
    delay_ms: u32,
    frames: usize,
}

pub fn draft_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("mouse-me").join("studio-draft"))
}

/// Saves the project as `project.json` plus one PNG per frame. The draft is
/// written beside the old one and swapped in, so a crash keeps the last good
/// copy.
pub fn save_draft(project: &StudioProject, dir: &Path) -> Result<(), String> {
    let parent = dir.parent().ok_or("Invalid draft folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".studio-draft-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;

    let mut roles = Vec::new();
    for def in STUDIO_ROLES {
        let Some(asset) = project.roles.get(def.id) else {
            continue;
        };
        for (index, frame) in asset.frames.iter().enumerate() {
            image::save_buffer(
                staging.path().join(format!("{}-{index}.png", def.id)),
                &frame.rgba,
                frame.width,
                frame.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("Could not save draft frame: {e}"))?;
        }
        roles.push(DraftRole {
            id: def.id.to_string(),
            xhot: asset.xhot,
            yhot: asset.yhot,
            delay_ms: asset.delay_ms,
            frames: asset.frames.len(),
        });
    }
    let manifest = DraftManifest {
        version: 1,
        name: project.name.clone(),
        comment: project.comment.clone(),
        sizes: project.sizes.clone(),
        fill_missing: project.fill_missing,
        roles,
    };
    let raw = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(staging.path().join("project.json"), raw).map_err(|e| e.to_string())?;

    // The old draft is moved aside, not deleted, until the new one is in
    // place. If that is interrupted, `load_draft` falls back to it.
    let staged = staging.keep();
    let retired = retired_draft_dir(dir);
    if dir.exists() {
        if retired.exists() {
            fs::remove_dir_all(&retired).map_err(|e| e.to_string())?;
        }
        if let Err(e) = fs::rename(dir, &retired) {
            let _ = fs::remove_dir_all(&staged);
            return Err(e.to_string());
        }
    }
    if let Err(e) = fs::rename(&staged, dir) {
        if !dir.exists() && retired.exists() {
            let _ = fs::rename(&retired, dir);
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(e.to_string());
    }
    let _ = fs::remove_dir_all(&retired);
    Ok(())
}

fn retired_draft_dir(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    dir.with_file_name(name)
}

/// Loads the draft, or the copy `save_draft` set aside if a save was cut off
/// before the new draft was in place.
pub fn load_draft(dir: &Path) -> Option<StudioProject> {
    read_draft(dir).or_else(|| read_draft(&retired_draft_dir(dir)))
}

fn read_draft(dir: &Path) -> Option<StudioProject> {
    let raw = fs::read_to_string(dir.join("project.json")).ok()?;
    let manifest: DraftManifest = serde_json::from_str(&raw).ok()?;
    let mut project = StudioProject {
        name: manifest.name,
        comment: manifest.comment,
        sizes: manifest.sizes,
        fill_missing: manifest.fill_missing,
        roles: HashMap::new(),
    };
    for role in manifest.roles {
        if role_def(&role.id).is_none() {
            continue;
        }
        let frames: Vec<CursorImage> = (0..role.frames.min(MAX_FRAMES))
            .filter_map(|index| {
                load_png_as_cursor(&dir.join(format!("{}-{index}.png", role.id))).ok()
            })
            .collect();
        if frames.is_empty() || project.add_frames(&role.id, frames, true).is_err() {
            continue;
        }
        project.set_hotspot(&role.id, role.xhot as i64, role.yhot as i64);
        project.set_delay(&role.id, role.delay_ms);
    }
    Some(project)
}

/// Orders frame files by name, comparing runs of digits as numbers, so
/// `frame2.png` comes before `frame10.png` whether or not they are padded.
pub fn sort_frame_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| {
        let name = |path: &PathBuf| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        };
        natural_cmp(&name(a), &name(b)).then_with(|| a.cmp(b))
    });
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.next_if(|c| c.is_ascii_digit()) {
                        digits.push(c);
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}
