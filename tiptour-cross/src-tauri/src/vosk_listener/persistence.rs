// On-disk settings for the always-on Vosk listener. Lives next to the
// recorder settings under `dirs::data_local_dir()/TipTour/`.
//
// File format is versioned from day one so future fields (e.g. custom
// wake-word, alternate model paths) can be added without breaking
// installs that wrote the old shape.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const VOSK_SETTINGS_FILE_NAME: &str = "vosk_settings.json";
const VOSK_MODEL_DIRECTORY_NAME: &str = "vosk-model";

/// Schema version. Bumped when the on-disk shape changes incompatibly.
const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoskSettings {
    /// Forward-compat tag so future code can branch on shape.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub is_enabled: bool,
    /// Custom model directory override; `None` means use the default
    /// path inside the TipTour data dir.
    #[serde(default)]
    pub model_path: Option<PathBuf>,
}

fn default_schema_version() -> u32 {
    CURRENT_SETTINGS_SCHEMA_VERSION
}

impl Default for VoskSettings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SETTINGS_SCHEMA_VERSION,
            is_enabled: false,
            model_path: None,
        }
    }
}

pub fn tiptour_root_directory() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    Some(path)
}

pub fn vosk_settings_file_path() -> Option<PathBuf> {
    let mut path = tiptour_root_directory()?;
    path.push(VOSK_SETTINGS_FILE_NAME);
    Some(path)
}

pub fn default_vosk_model_directory() -> Option<PathBuf> {
    let mut path = tiptour_root_directory()?;
    path.push(VOSK_MODEL_DIRECTORY_NAME);
    Some(path)
}

pub fn load_settings() -> VoskSettings {
    let Some(path) = vosk_settings_file_path() else {
        return VoskSettings::default();
    };
    let Ok(file_contents) = fs::read_to_string(&path) else {
        return VoskSettings::default();
    };
    serde_json::from_str(&file_contents).unwrap_or_default()
}

pub fn save_settings(settings: &VoskSettings) -> Result<(), String> {
    let path = vosk_settings_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // Atomic tmp+rename so a crash mid-write can't corrupt the file the
    // next launch reads.
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

/// True iff the configured (or default) model directory exists and looks
/// populated. Used by `mod.rs` to decide whether to auto-start the
/// listener at boot.
pub fn is_model_present(settings: &VoskSettings) -> bool {
    let candidate_directory = settings
        .model_path
        .clone()
        .or_else(default_vosk_model_directory);
    let Some(directory) = candidate_directory else {
        return false;
    };
    if !directory.exists() || !directory.is_dir() {
        return false;
    }
    // Vosk small en-us model ships with at least an `am/` subdirectory;
    // checking for any subentry is enough to distinguish "downloaded and
    // unzipped" from "empty/just-created".
    fs::read_dir(&directory)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}
