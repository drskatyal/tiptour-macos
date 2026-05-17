// Persistence + CRUD for the user's custom Vosk-trigger commands.
//
// Lives at `dirs::data_local_dir()/TipTour/custom_voice_commands.json`.
// Each entry pairs a voice phrase with an action the listener should
// run when it transcribes that phrase. The Vosk listener picks these
// up the next time it (re)builds its command grammar — see the
// "Reload commands" button in the Voice Commands tab.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const CUSTOM_COMMANDS_FILE_NAME: &str = "custom_voice_commands.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CustomVoiceCommandActionKind {
    /// Synthesize a keyboard shortcut like `"Cmd+Shift+P"`.
    KeyboardShortcut,
    /// Run a built-in system command name (e.g. "open-settings").
    /// Today only "open-settings" is plumbed end-to-end — others are
    /// accepted and persisted but won't fire until wired.
    SystemCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomVoiceCommand {
    pub id: String,
    pub name: String,
    pub voice_phrase: String,
    pub action_kind: CustomVoiceCommandActionKind,
    /// Action-kind-dependent payload. For `KeyboardShortcut` this is
    /// the chord string; for `SystemCommand` it's the system command
    /// name. Kept untyped so future kinds don't require a schema bump.
    pub action_payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomVoiceCommandsFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub entries: Vec<CustomVoiceCommand>,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

impl Default for CustomVoiceCommandsFile {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

fn custom_commands_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(CUSTOM_COMMANDS_FILE_NAME);
    Some(path)
}

pub fn load_custom_voice_commands_from_disk() -> CustomVoiceCommandsFile {
    let Some(path) = custom_commands_file_path() else {
        return CustomVoiceCommandsFile::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return CustomVoiceCommandsFile::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save_custom_voice_commands_to_disk(
    custom_commands: &CustomVoiceCommandsFile,
) -> Result<(), String> {
    let path = custom_commands_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized =
        serde_json::to_string_pretty(custom_commands).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_custom_voice_commands() -> Result<Vec<CustomVoiceCommand>, String> {
    Ok(load_custom_voice_commands_from_disk().entries)
}

#[tauri::command]
pub fn upsert_custom_voice_command(command: CustomVoiceCommand) -> Result<(), String> {
    let mut file = load_custom_voice_commands_from_disk();
    if let Some(existing) = file.entries.iter_mut().find(|entry| entry.id == command.id) {
        *existing = command;
    } else {
        file.entries.push(command);
    }
    save_custom_voice_commands_to_disk(&file)
}

#[tauri::command]
pub fn delete_custom_voice_command(id: String) -> Result<(), String> {
    let mut file = load_custom_voice_commands_from_disk();
    file.entries.retain(|entry| entry.id != id);
    save_custom_voice_commands_to_disk(&file)
}
