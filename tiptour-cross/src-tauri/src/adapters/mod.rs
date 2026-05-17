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
use tauri::{AppHandle, Emitter};

use crate::keychain;

pub mod apple_music;
pub mod brain_dump;
pub mod browser_cdp;
pub mod calendar_macos;
pub mod contacts;
pub mod defaults;
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
pub mod soniox;
pub mod spotify;
pub mod terminal_macos;
pub mod vscode;
pub mod web_search;
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
        brain_dump::manifest(),
        soniox::manifest(),
        web_search::manifest(),
        contacts::manifest(),
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

/// Frontend-facing version of `enabled_adapter_slugs_with_capability_hints`:
/// returns a flat array of {slug, hint} so the TS Gemini client can build
/// its tool description without shipping the 3K-token catalog every session.
#[tauri::command]
pub fn get_enabled_adapter_hints() -> Vec<serde_json::Value> {
    enabled_adapter_slugs_with_capability_hints()
        .into_iter()
        .map(|(slug, hint)| serde_json::json!({ "slug": slug, "hint": hint }))
        .collect()
}

/// Build the lazy-tool subset: just the slugs the user enabled, paired
/// with a one-line handler-shape hint extracted from a fixed lookup.
/// Used by audio_query / GeminiLiveClient to avoid shipping the full
/// 3K-token tool description on every call.
pub fn enabled_adapter_slugs_with_capability_hints() -> Vec<(String, String)> {
    // Hint table: terse handler shape per slug, kept here so we
    // don't have to walk every adapter manifest on every dispatch.
    // Adding a new adapter? Add its hint here too — the dispatcher
    // tolerates extra slugs in the map; missing ones just send the
    // slug name with an empty hint.
    let hints: &[(&str, &str)] = &[
        ("spotify", "spotify.play_track:{query}, .pause, .resume, .next, .previous, .current"),
        ("apple-music", "apple-music.play_track:{query}, .pause, .resume, .next, .previous, .current"),
        ("whatsapp", "whatsapp.send_message:{phone,text}, .open_chat:{phone}"),
        ("mail-macos", "mail-macos.compose/send:{to,subject,body,cc?}"),
        ("messages-macos", "messages-macos.send:{to,text}"),
        ("slack", "slack.post_message:{channel,text}, .post_dm:{user_id,text}"),
        ("calendar-macos", "calendar-macos.create_event:{summary,start_iso,end_iso?,location?,notes?}, .list_today"),
        ("reminders-macos", "reminders-macos.add:{text,due_iso?,notes?}, .list_today"),
        ("notes-macos", "notes-macos.create/append:{title,body}"),
        ("notion", "notion.create_page:{parent_page_id,title,body}, .append_text:{page_id,text}"),
        ("linear", "linear.create_issue:{team_key,title,description?,priority?}, .my_issues"),
        ("obsidian", "obsidian.open_note:{vault,file}, .create_note:{vault,name,content}, .append_to_daily:{vault,text}"),
        ("ms-to-do", "ms-to-do.add:{title,notes?}"),
        ("github", "github.create_issue:{repo,title,body?,labels?}, .my_review_requests"),
        ("vscode", "vscode.open:{path}, .open_in_cursor:{path}, .goto_line:{path,line,column?}"),
        ("terminal-macos", "terminal-macos.run:{command,cwd?}, .open_cwd:{path}"),
        ("finder", "finder.open_path/reveal_path:{path}"),
        ("file-explorer", "file-explorer.open_path/reveal_path:{path}"),
        ("safari", "safari.open_url/new_tab:{url}, .current_url, .current_title"),
        ("browser", "browser.open_url:{url,new_tab?}, .read_page_text:{tab_match?}, .click_text:{text,tab_match?}, .fill_field:{selector,value,tab_match?}, .list_tabs"),
        ("brain-dump", "brain-dump.capture_text:{text,tags?,title?}, .capture_screenshot:{caption?,tags?}, .daily_note_append:{text}, .find:{query}, .open_folder"),
        ("soniox", "soniox.start, .stop, .toggle, .state"),
        ("web-search", "web-search.search:{query,count?}, .news:{query,count?}"),
        ("contacts", "contacts.lookup:{name}, .suggest:{partial}"),
        ("pages", "pages.new_document, .open:{path}"),
        ("numbers", "numbers.new_document, .open:{path}"),
        ("keynote", "keynote.new_document, .open:{path}"),
        ("word", "word.open:{path}, .new_document, .save_as_pdf:{input_path,output_path}"),
        ("excel", "excel.open:{path}, .new_workbook"),
        ("powerpoint", "powerpoint.open:{path}, .new_presentation, .start_slideshow"),
        ("outlook-desktop", "outlook-desktop.compose:{to,subject,body}"),
    ];
    let enabled = enabled_slugs();
    enabled
        .into_iter()
        .map(|slug| {
            let hint = hints
                .iter()
                .find(|(s, _)| *s == slug.as_str())
                .map(|(_, h)| (*h).to_string())
                .unwrap_or_default();
            (slug, hint)
        })
        .collect()
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
    let normalized_slug = normalize_slug(&slug);
    // category indirection check happens BEFORE the enabled gate
    // because the resolved slug is the one that needs to be enabled.
    if let Some(category) = normalized_slug.strip_prefix("default:") {
        let resolved = defaults::resolve_category_to_adapter(category)?;
        return Box::pin(dispatch_adapter_command(app, resolved, handler, args)).await;
    }
    let enabled = enabled_slugs();
    if !enabled.contains(&normalized_slug) {
        let message = format!(
            "Adapter '{normalized_slug}' is disabled. Enable it in Settings → Connected apps."
        );
        // Surface the gate failure to any listening dock so the
        // confirmation chip shows the disabled-adapter remedy.
        let _ = app.emit(
            "adapter_dispatched",
            serde_json::json!({
                "slug": normalized_slug,
                "handler": handler,
                "ok": false,
                "message": message,
            }),
        );
        return Err(message);
    }
    let app_for_event = app.clone();
    let slug_for_event = normalized_slug.clone();
    let handler_for_event = handler.clone();
    let result = match normalized_slug.as_str() {
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
        "brain-dump" => brain_dump::dispatch(app, &handler, args).await,
        "soniox" => soniox::dispatch(app, &handler, args).await,
        "web-search" => web_search::dispatch(app, &handler, args).await,
        "contacts" => contacts::dispatch(app, &handler, args).await,
        _ => Err(format!("Unknown adapter slug: {normalized_slug} (was '{slug}')")),
    };

    // Fire-and-forget event so the dock can show a brief
    // confirmation chip without each adapter wiring its own emit.
    let ok = result.is_ok();
    let message = match &result {
        Ok(_) => format!("{slug_for_event} · {handler_for_event}"),
        Err(error) => error.clone(),
    };
    let _ = app_for_event.emit(
        "adapter_dispatched",
        serde_json::json!({
            "slug": slug_for_event,
            "handler": handler_for_event,
            "ok": ok,
            "message": message,
        }),
    );

    result
}

/// Normalize a slug that Gemini may have decorated with sub-paths
/// like "brain-dump:notes" or "reminders-macos.add". Strip everything
/// after the first `:` (unless the slug is `default:<category>`) or
/// `.` so the match arms below stay simple. Logs the cleanup so we
/// can spot model misbehavior in dev.
fn normalize_slug(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("default:") {
        // For default:<category>, split off any further decorators
        // ("default:tasks.add" -> "default:tasks").
        let category = rest.split([':', '.']).next().unwrap_or("");
        return format!("default:{category}");
    }
    let cleaned = raw.split([':', '.']).next().unwrap_or("").to_string();
    if cleaned != raw {
        eprintln!("[adapters] normalized slug '{raw}' -> '{cleaned}' (Gemini included extra suffix)");
    }
    cleaned
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize_slug;
    #[test]
    fn strips_dot_suffix() {
        assert_eq!(normalize_slug("reminders-macos.add"), "reminders-macos");
    }
    #[test]
    fn strips_colon_suffix() {
        assert_eq!(normalize_slug("brain-dump:notes"), "brain-dump");
    }
    #[test]
    fn preserves_default_category() {
        assert_eq!(normalize_slug("default:tasks"), "default:tasks");
    }
    #[test]
    fn strips_default_category_suffix() {
        assert_eq!(normalize_slug("default:tasks.add"), "default:tasks");
        assert_eq!(normalize_slug("default:tasks:overflow"), "default:tasks");
    }
    #[test]
    fn pass_through_clean_slug() {
        assert_eq!(normalize_slug("spotify"), "spotify");
        assert_eq!(normalize_slug("brain-dump"), "brain-dump");
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
