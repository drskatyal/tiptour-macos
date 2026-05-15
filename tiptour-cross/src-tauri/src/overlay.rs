// Overlay-window controller. The overlay is a second Tauri webview labelled
// "overlay" in tauri.conf.json: full-screen transparent, always-on-top,
// non-activating, click-through. Hosts the companion cursor + response
// bubble + waveform. Everything visual lives in overlay.html / overlay.ts;
// this module owns the window-level concerns (size to virtual desktop,
// ignore-cursor-events, show/hide, emit driving events).

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

pub fn ensure_installed(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("overlay") {
        configure_overlay_window(&window)?;
        // Stay hidden until a session starts.
        let _ = window.hide();
    }
    Ok(())
}

fn configure_overlay_window(window: &WebviewWindow) -> tauri::Result<()> {
    // Span the full virtual desktop so cursor coordinates from any monitor
    // land inside the overlay's coordinate space without translation.
    if let Ok(monitors) = window.available_monitors() {
        let mut min_x: i32 = i32::MAX;
        let mut min_y: i32 = i32::MAX;
        let mut max_x: i32 = i32::MIN;
        let mut max_y: i32 = i32::MIN;
        for monitor in &monitors {
            let position = monitor.position();
            let size = monitor.size();
            min_x = min_x.min(position.x);
            min_y = min_y.min(position.y);
            max_x = max_x.max(position.x + size.width as i32);
            max_y = max_y.max(position.y + size.height as i32);
        }
        if max_x > min_x && max_y > min_y {
            let _ = window.set_position(PhysicalPosition::new(min_x, min_y));
            let _ = window.set_size(PhysicalSize::new(
                (max_x - min_x) as u32,
                (max_y - min_y) as u32,
            ));
        }
    }

    // Always on top, no focus stealing, no taskbar.
    let _ = window.set_always_on_top(true);
    let _ = window.set_skip_taskbar(true);
    // Click-through: the overlay shouldn't intercept any mouse events. The
    // webview content is pointer-events: none on every element, but we
    // also need the OS-level window flag for cases where the webview hasn't
    // booted yet or a child happens to claim hit-testing.
    let _ = window.set_ignore_cursor_events(true);

    // On macOS join all spaces so the overlay floats over Mission Control
    // and fullscreen apps. Tauri exposes this through a platform-only
    // helper; gate behind the cfg so non-Mac builds still link.
    #[cfg(target_os = "macos")]
    {
        if let Ok(ns_window) = window.ns_window() {
            unsafe {
                use objc2::msg_send;
                use objc2::runtime::AnyObject;
                let ns_window_object = ns_window as *mut AnyObject;
                // NSWindowCollectionBehaviorCanJoinAllSpaces (1<<0) +
                // NSWindowCollectionBehaviorFullScreenAuxiliary (1<<8) so
                // the overlay rides on top of every space and fullscreen
                // window without becoming a fullscreen app itself.
                let collection_behavior: u64 = (1 << 0) | (1 << 8);
                let _: () = msg_send![ns_window_object, setCollectionBehavior: collection_behavior];
                // Set level above floating panel but below screen-saver:
                // NSStatusWindowLevel == 25.
                let _: () = msg_send![ns_window_object, setLevel: 25_i64];
            }
        }
    }

    Ok(())
}

#[derive(Serialize, Clone)]
pub struct CursorFlyTo {
    pub x: f64,
    pub y: f64,
    pub label: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ResponseBubble {
    pub text: String,
    #[serde(rename = "appendMode")]
    pub append_mode: bool,
    #[serde(rename = "anchorX", skip_serializing_if = "Option::is_none")]
    pub anchor_x: Option<f64>,
    #[serde(rename = "anchorY", skip_serializing_if = "Option::is_none")]
    pub anchor_y: Option<f64>,
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.show();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.hide();
    }
}

pub fn fly_cursor_to(app: &AppHandle, x: f64, y: f64, label: Option<String>) {
    show(app);
    let _ = app.emit("overlay/cursor_fly_to", CursorFlyTo { x, y, label });
}

pub fn hide_cursor(app: &AppHandle) {
    let _ = app.emit::<()>("overlay/cursor_hide", ());
}

pub fn show_response(app: &AppHandle, text: String, append_mode: bool, anchor: Option<(f64, f64)>) {
    show(app);
    let _ = app.emit(
        "overlay/response_show",
        ResponseBubble {
            text,
            append_mode,
            anchor_x: anchor.map(|(x, _)| x),
            anchor_y: anchor.map(|(_, y)| y),
        },
    );
}

pub fn hide_response(app: &AppHandle) {
    let _ = app.emit::<()>("overlay/response_hide", ());
}

pub fn set_speaking(app: &AppHandle, speaking: bool) {
    let _ = app.emit("overlay/waveform", SpeakingPayload { speaking });
}

#[derive(Serialize, Clone)]
struct SpeakingPayload {
    speaking: bool,
}

// Avoid unused-import warnings on platforms where the monitor-rectangle
// fallback path isn't exercised.
#[allow(dead_code)]
fn _retain_imports(window: &WebviewWindow) {
    let _ = window.set_position(LogicalPosition::new(0.0, 0.0));
    let _ = window.set_size(LogicalSize::new(0.0, 0.0));
}

// Tauri commands so the panel or the workflow runner can drive the overlay
// without owning the AppHandle.
#[tauri::command]
pub fn overlay_show(app: AppHandle) {
    show(&app);
}

#[tauri::command]
pub fn overlay_hide(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
pub fn overlay_fly_cursor_to(app: AppHandle, x: f64, y: f64, label: Option<String>) {
    fly_cursor_to(&app, x, y, label);
}

#[tauri::command]
pub fn overlay_show_response(
    app: AppHandle,
    text: String,
    append_mode: Option<bool>,
    anchor_x: Option<f64>,
    anchor_y: Option<f64>,
) {
    let anchor = match (anchor_x, anchor_y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    show_response(&app, text, append_mode.unwrap_or(false), anchor);
}

#[tauri::command]
pub fn overlay_hide_response(app: AppHandle) {
    hide_response(&app);
}

#[tauri::command]
pub fn overlay_set_speaking(app: AppHandle, speaking: bool) {
    set_speaking(&app, speaking);
}
