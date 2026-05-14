// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod grounding;
mod hotkey;
mod keychain;
mod tray;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            tray::install(app.handle())?;
            hotkey::install(app.handle())?;

            // Panel starts hidden; tray click reveals it.
            if let Some(window) = app.get_webview_window("panel") {
                let _ = window.hide();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            keychain::get_api_key,
            keychain::set_api_key,
            audio::start_mic_capture,
            audio::stop_mic_capture,
            audio::play_audio_chunk,
            grounding::prefetch_target_app,
            grounding::resolve_label,
            grounding::get_shortcut_index,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TipTour");
}
