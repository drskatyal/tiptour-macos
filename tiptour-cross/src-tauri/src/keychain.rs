// OS-native secret storage for LLM provider API keys with a
// file-based fallback.
//
// First-line storage is the OS keychain (macOS Keychain, Windows
// Credential Manager, Linux Secret Service) via the `keyring` crate.
// When that fails — most commonly on macOS when the user clicks
// "Don't Allow" on the first-launch keychain prompt, or on Linux
// without a running Secret Service daemon — we fall back to a
// JSON file under the per-user data dir.
//
// The fallback is mode-0600 + dropped on the user's local data
// directory (same place we already store settings.json, personas
// .json, etc) so it's no less secure than the rest of TipTour's
// on-disk state, and it means a real-run app NEVER silently drops
// an API key save.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use keyring::Entry;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.tiptour.cross";
const GEMINI_ACCOUNT: &str = "gemini-api-key";
const FALLBACK_FILE_NAME: &str = "api-keys-fallback.json";
const FALLBACK_DIR_NAME: &str = "TipTour";

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|error| error.to_string())
}

fn account_for_provider(provider_id: &str) -> String {
    if provider_id.eq_ignore_ascii_case("gemini") {
        GEMINI_ACCOUNT.to_string()
    } else {
        format!("{}-api-key", provider_id.to_ascii_lowercase())
    }
}

// ------------- File-based fallback -------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct FallbackFile {
    /// account -> raw secret. account keys match the OS keychain
    /// account naming so reads from either side stay consistent.
    keys: HashMap<String, String>,
}

static FALLBACK_CACHE: Lazy<Mutex<Option<FallbackFile>>> = Lazy::new(|| Mutex::new(None));

fn fallback_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(FALLBACK_DIR_NAME);
    Some(path.join(FALLBACK_FILE_NAME))
}

fn read_fallback() -> FallbackFile {
    if let Some(cached) = FALLBACK_CACHE.lock().unwrap().clone() {
        return cached;
    }
    let Some(path) = fallback_path() else {
        return FallbackFile::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return FallbackFile::default();
    };
    let parsed: FallbackFile = serde_json::from_str(&raw).unwrap_or_default();
    *FALLBACK_CACHE.lock().unwrap() = Some(parsed.clone());
    parsed
}

fn write_fallback(file: &FallbackFile) -> Result<(), String> {
    let Some(path) = fallback_path() else {
        return Err("no data_local_dir available for fallback storage".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write: {e}"))?;
    // Tighten file mode on unix so other users on the machine can't
    // read the fallback keys. No-op on Windows (NTFS ACLs handle it).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    *FALLBACK_CACHE.lock().unwrap() = Some(file.clone());
    Ok(())
}

/// Combined read: keychain first, fallback second. Either source's
/// "present" answer wins; only return None when both are empty.
pub fn read_provider_key(provider_id: &str) -> Result<Option<String>, String> {
    let account = account_for_provider(provider_id);
    if let Ok(entry) = entry(&account) {
        match entry.get_password() {
            Ok(value) => return Ok(Some(value)),
            Err(keyring::Error::NoEntry) => { /* fall through to file */ }
            Err(_other) => { /* fall through to file too */ }
        }
    }
    let fallback = read_fallback();
    Ok(fallback.keys.get(&account).cloned())
}

/// Combined write: try keychain first; if it fails for any reason
/// (NoEntry-create denied, Secret Service down, user clicked Don't
/// Allow on the macOS prompt), write to the file fallback instead.
/// Either way the user's key persists across restarts.
fn write_provider_key(account: &str, key: &str) -> Result<(), String> {
    if let Ok(entry) = entry(account) {
        if entry.set_password(key).is_ok() {
            return Ok(());
        }
    }
    let mut file = read_fallback();
    file.keys.insert(account.to_string(), key.to_string());
    write_fallback(&file)
}

fn delete_provider_key(account: &str) -> Result<(), String> {
    let mut had_any = false;
    if let Ok(entry) = entry(account) {
        if matches!(entry.delete_credential(), Ok(()) | Err(keyring::Error::NoEntry)) {
            had_any = true;
        }
    }
    let mut file = read_fallback();
    if file.keys.remove(account).is_some() {
        write_fallback(&file)?;
        had_any = true;
    }
    if !had_any {
        return Err("no stored key to clear".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn get_api_key() -> Result<Option<String>, String> {
    read_provider_key("gemini")
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    write_provider_key(GEMINI_ACCOUNT, &key)
}

#[tauri::command]
pub fn get_provider_api_key(provider_id: String) -> Result<Option<String>, String> {
    read_provider_key(&provider_id)
}

#[tauri::command]
pub fn set_provider_api_key(provider_id: String, key: String) -> Result<(), String> {
    let account = account_for_provider(&provider_id);
    write_provider_key(&account, &key)
}

#[tauri::command]
pub fn clear_provider_api_key(provider_id: String) -> Result<(), String> {
    let account = account_for_provider(&provider_id);
    delete_provider_key(&account)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The keyring crate uses a per-process in-memory mock on Linux CI
    // (where there's no Secret Service running) so these tests
    // exercise the routing logic without touching real OS credentials.
    // The mock is keyed by (service, account) and persists for the
    // duration of the process — each test below uses a unique
    // provider id to avoid cross-test contamination.

    #[test]
    fn account_for_gemini_uses_legacy_key() {
        // Existing settings UI calls get_api_key (no provider arg)
        // which must continue resolving to the same keychain entry
        // the user already populated under previous builds.
        assert_eq!(account_for_provider("gemini"), GEMINI_ACCOUNT);
        assert_eq!(account_for_provider("Gemini"), GEMINI_ACCOUNT);
    }

    #[test]
    fn account_for_non_gemini_provider_is_namespaced() {
        // Distinct providers get distinct keychain accounts so
        // saving an OpenAI key doesn't overwrite the Gemini key.
        assert_eq!(account_for_provider("openai"), "openai-api-key");
        assert_eq!(account_for_provider("Cerebras"), "cerebras-api-key");
        assert_ne!(
            account_for_provider("openai"),
            account_for_provider("anthropic")
        );
    }
}

