use mouse_me::core::applier::{apply_with_targets, validate_apply_input, ApplyTargets};

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

#[test]
fn validate_apply_input_rejects_path_names() {
    let err = validate_apply_input("../evil", 24).unwrap_err();
    assert!(err.contains("safe"));
}

#[test]
fn apply_with_no_targets_returns_ok_without_touching_the_session() {
    let targets = ApplyTargets {
        hyprland: false,
        gsettings: false,
        gtk: false,
        qt: false,
        environment: false,
        xresources: false,
        default_index: false,
        flatpak: false,
    };
    let warnings = apply_with_targets("modest-light", 24, &targets).unwrap();
    assert!(warnings.is_empty());
}
