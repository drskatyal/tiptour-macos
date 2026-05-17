// Default-adapter picker per category.
//
// Multiple adapters can satisfy the same intent: "play this song" can
// route to Spotify or Apple Music; "add to my list" can route to
// Reminders or Microsoft To Do; "send email" can hit Mail.app or
// Outlook. We let the user pick once during onboarding (or in
// Settings → Connected apps) and stick with the choice.
//
// Storage: `defaults.json` next to the rest of TipTour's per-user
// state, keyed by intent slug:
//
//   {
//     "tasks":   "reminders-macos",
//     "music":   "spotify",
//     "email":   "mail-macos",
//     "calendar":"calendar-macos",
//     "notes":   "notes-macos",
//     "messages":"messages-macos"
//   }
//
// The frontend reads these via `get_default_adapters` and writes them
// via `set_default_adapter`. Gemini's `control_app` tool can call
// `resolve_default_adapter` to find which slug to dispatch to when the
// user says "add eggs to my shopping list" without naming the app.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adapters::bundled_manifests;

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const DEFAULTS_FILE_NAME: &str = "default-adapters.json";

/// Category slug → adapter slug. Snake_case category names so the
/// Gemini system-prompt mapping stays predictable.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DefaultAdaptersFile {
    pub map: HashMap<String, String>,
}

fn defaults_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    Some(path.join(DEFAULTS_FILE_NAME))
}

fn load() -> DefaultAdaptersFile {
    let Some(path) = defaults_path() else {
        return DefaultAdaptersFile::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return DefaultAdaptersFile::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(file: &DefaultAdaptersFile) -> Result<(), String> {
    let path = defaults_path().ok_or_else(|| "no data_local_dir".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write: {e}"))
}

/// Canonical category list. Each entry is an "intent" Gemini might
/// resolve to: the user says "add milk to shopping list" → resolve
/// the `tasks` category → dispatch to the user's chosen adapter.
///
/// The candidates list is the static set of adapter slugs that could
/// satisfy this intent on each platform. Filtered at request time by
/// host OS + which adapters are actually installed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultCategory {
    pub category: String,
    pub display_label: String,
    pub description: String,
    pub candidate_slugs: Vec<String>,
    pub current_default: Option<String>,
}

pub fn category_catalog() -> Vec<DefaultCategory> {
    let current = load();
    let get = |k: &str| current.map.get(k).cloned();
    vec![
        DefaultCategory {
            category: "tasks".into(),
            display_label: "Tasks & shopping lists".into(),
            description: "Where 'remind me to…', 'add to shopping list', 'add to my todo' end up.".into(),
            candidate_slugs: vec![
                "reminders-macos".into(),
                "ms-to-do".into(),
                "notion".into(),
                "linear".into(),
            ],
            current_default: get("tasks"),
        },
        DefaultCategory {
            category: "music".into(),
            display_label: "Music".into(),
            description: "Where 'play X', 'pause', 'next song' route.".into(),
            candidate_slugs: vec!["spotify".into(), "apple-music".into()],
            current_default: get("music"),
        },
        DefaultCategory {
            category: "email".into(),
            display_label: "Email".into(),
            description: "Where 'send email to X', 'draft email' end up.".into(),
            candidate_slugs: vec!["mail-macos".into(), "outlook-desktop".into()],
            current_default: get("email"),
        },
        DefaultCategory {
            category: "calendar".into(),
            display_label: "Calendar".into(),
            description: "Where 'schedule a meeting', 'what's next today' end up.".into(),
            candidate_slugs: vec!["calendar-macos".into()],
            current_default: get("calendar"),
        },
        DefaultCategory {
            category: "notes".into(),
            display_label: "Notes & quick capture".into(),
            description: "Where 'save a note', 'jot this down' route.".into(),
            candidate_slugs: vec![
                "notes-macos".into(),
                "obsidian".into(),
                "notion".into(),
            ],
            current_default: get("notes"),
        },
        DefaultCategory {
            category: "messages".into(),
            display_label: "Messaging".into(),
            description: "Where 'send X a message' routes when no specific app is named.".into(),
            candidate_slugs: vec![
                "messages-macos".into(),
                "whatsapp".into(),
                "slack".into(),
            ],
            current_default: get("messages"),
        },
    ]
}

/// Filter category candidates to the ones actually compatible with
/// this host OS and present in the bundled-adapter list. The settings
/// UI displays only these.
#[tauri::command]
pub fn list_default_categories(host_platform: String) -> Vec<DefaultCategoryResolved> {
    let installed_slugs: std::collections::HashSet<String> = bundled_manifests()
        .into_iter()
        .filter(|m| m.supported_platforms.iter().any(|p| p == &host_platform))
        .map(|m| m.slug)
        .collect();
    category_catalog()
        .into_iter()
        .map(|category| {
            let viable: Vec<String> = category
                .candidate_slugs
                .iter()
                .filter(|s| installed_slugs.contains(*s))
                .cloned()
                .collect();
            DefaultCategoryResolved {
                category: category.category,
                display_label: category.display_label,
                description: category.description,
                viable_candidate_slugs: viable,
                current_default: category.current_default,
            }
        })
        .filter(|c| !c.viable_candidate_slugs.is_empty())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultCategoryResolved {
    pub category: String,
    pub display_label: String,
    pub description: String,
    pub viable_candidate_slugs: Vec<String>,
    pub current_default: Option<String>,
}

#[tauri::command]
pub fn set_default_adapter(category: String, slug: String) -> Result<(), String> {
    let mut file = load();
    file.map.insert(category, slug);
    save(&file)
}

#[tauri::command]
pub fn get_default_adapter(category: String) -> Result<Option<String>, String> {
    Ok(load().map.get(&category).cloned())
}

/// Called by the orchestrator when Gemini emits a `control_app` with
/// `slug == "default:<category>"` instead of a concrete adapter slug.
/// Returns the resolved adapter slug or an error suggesting the user
/// pick one in Settings.
pub fn resolve_category_to_adapter(category: &str) -> Result<String, String> {
    load()
        .map
        .get(category)
        .cloned()
        .ok_or_else(|| {
            format!(
                "No default adapter set for category '{category}'. Pick one in Settings → Connected apps → Defaults."
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_catalog_covers_intents_gemini_will_emit() {
        // Voice phrases like "add to shopping list" need to resolve to
        // the tasks category. If we ever drop a category, the
        // orchestrator's category-routing path breaks silently.
        let cats: Vec<String> = category_catalog()
            .into_iter()
            .map(|c| c.category)
            .collect();
        for expected in ["tasks", "music", "email", "calendar", "notes", "messages"] {
            assert!(cats.contains(&expected.to_string()), "missing {expected}");
        }
    }
}
