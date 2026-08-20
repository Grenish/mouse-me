use mouse_me::core::auth::{normalize_email, validate_password, AuthStore};

fn store() -> (tempfile::TempDir, AuthStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    (dir, AuthStore::load_from(path))
}

#[test]
fn normalize_email_trims_and_lowercases() {
    assert_eq!(
        normalize_email("  Grenish@Example.COM ").unwrap(),
        "grenish@example.com"
    );
}

#[test]
fn normalize_email_rejects_empty_and_malformed() {
    assert!(normalize_email("").unwrap_err().contains("Enter an email"));
    assert!(normalize_email("not-an-email")
        .unwrap_err()
        .contains("not valid"));
    assert!(normalize_email("a@b").unwrap_err().contains("not valid"));
    assert!(normalize_email("@site.com")
        .unwrap_err()
        .contains("not valid"));
}

#[test]
fn validate_password_enforces_length() {
    assert!(validate_password("short")
        .unwrap_err()
        .contains("at least 8"));
    assert!(validate_password("long-enough").is_ok());
}

#[test]
fn create_account_then_sign_in() {
    let (_dir, mut auth) = store();
    let email = auth
        .create_account("you@example.com", "secret123", "secret123")
        .unwrap();
    assert_eq!(email, "you@example.com");
    assert_eq!(auth.session_email(), Some("you@example.com"));

    auth.sign_out().unwrap();
    assert!(auth.session_email().is_none());

    let email = auth.sign_in("YOU@example.com", "secret123").unwrap();
    assert_eq!(email, "you@example.com");
    assert_eq!(auth.session_email(), Some("you@example.com"));
}

#[test]
fn create_account_rejects_duplicate_email() {
    let (_dir, mut auth) = store();
    auth.create_account("you@example.com", "secret123", "secret123")
        .unwrap();
    let err = auth
        .create_account("you@example.com", "secret123", "secret123")
        .unwrap_err();
    assert!(err.contains("already exists"));
}

#[test]
fn create_account_rejects_mismatched_passwords() {
    let (_dir, mut auth) = store();
    let err = auth
        .create_account("you@example.com", "secret123", "secret124")
        .unwrap_err();
    assert!(err.contains("do not match"));
}

#[test]
fn sign_in_rejects_unknown_email_and_wrong_password() {
    let (_dir, mut auth) = store();
    auth.create_account("you@example.com", "secret123", "secret123")
        .unwrap();
    let missing = auth.sign_in("other@example.com", "secret123").unwrap_err();
    assert!(missing.contains("No account"));
    let wrong = auth.sign_in("you@example.com", "wrongpass").unwrap_err();
    assert!(wrong.contains("Wrong password"));
}

#[test]
fn stored_file_does_not_contain_plaintext_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let mut auth = AuthStore::load_from(path.clone());
    auth.create_account("you@example.com", "secret123", "secret123")
        .unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("secret123"));
    assert!(raw.contains("you@example.com"));
    assert!(raw.contains("\"salt\""));
    assert!(raw.contains("\"hash\""));

    let restored = AuthStore::load_from(path);
    assert_eq!(restored.session_email(), Some("you@example.com"));
    let mut restored = restored;
    restored.sign_out().unwrap();
    restored.sign_in("you@example.com", "secret123").unwrap();
}
