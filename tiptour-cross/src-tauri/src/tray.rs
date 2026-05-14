// Native system tray. Tray icon click toggles the floating panel.
//
// On macOS this lights up an NSStatusItem; on Windows it installs a
// NOTIFYICONDATA entry. Tauri's tray-icon plugin abstracts the difference.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let quit_item = MenuItem::with_id(app, "quit", "Quit TipTour", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit_item])?;

    let _tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "quit" {
                app.exit(0);
            }
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
