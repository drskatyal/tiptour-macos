// OS-native secret storage for LLM provider API keys.
// macOS Keychain on Mac, Credential Manager on Windows, via the `keyring` crate.
//
// Each provider gets its own keychain entry keyed by the lowercase
// provider id ("gemini", "grok", "groq", "cerebras", "together",
// "fireworks", "anthropic", "openai"). The legacy single-key API
// (`get_api_key` / `set_api_key`) now aliases the gemini entry so
// existing settings UI keeps working without migration.

use keyring::Entry;

const SERVICE: &str = "com.tiptour.cross";
const GEMINI_ACCOUNT: &str = "gemini-api-key";

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

/// Internal reader used by `text_rewrite`. Returns `None` if no entry
/// exists for the provider, propagates any other keyring failure.
pub fn read_provider_key(provider_id: &str) -> Result<Option<String>, String> {
    let account = account_for_provider(provider_id);
    let entry = entry(&account)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub fn get_api_key() -> Result<Option<String>, String> {
    read_provider_key("gemini")
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    let entry = entry(GEMINI_ACCOUNT)?;
    entry.set_password(&key).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_provider_api_key(provider_id: String) -> Result<Option<String>, String> {
    read_provider_key(&provider_id)
}

#[tauri::command]
pub fn set_provider_api_key(provider_id: String, key: String) -> Result<(), String> {
    let account = account_for_provider(&provider_id);
    let entry = entry(&account)?;
    entry.set_password(&key).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn clear_provider_api_key(provider_id: String) -> Result<(), String> {
    let account = account_for_provider(&provider_id);
    let entry = entry(&account)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
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

