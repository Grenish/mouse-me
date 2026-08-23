use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use mouse_me::core::auth::{
    normalize_email, normalize_name, normalize_username, validate_password, AuthStore,
};
use serde_json::{json, Value};

struct MockUser {
    id: String,
    email: String,
    password: String,
    name: String,
    username: String,
}

struct MockState {
    users: HashMap<String, MockUser>,
    sessions: HashMap<String, String>,
}

fn start_mock() -> (String, Arc<Mutex<MockState>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(MockState {
        users: HashMap::new(),
        sessions: HashMap::new(),
    }));
    let thread_state = state.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = handle_connection(stream, &thread_state);
        }
    });
    (format!("http://{addr}"), state)
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    state: &Arc<Mutex<MockState>>,
) -> std::io::Result<()> {
    let mut buf = vec![0u8; 16_384];
    let n = stream.read(&mut buf)?;
    let raw = String::from_utf8_lossy(&buf[..n]).into_owned();
    let (header_text, body_text) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut cookie = String::new();
    let mut authorization = String::new();
    let mut content_length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "cookie" => cookie = value.trim().to_string(),
            "authorization" => authorization = value.trim().to_string(),
            "content-length" => content_length = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut payload = body_text.as_bytes().to_vec();
    while payload.len() < content_length {
        let extra = stream.read(&mut buf)?;
        if extra == 0 {
            break;
        }
        payload.extend_from_slice(&buf[..extra]);
    }
    payload.truncate(content_length);
    let json_body: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);

    let (status, extra_headers, response_body) =
        route(&method, &path, &cookie, &authorization, &json_body, state);
    let mut header_block = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        response_body.len()
    );
    for header in extra_headers {
        header_block.push_str(&header);
        header_block.push_str("\r\n");
    }
    header_block.push_str("\r\n");
    stream.write_all(header_block.as_bytes())?;
    stream.write_all(response_body.as_bytes())?;
    Ok(())
}

fn route(
    method: &str,
    path: &str,
    cookie: &str,
    authorization: &str,
    body: &Value,
    state: &Arc<Mutex<MockState>>,
) -> (u16, Vec<String>, String) {
    let token = session_token(cookie, authorization);
    match (method, path) {
        ("POST", "/api/auth/sign-up/email") => sign_up(body, state),
        ("POST", "/api/auth/sign-in/email") => sign_in(body, state),
        ("POST", "/api/auth/sign-out") => {
            if let Some(token) = token {
                state.lock().unwrap().sessions.remove(&token);
            }
            (200, Vec::new(), json!({ "success": true }).to_string())
        }
        ("GET", "/api/auth/get-session") => get_session(token.as_deref(), state),
        ("GET", "/api/user/settings") => user_settings(token.as_deref(), state),
        _ => (
            404,
            Vec::new(),
            json!({ "message": "Not found" }).to_string(),
        ),
    }
}

fn sign_up(body: &Value, state: &Arc<Mutex<MockState>>) -> (u16, Vec<String>, String) {
    let email = body.get("email").and_then(Value::as_str).unwrap_or("");
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let name = body.get("name").and_then(Value::as_str).unwrap_or("");
    let username = body.get("username").and_then(Value::as_str).unwrap_or("");
    let mut store = state.lock().unwrap();
    if store.users.values().any(|user| user.email == email) {
        return error(
            422,
            "USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL",
            "User already exists. Use another email.",
        );
    }
    if store.users.values().any(|user| user.username == username) {
        return error(422, "FAILED_TO_CREATE_USER", "Failed to create user");
    }
    let id = format!("user-{}", store.users.len() + 1);
    store.users.insert(
        email.to_string(),
        MockUser {
            id: id.clone(),
            email: email.to_string(),
            password: password.to_string(),
            name: name.to_string(),
            username: username.to_string(),
        },
    );
    let token = format!("tok-{}", store.sessions.len() + 1);
    store.sessions.insert(token.clone(), email.to_string());
    session_response(200, &token, &id, email, name, username)
}

fn sign_in(body: &Value, state: &Arc<Mutex<MockState>>) -> (u16, Vec<String>, String) {
    let email = body.get("email").and_then(Value::as_str).unwrap_or("");
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let store = state.lock().unwrap();
    let Some(user) = store.users.get(email) else {
        return error(
            401,
            "INVALID_EMAIL_OR_PASSWORD",
            "Invalid email or password",
        );
    };
    if user.password != password {
        return error(
            401,
            "INVALID_EMAIL_OR_PASSWORD",
            "Invalid email or password",
        );
    }
    let token = format!("tok-{}", store.sessions.len() + 1);
    drop(store);
    state
        .lock()
        .unwrap()
        .sessions
        .insert(token.clone(), email.to_string());
    let store = state.lock().unwrap();
    let user = store.users.get(email).unwrap();
    session_response(
        200,
        &token,
        &user.id,
        &user.email,
        &user.name,
        &user.username,
    )
}

fn get_session(token: Option<&str>, state: &Arc<Mutex<MockState>>) -> (u16, Vec<String>, String) {
    let Some(token) = token else {
        return (200, Vec::new(), "null".into());
    };
    let store = state.lock().unwrap();
    let Some(email) = store.sessions.get(token) else {
        return (200, Vec::new(), "null".into());
    };
    let user = store.users.get(email).unwrap();
    (
        200,
        Vec::new(),
        json!({
            "session": { "token": token },
            "user": {
                "id": user.id,
                "email": user.email,
                "name": user.name,
                "username": user.username,
            }
        })
        .to_string(),
    )
}

fn user_settings(token: Option<&str>, state: &Arc<Mutex<MockState>>) -> (u16, Vec<String>, String) {
    let Some(token) = token else {
        return error(401, "UNAUTHORIZED", "Unauthorized");
    };
    let store = state.lock().unwrap();
    let Some(email) = store.sessions.get(token) else {
        return error(401, "UNAUTHORIZED", "Unauthorized");
    };
    let user = store.users.get(email).unwrap();
    (
        200,
        Vec::new(),
        json!({
            "user": {
                "id": user.id,
                "email": user.email,
                "name": user.name,
                "username": user.username,
                "createdAt": "2026-08-22T17:05:39.000Z",
            },
            "packStats": { "total": 2, "published": 2, "drafts": 0 },
        })
        .to_string(),
    )
}

fn session_response(
    status: u16,
    token: &str,
    id: &str,
    email: &str,
    name: &str,
    username: &str,
) -> (u16, Vec<String>, String) {
    (
        status,
        vec![format!(
            "Set-Cookie: better-auth.session_token={token}; Path=/; HttpOnly"
        )],
        json!({
            "redirect": false,
            "token": token,
            "user": {
                "id": id,
                "email": email,
                "name": name,
                "username": username,
            }
        })
        .to_string(),
    )
}

fn error(status: u16, code: &str, message: &str) -> (u16, Vec<String>, String) {
    (
        status,
        Vec::new(),
        json!({ "code": code, "message": message }).to_string(),
    )
}

fn session_token(cookie: &str, authorization: &str) -> Option<String> {
    if let Some(token) = authorization
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return Some(token.to_string());
    }
    cookie.split(';').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        if name.trim() == "better-auth.session_token" {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn store_against(base: &str) -> (tempfile::TempDir, AuthStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    (
        dir,
        AuthStore::load_from(path).with_api_base(base.to_string()),
    )
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
fn name_and_username_match_website_rules() {
    assert_eq!(normalize_name(" Grenish ").unwrap(), "Grenish");
    assert_eq!(normalize_username("@Cool-User").unwrap(), "cool-user");
    assert!(normalize_username("Bad User").is_err());
}

#[test]
fn create_account_then_sign_in_against_web_api() {
    let (base, _) = start_mock();
    let (_dir, mut auth) = store_against(&base);
    let user = auth
        .create_account(
            "You Example",
            "you-example",
            "you@example.com",
            "secret123",
            "secret123",
        )
        .unwrap();
    assert_eq!(user.email, "you@example.com");
    assert_eq!(user.username, "you-example");
    assert_eq!(auth.session_email(), Some("you@example.com"));

    auth.sign_out().unwrap();
    assert!(auth.session_email().is_none());

    let user = auth.sign_in("YOU@example.com", "secret123").unwrap();
    assert_eq!(user.email, "you@example.com");
    assert_eq!(auth.user().unwrap().name, "You Example");
    assert_eq!(user.published_count, 2);
    assert_eq!(
        mouse_me::core::auth::format_joined(user.created_at.as_deref().unwrap()),
        "22 Aug 2026"
    );
}

#[test]
fn create_account_rejects_duplicate_email() {
    let (base, _) = start_mock();
    let (_dir, mut auth) = store_against(&base);
    auth.create_account(
        "You Example",
        "you-example",
        "you@example.com",
        "secret123",
        "secret123",
    )
    .unwrap();
    let err = auth
        .create_account(
            "You Example",
            "other-name",
            "you@example.com",
            "secret123",
            "secret123",
        )
        .unwrap_err();
    assert!(err.contains("already exists"));
}

#[test]
fn create_account_rejects_mismatched_passwords() {
    let (base, _) = start_mock();
    let (_dir, mut auth) = store_against(&base);
    let err = auth
        .create_account(
            "You Example",
            "you-example",
            "you@example.com",
            "secret123",
            "secret124",
        )
        .unwrap_err();
    assert!(err.contains("do not match"));
}

#[test]
fn sign_in_rejects_unknown_email_and_wrong_password() {
    let (base, _) = start_mock();
    let (_dir, mut auth) = store_against(&base);
    auth.create_account(
        "You Example",
        "you-example",
        "you@example.com",
        "secret123",
        "secret123",
    )
    .unwrap();
    let missing = auth.sign_in("other@example.com", "secret123").unwrap_err();
    assert!(missing.contains("Wrong email or password"));
    let wrong = auth.sign_in("you@example.com", "wrongpass").unwrap_err();
    assert!(wrong.contains("Wrong email or password"));
}

#[test]
fn stored_file_does_not_contain_plaintext_password() {
    let (base, _) = start_mock();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let mut auth = AuthStore::load_from(path.clone()).with_api_base(base);
    auth.create_account(
        "You Example",
        "you-example",
        "you@example.com",
        "secret123",
        "secret123",
    )
    .unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("secret123"));
    assert!(raw.contains("you@example.com"));
    assert!(raw.contains("better-auth.session_token"));
    assert!(!raw.contains("\"salt\""));
    assert!(!raw.contains("\"hash\""));

    let restored = AuthStore::load_from(path);
    assert_eq!(restored.session_email(), Some("you@example.com"));
    assert_eq!(restored.user().unwrap().username, "you-example");
}

#[test]
fn refresh_clears_an_expired_session() {
    let (base, state) = start_mock();
    let (_dir, mut auth) = store_against(&base);
    auth.create_account(
        "You Example",
        "you-example",
        "you@example.com",
        "secret123",
        "secret123",
    )
    .unwrap();
    state.lock().unwrap().sessions.clear();
    assert!(auth.refresh().unwrap().is_none());
    assert!(auth.session_email().is_none());
}
