use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use mouse_me::core::applier::{apply_hypr_cursor_prefs, apply_with_targets};
use mouse_me::core::importer::{import_cursor_pack, is_safe_theme_name};
use mouse_me::core::scanner::{get_active_cursor, scan_cursor_themes};
use mouse_me::core::settings::AppSettings;
use mouse_me::core::studio::{export_theme, load_png_as_cursor, STUDIO_ROLES};
use mouse_me::core::types::CursorImage;

slint::include_modules!();

static CURSOR_ARCHIVE_EXTENSIONS: [&str; 7] =
    ["zip", "tar.gz", "tgz", "tar.xz", "txz", "tar.bz2", "tar"];
static IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

pub fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;
    let window_handle = main_window.as_weak();
    let studio_images: Rc<RefCell<HashMap<String, CursorImage>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let studio_role: Rc<RefCell<String>> = Rc::new(RefCell::new("left_ptr".into()));

    let settings = AppSettings::load();
    apply_settings_to_window(&main_window, &settings);
    main_window.set_app_version(SharedString::from(env!("CARGO_PKG_VERSION")));
    refresh_studio_roles(&main_window, &studio_images.borrow());
    refresh_ui_state(&main_window);

    {
        let wh = window_handle.clone();
        main_window.on_apply_theme(move |theme_name, size| {
            let name_str = theme_name.as_str().to_string();
            let size_u32 = size as u32;
            let Some(w) = wh.upgrade() else { return };
            let settings = read_settings(&w);
            match apply_with_targets(&name_str, size_u32, &settings.apply_targets()) {
                Ok(()) => {
                    w.set_active_theme_name(SharedString::from(&name_str));
                    w.set_active_size(size);
                    set_status(&w, false, format!("Applied {} at {}px", name_str, size));
                    refresh_ui_state(&w);
                }
                Err(e) => set_status(&w, true, format!("{}", e)),
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_import_theme_dialog(move || {
            let Some(w) = wh.upgrade() else { return };
            let file = rfd::FileDialog::new()
                .set_title("Select cursor archive")
                .add_filter("Cursor archives", CURSOR_ARCHIVE_EXTENSIONS.as_slice())
                .pick_file();
            if let Some(path) = file {
                handle_import(&w, &path);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_import_folder_dialog(move || {
            let Some(w) = wh.upgrade() else { return };
            let folder = rfd::FileDialog::new()
                .set_title("Select unpacked cursor theme folder")
                .pick_folder();
            if let Some(path) = folder {
                handle_import(&w, &path);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_import_dropped(move |data| {
            let Some(w) = wh.upgrade() else { return };
            let paths = paths_from_transfer(&data);
            if paths.is_empty() {
                set_status(
                    &w,
                    true,
                    "That drop had no file path. Use Browse archive or Browse folder.".into(),
                );
                return;
            }
            for path in paths {
                handle_import(&w, &path);
            }
        });
    }

    {
        let wh = window_handle.clone();
        let studio_images = studio_images.clone();
        let studio_role = studio_role.clone();
        main_window.on_studio_file_dropped(move |data| {
            let Some(w) = wh.upgrade() else { return };
            let paths = paths_from_transfer(&data);
            let Some(path) = paths.into_iter().next() else {
                set_status(&w, true, "Drop a PNG or JPEG file onto the canvas.".into());
                return;
            };
            assign_studio_image(&w, &studio_images, &studio_role, &path);
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_delete_theme(move |theme_name| {
            let name_str = theme_name.as_str().to_string();
            let Some(w) = wh.upgrade() else { return };
            if !is_safe_theme_name(&name_str) {
                set_status(
                    &w,
                    true,
                    "That theme name is not a safe user theme path.".into(),
                );
                return;
            }
            let active = get_active_cursor();
            if name_str.eq_ignore_ascii_case(&active.theme_name) {
                set_status(
                    &w,
                    true,
                    "Apply another theme before removing the active theme.".into(),
                );
                return;
            }
            let Some(home) = dirs::home_dir() else {
                set_status(&w, true, "Could not locate home directory".into());
                return;
            };
            let paths = [
                home.join(".local")
                    .join("share")
                    .join("icons")
                    .join(&name_str),
                home.join(".icons").join(&name_str),
            ];
            let mut found = false;
            let mut errors = Vec::new();
            for path in paths {
                if std::fs::symlink_metadata(&path).is_ok() {
                    found = true;
                    if let Err(error) = std::fs::remove_dir_all(&path) {
                        errors.push(format!("{}: {}", path.display(), error));
                    }
                }
            }
            if !errors.is_empty() {
                set_status(
                    &w,
                    true,
                    format!("Could not remove {}: {}", name_str, errors.join(", ")),
                );
            } else if !found {
                set_status(
                    &w,
                    true,
                    format!("Could not find {} in the user icon directories", name_str),
                );
            } else {
                set_status(&w, false, format!("Removed {}", name_str));
                refresh_ui_state(&w);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_refresh_themes(move || {
            let Some(w) = wh.upgrade() else { return };
            if w.get_is_loading() {
                return;
            }
            w.set_is_loading(true);
            let wh2 = wh.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(40), move || {
                if let Some(w) = wh2.upgrade() {
                    refresh_ui_state(&w);
                    w.set_is_loading(false);
                    set_status(&w, false, "Library refreshed".into());
                }
            });
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_search_changed(move |query| {
            if let Some(w) = wh.upgrade() {
                w.set_search_query(query);
                refresh_ui_state(&w);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_size_changed(move |new_size| {
            let Some(w) = wh.upgrade() else { return };
            let size = new_size.clamp(1, 512);
            w.set_active_size(size);
            let mut settings = read_settings(&w);
            settings.preferred_size = size as u32;
            if let Err(error) = settings.save() {
                set_status(
                    &w,
                    true,
                    format!("Could not save size preference: {}", error),
                );
                return;
            }
            if settings.apply_size_immediately {
                let active = w.get_active_theme_name().as_str().to_string();
                if !active.is_empty() && active != "default" {
                    match apply_with_targets(&active, size as u32, &settings.apply_targets()) {
                        Ok(()) => set_status(&w, false, format!("Size set to {}px", size)),
                        Err(e) => set_status(&w, true, e),
                    }
                } else {
                    set_status(&w, false, format!("Size set to {}px", size));
                }
            } else {
                set_status(
                    &w,
                    false,
                    format!("Size staged at {}px — apply a pack to write it", size),
                );
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_page_changed(move |page| {
            if let Some(w) = wh.upgrade() {
                let mut settings = read_settings(&w);
                settings.last_page = page;
                if let Err(error) = settings.save() {
                    set_status(
                        &w,
                        true,
                        format!("Could not save page preference: {}", error),
                    );
                }
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_filter_changed(move || {
            if let Some(w) = wh.upgrade() {
                let settings = read_settings(&w);
                if let Err(error) = settings.save() {
                    set_status(
                        &w,
                        true,
                        format!("Could not save library filter: {}", error),
                    );
                }
                refresh_ui_state(&w);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_settings_changed(move || {
            if let Some(w) = wh.upgrade() {
                let settings = read_settings(&w);
                match settings.save() {
                    Ok(()) => set_status(&w, false, "Settings saved".into()),
                    Err(error) => {
                        set_status(&w, true, format!("Could not save settings: {}", error))
                    }
                }
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_hypr_prefs_changed(move || {
            let Some(w) = wh.upgrade() else { return };
            let settings = read_settings(&w);
            if let Err(error) = settings.save() {
                set_status(
                    &w,
                    true,
                    format!("Could not save Hyprland preferences: {}", error),
                );
                return;
            }
            match apply_hypr_cursor_prefs(
                settings.enable_hyprcursor,
                settings.hide_on_key_press,
                settings.hide_on_touch,
                settings.no_hardware_cursors,
                settings.inactive_timeout,
            ) {
                Ok(()) => set_status(&w, false, "Hyprland cursor preferences applied".into()),
                Err(e) => set_status(&w, true, e),
            }
        });
    }

    {
        let wh = window_handle.clone();
        let studio_images = studio_images.clone();
        let studio_role = studio_role.clone();
        main_window.on_studio_pick_image(move || {
            let Some(w) = wh.upgrade() else { return };
            let file = rfd::FileDialog::new()
                .set_title("Choose a PNG or JPEG cursor frame")
                .add_filter("Images", IMAGE_EXTENSIONS.as_slice())
                .pick_file();
            if let Some(path) = file {
                assign_studio_image(&w, &studio_images, &studio_role, &path);
            }
        });
    }

    {
        let wh = window_handle.clone();
        let studio_images = studio_images.clone();
        let studio_role = studio_role.clone();
        main_window.on_studio_select_role(move |role_id| {
            let role = role_id.as_str().to_string();
            *studio_role.borrow_mut() = role.clone();
            if let Some(w) = wh.upgrade() {
                w.set_studio_selected_role(SharedString::from(&role));
                show_selected_preview(&w, &studio_images.borrow(), &role);
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_studio_set_hotspot(move |nx, ny| {
            let Some(w) = wh.upgrade() else { return };
            let wpx = w.get_studio_img_w().max(1);
            let hpx = w.get_studio_img_h().max(1);
            let x = (nx.clamp(0.0, 1.0) * wpx as f32).round() as i32;
            let y = (ny.clamp(0.0, 1.0) * hpx as f32).round() as i32;
            w.set_studio_hotspot_x(x.clamp(0, wpx - 1));
            w.set_studio_hotspot_y(y.clamp(0, hpx - 1));
        });
    }

    {
        let wh = window_handle.clone();
        let studio_images = studio_images.clone();
        let studio_role = studio_role.clone();
        main_window.on_studio_use_for_all(move || {
            let Some(w) = wh.upgrade() else { return };
            let current = studio_role.borrow().clone();
            let mut images = studio_images.borrow_mut();
            let Some(src) = images.get(&current).cloned() else {
                set_status(&w, true, "Assign a PNG to this role first.".into());
                return;
            };
            for role in STUDIO_ROLES {
                images.insert(role.id.to_string(), src.clone());
            }
            drop(images);
            refresh_studio_roles(&w, &studio_images.borrow());
            set_status(&w, false, "Same frame applied to every role.".into());
        });
    }

    {
        let wh = window_handle.clone();
        let studio_images = studio_images.clone();
        main_window.on_studio_export(move || {
            let Some(w) = wh.upgrade() else { return };
            let name = w.get_studio_name().as_str().to_string();
            let comment = w.get_studio_comment().as_str().to_string();
            let size = w.get_studio_size() as u32;
            let xhot = w.get_studio_hotspot_x() as u32;
            let yhot = w.get_studio_hotspot_y() as u32;
            let images = studio_images.borrow();
            match export_theme(&name, &comment, size, xhot, yhot, &images) {
                Ok(folder) => {
                    drop(images);
                    set_status(&w, false, format!("Installed {}", folder));
                    refresh_ui_state(&w);
                    let settings = read_settings(&w);
                    if settings.auto_apply_on_import {
                        match apply_with_targets(
                            &folder,
                            settings.preferred_size,
                            &settings.apply_targets(),
                        ) {
                            Ok(()) => refresh_ui_state(&w),
                            Err(error) => set_status(&w, true, error),
                        }
                    }
                }
                Err(e) => set_status(&w, true, e),
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_themes_folder(move || {
            let Some(w) = wh.upgrade() else { return };
            if let Some(home) = dirs::home_dir() {
                let dir = home.join(".local").join("share").join("icons");
                if let Err(error) = std::fs::create_dir_all(&dir) {
                    set_status(
                        &w,
                        true,
                        format!("Could not create {}: {}", dir.display(), error),
                    );
                    return;
                }
                let opened = Command::new("xdg-open").arg(&dir).spawn().is_ok();
                if opened {
                    set_status(&w, false, format!("Opened {}", dir.display()));
                } else {
                    set_status(&w, true, format!("Could not open {}", dir.display()));
                }
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_check_for_update(move || {
            let Some(w) = wh.upgrade() else { return };
            if w.get_checking_update() {
                return;
            }
            w.set_checking_update(true);
            w.set_update_note(SharedString::from("Checking GitHub…"));
            let current = env!("CARGO_PKG_VERSION").to_string();
            let wh2 = wh.clone();
            std::thread::spawn(move || {
                let result = latest_release_tag();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(w) = wh2.upgrade() else { return };
                    w.set_checking_update(false);
                    match result {
                        Ok(tag) => {
                            let latest = tag.trim_start_matches('v');
                            if version_cmp(latest, &current) == std::cmp::Ordering::Greater {
                                if open_url("https://github.com/Grenish/mouse-me/releases/latest") {
                                    w.set_update_note(SharedString::from(format!(
                                        "{} is available. Opening the release page.",
                                        tag
                                    )));
                                } else {
                                    w.set_update_note(SharedString::from(format!(
                                        "{} is available, but the release page could not be opened.",
                                        tag
                                    )));
                                }
                            } else {
                                w.set_update_note(SharedString::from(format!(
                                    "You're on {}. That's the latest published build.",
                                    current
                                )));
                            }
                        }
                        Err(kind) if kind == "no-release" => {
                            w.set_update_note(SharedString::from(format!(
                                "You're on {}. No newer release has been published yet.",
                                current
                            )));
                        }
                        Err(_) => {
                            w.set_update_note(SharedString::from(
                                "Couldn't reach GitHub. Check the connection and try again.",
                            ));
                        }
                    }
                });
            });
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_support(move || {
            let Some(w) = wh.upgrade() else { return };
            if open_url("https://buymeacoffee.com/grenish") {
                set_status(&w, false, "Opened the support page".into());
            } else {
                set_status(&w, true, "Couldn't open buymeacoffee.com/grenish".into());
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_x(move || {
            let Some(w) = wh.upgrade() else { return };
            if open_url("https://x.com/grenish_rai") {
                set_status(&w, false, "Opened X".into());
            } else {
                set_status(&w, true, "Couldn't open x.com/grenish_rai".into());
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_contribute(move || {
            let Some(w) = wh.upgrade() else { return };
            if open_url("https://github.com/grenish/mouse-me") {
                set_status(&w, false, "Opened GitHub".into());
            } else {
                set_status(&w, true, "Couldn't open github.com/grenish/mouse-me".into());
            }
        });
    }

    main_window.run()?;
    Ok(())
}

fn apply_settings_to_window(window: &MainWindow, settings: &AppSettings) {
    window.set_apply_hyprland(settings.apply_hyprland);
    window.set_apply_gsettings(settings.apply_gsettings);
    window.set_apply_gtk(settings.apply_gtk);
    window.set_apply_qt(settings.apply_qt);
    window.set_apply_environment(settings.apply_environment);
    window.set_apply_xresources(settings.apply_xresources);
    window.set_apply_default_index(settings.apply_default_index);
    window.set_apply_flatpak(settings.apply_flatpak);
    window.set_auto_apply_on_import(settings.auto_apply_on_import);
    window.set_apply_size_immediately(settings.apply_size_immediately);
    window.set_show_user_themes(settings.show_user_themes);
    window.set_show_system_themes(settings.show_system_themes);
    window.set_library_filter(SharedString::from(&settings.library_filter));
    window.set_library_type(SharedString::from(&settings.library_type));
    window.set_enable_hyprcursor(settings.enable_hyprcursor);
    window.set_hide_on_key_press(settings.hide_on_key_press);
    window.set_hide_on_touch(settings.hide_on_touch);
    window.set_no_hardware_cursors(settings.no_hardware_cursors);
    window.set_inactive_timeout(settings.inactive_timeout);
    window.set_page(settings.last_page.clamp(0, 5));
    window.set_active_size(settings.preferred_size as i32);
}

fn read_settings(window: &MainWindow) -> AppSettings {
    AppSettings {
        apply_hyprland: window.get_apply_hyprland(),
        apply_gsettings: window.get_apply_gsettings(),
        apply_gtk: window.get_apply_gtk(),
        apply_qt: window.get_apply_qt(),
        apply_environment: window.get_apply_environment(),
        apply_xresources: window.get_apply_xresources(),
        apply_default_index: window.get_apply_default_index(),
        apply_flatpak: window.get_apply_flatpak(),
        auto_apply_on_import: window.get_auto_apply_on_import(),
        apply_size_immediately: window.get_apply_size_immediately(),
        show_user_themes: window.get_show_user_themes(),
        show_system_themes: window.get_show_system_themes(),
        library_filter: window.get_library_filter().to_string(),
        library_type: window.get_library_type().to_string(),
        last_page: window.get_page(),
        preferred_size: window.get_active_size().max(16) as u32,
        enable_hyprcursor: window.get_enable_hyprcursor(),
        hide_on_key_press: window.get_hide_on_key_press(),
        hide_on_touch: window.get_hide_on_touch(),
        no_hardware_cursors: window.get_no_hardware_cursors(),
        inactive_timeout: window.get_inactive_timeout(),
    }
}

fn handle_import(window: &MainWindow, path: &Path) {
    match import_cursor_pack(path) {
        Ok(imported) => {
            let joined = imported.join(", ");
            set_status(window, false, format!("Installed {}", joined));
            window.set_import_note(SharedString::from(format!("Installed {}.", joined)));
            refresh_ui_state(window);
            let settings = read_settings(window);
            if settings.auto_apply_on_import {
                if let Some(name) = imported.first() {
                    match apply_with_targets(
                        name,
                        settings.preferred_size,
                        &settings.apply_targets(),
                    ) {
                        Ok(()) => refresh_ui_state(window),
                        Err(error) => set_status(window, true, error),
                    }
                }
            }
        }
        Err(err) => {
            window.set_import_note(SharedString::from(err.clone()));
            set_status(window, true, err);
        }
    }
}

fn assign_studio_image(
    window: &MainWindow,
    studio_images: &Rc<RefCell<HashMap<String, CursorImage>>>,
    studio_role: &Rc<RefCell<String>>,
    path: &Path,
) {
    match load_png_as_cursor(path) {
        Ok(img) => {
            let role = studio_role.borrow().clone();
            let w = img.width as i32;
            let h = img.height as i32;
            studio_images.borrow_mut().insert(role.clone(), img.clone());
            window.set_studio_img_w(w);
            window.set_studio_img_h(h);
            if window.get_studio_hotspot_x() >= w {
                window.set_studio_hotspot_x(0);
            }
            if window.get_studio_hotspot_y() >= h {
                window.set_studio_hotspot_y(0);
            }
            let (preview, has) = to_slint_image(Some(&img));
            window.set_studio_preview(preview);
            window.set_studio_has_preview(has);
            refresh_studio_roles(window, &studio_images.borrow());
            set_status(
                window,
                false,
                format!("Loaded {} for {}", path.display(), role),
            );
        }
        Err(e) => set_status(window, true, e),
    }
}

fn show_selected_preview(window: &MainWindow, images: &HashMap<String, CursorImage>, role: &str) {
    if let Some(img) = images.get(role) {
        window.set_studio_img_w(img.width as i32);
        window.set_studio_img_h(img.height as i32);
        let (preview, has) = to_slint_image(Some(img));
        window.set_studio_preview(preview);
        window.set_studio_has_preview(has);
    } else {
        window.set_studio_has_preview(false);
        window.set_studio_preview(Image::default());
    }
}

fn refresh_studio_roles(window: &MainWindow, images: &HashMap<String, CursorImage>) {
    let items: Vec<StudioRole> = STUDIO_ROLES
        .iter()
        .map(|role| StudioRole {
            id: SharedString::from(role.id),
            label: SharedString::from(role.label),
            assigned: images.contains_key(role.id),
        })
        .collect();
    window.set_studio_roles(ModelRc::from(Rc::new(VecModel::from(items))));
}

fn refresh_ui_state(window: &MainWindow) {
    let active_state = get_active_cursor();
    window.set_active_theme_name(SharedString::from(&active_state.theme_name));
    if window.get_active_size() <= 0 {
        window.set_active_size(active_state.size as i32);
    }

    let all_themes = scan_cursor_themes();
    let search_q = window.get_search_query().to_lowercase();
    let filter = window.get_library_filter().to_string();
    let type_filter = window.get_library_type().to_string();
    let show_user = window.get_show_user_themes();
    let show_system = window.get_show_system_themes();

    let mut ui_items = Vec::new();

    for theme in all_themes {
        let is_user = theme.is_user;
        if is_user && !show_user {
            continue;
        }
        if !is_user && !show_system {
            continue;
        }
        if filter == "user" && !is_user {
            continue;
        }
        if filter == "system" && is_user {
            continue;
        }

        let type_name = theme.cursor_type.to_string().to_lowercase();
        if type_filter == "xcursor" && type_name != "xcursor" {
            continue;
        }
        if type_filter == "hyprcursor" && type_name != "hyprcursor" {
            continue;
        }

        if !search_q.is_empty() {
            let match_name = theme.name.to_lowercase().contains(&search_q);
            let match_disp = theme.display_name.to_lowercase().contains(&search_q);
            let match_comment = theme.comment.to_lowercase().contains(&search_q);
            if !match_name && !match_disp && !match_comment {
                continue;
            }
        }

        let is_active = theme.name.eq_ignore_ascii_case(&active_state.theme_name);
        let (def_img, has_preview) = to_slint_image(theme.preview_default.as_ref());
        let (ptr_img, _) = to_slint_image(theme.preview_pointer.as_ref());
        let (wait_img, _) = to_slint_image(theme.preview_wait.as_ref());
        let (txt_img, _) = to_slint_image(theme.preview_text.as_ref());
        let path_str = theme.path.to_string_lossy().into_owned();

        ui_items.push(ThemeItem {
            name: SharedString::from(&theme.name),
            display_name: SharedString::from(&theme.display_name),
            comment: SharedString::from(&theme.comment),
            cursor_type: SharedString::from(theme.cursor_type.to_string()),
            is_active,
            is_deletable: theme.is_user && !is_active,
            is_user,
            path: SharedString::from(path_str),
            preview_default: def_img,
            preview_pointer: ptr_img,
            preview_wait: wait_img,
            preview_text: txt_img,
            has_preview,
        });
    }

    window.set_theme_count(ui_items.len() as i32);
    window.set_themes(ModelRc::from(Rc::new(VecModel::from(ui_items))));
}

fn to_slint_image(cursor_img: Option<&CursorImage>) -> (Image, bool) {
    match cursor_img {
        Some(img) if img.width > 0 && img.height > 0 => {
            let mut pixel_buffer = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
            let slice = pixel_buffer.make_mut_bytes();
            if slice.len() == img.rgba.len() {
                slice.copy_from_slice(&img.rgba);
                (Image::from_rgba8(pixel_buffer), true)
            } else {
                (Image::default(), false)
            }
        }
        _ => (Image::default(), false),
    }
}

fn open_url(url: &str) -> bool {
    Command::new("xdg-open").arg(url).spawn().is_ok()
}

fn latest_release_tag() -> Result<String, String> {
    let output = Command::new("curl")
        .args([
            "-sS",
            "-f",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: mouse-me",
            "https://api.github.com/repos/Grenish/mouse-me/releases/latest",
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        if output.status.code() == Some(404) {
            return Err("no-release".into());
        }
        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if details.is_empty() {
            format!("curl failed with status {}", output.status)
        } else {
            details
        });
    }

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let tag = parsed
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if tag.is_empty() {
        return Err("no-release".into());
    }
    Ok(tag.to_string())
}

fn version_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |s: &str| {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .collect::<Vec<_>>()
    };
    let mut a = parse(left);
    let mut b = parse(right);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a.cmp(&b)
}

fn set_status(window: &MainWindow, is_error: bool, message: String) {
    window.set_status_is_error(is_error);
    window.set_status_message(SharedString::from(message));
}

fn paths_from_transfer(data: &slint::DataTransfer) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(text) = data.plain_text() {
        for line in text.split(|c| c == '\n' || c == '\r' || c == '\t') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let decoded = decode_uri(line);
            if decoded.is_empty() {
                continue;
            }
            let path = PathBuf::from(&decoded);
            if path.exists() {
                paths.push(path);
            }
        }
    }
    paths
}

fn decode_uri(raw: &str) -> String {
    let s = raw.trim().trim_matches('"').trim_matches('\'');
    let s = s.strip_prefix("file://").unwrap_or(s);
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
