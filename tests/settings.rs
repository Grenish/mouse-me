use mouse_me::core::settings::AppSettings;

#[test]
fn defaults_are_safe_for_first_launch() {
    let settings = AppSettings::default();
    assert!(settings.apply_hyprland);
    assert!(settings.show_user_themes);
    assert!(settings.show_system_themes);
    assert!(!settings.auto_apply_on_import);
    assert_eq!(settings.library_filter, "all");
    assert_eq!(settings.library_type, "all");
    assert_eq!(settings.preferred_size, 24);
    assert_eq!(settings.last_page, 0);
    assert_eq!(settings.inactive_timeout, 0);
}

#[test]
fn apply_targets_mirror_desktop_toggles() {
    let mut settings = AppSettings::default();
    settings.apply_hyprland = false;
    settings.apply_flatpak = false;
    settings.apply_gtk = true;
    let targets = settings.apply_targets();
    assert!(!targets.hyprland);
    assert!(!targets.flatpak);
    assert!(targets.gtk);
    assert!(targets.qt);
}

#[test]
fn settings_json_roundtrip_keeps_custom_values() {
    let mut settings = AppSettings::default();
    settings.library_filter = "user".into();
    settings.preferred_size = 48;
    settings.last_page = 5;
    settings.hide_on_key_press = true;

    let json = serde_json::to_string(&settings).unwrap();
    let restored: AppSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.library_filter, "user");
    assert_eq!(restored.preferred_size, 48);
    assert_eq!(restored.last_page, 5);
    assert!(restored.hide_on_key_press);
}

#[test]
fn invalid_json_falls_back_to_defaults() {
    let settings: AppSettings = serde_json::from_str("not-json").unwrap_or_default();
    assert_eq!(settings.preferred_size, 24);
}
