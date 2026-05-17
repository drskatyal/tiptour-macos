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
    save_index_at(index, &target_path)
}

/// Same atomic write as `save_index` but rooted at an explicit path —
/// the tests use this so they can exercise the tmp+rename behavior
/// inside a tempdir without touching the user's real data directory.
pub fn save_index_at(index: &FlowsIndex, target_path: &PathBuf) -> Result<(), String> {
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
    fs::rename(&temporary_path, target_path).map_err(|error| error.to_string())?;
    Ok(())
}

/// Inverse of `save_index_at` — read whatever's already on disk at the
/// explicit path, falling back to the default empty index if missing or
/// unparseable. Mirrors `load_index`'s tolerant behavior.
pub fn load_index_at(target_path: &PathBuf) -> FlowsIndex {
    let Ok(contents) = fs::read_to_string(target_path) else {
        return FlowsIndex::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn flow_summary_for_test(name: &str) -> FlowSummary {
        FlowSummary {
            flow_id: format!("id-{name}"),
            name: name.to_string(),
            created_at_unix_ms: 0,
            step_count: 1,
            trigger_aliases: Vec::new(),
        }
    }

    #[test]
    fn save_then_load_round_trips_through_disk() {
        let temporary_directory = tempdir().expect("tempdir");
        let index_file_path = temporary_directory.path().join(FLOWS_INDEX_FILE_NAME);
        let initial_index = FlowsIndex {
            version: CURRENT_SCHEMA_VERSION,
            entries: vec![
                flow_summary_for_test("morning routine"),
                flow_summary_for_test("evening shutdown"),
            ],
        };
        save_index_at(&initial_index, &index_file_path).expect("save");
        let reloaded_index = load_index_at(&index_file_path);
        assert_eq!(reloaded_index.entries.len(), 2);
        assert_eq!(reloaded_index.entries[0].name, "morning routine");
        assert_eq!(reloaded_index.entries[1].name, "evening shutdown");
    }

    #[test]
    fn atomic_write_does_not_overwrite_target_until_rename_step() {
        let temporary_directory = tempdir().expect("tempdir");
        let index_file_path = temporary_directory.path().join(FLOWS_INDEX_FILE_NAME);

        // Plant an existing valid index on disk.
        let original_index = FlowsIndex {
            version: CURRENT_SCHEMA_VERSION,
            entries: vec![flow_summary_for_test("original")],
        };
        save_index_at(&original_index, &index_file_path).expect("plant original");

        // Simulate a partial-failure scenario: drop a stale `.tmp`
        // sibling that an aborted earlier write would have left behind.
        // The next save should still succeed and the resulting target
        // should have the new contents (not the stale tmp leftover).
        let stale_temporary_path = temporary_directory.path().join("flows.json.tmp");
        std::fs::write(&stale_temporary_path, b"{ this is not valid json").expect("plant tmp");

        let replacement_index = FlowsIndex {
            version: CURRENT_SCHEMA_VERSION,
            entries: vec![flow_summary_for_test("replacement")],
        };
        save_index_at(&replacement_index, &index_file_path).expect("save replacement");

        let after_save_index = load_index_at(&index_file_path);
        assert_eq!(after_save_index.entries.len(), 1);
        assert_eq!(after_save_index.entries[0].name, "replacement");
    }

    #[test]
    fn load_returns_default_when_target_file_missing() {
        let temporary_directory = tempdir().expect("tempdir");
        let missing_file_path = temporary_directory.path().join("does_not_exist.json");
        let loaded_index = load_index_at(&missing_file_path);
        assert!(loaded_index.entries.is_empty());
    }

    #[test]
    fn load_returns_default_when_target_file_corrupted() {
        let temporary_directory = tempdir().expect("tempdir");
        let corrupted_file_path = temporary_directory.path().join(FLOWS_INDEX_FILE_NAME);
        std::fs::write(&corrupted_file_path, b"not valid json at all")
            .expect("plant corrupted");
        let loaded_index = load_index_at(&corrupted_file_path);
        assert!(loaded_index.entries.is_empty());
    }
}
