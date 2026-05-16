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
    /// Best-effort PID of the frontmost app at the moment
    /// `start_dictation` fired. Subsequent `type_dictation_chunk`
    /// calls refocus this PID before pasting so the user can
    /// glance at TipTour mid-dictation without the next chunk
    /// landing in TipTour's panel. None on platforms where the
    /// frontmost-app query isn't wired (Linux dev path).
    pub anchored_pid: Option<i32>,
    /// Human-readable bundle/window name of the anchored target,
    /// surfaced to the panel UI as "Dictating into <App>".
    pub anchored_app_name: Option<String>,
}

/// Best-effort frontmost-app snapshot. We deliberately avoid
/// depending on the platform-specific AX/UIA infrastructure here
/// because that already lives in `grounding/`; pulling it back into
/// dictation would entangle two large modules. Instead we sniff
/// the OS-native "frontmost process" identifier via the lightest
/// API per platform.
#[cfg(target_os = "macos")]
pub fn snapshot_frontmost_app() -> (Option<i32>, Option<String>) {
    use std::process::Command;
    // `osascript` is available on every Mac and won't require
    // accessibility permissions for "frontmost process" — only the
    // unix_id + name are read. ~30ms cost.
    let script = "tell application \"System Events\" to get {unix id, name} of first application process whose frontmost is true";
    let output = Command::new("osascript").arg("-e").arg(script).output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // Format: "12345, AppName"
            let mut parts = stdout.splitn(2, ", ");
            let pid = parts.next().and_then(|s| s.parse::<i32>().ok());
            let name = parts.next().map(|s| s.to_string());
            (pid, name)
        }
        _ => (None, None),
    }
}
#[cfg(target_os = "windows")]
pub fn snapshot_frontmost_app() -> (Option<i32>, Option<String>) {
    // Lightweight Win32 call — GetForegroundWindow then
    // GetWindowThreadProcessId. Both are no-permission APIs.
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return (None, None);
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return (None, None);
        }
        (Some(pid as i32), None)
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn snapshot_frontmost_app() -> (Option<i32>, Option<String>) {
    (None, None)
}

/// Re-focus the previously-anchored app so the next `type_dictation_chunk`
/// paste lands in the right window. Best-effort: if the app is gone
/// (closed, hidden) we type into whatever's currently focused.
#[cfg(target_os = "macos")]
fn refocus_anchored_pid(pid: i32) {
    use std::process::Command;
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true",
        pid
    );
    let _ = Command::new("osascript").arg("-e").arg(&script).output();
}
#[cfg(target_os = "windows")]
fn refocus_anchored_pid(pid: i32) {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, SetForegroundWindow,
    };
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let target_pid = lparam.0 as u32;
            let mut window_pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            if window_pid == target_pid {
                let _ = SetForegroundWindow(hwnd);
                return BOOL(0); // stop enumeration
            }
            BOOL(1)
        }
    }
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(pid as isize));
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn refocus_anchored_pid(_pid: i32) {
    // Linux dev path: no-op. Linux Tauri builds focus the previous
    // window by default when a Tauri window loses focus.
}

static DICTATION_STATE: Lazy<Mutex<DictationState>> =
    Lazy::new(|| Mutex::new(DictationState::default()));

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Frontmost-app summary for the dock's focused-app pip. Returns
/// just the human-readable name (the pid stays internal). None on
/// platforms where we can't sniff frontmost (or when the snapshot
/// failed transiently — caller should hide the pip in that case).
#[tauri::command]
pub fn get_frontmost_app_name() -> Option<String> {
    let (_pid, name) = snapshot_frontmost_app();
    name
}

#[tauri::command]
pub fn start_dictation(app: AppHandle) -> Result<DictationState, String> {
    // Snapshot the frontmost app BEFORE we touch the mutex so the
    // OS-level query isn't delayed by lock contention. The user
    // expected to have already clicked into their target field
    // before saying "start transcription".
    let (anchored_pid, anchored_app_name) = snapshot_frontmost_app();

    let mut state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
    *state = DictationState {
        is_active: true,
        characters_typed: 0,
        started_at_unix_ms: now_unix_ms(),
        anchored_pid,
        anchored_app_name,
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
    let anchored_pid_for_refocus = {
        let state = DICTATION_STATE.lock().map_err(|_| "dictation state poisoned")?;
        if !state.is_active {
            return Err(
                "Dictation is not active. Call start_dictation first.".to_string(),
            );
        }
        state.anchored_pid
    };
    if text.is_empty() {
        return Ok(());
    }
    // Refocus the anchored target before pasting so the user can
    // glance at TipTour (or Cmd-Tab away briefly) mid-dictation and
    // the next chunk still lands in the originally-focused field.
    if let Some(pid) = anchored_pid_for_refocus {
        refocus_anchored_pid(pid);
        // Tiny settle so the window-server actually flips focus
        // before the Cmd+V keystroke fires.
        std::thread::sleep(std::time::Duration::from_millis(40));
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
///
/// Restores the previous clipboard contents after reading so we don't
/// destroy whatever the user had on the pasteboard before we ran.
/// Non-text contents (images, files) aren't restored — arboard's
/// text-only API is good enough for the common case.
#[tauri::command]
pub fn capture_selection_via_clipboard() -> Result<String, String> {
    use crate::executor::cross_platform_input;

    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("clipboard open: {error}"))?;

    // Save whatever was on the clipboard so we can restore it after
    // we read the selection. Falls through silently if the previous
    // payload was non-text.
    let previous_clipboard_text = clipboard.get_text().ok();

    // Copy whatever's currently selected. We don't trigger a
    // select-all here — the user is expected to have already
    // highlighted the target text (rich editors don't reliably
    // forward Cmd+A across all surfaces).
    let copy_keys = ["Cmd", "C"];
    cross_platform_input::keyboard_shortcut(&copy_keys)?;
    // Small settle so the source app actually writes the clipboard
    // before we read it.
    std::thread::sleep(std::time::Duration::from_millis(80));
    let captured_text = clipboard
        .get_text()
        .map_err(|error| format!("clipboard read: {error}"))?;

    // Restore the user's previous clipboard. Tiny delay first so any
    // paste-watcher consumers (e.g. clipboard history apps) record
    // our captured text before we overwrite it.
    if let Some(previous_text) = previous_clipboard_text {
        std::thread::sleep(std::time::Duration::from_millis(120));
        let _ = clipboard.set_text(previous_text);
    }

    Ok(captured_text)
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
            anchored_pid: Some(12345),
            anchored_app_name: Some("Notes".to_string()),
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
