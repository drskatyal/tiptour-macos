// Global push-to-talk hotkey.
//
// Alt+B on both macOS and Windows. Picked for one-hand thumb-plus-index
// ergonomics: left thumb on Alt (immediately left of Space) and left index
// on B (one row above Space). Not assigned by either OS as a system shortcut.

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyB);
    let app_handle = app.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _sc, event| {
            if event.state == ShortcutState::Pressed {
                let _ = app_handle.emit("push_to_talk_toggled", ());
            }
        })
        .map_err(|error| tauri::Error::Anyhow(anyhow::anyhow!(error.to_string())))?;

    Ok(())
}
