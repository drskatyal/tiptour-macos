// `open_settings_window` Tauri command. The settings window is the
// deep-edit surface for everything that doesn't fit in the 320×320
// quick-action panel — API key, voice/model, hotkey, recordings,
// capabilities, etc. Created lazily on demand so we don't pay the
// webview boot cost until the user actually clicks the gear icon.

use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

const SETTINGS_WINDOW_LABEL: &str = "settings";
const SETTINGS_WINDOW_WIDTH: f64 = 720.0;
const SETTINGS_WINDOW_HEIGHT: f64 = 620.0;

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    // If the window already exists (user clicked the gear twice), just
    // bring it to the front — re-creating would 1) cost a second
    // webview boot, and 2) drop any in-progress edits in the tab.
    if let Some(existing_window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = existing_window.show();
        let _ = existing_window.set_focus();
        return Ok(());
    }

    let window_result = WebviewWindowBuilder::new(
        &app,
        SETTINGS_WINDOW_LABEL,
        WebviewUrl::App("settings.html".into()),
    )
    .title("TipTour Settings")
    .inner_size(SETTINGS_WINDOW_WIDTH, SETTINGS_WINDOW_HEIGHT)
    .min_inner_size(560.0, 480.0)
    .resizable(true)
    .decorations(true)
    .visible(true)
    .build();

    match window_result {
        Ok(window) => {
            let _ = window.set_size(LogicalSize::new(
                SETTINGS_WINDOW_WIDTH,
                SETTINGS_WINDOW_HEIGHT,
            ));
            let _ = window.set_focus();
            Ok(())
        }
        Err(error) => Err(format!("failed to create settings window: {error}")),
    }
}
