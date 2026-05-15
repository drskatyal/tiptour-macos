// Grounding layer entry point. Exposes a unified `ElementResolver` to
// the rest of the app and surfaces Tauri commands so the TS layer can
// pre-warm the cache on hotkey press, ground a Gemini-emitted label at
// tool-call time, and read the shortcut index back for prompt context.

pub mod persistence;
pub mod resolver;
pub mod target_app;
pub mod types;

#[cfg(target_os = "macos")]
pub mod ax_macos;

#[cfg(target_os = "windows")]
pub mod uia_windows;

use crate::capabilities::{
    fingerprint::{TreeNodeSnapshot, TreeSnapshot},
    types::Action,
    GroundingProvider,
};
use resolver::Box2dHint;
use types::{ResolvedTarget, ShortcutBinding};

#[tauri::command]
pub async fn prefetch_target_app() -> Result<(), String> {
    // Run the prefetch off the main thread so the hotkey handler returns
    // immediately. The cache it warms is consulted by `resolve_label`
    // milliseconds later — overlap with Gemini session setup is the win.
    tauri::async_runtime::spawn_blocking(|| {
        let target_app = match target_app::current_target_app() {
            Some(app) => app,
            None => return,
        };

        #[cfg(target_os = "macos")]
        {
            ax_macos::prefetch_for_app(target_app.process_id);
        }

        #[cfg(target_os = "windows")]
        {
            match uia_windows::index_foreground_application(&target_app) {
                Ok(shortcut_index) => {
                    let _ = persistence::save_shortcut_index(&shortcut_index);
                }
                Err(_error) => {
                    // Swallowing here keeps the hotkey path resilient — a
                    // failed index walk shouldn't block voice capture.
                }
            }
        }

        let _ = target_app;
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_label(label: String) -> Result<Option<ResolvedTarget>, String> {
    let target_app = target_app::current_target_app();
    tauri::async_runtime::spawn_blocking(move || resolver::resolve(&label, target_app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_label_with_hint(
    label: String,
    box_2d_normalized: Option<[u32; 4]>,
) -> Result<Option<ResolvedTarget>, String> {
    let target_app = target_app::current_target_app();
    tauri::async_runtime::spawn_blocking(move || {
        let box_2d_hint = box_2d_normalized.and_then(build_box_hint_from_main_screen);
        resolver::resolve_with_box_hint(&label, target_app, box_2d_hint)
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_shortcut_index(app_id: String) -> Result<Vec<ShortcutBinding>, String> {
    tauri::async_runtime::spawn_blocking(move || resolver::shortcut_index_for_application(&app_id))
        .await
        .map_err(|error| error.to_string())
}

// Public adapter so the capabilities explorer can drive the grounding
// backend through a single trait object without leaking platform details.
pub fn make_grounding_provider() -> Box<dyn GroundingProvider> {
    Box::new(RealGroundingProvider::default())
}

// Cap for the snapshot tree walk. Matches the depth the spec calls for
// (depth 4) — deep enough to capture the user-visible structure, shallow
// enough that big apps' AX/UIA trees fingerprint in well under a second.
const SNAPSHOT_TREE_MAX_DEPTH: usize = 4;
// Wait after Cmd+Z / Esc / Cmd+W before re-fingerprinting. The 200ms
// number matches the spec; many apps need a frame or two to repaint the
// post-undo state and the fingerprint would otherwise look unchanged.
const UNDO_SETTLE_MILLISECONDS: u64 = 200;
// Hard cap on how many Esc + Cmd+W presses return_to_state will issue
// while trying to navigate back to a known fingerprint.
const RETURN_TO_STATE_MAX_ATTEMPTS: usize = 5;

#[derive(Default)]
struct RealGroundingProvider {
    // Fingerprint stack used by undo_last_action to verify whether the
    // synthesized Cmd+Z / Ctrl+Z actually rolled the UI back to the state
    // we were in before the most recent execute_action.
    state_history: Vec<String>,
}

impl GroundingProvider for RealGroundingProvider {
    fn snapshot_foreground_app(&mut self) -> Result<TreeSnapshot, String> {
        #[cfg(target_os = "macos")]
        {
            let target_app = target_app::current_target_app()
                .ok_or_else(|| "no foreground app detected".to_string())?;
            return snapshot_macos_application(target_app.process_id);
        }

        #[cfg(target_os = "windows")]
        {
            return uia_windows::snapshot_foreground_tree(SNAPSHOT_TREE_MAX_DEPTH);
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            // Linux dev host: hand back a stable sentinel snapshot so unit
            // tests and CI keep moving without an AX/UIA backend.
            let _ = SNAPSHOT_TREE_MAX_DEPTH;
            Err("no AX/UIA backend on this platform".to_string())
        }
    }

    fn enumerate_candidate_actions(&mut self) -> Result<Vec<Action>, String> {
        let target_app = match target_app::current_target_app() {
            Some(app) => app,
            None => return Ok(Vec::new()),
        };

        #[cfg(target_os = "macos")]
        {
            let interactive_labels = enumerate_macos_interactive_labels(target_app.process_id);
            return Ok(build_click_actions_from_labels(interactive_labels));
        }

        #[cfg(target_os = "windows")]
        {
            let _ = target_app;
            let interactive_labels = uia_windows::enumerate_interactive_element_labels()
                .unwrap_or_default();
            return Ok(build_click_actions_from_labels(interactive_labels));
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = target_app;
            Ok(Vec::new())
        }
    }

    fn execute_action(&mut self, action: &Action) -> Result<Option<TreeSnapshot>, String> {
        // Snapshot the pre-action fingerprint so undo_last_action has a
        // ground truth to compare against. We push before we dispatch so a
        // failed dispatch still leaves the history coherent.
        let pre_action_snapshot = self.snapshot_foreground_app()?;
        let pre_action_fingerprint =
            crate::capabilities::fingerprint::fingerprint(&pre_action_snapshot);
        self.state_history.push(pre_action_fingerprint.clone());

        match action {
            Action::Click { element_name, .. } => {
                let target_app = target_app::current_target_app();
                let resolved_target = resolver::resolve(element_name, target_app)
                    .ok_or_else(|| format!("could not resolve label: {element_name}"))?;
                match resolved_target {
                    types::ResolvedTarget::Coordinate { x, y } => {
                        crate::executor::cross_platform_input::click_at(
                            x,
                            y,
                            crate::executor::action::MouseButton::Left,
                        )?;
                    }
                    types::ResolvedTarget::Shortcut { chord } => {
                        // The resolver preferred a keyboard shortcut for this
                        // element — fire the chord exactly as authored.
                        let chord_tokens: Vec<&str> = chord
                            .modifiers
                            .iter()
                            .map(|modifier| modifier.as_str())
                            .chain(std::iter::once(chord.key.as_str()))
                            .collect();
                        crate::executor::cross_platform_input::keyboard_shortcut(&chord_tokens)?;
                    }
                }
            }
            Action::Shortcut { chord_raw } => {
                let chord_tokens: Vec<&str> = chord_raw
                    .split('+')
                    .map(|token| token.trim())
                    .filter(|token| !token.is_empty())
                    .collect();
                crate::executor::cross_platform_input::keyboard_shortcut(&chord_tokens)?;
            }
            Action::Type { .. } | Action::Scroll { .. } | Action::Navigate { .. } => {
                // The explorer doesn't synthesize these today; if a future
                // emitter adds them we'd grow concrete branches here.
                return Ok(None);
            }
        }

        // Give the UI a moment to repaint before we sample the post-action
        // fingerprint, otherwise we'd routinely report "no visible change"
        // for actions that just hadn't rendered yet.
        std::thread::sleep(std::time::Duration::from_millis(UNDO_SETTLE_MILLISECONDS));
        let post_action_snapshot = self.snapshot_foreground_app()?;
        let post_action_fingerprint =
            crate::capabilities::fingerprint::fingerprint(&post_action_snapshot);
        if post_action_fingerprint == pre_action_fingerprint {
            // No visible change — explorer reads this as "skip this edge".
            return Ok(None);
        }
        Ok(Some(post_action_snapshot))
    }

    fn undo_last_action(&mut self) -> Result<bool, String> {
        let previous_fingerprint = match self.state_history.pop() {
            Some(fingerprint) => fingerprint,
            None => return Ok(false),
        };

        // Standard "undo last edit" chord on each platform. Real apps usually
        // honor this even when the user's last action was navigational —
        // worst case the fingerprint won't match and we return false.
        #[cfg(target_os = "macos")]
        let undo_chord: &[&str] = &["Cmd", "z"];
        #[cfg(not(target_os = "macos"))]
        let undo_chord: &[&str] = &["Ctrl", "z"];
        let _ = crate::executor::cross_platform_input::keyboard_shortcut(undo_chord);

        std::thread::sleep(std::time::Duration::from_millis(UNDO_SETTLE_MILLISECONDS));

        let after_undo_snapshot = match self.snapshot_foreground_app() {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(false),
        };
        let after_undo_fingerprint =
            crate::capabilities::fingerprint::fingerprint(&after_undo_snapshot);
        Ok(after_undo_fingerprint == previous_fingerprint)
    }

    fn return_to_state(&mut self, target_state_id: &str) -> Result<bool, String> {
        // Best-effort back-navigation: try Esc first (closes popovers,
        // dropdowns, dialogs), then Cmd/Ctrl+W (closes the frontmost tab or
        // sheet). Real BFS replay belongs to a future revision — this
        // heuristic is enough to recover from the kinds of states the
        // explorer typically opens via a single click.
        if let Ok(initial_snapshot) = self.snapshot_foreground_app() {
            if crate::capabilities::fingerprint::fingerprint(&initial_snapshot) == target_state_id {
                return Ok(true);
            }
        }

        #[cfg(target_os = "macos")]
        let close_window_chord: &[&str] = &["Cmd", "w"];
        #[cfg(not(target_os = "macos"))]
        let close_window_chord: &[&str] = &["Ctrl", "w"];

        for _attempt_index in 0..RETURN_TO_STATE_MAX_ATTEMPTS {
            let _ = crate::executor::cross_platform_input::keyboard_shortcut(&["Escape"]);
            std::thread::sleep(std::time::Duration::from_millis(UNDO_SETTLE_MILLISECONDS));
            if let Ok(snapshot) = self.snapshot_foreground_app() {
                if crate::capabilities::fingerprint::fingerprint(&snapshot) == target_state_id {
                    return Ok(true);
                }
            }

            let _ = crate::executor::cross_platform_input::keyboard_shortcut(close_window_chord);
            std::thread::sleep(std::time::Duration::from_millis(UNDO_SETTLE_MILLISECONDS));
            if let Ok(snapshot) = self.snapshot_foreground_app() {
                if crate::capabilities::fingerprint::fingerprint(&snapshot) == target_state_id {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn build_click_actions_from_labels(labels: Vec<String>) -> Vec<Action> {
    labels
        .into_iter()
        .filter(|label| {
            // Skip the deny-list at enumeration time so destructive actions
            // never enter the BFS frontier in the first place.
            let trial_action = Action::Click {
                element_name: label.clone(),
                element_path: Vec::new(),
            };
            !crate::capabilities::safety::is_destructive(&trial_action, label)
        })
        .map(|label| Action::Click {
            element_name: label,
            element_path: Vec::new(),
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn snapshot_macos_application(process_id: i32) -> Result<TreeSnapshot, String> {
    use accessibility_sys::{
        kAXChildrenAttribute, kAXDescriptionAttribute, kAXRoleAttribute, kAXTitleAttribute,
        AXError, AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef,
    };
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFRelease, CFType, CFTypeID, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};

    unsafe fn read_string_attribute(
        element: AXUIElementRef,
        attribute_name: &str,
    ) -> Option<String> {
        let attribute_cf = CFString::new(attribute_name);
        let mut raw_value: CFTypeRef = std::ptr::null_mut();
        let status: AXError = AXUIElementCopyAttributeValue(
            element,
            attribute_cf.as_concrete_TypeRef(),
            &mut raw_value,
        );
        if status != 0 || raw_value.is_null() {
            return None;
        }
        let type_id: CFTypeID = core_foundation::base::CFGetTypeID(raw_value);
        if type_id != CFString::type_id() {
            CFRelease(raw_value);
            return None;
        }
        let cf_string: CFString = CFString::wrap_under_create_rule(raw_value as CFStringRef);
        Some(cf_string.to_string())
    }

    unsafe fn walk_macos_node(
        element: AXUIElementRef,
        current_depth: usize,
        max_depth: usize,
    ) -> TreeNodeSnapshot {
        let role = read_string_attribute(element, kAXRoleAttribute).unwrap_or_default();
        let name = read_string_attribute(element, kAXTitleAttribute)
            .or_else(|| read_string_attribute(element, kAXDescriptionAttribute))
            .unwrap_or_default();

        let mut children: Vec<TreeNodeSnapshot> = Vec::new();
        if current_depth < max_depth {
            let children_attribute = CFString::new(kAXChildrenAttribute);
            let mut children_raw: CFTypeRef = std::ptr::null_mut();
            let copy_status: AXError = AXUIElementCopyAttributeValue(
                element,
                children_attribute.as_concrete_TypeRef(),
                &mut children_raw,
            );
            if copy_status == 0 && !children_raw.is_null() {
                let array_ref = children_raw as CFArrayRef;
                let array: CFArray<CFType> = CFArray::wrap_under_create_rule(array_ref);
                for child_index in 0..array.len() {
                    if let Some(child_cf_type) = array.get(child_index) {
                        let child_element_ref =
                            child_cf_type.as_CFTypeRef() as AXUIElementRef;
                        children.push(walk_macos_node(
                            child_element_ref,
                            current_depth + 1,
                            max_depth,
                        ));
                    }
                }
            }
        }

        TreeNodeSnapshot { role, name, children }
    }

    unsafe {
        let application_element: AXUIElementRef = AXUIElementCreateApplication(process_id);
        if application_element.is_null() {
            return Err("AXUIElementCreateApplication returned null".to_string());
        }
        let root_node = walk_macos_node(application_element, 0, SNAPSHOT_TREE_MAX_DEPTH);
        // Pick the first window's title as the foreground-window identity
        // signal — same heuristic the swift app uses.
        let foreground_window_title = root_node
            .children
            .iter()
            .find(|child| child.role == "AXWindow")
            .map(|window_node| window_node.name.clone())
            .filter(|title| !title.is_empty());
        CFRelease(application_element as CFTypeRef);
        Ok(TreeSnapshot {
            foreground_window_title,
            root: root_node,
        })
    }
}

#[cfg(target_os = "macos")]
fn enumerate_macos_interactive_labels(process_id: i32) -> Vec<String> {
    use accessibility_sys::{
        kAXChildrenAttribute, kAXDescriptionAttribute, kAXRoleAttribute, kAXTitleAttribute,
        AXError, AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef,
    };
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFRelease, CFType, CFTypeID, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};

    const INTERACTIVE_AX_ROLES: &[&str] = &[
        "AXButton",
        "AXMenuItem",
        "AXCheckBox",
        "AXRadioButton",
    ];
    const MAX_WALK_DEPTH: usize = 12;

    unsafe fn read_string(element: AXUIElementRef, attribute_name: &str) -> Option<String> {
        let attribute_cf = CFString::new(attribute_name);
        let mut raw_value: CFTypeRef = std::ptr::null_mut();
        let status: AXError = AXUIElementCopyAttributeValue(
            element,
            attribute_cf.as_concrete_TypeRef(),
            &mut raw_value,
        );
        if status != 0 || raw_value.is_null() {
            return None;
        }
        let type_id: CFTypeID = core_foundation::base::CFGetTypeID(raw_value);
        if type_id != CFString::type_id() {
            CFRelease(raw_value);
            return None;
        }
        let cf_string: CFString = CFString::wrap_under_create_rule(raw_value as CFStringRef);
        Some(cf_string.to_string())
    }

    unsafe fn walk(
        element: AXUIElementRef,
        current_depth: usize,
        labels: &mut Vec<String>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        if current_depth >= MAX_WALK_DEPTH {
            return;
        }
        let role = read_string(element, kAXRoleAttribute).unwrap_or_default();
        if INTERACTIVE_AX_ROLES.contains(&role.as_str()) {
            let label = read_string(element, kAXTitleAttribute)
                .or_else(|| read_string(element, kAXDescriptionAttribute))
                .unwrap_or_default();
            let trimmed_label = label.trim();
            if !trimmed_label.is_empty() && seen.insert(trimmed_label.to_string()) {
                labels.push(trimmed_label.to_string());
            }
        }

        let children_attribute = CFString::new(kAXChildrenAttribute);
        let mut children_raw: CFTypeRef = std::ptr::null_mut();
        let copy_status: AXError = AXUIElementCopyAttributeValue(
            element,
            children_attribute.as_concrete_TypeRef(),
            &mut children_raw,
        );
        if copy_status != 0 || children_raw.is_null() {
            return;
        }
        let array_ref = children_raw as CFArrayRef;
        let array: CFArray<CFType> = CFArray::wrap_under_create_rule(array_ref);
        for child_index in 0..array.len() {
            if let Some(child_cf_type) = array.get(child_index) {
                let child_element_ref = child_cf_type.as_CFTypeRef() as AXUIElementRef;
                walk(child_element_ref, current_depth + 1, labels, seen);
            }
        }
    }

    unsafe {
        let application_element: AXUIElementRef = AXUIElementCreateApplication(process_id);
        if application_element.is_null() {
            return Vec::new();
        }
        let mut labels: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        walk(application_element, 0, &mut labels, &mut seen);
        CFRelease(application_element as CFTypeRef);
        labels
    }
}

// Best-effort screen-dimension lookup so the resolver can convert Gemini's
// 0..=1000 normalized box_2d into absolute pixels. On Linux dev hosts we
// fall back to a sentinel that the resolver will reject.
fn build_box_hint_from_main_screen(normalized: [u32; 4]) -> Option<Box2dHint> {
    let (screen_width, screen_height) = main_screen_dimensions()?;
    Some(Box2dHint {
        normalized,
        screen_width,
        screen_height,
    })
}

#[cfg(target_os = "macos")]
fn main_screen_dimensions() -> Option<(f64, f64)> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    unsafe {
        let screen_class = AnyClass::get("NSScreen")?;
        let main_screen: *mut objc2::runtime::AnyObject = msg_send![screen_class, mainScreen];
        if main_screen.is_null() {
            return None;
        }
        let frame: objc2_foundation::NSRect = msg_send![main_screen, frame];
        Some((frame.size.width, frame.size.height))
    }
}

#[cfg(target_os = "windows")]
fn main_screen_dimensions() -> Option<(f64, f64)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return None;
        }
        Some((width as f64, height as f64))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main_screen_dimensions() -> Option<(f64, f64)> {
    None
}
