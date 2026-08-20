use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

/// Imports a cursor theme archive or directory into ~/.local/share/icons.
/// Returns a list of imported theme names.
pub fn import_cursor_pack(source_path: &Path) -> Result<Vec<String>, String> {
    let home = dirs::home_dir().ok_or("Could not locate home directory")?;
    let target_icons_dir = home.join(".local").join("share").join("icons");
    import_cursor_pack_into(source_path, &target_icons_dir)
}

/// Imports a cursor theme archive or directory into `target_icons_dir`.
pub fn import_cursor_pack_into(
    source_path: &Path,
    target_icons_dir: &Path,
) -> Result<Vec<String>, String> {
    if !source_path.exists() {
        return Err(format!(
            "Source path '{}' does not exist",
            source_path.display()
        ));
    }

    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let extract_root = temp_dir.path();

    if source_path.is_dir() {
        // Source is an uncompressed directory
        copy_dir_all(source_path, extract_root)
            .map_err(|e| format!("Failed to copy source directory: {}", e))?;
    } else {
        // Source is an archive
        unpack_archive(source_path, extract_root)?;
    }

    // Search for theme directories containing cursors/ or hyprcursors/ or manifest.hl
    let candidate_dirs = find_theme_roots(extract_root);
    if candidate_dirs.is_empty() {
        return Err(
            "No valid cursor theme found in the archive (missing 'cursors/' or 'hyprcursors/')."
                .into(),
        );
    }

    fs::create_dir_all(target_icons_dir)
        .map_err(|e| format!("Failed to create '{}': {}", target_icons_dir.display(), e))?;

    let mut imported_names = Vec::new();
    let mut seen_names = HashSet::new();

    for theme_dir in candidate_dirs {
        let theme_name = determine_theme_name(&theme_dir);
        if !seen_names.insert(theme_name.clone()) {
            continue;
        }

        let dest = target_icons_dir.join(&theme_name);
        let staging_dir = tempfile::tempdir_in(&target_icons_dir)
            .map_err(|e| format!("Failed to create staging directory: {}", e))?;
        let staged_theme = staging_dir.path().join(&theme_name);

        copy_dir_all(&theme_dir, &staged_theme)
            .map_err(|e| format!("Failed to copy theme to '{}': {}", dest.display(), e))?;

        // Ensure index.theme exists before replacing a previously installed theme.
        ensure_index_theme(&staged_theme, &theme_name)?;

        if dest.exists() || fs::symlink_metadata(&dest).is_ok() {
            remove_existing_path(&dest)
                .map_err(|e| format!("Failed to replace '{}': {}", dest.display(), e))?;
        }
        fs::rename(&staged_theme, &dest)
            .map_err(|e| format!("Failed to install '{}': {}", dest.display(), e))?;

        imported_names.push(theme_name);
    }

    Ok(imported_names)
}

/// Unpacks various archive formats into destination
fn unpack_archive(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file_name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    if file_name.ends_with(".zip") {
        let file = File::open(archive_path).map_err(|e| format!("Failed to open zip: {}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {}", e))?;
        archive
            .extract(dest)
            .map_err(|e| format!("Zip extract error: {}", e))?;
        return Ok(());
    }

    let file = File::open(archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
    let reader = BufReader::new(file);

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(reader);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest)
            .map_err(|e| format!("Tar.gz unpack error: {}", e))?;
    } else if file_name.ends_with(".tar.xz") || file_name.ends_with(".txz") {
        let xz = xz2::read::XzDecoder::new(reader);
        let mut tar = tar::Archive::new(xz);
        tar.unpack(dest)
            .map_err(|e| format!("Tar.xz unpack error: {}", e))?;
    } else if file_name.ends_with(".tar.bz2") || file_name.ends_with(".tbz2") {
        let bz = bzip2::read::BzDecoder::new(reader);
        let mut tar = tar::Archive::new(bz);
        tar.unpack(dest)
            .map_err(|e| format!("Tar.bz2 unpack error: {}", e))?;
    } else if file_name.ends_with(".tar") {
        let mut tar = tar::Archive::new(reader);
        tar.unpack(dest)
            .map_err(|e| format!("Tar unpack error: {}", e))?;
    } else {
        // Try fallback detection by attempting zip, then tar.gz
        if let Ok(file_retry) = File::open(archive_path) {
            if let Ok(mut zip_arc) = zip::ZipArchive::new(file_retry) {
                if zip_arc.extract(dest).is_ok() {
                    return Ok(());
                }
            }
        }
        return Err(format!("Unsupported archive format: {}", file_name));
    }

    Ok(())
}

/// Recursively searches for theme root folders
fn find_theme_roots(root: &Path) -> Vec<PathBuf> {
    let mut theme_roots = Vec::new();

    for entry in WalkDir::new(root).max_depth(4).into_iter().flatten() {
        if entry.file_type().is_dir() {
            let p = entry.path();
            if p.join("cursors").is_dir()
                || p.join("hyprcursors").is_dir()
                || p.join("manifest.hl").is_file()
            {
                // Avoid picking child directories inside cursors/
                if !theme_roots.iter().any(|r: &PathBuf| p.starts_with(r)) {
                    theme_roots.push(p.to_path_buf());
                }
            }
        }
    }

    theme_roots
}

/// Determines the best name for the theme
fn determine_theme_name(theme_dir: &Path) -> String {
    let index_file = theme_dir.join("index.theme");
    if let Ok(content) = fs::read_to_string(&index_file) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("Name=") {
                let clean = val.trim();
                if !clean.is_empty() {
                    return sanitize_theme_name(clean);
                }
            }
        }
    }

    let fallback = theme_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("custom-cursor");
    sanitize_theme_name(fallback)
}

/// Returns whether a name is safe to use as a single child directory name.
pub fn is_safe_theme_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(|character| character.is_control())
}

fn sanitize_theme_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else if character.is_whitespace() {
                '-'
            } else {
                '_'
            }
        })
        .collect();

    if is_safe_theme_name(&cleaned) {
        cleaned
    } else {
        "custom-cursor".into()
    }
}

/// Auto-creates index.theme if absent so GTK and X11 cleanly recognize it
fn ensure_index_theme(theme_dir: &Path, name: &str) -> Result<(), String> {
    let index_file = theme_dir.join("index.theme");
    if !index_file.exists() {
        let content = format!(
            "[Icon Theme]\nName={}\nComment=Imported with mouse-me\nInherits=core\n",
            name
        );
        fs::write(&index_file, content).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Recursively copies a directory
fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if ty.is_symlink() {
            let target = fs::read_link(&src_path)?;
            if !is_safe_relative_link(&target) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsafe symlink target '{}'", target.display()),
                ));
            }
            std::os::unix::fs::symlink(target, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn is_safe_relative_link(target: &Path) -> bool {
    target.is_relative()
        && !target.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn remove_existing_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.file_type().is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_safe_theme_name, sanitize_theme_name};

    #[test]
    fn rejects_path_traversal_theme_names() {
        assert!(!is_safe_theme_name("."));
        assert!(!is_safe_theme_name(".."));
        assert!(!is_safe_theme_name("nested/theme"));
        assert!(!is_safe_theme_name("nested\\theme"));
        assert!(!is_safe_theme_name("bad\nname"));
    }

    #[test]
    fn sanitizes_metadata_into_a_safe_directory_name() {
        assert_eq!(sanitize_theme_name("Soft Cursor Pack"), "Soft-Cursor-Pack");
        assert_eq!(sanitize_theme_name("../theme"), ".._theme");
        assert_eq!(sanitize_theme_name(".."), "custom-cursor");
    }
}
