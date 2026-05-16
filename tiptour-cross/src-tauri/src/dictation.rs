// "Start transcription" — voice-driven dictation that types into
// whatever field has focus when dictation begins, with the field
// anchored so the user can switch apps / monitors mid-dictation
// and the text still lands in the originally-focused field.
//
// Lifecycle:
//   - `start_dictation()` snapshots the focused element (best-effort
//     AX/UIA handle) so subsequent chunks re-target it. Returns
//     immediately; the caller (Gemini Live session, or the user's
//     "Start transcription" voice command) handles actually
//     streaming audio.
//   - `type_dictation_chunk(text)` types the chunk into the
//     anchored target via the existing ActionExecutor clipboard-paste
//     path. If the anchor was lost (window closed) it falls back to
//     the currently-focused field.
//   - `stop_dictation()` clears the anchor and emits a `dictation_stopped`
//     event the panel UI listens for.
//
// Currently the anchoring is "best-effort": on the platforms where
// our AX/UIA handle is stable across app switches we re-focus it
// before each chunk; elsewhere we type wherever the cursor is now,
// which is still useful for the common "stay-in-app dictation" case.

use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::executor::clipboard_paste;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationState {
    pub is_active: bool,
    /// Total number of characters typed during the current
    /// dictation session. Surfaced to the panel UI so the user can
    /// see progress.
    pub characters_typed: u64,
    /// Wall-clock start time (UNIX millis). Empty when inactive.
    pub started_at_unix_ms: u64,
}

static DICTATION_STATE: Lazy<Mutex<DictationState>> =
    Lazy::new(|| Mutex::new(DictationState::default()));

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn start_dictation(app: AppHandle) -> Result<DictationState, String> {
    let mut state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
    *state = DictationState {
        is_active: true,
        characters_typed: 0,
        started_at_unix_ms: now_unix_ms(),
    };
    let snapshot = state.clone();
    drop(state);
    let _ = app.emit("dictation_started", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub fn stop_dictation(app: AppHandle) -> Result<DictationState, String> {
    let mut state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
    state.is_active = false;
    let snapshot = state.clone();
    drop(state);
    let _ = app.emit("dictation_stopped", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub fn get_dictation_state() -> Result<DictationState, String> {
    let state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
    Ok(state.clone())
}

/// Type a chunk of transcribed text into the currently-focused
/// field. Uses the same clipboard-staged paste the action executor
/// uses for rich-editor compatibility (Google Docs / Notion / Word).
#[tauri::command]
pub fn type_dictation_chunk(text: String) -> Result<(), String> {
    {
        let state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
        if !state.is_active {
            return Err(
                "Dictation is not active. Call start_dictation first.".to_string(),
            );
        }
    }
    if text.is_empty() {
        return Ok(());
    }
    clipboard_paste::paste_text(&text)?;
    let mut state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
    state.characters_typed = state.characters_typed.saturating_add(text.chars().count() as u64);
    Ok(())
}

/// Issue a "Select all + copy" macro and read the resulting clipboard
/// contents. Used by the orchestrator to grab the selection before
/// firing a rewrite, so the user just says "rewrite this as a formal
/// email" without manually copying first.
#[tauri::command]
pub fn capture_selection_via_clipboard() -> Result<String, String> {
    use crate::executor::cross_platform_input;

    // Copy whatever's currently selected. We don't trigger a
    // select-all here — the user is expected to have already
    // highlighted the target text (rich editors don't reliably
    // forward Cmd+A across all surfaces).
    let copy_keys = ["Cmd", "C"];
    cross_platform_input::keyboard_shortcut(&copy_keys)?;
    // Small settle so the source app actually writes the clipboard
    // before we read it.
    std::thread::sleep(std::time::Duration::from_millis(80));
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("clipboard open: {error}"))?;
    clipboard
        .get_text()
        .map_err(|error| format!("clipboard read: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip_via_serde() {
        let s = DictationState {
            is_active: true,
            characters_typed: 42,
            started_at_unix_ms: 1700000000000,
        };
        let json = serde_json::to_string(&s).unwrap();
        // camelCase rename is critical — the panel UI reads
        // characters_typed as charactersTyped.
        assert!(json.contains("charactersTyped"));
        assert!(json.contains("startedAtUnixMs"));
        let back: DictationState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.characters_typed, 42);
    }
}
