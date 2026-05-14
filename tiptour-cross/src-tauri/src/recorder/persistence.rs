// On-disk layout for demonstrations and the recorder-enabled flag.
//
// dirs::data_local_dir()/TipTour/
//   recorder_settings.json          - { "is_recording_enabled": bool }
//   demonstrations/
//     {id}/
//       meta.json                   - serialized DemonstrationSummary
//       trace.jsonl                 - one TraceEntry per line
//       narration.wav               - optional, only in demonstration mode
//     ...
//   passive_traces/
//     {timestamp_unix_ms}.jsonl     - one TraceEntry per line

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::{Demonstration, DemonstrationSummary, TraceEntry};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const DEMONSTRATIONS_SUBDIRECTORY_NAME: &str = "demonstrations";
const PASSIVE_TRACES_SUBDIRECTORY_NAME: &str = "passive_traces";
const RECORDER_SETTINGS_FILE_NAME: &str = "recorder_settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderSettings {
    pub is_recording_enabled: bool,
}

impl Default for RecorderSettings {
    fn default() -> Self {
        Self {
            is_recording_enabled: false,
        }
    }
}

pub fn tiptour_root_directory() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    Some(path)
}

pub fn demonstrations_directory() -> Option<PathBuf> {
    let mut path = tiptour_root_directory()?;
    path.push(DEMONSTRATIONS_SUBDIRECTORY_NAME);
    Some(path)
}

pub fn passive_traces_directory() -> Option<PathBuf> {
    let mut path = tiptour_root_directory()?;
    path.push(PASSIVE_TRACES_SUBDIRECTORY_NAME);
    Some(path)
}

pub fn demonstration_directory_for_id(demonstration_id: &str) -> Option<PathBuf> {
    let mut path = demonstrations_directory()?;
    path.push(demonstration_id);
    Some(path)
}

pub fn settings_file_path() -> Option<PathBuf> {
    let mut path = tiptour_root_directory()?;
    path.push(RECORDER_SETTINGS_FILE_NAME);
    Some(path)
}

pub fn load_settings() -> RecorderSettings {
    let Some(path) = settings_file_path() else {
        return RecorderSettings::default();
    };
    let Ok(file_contents) = fs::read_to_string(&path) else {
        return RecorderSettings::default();
    };
    serde_json::from_str(&file_contents).unwrap_or_default()
}

pub fn save_settings(settings: &RecorderSettings) -> Result<(), String> {
    let path = settings_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let serialized = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(&path, serialized).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn append_trace_entry(trace_file_path: &PathBuf, entry: &TraceEntry) -> Result<(), String> {
    if let Some(parent) = trace_file_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_file_path)
        .map_err(|error| error.to_string())?;
    let serialized_entry = serde_json::to_string(entry).map_err(|error| error.to_string())?;
    writeln!(file, "{}", serialized_entry).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn write_demonstration_meta(
    demonstration_id: &str,
    summary: &DemonstrationSummary,
) -> Result<(), String> {
    let directory = demonstration_directory_for_id(demonstration_id)
        .ok_or_else(|| "no data dir".to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut meta_path = directory.clone();
    meta_path.push("meta.json");
    let serialized = serde_json::to_string_pretty(summary).map_err(|error| error.to_string())?;
    fs::write(&meta_path, serialized).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn list_demonstration_summaries() -> Result<Vec<DemonstrationSummary>, String> {
    let Some(directory) = demonstrations_directory() else {
        return Ok(Vec::new());
    };
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    let entries = fs::read_dir(&directory).map_err(|error| error.to_string())?;
    for entry_result in entries {
        let entry = entry_result.map_err(|error| error.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        let mut meta_path = entry.path();
        meta_path.push("meta.json");
        if !meta_path.exists() {
            continue;
        }
        let file_contents =
            fs::read_to_string(&meta_path).map_err(|error| error.to_string())?;
        let summary: DemonstrationSummary =
            serde_json::from_str(&file_contents).map_err(|error| error.to_string())?;
        summaries.push(summary);
    }
    Ok(summaries)
}

pub fn load_demonstration(demonstration_id: &str) -> Result<Demonstration, String> {
    let directory = demonstration_directory_for_id(demonstration_id)
        .ok_or_else(|| "no data dir".to_string())?;
    let mut meta_path = directory.clone();
    meta_path.push("meta.json");
    let meta_contents = fs::read_to_string(&meta_path).map_err(|error| error.to_string())?;
    let summary: DemonstrationSummary =
        serde_json::from_str(&meta_contents).map_err(|error| error.to_string())?;

    let mut trace_path = directory.clone();
    trace_path.push("trace.jsonl");
    let trace_entries = read_trace_jsonl(&trace_path)?;

    let mut narration_path = directory.clone();
    narration_path.push("narration.wav");
    let narration_audio_path = if narration_path.exists() {
        Some(narration_path.to_string_lossy().to_string())
    } else {
        None
    };

    Ok(Demonstration {
        id: summary.id,
        title: summary.title,
        narration_audio_path,
        trace: trace_entries,
        created_at_unix_ms: summary.created_at_unix_ms,
    })
}

pub fn read_trace_jsonl(trace_file_path: &PathBuf) -> Result<Vec<TraceEntry>, String> {
    if !trace_file_path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(trace_file_path).map_err(|error| error.to_string())?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for line_result in reader.lines() {
        let line = line_result.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: TraceEntry =
            serde_json::from_str(&line).map_err(|error| error.to_string())?;
        entries.push(entry);
    }
    Ok(entries)
}

pub fn list_passive_trace_files() -> Result<Vec<PathBuf>, String> {
    let Some(directory) = passive_traces_directory() else {
        return Ok(Vec::new());
    };
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    let entries = fs::read_dir(&directory).map_err(|error| error.to_string())?;
    for entry_result in entries {
        let entry = entry_result.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
            paths.push(path);
        }
    }
    Ok(paths)
}
