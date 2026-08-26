mod common;

use mouse_me::core::importer::{import_cursor_pack, import_cursor_pack_into};

#[test]
fn import_missing_path_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = import_cursor_pack_into(
        std::path::Path::new("/definitely/missing/theme.zip"),
        dir.path(),
    )
    .unwrap_err();
    assert!(err.contains("does not exist"));
}

#[test]
fn import_folder_without_cursors_returns_error() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("readme.txt"), "no cursors here").unwrap();
    let err = import_cursor_pack_into(src.path(), dest.path()).unwrap_err();
    assert!(err.contains("No valid cursor theme"));
}

#[test]
fn import_folder_installs_theme_and_writes_index() {
    let src_root = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    common::make_theme_dir(src_root.path(), "demo-pack", "Demo Pack");

    let names = import_cursor_pack_into(src_root.path(), dest.path()).unwrap();
    assert_eq!(names, vec!["Demo-Pack".to_string()]);
    let installed = dest.path().join("Demo-Pack");
    assert!(installed.join("cursors").is_dir());
    let index = std::fs::read_to_string(installed.join("index.theme")).unwrap();
    assert!(index.contains("Name=Demo Pack") || index.contains("Name=Demo-Pack"));
}

#[test]
fn import_zip_installs_theme() {
    let workspace = tempfile::tempdir().unwrap();
    let zip_path = workspace.path().join("pack.zip");
    common::write_zip(
        &zip_path,
        &[
            ("cool-cursors/", b""),
            ("cool-cursors/cursors/", b""),
            ("cool-cursors/cursors/left_ptr", b"not-binary-but-present"),
            (
                "cool-cursors/index.theme",
                b"[Icon Theme]\nName=Cool Cursors\n",
            ),
        ],
    );

    let dest = tempfile::tempdir().unwrap();
    let names = import_cursor_pack_into(&zip_path, dest.path()).unwrap();
    assert_eq!(names, vec!["Cool-Cursors".to_string()]);
    assert!(dest.path().join("Cool-Cursors").join("cursors").is_dir());
}

#[test]
fn import_cursor_pack_without_path_still_errors() {
    let err = import_cursor_pack(std::path::Path::new("/no/pack.zip")).unwrap_err();
    assert!(err.contains("does not exist"));
}

#[test]
fn import_zip_rejects_path_traversal() {
    let workspace = tempfile::tempdir().unwrap();
    let zip_path = workspace.path().join("evil.zip");
    common::write_zip(&zip_path, &[("../escape", b"owned")]);
    let dest = tempfile::tempdir().unwrap();
    let err = import_cursor_pack_into(&zip_path, dest.path()).unwrap_err();
    assert!(err.to_lowercase().contains("unsafe") || err.contains("No valid cursor theme"));
    assert!(!dest.path().join("escape").exists());
    assert!(!workspace.path().join("escape").exists());
}

#[test]
fn import_zip_keeps_windows_cursor_files() {
    let workspace = tempfile::tempdir().unwrap();
    let zip_path = workspace.path().join("pack.zip");
    common::write_zip(
        &zip_path,
        &[
            ("mixed/", b""),
            ("mixed/cursors/", b""),
            ("mixed/cursors/left_ptr", b"xcursor"),
            ("mixed/Windows/", b""),
            ("mixed/Windows/alternate.cur", b"RIFF"),
            ("mixed/index.theme", b"[Icon Theme]\nName=Mixed Pack\n"),
        ],
    );
    let dest = tempfile::tempdir().unwrap();
    let names = import_cursor_pack_into(&zip_path, dest.path()).unwrap();
    assert_eq!(names, vec!["Mixed-Pack".to_string()]);
    assert!(dest
        .path()
        .join("Mixed-Pack")
        .join("Windows")
        .join("alternate.cur")
        .exists());
}

#[test]
fn import_zip_skips_shell_payload_and_installs_theme() {
    let workspace = tempfile::tempdir().unwrap();
    let zip_path = workspace.path().join("payload.zip");
    common::write_zip(
        &zip_path,
        &[
            ("demo/", b""),
            ("demo/cursors/", b""),
            ("demo/cursors/left_ptr", b"cursor"),
            ("demo/install.sh", b"#!/bin/sh\necho hi\n"),
            ("demo/Windows/Install.inf", b"[Version]\n"),
        ],
    );
    let dest = tempfile::tempdir().unwrap();
    let names = import_cursor_pack_into(&zip_path, dest.path()).unwrap();
    assert_eq!(names, vec!["demo".to_string()]);
    let installed = dest.path().join("demo");
    assert!(installed.join("cursors").join("left_ptr").exists());
    assert!(!installed.join("install.sh").exists());
    assert!(!installed.join("Windows").join("Install.inf").exists());
}
