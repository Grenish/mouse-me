use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use flate2::read::GzDecoder;

use super::fsutil::set_secret_mode;

pub const REPO: &str = "Grenish/mouse-me";
const USER_AGENT: &str = concat!("Mouse-Me/", env!("CARGO_PKG_VERSION"));
const ARCHIVE_NAME: &str = "linux-x86_64.tar.gz";
const MAX_DOWNLOAD_BYTES: u64 = 80_000_000;

#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub archive_url: String,
    pub checksum_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingUpdate {
    version: String,
    path: String,
    #[serde(default)]
    sha256: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    version_cmp(latest, current) == std::cmp::Ordering::Greater
}

pub fn version_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |s: &str| {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .collect::<Vec<_>>()
    };
    let mut a = parse(left);
    let mut b = parse(right);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a.cmp(&b)
}

pub fn latest_release() -> Result<Release, String> {
    let body = http_get_text(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;
    parse_release_json(&body)
}

pub fn parse_release_json(body: &str) -> Result<Release, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|error| error.to_string())?;
    let tag = parsed
        .get("tag_name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if tag.is_empty() {
        return Err("no-release".into());
    }
    let assets = parsed
        .get("assets")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let archive_name = format!("mouse-me-{tag}-{ARCHIVE_NAME}");
    let checksum_name = format!("{archive_name}.sha256");
    let archive_url = asset_url(&assets, &archive_name)
        .ok_or_else(|| format!("Release {tag} has no Linux archive."))?;
    let checksum_url = asset_url(&assets, &checksum_name)
        .ok_or_else(|| format!("Release {tag} has no checksum file."))?;
    if !is_allowed_update_url(&archive_url) || !is_allowed_update_url(&checksum_url) {
        return Err("Release asset URL is not a trusted GitHub host.".into());
    }
    Ok(Release {
        tag: tag.to_string(),
        version: tag.trim_start_matches('v').to_string(),
        archive_url,
        checksum_url,
    })
}

pub fn apply_pending_update() -> Result<bool, String> {
    let Some(pending) = load_pending()? else {
        return Ok(false);
    };
    if !is_newer(&pending.version, current_version()) {
        let _ = clear_pending();
        return Ok(false);
    }
    if let Err(error) = verify_pending(&pending) {
        let _ = clear_pending();
        if pending.sha256.is_empty() {
            return Ok(false);
        }
        return Err(error);
    }
    let staged = PathBuf::from(&pending.path);
    install_file(&staged)?;
    let _ = clear_pending();
    relaunch()
}

pub fn stage_update(release: &Release) -> Result<PathBuf, String> {
    let binary = download_verified_binary(release)?;
    let hash = sha256_hex(&binary);
    let dir = updates_dir()?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(staged_file_name(&release.version)?);
    write_executable(&path, &binary)?;
    save_pending(&PendingUpdate {
        version: release.version.clone(),
        path: path.to_string_lossy().into_owned(),
        sha256: hash,
    })?;
    Ok(path)
}

pub fn install_update(release: &Release) -> Result<PathBuf, String> {
    let binary = download_verified_binary(release)?;
    let dest = executable_path()?;
    let staged = dest.with_extension("new");
    write_executable(&staged, &binary)?;
    replace_executable(&staged, &dest)?;
    let _ = clear_pending();
    Ok(dest)
}

pub fn relaunch() -> Result<bool, String> {
    let exe = executable_path()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    Command::new(&exe)
        .args(args)
        .spawn()
        .map_err(|error| format!("Could not restart Mouse Me: {error}"))?;
    std::process::exit(0);
}

pub fn parse_sha256_file(text: &str) -> Result<String, String> {
    let hash = text
        .lines()
        .find_map(|line| {
            let token = line.split_whitespace().next().unwrap_or("");
            if token.len() == 64 && token.chars().all(|ch| ch.is_ascii_hexdigit()) {
                Some(token.to_ascii_lowercase())
            } else {
                None
            }
        })
        .ok_or_else(|| "Checksum file is invalid.".to_string())?;
    Ok(hash)
}

pub fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut tar = tar::Archive::new(decoder);
    let entries = tar
        .entries()
        .map_err(|error| format!("Could not read the update archive: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not read the update archive: {error}"))?;
        let kind = entry.header().entry_type();
        if kind.is_symlink()
            || kind.is_hard_link()
            || kind.is_fifo()
            || kind.is_block_special()
            || kind.is_character_special()
        {
            return Err("Update archive contains a disallowed entry.".into());
        }
        let path = entry
            .path()
            .map_err(|error| format!("Could not read the update archive: {error}"))?
            .into_owned();
        if path.as_os_str() != "mouse-me" {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .take(MAX_DOWNLOAD_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read the update archive: {error}"))?;
        if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err("Update archive entry is too large.".into());
        }
        if bytes.is_empty() {
            return Err("The update archive did not contain mouse-me.".into());
        }
        return Ok(bytes);
    }
    Err("The update archive did not contain mouse-me.".into())
}

pub fn is_allowed_update_url(url: &str) -> bool {
    let Some(host) = https_host(url) else {
        return false;
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    host == "github.com" || host == "api.github.com" || host.ends_with(".githubusercontent.com")
}

fn download_verified_binary(release: &Release) -> Result<Vec<u8>, String> {
    if !is_allowed_update_url(&release.archive_url) || !is_allowed_update_url(&release.checksum_url)
    {
        return Err("Release asset URL is not a trusted GitHub host.".into());
    }
    let archive = http_get_bytes(&release.archive_url)?;
    let checksum_text = http_get_text(&release.checksum_url)?;
    let expected = parse_sha256_file(&checksum_text)?;
    let actual = sha256_hex(&archive);
    if expected != actual {
        return Err("Update checksum did not match.".into());
    }
    extract_binary(&archive)
}

fn verify_pending(pending: &PendingUpdate) -> Result<(), String> {
    if pending.sha256.len() != 64 || !pending.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("Pending update is missing a checksum.".into());
    }
    let dir = updates_dir()?;
    let staged = PathBuf::from(&pending.path);
    if !staged_path_is_allowed(&dir, &staged, &pending.version) {
        return Err("Pending update path is not allowed.".into());
    }
    let bytes = fs::read(&staged).map_err(|_| "The downloaded update is missing.".to_string())?;
    if sha256_hex(&bytes) != pending.sha256.to_ascii_lowercase() {
        return Err("Pending update checksum did not match.".into());
    }
    Ok(())
}

fn staged_file_name(version: &str) -> Result<String, String> {
    if version.is_empty()
        || !version.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        || version.starts_with('.')
        || version.ends_with('.')
        || version.contains("..")
    {
        return Err("Update version is invalid.".into());
    }
    Ok(format!("mouse-me-{version}"))
}

fn staged_path_is_allowed(updates_dir: &Path, staged: &Path, version: &str) -> bool {
    let Ok(expected_name) = staged_file_name(version) else {
        return false;
    };
    let Ok(dir) = updates_dir.canonicalize() else {
        return false;
    };
    let Ok(file) = staged.canonicalize() else {
        return false;
    };
    if !file.starts_with(&dir) {
        return false;
    }
    file.file_name().and_then(|name| name.to_str()) == Some(expected_name.as_str())
        && file
            .components()
            .all(|component| !matches!(component, Component::ParentDir | Component::Prefix(_)))
}

fn asset_url(assets: &[serde_json::Value], name: &str) -> Option<String> {
    assets.iter().find_map(|asset| {
        let asset_name = asset.get("name").and_then(|value| value.as_str())?;
        if asset_name == name {
            asset
                .get("browser_download_url")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        } else {
            None
        }
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.split('@').next_back()?;
    if host.is_empty() || host.contains('\\') {
        return None;
    }
    Some(host.split(':').next().unwrap_or(host))
}

fn http_get_text(url: &str) -> Result<String, String> {
    let bytes = http_get_bytes(url)?;
    String::from_utf8(bytes).map_err(|_| "GitHub returned invalid text.".to_string())
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    if !is_allowed_update_url(url) {
        return Err("Refusing to download from an untrusted host.".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .redirects(0)
        .user_agent(USER_AGENT)
        .build();
    let accept = if url.contains("api.github.com") {
        "application/vnd.github+json"
    } else {
        "application/octet-stream"
    };
    let mut current = url.to_string();
    for _ in 0..8 {
        let response = match agent.get(&current).set("Accept", accept).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(404, _)) => return Err("no-release".into()),
            Err(ureq::Error::Status(code, response))
                if matches!(code, 301 | 302 | 303 | 307 | 308) =>
            {
                current = follow_https_redirect(&current, response.header("location"))?;
                continue;
            }
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!("GitHub returned HTTP {code}."));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(format!("Could not reach GitHub: {error}"));
            }
        };
        if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            current = follow_https_redirect(&current, response.header("location"))?;
            continue;
        }
        if !(200..300).contains(&response.status()) {
            if response.status() == 404 {
                return Err("no-release".into());
            }
            return Err(format!("GitHub returned HTTP {}.", response.status()));
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_DOWNLOAD_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not download the update: {error}"))?;
        return Ok(bytes);
    }
    Err("Too many redirects while downloading the update.".into())
}

fn follow_https_redirect(current: &str, location: Option<&str>) -> Result<String, String> {
    let location = location.ok_or_else(|| "GitHub redirect was missing a location.".to_string())?;
    let next = resolve_https_redirect(current, location)?;
    if !is_allowed_update_url(&next) {
        return Err("GitHub redirected to an untrusted host.".into());
    }
    Ok(next)
}

fn resolve_https_redirect(current: &str, location: &str) -> Result<String, String> {
    if location.starts_with("https://") {
        return Ok(location.to_string());
    }
    if location.starts_with("http://") {
        return Err("Refusing a non-HTTPS update redirect.".into());
    }
    if location.starts_with('/') {
        let host = https_host(current).ok_or("Invalid update URL.")?;
        return Ok(format!("https://{host}{location}"));
    }
    Err("Invalid update redirect.".into())
}

fn executable_path() -> Result<PathBuf, String> {
    let path = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(path.canonicalize().unwrap_or(path))
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, bytes).map_err(|error| format!("Could not write the update: {error}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("Could not make the update executable: {error}"))?;
    Ok(())
}

fn replace_executable(staged: &Path, dest: &Path) -> Result<(), String> {
    match fs::rename(staged, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(staged, dest)
                .map_err(|error| format!("Could not install the update: {error}"))?;
            let _ = fs::remove_file(staged);
            fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("Could not make the update executable: {error}"))?;
            Ok(())
        }
    }
}

fn install_file(staged: &Path) -> Result<(), String> {
    let dest = executable_path()?;
    let tmp = dest.with_extension("new");
    fs::copy(staged, &tmp).map_err(|error| format!("Could not install the update: {error}"))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("Could not make the update executable: {error}"))?;
    replace_executable(&tmp, &dest)
}

fn updates_dir() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .map(|dir| dir.join("mouse-me").join("updates"))
        .ok_or_else(|| "Could not locate the update directory.".to_string())
}

fn pending_path() -> Result<PathBuf, String> {
    Ok(updates_dir()?.join("pending.json"))
}

fn load_pending() -> Result<Option<PendingUpdate>, String> {
    let path = pending_path()?;
    let Ok(raw) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    Ok(serde_json::from_str(&raw).ok())
}

fn save_pending(pending: &PendingUpdate) -> Result<(), String> {
    let path = pending_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(pending).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    set_secret_mode(&path)
}

fn clear_pending() -> Result<(), String> {
    let path = pending_path()?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod internals {
    use super::{
        extract_binary, is_allowed_update_url, parse_release_json, parse_sha256_file,
        staged_path_is_allowed, version_cmp,
    };
    use std::cmp::Ordering;
    use std::fs;
    use std::io::Write;

    #[test]
    fn newer_tags_compare_numerically() {
        assert_eq!(version_cmp("0.2.1", "0.1.0"), Ordering::Greater);
        assert_eq!(version_cmp("v0.2.0", "0.2.0"), Ordering::Equal);
        assert_eq!(version_cmp("0.2.0", "0.2.1"), Ordering::Less);
    }

    #[test]
    fn checksum_ignores_dist_prefix() {
        let file = "bd2407bb240923c3d6bdeb65f6c054f0c139de17fd06215470440e170583714e  dist/mouse-me-v0.2.1-linux-x86_64.tar.gz\n";
        assert_eq!(
            parse_sha256_file(file).unwrap(),
            "bd2407bb240923c3d6bdeb65f6c054f0c139de17fd06215470440e170583714e"
        );
    }

    #[test]
    fn release_json_picks_linux_archive() {
        let body = r#"{
            "tag_name": "v0.2.1",
            "assets": [
                {
                    "name": "mouse-me-v0.2.1-linux-x86_64.tar.gz",
                    "browser_download_url": "https://github.com/Grenish/mouse-me/releases/download/v0.2.1/app.tgz"
                },
                {
                    "name": "mouse-me-v0.2.1-linux-x86_64.tar.gz.sha256",
                    "browser_download_url": "https://github.com/Grenish/mouse-me/releases/download/v0.2.1/app.sha256"
                }
            ]
        }"#;
        let release = parse_release_json(body).unwrap();
        assert_eq!(release.tag, "v0.2.1");
        assert_eq!(release.version, "0.2.1");
        assert!(release.archive_url.contains("github.com"));
    }

    #[test]
    fn untrusted_asset_host_is_rejected() {
        let body = r#"{
            "tag_name": "v0.2.1",
            "assets": [
                {
                    "name": "mouse-me-v0.2.1-linux-x86_64.tar.gz",
                    "browser_download_url": "https://evil.example/app.tgz"
                },
                {
                    "name": "mouse-me-v0.2.1-linux-x86_64.tar.gz.sha256",
                    "browser_download_url": "https://evil.example/app.sha256"
                }
            ]
        }"#;
        assert!(parse_release_json(body).unwrap_err().contains("trusted"));
        assert!(!is_allowed_update_url("http://github.com/foo"));
        assert!(is_allowed_update_url(
            "https://objects.githubusercontent.com/foo"
        ));
    }

    #[test]
    fn extract_reads_root_mouse_me_only() {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut decoy = tar::Header::new_gnu();
            decoy.set_size(4);
            decoy.set_cksum();
            builder
                .append_data(&mut decoy, "docs/mouse-me", &b"bad\n"[..])
                .unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_cksum();
            builder
                .append_data(&mut header, "mouse-me", &b"bin\n"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut gz = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            encoder.write_all(&tar_bytes).unwrap();
            encoder.finish().unwrap();
        }
        assert_eq!(extract_binary(&gz).unwrap(), b"bin\n");
    }

    #[test]
    fn pending_path_must_stay_in_updates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let updates = dir.path().join("updates");
        fs::create_dir_all(&updates).unwrap();
        let ok = updates.join("mouse-me-0.3.0");
        fs::write(&ok, b"bin").unwrap();
        assert!(staged_path_is_allowed(&updates, &ok, "0.3.0"));
        let outside = dir.path().join("evil");
        fs::write(&outside, b"bin").unwrap();
        assert!(!staged_path_is_allowed(&updates, &outside, "0.3.0"));
    }
}
