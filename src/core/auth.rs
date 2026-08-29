use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::fsutil::set_secret_mode;

pub const DEFAULT_API_BASE: &str = "https://mouse-me-web.vercel.app";
const API_ENV: &str = "MOUSE_ME_API_URL";
const USERNAME_MAX: usize = 32;
const NAME_MAX: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user: Option<AuthUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default, alias = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub published_count: u32,
}

#[derive(Debug, Clone)]
pub struct AuthStore {
    path: PathBuf,
    data: AuthFile,
    api_base: String,
}

#[derive(Debug, Deserialize)]
struct AuthSuccessBody {
    token: Option<String>,
    user: Option<RemoteUser>,
}

#[derive(Debug, Deserialize)]
struct SessionBody {
    user: Option<RemoteUser>,
}

#[derive(Debug, Deserialize, Default)]
struct RemoteUser {
    id: Option<String>,
    email: Option<String>,
    name: Option<String>,
    username: Option<String>,
    image: Option<String>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PackStatsBody {
    #[serde(default)]
    published: u32,
}

#[derive(Debug, Deserialize)]
struct SettingsBody {
    user: Option<RemoteUser>,
    #[serde(default, rename = "packStats")]
    pack_stats: PackStatsBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: Option<String>,
    message: Option<String>,
}

struct HttpResponse {
    status: u16,
    set_cookies: Vec<String>,
    body: String,
    location: Option<String>,
}

impl AuthStore {
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("mouse-me").join("auth.json"))
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::config_path().ok_or("Could not locate config directory")?;
        Ok(Self::load_from(path))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<AuthFile>(&raw).ok())
            .filter(is_remote_session_file)
            .unwrap_or_default();
        let api_base = data
            .api_base
            .as_deref()
            .map(normalize_api_base)
            .filter(|base| !base.is_empty())
            .unwrap_or_else(resolve_api_base);
        Self {
            path,
            data,
            api_base,
        }
    }

    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = normalize_api_base(&api_base.into());
        self
    }

    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    pub fn user(&self) -> Option<&AuthUser> {
        self.data.user.as_ref()
    }

    pub fn session_email(&self) -> Option<&str> {
        self.data.user.as_ref().map(|user| user.email.as_str())
    }

    pub fn profile_url(&self) -> Option<String> {
        self.data
            .user
            .as_ref()
            .map(|user| format!("{}/u/{}", self.api_base, urlencoding_path(&user.username)))
    }

    pub fn forgot_password_url(&self) -> String {
        format!("{}/forgot-password", self.api_base)
    }

    pub fn sign_in(&mut self, email: &str, password: &str) -> Result<AuthUser, String> {
        let email = normalize_email(email)?;
        validate_password(password)?;
        let body = serde_json::json!({
            "email": email,
            "password": password,
            "rememberMe": true,
        });
        let response = self.request("POST", "/api/auth/sign-in/email", true, Some(&body))?;
        self.apply_auth_response("sign in", response)
    }

    pub fn create_account(
        &mut self,
        name: &str,
        username: &str,
        email: &str,
        password: &str,
        confirm: &str,
    ) -> Result<AuthUser, String> {
        let name = normalize_name(name)?;
        let username = normalize_username(username)?;
        let email = normalize_email(email)?;
        validate_password(password)?;
        if password != confirm {
            return Err("Passwords do not match.".into());
        }
        let body = serde_json::json!({
            "email": email,
            "password": password,
            "name": name,
            "username": username,
        });
        let response = self.request("POST", "/api/auth/sign-up/email", true, Some(&body))?;
        self.apply_auth_response("create an account", response)
    }

    pub fn sign_out(&mut self) -> Result<(), String> {
        if self.data.cookie.is_some() || self.data.token.is_some() {
            let body = serde_json::json!({});
            let _ = self.request("POST", "/api/auth/sign-out", true, Some(&body));
        }
        self.data.cookie = None;
        self.data.token = None;
        self.data.user = None;
        self.data.api_base = Some(self.api_base.clone());
        self.save()
    }

    pub fn refresh(&mut self) -> Result<Option<AuthUser>, String> {
        if self.data.cookie.is_none() && self.data.token.is_none() {
            self.data.user = None;
            return Ok(None);
        }
        match self.request("GET", "/api/auth/get-session", false, None) {
            Ok(response) if response.status == 200 => {
                if response.body.trim() == "null" || response.body.trim().is_empty() {
                    self.clear_session()?;
                    return Ok(None);
                }
                let parsed: SessionBody =
                    serde_json::from_str(&response.body).unwrap_or(SessionBody { user: None });
                let Some(user) = parsed.user.and_then(|remote| remote.into_user()) else {
                    self.clear_session()?;
                    return Ok(None);
                };
                self.merge_cookies(&response.set_cookies);
                self.data.user = Some(user);
                self.data.api_base = Some(self.api_base.clone());
                self.save()?;
                Ok(self.hydrate_profile())
            }
            Ok(response) if response.status == 401 => {
                self.clear_session()?;
                Ok(None)
            }
            Ok(_) | Err(_) => Ok(self.data.user.clone()),
        }
    }

    fn apply_auth_response(
        &mut self,
        action: &str,
        response: HttpResponse,
    ) -> Result<AuthUser, String> {
        if !(200..300).contains(&response.status) {
            return Err(map_error_body(action, response.status, &response.body));
        }
        let parsed: AuthSuccessBody = serde_json::from_str(&response.body).map_err(|_| {
            format!("Mouse Me returned an unexpected response while trying to {action}.")
        })?;
        let user = parsed
            .user
            .and_then(|remote| remote.into_user())
            .ok_or_else(|| format!("Mouse Me did not return a user while trying to {action}."))?;
        self.merge_cookies(&response.set_cookies);
        if let Some(token) = parsed.token.filter(|token| !token.is_empty()) {
            self.data.token = Some(token.clone());
            if self.data.cookie.as_deref().unwrap_or("").is_empty() {
                self.data.cookie = Some(format!("better-auth.session_token={token}"));
            }
        }
        if self.data.cookie.as_deref().unwrap_or("").is_empty() && self.data.token.is_none() {
            return Err(format!(
                "Mouse Me signed you in but did not return a session. Try {action} again."
            ));
        }
        self.data.user = Some(user);
        self.data.api_base = Some(self.api_base.clone());
        self.save()?;
        Ok(self
            .hydrate_profile()
            .or_else(|| self.data.user.clone())
            .ok_or_else(|| "Could not read the signed-in account.".to_string())?)
    }

    fn hydrate_profile(&mut self) -> Option<AuthUser> {
        let response = self.request("GET", "/api/user/settings", true, None).ok()?;
        if response.status != 200 {
            return self.data.user.clone();
        }
        let parsed: SettingsBody = serde_json::from_str(&response.body).ok()?;
        let mut user = parsed
            .user
            .and_then(RemoteUser::into_user)
            .or_else(|| self.data.user.clone())?;
        user.published_count = parsed.pack_stats.published;
        if user.created_at.is_none() {
            if let Some(existing) = self
                .data
                .user
                .as_ref()
                .and_then(|item| item.created_at.clone())
            {
                user.created_at = Some(existing);
            }
        }
        self.merge_cookies(&response.set_cookies);
        self.data.user = Some(user.clone());
        self.data.api_base = Some(self.api_base.clone());
        let _ = self.save();
        Some(user)
    }

    pub fn fetch_text(&self, path: &str) -> Result<(u16, String), String> {
        if !is_safe_api_path(path) {
            return Err("Invalid catalog path.".into());
        }
        let response = self.request("GET", path, true, None)?;
        Ok((response.status, response.body))
    }

    pub fn download_to(&self, path: &str, dest: &Path, max_bytes: u64) -> Result<(), String> {
        if !is_safe_api_path(path) {
            return Err("Invalid download path.".into());
        }
        let url = format!("{}{path}", self.api_base);
        let origin_base = origin_of(&self.api_base).unwrap_or_else(|| self.api_base.clone());
        let mut current = url;
        for origin in request_origins(&self.api_base) {
            for _ in 0..5 {
                let response = self.send_download(&current, &origin)?;
                if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
                    let location = response
                        .header("location")
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| "Could not follow the download redirect.".to_string())?;
                    let next = resolve_redirect(&current, location);
                    if origin_of(&next).as_deref() != Some(origin_base.as_str()) {
                        return Err("Download redirected off the Mouse Me site.".into());
                    }
                    current = next;
                    continue;
                }
                let status = response.status();
                if !(200..300).contains(&status) {
                    let body = response.into_string().unwrap_or_default();
                    return Err(map_download_error(status, &body));
                }
                let mut reader = response.into_reader().take(max_bytes.saturating_add(1));
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                let mut file = fs::File::create(dest).map_err(|error| error.to_string())?;
                let copied = io::copy(&mut reader, &mut file).map_err(|error| error.to_string())?;
                if copied > max_bytes {
                    let _ = fs::remove_file(dest);
                    return Err("Pack archive is too large.".into());
                }
                if copied == 0 {
                    let _ = fs::remove_file(dest);
                    return Err("Pack archive was empty.".into());
                }
                return Ok(());
            }
        }
        Err("Could not download the pack.".into())
    }

    fn send_download(&self, url: &str, origin: &str) -> Result<ureq::Response, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .redirects(0)
            .user_agent(&user_agent())
            .build();
        let mut request = agent
            .get(url)
            .set("Accept", "*/*")
            .set("Origin", origin)
            .set("User-Agent", &user_agent());
        if let Some(cookie) = self
            .data
            .cookie
            .as_deref()
            .filter(|cookie| !cookie.is_empty())
        {
            request = request.set("Cookie", cookie);
        }
        if let Some(token) = self.data.token.as_deref().filter(|token| !token.is_empty()) {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        match request.call() {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(_, response)) => Ok(response),
            Err(ureq::Error::Transport(error)) => {
                Err(network_error(&self.api_base, &error.to_string()))
            }
        }
    }

    pub fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
        if !is_allowed_avatar_url(url) {
            return Err("Invalid image URL.".into());
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .redirects(5)
            .user_agent(&user_agent())
            .build();
        let response = match agent.get(url).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(error)) => {
                return Err(format!("Could not download the profile photo: {error}"));
            }
        };
        if !(200..300).contains(&response.status()) {
            return Err("Could not download the profile photo.".into());
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(2_000_000)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not download the profile photo: {error}"))?;
        if bytes.is_empty() {
            return Err("The profile photo was empty.".into());
        }
        Ok(bytes)
    }

    fn clear_session(&mut self) -> Result<(), String> {
        self.data.cookie = None;
        self.data.token = None;
        self.data.user = None;
        self.data.api_base = Some(self.api_base.clone());
        self.save()
    }

    fn merge_cookies(&mut self, set_cookies: &[String]) {
        let merged = merge_cookie_header(self.data.cookie.as_deref().unwrap_or(""), set_cookies);
        self.data.cookie = if merged.is_empty() {
            None
        } else {
            Some(merged)
        };
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        send_credentials: bool,
        body: Option<&serde_json::Value>,
    ) -> Result<HttpResponse, String> {
        let url = format!("{}{path}", self.api_base);
        let mut last_origin_error = None;
        for origin in request_origins(&self.api_base) {
            let response =
                self.send_following_redirects(method, &url, &origin, send_credentials, body)?;
            if response.status == 403 && is_invalid_origin(&response.body) {
                last_origin_error = Some(response);
                continue;
            }
            return Ok(response);
        }
        last_origin_error.ok_or_else(|| "Could not reach Mouse Me.".to_string())
    }

    fn send_following_redirects(
        &self,
        method: &str,
        url: &str,
        origin: &str,
        send_credentials: bool,
        body: Option<&serde_json::Value>,
    ) -> Result<HttpResponse, String> {
        let mut current = url.to_string();
        let origin_base = origin_of(&self.api_base).unwrap_or_else(|| self.api_base.clone());
        for _ in 0..5 {
            let response = self.send_once(method, &current, origin, send_credentials, body)?;
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                if let Some(location) = response.location.as_deref().filter(|loc| !loc.is_empty()) {
                    let next = resolve_redirect(&current, location);
                    if origin_of(&next).as_deref() != Some(origin_base.as_str()) {
                        return Err("Sign-in redirected off the Mouse Me site.".into());
                    }
                    current = next;
                    continue;
                }
            }
            return Ok(response);
        }
        Err("Could not reach Mouse Me (too many redirects).".into())
    }

    fn send_once(
        &self,
        method: &str,
        url: &str,
        origin: &str,
        send_credentials: bool,
        body: Option<&serde_json::Value>,
    ) -> Result<HttpResponse, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .redirects(0)
            .user_agent(&user_agent())
            .build();
        let mut request = match method {
            "GET" => agent.get(url),
            "POST" => agent.post(url),
            _ => return Err("Unsupported auth request.".into()),
        };
        request = request
            .set("Accept", "application/json")
            .set("Origin", origin)
            .set("User-Agent", &user_agent());
        if send_credentials || method == "GET" {
            if let Some(cookie) = self
                .data
                .cookie
                .as_deref()
                .filter(|cookie| !cookie.is_empty())
            {
                request = request.set("Cookie", cookie);
            }
            if let Some(token) = self.data.token.as_deref().filter(|token| !token.is_empty()) {
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
        }
        let result = if let Some(body) = body {
            request
                .set("Content-Type", "application/json")
                .send_json(body.clone())
        } else {
            request.call()
        };
        match result {
            Ok(response) => read_response(response),
            Err(ureq::Error::Status(_, response)) => read_response(response),
            Err(ureq::Error::Transport(error)) => {
                Err(network_error(&self.api_base, &error.to_string()))
            }
        }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let raw = serde_json::to_string_pretty(&self.data).map_err(|error| error.to_string())?;
        let parent = self.path.parent().ok_or("Invalid auth path")?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temporary
            .write_all(raw.as_bytes())
            .map_err(|error| error.to_string())?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.error.to_string())?;
        set_secret_mode(&self.path)?;
        Ok(())
    }
}

impl RemoteUser {
    fn into_user(self) -> Option<AuthUser> {
        Some(AuthUser {
            id: self.id.filter(|id| !id.is_empty())?,
            email: self.email.filter(|email| !email.is_empty())?,
            name: self.name.unwrap_or_default(),
            username: self.username.unwrap_or_default(),
            image: self.image.filter(|image| !image.is_empty()),
            created_at: self.created_at.filter(|value| !value.is_empty()),
            published_count: 0,
        })
    }
}

pub fn format_joined(raw: &str) -> String {
    let date = raw.get(..10).unwrap_or(raw);
    let mut parts = date.split('-');
    let year = parts.next().unwrap_or("");
    let month = match parts.next().unwrap_or("") {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return raw.to_string(),
    };
    let day = parts
        .next()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0);
    if year.len() != 4 || day == 0 {
        return raw.to_string();
    }
    format!("{day} {month} {year}")
}

pub fn format_published(count: u32) -> String {
    match count {
        0 => "None yet".into(),
        1 => "1 pack".into(),
        n => format!("{n} packs"),
    }
}

pub fn decode_avatar(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let image = image::load_from_memory(bytes).map_err(|_| "Could not read the profile photo.")?;
    let image = image.resize(128, 128, image::imageops::FilterType::Triangle);
    let rgba = image.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

pub fn resolve_api_base() -> String {
    std::env::var(API_ENV)
        .ok()
        .map(|value| normalize_api_base(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| normalize_api_base(DEFAULT_API_BASE))
}

pub fn normalize_email(raw: &str) -> Result<String, String> {
    let email = raw.trim().to_ascii_lowercase();
    if email.is_empty() {
        return Err("Enter an email address.".into());
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err("That email is not valid.".into());
    };
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || local.starts_with('.')
        || local.ends_with('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || email.contains("..")
        || email
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        || !email
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '+' | '-'))
    {
        return Err("That email is not valid.".into());
    }
    Ok(email)
}

pub fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().any(|ch| ch.is_control()) {
        return Err("Password contains characters that cannot be used.".into());
    }
    if password.chars().count() < 8 {
        return Err("Password must be at least 8 characters.".into());
    }
    if password.chars().count() > 256 {
        return Err("Password is too long.".into());
    }
    Ok(())
}

pub fn normalize_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Enter your name.".into());
    }
    if name.chars().any(|ch| ch.is_control()) {
        return Err("Name contains characters that cannot be used.".into());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("Name must be {NAME_MAX} characters or fewer."));
    }
    Ok(name.to_string())
}

pub fn normalize_username(raw: &str) -> Result<String, String> {
    let username = raw.trim().trim_start_matches('@').to_ascii_lowercase();
    if username.is_empty() {
        return Err("Choose a username.".into());
    }
    if username.len() > USERNAME_MAX {
        return Err("Use letters, numbers, and hyphens.".into());
    }
    let valid = username.chars().enumerate().all(|(index, ch)| match ch {
        'a'..='z' | '0'..='9' => true,
        '-' => index > 0 && index + 1 < username.len(),
        _ => false,
    });
    if !valid {
        return Err("Use letters, numbers, and hyphens.".into());
    }
    Ok(username)
}

fn is_remote_session_file(data: &AuthFile) -> bool {
    data.cookie.is_some() || data.token.is_some() || data.user.is_some() || data.api_base.is_some()
}

fn normalize_api_base(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

fn user_agent() -> String {
    format!("Mouse-Me/{} (Linux)", env!("CARGO_PKG_VERSION"))
}

fn urlencoding_path(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => out.push(ch),
            _ => {
                for byte in ch.encode_utf8(&mut [0; 4]).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn network_error(base: &str, detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("connection refused") || lower.contains("failed to connect") {
        format!("Could not reach Mouse Me at {base}. Start the website, or set {API_ENV}.")
    } else {
        "Could not reach Mouse Me. Check your connection and try again.".into()
    }
}

fn map_error_body(action: &str, status: u16, body: &str) -> String {
    let parsed: ErrorBody = serde_json::from_str(body).unwrap_or(ErrorBody {
        code: None,
        message: None,
    });
    match parsed.code.as_deref() {
        Some("INVALID_EMAIL_OR_PASSWORD") | Some("INVALID_PASSWORD") => {
            "Wrong email or password.".into()
        }
        Some("USER_ALREADY_EXISTS") | Some("USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL") => {
            "An account already exists for that email. Sign in instead.".into()
        }
        Some("INVALID_EMAIL") => "That email is not valid.".into(),
        Some("PASSWORD_TOO_SHORT") => "Password must be at least 8 characters.".into(),
        Some("PASSWORD_TOO_LONG") => "Password is too long.".into(),
        Some("FAILED_TO_CREATE_USER") => {
            "Could not create that account. The username or email may already be taken.".into()
        }
        Some("CROSS_SITE_NAVIGATION_LOGIN_BLOCKED") | Some("INVALID_ORIGIN") => {
            "Could not sign in because the site rejected this app. Try again.".into()
        }
        _ => {
            if let Some(message) = parsed
                .message
                .as_deref()
                .map(str::trim)
                .filter(|message| !message.is_empty())
            {
                if message.chars().count() <= 160 {
                    return message.to_string();
                }
            }
            match status {
                401 => "Wrong email or password.".into(),
                409 => "An account already exists for that email. Sign in instead.".into(),
                422 | 400 => format!("Could not {action}. Check the details and try again."),
                _ => format!("Could not {action} (HTTP {status})."),
            }
        }
    }
}

fn read_response(response: ureq::Response) -> Result<HttpResponse, String> {
    let status = response.status();
    let location = response.header("location").map(ToOwned::to_owned);
    let mut set_cookies = Vec::new();
    if let Some(value) = response.header("set-cookie") {
        set_cookies.push(value.to_string());
    }
    let body = response.into_string().unwrap_or_default();
    Ok(HttpResponse {
        status,
        set_cookies,
        body,
        location,
    })
}

fn request_origins(api_base: &str) -> Vec<String> {
    vec![api_base.to_string()]
}

fn is_allowed_avatar_url(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host.split(':').next().unwrap_or(host);
        return host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost");
    }
    false
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        return None;
    };
    let host = rest.split(['/', '?', '#']).next()?.trim();
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let host = match scheme {
        "https" => host.strip_suffix(":443").unwrap_or(&host).to_string(),
        "http" => host.strip_suffix(":80").unwrap_or(&host).to_string(),
        _ => host,
    };
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

fn is_invalid_origin(body: &str) -> bool {
    body.contains("INVALID_ORIGIN") || body.to_ascii_lowercase().contains("invalid origin")
}

fn is_safe_api_path(path: &str) -> bool {
    path.starts_with("/api/")
        && !path.contains("..")
        && !path.contains('\\')
        && path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
}

fn map_download_error(status: u16, body: &str) -> String {
    let parsed: ErrorBody = serde_json::from_str(body).unwrap_or(ErrorBody {
        code: None,
        message: None,
    });
    if let Some(message) = parsed
        .message
        .as_deref()
        .or(parsed.code.as_deref())
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        if message.chars().count() <= 160 {
            return message.to_string();
        }
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(error) = value.get("error").and_then(|value| value.as_str()) {
            let error = error.trim();
            if !error.is_empty() && error.chars().count() <= 160 {
                return error.to_string();
            }
        }
    }
    match status {
        404 => "Pack archive not found or not published.".into(),
        401 => "Sign in to download that pack.".into(),
        _ => format!("Could not download the pack (HTTP {status})."),
    }
}

fn resolve_redirect(current: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    if location.starts_with('/') {
        if let Some(scheme_end) = current.find("://") {
            let host = current[scheme_end + 3..]
                .split('/')
                .next()
                .unwrap_or(&current[scheme_end + 3..]);
            return format!("{}://{}{}", &current[..scheme_end], host, location);
        }
    }
    location.to_string()
}

fn merge_cookie_header(existing: &str, set_cookies: &[String]) -> String {
    let mut cookies = BTreeMap::new();
    for part in existing.split(';') {
        insert_cookie(&mut cookies, part);
    }
    for header in set_cookies {
        if let Some(pair) = header.split(';').next() {
            insert_cookie(&mut cookies, pair);
        }
    }
    cookies
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn insert_cookie(cookies: &mut BTreeMap<String, String>, pair: &str) {
    let pair = pair.trim();
    if pair.is_empty() {
        return;
    }
    let (name, value) = match pair.split_once('=') {
        Some((name, value)) => (name.trim(), value.trim()),
        None => return,
    };
    if name.is_empty() || name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("domain") {
        return;
    }
    if value.is_empty() {
        cookies.remove(name);
        return;
    }
    cookies.insert(name.to_string(), value.to_string());
}

#[cfg(test)]
mod internals {
    use super::{
        map_error_body, merge_cookie_header, normalize_api_base, normalize_name,
        normalize_username, resolve_redirect,
    };

    #[test]
    fn api_base_drops_trailing_slash() {
        assert_eq!(
            normalize_api_base("https://mouse-me-web.vercel.app/"),
            "https://mouse-me-web.vercel.app"
        );
        assert_eq!(
            resolve_redirect(
                "https://mouse-me-web.vercel.app//api/auth/sign-in/email",
                "/api/auth/sign-in/email"
            ),
            "https://mouse-me-web.vercel.app/api/auth/sign-in/email"
        );
    }

    #[test]
    fn username_rules_match_the_website() {
        assert_eq!(normalize_username(" @Grenish-Rai ").unwrap(), "grenish-rai");
        assert!(normalize_username("")
            .unwrap_err()
            .contains("Choose a username"));
        assert!(normalize_username("-nope").unwrap_err().contains("letters"));
        assert!(normalize_username("nope-").unwrap_err().contains("letters"));
        assert!(normalize_username("no pe").unwrap_err().contains("letters"));
    }

    #[test]
    fn name_is_required() {
        assert_eq!(normalize_name("  Grenish  ").unwrap(), "Grenish");
        assert!(normalize_name("   ")
            .unwrap_err()
            .contains("Enter your name"));
    }

    #[test]
    fn cookie_merge_keeps_session_token() {
        let merged = merge_cookie_header(
            "",
            &["better-auth.session_token=abc; Path=/; HttpOnly".into()],
        );
        assert_eq!(merged, "better-auth.session_token=abc");
        let updated =
            merge_cookie_header(&merged, &["better-auth.session_token=xyz; Path=/".into()]);
        assert_eq!(updated, "better-auth.session_token=xyz");
    }

    #[test]
    fn origin_stays_on_the_api_host() {
        assert_eq!(
            super::origin_of("https://mouse-me-web.vercel.app/api/auth/sign-in/email").as_deref(),
            Some("https://mouse-me-web.vercel.app")
        );
        assert_eq!(
            super::origin_of("https://Mouse-Me-Web.vercel.app/api"),
            super::origin_of("https://mouse-me-web.vercel.app:443/api")
        );
        assert_ne!(
            super::origin_of("https://mouse-me-web.vercel.app/api"),
            super::origin_of("https://evil.example/api")
        );
        assert!(!super::is_allowed_avatar_url("http://evil.example/a.png"));
        assert!(super::is_allowed_avatar_url("https://cdn.example/a.png"));
        assert!(super::is_allowed_avatar_url("http://127.0.0.1:3000/a.png"));
    }

    #[test]
    fn better_auth_error_codes_are_readable() {
        assert_eq!(
            map_error_body(
                "sign in",
                401,
                r#"{"code":"INVALID_EMAIL_OR_PASSWORD","message":"Invalid email or password"}"#
            ),
            "Wrong email or password."
        );
        assert!(map_error_body(
            "create an account",
            422,
            r#"{"code":"USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL"}"#
        )
        .contains("already exists"));
    }

    #[test]
    fn joined_date_is_readable() {
        assert_eq!(
            super::format_joined("2026-08-22T17:05:39.000Z"),
            "22 Aug 2026"
        );
        assert_eq!(super::format_published(0), "None yet");
        assert_eq!(super::format_published(1), "1 pack");
        assert_eq!(super::format_published(4), "4 packs");
    }
}
