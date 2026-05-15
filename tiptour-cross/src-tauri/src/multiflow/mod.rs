// Multiflow — saved cross-app demonstrations the user can recall by
// voice. The recorder writes the underlying demonstration directory;
// multiflow indexes those demonstrations under user-friendly names with
// optional alias phrases, fuzzy-matches voice queries against them, and
// replays the saved input trace through the cross-platform input layer.
//
// Data flow:
//   user voice -> Gemini transcript -> `run_saved_flow` tool call ->
//   `run_flow_by_name` Tauri command -> matcher::best_match_for_query ->
//   replayer::replay_flow -> cross_platform_input::* -> OS

pub mod matcher;
pub mod replayer;
pub mod storage;
pub mod types;

use chrono::Utc;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::recorder;
use crate::recorder::types::Demonstration;
use storage::{load_index, remove_flow as storage_remove_flow, upsert_flow};
use types::{FlowSummary, ReplayProgress};

#[tauri::command]
pub fn start_recording_flow(name: String) -> Result<String, String> {
    // Reuse the recorder's existing demonstration entry point. The
    // demonstration title doubles as the human-friendly flow name in the
    // index; we re-stamp it as the flow's `name` field at stop time so
    // any later rename through the panel UI just updates the index
    // without rewriting the demonstration on disk.
    recorder::start_demonstration(name)
}

#[tauri::command]
pub fn stop_recording_flow() -> Result<FlowSummary, String> {
    let stopped_demonstration: Demonstration = recorder::stop_demonstration()?;
    let summary = FlowSummary {
        flow_id: stopped_demonstration.id.clone(),
        name: stopped_demonstration.title.clone(),
        created_at_unix_ms: stopped_demonstration.created_at_unix_ms,
        step_count: stopped_demonstration.trace.len(),
        trigger_aliases: Vec::new(),
    };
    upsert_flow(summary.clone())?;
    Ok(summary)
}

#[tauri::command]
pub fn list_flows() -> Result<Vec<FlowSummary>, String> {
    Ok(load_index().entries)
}

#[tauri::command]
pub fn delete_flow(name: String) -> Result<(), String> {
    let index = load_index();
    // Match by exact flow_id first (so the UI can delete by id when it
    // already knows one); fall back to a case-insensitive name match.
    let entry_to_remove = index
        .entries
        .iter()
        .find(|entry| entry.flow_id == name)
        .or_else(|| {
            index
                .entries
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(&name))
        })
        .cloned();
    let entry_to_remove = entry_to_remove
        .ok_or_else(|| format!("no flow matched '{name}'"))?;

    storage_remove_flow(&entry_to_remove.flow_id)?;

    // Trash the on-disk demonstration directory. We don't fail the
    // delete if the directory is already gone — the user's mental
    // model is "the flow is gone", which the index removal accomplishes.
    if let Some(demonstration_directory) =
        crate::recorder::persistence::demonstration_directory_for_id(&entry_to_remove.flow_id)
    {
        let _ = std::fs::remove_dir_all(&demonstration_directory);
    }
    Ok(())
}

#[tauri::command]
pub async fn find_flow_by_voice_query(query: String) -> Result<Option<FlowSummary>, String> {
    let entries = load_index().entries;
    Ok(matcher::best_match_for_query(&query, &entries)
        .map(|(matched_entry, _score)| matched_entry.clone()))
}

#[tauri::command]
pub async fn run_flow_by_name(name: String, app: AppHandle) -> Result<String, String> {
    let entries = load_index().entries;
    let matched_entry = matcher::best_match_for_query(&name, &entries)
        .map(|(entry, _score)| entry.clone())
        .ok_or_else(|| format!("no saved flow matched '{name}'"))?;

    let replay_id = uuid::Uuid::new_v4().to_string();
    // Installing the new replay token implicitly tells any in-flight
    // replay to bail on its next step check.
    replayer::set_active_replay_token(&replay_id);

    let (progress_sender, mut progress_receiver) = mpsc::channel::<ReplayProgress>(64);
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(progress) = progress_receiver.recv().await {
            let _ = app_clone.emit("multiflow_progress", progress);
        }
    });

    let flow_id_for_task = matched_entry.flow_id.clone();
    let replay_id_for_return = replay_id.clone();
    tauri::async_runtime::spawn(async move {
        let _ = replayer::replay_flow(flow_id_for_task, replay_id, progress_sender).await;
    });

    Ok(replay_id_for_return)
}

/// Add or replace trigger aliases on an existing flow. The panel UI
/// calls this when the user edits the alias mini-input.
#[tauri::command]
pub fn set_flow_trigger_aliases(
    flow_id: String,
    trigger_aliases: Vec<String>,
) -> Result<(), String> {
    let mut index = load_index();
    let entry = index
        .entries
        .iter_mut()
        .find(|entry| entry.flow_id == flow_id)
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    entry.trigger_aliases = trigger_aliases
        .into_iter()
        .map(|alias| alias.trim().to_string())
        .filter(|alias| !alias.is_empty())
        .collect();
    storage::save_index(&index)
}

// Silence dead-code warning on the recorder-side timestamp helper used
// during testing of the multiflow index timing semantics.
#[allow(dead_code)]
fn current_unix_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Externally interrupt any currently-running flow replay by stamping a
/// sentinel token onto the global slot. The replayer's per-step token
/// check fails and the loop exits cleanly with a `Paused` progress
/// event. The Gemini Live session, if any, is untouched.
#[tauri::command]
pub fn pause_active_replay() -> Result<(), String> {
    replayer::set_active_replay_token("__paused_by_user__");
    Ok(())
}

/// Convert an existing on-disk demonstration into a multiflow entry
/// without recording anything new. Used by the Recordings tab's
/// "Convert to flow" button so the user doesn't have to re-record a
/// flow they already captured.
#[tauri::command]
pub fn adopt_demonstration_as_flow(
    demonstration_id: String,
    name: String,
) -> Result<FlowSummary, String> {
    let demonstration =
        crate::recorder::persistence::load_demonstration(&demonstration_id)?;
    let trimmed_name = name.trim();
    let final_name = if trimmed_name.is_empty() {
        demonstration.title.clone()
    } else {
        trimmed_name.to_string()
    };

    let summary = FlowSummary {
        flow_id: demonstration.id,
        name: final_name,
        created_at_unix_ms: demonstration.created_at_unix_ms,
        step_count: demonstration.trace.len(),
        trigger_aliases: Vec::new(),
    };
    upsert_flow(summary.clone())?;
    Ok(summary)
}

/// Rename a saved flow without touching the underlying demonstration
/// directory. Surfaces from the Saved Flows tab's Rename button.
#[tauri::command]
pub fn rename_flow(flow_id: String, new_name: String) -> Result<(), String> {
    let trimmed_new_name = new_name.trim().to_string();
    if trimmed_new_name.is_empty() {
        return Err("new flow name cannot be empty".to_string());
    }
    let mut index = storage::load_index();
    let entry = index
        .entries
        .iter_mut()
        .find(|entry| entry.flow_id == flow_id)
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    entry.name = trimmed_new_name;
    storage::save_index(&index)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedFlow {
    pub format: String,
    pub schema_version: u32,
    pub summary: FlowSummary,
    pub demonstration: crate::recorder::types::Demonstration,
}

const EXPORT_FORMAT_IDENTIFIER: &str = "tiptour-flow";
const EXPORT_SCHEMA_VERSION: u32 = 1;

/// Serialize a saved flow + its underlying demonstration into a
/// self-contained JSON blob the user can save to disk and share.
#[tauri::command]
pub fn export_flow(flow_id: String) -> Result<String, String> {
    let index = load_index();
    let summary = index
        .entries
        .iter()
        .find(|entry| entry.flow_id == flow_id)
        .cloned()
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    let demonstration =
        crate::recorder::persistence::load_demonstration(&summary.flow_id)?;
    let exported = ExportedFlow {
        format: EXPORT_FORMAT_IDENTIFIER.to_string(),
        schema_version: EXPORT_SCHEMA_VERSION,
        summary,
        demonstration,
    };
    serde_json::to_string_pretty(&exported).map_err(|error| error.to_string())
}

/// Deserialize an exported-flow blob and write it into the local
/// multiflow + demonstrations layout. Returns the resulting summary so
/// the UI can update its list without reloading.
#[tauri::command]
pub fn import_flow(exported_blob: String) -> Result<FlowSummary, String> {
    let exported: ExportedFlow =
        serde_json::from_str(&exported_blob).map_err(|error| error.to_string())?;
    if exported.format != EXPORT_FORMAT_IDENTIFIER {
        return Err(format!(
            "unrecognized export format '{}'",
            exported.format
        ));
    }
    crate::recorder::persistence::write_demonstration_from_export(&exported.demonstration)?;
    upsert_flow(exported.summary.clone())?;
    Ok(exported.summary)
}
