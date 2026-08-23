use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use flate2::read::GzDecoder;

pub const REPO: &str = "Grenish/mouse-me";
const USER_AGENT: &str = concat!("Mouse-Me/", env!("CARGO_PKG_VERSION"));
const ARCHIVE_NAME: &str = "linux-x86_64.tar.gz";

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
    let staged = PathBuf::from(&pending.path);
    if !staged.is_file() {
        let _ = clear_pending();
        return Err("The downloaded update is missing.".into());
    }
    install_file(&staged)?;
    let _ = clear_pending();
    relaunch()
}

pub fn stage_update(release: &Release) -> Result<PathBuf, String> {
    let binary = download_verified_binary(release)?;
    let dir = updates_dir()?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("mouse-me-{}", release.version));
    write_executable(&path, &binary)?;
    save_pending(&PendingUpdate {
        version: release.version.clone(),
        path: path.to_string_lossy().into_owned(),
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
        let mut entry =
            entry.map_err(|error| format!("Could not read the update archive: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("Could not read the update archive: {error}"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name != "mouse-me" {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read the update archive: {error}"))?;
        if bytes.is_empty() {
            return Err("The update archive did not contain mouse-me.".into());
        }
        return Ok(bytes);
    }
    Err("The update archive did not contain mouse-me.".into())
}

fn download_verified_binary(release: &Release) -> Result<Vec<u8>, String> {
    let archive = http_get_bytes(&release.archive_url)?;
    let checksum_text = http_get_text(&release.checksum_url)?;
    let expected = parse_sha256_file(&checksum_text)?;
    let actual = sha256_hex(&archive);
    if expected != actual {
        return Err("Update checksum did not match.".into());
    }
    extract_binary(&archive)
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
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn http_get_text(url: &str) -> Result<String, String> {
    let bytes = http_get_bytes(url)?;
    String::from_utf8(bytes).map_err(|_| "GitHub returned invalid text.".to_string())
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .redirects(8)
        .user_agent(USER_AGENT)
        .build();
    let accept = if url.contains("api.github.com") {
        "application/vnd.github+json"
    } else {
        "application/octet-stream"
    };
    let response = match agent.get(url).set("Accept", accept).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(404, _)) => return Err("no-release".into()),
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!("GitHub returned HTTP {code}."));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(format!("Could not reach GitHub: {error}"));
        }
    };
    if !(200..300).contains(&response.status()) {
        if response.status() == 404 {
            return Err("no-release".into());
        }
        return Err(format!("GitHub returned HTTP {}.", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(80_000_000)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not download the update: {error}"))?;
    Ok(bytes)
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
    .map_err(|error| error.to_string())
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
    use super::{extract_binary, parse_release_json, parse_sha256_file, version_cmp};
    use std::cmp::Ordering;
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
                    "browser_download_url": "https://example.test/app.tgz"
                },
                {
                    "name": "mouse-me-v0.2.1-linux-x86_64.tar.gz.sha256",
                    "browser_download_url": "https://example.test/app.sha256"
                }
            ]
        }"#;
        let release = parse_release_json(body).unwrap();
        assert_eq!(release.tag, "v0.2.1");
        assert_eq!(release.version, "0.2.1");
        assert_eq!(release.archive_url, "https://example.test/app.tgz");
    }

    #[test]
    fn extract_reads_mouse_me_from_tar_gz() {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
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
}
