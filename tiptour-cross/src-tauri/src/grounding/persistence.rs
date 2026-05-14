// On-disk cache for UIA-harvested shortcut indexes. Keyed by a stable
// `application_identifier` derived from the foreground app's executable
// path + file version so an app upgrade silently invalidates the cache.

use std::fs;
use std::path::PathBuf;

use super::types::ShortcutIndex;

const CACHE_DIRECTORY_NAME: &str = "TipTour";
const SHORTCUTS_SUBDIRECTORY_NAME: &str = "shortcuts";

fn shortcuts_directory() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(CACHE_DIRECTORY_NAME);
    path.push(SHORTCUTS_SUBDIRECTORY_NAME);
    Some(path)
}

fn cache_file_path_for_application(application_identifier: &str) -> Option<PathBuf> {
    let mut directory = shortcuts_directory()?;
    // Replace path separators so a full exe path can be used as the key
    // without escaping the cache directory.
    let sanitized = application_identifier
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_");
    directory.push(format!("{sanitized}.json"));
    Some(directory)
}

pub fn save_shortcut_index(shortcut_index: &ShortcutIndex) -> Result<(), String> {
    let directory = shortcuts_directory()
        .ok_or_else(|| "No local data directory available".to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;

    let file_path = cache_file_path_for_application(&shortcut_index.application_identifier)
        .ok_or_else(|| "No local data directory available".to_string())?;

    let serialized =
        serde_json::to_vec_pretty(shortcut_index).map_err(|error| error.to_string())?;
    fs::write(&file_path, serialized).map_err(|error| error.to_string())
}

pub fn load_shortcut_index(application_identifier: &str) -> Option<ShortcutIndex> {
    let file_path = cache_file_path_for_application(application_identifier)?;
    let bytes = fs::read(&file_path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// TODO: cache invalidation hook. For now we rely on file_version being part
// of the key — a stale entry simply gets re-indexed on next launch.
