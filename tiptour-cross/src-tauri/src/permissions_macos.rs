// macOS Accessibility (AX) and Screen Recording permission helpers.
//
// On first launch Mac will deny AX queries and screen capture until the user
// grants permission in System Settings → Privacy & Security. Without prompts
// the app silently fails — hotkey appears not to work, AX walk returns
// empty, screen capture errors. These helpers surface the system prompt
// once per missing permission and let the panel reflect status.

#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(
        options: core_foundation::dictionary::CFDictionaryRef,
    ) -> bool;
    fn AXIsProcessTrusted() -> bool;
    static kAXTrustedCheckOptionPrompt: core_foundation::string::CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() }
}

#[tauri::command]
pub fn request_accessibility_permission() -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = CFBoolean::true_value();
        let pairs: Vec<(CFString, CFBoolean)> = vec![(key, value)];
        let options = CFDictionary::from_CFType_pairs(&pairs);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}

#[tauri::command]
pub fn check_screen_recording_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[tauri::command]
pub fn request_screen_recording_permission() -> bool {
    // CGRequestScreenCaptureAccess opens the prompt the first time and
    // returns whether access is now granted. After the user grants
    // permission they must restart the app for the new entitlement to
    // take effect — TCC caches the decision per-pid.
    unsafe { CGRequestScreenCaptureAccess() }
}
