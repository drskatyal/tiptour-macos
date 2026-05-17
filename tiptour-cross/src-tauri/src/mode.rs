// Operating mode: Autopilot (runner clicks/types for the user, default) vs
// Teaching (overlay points; user clicks). Persisted to a small JSON file
// under dirs::data_local_dir so the choice survives launches.

use std::fs;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperatingMode {
    Autopilot,
    Teaching,
}

impl Default for OperatingMode {
    fn default() -> Self {
        OperatingMode::Autopilot
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ModeSettings {
    operating_mode: OperatingMode,
}

static CURRENT_MODE: Mutex<Option<OperatingMode>> = Mutex::new(None);

fn settings_path() -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    Some(base.join("TipTour").join("mode_settings.json"))
}

fn load_from_disk() -> OperatingMode {
    let path = match settings_path() {
        Some(path) => path,
        None => return OperatingMode::default(),
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return OperatingMode::default(),
    };
    serde_json::from_str::<ModeSettings>(&text)
        .map(|settings| settings.operating_mode)
        .unwrap_or_default()
}

fn save_to_disk(mode: OperatingMode) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let settings = ModeSettings {
        operating_mode: mode,
    };
    let serialized = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    fs::write(&path, serialized).map_err(|error| error.to_string())
}

pub fn current_operating_mode() -> OperatingMode {
    let mut guard = CURRENT_MODE.lock();
    if let Some(cached) = *guard {
        return cached;
    }
    let loaded = load_from_disk();
    *guard = Some(loaded);
    loaded
}

#[tauri::command]
pub fn get_operating_mode() -> OperatingMode {
    current_operating_mode()
}

#[tauri::command]
pub fn set_operating_mode(mode: OperatingMode) -> Result<(), String> {
    save_to_disk(mode)?;
    *CURRENT_MODE.lock() = Some(mode);
    Ok(())
}
