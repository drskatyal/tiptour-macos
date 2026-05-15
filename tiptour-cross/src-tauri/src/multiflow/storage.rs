// Multiflow on-disk index. Persists `flows.json` next to the recorder's
// demonstration directory under `dirs::data_local_dir()/TipTour/multiflow/`.
//
// Schema: { "version": 1, "entries": [FlowSummary, ...] }. Writes are
// atomic — we serialize to a tmp file in the same directory and then
// rename over the target, so a crash mid-write can't leave the index
// truncated or partially-encoded.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::FlowSummary;

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const MULTIFLOW_SUBDIRECTORY_NAME: &str = "multiflow";
const FLOWS_INDEX_FILE_NAME: &str = "flows.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowsIndex {
    pub version: u32,
    pub entries: Vec<FlowSummary>,
}

impl Default for FlowsIndex {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

fn multiflow_directory() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(MULTIFLOW_SUBDIRECTORY_NAME);
    Some(path)
}

pub fn index_path() -> Option<PathBuf> {
    let mut path = multiflow_directory()?;
    path.push(FLOWS_INDEX_FILE_NAME);
    Some(path)
}

pub fn load_index() -> FlowsIndex {
    let Some(path) = index_path() else {
        return FlowsIndex::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return FlowsIndex::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

/// Atomically replace `flows.json`. Writes to `flows.json.tmp` first and
/// renames, so the index file is either the old contents or the new
/// contents — never a half-written file.
pub fn save_index(index: &FlowsIndex) -> Result<(), String> {
    let target_path = index_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let serialized =
        serde_json::to_string_pretty(index).map_err(|error| error.to_string())?;

    let mut temporary_path = target_path.clone();
    let temporary_file_name = match target_path.file_name() {
        Some(name) => format!("{}.tmp", name.to_string_lossy()),
        None => "flows.json.tmp".to_string(),
    };
    temporary_path.set_file_name(temporary_file_name);

    fs::write(&temporary_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&temporary_path, &target_path).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn upsert_flow(updated_flow: FlowSummary) -> Result<(), String> {
    let mut index = load_index();
    if let Some(existing_entry) = index
        .entries
        .iter_mut()
        .find(|entry| entry.flow_id == updated_flow.flow_id)
    {
        *existing_entry = updated_flow;
    } else {
        index.entries.push(updated_flow);
    }
    save_index(&index)
}

pub fn remove_flow(flow_id: &str) -> Result<(), String> {
    let mut index = load_index();
    index.entries.retain(|entry| entry.flow_id != flow_id);
    save_index(&index)
}
