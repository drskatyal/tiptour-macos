// Indicator-pill-strip window controller. The window is the fourth
// Tauri webview declared in `tauri.conf.json` under the label
// `indicators`: narrow vertical strip glued to one edge of the primary
// monitor, transparent, always-on-top, click-through except on the pill
// surfaces themselves.
//
// All visual state lives in `indicators.html` / `src/indicators.ts`;
// this module owns the OS-level window concerns: pin to the right (or
// left) edge of the primary monitor, set the click-through flag,
// macOS space-collection behavior + window level, and show/hide based
// on user settings.

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::indicators_settings::{
    load_indicators_settings_from_disk, IndicatorPosition, IndicatorSettings,
};

const INDICATORS_WINDOW_LABEL: &str = "indicators";
const INDICATORS_STRIP_WIDTH_PHYSICAL: u32 = 320;

pub fn ensure_installed(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(INDICATORS_WINDOW_LABEL) {
        configure_indicators_window(&window)?;
        let settings = load_indicators_settings_from_disk();
        apply_settings_to_window(&window, &settings);
    }
    Ok(())
}

fn configure_indicators_window(window: &WebviewWindow) -> tauri::Result<()> {
    // Stay above the user's actual app windows but never claim focus.
    let _ = window.set_always_on_top(true);
    let _ = window.set_skip_taskbar(true);
    // Default to click-through. The webview JS toggles this off on
    // hover-enter of an individual pill so the pill is interactive,
    // then back on when the mouse leaves the stack.
    let _ = window.set_ignore_cursor_events(true);

    // macOS: ride along every space and over fullscreen apps via the
    // same NSWindow trick `overlay.rs` uses. NSStatusWindowLevel (25)
    // sits above the floating panel but below screen savers.
    #[cfg(target_os = "macos")]
    {
        if let Ok(ns_window) = window.ns_window() {
            unsafe {
                use objc2::msg_send;
                use objc2::runtime::AnyObject;
                let ns_window_object = ns_window as *mut AnyObject;
                let collection_behavior: u64 = (1 << 0) | (1 << 8);
                let _: () = msg_send![ns_window_object, setCollectionBehavior: collection_behavior];
                let _: () = msg_send![ns_window_object, setLevel: 25_i64];
            }
        }
    }

    Ok(())
}

/// Look up the primary monitor and snap the indicator strip to the
/// configured edge. Falls back to the first available monitor when no
/// monitor advertises itself as primary (some Linux + multi-monitor
/// Windows configs).
fn snap_to_primary_monitor_edge(window: &WebviewWindow, position: IndicatorPosition) {
    let primary_monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.available_monitors().ok().and_then(|monitors| monitors.into_iter().next()));

    let Some(primary_monitor) = primary_monitor else {
        return;
    };
    let monitor_position = primary_monitor.position();
    let monitor_size = primary_monitor.size();

    let strip_x = match position {
        IndicatorPosition::RightEdge => {
            monitor_position.x + (monitor_size.width as i32)
                - (INDICATORS_STRIP_WIDTH_PHYSICAL as i32)
        }
        IndicatorPosition::LeftEdge => monitor_position.x,
        // Disabled is handled before this function is called — but if
        // we somehow get here, default to the right edge so the window
        // still has a sane position.
        IndicatorPosition::Disabled => {
            monitor_position.x + (monitor_size.width as i32)
                - (INDICATORS_STRIP_WIDTH_PHYSICAL as i32)
        }
    };
    let strip_y = monitor_position.y;

    let _ = window.set_position(PhysicalPosition::new(strip_x, strip_y));
    let _ = window.set_size(PhysicalSize::new(
        INDICATORS_STRIP_WIDTH_PHYSICAL,
        monitor_size.height,
    ));
}

fn apply_settings_to_window(window: &WebviewWindow, settings: &IndicatorSettings) {
    match settings.position {
        IndicatorPosition::Disabled => {
            let _ = window.hide();
        }
        IndicatorPosition::RightEdge | IndicatorPosition::LeftEdge => {
            snap_to_primary_monitor_edge(window, settings.position);
            let _ = window.show();
        }
    }
}

/// Re-snap and show/hide the indicators window after a settings change.
/// Public so `indicators_settings::set_indicators_settings` can call it.
pub fn apply_settings(app: &AppHandle, settings: &IndicatorSettings) {
    if let Some(window) = app.get_webview_window(INDICATORS_WINDOW_LABEL) {
        apply_settings_to_window(&window, settings);
    }
}

/// JS-driven toggle of the OS-level click-through flag. The webview
/// calls this on pill mouseenter / stack-mouseleave instead of using
/// `getCurrentWindow().setIgnoreCursorEvents` directly so all the
/// platform-specific edge cases (macOS needs the flag re-applied after
/// resize, etc.) stay in Rust.
#[tauri::command]
pub fn indicators_set_click_through(
    app: AppHandle,
    is_click_through: bool,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(INDICATORS_WINDOW_LABEL) {
        window
            .set_ignore_cursor_events(is_click_through)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
