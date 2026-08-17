//! API keys stored in the OS keychain (macOS Keychain via the `keyring` crate),
//! so secrets never live in the webview or a plaintext file.

use keyring::Entry;

const SERVICE: &str = "ai.uniwise.toolkit";

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, name).map_err(|e| e.to_string())
}

/// Store (or, when `value` is empty, clear) the key for a provider.
pub fn set_key(name: &str, value: &str) -> Result<(), String> {
    let entry = entry(name)?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn get_key(name: &str) -> Option<String> {
    entry(name).ok()?.get_password().ok()
}

pub fn has_key(name: &str) -> bool {
    get_key(name).map(|v| !v.is_empty()).unwrap_or(false)
}
