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
    assert!(!settings.auto_update);
    assert_eq!(settings.auto_update_when, "next-launch");
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

#[test]
fn setting_keys_roundtrip() {
    let mut settings = AppSettings::default();
    for key in AppSettings::keys() {
        let value = settings.get_key(key).unwrap();
        settings.set_key(key, &value).unwrap();
        assert_eq!(settings.get_key(key).unwrap(), value);
    }
}

#[test]
fn setting_key_parses_aliases_and_rejects_bad_values() {
    let mut settings = AppSettings::default();
    assert_eq!(
        settings.set_key("apply_hyprland", "off").unwrap(),
        mouse_me::core::settings::SettingEffect::None
    );
    assert_eq!(settings.get_key("apply-hyprland").unwrap(), "false");
    assert_eq!(
        settings.set_key("enable-hyprcursor", "yes").unwrap(),
        mouse_me::core::settings::SettingEffect::HyprPrefs
    );
    assert!(settings.enable_hyprcursor);
    assert_eq!(
        settings.set_key("preferred-size", "48").unwrap(),
        mouse_me::core::settings::SettingEffect::PreferredSize
    );
    assert_eq!(settings.preferred_size, 48);
    assert!(settings.set_key("preferred-size", "0").is_err());
    assert!(settings.set_key("auto-update-when", "later").is_err());
    assert!(settings.set_key("not-a-key", "true").is_err());
    settings.set_key("auto-update-when", "instantly").unwrap();
    assert_eq!(settings.auto_update_when, "instantly");
}

#[test]
fn missing_auto_update_fields_keep_other_settings() {
    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("auto_update");
    object.remove("auto_update_when");
    object.insert("apply_hyprland".into(), serde_json::json!(false));
    let restored: AppSettings = serde_json::from_value(value).unwrap();
    assert!(!restored.auto_update);
    assert_eq!(restored.auto_update_when, "next-launch");
    assert!(!restored.apply_hyprland);
    assert!(restored.apply_gtk);
}
