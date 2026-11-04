mod common;

use mouse_me::core::studio::{
    export_project_into, load_draft, load_png_as_cursor, load_theme_project, sanitize_theme_name,
    save_draft, scale_cursor, sort_frame_paths, square_pad, suggest_hotspot, Anchor, StudioProject,
    STUDIO_MARKER, STUDIO_ROLES,
};
use mouse_me::core::types::CursorImage;
use mouse_me::core::xcursor::{parse_xcursor_file, parse_xcursor_frames};

#[test]
fn sanitize_replaces_spaces_and_unsafe_characters() {
    assert_eq!(sanitize_theme_name("My Pointer"), "My-Pointer");
    assert_eq!(sanitize_theme_name("cool/theme"), "cool_theme");
    assert_eq!(sanitize_theme_name("  "), "custom-pointer");
    assert_eq!(sanitize_theme_name(""), "custom-pointer");
    assert_eq!(sanitize_theme_name("keep_this-01"), "keep_this-01");
}

#[test]
fn studio_roles_cover_core_pointer_names() {
    let ids: Vec<&str> = STUDIO_ROLES.iter().map(|role| role.id).collect();
    assert!(ids.contains(&"left_ptr"));
    assert!(ids.contains(&"pointer"));
    assert!(ids.contains(&"text"));
    assert!(ids.contains(&"wait"));
    assert_eq!(STUDIO_ROLES.len(), 14);
}

#[test]
fn scale_cursor_clamps_size_and_hotspot() {
    let image = common::solid_cursor(64, 64, [10, 20, 30, 255]);
    let (scaled, xhot, yhot) = scale_cursor(&image, 8, 80, 80);
    assert_eq!(scaled.width, 16);
    assert_eq!(scaled.height, 16);
    assert_eq!(xhot, 15);
    assert_eq!(yhot, 15);
}

#[test]
fn scale_cursor_keeps_matching_size() {
    let image = common::solid_cursor(32, 32, [1, 2, 3, 255]);
    let (scaled, xhot, yhot) = scale_cursor(&image, 32, 4, 5);
    assert_eq!(scaled.width, 32);
    assert_eq!(xhot, 4);
    assert_eq!(yhot, 5);
}

#[test]
fn load_png_as_cursor_rejects_oversized_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    let image = image::RgbaImage::new(513, 32);
    image.save(&path).unwrap();
    let err = mouse_me::core::studio::load_png_as_cursor(&path).unwrap_err();
    assert!(err.contains("too large") || err.contains("max"));
}

#[test]
fn load_png_as_cursor_reads_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    common::write_png(&path, 12, 10, [9, 8, 7, 255]);
    let image = load_png_as_cursor(&path).unwrap();
    assert_eq!(image.width, 12);
    assert_eq!(image.height, 10);
    assert_eq!(image.rgba.len(), 12 * 10 * 4);
}

#[test]
fn load_png_as_cursor_rejects_missing_file() {
    let err = load_png_as_cursor(std::path::Path::new("/no/such.png")).unwrap_err();
    assert!(err.contains("Could not read image"));
}

/// A transparent square with one opaque pixel at (x, y) and an opaque block
/// elsewhere, so hotspot guesses have something to find.
fn dot(edge: u32, x: u32, y: u32) -> CursorImage {
    let mut image = common::solid_cursor(edge, edge, [0, 0, 0, 0]);
    let at = ((y * edge + x) * 4) as usize;
    image.rgba[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
    image
}

fn project_with(role: &str, frames: Vec<CursorImage>) -> StudioProject {
    let mut project = StudioProject {
        name: "Test Studio".into(),
        comment: "From tests".into(),
        ..StudioProject::default()
    };
    project.add_frames(role, frames, false).unwrap();
    project
}

#[test]
fn export_requires_at_least_one_role() {
    let dir = tempfile::tempdir().unwrap();
    let err = export_project_into(&StudioProject::default(), dir.path()).unwrap_err();
    assert!(err.contains("at least one role"));
}

#[test]
fn export_writes_every_size_with_aliases_and_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project_with(
        "left_ptr",
        vec![common::solid_cursor(32, 32, [255, 0, 0, 255])],
    );
    project.sizes = vec![48, 24, 32, 24];

    let name = export_project_into(&project, dir.path()).unwrap();

    assert_eq!(name, "Test-Studio");
    let theme = dir.path().join(&name);
    let index = std::fs::read_to_string(theme.join("index.theme")).unwrap();
    assert!(index.contains("Name=Test Studio"));
    assert!(index.contains("Comment=From tests"));
    assert!(theme.join(STUDIO_MARKER).exists());

    let left_ptr = theme.join("cursors").join("left_ptr");
    for size in [24, 32, 48] {
        let frames = parse_xcursor_frames(&left_ptr, size).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].image.width, size);
    }
    assert!(theme.join("cursors").join("default").exists());
    // fill_missing is on by default, so other roles fall back to Default.
    assert!(parse_xcursor_file(&theme.join("cursors").join("pointer"), 24).is_some());
}

#[test]
fn export_without_fill_skips_empty_roles() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project_with(
        "left_ptr",
        vec![common::solid_cursor(24, 24, [1, 2, 3, 255])],
    );
    project.fill_missing = false;
    let name = export_project_into(&project, dir.path()).unwrap();
    let cursors = dir.path().join(name).join("cursors");
    assert!(cursors.join("left_ptr").exists());
    assert!(!cursors.join("pointer").exists());
    assert!(!cursors.join("hand2").exists());
}

#[test]
fn each_role_keeps_its_own_hotspot() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project_with("left_ptr", vec![dot(32, 0, 0)]);
    project
        .add_frames("text", vec![dot(32, 16, 16)], false)
        .unwrap();
    project.set_hotspot("left_ptr", 2, 3);
    project.set_hotspot("text", 15, 17);
    project.sizes = vec![32];

    let name = export_project_into(&project, dir.path()).unwrap();
    let cursors = dir.path().join(name).join("cursors");
    let arrow = &parse_xcursor_frames(&cursors.join("left_ptr"), 32).unwrap()[0];
    let beam = &parse_xcursor_frames(&cursors.join("text"), 32).unwrap()[0];
    assert_eq!((arrow.xhot, arrow.yhot), (2, 3));
    assert_eq!((beam.xhot, beam.yhot), (15, 17));
}

#[test]
fn animated_role_exports_all_frames_with_delay() {
    let dir = tempfile::tempdir().unwrap();
    let frames = (0..3)
        .map(|i| common::solid_cursor(32, 32, [i * 80, 0, 0, 255]))
        .collect();
    let mut project = project_with("wait", frames);
    project.set_delay("wait", 120);
    project.sizes = vec![24, 48];

    let name = export_project_into(&project, dir.path()).unwrap();
    let wait = dir.path().join(name).join("cursors").join("wait");
    for size in [24, 48] {
        let parsed = parse_xcursor_frames(&wait, size).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(parsed.iter().all(|frame| frame.delay_ms == 120));
        assert!(parsed.iter().all(|frame| frame.image.width == size));
    }
}

#[test]
fn export_refuses_to_replace_a_foreign_theme() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = common::make_theme_dir(dir.path(), "Test-Studio", "Someone else");
    let project = project_with(
        "left_ptr",
        vec![common::solid_cursor(24, 24, [9, 9, 9, 255])],
    );

    let err = export_project_into(&project, dir.path()).unwrap_err();
    assert!(err.contains("not made in Studio"));
    let index = std::fs::read_to_string(foreign.join("index.theme")).unwrap();
    assert!(index.contains("Someone else"));
}

#[test]
fn export_replaces_its_own_theme() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project_with(
        "left_ptr",
        vec![common::solid_cursor(24, 24, [9, 9, 9, 255])],
    );
    export_project_into(&project, dir.path()).unwrap();
    project.comment = "Second pass".into();
    let name = export_project_into(&project, dir.path()).unwrap();

    let index = std::fs::read_to_string(dir.path().join(&name).join("index.theme")).unwrap();
    assert!(index.contains("Comment=Second pass"));
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging folders left behind: {leftovers:?}"
    );
}

#[test]
fn non_square_art_is_padded_not_stretched() {
    let tall = common::solid_cursor(8, 32, [255, 255, 255, 255]);
    let (square, ox, oy) = square_pad(&tall);
    assert_eq!((square.width, square.height), (32, 32));
    assert_eq!((ox, oy), (12, 0));
    // Left edge column stays transparent: the beam was centred, not stretched.
    assert_eq!(square.rgba[3], 0);

    let (scaled, xhot, yhot) = scale_cursor(&tall, 16, 4, 16);
    assert_eq!((scaled.width, scaled.height), (16, 16));
    assert_eq!((xhot, yhot), (8, 8));
}

#[test]
fn hotspot_guesses_follow_the_role_anchor() {
    let mut image = common::solid_cursor(32, 32, [0, 0, 0, 0]);
    for y in 4..20 {
        for x in 6..14 {
            let at = ((y * 32 + x) * 4) as usize;
            image.rgba[at + 3] = 255;
        }
    }
    assert_eq!(suggest_hotspot(Anchor::TopLeft, &image), (6, 4));
    assert_eq!(suggest_hotspot(Anchor::Top, &image), (9, 4));
    assert_eq!(suggest_hotspot(Anchor::Center, &image), (9, 11));
    assert_eq!(suggest_hotspot(Anchor::BottomLeft, &image), (6, 19));
    let empty = common::solid_cursor(8, 8, [0, 0, 0, 0]);
    assert_eq!(suggest_hotspot(Anchor::Center, &empty), (0, 0));
}

#[test]
fn later_frames_are_fitted_to_the_first() {
    let mut project = project_with("wait", vec![common::solid_cursor(32, 32, [1, 1, 1, 255])]);
    project
        .add_frames(
            "wait",
            vec![common::solid_cursor(64, 48, [2, 2, 2, 255])],
            false,
        )
        .unwrap();
    let asset = &project.roles["wait"];
    assert_eq!(asset.frames.len(), 2);
    assert!(asset
        .frames
        .iter()
        .all(|frame| frame.width == 32 && frame.height == 32));
}

#[test]
fn frame_editing_keeps_the_role_consistent() {
    let frames = (0..3)
        .map(|i| common::solid_cursor(16, 16, [i, 0, 0, 255]))
        .collect();
    let mut project = project_with("wait", frames);
    assert_eq!(project.move_frame("wait", 0, 1), 1);
    assert_eq!(project.roles["wait"].frames[0].rgba[0], 1);
    assert_eq!(project.move_frame("wait", 2, 1), 2);
    project.remove_frame("wait", 0);
    project.remove_frame("wait", 0);
    project.remove_frame("wait", 0);
    assert!(!project.roles.contains_key("wait"));
}

#[test]
fn fill_empty_copies_into_unassigned_roles_only() {
    let mut project = project_with("left_ptr", vec![dot(16, 1, 1)]);
    project
        .add_frames("text", vec![dot(16, 8, 8)], false)
        .unwrap();
    let filled = project.fill_empty_from("left_ptr");
    assert_eq!(filled, STUDIO_ROLES.len() - 2);
    assert_eq!(project.roles.len(), STUDIO_ROLES.len());
    assert_eq!(project.roles["text"].xhot, 8);
}

#[test]
fn hotspot_is_clamped_to_the_frame() {
    let mut project = project_with("left_ptr", vec![dot(16, 0, 0)]);
    project.set_hotspot("left_ptr", 99, -4);
    assert_eq!(
        (
            project.roles["left_ptr"].xhot,
            project.roles["left_ptr"].yhot
        ),
        (15, 0)
    );
}

#[test]
fn installed_theme_opens_as_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let frames = (0..2)
        .map(|i| common::solid_cursor(32, 32, [i * 100, 0, 0, 255]))
        .collect();
    let mut project = project_with("wait", frames);
    project
        .add_frames("text", vec![dot(32, 16, 16)], false)
        .unwrap();
    project.set_hotspot("text", 16, 10);
    project.fill_missing = false;
    project.sizes = vec![32];
    let name = export_project_into(&project, dir.path()).unwrap();

    let opened = load_theme_project(&dir.path().join(&name)).unwrap();
    assert_eq!(opened.name, "Test Studio");
    assert_eq!(opened.roles["wait"].frames.len(), 2);
    assert_eq!(opened.roles["text"].edge(), 32);
    assert_eq!(
        (opened.roles["text"].xhot, opened.roles["text"].yhot),
        (16, 10)
    );
    assert!(!opened.roles.contains_key("pointer"));
}

#[test]
fn foreign_theme_opens_as_a_remix() {
    let dir = tempfile::tempdir().unwrap();
    let theme = common::make_theme_dir(dir.path(), "Breeze", "Breeze");
    mouse_me::core::xcursor::write_xcursor_file(
        &theme.join("cursors").join("default"),
        &common::solid_cursor(24, 24, [5, 5, 5, 255]),
        3,
        4,
        50,
    )
    .unwrap();
    let opened = load_theme_project(&theme).unwrap();
    assert_eq!(opened.name, "Breeze Remix");
    assert_eq!(
        (opened.roles["left_ptr"].xhot, opened.roles["left_ptr"].yhot),
        (3, 4)
    );
}

#[test]
fn draft_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("studio-draft");
    let frames = (0..2)
        .map(|i| common::solid_cursor(20, 20, [i * 90, 7, 7, 255]))
        .collect();
    let mut project = project_with("wait", frames);
    project.set_hotspot("wait", 5, 6);
    project.set_delay("wait", 200);
    project.sizes = vec![32, 64];
    project.fill_missing = false;

    save_draft(&project, &draft).unwrap();
    save_draft(&project, &draft).unwrap();
    let loaded = load_draft(&draft).unwrap();

    assert_eq!(loaded.name, "Test Studio");
    assert_eq!(loaded.sizes, vec![32, 64]);
    assert!(!loaded.fill_missing);
    let wait = &loaded.roles["wait"];
    assert_eq!(wait.frames.len(), 2);
    assert_eq!((wait.xhot, wait.yhot, wait.delay_ms), (5, 6, 200));
    assert_eq!(wait.frames[1].rgba, project.roles["wait"].frames[1].rgba);
}

#[test]
fn draft_survives_a_save_cut_off_before_the_swap() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("studio-draft");
    let project = project_with(
        "left_ptr",
        vec![common::solid_cursor(16, 16, [1, 2, 3, 255])],
    );
    save_draft(&project, &draft).unwrap();

    // A save that set the old draft aside and then stopped.
    std::fs::rename(&draft, dir.path().join("studio-draft.old")).unwrap();

    let loaded = load_draft(&draft).expect("the set-aside draft should load");
    assert!(loaded.roles.contains_key("left_ptr"));

    save_draft(&project, &draft).unwrap();
    assert!(draft.join("project.json").is_file());
    assert!(!dir.path().join("studio-draft.old").exists());
}

#[test]
fn frame_paths_sort_numbers_by_value() {
    let mut paths: Vec<std::path::PathBuf> = [
        "/b/frame10.png",
        "/a/frame2.png",
        "/c/frame1.png",
        "/a/Frame11.png",
        "/a/frame02b.png",
    ]
    .iter()
    .map(Into::into)
    .collect();
    sort_frame_paths(&mut paths);
    let names: Vec<_> = paths
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "frame1.png",
            "frame2.png",
            "frame02b.png",
            "frame10.png",
            "Frame11.png"
        ]
    );
}
