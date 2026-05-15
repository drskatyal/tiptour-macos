// First-run onboarding flag persistence. The wizard runs only when no
// keychain key AND no on-disk completion flag exist; once the user
// finishes (or explicitly re-triggers it from the About tab) we drop a
// `tiptour_first_run_done.json` next to the rest of the app's data so
// subsequent launches go straight to the normal panel.
//
// File path: `dirs::data_local_dir()/TipTour/tiptour_first_run_done.json`

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const ONBOARDING_FLAG_FILE_NAME: &str = "tiptour_first_run_done.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardingFlagFile {
    schema_version: u32,
    completed_at_unix_seconds: i64,
}

fn flag_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(ONBOARDING_FLAG_FILE_NAME);
    Some(path)
}

/// True when the onboarding wizard has not yet been completed for this
/// installation. Defaults to true when we can't read the data dir at all
/// — better to show the wizard than to silently skip it.
#[tauri::command]
pub fn is_first_run() -> bool {
    match flag_file_path() {
        Some(path) => !path.exists(),
        None => true,
    }
}

#[tauri::command]
pub fn mark_first_run_complete() -> Result<(), String> {
    let target_path = flag_file_path().ok_or_else(|| "no data_local_dir on this OS".to_string())?;
    if let Some(parent_directory) = target_path.parent() {
        fs::create_dir_all(parent_directory).map_err(|error| error.to_string())?;
    }
    let payload = OnboardingFlagFile {
        schema_version: CURRENT_SCHEMA_VERSION,
        completed_at_unix_seconds: chrono::Utc::now().timestamp(),
    };
    let serialized = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    fs::write(&target_path, serialized).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn reset_first_run() -> Result<(), String> {
    let Some(target_path) = flag_file_path() else {
        return Ok(());
    };
    if !target_path.exists() {
        return Ok(());
    }
    fs::remove_file(&target_path).map_err(|error| error.to_string())
}
