use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const MAX_ARCHIVE_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_ENTRIES: usize = 20_000;
const MAX_COMPRESSION_RATIO: u64 = 100;
const MAX_SEARCH_DEPTH: usize = 8;

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

    let source_metadata = fs::metadata(source_path)
        .map_err(|e| format!("Could not inspect '{}': {}", source_path.display(), e))?;
    if source_metadata.is_file() && source_metadata.len() > MAX_ARCHIVE_INPUT_BYTES {
        return Err(format!(
            "Source archive is too large (maximum is {} MiB)",
            MAX_ARCHIVE_INPUT_BYTES / (1024 * 1024)
        ));
    }

    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let extract_root = temp_dir.path();

    if source_path.is_dir() {
        copy_theme_tree(source_path, extract_root)
            .map_err(|e| format!("Failed to copy source directory: {}", e))?;
    } else {
        unpack_archive(source_path, extract_root)?;
    }

    validate_extracted_tree(extract_root)?;

    // Search for theme directories containing cursors/ or hyprcursors/ or manifest.hl
    let candidate_dirs = find_theme_roots(extract_root)?;
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
            return Err(format!(
                "Multiple imported themes resolve to the same directory name '{}'",
                theme_name
            ));
        }

        let dest = target_icons_dir.join(&theme_name);
        let staging_dir = tempfile::tempdir_in(&target_icons_dir)
            .map_err(|e| format!("Failed to create staging directory: {}", e))?;
        let staged_theme = staging_dir.path().join(&theme_name);

        copy_theme_tree(&theme_dir, &staged_theme)
            .map_err(|e| format!("Failed to copy theme to '{}': {}", dest.display(), e))?;

        // Ensure index.theme exists before replacing a previously installed theme.
        ensure_index_theme(&staged_theme, &theme_name)?;

        replace_staged_theme(&staged_theme, &dest, target_icons_dir)?;

        imported_names.push(theme_name);
    }

    Ok(imported_names)
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    Zip,
    TarGz,
    TarXz,
    TarBz2,
    Tar,
}

fn detect_archive_kind(archive_path: &Path) -> Option<ArchiveKind> {
    let file_name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    if file_name.ends_with(".zip") {
        return Some(ArchiveKind::Zip);
    }
    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") || file_name.ends_with(".gz") {
        return Some(ArchiveKind::TarGz);
    }
    if file_name.ends_with(".tar.xz") || file_name.ends_with(".txz") || file_name.ends_with(".xz") {
        return Some(ArchiveKind::TarXz);
    }
    if file_name.ends_with(".tar.bz2")
        || file_name.ends_with(".tbz2")
        || file_name.ends_with(".bz2")
    {
        return Some(ArchiveKind::TarBz2);
    }
    if file_name.ends_with(".tar") {
        return Some(ArchiveKind::Tar);
    }

    let mut magic = [0u8; 6];
    let mut file = File::open(archive_path).ok()?;
    let read = file.read(&mut magic).ok()?;
    if read >= 2 && magic.starts_with(b"PK") {
        return Some(ArchiveKind::Zip);
    }
    if read >= 2 && magic.get(0..2) == Some(&[0x1f, 0x8b]) {
        return Some(ArchiveKind::TarGz);
    }
    if read >= 6 && magic == [0xfd, b'7', b'z', b'X', b'Z', 0x00] {
        return Some(ArchiveKind::TarXz);
    }
    if read >= 3 && magic.starts_with(b"BZh") {
        return Some(ArchiveKind::TarBz2);
    }
    None
}

/// Unpacks various archive formats into destination, entry by entry.
fn unpack_archive(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let kind = detect_archive_kind(archive_path).ok_or_else(|| {
        format!(
            "Unsupported archive format: {}",
            archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        )
    })?;

    match kind {
        ArchiveKind::Zip => unpack_zip(archive_path, dest),
        ArchiveKind::TarGz => {
            let file =
                File::open(archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
            unpack_tar(
                tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file))),
                dest,
            )
        }
        ArchiveKind::TarXz => {
            let file =
                File::open(archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
            unpack_tar(
                tar::Archive::new(xz2::read::XzDecoder::new(BufReader::new(file))),
                dest,
            )
        }
        ArchiveKind::TarBz2 => {
            let file =
                File::open(archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
            unpack_tar(
                tar::Archive::new(bzip2::read::BzDecoder::new(BufReader::new(file))),
                dest,
            )
        }
        ArchiveKind::Tar => {
            let file =
                File::open(archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
            unpack_tar(tar::Archive::new(BufReader::new(file)), dest)
        }
    }
}

fn unpack_zip(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|e| format!("Failed to open zip: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {}", e))?;
    if archive.len() > MAX_EXTRACTED_ENTRIES {
        return Err(format!(
            "Archive contains too many entries (maximum is {MAX_EXTRACTED_ENTRIES})"
        ));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Zip extract error: {}", e))?;
        if entry.is_symlink() {
            continue;
        }
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| "Archive contains an unsafe path.".to_string())?;
        if !is_safe_rel_path(&rel) {
            return Err(format!(
                "Archive contains an unsafe path '{}'",
                rel.display()
            ));
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        let size = entry.size();
        enforce_ratio(entry.compressed_size(), size)?;
        total = account_extracted(total, size)?;
        write_allowed_file(&out, &rel, &mut entry, size)?;
    }
    Ok(())
}

fn unpack_tar<R: Read>(mut archive: tar::Archive<R>, dest: &Path) -> Result<(), String> {
    let mut total = 0u64;
    let mut entries = 0usize;
    for entry in archive
        .entries()
        .map_err(|e| format!("Tar unpack error: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Tar unpack error: {}", e))?;
        entries += 1;
        if entries > MAX_EXTRACTED_ENTRIES {
            return Err(format!(
                "Archive contains too many entries (maximum is {MAX_EXTRACTED_ENTRIES})"
            ));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink()
            || kind.is_hard_link()
            || kind.is_fifo()
            || kind.is_block_special()
            || kind.is_character_special()
            || kind.is_gnu_sparse()
        {
            continue;
        }
        let rel = entry
            .path()
            .map_err(|e| format!("Tar unpack error: {}", e))?
            .into_owned();
        if !is_safe_rel_path(&rel) {
            return Err(format!(
                "Archive contains an unsafe path '{}'",
                rel.display()
            ));
        }
        let out = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        let size = entry.header().size().unwrap_or(0);
        total = account_extracted(total, size)?;
        write_allowed_file(&out, &rel, &mut entry, size)?;
    }
    Ok(())
}

fn write_allowed_file(
    out: &Path,
    rel: &Path,
    reader: &mut impl Read,
    declared: u64,
) -> Result<(), String> {
    if !is_allowed_asset(rel) {
        return Ok(());
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut header = [0u8; 8];
    let n = reader
        .read(&mut header)
        .map_err(|e| format!("Could not read archive file '{}': {}", rel.display(), e))?;
    if is_forbidden_payload(&header[..n]) {
        return Err(format!(
            "Archive contains an executable payload '{}'",
            rel.display()
        ));
    }
    let mut file = File::create(out).map_err(|e| e.to_string())?;
    file.write_all(&header[..n]).map_err(|e| e.to_string())?;
    let budget = MAX_EXTRACTED_BYTES.min(declared.saturating_add(4096));
    let copied = io::copy(&mut reader.take(budget), &mut file).map_err(|e| e.to_string())?;
    if declared > 0 && (n as u64).saturating_add(copied) > declared.saturating_add(16) {
        return Err(format!(
            "Archive file '{}' exceeded its declared size",
            rel.display()
        ));
    }
    Ok(())
}

fn account_extracted(total: u64, extra: u64) -> Result<u64, String> {
    let next = total
        .checked_add(extra)
        .ok_or("Extracted archive size overflow")?;
    if extra > MAX_EXTRACTED_BYTES || next > MAX_EXTRACTED_BYTES {
        return Err(format!(
            "Extracted archive is too large (maximum is {} MiB)",
            MAX_EXTRACTED_BYTES / (1024 * 1024)
        ));
    }
    Ok(next)
}

fn enforce_ratio(compressed: u64, uncompressed: u64) -> Result<(), String> {
    if compressed > 0
        && uncompressed > 1024 * 1024
        && uncompressed / compressed.max(1) > MAX_COMPRESSION_RATIO
    {
        return Err("Archive compression ratio is too high.".into());
    }
    Ok(())
}

fn is_safe_rel_path(path: &Path) -> bool {
    !path.is_absolute()
        && !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn is_allowed_asset(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    if matches!(
        name.as_str(),
        "index.theme"
            | "cursor.theme"
            | "manifest.hl"
            | "manifest.toml"
            | "readme"
            | "readme.md"
            | "readme.txt"
            | "license"
            | "license.md"
            | "license.txt"
            | "copying"
            | "copying.txt"
    ) {
        return true;
    }
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        return matches!(
            ext.to_ascii_lowercase().as_str(),
            "png"
                | "svg"
                | "jpg"
                | "jpeg"
                | "webp"
                | "hl"
                | "toml"
                | "theme"
                | "cursor"
                | "cur"
                | "ani"
                | "ico"
        );
    }
    path.components().any(|component| {
        component.as_os_str() == "cursors" || component.as_os_str() == "hyprcursors"
    })
}

fn is_forbidden_payload(header: &[u8]) -> bool {
    header.starts_with(&[0x7f, b'E', b'L', b'F'])
        || header.starts_with(b"MZ")
        || header.starts_with(b"#!")
}

fn validate_extracted_tree(root: &Path) -> Result<(), String> {
    let mut entries = 0usize;
    let mut total_bytes = 0u64;

    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry.map_err(|e| format!("Could not validate extracted archive: {}", e))?;
        entries = entries
            .checked_add(1)
            .ok_or("Extracted archive entry count overflow")?;
        if entries > MAX_EXTRACTED_ENTRIES {
            return Err(format!(
                "Archive contains too many entries (maximum is {})",
                MAX_EXTRACTED_ENTRIES
            ));
        }

        if entry.file_type().is_file() {
            let bytes = entry
                .metadata()
                .map_err(|e| format!("Could not inspect extracted file: {}", e))?
                .len();
            total_bytes = total_bytes
                .checked_add(bytes)
                .ok_or("Extracted archive size overflow")?;
            if bytes > MAX_EXTRACTED_BYTES || total_bytes > MAX_EXTRACTED_BYTES {
                return Err(format!(
                    "Extracted archive is too large (maximum is {} MiB)",
                    MAX_EXTRACTED_BYTES / (1024 * 1024)
                ));
            }
        }
    }
    Ok(())
}

fn replace_staged_theme(staged: &Path, dest: &Path, target_root: &Path) -> Result<(), String> {
    let backup_dir = tempfile::tempdir_in(target_root)
        .map_err(|e| format!("Failed to create replacement backup: {}", e))?;
    let backup = backup_dir.path().join("previous-theme");
    let had_existing = fs::symlink_metadata(dest).is_ok();

    if had_existing {
        fs::rename(dest, &backup)
            .map_err(|e| format!("Could not stage existing theme '{}': {}", dest.display(), e))?;
    }

    if let Err(error) = fs::rename(staged, dest) {
        if had_existing {
            let _ = fs::rename(&backup, dest);
        }
        return Err(format!("Failed to install '{}': {}", dest.display(), error));
    }

    Ok(())
}

/// Recursively searches for theme root folders
fn find_theme_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut theme_roots = Vec::new();

    for entry in WalkDir::new(root).max_depth(MAX_SEARCH_DEPTH).into_iter() {
        let entry = entry.map_err(|e| format!("Could not inspect extracted archive: {}", e))?;
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

    Ok(theme_roots)
}

/// Determines the best name for the theme
fn determine_theme_name(theme_dir: &Path) -> String {
    let index_file = theme_dir.join("index.theme");
    if let Ok(content) = fs::read_to_string(&index_file) {
        let mut in_icon_theme = false;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                in_icon_theme = line.eq_ignore_ascii_case("[Icon Theme]");
                continue;
            }
            if in_icon_theme {
                if let Some(val) = line.strip_prefix("Name=") {
                    let clean = val.trim();
                    if !clean.is_empty() {
                        return sanitize_theme_name(clean);
                    }
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
            "[Icon Theme]\nName={}\nComment=Imported with mouse-me\n",
            name
        );
        fs::write(&index_file, content).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn copy_theme_tree(src: &Path, dst: &Path) -> io::Result<()> {
    copy_theme_tree_inner(src, dst, Path::new(""))
}

fn copy_theme_tree_inner(src: &Path, dst: &Path, rel: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let child_rel = rel.join(entry.file_name());

        if ty.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsafe symlink '{}'", src_path.display()),
            ));
        }
        if ty.is_dir() {
            copy_theme_tree_inner(&src_path, &dst_path, &child_rel)?;
        } else if is_allowed_asset(&child_rel) {
            let mut header = [0u8; 8];
            let n = File::open(&src_path)?.read(&mut header)?;
            if is_forbidden_payload(&header[..n]) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("executable payload '{}'", src_path.display()),
                ));
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
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
