// Native system tray. Left-click toggles the floating panel;
// right-click opens a menu with Show/Hide panel, Settings, and Quit.
//
// On macOS this lights up an NSStatusItem; on Windows it installs a
// NOTIFYICONDATA entry. Tauri's tray-icon plugin abstracts the difference.

use std::thread;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let toggle_panel_item = MenuItem::with_id(
        app,
        "toggle_panel",
        "Show / Hide panel",
        true,
        None::<&str>,
    )?;
    let open_settings_item = MenuItem::with_id(
        app,
        "open_settings",
        "Settings…",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit TipTour", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle_panel_item, &open_settings_item, &quit_item],
    )?;

    let _tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => request_graceful_shutdown(app),
            "toggle_panel" => {
                if let Some(window) = app.get_webview_window("panel") {
                    toggle_panel(&window);
                }
            }
            "open_settings" => {
                // Delegate to the settings_window module's command so we
                // route through the same lazy-create path as the gear
                // icon in the panel header.
                if let Err(open_error) =
                    crate::settings_window::open_settings_window(app.clone())
                {
                    eprintln!("[tray] open settings failed: {open_error}");
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("panel") {
                    toggle_panel(&window);
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn toggle_panel(window: &WebviewWindow) {
    match window.is_visible() {
        Ok(true) => {
            let _ = window.hide();
        }
        _ => {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Shut down cleanly when the user picks "Quit TipTour" from the tray
/// menu. Without this step `app.exit(0)` would terminate an in-flight
/// multiflow replay mid-keystroke, potentially leaving the user's
/// foreground app in a half-typed state. The sequence:
///   1. Stamp a "user quit" sentinel onto the replay token slot. Any
///      running replay sees its token superseded on its next per-step
///      check (sub-50ms) and exits cleanly via the existing Paused
///      progress event.
///   2. Stop the global mic and screen streams via the audio/screen
///      commands' Drop paths — both are idempotent and safe to call
///      from a non-async context.
///   3. Give the runner a 150ms grace window to observe the cancellation
///      before tearing down the process.
///   4. Call `app.exit(0)` from a spawned thread so the menu-event
///      callback can return immediately.
fn request_graceful_shutdown(app: &AppHandle) {
    crate::multiflow::replayer::set_active_replay_token("__user_quit__");
    let _ = crate::audio::stop_mic_capture();
    let app_clone = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        app_clone.exit(0);
    });
}
