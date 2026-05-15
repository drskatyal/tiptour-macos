// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_metadata;
mod app_settings;
mod audio;
mod capabilities;
mod custom_commands;
mod executor;
mod grounding;
mod highlight;
mod hotkey;
mod keychain;
mod mode;
mod multiflow;
mod overlay;
#[cfg(target_os = "macos")]
mod permissions_macos;
mod permissions;
mod recorder;
mod screen;
mod settings_window;
mod tray;
mod vosk_listener;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            tray::install(app.handle())?;
            // Hotkey registration can fail if another running app holds
            // Alt+X (Microsoft Word's "insert symbol" chord, for example).
            // Treat that as a soft failure: log it and keep the app
            // usable — the user can still open the panel from the tray
            // and click Start. Without this softening the whole app
            // crashes at setup via the `.expect()` in main(), which is
            // a much worse first-run experience than a missing hotkey.
            if let Err(hotkey_install_error) = hotkey::install(app.handle()) {
                eprintln!(
                    "[hotkey] global Alt+X registration failed (likely conflicting app): {hotkey_install_error}",
                );
            }
            overlay::ensure_installed(app.handle())?;
            highlight::start_listening(app.handle().clone());
            // Try to resume the always-on local listener if the user
            // opted in on a previous launch. No-op when the feature
            // flag is off or the model isn't on disk.
            vosk_listener::auto_start_if_user_opted_in(app.handle());

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
            highlight::get_current_highlight_context,
            capabilities::explore_app,
            capabilities::list_capabilities,
            capabilities::retrieve_tools,
            capabilities::invoke_capability,
            capabilities::list_capability_index_summaries,
            capabilities::clear_capability_cache,
            capabilities::get_destructive_keywords,
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
            recorder::list_demonstration_screenshots,
            recorder::delete_demonstration,
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
            multiflow::start_recording_flow,
            multiflow::stop_recording_flow,
            multiflow::list_flows,
            multiflow::delete_flow,
            multiflow::run_flow_by_name,
            multiflow::pause_active_replay,
            multiflow::find_flow_by_voice_query,
            multiflow::set_flow_trigger_aliases,
            multiflow::adopt_demonstration_as_flow,
            multiflow::rename_flow,
            multiflow::export_flow,
            multiflow::import_flow,
            vosk_listener::is_listener_enabled,
            vosk_listener::set_listener_enabled,
            vosk_listener::start_listener,
            vosk_listener::stop_listener,
            vosk_listener::download_vosk_model_if_needed,
            app_settings::get_app_settings,
            app_settings::set_app_settings,
            app_settings::reset_all_settings,
            app_metadata::get_app_metadata,
            app_metadata::open_data_folder_in_os_file_browser,
            app_metadata::quit_app_gracefully,
            app_metadata::clear_api_key_from_keychain,
            custom_commands::list_custom_voice_commands,
            custom_commands::upsert_custom_voice_command,
            custom_commands::delete_custom_voice_command,
            settings_window::open_settings_window,
            hotkey::reregister_push_to_talk_hotkey,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TipTour");
}
