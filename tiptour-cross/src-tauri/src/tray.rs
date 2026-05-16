// Native system tray. Left-click toggles the floating panel; right-click
// opens a richer menu including session toggle, mode radio, recent
// flows submenu, settings, and quit.
//
// Tauri 2's tray API requires building a fresh `Menu` each time we want
// to update visible state (no in-place item mutation across all
// platforms). `rebuild_tray_menu` is callable from anywhere — the
// session-status emit path on the panel side fires it after every
// state transition.

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use once_cell::sync::Lazy;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WebviewWindow,
};

const TRAY_ICON_ID: &str = "main";

// Tracked session state so the menu can render the right toggle text
// and the active-mode check. Updated via setters that also rebuild the
// menu, so the tray reflects truth without anyone polling.
static IS_SESSION_ACTIVE: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;

    let _tray = TrayIconBuilder::with_id(TRAY_ICON_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref().to_string()))
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

fn handle_menu_event(app: &AppHandle, event_id: String) {
    match event_id.as_str() {
        "quit" => request_graceful_shutdown(app),
        "toggle_panel" => {
            if let Some(window) = app.get_webview_window("panel") {
                toggle_panel(&window);
            }
        }
        "session_toggle" => {
            // The panel owns the actual GeminiLiveSession lifecycle —
            // emit the same event the hotkey publishes so the existing
            // togglePushToTalk path runs end-to-end.
            let _ = app.emit("tray_session_toggle", ());
            let _ = app.emit("push_to_talk_toggled", ());
        }
        "mode_autopilot" => {
            let _ = crate::mode::set_operating_mode(crate::mode::OperatingMode::Autopilot);
            let _ = rebuild_tray_menu(app);
        }
        "mode_teaching" => {
            let _ = crate::mode::set_operating_mode(crate::mode::OperatingMode::Teaching);
            let _ = rebuild_tray_menu(app);
        }
        "open_settings" => {
            if let Err(open_error) =
                crate::settings_window::open_settings_window(app.clone())
            {
                eprintln!("[tray] open settings failed: {open_error}");
            }
        }
        other_id => {
            // Recent-flows submenu items are stamped with id `flow:<name>`.
            if let Some(flow_name) = other_id.strip_prefix("flow:") {
                let app_clone = app.clone();
                let flow_name_owned = flow_name.to_string();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::multiflow::run_flow_by_name(
                        flow_name_owned,
                        app_clone,
                    )
                    .await;
                });
            }
        }
    }
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let toggle_panel_item = MenuItem::with_id(
        app,
        "toggle_panel",
        "Show / Hide panel",
        true,
        None::<&str>,
    )?;

    let session_active = *IS_SESSION_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let session_label = if session_active {
        "Stop session"
    } else {
        "Start session"
    };
    let session_toggle_item = MenuItem::with_id(
        app,
        "session_toggle",
        session_label,
        true,
        None::<&str>,
    )?;

    let current_mode = crate::mode::current_operating_mode();
    let mode_autopilot_item = CheckMenuItem::with_id(
        app,
        "mode_autopilot",
        "Mode: Autopilot",
        true,
        matches!(current_mode, crate::mode::OperatingMode::Autopilot),
        None::<&str>,
    )?;
    let mode_teaching_item = CheckMenuItem::with_id(
        app,
        "mode_teaching",
        "Mode: Teaching",
        true,
        matches!(current_mode, crate::mode::OperatingMode::Teaching),
        None::<&str>,
    )?;

    // Recent flows submenu — top 3 by created_at descending. Empty list
    // is collapsed away so the user doesn't see a dead "(none)" branch.
    let recent_flows = crate::multiflow::list_flows().unwrap_or_default();
    let mut sorted_flows = recent_flows;
    sorted_flows.sort_by(|a, b| b.created_at_unix_ms.cmp(&a.created_at_unix_ms));
    let top_three_flows: Vec<_> = sorted_flows.into_iter().take(3).collect();

    let recent_flows_submenu_opt = if top_three_flows.is_empty() {
        None
    } else {
        let mut flow_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
        for flow in &top_three_flows {
            flow_items.push(MenuItem::with_id(
                app,
                format!("flow:{}", flow.name),
                &flow.name,
                true,
                None::<&str>,
            )?);
        }
        let item_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = flow_items
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>)
            .collect();
        Some(Submenu::with_items(app, "Recent flows", true, &item_refs)?)
    };

    let open_settings_item = MenuItem::with_id(
        app,
        "open_settings",
        "Settings…",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit TipTour", true, None::<&str>)?;

    let mut item_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![
        &toggle_panel_item,
        &session_toggle_item,
        &mode_autopilot_item,
        &mode_teaching_item,
    ];
    if let Some(ref submenu) = recent_flows_submenu_opt {
        item_refs.push(submenu);
    }
    item_refs.push(&open_settings_item);
    item_refs.push(&quit_item);

    Menu::with_items(app, &item_refs)
}

/// Rebuild the tray menu from current state. Called whenever any of the
/// underlying state (session active / mode / recent flows) changes.
pub fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

/// Public setter the panel calls (via the `set_tray_session_active`
/// command) when the session opens or closes.
pub fn set_session_active(app: &AppHandle, active: bool) {
    if let Ok(mut guard) = IS_SESSION_ACTIVE.lock() {
        *guard = active;
    }
    let _ = rebuild_tray_menu(app);
}

/// Tauri command — invoked from the panel after a Gemini Live session
/// open/close so the tray "Start session" / "Stop session" toggle and
/// the recent-flows snapshot reflect truth.
#[tauri::command]
pub fn set_tray_session_active(active: bool, app: AppHandle) -> Result<(), String> {
    set_session_active(&app, active);
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
/// menu. Keeps an in-flight multiflow replay from being cut off
/// mid-keystroke.
fn request_graceful_shutdown(app: &AppHandle) {
    crate::multiflow::replayer::set_active_replay_token("__user_quit__");
    let _ = crate::audio::stop_mic_capture();
    crate::crash_recovery::mark_clean_shutdown();
    let app_clone = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        app_clone.exit(0);
    });
}
