// Crash-recovery sentinel. Every boot stamps `app_state.json` with
// `clean_shutdown: false` and a fresh timestamp. The graceful-quit
// path flips the flag to `true` before `app.exit`. On the next boot,
// if the previous file existed AND its flag was still false, we know
// the previous run crashed (or was force-killed) and we surface a
// banner the user can click to file a bug report.

use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const STATE_FILE_NAME: &str = "app_state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRunState {
    pub started_at_unix_ms: i64,
    pub clean_shutdown: bool,
}

fn state_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(STATE_FILE_NAME);
    Some(path)
}

fn write_state(state: &AppRunState) -> Result<(), String> {
    let path = state_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

fn read_state() -> Option<AppRunState> {
    let path = state_file_path()?;
    let contents = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Inspect the previous boot's state then stamp a new one for the
/// current run. Returns `true` if the previous run did NOT shut down
/// cleanly (so the caller can show a crash-report banner).
pub fn record_boot_and_detect_previous_crash() -> bool {
    let previous_state = read_state();
    let previous_was_crash = previous_state
        .as_ref()
        .map(|state| !state.clean_shutdown)
        .unwrap_or(false);

    let fresh_state = AppRunState {
        started_at_unix_ms: Utc::now().timestamp_millis(),
        clean_shutdown: false,
    };
    let _ = write_state(&fresh_state);

    previous_was_crash
}

/// Flip the current run's `clean_shutdown` flag to true. The graceful
/// quit path calls this before `app.exit(0)`; if the process is
/// force-killed it never runs, and the next boot will treat the
/// previous run as crashed.
pub fn mark_clean_shutdown() {
    let state = AppRunState {
        started_at_unix_ms: Utc::now().timestamp_millis(),
        clean_shutdown: true,
    };
    let _ = write_state(&state);
}

#[allow(dead_code)]
pub fn reset_in_memory_state_for_tests() {
    // Nothing in-memory to clear today — kept as a symmetry hook with
    // other modules' test helpers in case we add a cache later.
}
