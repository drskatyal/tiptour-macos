// Recorder module — Phase 3 demonstration recorder + passive pattern miner.
// Exposes a small set of Tauri commands. Recording is OFF by default and
// must be enabled by the user via `set_recording_enabled(true)`. The flag
// is persisted to disk so the opt-in survives app restarts. Every command
// that mutates recording state additionally re-checks the persisted flag —
// the UI cannot accidentally start recording on a fresh launch.

pub mod audio_recorder;
pub mod input_capture;
pub mod pattern_miner;
pub mod persistence;
pub mod privacy;
pub mod recorder;
pub mod state_capture;
pub mod types;

use std::sync::Arc;

use once_cell::sync::Lazy;

use recorder::Recorder;
use types::{Demonstration, DemonstrationSummary, WorkflowPattern};

static GLOBAL_RECORDER: Lazy<Arc<Recorder>> = Lazy::new(|| Arc::new(Recorder::new()));

fn is_recording_currently_enabled_on_disk() -> bool {
    persistence::load_settings().is_recording_enabled
}

#[tauri::command]
pub fn is_recording_enabled() -> Result<bool, String> {
    Ok(is_recording_currently_enabled_on_disk())
}

#[tauri::command]
pub fn set_recording_enabled(enabled: bool) -> Result<(), String> {
    let new_settings = persistence::RecorderSettings {
        is_recording_enabled: enabled,
    };
    persistence::save_settings(&new_settings)?;
    Ok(())
}

#[tauri::command]
pub fn start_passive_recording() -> Result<(), String> {
    if !is_recording_currently_enabled_on_disk() {
        return Err("recording is not enabled — user must opt in".to_string());
    }
    GLOBAL_RECORDER.start_passive_recording()
}

#[tauri::command]
pub fn stop_passive_recording() -> Result<(), String> {
    GLOBAL_RECORDER.stop_passive_recording()
}

#[tauri::command]
pub fn start_demonstration(title: String) -> Result<String, String> {
    if !is_recording_currently_enabled_on_disk() {
        return Err("recording is not enabled — user must opt in".to_string());
    }
    GLOBAL_RECORDER.start_demonstration(title)
}

#[tauri::command]
pub fn stop_demonstration() -> Result<Demonstration, String> {
    GLOBAL_RECORDER.stop_demonstration()
}

#[tauri::command]
pub fn append_narration_audio_chunk(pcm: Vec<u8>) -> Result<(), String> {
    GLOBAL_RECORDER.append_narration_audio_chunk(&pcm)
}

#[tauri::command]
pub fn list_demonstrations() -> Result<Vec<DemonstrationSummary>, String> {
    persistence::list_demonstration_summaries()
}

#[tauri::command]
pub fn load_demonstration(demonstration_id: String) -> Result<Demonstration, String> {
    persistence::load_demonstration(&demonstration_id)
}

#[tauri::command]
pub fn mine_patterns(min_occurrences: usize) -> Result<Vec<WorkflowPattern>, String> {
    pattern_miner::mine_patterns_from_all_passive_traces(min_occurrences)
}

#[tauri::command]
pub fn list_demonstration_screenshots(
    demonstration_id: String,
) -> Result<Vec<String>, String> {
    persistence::list_screenshot_paths_for_demonstration(&demonstration_id)
}

#[tauri::command]
pub fn delete_demonstration(demonstration_id: String) -> Result<(), String> {
    persistence::delete_demonstration_on_disk(&demonstration_id)
}
