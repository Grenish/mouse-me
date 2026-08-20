use mouse_me::core::applier::{validate_apply_input, ApplyTargets};

#[test]
fn apply_targets_all_enables_every_desktop() {
    let targets = ApplyTargets::all();
    assert!(targets.hyprland);
    assert!(targets.gsettings);
    assert!(targets.gtk);
    assert!(targets.qt);
    assert!(targets.environment);
    assert!(targets.xresources);
    assert!(targets.default_index);
    assert!(targets.flatpak);
}

#[test]
fn validate_apply_input_rejects_empty_theme() {
    let err = validate_apply_input("   ", 24).unwrap_err();
    assert!(err.contains("cannot be empty"));
}

#[test]
fn validate_apply_input_rejects_control_characters() {
    let err = validate_apply_input("bad\ntheme", 24).unwrap_err();
    assert!(err.contains("control characters"));
}

#[test]
fn validate_apply_input_rejects_zero_size() {
    let err = validate_apply_input("Adwaita", 0).unwrap_err();
    assert!(err.contains("greater than zero"));
}

#[test]
fn validate_apply_input_accepts_normal_theme() {
    assert!(validate_apply_input("modest-light", 24).is_ok());
    assert!(validate_apply_input("Yaru", 64).is_ok());
}
