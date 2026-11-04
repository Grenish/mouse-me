use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, VecModel};

use mouse_me::core::studio::{
    draft_dir, export_project, load_draft, load_png_as_cursor, load_theme_project, role_def,
    sanitize_theme_name, save_draft, sort_frame_paths, StudioProject, SIZE_CHOICES, STUDIO_ROLES,
};
use mouse_me::core::types::{CursorImage, CursorTheme, CursorType};

use super::{
    apply_theme_async, paths_from_transfer, read_settings, rescan_library, set_status,
    to_slint_image, MainWindow, StudioRole, StudioSize, IMAGE_EXTENSIONS,
};

const DRAFT_SAVE_DELAY: Duration = Duration::from_millis(600);
const CONFIRM_WINDOW: Duration = Duration::from_secs(4);

struct Studio {
    project: StudioProject,
    role: String,
    frame: usize,
    /// A destructive action waiting for a second click, and when it was asked.
    pending: Option<(&'static str, Instant)>,
    save_timer: Timer,
}

type Shared = Rc<RefCell<Studio>>;

pub(super) fn install(window: &MainWindow, library_cache: &Arc<Mutex<Vec<CursorTheme>>>) {
    let project = draft_dir()
        .and_then(|dir| load_draft(&dir))
        .unwrap_or_default();
    let studio: Shared = Rc::new(RefCell::new(Studio {
        project,
        role: STUDIO_ROLES[0].id.to_string(),
        frame: 0,
        pending: None,
        save_timer: Timer::default(),
    }));

    window.set_studio_mat_tile(mat_tile());
    paint_meta(window, &studio.borrow());
    paint(window, &studio.borrow());

    let wh = window.as_weak();

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_select_role(move |role| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            st.role = role.to_string();
            st.frame = 0;
            // A Clear armed on one role must not fire on another.
            st.pending = None;
            paint(&w, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_select_frame(move |index| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            st.frame = index.max(0) as usize;
            w.set_studio_frame_index(st.frame as i32);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_pick_frames(move |replace| {
            let Some(w) = wh.upgrade() else { return };
            let label = role_label(&studio.borrow().role);
            let Some(paths) = rfd::FileDialog::new()
                .set_title(format!("Choose frames for {label}"))
                .add_filter("Images", IMAGE_EXTENSIONS.as_slice())
                .pick_files()
            else {
                return;
            };
            add_paths(&w, &studio, paths, replace);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_files_dropped(move |data| {
            let Some(w) = wh.upgrade() else { return };
            let paths = paths_from_transfer(&data);
            if paths.is_empty() {
                set_status(&w, true, "That drop had no image files.".into());
                return;
            }
            add_paths(&w, &studio, paths, false);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_remove_frame(move || {
            let Some(w) = wh.upgrade() else { return };
            edit(&w, &studio, |st| {
                let (role, frame) = (st.role.clone(), st.frame);
                st.project.remove_frame(&role, frame);
                st.frame = frame.saturating_sub(1);
            });
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_move_frame(move |delta| {
            let Some(w) = wh.upgrade() else { return };
            edit(&w, &studio, |st| {
                let role = st.role.clone();
                st.frame = st.project.move_frame(&role, st.frame, delta);
            });
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_clear_role(move || {
            let Some(w) = wh.upgrade() else { return };
            let (label, has_art) = {
                let st = studio.borrow();
                (
                    role_label(&st.role),
                    st.project.roles.contains_key(&st.role),
                )
            };
            if has_art
                && !confirm(
                    &w,
                    &studio,
                    "clear",
                    &format!("Click Clear again to remove every frame from {label}."),
                )
            {
                return;
            }
            edit(&w, &studio, |st| {
                let role = st.role.clone();
                st.project.roles.remove(&role);
                st.frame = 0;
            });
            set_status(&w, false, format!("Cleared {label}"));
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_fill_empty(move || {
            let Some(w) = wh.upgrade() else { return };
            let role = studio.borrow().role.clone();
            if !studio.borrow().project.roles.contains_key(&role) {
                set_status(
                    &w,
                    true,
                    format!(
                        "Give {} some art first, then fill the rest from it.",
                        role_label(&role)
                    ),
                );
                return;
            }
            let mut filled = 0;
            edit(&w, &studio, |st| filled = st.project.fill_empty_from(&role));
            let message = match filled {
                0 => "Every role already has art.".to_string(),
                1 => format!("Copied {} into 1 empty role", role_label(&role)),
                n => format!("Copied {} into {n} empty roles", role_label(&role)),
            };
            set_status(&w, false, message);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_set_hotspot(move |nx, ny| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            let role = st.role.clone();
            let Some(edge) = st.project.roles.get(&role).map(|asset| asset.edge()) else {
                return;
            };
            let to_pixel = |n: f32| (n.clamp(0.0, 1.0) * edge as f32).floor() as i64;
            st.project.set_hotspot(&role, to_pixel(nx), to_pixel(ny));
            paint_hotspot(&w, &st);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_nudge_hotspot(move |dx, dy| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            let role = st.role.clone();
            let Some((x, y)) = st
                .project
                .roles
                .get(&role)
                .map(|asset| (asset.xhot as i64, asset.yhot as i64))
            else {
                return;
            };
            st.project.set_hotspot(&role, x + dx as i64, y + dy as i64);
            paint_hotspot(&w, &st);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_auto_hotspot(move || {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            let role = st.role.clone();
            st.project.auto_hotspot(&role);
            paint_hotspot(&w, &st);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_set_delay(move |delay| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            let role = st.role.clone();
            st.project.set_delay(&role, delay.max(0) as u32);
            w.set_studio_delay(delay);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_toggle_size(move |px| {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            let px = px.max(0) as u32;
            if st.project.sizes.contains(&px) {
                if st.project.sizes.len() == 1 {
                    set_status(&w, true, "A theme needs at least one size.".into());
                    return;
                }
                st.project.sizes.retain(|size| *size != px);
            } else {
                st.project.sizes.push(px);
                st.project.sizes.sort_unstable();
            }
            paint_sizes(&w, &st.project);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_meta_changed(move || {
            let Some(w) = wh.upgrade() else { return };
            let mut st = studio.borrow_mut();
            read_meta(&w, &mut st.project);
            paint_summary(&w, &st.project);
            schedule_save(&studio, &st);
        });
    }

    {
        let (wh, studio) = (wh.clone(), studio.clone());
        window.on_studio_new_project(move || {
            let Some(w) = wh.upgrade() else { return };
            if !confirm(
                &w,
                &studio,
                "new",
                "Click New again to clear the Studio draft.",
            ) {
                return;
            }
            replace_project(&w, &studio, StudioProject::default());
            set_status(&w, false, "Started a new Studio draft".into());
        });
    }

    {
        let (wh, studio, cache) = (wh.clone(), studio.clone(), library_cache.clone());
        window.on_studio_open_theme(move |index| {
            let Some(w) = wh.upgrade() else { return };
            let source = {
                let themes = cache.lock().unwrap_or_else(|e| e.into_inner());
                theme_sources(&themes)
                    .get(index.max(0) as usize)
                    .map(|theme| (theme.display_name.clone(), theme.path.clone()))
            };
            let Some((name, path)) = source else {
                set_status(&w, true, "Pick an installed theme to open.".into());
                return;
            };
            if !confirm(
                &w,
                &studio,
                "open",
                &format!("Click Open theme again to replace the Studio draft with {name}."),
            ) {
                return;
            }
            match load_theme_project(&path) {
                Ok(project) => {
                    let count = project.roles.len();
                    replace_project(&w, &studio, project);
                    set_status(&w, false, format!("Opened {name}: {count} roles loaded"));
                }
                Err(error) => set_status(&w, true, error),
            }
        });
    }

    {
        let (wh, studio, cache) = (wh.clone(), studio.clone(), library_cache.clone());
        window.on_studio_export(move || {
            let Some(w) = wh.upgrade() else { return };
            let result = {
                let mut st = studio.borrow_mut();
                read_meta(&w, &mut st.project);
                export_project(&st.project)
            };
            match result {
                Ok(folder) => {
                    set_status(&w, false, format!("Installed {folder}"));
                    let settings = read_settings(&w);
                    if settings.auto_apply_on_import {
                        apply_theme_async(&w, folder, settings.preferred_size, Some(&cache));
                    } else {
                        rescan_library(&w, &cache, None, false);
                    }
                }
                Err(error) => set_status(&w, true, error),
            }
        });
    }
}

/// Lists the installed XCursor themes Studio can open, in menu order.
fn theme_sources(themes: &[CursorTheme]) -> Vec<&CursorTheme> {
    let mut sources: Vec<&CursorTheme> = themes
        .iter()
        .filter(|theme| theme.cursor_type == CursorType::XCursor)
        .filter(|theme| theme.path.join("cursors").is_dir())
        .collect();
    sources.sort_by_key(|theme| theme.display_name.to_lowercase());
    sources
}

pub(super) fn paint_theme_choices(window: &MainWindow, themes: &[CursorTheme]) {
    let names: Vec<SharedString> = theme_sources(themes)
        .iter()
        .map(|theme| SharedString::from(theme.display_name.as_str()))
        .collect();
    if window.get_studio_open_index() as usize >= names.len() {
        window.set_studio_open_index(0);
    }
    window.set_studio_theme_choices(ModelRc::from(Rc::new(VecModel::from(names))));
}

fn add_paths(window: &MainWindow, studio: &Shared, mut paths: Vec<PathBuf>, replace: bool) {
    // frame2.png comes before frame10.png, padded or not.
    sort_frame_paths(&mut paths);
    let mut images: Vec<CursorImage> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in &paths {
        match load_png_as_cursor(path) {
            Ok(image) => images.push(image),
            Err(error) => skipped.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    if images.is_empty() {
        let reason = skipped
            .into_iter()
            .next()
            .unwrap_or("No images were chosen".into());
        set_status(window, true, reason);
        return;
    }

    let count = images.len();
    let role = studio.borrow().role.clone();
    let mut outcome = Ok(0);
    edit(window, studio, |st| {
        let before = if replace {
            0
        } else {
            st.project
                .roles
                .get(&role)
                .map(|a| a.frames.len())
                .unwrap_or(0)
        };
        outcome = st.project.add_frames(&role, images, replace);
        st.frame = before;
    });

    let label = role_label(&role);
    match outcome {
        Ok(_) if skipped.is_empty() => {
            let noun = if count == 1 { "frame" } else { "frames" };
            set_status(window, false, format!("Added {count} {noun} to {label}"));
        }
        Ok(_) => set_status(
            window,
            true,
            format!("Added {} to {label}, skipped {}", count, skipped.join("; ")),
        ),
        Err(error) => set_status(window, true, error),
    }
}

/// Runs a change to the project, then repaints and schedules a draft save.
fn edit(window: &MainWindow, studio: &Shared, change: impl FnOnce(&mut Studio)) {
    let mut st = studio.borrow_mut();
    change(&mut st);
    paint(window, &st);
    schedule_save(studio, &st);
}

fn replace_project(window: &MainWindow, studio: &Shared, project: StudioProject) {
    let mut st = studio.borrow_mut();
    st.project = project;
    st.role = STUDIO_ROLES[0].id.to_string();
    st.frame = 0;
    paint_meta(window, &st);
    paint(window, &st);
    schedule_save(studio, &st);
}

/// Asks for a second click within a few seconds before a destructive action.
/// An empty draft has nothing to lose, so it never asks.
fn confirm(window: &MainWindow, studio: &Shared, action: &'static str, prompt: &str) -> bool {
    let mut st = studio.borrow_mut();
    if st.project.roles.is_empty() {
        return true;
    }
    let armed = matches!(st.pending, Some((pending, at)) if pending == action && at.elapsed() < CONFIRM_WINDOW);
    if armed {
        st.pending = None;
        return true;
    }
    st.pending = Some((action, Instant::now()));
    set_status(window, false, prompt.to_string());
    false
}

fn schedule_save(studio: &Shared, st: &Studio) {
    let weak = Rc::downgrade(studio);
    st.save_timer
        .start(slint::TimerMode::SingleShot, DRAFT_SAVE_DELAY, move || {
            let (Some(studio), Some(dir)) = (weak.upgrade(), draft_dir()) else {
                return;
            };
            let saved = save_draft(&studio.borrow().project, &dir);
            if let Err(error) = saved {
                eprintln!("mouse-me: could not save the Studio draft: {error}");
            }
        });
}

fn read_meta(window: &MainWindow, project: &mut StudioProject) {
    project.name = window.get_studio_name().to_string();
    project.comment = window.get_studio_comment().to_string();
    project.fill_missing = window.get_studio_fill_missing();
}

fn paint_meta(window: &MainWindow, st: &Studio) {
    window.set_studio_name(SharedString::from(st.project.name.as_str()));
    window.set_studio_comment(SharedString::from(st.project.comment.as_str()));
    window.set_studio_fill_missing(st.project.fill_missing);
    paint_sizes(window, &st.project);
    paint_summary(window, &st.project);
}

/// The coverage line under the role list and the install path, both of
/// which follow the theme name and the fill setting.
fn paint_summary(window: &MainWindow, project: &StudioProject) {
    let note = match project.fallback_role() {
        None => "Pick a role, then drop PNG art on the mat.".to_string(),
        Some(_) if project.roles.len() == STUDIO_ROLES.len() => {
            "Every role has its own art.".to_string()
        }
        Some(role) if project.fill_missing => {
            format!("Roles without art will use {}.", role_label(role))
        }
        Some(_) => "Roles without art fall back to Adwaita.".to_string(),
    };
    window.set_studio_coverage_note(SharedString::from(note));
    window.set_studio_folder(SharedString::from(sanitize_theme_name(&project.name)));
}

fn paint_sizes(window: &MainWindow, project: &StudioProject) {
    let sizes: Vec<StudioSize> = SIZE_CHOICES
        .iter()
        .map(|&px| StudioSize {
            px: px as i32,
            on: project.sizes.contains(&px),
        })
        .collect();
    window.set_studio_sizes(ModelRc::from(Rc::new(VecModel::from(sizes))));
}

fn paint(window: &MainWindow, st: &Studio) {
    let roles: Vec<StudioRole> = STUDIO_ROLES
        .iter()
        .enumerate()
        .map(|(index, def)| {
            let asset = st.project.roles.get(def.id);
            let thumb = asset
                .map(|asset| to_slint_image(Some(&asset.frames[0].centered_glyph())).0)
                .unwrap_or_default();
            StudioRole {
                id: SharedString::from(def.id),
                label: SharedString::from(def.label),
                hint: SharedString::from(def.hint),
                group: SharedString::from(def.group),
                first: index == 0 || STUDIO_ROLES[index - 1].group != def.group,
                assigned: asset.is_some(),
                frames: asset.map(|asset| asset.frames.len() as i32).unwrap_or(0),
                thumb,
            }
        })
        .collect();
    window.set_studio_assigned(roles.iter().filter(|role| role.assigned).count() as i32);
    paint_summary(window, &st.project);
    window.set_studio_roles(ModelRc::from(Rc::new(VecModel::from(roles))));

    let def = role_def(&st.role).unwrap_or(&STUDIO_ROLES[0]);
    window.set_studio_selected_role(SharedString::from(def.id));
    window.set_studio_role_label(SharedString::from(def.label));
    window.set_studio_role_hint(SharedString::from(def.hint));

    let asset = st.project.roles.get(def.id);
    let frames: Vec<Image> = asset
        .map(|asset| {
            asset
                .frames
                .iter()
                .map(|frame| to_slint_image(Some(frame)).0)
                .collect()
        })
        .unwrap_or_default();
    let last = frames.len().saturating_sub(1);
    window.set_studio_frames(ModelRc::from(Rc::new(VecModel::from(frames))));
    window.set_studio_frame_index(st.frame.min(last) as i32);
    window.set_studio_edge(asset.map(|asset| asset.edge() as i32).unwrap_or(32));
    window.set_studio_delay(asset.map(|asset| asset.delay_ms as i32).unwrap_or(80));
    paint_hotspot(window, st);
}

fn paint_hotspot(window: &MainWindow, st: &Studio) {
    let (x, y) = st
        .project
        .roles
        .get(&st.role)
        .map(|asset| (asset.xhot as i32, asset.yhot as i32))
        .unwrap_or((0, 0));
    window.set_studio_hotspot_x(x);
    window.set_studio_hotspot_y(y);
}

fn role_label(id: &str) -> &'static str {
    role_def(id).map(|def| def.label).unwrap_or("this role")
}

/// One square of the cutting mat behind the canvas: a faint line along two
/// edges, tiled across the whole stage.
fn mat_tile() -> Image {
    const EDGE: u32 = 24;
    let mut rgba = Vec::with_capacity((EDGE * EDGE * 4) as usize);
    for y in 0..EDGE {
        for x in 0..EDGE {
            let alpha = if x == 0 || y == 0 { 10 } else { 0 };
            rgba.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    let image = CursorImage {
        width: EDGE,
        height: EDGE,
        rgba,
    };
    to_slint_image(Some(&image)).0
}
