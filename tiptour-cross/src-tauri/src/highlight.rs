// Focus highlight brush — listen-only OS-level chord detector.
//
// Hold Ctrl+Shift and drag the mouse to paint a freeform highlight region.
// The captured polyline + bounding rect becomes "this area" context for
// Gemini (mirrors the Swift app's `GlobalHighlightShortcutMonitor` +
// `FocusHighlightContext`). The hook never blocks input; it only observes.
//
// macOS: a `CGEventTap` watching modifier + mouse-move events.
// Windows: paired `SetWindowsHookExW` hooks for low-level keyboard + mouse.
// Other platforms (Linux dev hosts): no-op so `cargo check` stays green.

use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HighlightRect {
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightContext {
    pub bounding_rect: HighlightRect,
    pub points: Vec<(f64, f64)>,
    pub anchor_point: Option<(f64, f64)>,
    // Optional AX text range captured under the highlight centroid at
    // paint-end. The workflow runner restores this range on the focused
    // element before pasting so "rewrite this" lands inside the
    // originally-highlighted run even if the user has moved focus away.
    //
    // None on non-text highlights where the platform's range query
    // returns nothing. None on Windows by design: UIA text ranges are
    // opaque cookies without integer offsets, so we instead capture the
    // text *content* under the highlight (see `armed_text_content`) and
    // restore by find-and-select rather than range-replay.
    pub armed_element_role: Option<String>,
    pub armed_text_range_location: Option<i64>,
    pub armed_text_range_length: Option<i64>,
    // Plain-text snapshot of the word/range under the highlight centroid.
    // macOS leaves this as None (the integer range above is the canonical
    // path there). Windows fills it via UIA TextPattern.range_from_point
    // → expand_to_enclosing_unit(Word) → get_text, and the runner uses it
    // to find-and-select before pasting on the Windows SetSelectedText
    // path.
    pub armed_text_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightPointEvent {
    pub x: f64,
    pub y: f64,
    pub is_first_point: bool,
}

#[derive(Default)]
pub struct HighlightState {
    pub points: Vec<(f64, f64)>,
    pub is_painting: bool,
    pub last_completed: Option<HighlightContext>,
    pub app_handle: Option<AppHandle>,
}

pub static HIGHLIGHT_STATE: Lazy<Arc<Mutex<HighlightState>>> =
    Lazy::new(|| Arc::new(Mutex::new(HighlightState::default())));

// Minimum points + minimum bounding-box dimension required to publish a
// highlight. Mirrors the `>= 2 points`, `width/height > 4` checks in the
// Swift `FocusHighlightContext` initializer so a stray Ctrl+Shift tap
// doesn't surface a degenerate region.
const MINIMUM_POINTS_FOR_VALID_HIGHLIGHT: usize = 2;
const MINIMUM_DIMENSION_PIXELS_FOR_VALID_HIGHLIGHT: f64 = 4.0;
const BOUNDING_RECT_INSET_PIXELS: f64 = 12.0;

/// Install the listen-only OS hook. Safe to call once on app startup.
pub fn start_listening(app_handle: AppHandle) {
    HIGHLIGHT_STATE.lock().app_handle = Some(app_handle);

    #[cfg(target_os = "macos")]
    {
        macos::install_event_tap();
    }
    #[cfg(target_os = "windows")]
    {
        windows_hooks::install_hooks();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // No global hook on Linux dev hosts; the state still exists so the
        // Tauri command surface compiles and returns `None`.
    }
}

/// Latest completed highlight, or in-progress paint if the user is still
/// holding Ctrl+Shift. `None` if nothing has been painted yet.
pub fn current_highlight_context() -> Option<HighlightContext> {
    let state = HIGHLIGHT_STATE.lock();
    if state.is_painting && state.points.len() >= MINIMUM_POINTS_FOR_VALID_HIGHLIGHT {
        return build_context_from_points(&state.points);
    }
    state.last_completed.clone()
}

pub fn clear() {
    let mut state = HIGHLIGHT_STATE.lock();
    state.points.clear();
    state.is_painting = false;
    state.last_completed = None;
}

#[tauri::command]
pub fn get_current_highlight_context() -> Option<HighlightContext> {
    current_highlight_context()
}

// Shared transition handlers invoked by the platform hooks. Kept in the
// parent module so both backends route through identical state machinery.

fn handle_paint_begin(x: f64, y: f64) {
    let app_handle_clone;
    {
        let mut state = HIGHLIGHT_STATE.lock();
        state.points.clear();
        state.points.push((x, y));
        state.is_painting = true;
        app_handle_clone = state.app_handle.clone();
    }
    if let Some(app_handle) = app_handle_clone {
        let _ = app_handle.emit(
            "highlight_point",
            HighlightPointEvent {
                x,
                y,
                is_first_point: true,
            },
        );
    }
}

fn handle_paint_move(x: f64, y: f64) {
    let app_handle_clone;
    {
        let mut state = HIGHLIGHT_STATE.lock();
        if !state.is_painting {
            return;
        }
        // Coalesce identical points so the SVG path stays cheap when the
        // user pauses mid-paint; the CGEventTap fires .mouseMoved bursts
        // even on idle hands.
        if let Some(&(last_x, last_y)) = state.points.last() {
            if (last_x - x).abs() < 0.5 && (last_y - y).abs() < 0.5 {
                return;
            }
        }
        state.points.push((x, y));
        app_handle_clone = state.app_handle.clone();
    }
    if let Some(app_handle) = app_handle_clone {
        let _ = app_handle.emit(
            "highlight_point",
            HighlightPointEvent {
                x,
                y,
                is_first_point: false,
            },
        );
    }
}

fn handle_paint_end() {
    let (app_handle_clone, completed_context_clone) = {
        let mut state = HIGHLIGHT_STATE.lock();
        if !state.is_painting {
            return;
        }
        state.is_painting = false;
        let completed = build_context_from_points(&state.points);
        state.last_completed = completed.clone();
        (state.app_handle.clone(), completed)
    };
    if let (Some(app_handle), Some(context)) = (app_handle_clone, completed_context_clone) {
        let _ = app_handle.emit("highlight_painted", context);
    }
}

fn build_context_from_points(points: &[(f64, f64)]) -> Option<HighlightContext> {
    if points.len() < MINIMUM_POINTS_FOR_VALID_HIGHLIGHT {
        return None;
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &(x, y) in points {
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if x > max_x {
            max_x = x;
        }
        if y > max_y {
            max_y = y;
        }
    }

    let origin_x = min_x - BOUNDING_RECT_INSET_PIXELS;
    let origin_y = min_y - BOUNDING_RECT_INSET_PIXELS;
    let width = (max_x - min_x) + 2.0 * BOUNDING_RECT_INSET_PIXELS;
    let height = (max_y - min_y) + 2.0 * BOUNDING_RECT_INSET_PIXELS;

    if width < MINIMUM_DIMENSION_PIXELS_FOR_VALID_HIGHLIGHT
        || height < MINIMUM_DIMENSION_PIXELS_FOR_VALID_HIGHLIGHT
    {
        return None;
    }

    let anchor_point = points.last().copied();

    // Geometric centroid of the painted polyline — used as the sample
    // point for AXRangeForPosition so the armed range tracks the visual
    // center of what the user actually painted (not just the last hover).
    let mut centroid_x = 0.0;
    let mut centroid_y = 0.0;
    for &(x, y) in points {
        centroid_x += x;
        centroid_y += y;
    }
    centroid_x /= points.len() as f64;
    centroid_y /= points.len() as f64;

    let ArmedTextCapture {
        element_role,
        range_location,
        range_length,
        text_content,
    } = capture_armed_text_at_point(centroid_x, centroid_y);

    Some(HighlightContext {
        bounding_rect: HighlightRect {
            origin_x,
            origin_y,
            width,
            height,
        },
        points: points.to_vec(),
        anchor_point,
        armed_element_role: element_role,
        armed_text_range_location: range_location,
        armed_text_range_length: range_length,
        armed_text_content: text_content,
    })
}

/// Result of querying the focused element for the armed text under the
/// painted highlight centroid. macOS fills the integer range fields and
/// leaves `text_content` None. Windows fills `text_content` and leaves the
/// range fields None — UIA text ranges are opaque cookies, not addressable
/// offsets, so the runner restores via find-and-select on Windows.
pub struct ArmedTextCapture {
    pub element_role: Option<String>,
    pub range_location: Option<i64>,
    pub range_length: Option<i64>,
    pub text_content: Option<String>,
}

#[cfg(target_os = "macos")]
fn capture_armed_text_at_point(sample_x: f64, sample_y: f64) -> ArmedTextCapture {
    let (element_role, range_location, range_length) =
        capture_armed_text_range_at_point_macos(sample_x, sample_y);
    ArmedTextCapture {
        element_role,
        range_location,
        range_length,
        text_content: None,
    }
}

#[cfg(target_os = "windows")]
fn capture_armed_text_at_point(sample_x: f64, sample_y: f64) -> ArmedTextCapture {
    use uiautomation::patterns::UITextPattern;
    use uiautomation::types::{Point as UiaPoint, TextUnit};
    use uiautomation::UIAutomation;

    let automation = match UIAutomation::new() {
        Ok(automation) => automation,
        Err(_) => return ArmedTextCapture::empty(),
    };
    let focused_element = match automation.get_focused_element() {
        Ok(element) => element,
        Err(_) => return ArmedTextCapture::empty(),
    };
    let element_role = focused_element
        .get_localized_control_type()
        .ok()
        .filter(|role| !role.is_empty());

    let text_pattern: UITextPattern = match focused_element.get_pattern::<UITextPattern>() {
        Ok(pattern) => pattern,
        Err(_) => {
            return ArmedTextCapture {
                element_role,
                range_location: None,
                range_length: None,
                text_content: None,
            };
        }
    };

    let sample_point = UiaPoint::new(sample_x as i32, sample_y as i32);
    let range = match text_pattern.range_from_point(sample_point) {
        Ok(range) => range,
        Err(_) => {
            return ArmedTextCapture {
                element_role,
                range_location: None,
                range_length: None,
                text_content: None,
            };
        }
    };

    // Expand to the enclosing word so a single-click position becomes a
    // meaningful, find-and-selectable run of text instead of a zero-length
    // caret. Truncating with -1 returns the entire word.
    let _ = range.expand_to_enclosing_unit(TextUnit::Word);
    let text_content = range
        .get_text(-1)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty());

    ArmedTextCapture {
        element_role,
        range_location: None,
        range_length: None,
        text_content,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn capture_armed_text_at_point(_sample_x: f64, _sample_y: f64) -> ArmedTextCapture {
    ArmedTextCapture::empty()
}

impl ArmedTextCapture {
    fn empty() -> Self {
        Self {
            element_role: None,
            range_location: None,
            range_length: None,
            text_content: None,
        }
    }
}

#[cfg(target_os = "macos")]
fn capture_armed_text_range_at_point_macos(
    sample_x: f64,
    sample_y: f64,
) -> (Option<String>, Option<i64>, Option<i64>) {
    use accessibility_sys::{
        kAXFocusedUIElementAttribute, kAXRangeForPositionParameterizedAttribute,
        kAXRoleAttribute, kAXValueTypeCFRange, AXUIElementCopyAttributeValue,
        AXUIElementCopyParameterizedAttributeValue, AXUIElementCreateSystemWide, AXUIElementRef,
        AXUIElementSetMessagingTimeout, AXValueCreate, AXValueGetType, AXValueGetValue, AXValueRef,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::geometry::CGPoint;

    #[repr(C)]
    struct CoreFoundationRange {
        location: core::ffi::c_long,
        length: core::ffi::c_long,
    }

    unsafe {
        let system_wide_element: AXUIElementRef = AXUIElementCreateSystemWide();
        if system_wide_element.is_null() {
            return (None, None, None);
        }
        AXUIElementSetMessagingTimeout(system_wide_element, 0.4);
        let focused_attribute = CFString::new(kAXFocusedUIElementAttribute);
        let mut focused_raw: CFTypeRef = std::ptr::null_mut();
        let focused_status = AXUIElementCopyAttributeValue(
            system_wide_element,
            focused_attribute.as_concrete_TypeRef(),
            &mut focused_raw,
        );
        CFRelease(system_wide_element as CFTypeRef);
        if focused_status != 0 || focused_raw.is_null() {
            return (None, None, None);
        }
        let focused_element_ref = focused_raw as AXUIElementRef;
        AXUIElementSetMessagingTimeout(focused_element_ref, 0.4);

        // Pull the role first — gives the caller something to gate
        // unconditional range writes on, and helps debugging when the
        // armed range later fails to round-trip.
        let mut role_value: Option<String> = None;
        let role_attribute = CFString::new(kAXRoleAttribute);
        let mut role_raw: CFTypeRef = std::ptr::null_mut();
        let role_status = AXUIElementCopyAttributeValue(
            focused_element_ref,
            role_attribute.as_concrete_TypeRef(),
            &mut role_raw,
        );
        if role_status == 0 && !role_raw.is_null() {
            let cf_string_ref = role_raw as CFStringRef;
            let role_cf = CFString::wrap_under_get_rule(cf_string_ref);
            role_value = Some(role_cf.to_string());
            CFRelease(role_raw);
        }

        // Build a CGPoint AXValue at the painted centroid and ask the
        // focused element which text range covers that screen position.
        let mut cg_point = CGPoint::new(sample_x, sample_y);
        let point_value_ref = AXValueCreate(
            accessibility_sys::kAXValueTypeCGPoint,
            &mut cg_point as *mut CGPoint as *mut std::ffi::c_void,
        );
        if point_value_ref.is_null() {
            CFRelease(focused_raw);
            return (role_value, None, None);
        }

        let range_for_position_attribute = CFString::new(kAXRangeForPositionParameterizedAttribute);
        let mut range_raw: CFTypeRef = std::ptr::null_mut();
        let range_status = AXUIElementCopyParameterizedAttributeValue(
            focused_element_ref,
            range_for_position_attribute.as_concrete_TypeRef(),
            point_value_ref as CFTypeRef,
            &mut range_raw,
        );
        CFRelease(point_value_ref as CFTypeRef);
        CFRelease(focused_raw);
        if range_status != 0 || range_raw.is_null() {
            return (role_value, None, None);
        }

        let value_ref = range_raw as AXValueRef;
        if AXValueGetType(value_ref) != kAXValueTypeCFRange {
            CFRelease(range_raw);
            return (role_value, None, None);
        }
        let mut cf_range = CoreFoundationRange {
            location: 0,
            length: 0,
        };
        let got_value = AXValueGetValue(
            value_ref,
            kAXValueTypeCFRange,
            &mut cf_range as *mut CoreFoundationRange as *mut std::ffi::c_void,
        );
        CFRelease(range_raw);
        if !got_value {
            return (role_value, None, None);
        }
        (
            role_value,
            Some(cf_range.location as i64),
            Some(cf_range.length as i64),
        )
    }
}

// ---------------- macOS implementation ----------------

#[cfg(target_os = "macos")]
mod macos {
    use super::{handle_paint_begin, handle_paint_end, handle_paint_move};
    use core_foundation::base::TCFType;
    use core_foundation::mach_port::CFMachPort;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, EventField,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    // CGEventFlags bit positions matching Quartz's `kCGEventFlagMaskShift`
    // / `kCGEventFlagMaskControl`. We avoid AppKit's NSEvent here so the
    // hook stays in pure CoreGraphics-land.
    const CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    const CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
    const CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    const CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

    static HOOK_IS_INSTALLED: AtomicBool = AtomicBool::new(false);

    pub fn install_event_tap() {
        if HOOK_IS_INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }

        // The CGEventTap callback must execute on a thread with an active
        // CFRunLoop. We spawn a dedicated OS thread, install the tap there,
        // and let that thread own the run loop for the life of the app.
        thread::Builder::new()
            .name("tiptour-highlight-tap".to_string())
            .spawn(run_event_tap_thread)
            .expect("failed to spawn highlight CGEventTap thread");
    }

    fn run_event_tap_thread() {
        let monitored_event_types = vec![
            CGEventType::FlagsChanged,
            CGEventType::MouseMoved,
            CGEventType::LeftMouseDragged,
            CGEventType::RightMouseDragged,
        ];

        let event_tap = match CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            monitored_event_types,
            handle_cg_event,
        ) {
            Ok(tap) => tap,
            Err(_) => {
                eprintln!("highlight: CGEventTap::new failed (accessibility permission?)");
                HOOK_IS_INSTALLED.store(false, Ordering::SeqCst);
                return;
            }
        };

        // CGEventTap's Drop will tear the tap down — keep it alive by
        // anchoring it inside this thread's stack frame for the run loop's
        // lifetime.
        let mach_port: &CFMachPort = event_tap.mach_port();
        let run_loop_source = match mach_port.create_runloop_source(0) {
            Ok(source) => source,
            Err(_) => {
                eprintln!("highlight: create_runloop_source failed");
                return;
            }
        };

        let current_run_loop = CFRunLoop::get_current();
        unsafe {
            current_run_loop.add_source(&run_loop_source, kCFRunLoopCommonModes);
        }
        event_tap.enable();

        CFRunLoop::run_current();
        // Keep `event_tap` alive until the run loop exits so the tap's
        // backing resources aren't dropped prematurely.
        drop(event_tap);
    }

    fn handle_cg_event(
        _proxy: core_graphics::event::CGEventTapProxy,
        event_type: CGEventType,
        event: &CGEvent,
    ) -> Option<CGEvent> {
        if matches!(
            event_type,
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
        ) {
            // The system disabled our tap (commonly under load). The
            // outer enable on the next start_listening() call would relight
            // it; in practice this rarely fires for listen-only taps.
            return None;
        }

        let flags = event.get_flags().bits();
        let is_ctrl_shift_chord_held = (flags & CG_EVENT_FLAG_MASK_CONTROL) != 0
            && (flags & CG_EVENT_FLAG_MASK_SHIFT) != 0
            && (flags & CG_EVENT_FLAG_MASK_ALTERNATE) == 0
            && (flags & CG_EVENT_FLAG_MASK_COMMAND) == 0;

        let location_x =
            event.get_integer_value_field(EventField::MOUSE_EVENT_X) as f64;
        let location_y =
            event.get_integer_value_field(EventField::MOUSE_EVENT_Y) as f64;
        // FlagsChanged events don't carry mouse fields; fall back to the
        // generic CGEvent location accessor in that case.
        let mouse_point = if location_x == 0.0 && location_y == 0.0 {
            event.location()
        } else {
            core_graphics::geometry::CGPoint::new(location_x, location_y)
        };

        let is_already_painting = super::HIGHLIGHT_STATE.lock().is_painting;
        let is_mouse_movement_event = matches!(
            event_type,
            CGEventType::MouseMoved
                | CGEventType::LeftMouseDragged
                | CGEventType::RightMouseDragged
        );

        if is_ctrl_shift_chord_held && !is_already_painting {
            handle_paint_begin(mouse_point.x, mouse_point.y);
        } else if !is_ctrl_shift_chord_held && is_already_painting {
            handle_paint_end();
        } else if is_ctrl_shift_chord_held && is_mouse_movement_event {
            handle_paint_move(mouse_point.x, mouse_point.y);
        }

        None
    }
}

// ---------------- Windows implementation ----------------

#[cfg(target_os = "windows")]
mod windows_hooks {
    use super::{handle_paint_begin, handle_paint_end, handle_paint_move};
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, HHOOK, KBDLLHOOKSTRUCT, MSG,
        MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MOUSEMOVE,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    // Virtual-key codes for the Ctrl + Shift chord. We accept either left
    // or right modifier so the user isn't forced onto one side.
    const VK_LCONTROL: u32 = 0xA2;
    const VK_RCONTROL: u32 = 0xA3;
    const VK_LSHIFT: u32 = 0xA0;
    const VK_RSHIFT: u32 = 0xA1;
    const VK_LMENU: u32 = 0xA4; // Left Alt
    const VK_RMENU: u32 = 0xA5; // Right Alt
    const VK_LWIN: u32 = 0x5B;
    const VK_RWIN: u32 = 0x5C;

    static HOOK_IS_INSTALLED: AtomicBool = AtomicBool::new(false);
    static IS_CONTROL_HELD: AtomicBool = AtomicBool::new(false);
    static IS_SHIFT_HELD: AtomicBool = AtomicBool::new(false);
    static IS_ALT_OR_WIN_HELD: AtomicBool = AtomicBool::new(false);
    static IS_PAINTING_LOCAL_FLAG: AtomicBool = AtomicBool::new(false);

    pub fn install_hooks() {
        if HOOK_IS_INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }

        // Windows low-level hooks must be installed on a thread that runs
        // a message pump (GetMessage loop). Spawn a dedicated thread; the
        // hooks dispatch through the kernel back to our callbacks.
        thread::Builder::new()
            .name("tiptour-highlight-hooks".to_string())
            .spawn(run_message_pump_thread)
            .expect("failed to spawn highlight Windows hook thread");
    }

    fn run_message_pump_thread() {
        unsafe {
            let keyboard_hook: HHOOK =
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0) {
                    Ok(handle) => handle,
                    Err(error) => {
                        eprintln!("highlight: SetWindowsHookExW(WH_KEYBOARD_LL) failed: {error:?}");
                        return;
                    }
                };
            let mouse_hook: HHOOK =
                match SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), None, 0) {
                    Ok(handle) => handle,
                    Err(error) => {
                        eprintln!("highlight: SetWindowsHookExW(WH_MOUSE_LL) failed: {error:?}");
                        return;
                    }
                };

            // Standard message pump — hooks call into our procs from the
            // kernel; the pump just needs to be alive for delivery.
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                // No translate/dispatch needed for hook delivery.
            }

            let _ = keyboard_hook;
            let _ = mouse_hook;
        }
    }

    unsafe extern "system" fn low_level_keyboard_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let kbd_struct = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
            let virtual_key_code = kbd_struct.vkCode;
            let message_type = w_param.0 as u32;
            let is_key_down = message_type == WM_KEYDOWN || message_type == WM_SYSKEYDOWN;
            let is_key_up = message_type == WM_KEYUP || message_type == WM_SYSKEYUP;

            if is_key_down || is_key_up {
                match virtual_key_code {
                    VK_LCONTROL | VK_RCONTROL => {
                        IS_CONTROL_HELD.store(is_key_down, Ordering::SeqCst)
                    }
                    VK_LSHIFT | VK_RSHIFT => IS_SHIFT_HELD.store(is_key_down, Ordering::SeqCst),
                    VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN => {
                        IS_ALT_OR_WIN_HELD.store(is_key_down, Ordering::SeqCst)
                    }
                    _ => {}
                }

                update_paint_chord_state(None);
            }
        }
        CallNextHookEx(HHOOK(null_mut()), n_code, w_param, l_param)
    }

    unsafe extern "system" fn low_level_mouse_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let message_type = w_param.0 as u32;
            if message_type == WM_MOUSEMOVE {
                let mouse_struct = &*(l_param.0 as *const MSLLHOOKSTRUCT);
                let mouse_point = (mouse_struct.pt.x as f64, mouse_struct.pt.y as f64);
                update_paint_chord_state(Some(mouse_point));
            }
        }
        CallNextHookEx(HHOOK(null_mut()), n_code, w_param, l_param)
    }

    fn update_paint_chord_state(latest_mouse_point: Option<(f64, f64)>) {
        let is_chord_held = IS_CONTROL_HELD.load(Ordering::SeqCst)
            && IS_SHIFT_HELD.load(Ordering::SeqCst)
            && !IS_ALT_OR_WIN_HELD.load(Ordering::SeqCst);
        let was_painting = IS_PAINTING_LOCAL_FLAG.load(Ordering::SeqCst);

        let (point_x, point_y) =
            latest_mouse_point.unwrap_or_else(|| current_cursor_position().unwrap_or((0.0, 0.0)));

        if is_chord_held && !was_painting {
            IS_PAINTING_LOCAL_FLAG.store(true, Ordering::SeqCst);
            handle_paint_begin(point_x, point_y);
        } else if !is_chord_held && was_painting {
            IS_PAINTING_LOCAL_FLAG.store(false, Ordering::SeqCst);
            handle_paint_end();
        } else if is_chord_held && latest_mouse_point.is_some() {
            handle_paint_move(point_x, point_y);
        }
    }

    fn current_cursor_position() -> Option<(f64, f64)> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut cursor_point = POINT::default();
        unsafe {
            if GetCursorPos(&mut cursor_point).is_ok() {
                Some((cursor_point.x as f64, cursor_point.y as f64))
            } else {
                None
            }
        }
    }
}
