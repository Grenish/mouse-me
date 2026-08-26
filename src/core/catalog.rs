use serde::Deserialize;
use std::path::Path;

use super::auth::AuthStore;
use super::importer::import_cursor_pack;

const MAX_PACK_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPack {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub owner_username: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct PacksBody {
    #[serde(default)]
    packs: Vec<RemotePack>,
}

#[derive(Debug, Deserialize)]
struct RemotePack {
    id: String,
    name: String,
    slug: String,
    #[serde(default, alias = "ownerUsername")]
    owner_username: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

impl RemotePack {
    fn into_pack(self) -> Option<CatalogPack> {
        let id = self.id.trim();
        let name = self.name.trim();
        let slug = self.slug.trim();
        if id.is_empty() || name.is_empty() || slug.is_empty() {
            return None;
        }
        if !is_pack_id(id) {
            return None;
        }
        Some(CatalogPack {
            id: id.to_string(),
            name: name.to_string(),
            slug: slug.to_string(),
            owner_username: self
                .owner_username
                .unwrap_or_default()
                .trim()
                .trim_start_matches('@')
                .to_string(),
            version: self.version.unwrap_or_default().trim().to_string(),
        })
    }
}

pub fn looks_like_filesystem_source(source: &str) -> bool {
    let source = source.trim();
    if source.is_empty() {
        return false;
    }
    Path::new(source).exists()
        || source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with('/')
        || source.starts_with("~/")
        || source.starts_with("file:")
}

pub fn parse_packs_json(body: &str) -> Result<Vec<CatalogPack>, String> {
    let parsed: PacksBody = serde_json::from_str(body)
        .map_err(|_| "Mouse Me returned an unexpected catalog.".to_string())?;
    Ok(parsed
        .packs
        .into_iter()
        .filter_map(RemotePack::into_pack)
        .collect())
}

pub fn list_packs(store: &AuthStore) -> Result<Vec<CatalogPack>, String> {
    let (status, body) = store.fetch_text("/api/packs")?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not load the catalog (HTTP {status})."));
    }
    parse_packs_json(&body)
}

pub fn resolve_pack<'a>(packs: &'a [CatalogPack], query: &str) -> Result<&'a CatalogPack, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Enter a pack name.".into());
    }
    let lookup = lookup_query(query);
    if lookup.is_empty() {
        return Err("Enter a pack name.".into());
    }

    if let Some(id) = pack_id_from_query(&lookup) {
        return packs
            .iter()
            .find(|pack| pack.id.eq_ignore_ascii_case(id))
            .ok_or_else(|| format!("No published pack matches '{query}'."));
    }

    let (owner, name) = split_owner_name(&lookup);
    let needle = slugify(name);
    if needle.is_empty() {
        return Err("Enter a pack name.".into());
    }

    let matches: Vec<&CatalogPack> = packs
        .iter()
        .filter(|pack| pack_matches(pack, owner, name, &needle))
        .collect();

    match matches.as_slice() {
        [pack] => Ok(*pack),
        [] => Err(no_match_message(packs, query, &needle)),
        many => Err(ambiguous_message(query, many)),
    }
}

pub fn download_and_import(store: &AuthStore, pack: &CatalogPack) -> Result<Vec<String>, String> {
    if !is_pack_id(&pack.id) {
        return Err("Pack id is not valid.".into());
    }
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let dest = dir.path().join("pack.bin");
    store.download_to(
        &format!("/api/packs/{}/download", pack.id),
        &dest,
        MAX_PACK_BYTES,
    )?;
    import_cursor_pack(&dest)
}

pub fn install_named_pack(
    store: &AuthStore,
    query: &str,
) -> Result<(CatalogPack, Vec<String>), String> {
    let packs = list_packs(store)?;
    let pack = resolve_pack(&packs, query)?.clone();
    let imported = download_and_import(store, &pack)?;
    Ok((pack, imported))
}

fn pack_matches(pack: &CatalogPack, owner: Option<&str>, name: &str, needle: &str) -> bool {
    if let Some(owner) = owner {
        if !pack.owner_username.eq_ignore_ascii_case(owner) {
            return false;
        }
    }
    if pack.slug.eq_ignore_ascii_case(name) || pack.name.eq_ignore_ascii_case(name) {
        return true;
    }
    let name_slug = slugify(&pack.name);
    pack.slug.eq_ignore_ascii_case(needle)
        || name_slug == needle
        || pack.slug.starts_with(&format!("{needle}-"))
}

fn lookup_query(query: &str) -> String {
    let trimmed = query.trim().trim_start_matches('@');
    if let Some(id) = pack_id_from_url(trimmed) {
        return id.to_string();
    }
    trimmed.to_string()
}

fn split_owner_name(query: &str) -> (Option<&str>, &str) {
    if let Some((owner, name)) = query.split_once('/') {
        let owner = owner.trim().trim_start_matches('@');
        let name = name.trim().trim_start_matches('@');
        if !owner.is_empty() && !name.is_empty() && !owner.contains('/') {
            return (Some(owner), name);
        }
    }
    (None, query)
}

fn pack_id_from_query(query: &str) -> Option<&str> {
    if is_pack_id(query) {
        Some(query)
    } else {
        None
    }
}

fn pack_id_from_url(query: &str) -> Option<&str> {
    let path = if let Some(rest) = query
        .strip_prefix("https://")
        .or_else(|| query.strip_prefix("http://"))
    {
        rest.split_once('/')?.1
    } else if query.starts_with('/') {
        query.trim_start_matches('/')
    } else {
        return None;
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let id = match parts.as_slice() {
        ["browse", id] => *id,
        ["api", "packs", id] => *id,
        ["api", "packs", id, "download"] => *id,
        _ => return None,
    };
    is_pack_id(id).then_some(id)
}

fn is_pack_id(value: &str) -> bool {
    let mut parts = value.split('-');
    let counts = [8usize, 4, 4, 4, 12];
    for expected in counts {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != expected || !part.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '_' | '-' | '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn no_match_message(packs: &[CatalogPack], query: &str, needle: &str) -> String {
    let suggestions: Vec<&CatalogPack> = packs
        .iter()
        .filter(|pack| {
            slugify(&pack.name).contains(needle)
                || pack.slug.contains(needle)
                || pack
                    .name
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
        })
        .take(5)
        .collect();
    if suggestions.is_empty() {
        format!("No published pack named '{query}'.")
    } else {
        let mut message = format!("No published pack named '{query}'. Closest matches:");
        for pack in suggestions {
            message.push_str(&format!("\n  {}  {}", pack.name, pack.slug));
        }
        message
    }
}

fn ambiguous_message(query: &str, packs: &[&CatalogPack]) -> String {
    let mut message = format!("Multiple packs match '{query}'. Use a slug:");
    for pack in packs.iter().take(8) {
        message.push_str(&format!("\n  mouse-me add {}", pack.slug));
    }
    message
}

pub fn pack_spec(pack: &CatalogPack) -> String {
    if pack.owner_username.is_empty() {
        pack.slug.clone()
    } else {
        format!("{}/{}", pack.owner_username, pack.slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_packs() -> Vec<CatalogPack> {
        parse_packs_json(
            r#"{
              "packs": [
                {"id":"0d06f5b8-c265-4fdb-a4c1-72dec023244c","name":"Remus White","slug":"remus-white-0d06f5b8","ownerUsername":"mouseme","version":"0.1.0"},
                {"id":"cd351aa2-6128-4c33-9f3b-590780b818e5","name":"Deepin, classic style","slug":"deepin-classic-style-cd351aa2","ownerUsername":"mouseme","version":"0.1.0"},
                {"id":"eb887814-1885-42e4-a8e2-1512c2d385d7","name":"Modest Light","slug":"modest-light-eb887814","ownerUsername":"mouseme","version":"0.1.0"},
                {"id":"65971e6c-0843-4d06-8206-4f179da4157c","name":"Niko Cursor","slug":"niko-cursor-65971e6c","ownerUsername":"mouseme","version":"0.1.0"}
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn resolves_name_slug_id_and_owner() {
        let packs = sample_packs();
        assert_eq!(
            resolve_pack(&packs, "Modest Light").unwrap().slug,
            "modest-light-eb887814"
        );
        assert_eq!(
            resolve_pack(&packs, "modest-light").unwrap().name,
            "Modest Light"
        );
        assert_eq!(
            resolve_pack(&packs, "eb887814-1885-42e4-a8e2-1512c2d385d7")
                .unwrap()
                .name,
            "Modest Light"
        );
        assert_eq!(
            resolve_pack(&packs, "mouseme/niko-cursor").unwrap().slug,
            "niko-cursor-65971e6c"
        );
        assert_eq!(
            resolve_pack(
                &packs,
                "https://mouse-me-web.vercel.app/browse/cd351aa2-6128-4c33-9f3b-590780b818e5"
            )
            .unwrap()
            .name,
            "Deepin, classic style"
        );
    }

    #[test]
    fn unknown_pack_is_an_error() {
        let packs = sample_packs();
        let error = resolve_pack(&packs, "does-not-exist").unwrap_err();
        assert!(error.contains("No published pack"));
    }

    #[test]
    fn filesystem_sources_are_local_paths() {
        assert!(looks_like_filesystem_source("./cursors.zip"));
        assert!(looks_like_filesystem_source("/tmp/pack.tar.gz"));
        assert!(!looks_like_filesystem_source("modest-light"));
        assert!(!looks_like_filesystem_source("mouseme/modest-light"));
    }

    #[test]
    fn rejects_non_uuid_ids_in_catalog_json() {
        let packs = parse_packs_json(
            r#"{"packs":[{"id":"../etc/passwd","name":"x","slug":"x","ownerUsername":"a"}]}"#,
        )
        .unwrap();
        assert!(packs.is_empty());
    }
}
