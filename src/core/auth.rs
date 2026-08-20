use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

const SALT_LEN: usize = 16;
const HASH_ITERS: u32 = 16_384;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    #[serde(default)]
    session_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    email: String,
    salt: String,
    hash: String,
}

#[derive(Debug, Clone)]
pub struct AuthStore {
    path: PathBuf,
    data: AuthFile,
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
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    pub fn session_email(&self) -> Option<&str> {
        self.data.session_email.as_deref()
    }

    pub fn create_account(
        &mut self,
        email: &str,
        password: &str,
        confirm: &str,
    ) -> Result<String, String> {
        let email = normalize_email(email)?;
        validate_password(password)?;
        if password != confirm {
            return Err("Passwords do not match.".into());
        }
        if self.find_account(&email).is_some() {
            return Err("An account already exists for that email. Sign in instead.".into());
        }

        let salt = random_bytes(SALT_LEN)?;
        let hash = hash_password(password, &salt);
        self.data.accounts.push(StoredAccount {
            email: email.clone(),
            salt: to_hex(&salt),
            hash: to_hex(&hash),
        });
        self.data.session_email = Some(email.clone());
        self.save()?;
        Ok(email)
    }

    pub fn sign_in(&mut self, email: &str, password: &str) -> Result<String, String> {
        let email = normalize_email(email)?;
        validate_password(password)?;
        let account = self
            .find_account(&email)
            .ok_or_else(|| "No account for that email.".to_string())?
            .clone();

        let salt = from_hex(&account.salt)?;
        let expected = from_hex(&account.hash)?;
        if salt.len() != SALT_LEN || expected.len() != 32 {
            return Err("Could not read the saved account.".into());
        }
        let actual = hash_password(password, &salt);
        if !constant_time_eq(&expected, &actual) {
            return Err("Wrong password.".into());
        }

        self.data.session_email = Some(email.clone());
        self.save()?;
        Ok(email)
    }

    pub fn sign_out(&mut self) -> Result<(), String> {
        self.data.session_email = None;
        self.save()
    }

    fn find_account(&self, email: &str) -> Option<&StoredAccount> {
        self.data
            .accounts
            .iter()
            .find(|account| account.email == email)
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
        Ok(())
    }
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

fn hash_password(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    hasher.update(HASH_ITERS.to_le_bytes());
    let mut acc: [u8; 32] = hasher.finalize().into();
    for _ in 1..HASH_ITERS {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(acc);
        acc = hasher.finalize().into();
    }
    acc
}

fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open("/dev/urandom")
        .map_err(|error| format!("Could not generate a password salt: {}", error))?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("Could not generate a password salt: {}", error))?;
    Ok(bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

fn from_hex(raw: &str) -> Result<Vec<u8>, String> {
    if raw.len() % 2 != 0 || raw.is_empty() {
        return Err("Could not read the saved account.".into());
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("Could not read the saved account.".into()),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut acc = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        acc |= a ^ b;
    }
    acc == 0
}

#[cfg(test)]
mod internals {
    use super::{from_hex, to_hex};

    #[test]
    fn hex_roundtrip() {
        let bytes = [0x00, 0x0f, 0xa0, 0xff];
        assert_eq!(to_hex(&bytes), "000fa0ff");
        assert_eq!(from_hex("000fa0ff").unwrap(), bytes);
    }
}
