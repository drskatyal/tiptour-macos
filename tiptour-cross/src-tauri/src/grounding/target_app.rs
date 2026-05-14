// Resolves "what app is the user pointing at?" — the app under the mouse
// at hotkey press time, falling back to the frontmost app. Mirrors the
// Swift app's hover-window-then-frontmost-fallback policy so prefetch
// targets the app the user was actually pointing at, not TipTour itself.

use super::types::TargetApp;

#[cfg(target_os = "windows")]
pub fn current_target_app() -> Option<TargetApp> {
    // TODO: full implementation via GetForegroundWindow + GetWindowThreadProcessId
    // + QueryFullProcessImageName + GetFileVersionInfo. For Phase 1 scaffold
    // we return a minimal stub so the pipeline compiles and downstream
    // callers can be exercised against a hand-crafted TargetApp from tests.
    Some(TargetApp {
        process_id: 0,
        bundle_identifier: None,
        executable_path: None,
        display_name: None,
        file_version: None,
    })
}

#[cfg(target_os = "macos")]
pub fn current_target_app() -> Option<TargetApp> {
    // TODO: port the Swift `windowUnderMouse → frontmost fallback` logic via
    // CGWindowListCopyWindowInfo + NSWorkspace.shared.frontmostApplication.
    // Phase 1 scaffold returns a stub so the rest of the pipeline links.
    Some(TargetApp {
        process_id: 0,
        bundle_identifier: None,
        executable_path: None,
        display_name: None,
        file_version: None,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn current_target_app() -> Option<TargetApp> {
    None
}
