// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapters;
mod agent_memory;
mod app_discovery;
mod audio_query;
mod soniox_transcribe;
mod app_metadata;
mod app_settings;
mod audio;
mod bug_report;
mod capabilities;
mod conversation_history;
mod cost_meter;
mod diagnostics;
mod crash_recovery;
mod custom_commands;
mod dictation;
mod executor;
mod gemini_live_client;
mod grounding;
mod highlight;
mod hotkey;
mod indicators;
mod indicators_settings;
mod indicators_window;
mod keychain;
mod mode;
mod multiflow;
mod onboarding;
mod overlay;
mod personas;
#[cfg(target_os = "macos")]
mod permissions_macos;
mod permissions;
mod recorder;
mod screen;
mod settings_window;
mod subagents;
mod tasks;
mod text_rewrite;
mod tool_dispatch;
mod tray;
mod vosk_listener;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // macOS: become a true menu-bar / accessory app so we don't
            // show up in the Dock alongside the tray icon. Without this,
            // the user sees two icons — one in the Dock, one in the
            // status bar — and the panel window steals focus from
            // whatever they were doing.
            #[cfg(target_os = "macos")]
            {
                let _ = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // Detect whether the previous run shut down cleanly before
            // stamping a fresh boot sentinel. If it didn't, surface a
            // banner the panel listens for.
            let previous_run_crashed = crash_recovery::record_boot_and_detect_previous_crash();
            if previous_run_crashed {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Wait a beat so the panel webview has had time to
                    // install its listener before we emit.
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let _ = tauri::Emitter::emit(&app_handle, "previous_session_crashed", ());
                });
            }
            tray::install(app.handle())?;
            // Hotkey registration can fail if another running app holds
            // Alt+X (Microsoft Word's "insert symbol" chord, for example).
            // Treat that as a soft failure: log it and keep the app
            // usable — the user can still open the panel from the tray
            // and click Start. Without this softening the whole app
            // crashes at setup via the `.expect()` in main(), which is
            // a much worse first-run experience than a missing hotkey.
            if let Err(hotkey_install_error) = hotkey::install(app.handle()) {
                let message = format!(
                    "{hotkey_install_error}"
                );
                eprintln!("[hotkey] global hotkey registration failed: {message}");
                // Emit so the panel can surface a banner — users were
                // hitting Alt+X expecting it to work and getting silence
                // because the failure was console-only.
                let app_handle_for_emit = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let _ = tauri::Emitter::emit(
                        &app_handle_for_emit,
                        "hotkey_registration_failed",
                        message,
                    );
                });
            }
            overlay::ensure_installed(app.handle())?;
            indicators_window::ensure_installed(app.handle())?;
            // Center the dock window at the bottom of the primary
            // monitor's work area. The tauri.conf.json gives the dock
            // a fixed initial x/y guess that's right for a 1920×1080
            // primary, but real users have all sizes — position it
            // dynamically here so it lands flush bottom-center on
            // every display.
            if let Some(dock_window) = app.handle().get_webview_window("dock") {
                if let Ok(Some(primary_monitor)) = dock_window.primary_monitor() {
                    let monitor_position = primary_monitor.position();
                    let monitor_size = primary_monitor.size();
                    let dock_width: u32 = 520;
                    let dock_height: u32 = 120;
                    // Sit ~7% above the bottom edge so the dock floats
                    // in the visual lower-middle rather than glued to
                    // the screen edge. On a 1080p display this lands
                    // around y=930; on a 1440p display around y=1310.
                    let bottom_margin = (monitor_size.height as f32 * 0.07) as i32;
                    let new_x = monitor_position.x
                        + ((monitor_size.width as i32 - dock_width as i32) / 2);
                    let new_y = monitor_position.y + monitor_size.height as i32
                        - dock_height as i32
                        - bottom_margin;
                    let _ = dock_window
                        .set_position(tauri::PhysicalPosition::new(new_x, new_y));
                    // Make the dock taller so the action-pill tooltip
                    // that floats above the chip cluster has room to
                    // render — the previous 60px height was clipping
                    // the tooltip to a sliver.
                    let _ = dock_window.set_size(tauri::PhysicalSize::new(
                        dock_width, dock_height,
                    ));
                }
            }

            // Position the panel near the top-right of the primary
            // monitor so users always see it when the app launches.
            // Without this Tauri picks a default that's often offscreen
            // on multi-monitor setups, which has been the root of
            // "I launched the app and nothing happened" reports.
            if let Some(panel_window) = app.handle().get_webview_window("panel") {
                if let Ok(Some(primary_monitor)) = panel_window.primary_monitor() {
                    let monitor_position = primary_monitor.position();
                    let monitor_size = primary_monitor.size();
                    let panel_width: u32 = 380;
                    let panel_height: u32 = 600;
                    // 24px from the right edge, 80px from the top so it
                    // doesn't collide with the menu bar / taskbar.
                    let new_x = monitor_position.x + monitor_size.width as i32
                        - panel_width as i32
                        - 24;
                    let new_y = monitor_position.y + 80;
                    let _ = panel_window
                        .set_position(tauri::PhysicalPosition::new(new_x, new_y));
                    let _ = panel_window.show();
                    let _ = panel_window.set_focus();
                }
            }
            // Stash the AppHandle globally so deep callers (the
            // recorder's fire-and-forget screenshot writer, etc.) can
            // emit indicator events without threading a handle through
            // every layer.
            indicators::install_global_app_handle(app.handle().clone());
            highlight::start_listening(app.handle().clone());
            // Try to resume the always-on local listener if the user
            // opted in on a previous launch. No-op when the feature
            // flag is off or the model isn't on disk.
            vosk_listener::auto_start_if_user_opted_in(app.handle());

            // Warm the discovered-apps cache from disk and trigger a
            // background rescan if the on-disk snapshot is stale. The
            // Vosk grammar builder reads from this cache, so we want
            // results available before the user says the wake word.
            app_discovery::kickoff_background_scan(app.handle());

            // Show the panel on first launch so the user actually sees
            // something when the app boots — otherwise the tray icon is
            // the only visible artifact and first-run feels broken. The
            // tray click still toggles visibility after this.
            if let Some(window) = app.get_webview_window("panel") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            keychain::get_api_key,
            keychain::set_api_key,
            keychain::get_provider_api_key,
            keychain::set_provider_api_key,
            keychain::clear_provider_api_key,
            adapters::list_adapters,
            adapters::set_adapter_enabled,
            adapters::get_enabled_adapter_hints,
            adapters::dispatch_adapter_command,
            adapters::defaults::list_default_categories,
            adapters::defaults::set_default_adapter,
            adapters::defaults::get_default_adapter,
            audio_query::begin_quick_voice_capture,
            audio_query::append_quick_voice_chunk,
            audio_query::end_quick_voice_capture_and_dispatch,
            soniox_transcribe::toggle_soniox_transcription,
            soniox_transcribe::start_soniox_transcription,
            soniox_transcribe::stop_soniox_transcription,
            soniox_transcribe::append_soniox_audio_chunk,
            soniox_transcribe::get_soniox_state,
            text_rewrite::list_providers,
            text_rewrite::rewrite_selection,
            text_rewrite::open_drafting_window_with_text,
            text_rewrite::rewrite_selection_into_drafting_window,
            text_rewrite::paste_from_drafting_window,
            text_rewrite::clipboard_rich::put_clipboard_rich,
            dictation::get_frontmost_app_name,
            dictation::start_dictation,
            dictation::stop_dictation,
            dictation::get_dictation_state,
            dictation::type_dictation_chunk,
            dictation::capture_selection_via_clipboard,
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
            app_settings::set_app_settings_with_broadcast,
            app_settings::set_app_setting_field,
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
            hotkey::reregister_transcribe_hotkey,
            app_discovery::list_discovered_apps,
            app_discovery::rescan_installed_apps,
            app_discovery::set_app_command_enabled,
            indicators_settings::get_indicators_settings,
            indicators_settings::set_indicators_settings,
            indicators_window::indicators_set_click_through,
            indicators::emit_indicator_from_frontend,
            agent_memory::remember,
            agent_memory::recall,
            agent_memory::forget,
            agent_memory::list_memories,
            agent_memory::update_memory,
            agent_memory::list_top_importance_memories,
            conversation_history::append_session_turn,
            conversation_history::list_sessions,
            conversation_history::get_session_history,
            conversation_history::delete_session,
            conversation_history::get_last_session_tail,
            diagnostics::run_diagnostics,
            subagents::spawn_subagent_command,
            subagents::list_subagents,
            subagents::get_subagent_status,
            subagents::cancel_subagent,
            subagents::pause_subagent,
            subagents::resume_subagent,
            subagents::report_subagent_progress,
            subagents::append_subagent_transcript,
            subagents::sweep_stalled_subagents_command,
            subagents::get_subagent_transcript_path,
            tasks::create_task,
            tasks::update_task_status,
            tasks::update_task,
            tasks::delete_task,
            tasks::list_tasks,
            tasks::count_tasks_in_progress,
            tasks::dispatch_task_to_subagent,
            tool_dispatch::dispatch_tool_call,
            onboarding::is_first_run,
            onboarding::mark_first_run_complete,
            onboarding::reset_first_run,
            personas::list_personas,
            personas::get_active_persona,
            personas::set_active_persona,
            personas::upsert_custom_persona,
            personas::delete_persona,
            personas::match_persona_by_voice,
            cost_meter::record_usage,
            cost_meter::get_session_cost,
            cost_meter::get_today_cost,
            cost_meter::get_cost_history,
            bug_report::export_bug_report,
            tray::set_tray_session_active,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TipTour");
}
