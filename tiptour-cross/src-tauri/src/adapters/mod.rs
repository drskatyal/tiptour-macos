// Connected-app adapter registry.
//
// An adapter is a small bundle of functions that lets TipTour control
// a specific third-party app — Spotify, WhatsApp, Notion, Linear, etc.
// Each adapter ships a `Manifest` (declared metadata + voice triggers
// + required capabilities + auth shape) and zero or more handler
// functions that the orchestrator invokes when a voice command
// matches the adapter's triggers.
//
// v1 strategy: adapters are Rust modules compiled into the binary,
// registered in `BUNDLED_ADAPTERS`. The settings UI lists them with
// install / enable toggles, runs the per-adapter auth flow when the
// user enables one, and stores credentials in the OS keychain under
// the adapter's slug.
//
// v2 plan (not in this commit): a TypeScript runtime in a hidden
// Tauri webview that loads user-installed adapter files from
// `~/Library/Application Support/TipTour/adapters/<slug>/` so the
// "AI coder generates a new adapter" flow can write to disk and
// have the file picked up live without rebuilding the binary.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::keychain;

pub mod apple_music;
pub mod browser_cdp;
pub mod calendar_macos;
pub mod file_explorer;
pub mod finder;
pub mod github;
pub mod helpers;
pub mod iwork;
pub mod linear;
pub mod mail_macos;
pub mod messages_macos;
pub mod microsoft_to_do;
pub mod notes_macos;
pub mod notion;
pub mod obsidian;
pub mod office_windows;
pub mod reminders_macos;
pub mod safari;
pub mod slack;
pub mod spotify;
pub mod terminal_macos;
pub mod vscode;
pub mod whatsapp;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterManifest {
    /// Stable slug — keys the keychain entry + on-disk install dir.
    pub slug: String,
    /// Display name shown in the settings UI.
    pub name: String,
    /// One-line description for the settings list row.
    pub description: String,
    /// Category for grouping in the settings UI ("music",
    /// "productivity", "communication", "dev", "files").
    pub category: String,
    /// Voice triggers — phrases that, when matched by Gemini Live's
    /// tool router, should dispatch to this adapter. Lowercase, no
    /// punctuation. ("play spotify", "send a whatsapp", …)
    pub voice_triggers: Vec<String>,
    /// Which OS-level capabilities the adapter needs. Surfaced to
    /// the user before install so they know what they're granting.
    pub capabilities: Vec<AdapterCapability>,
    /// Auth shape — determines what UI the install flow shows.
    pub auth: AdapterAuth,
    /// Platforms the adapter supports. The settings UI hides
    /// adapters whose `supported_platforms` doesn't include the
    /// host OS so users don't see uninstallable rows.
    pub supported_platforms: Vec<String>,
    /// Free-form text shown on the install page — instructions
    /// for getting an API key, OAuth scope notes, etc.
    #[serde(default)]
    pub setup_notes: String,
}

/// Capability gates. The runtime checks these before allowing the
/// adapter's handler to call the corresponding API surface.
/// (Bundled-Rust adapters use these for documentation today; the
/// v2 TS-runtime adapters will use them for actual sandboxing.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterCapability {
    /// Read or write the system clipboard.
    Clipboard,
    /// Make outbound HTTPS requests.
    Network,
    /// Read/write a keychain entry under the adapter's slug.
    Keychain,
    /// Spawn a process (CLI wrappers like git, docker).
    Spawn,
    /// Open a URL or URI-scheme link through the OS default handler.
    OpenUri,
    /// Run an AppleScript snippet via osascript (macOS only).
    Osascript,
    /// Send synthesized keystrokes to the foreground app.
    Keystrokes,
}

/// Auth shape. Drives the install-page UX.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AdapterAuth {
    /// No auth required (URI-scheme adapters, AppleScript-only
    /// adapters running on the user's own machine).
    None,
    /// User pastes an API key / personal access token.
    ApiKey {
        /// Human-readable name of the credential ("Personal access
        /// token", "Integration secret").
        label: String,
        /// Link to the provider's "create a token" page.
        help_url: String,
    },
    /// Three-leg OAuth via a redirect URL. The adapter implements
    /// the redirect handler; the settings UI opens the auth url
    /// in the default browser.
    OAuth2 {
        scopes: Vec<String>,
        help_url: String,
    },
}

/// Snapshot of an adapter for the settings UI. The install status +
/// keychain presence is computed at request time so the UI doesn't
/// have to poll.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterListing {
    #[serde(flatten)]
    pub manifest: AdapterManifest,
    /// True when a credential has been saved for this adapter's
    /// slug (or when the adapter requires no auth).
    pub is_installed: bool,
    /// True if the adapter is in the user's "enabled" set
    /// (persisted in app_settings — defaults off so we don't ship
    /// pre-enabled integrations).
    pub is_enabled: bool,
}

/// Returns every bundled adapter the binary knows about. The
/// settings UI calls `list_adapters` and renders this verbatim.
pub fn bundled_manifests() -> Vec<AdapterManifest> {
    vec![
        // music
        spotify::manifest(),
        apple_music::manifest(),
        // communication
        whatsapp::manifest(),
        mail_macos::manifest(),
        messages_macos::manifest(),
        slack::manifest(),
        // productivity
        calendar_macos::manifest(),
        reminders_macos::manifest(),
        notes_macos::manifest(),
        notion::manifest(),
        linear::manifest(),
        obsidian::manifest(),
        microsoft_to_do::manifest(),
        iwork::pages_manifest(),
        iwork::numbers_manifest(),
        iwork::keynote_manifest(),
        office_windows::word_manifest(),
        office_windows::excel_manifest(),
        office_windows::powerpoint_manifest(),
        office_windows::outlook_desktop_manifest(),
        // dev
        github::manifest(),
        vscode::manifest(),
        terminal_macos::manifest(),
        // files
        finder::manifest(),
        file_explorer::manifest(),
        safari::manifest(),
        browser_cdp::manifest(),
    ]
}

#[tauri::command]
pub fn list_adapters() -> Vec<AdapterListing> {
    let enabled = enabled_slugs();
    bundled_manifests()
        .into_iter()
        .map(|manifest| {
            let is_installed = match &manifest.auth {
                AdapterAuth::None => true,
                _ => keychain::read_provider_key(&manifest.slug)
                    .map(|opt| opt.is_some())
                    .unwrap_or(false),
            };
            let is_enabled = enabled.contains(&manifest.slug);
            AdapterListing {
                manifest,
                is_installed,
                is_enabled,
            }
        })
        .collect()
}

#[tauri::command]
pub fn set_adapter_enabled(slug: String, enabled: bool) -> Result<(), String> {
    let mut current = enabled_slugs();
    if enabled {
        if !current.contains(&slug) {
            current.push(slug);
        }
    } else {
        current.retain(|s| s != &slug);
    }
    persist_enabled_slugs(&current)
}

/// Read the per-user list of enabled adapter slugs. The file sits
/// next to the rest of TipTour's settings.json under the OS data
/// dir; missing = no adapters enabled.
fn enabled_slugs() -> Vec<String> {
    let Some(mut path) = dirs::data_local_dir() else {
        return Vec::new();
    };
    path.push("TipTour");
    path.push("adapters-enabled.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

fn persist_enabled_slugs(slugs: &[String]) -> Result<(), String> {
    let mut path =
        dirs::data_local_dir().ok_or_else(|| "no data_local_dir available".to_string())?;
    path.push("TipTour");
    std::fs::create_dir_all(&path).map_err(|error| format!("create dir: {error}"))?;
    path.push("adapters-enabled.json");
    let json = serde_json::to_string_pretty(slugs)
        .map_err(|error| format!("serialize: {error}"))?;
    std::fs::write(&path, json).map_err(|error| format!("write: {error}"))
}

/// Dispatch entry-point. Gemini Live's tool router and the in-app
/// command palette call this with a slug + a JSON args blob; the
/// adapter decodes the args according to its handler shape.
///
/// We keep dispatch inside one place so the runtime checks (is the
/// adapter enabled? does it have its key?) live in one spot.
#[tauri::command]
pub async fn dispatch_adapter_command(
    app: AppHandle,
    slug: String,
    handler: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let enabled = enabled_slugs();
    if !enabled.contains(&slug) {
        return Err(format!(
            "Adapter '{slug}' is disabled. Enable it in Settings → Connected apps."
        ));
    }
    match slug.as_str() {
        "spotify" => spotify::dispatch(app, &handler, args).await,
        "apple-music" => apple_music::dispatch(app, &handler, args).await,
        "whatsapp" => whatsapp::dispatch(app, &handler, args).await,
        "mail-macos" => mail_macos::dispatch(app, &handler, args).await,
        "messages-macos" => messages_macos::dispatch(app, &handler, args).await,
        "slack" => slack::dispatch(app, &handler, args).await,
        "calendar-macos" => calendar_macos::dispatch(app, &handler, args).await,
        "reminders-macos" => reminders_macos::dispatch(app, &handler, args).await,
        "notes-macos" => notes_macos::dispatch(app, &handler, args).await,
        "notion" => notion::dispatch(app, &handler, args).await,
        "linear" => linear::dispatch(app, &handler, args).await,
        "obsidian" => obsidian::dispatch(app, &handler, args).await,
        "ms-to-do" => microsoft_to_do::dispatch(app, &handler, args).await,
        "pages" => iwork::dispatch_pages(app, &handler, args).await,
        "numbers" => iwork::dispatch_numbers(app, &handler, args).await,
        "keynote" => iwork::dispatch_keynote(app, &handler, args).await,
        "word" => office_windows::dispatch_word(app, &handler, args).await,
        "excel" => office_windows::dispatch_excel(app, &handler, args).await,
        "powerpoint" => office_windows::dispatch_powerpoint(app, &handler, args).await,
        "outlook-desktop" => office_windows::dispatch_outlook_desktop(app, &handler, args).await,
        "github" => github::dispatch(app, &handler, args).await,
        "vscode" => vscode::dispatch(app, &handler, args).await,
        "terminal-macos" => terminal_macos::dispatch(app, &handler, args).await,
        "finder" => finder::dispatch(app, &handler, args).await,
        "file-explorer" => file_explorer::dispatch(app, &handler, args).await,
        "safari" => safari::dispatch(app, &handler, args).await,
        "browser" => browser_cdp::dispatch(app, &handler, args).await,
        _ => Err(format!("Unknown adapter slug: {slug}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifests_cover_v1_ship_list() {
        // If we ever drop spotify or whatsapp from the bundled
        // adapters, the settings UI's "Music" + "Communication"
        // categories go empty. Fail loudly here so a careless edit
        // shows up at PR review.
        let manifests = bundled_manifests();
        let slugs: Vec<&str> = manifests.iter().map(|m| m.slug.as_str()).collect();
        assert!(slugs.contains(&"spotify"));
        assert!(slugs.contains(&"whatsapp"));
    }

    #[test]
    fn each_manifest_has_complete_metadata() {
        // Don't ship an adapter with an empty description or zero
        // voice triggers — the settings UI shows the description
        // and the orchestrator's tool router matches the triggers.
        for manifest in bundled_manifests() {
            assert!(!manifest.name.is_empty(), "manifest {} has empty name", manifest.slug);
            assert!(!manifest.description.is_empty(), "{} empty description", manifest.slug);
            assert!(!manifest.voice_triggers.is_empty(), "{} no voice triggers", manifest.slug);
            assert!(!manifest.category.is_empty(), "{} no category", manifest.slug);
            assert!(
                !manifest.supported_platforms.is_empty(),
                "{} no supported_platforms",
                manifest.slug
            );
        }
    }
}
