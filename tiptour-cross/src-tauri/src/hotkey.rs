// Global hotkeys.
//
// Two chords:
//   push_to_talk   — default Alt+X — fires the quick voice flow
//   transcribe     — default Alt+Z — toggles Soniox transcription
//
// Both are customizable via the Settings dashboard. The two-key
// constraint (one modifier + one key) holds for both so the user
// never trips on a 3-modifier chord they have to remember.

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::app_settings::load_app_settings_from_disk;

// Track each registered chord separately so reregister can find the
// right one to unregister before swapping in the user's new pick.
static CURRENTLY_REGISTERED_PUSH_TO_TALK: Mutex<Option<Shortcut>> = Mutex::new(None);
static CURRENTLY_REGISTERED_TRANSCRIBE: Mutex<Option<Shortcut>> = Mutex::new(None);

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let settings = load_app_settings_from_disk();

    let push_to_talk_chord = parse_chord_string(&settings.push_to_talk_chord)
        .unwrap_or_else(|| Shortcut::new(Some(Modifiers::ALT), Code::KeyX));
    register_chord(app, push_to_talk_chord, "push_to_talk_toggled")?;
    *CURRENTLY_REGISTERED_PUSH_TO_TALK.lock() = Some(push_to_talk_chord);

    let transcribe_chord = parse_chord_string(&settings.transcribe_chord)
        .unwrap_or_else(|| Shortcut::new(Some(Modifiers::ALT), Code::KeyZ));
    register_chord(app, transcribe_chord, "transcribe_toggled")?;
    *CURRENTLY_REGISTERED_TRANSCRIBE.lock() = Some(transcribe_chord);

    Ok(())
}

fn register_chord(app: &AppHandle, chord: Shortcut, event_name: &'static str) -> tauri::Result<()> {
    let app_handle = app.clone();
    // Press/release events get emitted separately so the panel can
    // choose between toggle semantics (act on press only) and
    // hold-to-record semantics (start on press, stop on release).
    // Event names: `<event_name>` for press, `<event_name>_released`
    // for release.
    let press_event = event_name;
    let release_event: &'static str = match event_name {
        "push_to_talk_toggled" => "push_to_talk_released",
        "transcribe_toggled" => "transcribe_released",
        _ => event_name,
    };

    app.global_shortcut()
        .on_shortcut(chord, move |_app, _sc, event| {
            match event.state {
                ShortcutState::Pressed => {
                    let _ = app_handle.emit(press_event, ());
                }
                ShortcutState::Released => {
                    let _ = app_handle.emit(release_event, ());
                }
            }
        })
        .map_err(|error| tauri::Error::Anyhow(anyhow::anyhow!(error.to_string())))?;

    Ok(())
}

/// Parse user-typed chord strings like `"Alt+X"`, `"Ctrl+Shift+Space"`,
/// `"Cmd+Option+Shift+/"`. Returns `None` for unrecognized chords so
/// the caller can fall back to the default instead of silently losing
/// the hotkey.
pub fn parse_chord_string(chord_string: &str) -> Option<Shortcut> {
    let mut modifiers = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for raw_token in chord_string.split('+') {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        match lower.as_str() {
            "alt" | "option" | "opt" => modifiers |= Modifiers::ALT,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "cmd" | "command" | "meta" | "super" | "win" => modifiers |= Modifiers::META,
            _ => {
                key_code = key_code_from_token(token);
                if key_code.is_none() {
                    // Unknown final token; whole chord is unparseable.
                    return None;
                }
            }
        }
    }

    Some(Shortcut::new(Some(modifiers), key_code?))
}

fn key_code_from_token(token: &str) -> Option<Code> {
    let upper = token.to_ascii_uppercase();
    // Single letter A–Z.
    if upper.len() == 1 {
        let character = upper.chars().next()?;
        if character.is_ascii_alphabetic() {
            return match character {
                'A' => Some(Code::KeyA),
                'B' => Some(Code::KeyB),
                'C' => Some(Code::KeyC),
                'D' => Some(Code::KeyD),
                'E' => Some(Code::KeyE),
                'F' => Some(Code::KeyF),
                'G' => Some(Code::KeyG),
                'H' => Some(Code::KeyH),
                'I' => Some(Code::KeyI),
                'J' => Some(Code::KeyJ),
                'K' => Some(Code::KeyK),
                'L' => Some(Code::KeyL),
                'M' => Some(Code::KeyM),
                'N' => Some(Code::KeyN),
                'O' => Some(Code::KeyO),
                'P' => Some(Code::KeyP),
                'Q' => Some(Code::KeyQ),
                'R' => Some(Code::KeyR),
                'S' => Some(Code::KeyS),
                'T' => Some(Code::KeyT),
                'U' => Some(Code::KeyU),
                'V' => Some(Code::KeyV),
                'W' => Some(Code::KeyW),
                'X' => Some(Code::KeyX),
                'Y' => Some(Code::KeyY),
                'Z' => Some(Code::KeyZ),
                _ => None,
            };
        }
        if character.is_ascii_digit() {
            return match character {
                '0' => Some(Code::Digit0),
                '1' => Some(Code::Digit1),
                '2' => Some(Code::Digit2),
                '3' => Some(Code::Digit3),
                '4' => Some(Code::Digit4),
                '5' => Some(Code::Digit5),
                '6' => Some(Code::Digit6),
                '7' => Some(Code::Digit7),
                '8' => Some(Code::Digit8),
                '9' => Some(Code::Digit9),
                _ => None,
            };
        }
    }
    match upper.as_str() {
        "SPACE" => Some(Code::Space),
        "ENTER" | "RETURN" => Some(Code::Enter),
        "ESC" | "ESCAPE" => Some(Code::Escape),
        "TAB" => Some(Code::Tab),
        "BACKSPACE" => Some(Code::Backspace),
        "DELETE" | "DEL" => Some(Code::Delete),
        "F1" => Some(Code::F1),
        "F2" => Some(Code::F2),
        "F3" => Some(Code::F3),
        "F4" => Some(Code::F4),
        "F5" => Some(Code::F5),
        "F6" => Some(Code::F6),
        "F7" => Some(Code::F7),
        "F8" => Some(Code::F8),
        "F9" => Some(Code::F9),
        "F10" => Some(Code::F10),
        "F11" => Some(Code::F11),
        "F12" => Some(Code::F12),
        "/" | "SLASH" => Some(Code::Slash),
        "\\" | "BACKSLASH" => Some(Code::Backslash),
        "," | "COMMA" => Some(Code::Comma),
        "." | "PERIOD" | "DOT" => Some(Code::Period),
        ";" | "SEMICOLON" => Some(Code::Semicolon),
        "'" | "QUOTE" => Some(Code::Quote),
        "[" | "LEFTBRACKET" => Some(Code::BracketLeft),
        "]" | "RIGHTBRACKET" => Some(Code::BracketRight),
        "-" | "MINUS" => Some(Code::Minus),
        "=" | "EQUAL" => Some(Code::Equal),
        _ => None,
    }
}

/// Re-register the global hotkey to the chord string the settings UI
/// captured. Persists the chord to settings.json first so the new
/// chord survives a relaunch; returns an error string if either step
/// fails. On parse failure we explicitly keep the current chord rather
/// than silently dropping the hotkey.
#[tauri::command]
pub fn reregister_push_to_talk_hotkey(app: AppHandle, chord_string: String) -> Result<(), String> {
    reregister_hotkey(
        app,
        chord_string,
        "push_to_talk_toggled",
        &CURRENTLY_REGISTERED_PUSH_TO_TALK,
        |s, value| s.push_to_talk_chord = value,
    )
}

#[tauri::command]
pub fn reregister_transcribe_hotkey(app: AppHandle, chord_string: String) -> Result<(), String> {
    reregister_hotkey(
        app,
        chord_string,
        "transcribe_toggled",
        &CURRENTLY_REGISTERED_TRANSCRIBE,
        |s, value| s.transcribe_chord = value,
    )
}

fn reregister_hotkey(
    app: AppHandle,
    chord_string: String,
    event_name: &'static str,
    slot: &Mutex<Option<Shortcut>>,
    persist_field: fn(&mut crate::app_settings::AppSettings, String),
) -> Result<(), String> {
    let parsed_chord = parse_chord_string(&chord_string)
        .ok_or_else(|| format!("could not parse chord '{chord_string}'"))?;

    if let Some(previous_chord) = *slot.lock() {
        let _ = app.global_shortcut().unregister(previous_chord);
    }

    let mut current_settings = load_app_settings_from_disk();
    persist_field(&mut current_settings, chord_string);
    crate::app_settings::save_app_settings_to_disk(&current_settings)?;

    register_chord(&app, parsed_chord, event_name).map_err(|error| error.to_string())?;
    *slot.lock() = Some(parsed_chord);
    Ok(())
}

// Suppress unused-import warning when target features hide the manager
// re-export path used elsewhere.
#[allow(dead_code)]
fn _retain_manager_import(app: &AppHandle) {
    let _ = app.get_webview_window("panel");
}
