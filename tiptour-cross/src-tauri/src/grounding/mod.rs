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

#[derive(Default)]
struct RealGroundingProvider;

impl GroundingProvider for RealGroundingProvider {
    fn snapshot_foreground_app(&mut self) -> Result<TreeSnapshot, String> {
        let target_app = target_app::current_target_app()
            .ok_or_else(|| "no foreground app detected".to_string())?;

        let root_node = TreeNodeSnapshot {
            role: "Application".to_string(),
            name: target_app
                .display_name
                .clone()
                .or_else(|| target_app.bundle_identifier.clone())
                .or_else(|| target_app.executable_path.clone())
                .unwrap_or_else(|| format!("pid:{}", target_app.process_id)),
            // Deep tree walking lives in ax_macos / uia_windows; the
            // explorer only needs identity-grade fingerprints to detect
            // post-action visible changes, so a shallow root is enough
            // until the deep walker is exposed here.
            children: Vec::new(),
        };

        Ok(TreeSnapshot {
            foreground_window_title: target_app.display_name.clone(),
            root: root_node,
        })
    }

    fn enumerate_candidate_actions(&mut self) -> Result<Vec<Action>, String> {
        // The shortcut index already enumerates menu-driven actions on
        // Windows. The explorer treats every cached binding as a candidate
        // Shortcut action so it can BFS through reachable states without
        // touching the live UIA tree on every iteration.
        let target_app = match target_app::current_target_app() {
            Some(app) => app,
            None => return Ok(Vec::new()),
        };

        #[cfg(target_os = "windows")]
        {
            let application_identifier = uia_windows::build_application_identifier(&target_app);
            let shortcut_bindings =
                resolver::shortcut_index_for_application(&application_identifier);
            let actions = shortcut_bindings
                .into_iter()
                .map(|binding| Action::Shortcut {
                    chord_raw: binding.accelerator.raw,
                })
                .collect();
            return Ok(actions);
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = target_app;
            Ok(Vec::new())
        }
    }

    fn execute_action(&mut self, _action: &Action) -> Result<Option<TreeSnapshot>, String> {
        // Execution lives in the action-executor module owned by another
        // agent. Returning Ok(None) signals "ran, no visible change" so the
        // explorer skips edge creation rather than aborting BFS.
        Ok(None)
    }

    fn undo_last_action(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    fn return_to_state(&mut self, _state_id: &str) -> Result<bool, String> {
        Ok(false)
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
