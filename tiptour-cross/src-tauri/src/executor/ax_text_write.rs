// Direct AX/UIA text insertion into the system-wide focused element.
//
// The runner prefers this over keystroke synthesis or clipboard paste when
// the LLM marked a Type step as `into_focused`, because it (a) preserves
// the user's clipboard, (b) avoids racing with the user's actual keyboard,
// and (c) lands the text deterministically inside the focused field even
// if the OS would have routed synthetic key events elsewhere.
//
// On failure the caller is expected to fall back to its existing
// clipboard/keystroke heuristic — this helper never throws, it just
// returns Err so the caller can drop through.

#[cfg(target_os = "macos")]
pub fn attempt_ax_text_write(text: &str) -> Result<(), String> {
    use accessibility_sys::{
        kAXFocusedUIElementAttribute, kAXSelectedTextAttribute, AXUIElementCopyAttributeValue,
        AXUIElementCreateSystemWide, AXUIElementRef, AXUIElementSetAttributeValue,
        AXUIElementSetMessagingTimeout,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;

    unsafe {
        let system_wide_element: AXUIElementRef = AXUIElementCreateSystemWide();
        if system_wide_element.is_null() {
            return Err("AXUIElementCreateSystemWide returned null".to_string());
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
            return Err(format!(
                "AXFocusedUIElement copy failed with status {focused_status}"
            ));
        }
        let focused_element_ref = focused_raw as AXUIElementRef;
        AXUIElementSetMessagingTimeout(focused_element_ref, 0.4);

        // Setting kAXSelectedTextAttribute replaces the current selection,
        // or inserts at the caret when nothing is selected. Most cocoa
        // text views and many Electron text inputs (with manual AX) honor
        // this; if the focused element doesn't expose it we get an error
        // and the caller falls back.
        let selected_text_attribute = CFString::new(kAXSelectedTextAttribute);
        let payload = CFString::new(text);
        let set_status = AXUIElementSetAttributeValue(
            focused_element_ref,
            selected_text_attribute.as_concrete_TypeRef(),
            payload.as_concrete_TypeRef() as CFTypeRef,
        );
        CFRelease(focused_raw);
        if set_status != 0 {
            return Err(format!(
                "AXUIElementSetAttributeValue(kAXSelectedTextAttribute) failed with status {set_status}"
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub fn attempt_ax_text_write(text: &str) -> Result<(), String> {
    use uiautomation::patterns::UIValuePattern;
    use uiautomation::UIAutomation;

    let automation =
        UIAutomation::new().map_err(|error| format!("UIAutomation::new failed: {error}"))?;
    let focused_element = automation
        .get_focused_element()
        .map_err(|error| format!("get_focused_element failed: {error}"))?;

    // uiautomation 0.16 doesn't expose UITextRange::set_selected_text the
    // way newer wrappers do — the selection-replace surface is gone, so
    // ValuePattern.set_value is the only direct-write path we have for
    // Windows. The runner's clipboard-paste fallback handles every other
    // case (rich web editors, controls without ValuePattern, etc).
    if let Ok(value_pattern) = focused_element.get_pattern::<UIValuePattern>() {
        if value_pattern.set_value(text).is_ok() {
            return Ok(());
        }
    }

    Err("focused element does not expose ValuePattern.set_value".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn attempt_ax_text_write(_text: &str) -> Result<(), String> {
    Err("attempt_ax_text_write not supported on this platform".to_string())
}
