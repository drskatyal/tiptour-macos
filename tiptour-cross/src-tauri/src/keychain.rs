// OS-native secret storage for the Gemini API key.
// macOS Keychain on Mac, Credential Manager on Windows, via the `keyring` crate.

use keyring::Entry;

const SERVICE: &str = "com.tiptour.cross";
const ACCOUNT: &str = "gemini-api-key";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_api_key() -> Result<Option<String>, String> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    let entry = entry()?;
    entry.set_password(&key).map_err(|error| error.to_string())
}
