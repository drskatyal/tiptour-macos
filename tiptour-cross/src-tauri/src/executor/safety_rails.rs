// Workflow-runner safety rails: modal-dialog detection, app-switch detection,
// and post-action UI-settle polling. Ported from the Swift app's
// `WorkflowRunner` guardrails so autopilot pauses cleanly when the system
// pops a modal, when the user Cmd-Tabs away mid-plan, or before the runner
// grounds the next step against a UI tree that hasn't repainted yet.
//
// All three rails are best-effort: when the underlying AX/UIA backend can't
// answer, we fail open (return "no modal", "no app switch", flat sleep) so
// the runner keeps making forward progress instead of stalling.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::capabilities::fingerprint::fingerprint;
use crate::executor::workflow_plan::StepType;
use crate::grounding;

// How long the runner waits when a step's outcome depends on the foreground
// UI mutating (button click, key chord, paste). We poll the fingerprint at
// FINGERPRINT_SAMPLE_INTERVAL and return once three consecutive samples
// match, capped at SETTLE_MAX_ELAPSED.
const SETTLE_MAX_ELAPSED: Duration = Duration::from_millis(600);
const FINGERPRINT_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const STABLE_SAMPLE_COUNT_REQUIRED: usize = 3;
// Steps that don't drive an AX/UIA repaint (launching an app, opening a
// URL, scrolling) just need a flat pause — the new window/scroll usually
// finishes painting well within 350ms and we don't want to spin the
// grounding backend for no reason.
const FLAT_SETTLE_DURATION: Duration = Duration::from_millis(350);

// Cache the modal-dialog answer for a short window so the runner can poll
// this between every step without hammering AX/UIA. 100ms matches the
// human-perceptible reaction window — a modal that pops up a tick after
// the cache fill will be caught on the next step.
const MODAL_CACHE_DURATION: Duration = Duration::from_millis(100);

static MODAL_DIALOG_CACHE: Lazy<Mutex<Option<(Instant, bool)>>> =
    Lazy::new(|| Mutex::new(None));

/// True when the foreground app is currently showing a modal dialog/sheet
/// that should pause the workflow runner. Cached for MODAL_CACHE_DURATION
/// so back-to-back step transitions don't re-query AX/UIA.
pub fn modal_dialog_blocking() -> bool {
    {
        let cache = MODAL_DIALOG_CACHE.lock().unwrap();
        if let Some((cached_at, cached_value)) = *cache {
            if cached_at.elapsed() < MODAL_CACHE_DURATION {
                return cached_value;
            }
        }
    }

    let detected = detect_modal_dialog();
    let mut cache = MODAL_DIALOG_CACHE.lock().unwrap();
    *cache = Some((Instant::now(), detected));
    detected
}

/// Returns the current frontmost-app pid. Delegates to the grounding
/// target-app helper so the same hover-then-frontmost policy as the rest
/// of the app applies here.
pub fn current_frontmost_pid() -> Option<i32> {
    grounding::target_app::current_target_app().map(|target_app| target_app.process_id)
}

/// True when the frontmost app pid no longer matches the pid we captured
/// at workflow start AND the new frontmost app isn't TipTour itself. The
/// TipTour-pid filter prevents a tray click or panel focus event from
/// tripping the runner's pause path.
pub fn user_switched_away_from(original_pid: Option<i32>) -> bool {
    let workflow_starting_pid = match original_pid {
        Some(pid) => pid,
        None => return false,
    };
    let current_pid = match current_frontmost_pid() {
        Some(pid) => pid,
        None => return false,
    };
    if current_pid == workflow_starting_pid {
        return false;
    }
    let own_process_id = std::process::id() as i32;
    if current_pid == own_process_id {
        return false;
    }
    true
}

/// Pause until the foreground UI tree fingerprint is stable, or the
/// elapsed budget is exhausted. For step types where no AX/UIA repaint is
/// expected (LaunchApp, OpenUrl, Scroll) this is a flat sleep — polling
/// the tree wouldn't tell us anything actionable.
pub async fn settle_until_ui_stable(step_type: StepType) {
    if !step_type_should_poll_fingerprint(step_type) {
        tokio::time::sleep(FLAT_SETTLE_DURATION).await;
        return;
    }

    let polling_started_at = Instant::now();
    let mut consecutive_matching_samples: usize = 0;
    let mut last_fingerprint: Option<String> = None;

    while polling_started_at.elapsed() < SETTLE_MAX_ELAPSED {
        tokio::time::sleep(FINGERPRINT_SAMPLE_INTERVAL).await;
        let current_fingerprint = match sample_foreground_fingerprint() {
            Some(value) => value,
            None => {
                // No AX/UIA backend (e.g. Linux dev host) — fall back to a
                // flat sleep so the runner doesn't busy-loop.
                tokio::time::sleep(FLAT_SETTLE_DURATION).await;
                return;
            }
        };

        if last_fingerprint.as_deref() == Some(current_fingerprint.as_str()) {
            consecutive_matching_samples += 1;
            if consecutive_matching_samples >= STABLE_SAMPLE_COUNT_REQUIRED {
                return;
            }
        } else {
            consecutive_matching_samples = 1;
            last_fingerprint = Some(current_fingerprint);
        }
    }
}

fn step_type_should_poll_fingerprint(step_type: StepType) -> bool {
    matches!(
        step_type,
        StepType::Click
            | StepType::RightClick
            | StepType::DoubleClick
            | StepType::KeyboardShortcut
            | StepType::PressKey
            | StepType::Type
            | StepType::SetValue
    )
}

fn sample_foreground_fingerprint() -> Option<String> {
    let mut grounding_provider = grounding::make_grounding_provider();
    let snapshot = grounding_provider.snapshot_foreground_app().ok()?;
    Some(fingerprint(&snapshot))
}

// ---------------- Modal detection ----------------

#[cfg(target_os = "macos")]
fn detect_modal_dialog() -> bool {
    use accessibility_sys::{
        kAXChildrenAttribute, kAXRoleAttribute, kAXSubroleAttribute, kAXWindowsAttribute, AXError,
        AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef,
        AXUIElementSetMessagingTimeout,
    };
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFRelease, CFType, CFTypeID, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};

    const AX_DIALOG_SUBROLES: &[&str] = &["AXDialog", "AXSystemDialog"];
    const AX_SHEET_ROLE: &str = "AXSheet";
    const AX_DIALOG_QUERY_TIMEOUT_SECONDS: f32 = 0.4;

    unsafe fn read_string(element: AXUIElementRef, attribute_name: &str) -> Option<String> {
        let attribute = CFString::new(attribute_name);
        let mut raw: CFTypeRef = std::ptr::null_mut();
        let status: AXError =
            AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut raw);
        if status != 0 || raw.is_null() {
            return None;
        }
        let type_id: CFTypeID = core_foundation::base::CFGetTypeID(raw);
        if type_id != CFString::type_id() {
            CFRelease(raw);
            return None;
        }
        let value: CFString = CFString::wrap_under_create_rule(raw as CFStringRef);
        Some(value.to_string())
    }

    unsafe fn child_has_sheet(window_element: AXUIElementRef) -> bool {
        let children_attribute = CFString::new(kAXChildrenAttribute);
        let mut children_raw: CFTypeRef = std::ptr::null_mut();
        let status: AXError = AXUIElementCopyAttributeValue(
            window_element,
            children_attribute.as_concrete_TypeRef(),
            &mut children_raw,
        );
        if status != 0 || children_raw.is_null() {
            return false;
        }
        let children_array: CFArray<CFType> =
            CFArray::wrap_under_create_rule(children_raw as CFArrayRef);
        for child_index in 0..children_array.len() {
            if let Some(child_cf_type) = children_array.get(child_index) {
                let child_element_ref: AXUIElementRef =
                    child_cf_type.as_CFTypeRef() as AXUIElementRef;
                if let Some(role) = read_string(child_element_ref, kAXRoleAttribute) {
                    if role == AX_SHEET_ROLE {
                        return true;
                    }
                }
            }
        }
        false
    }

    let frontmost_pid = match current_frontmost_pid() {
        Some(pid) => pid,
        None => return false,
    };

    unsafe {
        let application_element: AXUIElementRef = AXUIElementCreateApplication(frontmost_pid);
        if application_element.is_null() {
            return false;
        }
        AXUIElementSetMessagingTimeout(application_element, AX_DIALOG_QUERY_TIMEOUT_SECONDS);

        let windows_attribute = CFString::new(kAXWindowsAttribute);
        let mut windows_raw: CFTypeRef = std::ptr::null_mut();
        let status: AXError = AXUIElementCopyAttributeValue(
            application_element,
            windows_attribute.as_concrete_TypeRef(),
            &mut windows_raw,
        );
        if status != 0 || windows_raw.is_null() {
            CFRelease(application_element as CFTypeRef);
            return false;
        }

        let windows_array: CFArray<CFType> =
            CFArray::wrap_under_create_rule(windows_raw as CFArrayRef);

        let mut found_modal = false;
        for window_index in 0..windows_array.len() {
            let window_cf_type = match windows_array.get(window_index) {
                Some(value) => value,
                None => continue,
            };
            let window_element_ref: AXUIElementRef =
                window_cf_type.as_CFTypeRef() as AXUIElementRef;

            if let Some(subrole) = read_string(window_element_ref, kAXSubroleAttribute) {
                if AX_DIALOG_SUBROLES.contains(&subrole.as_str()) {
                    found_modal = true;
                    break;
                }
            }
            if child_has_sheet(window_element_ref) {
                found_modal = true;
                break;
            }
        }
        CFRelease(application_element as CFTypeRef);
        found_modal
    }
}

#[cfg(target_os = "windows")]
fn detect_modal_dialog() -> bool {
    use uiautomation::controls::ControlType;
    use uiautomation::patterns::UIWindowPattern;
    use uiautomation::variants::Variant;
    use uiautomation::{UIAutomation, UIElement};

    fn climb_to_owning_window(
        automation: &UIAutomation,
        starting_element: UIElement,
    ) -> Option<UIElement> {
        let mut current_element = starting_element;
        for _ in 0..16 {
            let control_type = current_element.get_control_type().ok();
            if matches!(control_type, Some(ControlType::Window)) {
                return Some(current_element);
            }
            let tree_walker = automation.get_control_view_walker().ok()?;
            match tree_walker.get_parent(&current_element) {
                Ok(parent_element) => current_element = parent_element,
                Err(_) => return None,
            }
        }
        None
    }

    let automation = match UIAutomation::new() {
        Ok(value) => value,
        Err(_) => return false,
    };
    let focused_element = match automation.get_focused_element() {
        Ok(value) => value,
        Err(_) => return false,
    };

    let owning_window = match climb_to_owning_window(&automation, focused_element) {
        Some(value) => value,
        None => return false,
    };

    // First check the WindowPattern's modal visual-state. Apps that mark
    // their dialog with WindowVisualState::Modal are the easy case.
    if let Ok(window_pattern) = owning_window.get_pattern::<UIWindowPattern>() {
        if let Ok(is_modal) = window_pattern.is_modal() {
            if is_modal {
                return true;
            }
        }
    }

    // Fallback heuristic — many older Win32 dialog hosts don't expose the
    // Modal property accurately, but their accessible name contains
    // "Dialog" or "Alert". This matches the Swift app's loose check.
    if let Ok(window_name) = owning_window.get_name() {
        let lower = window_name.to_ascii_lowercase();
        if lower.contains("dialog") || lower.contains("alert") {
            return true;
        }
    }
    let _ = Variant::default();
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_modal_dialog() -> bool {
    false
}
