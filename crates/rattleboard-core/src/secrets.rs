//! API keys live in the OS credential store (macOS Keychain, Windows Credential
//! Manager, Secret Service on Linux), never on disk next to the settings file.
//!
//! Two exceptions keep them in a file instead (see `use_file`): a debug profile (see the
//! desktop's `profile`), so trying changes out never touches the credential store, and
//! Rattleboard Web, which runs in a container with no credential store at all. That file is
//! only readable by its owner (0600).
//!
//! On Linux the store is the Secret Service (GNOME Keyring, KWallet). Without one, saving a
//! key fails with a clear error; it is never written anywhere in plain text instead.

const SERVICE: &str = "io.github.slastrina.rattleboard";
pub const LOCAL_KEY: &str = "local-api-key";
pub const CLOUD_TOKEN: &str = "cloud-api-token";
pub const SERVER_KEY: &str = "api-server-key";

/// Each saved board's key has an account of its own.
pub fn board_account(id: &str) -> String {
    format!("board-{id}")
}

/// Where secrets go instead of the credential store, once set (see the module comment).
pub static PROFILE_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Keeps secrets in `path` (created 0600) instead of the credential store, for the rest of
/// the process. Only the first call counts.
pub fn use_file(path: std::path::PathBuf) {
    let _ = PROFILE_FILE.set(path);
}

/// True when secrets go to a file rather than the credential store.
pub fn in_file() -> bool {
    PROFILE_FILE.get().is_some()
}

/// One lock for every read-modify-write of the file, so two saves can't lose each other's key.
static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn profile_keys() -> Option<(std::path::PathBuf, serde_json::Map<String, serde_json::Value>)> {
    let path = PROFILE_FILE.get()?.clone();
    let keys = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    Some((path, keys))
}

/// Writes `contents` to `path` readable and writable only by its owner (0600 on Unix), also
/// when the file already existed with wider permissions.
pub fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Written next to it and renamed over it, so a crash never leaves half a file.
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

pub fn get(account: &str) -> String {
    if let Some((_, keys)) = profile_keys() {
        return keys.get(account).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    }
    #[cfg(feature = "keyring")]
    {
        keyring::Entry::new(SERVICE, account)
            .and_then(|e| e.get_password())
            .unwrap_or_default()
    }
    #[cfg(not(feature = "keyring"))]
    {
        let _ = (SERVICE, account);
        String::new()
    }
}

pub fn set(account: &str, value: &str) -> Result<(), String> {
    if PROFILE_FILE.get().is_some() {
        let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (path, mut keys) = profile_keys().expect("file store set");
        if value.is_empty() {
            keys.remove(account);
        } else {
            keys.insert(account.into(), value.into());
        }
        return write_private(&path, &serde_json::Value::Object(keys).to_string()).map_err(|e| e.to_string());
    }
    #[cfg(feature = "keyring")]
    {
        let entry = keyring::Entry::new(SERVICE, account).map_err(store_error)?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(store_error(e)),
            }
        } else {
            entry.set_password(value).map_err(store_error)
        }
    }
    #[cfg(not(feature = "keyring"))]
    {
        let _ = (SERVICE, account, value);
        Err("No secret store is set up".into())
    }
}

/// The store's error, in words that say what to do about it. On Linux a missing Secret
/// Service is the usual cause; nothing falls back to a plain-text file.
#[cfg(feature = "keyring")]
fn store_error(e: keyring::Error) -> String {
    #[cfg(target_os = "linux")]
    if matches!(e, keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_)) {
        return format!(
            "No Secret Service keyring is available ({e}). Install and unlock GNOME Keyring or KWallet \
             (package gnome-keyring or kwalletmanager); Rattleboard never saves keys in plain text."
        );
    }
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rattleboard-private-{}", std::process::id()));
        let path = dir.join("secrets.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "{\"a\":\"b\"}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":\"b\"}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
