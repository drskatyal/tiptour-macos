// Cross-platform mouse and keyboard delivery. Uses the `enigo` crate so
// a single code path covers macOS (CGEvent under the hood) and Windows
// (SendInput). The Swift app uses CUA Driver Core directly; here we lean
// on enigo as the common synth layer.
//
// All public functions are best-effort: they return a `Result<(),String>`
// where the error message is suitable for surfacing to the user. Errors
// only happen when enigo fails to initialize (no display server, locked
// screen, etc.) — at that point automation is unrecoverable for this run.

use crate::executor::action::MouseButton;
use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
};

/// Construct a fresh enigo instance. Instances are cheap and stateless
/// for our purposes — recreating per call keeps each operation independent
/// and avoids carrying state across long-lived runner tasks.
fn enigo_instance() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|error| format!("enigo init failed: {error}"))
}

pub fn move_mouse(x: f64, y: f64) -> Result<(), String> {
    let mut enigo = enigo_instance()?;
    enigo
        .move_mouse(x as i32, y as i32, Coordinate::Abs)
        .map_err(|error| format!("move_mouse failed: {error}"))
}

pub fn click_at(x: f64, y: f64, button: MouseButton) -> Result<(), String> {
    let mut enigo = enigo_instance()?;
    enigo
        .move_mouse(x as i32, y as i32, Coordinate::Abs)
        .map_err(|error| format!("move before click failed: {error}"))?;
    let enigo_button = map_button(button);
    enigo
        .button(enigo_button, Direction::Click)
        .map_err(|error| format!("click failed: {error}"))
}

pub fn double_click_at(x: f64, y: f64) -> Result<(), String> {
    let mut enigo = enigo_instance()?;
    enigo
        .move_mouse(x as i32, y as i32, Coordinate::Abs)
        .map_err(|error| format!("move before double-click failed: {error}"))?;
    enigo
        .button(Button::Left, Direction::Click)
        .map_err(|error| format!("first click failed: {error}"))?;
    enigo
        .button(Button::Left, Direction::Click)
        .map_err(|error| format!("second click failed: {error}"))
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

/// Parse a single token from a chord description (`"Cmd"`, `"Shift"`,
/// `"A"`, `"Return"`, ...) into an enigo `Key`. Returns `None` for
/// tokens we don't recognize — the runner should treat that as a soft
/// failure and skip the step rather than mis-press.
fn key_from_token(token: &str) -> Option<Key> {
    // Single character — letter, digit, or symbol. enigo accepts these
    // through `Key::Unicode`, which routes them to the same synth path
    // as `type_text` so case and shifted-symbols behave consistently.
    let lower = token.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.chars().count() == 1 {
        return Some(Key::Unicode(lower.chars().next()?));
    }
    Some(match lower.as_str() {
        // Modifiers. On Windows, `Cmd`/`Meta` map to Ctrl per the
        // cross-platform-chord contract; we apply that remap at the
        // shortcut level (see `keyboard_shortcut`) so callers always
        // speak Mac vocabulary.
        "cmd" | "command" | "meta" | "super" | "win" => {
            #[cfg(target_os = "macos")]
            { Key::Meta }
            #[cfg(not(target_os = "macos"))]
            { Key::Control }
        }
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" | "opt" => Key::Alt,
        // Common named keys we need for the workflow surface.
        "return" | "enter" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" | "spacebar" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdown" | "pgdn" => Key::PageDown,
        "f1" => Key::F1, "f2" => Key::F2, "f3" => Key::F3, "f4" => Key::F4,
        "f5" => Key::F5, "f6" => Key::F6, "f7" => Key::F7, "f8" => Key::F8,
        "f9" => Key::F9, "f10" => Key::F10, "f11" => Key::F11, "f12" => Key::F12,
        _ => return None,
    })
}

fn is_modifier_token(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "cmd" | "command" | "meta" | "super" | "win"
            | "ctrl" | "control"
            | "shift"
            | "alt" | "option" | "opt"
    )
}

/// Press a modifier-and-key chord. Modifiers are pressed down in the
/// order given, the final non-modifier key is tapped, and modifiers are
/// released in reverse order. This matches the convention enigo's own
/// chord helpers use and survives the picky handlers some apps install
/// on individual key events.
pub fn keyboard_shortcut(keys: &[&str]) -> Result<(), String> {
    if keys.is_empty() {
        return Err("keyboard_shortcut called with no keys".to_string());
    }
    let mut enigo = enigo_instance()?;

    // Split into modifiers + terminal key. We allow more than one
    // terminal key by treating only the last non-modifier as the press
    // target; intermediate non-modifiers (rare) get tapped in sequence.
    let mut modifier_tokens: Vec<&str> = Vec::new();
    let mut press_tokens: Vec<&str> = Vec::new();
    for token in keys {
        if is_modifier_token(token) {
            modifier_tokens.push(token);
        } else {
            press_tokens.push(token);
        }
    }

    // Press modifiers down.
    let mut pressed_modifiers: Vec<Key> = Vec::with_capacity(modifier_tokens.len());
    for token in &modifier_tokens {
        if let Some(key) = key_from_token(token) {
            enigo
                .key(key, Direction::Press)
                .map_err(|error| format!("modifier press failed: {error}"))?;
            pressed_modifiers.push(key);
        }
    }

    // Tap each non-modifier. With the modifiers held this produces the
    // canonical Cmd+S / Ctrl+Shift+P shortcut behavior.
    let mut press_result: Result<(), String> = Ok(());
    for token in &press_tokens {
        if let Some(key) = key_from_token(token) {
            if let Err(error) = enigo.key(key, Direction::Click) {
                press_result = Err(format!("chord click failed: {error}"));
                break;
            }
        }
    }

    // Release modifiers in reverse order regardless of press_result so we
    // don't leave a stuck Shift/Cmd if the chord failed mid-flight.
    for key in pressed_modifiers.into_iter().rev() {
        let _ = enigo.key(key, Direction::Release);
    }

    press_result
}

pub fn type_text(text: &str) -> Result<(), String> {
    let mut enigo = enigo_instance()?;
    enigo
        .text(text)
        .map_err(|error| format!("type_text failed: {error}"))
}

pub fn scroll(x: f64, y: f64, dx: f64, dy: f64) -> Result<(), String> {
    let mut enigo = enigo_instance()?;
    // Move first so the scroll lands on the intended scroll surface.
    enigo
        .move_mouse(x as i32, y as i32, Coordinate::Abs)
        .map_err(|error| format!("move before scroll failed: {error}"))?;
    if dx.abs() >= 1.0 {
        enigo
            .scroll(dx as i32, Axis::Horizontal)
            .map_err(|error| format!("horizontal scroll failed: {error}"))?;
    }
    if dy.abs() >= 1.0 {
        enigo
            .scroll(dy as i32, Axis::Vertical)
            .map_err(|error| format!("vertical scroll failed: {error}"))?;
    }
    Ok(())
}
