// Cross-platform permission surface. On macOS, real AX + Screen Recording
// checks via the helpers in permissions_macos.rs. On Windows, no prompts
// are needed — the OS doesn't gate input synthesis, UIA reads, or
// Windows.Graphics.Capture for desktop user-mode apps, so every query
// returns "granted."

#[cfg(target_os = "macos")]
mod inner {
    pub use crate::permissions_macos::*;
}

#[cfg(not(target_os = "macos"))]
mod inner {
    #[tauri::command]
    pub fn check_accessibility_permission() -> bool {
        true
    }
    #[tauri::command]
    pub fn request_accessibility_permission() -> bool {
        true
    }
    #[tauri::command]
    pub fn check_screen_recording_permission() -> bool {
        true
    }
    #[tauri::command]
    pub fn request_screen_recording_permission() -> bool {
        true
    }
}

pub use inner::*;
