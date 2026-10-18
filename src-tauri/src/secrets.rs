//! API keys live in the OS credential store (macOS Keychain, Windows Credential
//! Manager, Secret Service on Linux), never on disk next to the settings file.

const SERVICE: &str = "io.github.slastrina.rattleboard";
pub const LOCAL_KEY: &str = "local-api-key";
pub const CLOUD_TOKEN: &str = "cloud-api-token";
pub const SERVER_KEY: &str = "api-server-key";

/// Each saved board's key has an account of its own.
pub fn board_account(id: &str) -> String {
    format!("board-{id}")
}

/// A profile (debug builds, see `crate::profile`) keeps its keys in a file in its own config
/// folder instead, so trying changes out never touches the credential store.
pub static PROFILE_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

fn profile_keys() -> Option<(std::path::PathBuf, serde_json::Map<String, serde_json::Value>)> {
    let path = PROFILE_FILE.get()?.clone();
    let keys = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    Some((path, keys))
}

pub fn get(account: &str) -> String {
    if let Some((_, keys)) = profile_keys() {
        return keys.get(account).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    }
    keyring::Entry::new(SERVICE, account)
        .and_then(|e| e.get_password())
        .unwrap_or_default()
}

pub fn set(account: &str, value: &str) -> Result<(), String> {
    if let Some((path, mut keys)) = profile_keys() {
        keys.insert(account.into(), value.into());
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
        return std::fs::write(&path, serde_json::Value::Object(keys).to_string()).map_err(|e| e.to_string());
    }
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
    if value.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        entry.set_password(value).map_err(|e| e.to_string())
    }
}
