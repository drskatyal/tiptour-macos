// Global push-to-talk hotkey.
//
// macOS: Ctrl+Option (modifier-only chord, matches TipTour's Swift app).
// Windows: Ctrl+Alt (Option doesn't exist; Alt is the closest analog).
//
// Tauri's global-shortcut plugin only fires on full chord press, so we use
// Ctrl+Alt+Space on both platforms as the registered shortcut to avoid the
// modifier-only edge case during Phase 0. A platform-native CGEventTap /
// low-level keyboard hook for true modifier-only triggers is a Phase 0.5
// follow-up.

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
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
