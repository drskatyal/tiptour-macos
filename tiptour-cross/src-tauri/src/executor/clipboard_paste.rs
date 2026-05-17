// Clipboard-staged paste helper. Mirrors the rich-editor fallback in
// `TipTour/ActionExecutor.swift`: when AX selected-text insertion is
// unavailable or unreliable (Google Docs, Notion web, Figma), copy the
// payload to the system clipboard and synthesize Cmd+V (macOS) / Ctrl+V
// (Windows). Restores the previous clipboard contents after the paste
// completes so we don't clobber the user's clipboard history.

use arboard::Clipboard;

use crate::executor::cross_platform_input;

pub fn paste_text(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| format!("clipboard open: {error}"))?;

    // Capture previous text so we can restore it after pasting. Non-text
    // clipboard contents (images, files) are intentionally not restored —
    // arboard's text-only API is good enough for the common case and
    // matches the Swift implementation's pragmatic scope.
    let previous_text = clipboard.get_text().ok();

    clipboard
        .set_text(text.to_string())
        .map_err(|error| format!("clipboard write: {error}"))?;

    // Tiny settle delay — some clipboard owners (Chrome, Electron) need a
    // moment to pick up the pasteboard change before Cmd+V fires.
    std::thread::sleep(std::time::Duration::from_millis(40));

    let paste_keys = ["Cmd", "V"];
    let paste_result = cross_platform_input::keyboard_shortcut(&paste_keys);

    // Restore previous text on a small delay so the paste consumer reads
    // our text first. Best-effort; if restoration fails the user's
    // clipboard is left holding the typed payload.
    if let Some(previous) = previous_text {
        std::thread::sleep(std::time::Duration::from_millis(120));
        let _ = clipboard.set_text(previous);
    }

    paste_result
}
