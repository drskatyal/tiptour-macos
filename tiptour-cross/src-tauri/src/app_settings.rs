// User-facing app settings: Gemini voice/model, push-to-talk chord.
//
// Lives at `dirs::data_local_dir()/TipTour/settings.json` — deliberately
// separate from `mode_settings.json`, `recorder_settings.json`, and
// `vosk_settings.json` for now so the new settings dashboard can iterate
// on schema without touching battle-tested per-feature files. We'll
// consolidate later once the shape stabilizes.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const SETTINGS_FILE_NAME: &str = "settings.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

// Defaults — chosen to match the values the codebase ships with today so
// loading a missing settings file produces zero behavioural change.
const DEFAULT_GEMINI_VOICE: &str = "Kore";
const DEFAULT_GEMINI_MODEL: &str = "gemini-3.1-flash-live-preview";
const DEFAULT_PUSH_TO_TALK_CHORD: &str = "Alt+X";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_voice")]
    pub gemini_voice: String,
    #[serde(default = "default_model")]
    pub gemini_model: String,
    #[serde(default = "default_chord")]
    pub push_to_talk_chord: String,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}
fn default_voice() -> String {
    DEFAULT_GEMINI_VOICE.to_string()
}
fn default_model() -> String {
    DEFAULT_GEMINI_MODEL.to_string()
}
fn default_chord() -> String {
    DEFAULT_PUSH_TO_TALK_CHORD.to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            gemini_voice: default_voice(),
            gemini_model: default_model(),
            push_to_talk_chord: default_chord(),
        }
    }
}

fn settings_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(SETTINGS_FILE_NAME);
    Some(path)
}

pub fn load_app_settings_from_disk() -> AppSettings {
    let Some(path) = settings_file_path() else {
        return AppSettings::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save_app_settings_to_disk(settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // Atomic tmp+rename so a crash mid-write can't truncate the file.
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_app_settings() -> Result<AppSettings, String> {
    Ok(load_app_settings_from_disk())
}

#[tauri::command]
pub fn set_app_settings(settings: AppSettings) -> Result<(), String> {
    save_app_settings_to_disk(&settings)
}

/// Wipe every settings file we know about. Called from the "Reset all
/// settings" button in the About tab. The Gemini API key in the
/// platform keychain is cleared separately by the caller because it
/// lives outside of `data_local_dir`.
#[tauri::command]
pub fn reset_all_settings() -> Result<(), String> {
    let mut root = match dirs::data_local_dir() {
        Some(path) => path,
        None => return Err("no data dir".to_string()),
    };
    root.push(ROOT_DIRECTORY_NAME);
    for file_name in [
        SETTINGS_FILE_NAME,
        "mode_settings.json",
        "recorder_settings.json",
        "vosk_settings.json",
    ] {
        let target = root.join(file_name);
        if target.exists() {
            // Ignore individual failures so a permission error on one
            // file doesn't leave the rest in place.
            let _ = fs::remove_file(target);
        }
    }
    Ok(())
}
