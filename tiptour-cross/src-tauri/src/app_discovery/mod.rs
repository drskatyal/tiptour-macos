// Installed-application discovery + per-app voice-launch grammar source.
//
// The local Vosk grammar builder pulls the user-enabled discovered apps
// from this module and injects "open <alias>" / "launch <alias>" phrases
// so the wake-word flow can launch apps entirely on-device — no Gemini
// round-trip needed. Results are cached to disk so subsequent launches
// don't re-scan the filesystem on the critical path.

pub mod aliases;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod types;
#[cfg(target_os = "windows")]
mod windows;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

pub use types::DiscoveredApp;

// 24h freshness window. Beyond this the background scan re-runs at boot.
const CACHE_MAX_AGE_SECONDS: u64 = 60 * 60 * 24;

// File schema we persist on disk. Versioned so a future shape change
// (e.g. adding icon paths) can ignore old caches instead of crashing.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedDiscoveryCache {
    schema_version: u32,
    saved_at_unix_seconds: u64,
    apps: Vec<DiscoveredApp>,
}

const CURRENT_CACHE_SCHEMA_VERSION: u32 = 1;

static IN_MEMORY_CACHE: Lazy<Mutex<Vec<DiscoveredApp>>> = Lazy::new(|| Mutex::new(Vec::new()));

fn cache_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push("TipTour");
    let _ = fs::create_dir_all(&path);
    path.push("discovered_apps.json");
    Some(path)
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn load_cache_from_disk() -> Option<PersistedDiscoveryCache> {
    let path = cache_file_path()?;
    let bytes = fs::read(&path).ok()?;
    let cache: PersistedDiscoveryCache = serde_json::from_slice(&bytes).ok()?;
    if cache.schema_version != CURRENT_CACHE_SCHEMA_VERSION {
        return None;
    }
    Some(cache)
}

fn save_cache_to_disk(apps: &[DiscoveredApp]) -> Result<(), String> {
    let path = cache_file_path().ok_or_else(|| "no data dir for cache".to_string())?;
    let snapshot = PersistedDiscoveryCache {
        schema_version: CURRENT_CACHE_SCHEMA_VERSION,
        saved_at_unix_seconds: now_unix_seconds(),
        apps: apps.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())
}

fn cache_is_fresh(cache: &PersistedDiscoveryCache) -> bool {
    let age_seconds = now_unix_seconds().saturating_sub(cache.saved_at_unix_seconds);
    age_seconds < CACHE_MAX_AGE_SECONDS
}

/// Synchronously scan the local filesystem for installed apps. Fast
/// (~50ms macOS / ~200ms Windows in practice) and does no network I/O.
pub fn scan_installed_applications() -> Result<Vec<DiscoveredApp>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos::scan())
    }
    #[cfg(target_os = "windows")]
    {
        Ok(windows::scan())
    }
    #[cfg(target_os = "linux")]
    {
        Ok(linux::scan())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(Vec::new())
    }
}

/// Returns the current cached discovered-app list. If the in-memory
/// cache is empty, falls back to the on-disk snapshot; if that's also
/// empty, returns an empty vec — callers should fall through to
/// `rescan_installed_apps` for a fresh scan.
fn current_cached_apps() -> Vec<DiscoveredApp> {
    let in_memory = IN_MEMORY_CACHE.lock();
    if !in_memory.is_empty() {
        return in_memory.clone();
    }
    drop(in_memory);
    if let Some(persisted) = load_cache_from_disk() {
        let mut in_memory_writer = IN_MEMORY_CACHE.lock();
        *in_memory_writer = persisted.apps.clone();
        return persisted.apps;
    }
    Vec::new()
}

/// Replace the in-memory cache and persist to disk. Merges the new scan
/// against any persisted per-app enable flags so user toggles aren't
/// wiped on every rescan.
fn replace_cache_preserving_user_overrides(freshly_scanned: Vec<DiscoveredApp>) -> Vec<DiscoveredApp> {
    let previous_enable_flags: std::collections::HashMap<String, bool> = current_cached_apps()
        .into_iter()
        .map(|app| (app.canonical_id.clone(), app.is_user_enabled))
        .collect();

    let merged: Vec<DiscoveredApp> = freshly_scanned
        .into_iter()
        .map(|mut app| {
            if let Some(previous) = previous_enable_flags.get(&app.canonical_id) {
                app.is_user_enabled = *previous;
            }
            app
        })
        .collect();

    {
        let mut writer = IN_MEMORY_CACHE.lock();
        *writer = merged.clone();
    }
    if let Err(error) = save_cache_to_disk(&merged) {
        eprintln!("[app_discovery] failed to persist cache: {error}");
    }
    merged
}

/// Returns only the discovered apps the user has not disabled. Used by
/// the Vosk grammar builder so disabled apps don't leak phrases into the
/// recognizer.
pub fn enabled_apps_for_grammar() -> Vec<DiscoveredApp> {
    current_cached_apps()
        .into_iter()
        .filter(|app| app.is_user_enabled)
        .collect()
}

/// Find a discovered (and enabled) app whose alias the local recognizer
/// just matched. Returns the full record so the dispatcher can pull the
/// launch identifier without rescanning.
pub fn find_enabled_app_by_alias(spoken_alias_lowercased: &str) -> Option<DiscoveredApp> {
    let normalized = spoken_alias_lowercased.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    enabled_apps_for_grammar()
        .into_iter()
        .find(|app| app.aliases.iter().any(|alias| alias.to_ascii_lowercase() == normalized))
}

// -- Tauri command surface ---------------------------------------------

#[tauri::command]
pub fn list_discovered_apps() -> Result<Vec<DiscoveredApp>, String> {
    let cached = current_cached_apps();
    if !cached.is_empty() {
        return Ok(cached);
    }
    // First-ever call before any background scan finished: do it
    // synchronously rather than returning a misleading empty list.
    let freshly_scanned = scan_installed_applications()?;
    Ok(replace_cache_preserving_user_overrides(freshly_scanned))
}

#[tauri::command]
pub fn rescan_installed_apps() -> Result<Vec<DiscoveredApp>, String> {
    let freshly_scanned = scan_installed_applications()?;
    Ok(replace_cache_preserving_user_overrides(freshly_scanned))
}

#[tauri::command]
pub fn set_app_command_enabled(canonical_id: String, enabled: bool) -> Result<(), String> {
    let mut writer = IN_MEMORY_CACHE.lock();
    let mut found_any = false;
    for app in writer.iter_mut() {
        if app.canonical_id == canonical_id {
            app.is_user_enabled = enabled;
            found_any = true;
        }
    }
    if !found_any {
        return Err(format!("no discovered app with id {canonical_id}"));
    }
    let snapshot = writer.clone();
    drop(writer);
    save_cache_to_disk(&snapshot)
}

/// Background discovery kickoff. Called from `main.rs` setup. Loads any
/// existing on-disk cache eagerly so the first Vosk grammar build sees
/// data immediately, then spawns a background task that rescans if the
/// cache is stale or missing.
pub fn kickoff_background_scan(_app: &AppHandle) {
    // Warm the in-memory cache from disk so the very first
    // `enabled_apps_for_grammar()` call after launch returns the previous
    // session's discovered list — no scan latency on the critical path.
    if let Some(persisted) = load_cache_from_disk() {
        let mut writer = IN_MEMORY_CACHE.lock();
        *writer = persisted.apps.clone();
    }

    let should_rescan = match load_cache_from_disk() {
        Some(cache) => !cache_is_fresh(&cache),
        None => true,
    };
    if !should_rescan {
        return;
    }

    tauri::async_runtime::spawn(async move {
        // The scan itself is sync filesystem I/O; running it inside a
        // Tokio task is fine because we're not holding any awaits across
        // a long blocking call here.
        match scan_installed_applications() {
            Ok(freshly_scanned) => {
                let _ = replace_cache_preserving_user_overrides(freshly_scanned);
            }
            Err(error) => {
                eprintln!("[app_discovery] background scan failed: {error}");
            }
        }
    });
}
