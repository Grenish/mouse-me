use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, VecModel};

use mouse_me::core::applier::{apply_hypr_cursor_prefs, apply_with_targets};
use mouse_me::core::auth::{decode_avatar, format_joined, format_published, AuthStore, AuthUser};
use mouse_me::core::device_info::{collect_device_info, copy_to_clipboard};
use mouse_me::core::importer::{import_cursor_pack, is_safe_theme_name};
use mouse_me::core::scanner::{get_active_cursor, scan_cursor_themes};
use mouse_me::core::settings::AppSettings;
use mouse_me::core::studio::{export_theme, load_png_as_cursor, STUDIO_ROLES};
use mouse_me::core::types::{CursorImage, CursorTheme};
use mouse_me::core::updater::{self, Release};

slint::include_modules!();

static CURSOR_ARCHIVE_EXTENSIONS: [&str; 6] = ["zip", "gz", "tgz", "xz", "bz2", "tar"];
static ALL_FILE_EXTENSIONS: [&str; 1] = ["*"];
static IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];
const LIBRARY_CARD_HEIGHT: f32 = 92.0;
const LIBRARY_CARD_GAP: f32 = 10.0;

const APP_ID: &str = "mouse-me";

pub fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(error) = updater::apply_pending_update() {
        eprintln!("mouse-me: pending update failed: {error}");
    }

    let _ = slint::set_xdg_app_id(APP_ID);
    prepare_hyprland_float();

    let main_window = MainWindow::new()?;
    let window_handle = main_window.as_weak();
    let studio_images: Rc<RefCell<HashMap<String, CursorImage>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let studio_role: Rc<RefCell<String>> = Rc::new(RefCell::new("left_ptr".into()));
    let library_cache: Arc<Mutex<Vec<CursorTheme>>> = Arc::new(Mutex::new(Vec::new()));

    let settings = AppSettings::load();
    apply_settings_to_window(&main_window, &settings);
    main_window.set_app_version(SharedString::from(env!("CARGO_PKG_VERSION")));
    apply_saved_session(&main_window);
    refresh_studio_roles(&main_window, &studio_images.borrow());
    rescan_library(&main_window, &library_cache, None, false);
    refresh_device_info_state(&main_window);
    start_auto_update(&main_window);

    {
        let wh = window_handle.clone();
        main_window.on_apply_theme(move |theme_name, size| {
            let name_str = theme_name.as_str().to_string();
            let size_u32 = size.clamp(1, 512) as u32;
            let Some(w) = wh.upgrade() else { return };
            apply_theme_async(&w, name_str, size_u32);
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
        main_window.on_import_theme_dialog(move || {
            let Some(w) = wh.upgrade() else { return };
            let file = rfd::FileDialog::new()
                .set_title("Select cursor archive")
                .add_filter("Cursor archives", CURSOR_ARCHIVE_EXTENSIONS.as_slice())
                .add_filter("All files", ALL_FILE_EXTENSIONS.as_slice())
                .pick_file();
            if let Some(path) = file {
                handle_import(&w, &path, &library_cache);
            }
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
        main_window.on_import_folder_dialog(move || {
            let Some(w) = wh.upgrade() else { return };
            let folder = rfd::FileDialog::new()
                .set_title("Select unpacked cursor theme folder")
                .pick_folder();
            if let Some(path) = folder {
                handle_import(&w, &path, &library_cache);
            }
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
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
                handle_import(&w, &path, &library_cache);
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
        let library_cache = library_cache.clone();
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
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let is_theme = metadata.is_dir()
                    && (path.join("cursors").is_dir()
                        || path.join("hyprcursors").is_dir()
                        || path.join("manifest.hl").is_file());
                if !is_theme {
                    continue;
                }
                found = true;
                if let Err(error) = std::fs::remove_dir_all(&path) {
                    errors.push(format!("{}: {}", path.display(), error));
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
                rescan_library(&w, &library_cache, None, false);
            }
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
        main_window.on_refresh_themes(move || {
            let Some(w) = wh.upgrade() else { return };
            if w.get_is_loading() {
                return;
            }
            rescan_library(&w, &library_cache, None, true);
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
        main_window.on_search_changed(move |query| {
            if let Some(w) = wh.upgrade() {
                w.set_search_query(query);
                paint_library(
                    &w,
                    &library_cache.lock().unwrap_or_else(|e| e.into_inner()),
                    None,
                );
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
                    apply_theme_async(&w, active, size as u32);
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
                if page == 3 {
                    refresh_device_info_state(&w);
                }
                if page == 4 && w.get_auth_signed_in() {
                    refresh_profile(&w);
                }
            }
        });
    }

    {
        let wh = window_handle.clone();
        let library_cache = library_cache.clone();
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
                paint_library(
                    &w,
                    &library_cache.lock().unwrap_or_else(|e| e.into_inner()),
                    None,
                );
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
            let weak = w.as_weak();
            std::thread::spawn(move || {
                let result = apply_hypr_cursor_prefs(
                    settings.enable_hyprcursor,
                    settings.hide_on_key_press,
                    settings.hide_on_touch,
                    settings.no_hardware_cursors,
                    settings.inactive_timeout,
                );
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(w) = weak.upgrade() else { return };
                    match result {
                        Ok(()) => {
                            set_status(&w, false, "Hyprland cursor preferences applied".into())
                        }
                        Err(e) => set_status(&w, true, e),
                    }
                });
            });
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
        let library_cache = library_cache.clone();
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
                    rescan_library(&w, &library_cache, Some(folder.clone()), false);
                    let settings = read_settings(&w);
                    if settings.auto_apply_on_import {
                        apply_theme_async(&w, folder, settings.preferred_size);
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
            check_for_update(&w, true);
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_apply_update(move || {
            let Some(w) = wh.upgrade() else { return };
            let tag = w.get_update_tag().to_string();
            if tag.is_empty() || w.get_checking_update() {
                return;
            }
            apply_release_now(&w, tag, true);
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_dismiss_update(move || {
            let Some(w) = wh.upgrade() else { return };
            let tag = w.get_update_tag().to_string();
            w.set_update_available(false);
            if tag.is_empty() {
                w.set_update_note(SharedString::from(""));
            } else {
                w.set_update_note(SharedString::from(format!("We'll leave {tag} for later.")));
            }
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

    {
        let wh = window_handle.clone();
        main_window.on_copy_device_info(move || {
            let Some(w) = wh.upgrade() else { return };
            let info = collect_device_info();
            match copy_to_clipboard(&info.full_report) {
                Ok(()) => {
                    set_status(&w, false, "Copied device & debug info to clipboard.".into());
                }
                Err(error) => {
                    set_status(&w, true, format!("Could not copy to clipboard: {}", error));
                }
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_refresh_device_info(move || {
            let Some(w) = wh.upgrade() else { return };
            refresh_device_info_state(&w);
            set_status(&w, false, "Refreshed device & system information.".into());
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_sign_in(move || {
            let Some(w) = wh.upgrade() else { return };
            let email = w.get_auth_email().to_string();
            let password = w.get_auth_password().to_string();
            run_auth(&w, move |store| store.sign_in(&email, &password).map(Some));
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_create_account(move || {
            let Some(w) = wh.upgrade() else { return };
            let name = w.get_auth_name().to_string();
            let username = w.get_auth_username().to_string();
            let email = w.get_auth_email().to_string();
            let password = w.get_auth_password().to_string();
            let confirm = w.get_auth_confirm().to_string();
            run_auth(&w, move |store| {
                store
                    .create_account(&name, &username, &email, &password, &confirm)
                    .map(Some)
            });
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_sign_out(move || {
            let Some(w) = wh.upgrade() else { return };
            run_auth(&w, |store| store.sign_out().map(|()| None));
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_forgot_password(move || {
            let Some(w) = wh.upgrade() else { return };
            let url = AuthStore::load()
                .map(|store| store.forgot_password_url())
                .unwrap_or_else(|_| {
                    format!("{}/forgot-password", mouse_me::core::auth::DEFAULT_API_BASE)
                });
            if open_url(&url) {
                set_status(&w, false, "Opened the reset page".into());
            } else {
                w.set_auth_error(SharedString::from(format!("Couldn't open {url}")));
            }
        });
    }

    {
        let wh = window_handle.clone();
        main_window.on_open_account_page(move || {
            let Some(w) = wh.upgrade() else { return };
            match AuthStore::load().ok().and_then(|store| store.profile_url()) {
                Some(url) if open_url(&url) => set_status(&w, false, "Opened your profile".into()),
                Some(url) => w.set_auth_error(SharedString::from(format!("Couldn't open {url}"))),
                None => w.set_auth_error(SharedString::from("You're not signed in.")),
            }
        });
    }

    let pid = std::process::id();
    Timer::single_shot(Duration::from_millis(40), move || {
        center_floating_hypr_window(pid);
    });

    main_window.run()?;
    Ok(())
}

fn hyprland_is_running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

fn hypr_eval(script: &str) {
    if !hyprland_is_running() {
        return;
    }
    let _ = Command::new("hyprctl")
        .args(["eval", script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn prepare_hyprland_float() {
    hypr_eval(
        r#"
        pcall(function()
            hl.window_rule({
                name = "mouse-me-float",
                match = { class = "^(mouse-me)$" },
                float = true,
                center = true,
            })
        end)
        pcall(function()
            hl.window_rule({
                name = "mouse-me-float-title",
                match = { title = "^Mouse Me$" },
                float = true,
                center = true,
            })
        end)
        "#,
    );
}

fn center_floating_hypr_window(pid: u32) {
    hypr_eval(&format!(
        r#"
        pcall(function()
            hl.dispatch(hl.dsp.window.float({{ action = "set", window = "pid:{pid}" }}))
            hl.dispatch(hl.dsp.window.center({{ window = "pid:{pid}" }}))
        end)
        "#
    ));
}

fn apply_saved_session(window: &MainWindow) {
    match AuthStore::load() {
        Ok(store) => {
            apply_auth_session(window, store.user());
            if store.user().is_some() {
                let weak = window.as_weak();
                std::thread::spawn(move || {
                    let result = AuthStore::load().and_then(|mut store| store.refresh());
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(window) = weak.upgrade() else { return };
                        if let Ok(user) = result {
                            apply_auth_session(&window, user.as_ref());
                        }
                    });
                });
            }
        }
        Err(error) => window.set_auth_error(SharedString::from(error)),
    }
}

fn apply_auth_session(window: &MainWindow, user: Option<&AuthUser>) {
    window.set_auth_signed_in(user.is_some());
    window.set_auth_creating(false);
    window.set_auth_busy(false);
    window.set_auth_error(SharedString::from(""));
    window.set_auth_password(SharedString::from(""));
    window.set_auth_confirm(SharedString::from(""));
    match user {
        Some(user) => {
            window.set_auth_session_email(SharedString::from(user.email.as_str()));
            window.set_auth_session_name(SharedString::from(user.name.as_str()));
            window.set_auth_session_username(SharedString::from(user.username.as_str()));
            window.set_auth_email(SharedString::from(user.email.as_str()));
            window.set_auth_joined(SharedString::from(
                user.created_at
                    .as_deref()
                    .map(format_joined)
                    .unwrap_or_default(),
            ));
            window.set_auth_published_label(SharedString::from(format_published(
                user.published_count,
            )));
            if !user.name.is_empty() {
                window.set_auth_name(SharedString::from(user.name.as_str()));
            }
            if !user.username.is_empty() {
                window.set_auth_username(SharedString::from(user.username.as_str()));
            }
            load_auth_avatar(window, user.image.as_deref());
        }
        None => {
            window.set_auth_session_email(SharedString::from(""));
            window.set_auth_session_name(SharedString::from(""));
            window.set_auth_session_username(SharedString::from(""));
            window.set_auth_joined(SharedString::from(""));
            window.set_auth_published_label(SharedString::from(""));
            window.set_auth_has_avatar(false);
            window.set_auth_avatar(Image::default());
        }
    }
}

fn load_auth_avatar(window: &MainWindow, image_url: Option<&str>) {
    let Some(url) = image_url.filter(|url| !url.is_empty()) else {
        window.set_auth_has_avatar(false);
        window.set_auth_avatar(Image::default());
        return;
    };
    let url = url.to_string();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let result = AuthStore::download_bytes(&url).and_then(|bytes| decode_avatar(&bytes));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            if !window.get_auth_signed_in() {
                return;
            }
            if let Ok((width, height, rgba)) = result {
                let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
                let slice = buffer.make_mut_bytes();
                if slice.len() == rgba.len() {
                    slice.copy_from_slice(&rgba);
                    window.set_auth_avatar(Image::from_rgba8(buffer));
                    window.set_auth_has_avatar(true);
                }
            }
        });
    });
}

fn refresh_profile(window: &MainWindow) {
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let result = AuthStore::load().and_then(|mut store| store.refresh());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            if let Ok(user) = result {
                apply_auth_session(&window, user.as_ref());
            }
        });
    });
}

fn run_auth(
    window: &MainWindow,
    op: impl FnOnce(&mut AuthStore) -> Result<Option<AuthUser>, String> + Send + 'static,
) {
    if window.get_auth_busy() {
        return;
    }
    window.set_auth_busy(true);
    window.set_auth_error(SharedString::from(""));
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let result = AuthStore::load().and_then(|mut store| op(&mut store));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(user) => apply_auth_session(&window, user.as_ref()),
                Err(error) => {
                    window.set_auth_busy(false);
                    window.set_auth_error(SharedString::from(error));
                }
            }
        });
    });
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
    window.set_auto_update(settings.auto_update);
    window.set_auto_update_when(settings.auto_update_when_index());
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
        auto_update: window.get_auto_update(),
        auto_update_when: AppSettings::auto_update_when_from_index(window.get_auto_update_when()),
    }
}

fn handle_import(window: &MainWindow, path: &Path, library_cache: &Arc<Mutex<Vec<CursorTheme>>>) {
    if window.get_is_loading() {
        set_status(window, true, "An import is already in progress.".into());
        return;
    }

    window.set_is_loading(true);
    let path = path.to_path_buf();
    let settings = read_settings(window);
    let weak_window = window.as_weak();
    let library_cache = library_cache.clone();

    std::thread::spawn(move || {
        let result = match import_cursor_pack(&path) {
            Err(error) => Err(error),
            Ok(imported) => {
                let apply_result = if settings.auto_apply_on_import {
                    imported.first().map(|name| {
                        apply_with_targets(name, settings.preferred_size, &settings.apply_targets())
                    })
                } else {
                    None
                };
                Ok((imported, apply_result))
            }
        };

        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak_window.upgrade() else {
                return;
            };
            window.set_is_loading(false);
            match result {
                Err(error) => {
                    window.set_import_note(SharedString::from(error.clone()));
                    set_status(&window, true, error);
                }
                Ok((imported, apply_result)) => {
                    let joined = imported.join(", ");
                    window.set_import_note(SharedString::from(format!("Installed {}.", joined)));
                    rescan_library(&window, &library_cache, imported.first().cloned(), false);
                    match (imported.first(), apply_result) {
                        (Some(name), Some(Ok(warnings))) => {
                            mark_theme_applied(
                                &window,
                                name,
                                read_settings(&window).preferred_size,
                                &warnings,
                            );
                        }
                        (_, Some(Err(error))) => set_status(&window, true, error),
                        _ => set_status(&window, false, format!("Installed {}", joined)),
                    }
                }
            }
        });
    });
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
        if window.get_studio_hotspot_x() < 0 || window.get_studio_hotspot_x() >= img.width as i32 {
            window.set_studio_hotspot_x(0);
        }
        if window.get_studio_hotspot_y() < 0 || window.get_studio_hotspot_y() >= img.height as i32 {
            window.set_studio_hotspot_y(0);
        }
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

fn mark_theme_applied(window: &MainWindow, name: &str, size: u32, warnings: &[String]) {
    window.set_active_theme_name(SharedString::from(name));
    if warnings.is_empty() {
        set_status(window, false, format!("Applied {} at {}px", name, size));
    } else {
        set_status(
            window,
            false,
            format!(
                "Applied {} at {}px with warnings: {}",
                name,
                size,
                warnings.join("; ")
            ),
        );
    }
    window.set_active_size(size as i32);

    let themes = window.get_themes();
    let Some(model) = themes.as_any().downcast_ref::<VecModel<ThemeItem>>() else {
        return;
    };

    let mut applied_index = None;
    for i in 0..model.row_count() {
        let Some(mut item) = model.row_data(i) else {
            continue;
        };
        let is_active = item.name.eq_ignore_ascii_case(name);
        if is_active {
            applied_index = Some(i);
        }
        if item.is_active != is_active {
            item.is_active = is_active;
            item.is_deletable = item.is_user && !is_active;
            model.set_row_data(i, item);
        }
    }

    if let Some(index) = applied_index {
        reveal_library_item(window, index);
    }
}

fn reveal_library_item(window: &MainWindow, index: usize) {
    let item_top = index as f32 * (LIBRARY_CARD_HEIGHT + LIBRARY_CARD_GAP);
    let item_bottom = item_top + LIBRARY_CARD_HEIGHT;
    let view_h = window.get_library_visible_height();
    let current = window.get_library_viewport_y();
    if view_h <= 0.0 {
        return;
    }

    let mut next = current;
    if item_top < current {
        next = item_top;
    } else if item_bottom > current + view_h {
        next = (item_bottom - view_h).max(0.0);
    }

    if (next - current).abs() > 0.5 {
        window.set_library_viewport_y(next);
    }
}

fn apply_theme_async(window: &MainWindow, name: String, size: u32) {
    let settings = read_settings(window);
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let result = apply_with_targets(&name, size, &settings.apply_targets());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(warnings) => mark_theme_applied(&window, &name, size, &warnings),
                Err(error) => set_status(&window, true, error),
            }
        });
    });
}

fn rescan_library(
    window: &MainWindow,
    cache: &Arc<Mutex<Vec<CursorTheme>>>,
    applied_theme: Option<String>,
    announce: bool,
) {
    window.set_is_loading(true);
    let weak = window.as_weak();
    let cache = cache.clone();
    std::thread::spawn(move || {
        let themes = scan_cursor_themes();
        let active = get_active_cursor();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            *cache.lock().unwrap_or_else(|e| e.into_inner()) = themes;
            window.set_is_loading(false);
            let highlight = applied_theme
                .clone()
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    let name = active.theme_name.trim();
                    if name.is_empty() || name.eq_ignore_ascii_case("default") {
                        None
                    } else {
                        Some(name.to_string())
                    }
                });
            if applied_theme.is_none() {
                if !active.theme_name.is_empty() {
                    window.set_active_theme_name(SharedString::from(&active.theme_name));
                }
                if window.get_active_size() <= 0 {
                    window.set_active_size(active.size as i32);
                }
            }
            paint_library(
                &window,
                &cache.lock().unwrap_or_else(|e| e.into_inner()),
                highlight.as_deref(),
            );
            if announce {
                set_status(&window, false, "Library refreshed".into());
            }
        });
    });
}

fn paint_library(window: &MainWindow, all_themes: &[CursorTheme], applied_theme: Option<&str>) {
    let active_name = applied_theme
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .unwrap_or_else(|| window.get_active_theme_name().to_string());
    if !active_name.is_empty() {
        window.set_active_theme_name(SharedString::from(&active_name));
    }

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

        let is_active = theme.name.eq_ignore_ascii_case(&active_name);
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

    let applied_index = applied_theme.and_then(|name| {
        ui_items
            .iter()
            .position(|item| item.name.eq_ignore_ascii_case(name))
    });
    window.set_theme_count(ui_items.len() as i32);
    window.set_themes(ModelRc::from(Rc::new(VecModel::from(ui_items))));
    if let Some(index) = applied_index {
        reveal_library_item(window, index);
    }
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

fn start_auto_update(window: &MainWindow) {
    if !window.get_auto_update() {
        return;
    }
    check_for_update(window, false);
}

fn check_for_update(window: &MainWindow, interactive: bool) {
    if window.get_checking_update() {
        return;
    }
    window.set_checking_update(true);
    if interactive {
        window.set_update_available(false);
        window.set_update_note(SharedString::from("Checking GitHub…"));
    }
    let current = updater::current_version().to_string();
    let when = AppSettings::auto_update_when_from_index(window.get_auto_update_when());
    let auto = window.get_auto_update();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        if !interactive {
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
        let result = updater::latest_release();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(release) if updater::is_newer(&release.version, &current) => {
                    if interactive {
                        window.set_checking_update(false);
                        window.set_update_tag(SharedString::from(release.tag.clone()));
                        window.set_update_available(true);
                        window.set_update_note(SharedString::from(""));
                    } else if auto {
                        match when.as_str() {
                            "instantly" => apply_release_now(&window, release.tag.clone(), true),
                            "background" => apply_release_now(&window, release.tag.clone(), false),
                            _ => stage_release(&window, release),
                        }
                    } else {
                        window.set_checking_update(false);
                    }
                }
                Ok(_) => {
                    window.set_checking_update(false);
                    window.set_update_available(false);
                    if interactive {
                        window.set_update_note(SharedString::from(format!(
                            "You're on {current}. That's the latest published build."
                        )));
                    }
                }
                Err(kind) if kind == "no-release" => {
                    window.set_checking_update(false);
                    window.set_update_available(false);
                    if interactive {
                        window.set_update_note(SharedString::from(format!(
                            "You're on {current}. No newer release has been published yet."
                        )));
                    }
                }
                Err(_) => {
                    window.set_checking_update(false);
                    if interactive {
                        window.set_update_note(SharedString::from(
                            "Couldn't reach GitHub. Check the connection and try again.",
                        ));
                    }
                }
            }
        });
    });
}

fn stage_release(window: &MainWindow, release: Release) {
    window.set_checking_update(true);
    window.set_update_note(SharedString::from(format!(
        "Downloading {} for the next launch…",
        release.tag
    )));
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let result = updater::stage_update(&release);
        let tag = release.tag;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            window.set_checking_update(false);
            match result {
                Ok(_) => window.set_update_note(SharedString::from(format!(
                    "{tag} will install the next time you open Mouse Me."
                ))),
                Err(error) => window.set_update_note(SharedString::from(error)),
            }
        });
    });
}

fn apply_release_now(window: &MainWindow, tag: String, restart: bool) {
    window.set_checking_update(true);
    window.set_update_available(false);
    window.set_update_note(SharedString::from(format!("Downloading {tag}…")));
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let installed = updater::latest_release()
            .and_then(|release| updater::install_update(&release).map(|_| release.tag));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            match installed {
                Ok(installed_tag) => {
                    if restart {
                        window.set_update_note(SharedString::from(format!(
                            "{installed_tag} is installed. Restarting…"
                        )));
                        if let Err(error) = updater::relaunch() {
                            window.set_checking_update(false);
                            window.set_update_note(SharedString::from(error));
                        }
                    } else {
                        window.set_checking_update(false);
                        window.set_update_note(SharedString::from(format!(
                            "{installed_tag} is installed. Restart Mouse Me to use it."
                        )));
                    }
                }
                Err(error) => {
                    window.set_checking_update(false);
                    window.set_update_note(SharedString::from(error));
                }
            }
        });
    });
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
            if (path.is_file() || path.is_dir()) && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn decode_uri(raw: &str) -> String {
    let s = raw.trim().trim_matches('"').trim_matches('\'');
    let s = if let Some(path) = s.strip_prefix("file://") {
        if path.starts_with('/') {
            path
        } else if let Some(path) = path.strip_prefix("localhost/") {
            return decode_percent_bytes(&format!("/{}", path));
        } else {
            return String::new();
        }
    } else {
        s
    };
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

fn decode_percent_bytes(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
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

fn refresh_device_info_state(window: &MainWindow) {
    let info = collect_device_info();
    window.set_device_os(SharedString::from(info.os));
    window.set_device_desktop(SharedString::from(info.desktop));
    window.set_device_session(SharedString::from(info.session));
    window.set_device_kernel(SharedString::from(info.kernel));
    window.set_device_cursor(SharedString::from(info.cursor));
    window.set_device_gtk(SharedString::from(info.gtk));
    window.set_device_qt(SharedString::from(info.qt));
    window.set_device_env(SharedString::from(info.env_vars));
}
