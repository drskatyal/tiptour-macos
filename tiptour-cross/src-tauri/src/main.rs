// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod capabilities;
mod executor;
mod grounding;
mod hotkey;
mod keychain;
mod mode;
mod overlay;
#[cfg(target_os = "macos")]
mod permissions_macos;
mod permissions;
mod recorder;
mod screen;
mod tray;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            tray::install(app.handle())?;
            hotkey::install(app.handle())?;
            overlay::ensure_installed(app.handle())?;

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
            grounding::resolve_label_with_hint,
            grounding::get_shortcut_index,
            capabilities::explore_app,
            capabilities::list_capabilities,
            capabilities::retrieve_tools,
            capabilities::invoke_capability,
            recorder::is_recording_enabled,
            recorder::set_recording_enabled,
            recorder::start_passive_recording,
            recorder::stop_passive_recording,
            recorder::start_demonstration,
            recorder::stop_demonstration,
            recorder::append_narration_audio_chunk,
            recorder::list_demonstrations,
            recorder::load_demonstration,
            recorder::mine_patterns,
            screen::start_screen_stream,
            screen::stop_screen_stream,
            executor::execute_workflow_plan,
            permissions::check_accessibility_permission,
            permissions::request_accessibility_permission,
            permissions::check_screen_recording_permission,
            permissions::request_screen_recording_permission,
            overlay::overlay_show,
            overlay::overlay_hide,
            overlay::overlay_fly_cursor_to,
            overlay::overlay_show_response,
            overlay::overlay_hide_response,
            overlay::overlay_set_speaking,
            mode::get_operating_mode,
            mode::set_operating_mode,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TipTour");
}
