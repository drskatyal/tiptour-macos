// macOS application scan. Walks the three system Application directories
// and reads each `.app` bundle's Info.plist for the display name + bundle
// identifier. We use the `plist` crate so binary plists (the common case
// for App Store apps) decode without shelling out to `defaults`.

#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};

use super::aliases::generate_aliases;
use super::types::{DiscoveredApp, LaunchTarget};

/// Bundle-id prefixes for Apple framework helpers and other internals we
/// never want to expose as user-launchable. These have visible .app
/// bundles but launching them either does nothing or opens an internal
/// settings panel the user didn't ask for.
const SKIPPED_BUNDLE_ID_PREFIXES: &[&str] = &[
    "com.apple.launchservices",
    "com.apple.AddressBookManager",
    "com.apple.CoreServices",
    "com.apple.AssetCacheLocatorService",
];

pub fn scan() -> Vec<DiscoveredApp> {
    let mut discovered: Vec<DiscoveredApp> = Vec::new();

    let mut search_roots: Vec<PathBuf> = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        search_roots.push(home.join("Applications"));
    }

    for root in search_roots {
        scan_directory_recursively(&root, &mut discovered, 0);
    }

    // Same canonical_id can show up if the same app is symlinked under
    // both ~/Applications and /Applications — keep the first hit.
    discovered.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    discovered.dedup_by(|a, b| a.canonical_id == b.canonical_id);
    discovered
}

fn scan_directory_recursively(
    directory: &Path,
    accumulator: &mut Vec<DiscoveredApp>,
    depth: usize,
) {
    // Bound recursion: utility bundles nest dozens of helper .apps inside
    // their own Resources directory and we don't want to surface those.
    if depth > 2 {
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry_result in entries {
        let Ok(entry) = entry_result else { continue };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) == Some("app") {
            if let Some(app) = parse_app_bundle(&path) {
                accumulator.push(app);
            }
            // Don't recurse into the .app itself.
            continue;
        }
        // Otherwise it's a regular subdirectory like "Utilities/" —
        // recurse one level so we catch user-installed apps grouped
        // under category folders.
        scan_directory_recursively(&path, accumulator, depth + 1);
    }
}

fn parse_app_bundle(app_bundle_path: &Path) -> Option<DiscoveredApp> {
    let info_plist_path = app_bundle_path.join("Contents/Info.plist");
    let plist_value = plist::Value::from_file(&info_plist_path).ok()?;
    let dictionary = plist_value.as_dictionary()?;

    let bundle_identifier = dictionary
        .get("CFBundleIdentifier")
        .and_then(|value| value.as_string())
        .map(|s| s.to_string());

    let display_name = dictionary
        .get("CFBundleDisplayName")
        .and_then(|value| value.as_string())
        .or_else(|| {
            dictionary
                .get("CFBundleName")
                .and_then(|value| value.as_string())
        })
        .map(|s| s.to_string())
        .or_else(|| {
            app_bundle_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|s| s.to_string())
        })?;

    if let Some(bundle_id) = &bundle_identifier {
        let lowercased = bundle_id.to_lowercase();
        for skipped in SKIPPED_BUNDLE_ID_PREFIXES {
            if lowercased.starts_with(&skipped.to_lowercase()) {
                return None;
            }
        }
    }

    let aliases = generate_aliases(&display_name);
    if aliases.is_empty() {
        return None;
    }

    let (canonical_id, launch_target) = match bundle_identifier {
        Some(bundle_id) => (
            bundle_id.clone(),
            LaunchTarget::BundleId { value: bundle_id },
        ),
        None => (
            app_bundle_path.to_string_lossy().to_string(),
            LaunchTarget::ExecutablePath {
                value: app_bundle_path.to_string_lossy().to_string(),
            },
        ),
    };

    Some(DiscoveredApp {
        canonical_id,
        display_name,
        launch_target,
        aliases,
        is_user_enabled: true,
    })
}
