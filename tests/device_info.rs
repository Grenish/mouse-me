use mouse_me::core::device_info::collect_device_info;

#[test]
fn test_collect_device_info_non_empty() {
    let info = collect_device_info();

    assert!(!info.os.is_empty(), "OS string should not be empty");
    assert!(!info.kernel.is_empty(), "Kernel string should not be empty");
    assert!(!info.desktop.is_empty(), "Desktop string should not be empty");
    assert!(!info.session.is_empty(), "Session string should not be empty");
    assert!(!info.cursor.is_empty(), "Cursor string should not be empty");
    assert!(!info.gtk.is_empty(), "GTK string should not be empty");
    assert!(!info.qt.is_empty(), "Qt string should not be empty");
    assert!(!info.env_vars.is_empty(), "Env vars string should not be empty");
    assert!(
        info.full_report.contains("Mouse Me Debug"),
        "Report should contain header"
    );
    assert!(
        info.full_report.contains(&info.os),
        "Report should include OS"
    );
}
