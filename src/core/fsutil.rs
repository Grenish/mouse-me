use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn set_secret_mode(path: &Path) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Could not lock down {}: {error}", path.display()))
}
