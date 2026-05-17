// Persisted user preferences for the side-of-screen indicator pills.
//
// Lives at `dirs::data_local_dir()/TipTour/indicators_settings.json`,
// kept deliberately separate from `app_settings.json` so the indicator
// surface can iterate its own schema without churning the central
// settings file. Loaded on each emit so live edits in the settings
// dashboard take effect without a restart.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const SETTINGS_FILE_NAME: &str = "indicators_settings.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Which side of the primary monitor the indicator strip is glued to,
/// or `Disabled` to hide the window entirely.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorPosition {
    RightEdge,
    LeftEdge,
    Disabled,
}

/// How aggressive the hover/idle treatment is on each pill.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorDensity {
    Compact,
    Normal,
    Verbose,
}

/// Auto-dismiss timeout in milliseconds. `0` is the sentinel for
/// "never auto-dismiss" — the user has to click the pill to dismiss.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorAutoDismiss {
    ThreeSeconds,
    FiveSeconds,
    TenSeconds,
    Forever,
}

impl IndicatorAutoDismiss {
    pub fn timeout_milliseconds(&self) -> u32 {
        match self {
            IndicatorAutoDismiss::ThreeSeconds => 3_000,
            IndicatorAutoDismiss::FiveSeconds => 5_000,
            IndicatorAutoDismiss::TenSeconds => 10_000,
            IndicatorAutoDismiss::Forever => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorMaxVisible {
    Four,
    Six,
    Ten,
}

impl IndicatorMaxVisible {
    pub fn pill_count(&self) -> usize {
        match self {
            IndicatorMaxVisible::Four => 4,
            IndicatorMaxVisible::Six => 6,
            IndicatorMaxVisible::Ten => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorSound {
    Silent,
    SoftTick,
}

/// Per-kind enable flags. Matches the `IndicatorKind` enum in
/// `indicators.rs` field-for-field. All true by default so a fresh
/// install shows everything until the user opts out of a noisy kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorTypeToggles {
    #[serde(default = "default_true")]
    pub workflow_step: bool,
    #[serde(default = "default_true")]
    pub flow_done: bool,
    #[serde(default = "default_true")]
    pub voice_command: bool,
    #[serde(default = "default_true")]
    pub app_launched: bool,
    #[serde(default = "default_true")]
    pub screenshot: bool,
    #[serde(default = "default_true")]
    pub error: bool,
}

fn default_true() -> bool {
    true
}

impl Default for IndicatorTypeToggles {
    fn default() -> Self {
        Self {
            workflow_step: true,
            flow_done: true,
            voice_command: true,
            app_launched: true,
            screenshot: true,
            error: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorSettings {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_position")]
    pub position: IndicatorPosition,
    #[serde(default = "default_density")]
    pub density: IndicatorDensity,
    #[serde(default = "default_auto_dismiss")]
    pub auto_dismiss: IndicatorAutoDismiss,
    #[serde(default = "default_max_visible")]
    pub max_visible: IndicatorMaxVisible,
    #[serde(default = "default_sound")]
    pub sound: IndicatorSound,
    #[serde(default)]
    pub types_enabled: IndicatorTypeToggles,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}
fn default_position() -> IndicatorPosition {
    IndicatorPosition::RightEdge
}
fn default_density() -> IndicatorDensity {
    IndicatorDensity::Normal
}
fn default_auto_dismiss() -> IndicatorAutoDismiss {
    IndicatorAutoDismiss::FiveSeconds
}
fn default_max_visible() -> IndicatorMaxVisible {
    IndicatorMaxVisible::Six
}
fn default_sound() -> IndicatorSound {
    IndicatorSound::Silent
}

impl Default for IndicatorSettings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            position: default_position(),
            density: default_density(),
            auto_dismiss: default_auto_dismiss(),
            max_visible: default_max_visible(),
            sound: default_sound(),
            types_enabled: IndicatorTypeToggles::default(),
        }
    }
}

fn settings_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(SETTINGS_FILE_NAME);
    Some(path)
}

pub fn load_indicators_settings_from_disk() -> IndicatorSettings {
    let Some(path) = settings_file_path() else {
        return IndicatorSettings::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return IndicatorSettings::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save_indicators_settings_to_disk(settings: &IndicatorSettings) -> Result<(), String> {
    let path = settings_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_indicators_settings() -> Result<IndicatorSettings, String> {
    Ok(load_indicators_settings_from_disk())
}

#[tauri::command]
pub fn set_indicators_settings(
    settings: IndicatorSettings,
    app: tauri::AppHandle,
) -> Result<(), String> {
    save_indicators_settings_to_disk(&settings)?;
    // Re-snap the window to the new edge / hide on Disabled. Failures
    // here are non-fatal — the settings still landed on disk.
    crate::indicators_window::apply_settings(&app, &settings);
    Ok(())
}
