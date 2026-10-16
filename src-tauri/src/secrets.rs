//! API keys live in the OS credential store (macOS Keychain, Windows Credential
//! Manager, Secret Service on Linux), never on disk next to the settings file.

const SERVICE: &str = "io.github.slastrina.rattleboard";
pub const LOCAL_KEY: &str = "local-api-key";
pub const CLOUD_TOKEN: &str = "cloud-api-token";

pub fn get(account: &str) -> String {
    keyring::Entry::new(SERVICE, account)
        .and_then(|e| e.get_password())
        .unwrap_or_default()
}

pub fn set(account: &str, value: &str) -> Result<(), String> {
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
