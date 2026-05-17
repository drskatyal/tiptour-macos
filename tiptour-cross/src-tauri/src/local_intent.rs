// Local intent router.
//
// Some commands are universal + platform-native and DO NOT need an
// LLM round-trip:
//   - maximize / minimize / fullscreen the focused window
//   - show desktop / hide all windows
//   - switch app (Cmd+Tab / Alt+Tab)
//   - close window (Cmd+W / Alt+F4)
//   - open <app name>
//
// We match the user's transcribed phrase against a small phrase
// table, and when one fires we synthesize the right OS-level
// action (key chord or app launch) directly. No network, no
// keychain dependency, no Gemini call.
//
// Anything that doesn't match here falls through to the normal
// Gemini flow.

use serde::Serialize;

/// Result the panel reports back so it can show "Maximized window"
/// or similar feedback instead of pretending nothing happened.
#[derive(Debug, Clone, Serialize)]
pub struct LocalIntentMatch {
    pub matched: bool,
    pub intent: String,
    pub message: String,
}

fn normalize(phrase: &str) -> String {
    phrase
        .trim()
        .to_lowercase()
        // Strip common filler tokens so "could you please maximize the
        // window" still matches the "maximize" intent.
        .replace("could you", "")
        .replace("can you", "")
        .replace("please", "")
        .replace("the window", "")
        .replace("this window", "")
        .replace("a window", "")
        .replace("  ", " ")
        .trim()
        .to_string()
}

/// True when `phrase` contains every word in `needle` in order
/// (allowing extra words between). Cheap fuzzy match.
fn contains_all_in_order(phrase: &str, needle: &str) -> bool {
    let mut cursor = 0usize;
    for word in needle.split_whitespace() {
        match phrase[cursor..].find(word) {
            Some(found) => {
                cursor += found + word.len();
            }
            None => return false,
        }
    }
    true
}

fn maximize_window() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // macOS: hold Option + click green button is the native flow,
        // but we don't have that gesture. The reliable substitute is
        // Control-Command-F which toggles fullscreen across most
        // apps. Falling back to Cmd+M (minimize) would be wrong.
        run_osascript(
            r#"tell application "System Events" to keystroke "f" using {control down, command down}"#,
        )
    }
    #[cfg(target_os = "windows")]
    {
        // Win+Up maximizes the focused window in every desktop
        // environment since Vista.
        send_windows_key_chord(&["win", "up"])
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Err("maximize not implemented on this OS".into())
    }
}

fn minimize_window() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        run_osascript(r#"tell application "System Events" to keystroke "m" using command down"#)
    }
    #[cfg(target_os = "windows")]
    {
        send_windows_key_chord(&["win", "down"])
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Err("minimize not implemented on this OS".into())
    }
}

fn show_desktop() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // F11 is the Mission Control "show desktop" hotkey on macOS.
        run_osascript(r#"tell application "System Events" to key code 103"#)
    }
    #[cfg(target_os = "windows")]
    {
        send_windows_key_chord(&["win", "d"])
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Err("show desktop not implemented on this OS".into())
    }
}

fn close_window() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        run_osascript(r#"tell application "System Events" to keystroke "w" using command down"#)
    }
    #[cfg(target_os = "windows")]
    {
        // Alt+F4 closes the active window across every Windows app.
        send_windows_key_chord(&["alt", "f4"])
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Err("close window not implemented on this OS".into())
    }
}

fn open_app(app_name: &str) -> Result<(), String> {
    let cleaned = app_name.trim();
    if cleaned.is_empty() {
        return Err("no app name supplied".into());
    }
    #[cfg(target_os = "macos")]
    {
        // `open -a` resolves the bundle by name regardless of where
        // the .app lives. Failing to launch returns non-zero which
        // bubbles up as a clear error string.
        let output = std::process::Command::new("open")
            .arg("-a")
            .arg(cleaned)
            .output()
            .map_err(|spawn_error| format!("open spawn: {spawn_error}"))?;
        if !output.status.success() {
            return Err(format!(
                "couldn't launch '{cleaned}': {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // `cmd /C start "" "<name>"` searches Start menu shortcuts +
        // PATH. The empty quoted title is required because the first
        // quoted arg to `start` is interpreted as the window title.
        let output = std::process::Command::new("cmd")
            .args(["/C", "start", "", cleaned])
            .output()
            .map_err(|spawn_error| format!("start spawn: {spawn_error}"))?;
        if !output.status.success() {
            return Err(format!(
                "couldn't launch '{cleaned}': {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = cleaned;
        Err("open app not implemented on this OS".into())
    }
}

#[cfg(target_os = "macos")]
fn run_osascript(script: &str) -> Result<(), String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|spawn_error| format!("osascript spawn: {spawn_error}"))?;
    if !output.status.success() {
        return Err(format!(
            "osascript: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn send_windows_key_chord(keys: &[&str]) -> Result<(), String> {
    use rdev::{simulate, EventType, Key};
    fn key_for(name: &str) -> Option<Key> {
        match name.to_ascii_lowercase().as_str() {
            "win" | "meta" => Some(Key::MetaLeft),
            "alt" => Some(Key::Alt),
            "ctrl" | "control" => Some(Key::ControlLeft),
            "shift" => Some(Key::ShiftLeft),
            "up" => Some(Key::UpArrow),
            "down" => Some(Key::DownArrow),
            "left" => Some(Key::LeftArrow),
            "right" => Some(Key::RightArrow),
            "d" => Some(Key::KeyD),
            "f4" => Some(Key::F4),
            single if single.len() == 1 => {
                let c = single.chars().next()?;
                match c {
                    'a'..='z' => {
                        // rdev exposes letters as Key::KeyA..KeyZ
                        // via name strings; we don't enumerate every
                        // letter here, only the ones our chords use.
                        match c {
                            'd' => Some(Key::KeyD),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    let chord: Vec<Key> = keys
        .iter()
        .map(|name| key_for(name).ok_or_else(|| format!("unknown key '{name}'")))
        .collect::<Result<Vec<_>, _>>()?;
    // Press in order, release in reverse — same shape as a real
    // user hotkey.
    for key in &chord {
        simulate(&EventType::KeyPress(*key))
            .map_err(|simulation_error| format!("key press: {simulation_error:?}"))?;
    }
    for key in chord.iter().rev() {
        simulate(&EventType::KeyRelease(*key))
            .map_err(|simulation_error| format!("key release: {simulation_error:?}"))?;
    }
    Ok(())
}

/// Try to handle the phrase locally. Returns Ok(matched=true) when
/// a known intent ran. Ok(matched=false) when the phrase didn't
/// match any local intent — the caller should fall through to the
/// LLM. Err only when an intent matched but execution failed.
#[tauri::command]
pub fn try_local_intent(phrase: String) -> Result<LocalIntentMatch, String> {
    let cleaned = normalize(&phrase);

    // Window management — order matters: more specific phrases
    // first so "close window" doesn't match "close" inside a
    // longer phrase like "close the file menu".
    if contains_all_in_order(&cleaned, "show desktop")
        || cleaned == "minimize all"
        || cleaned == "hide all windows"
    {
        show_desktop()?;
        return Ok(LocalIntentMatch {
            matched: true,
            intent: "show_desktop".into(),
            message: "Showed desktop".into(),
        });
    }
    if contains_all_in_order(&cleaned, "close window") || cleaned == "close" {
        close_window()?;
        return Ok(LocalIntentMatch {
            matched: true,
            intent: "close_window".into(),
            message: "Closed window".into(),
        });
    }
    if contains_all_in_order(&cleaned, "maximize") || contains_all_in_order(&cleaned, "full screen")
    {
        maximize_window()?;
        return Ok(LocalIntentMatch {
            matched: true,
            intent: "maximize".into(),
            message: "Maximized window".into(),
        });
    }
    if contains_all_in_order(&cleaned, "minimize") {
        minimize_window()?;
        return Ok(LocalIntentMatch {
            matched: true,
            intent: "minimize".into(),
            message: "Minimized window".into(),
        });
    }

    // "open <app>" — the rest of the phrase after the keyword is
    // the app name to launch.
    for keyword in ["open ", "launch ", "start "] {
        if let Some(index) = cleaned.find(keyword) {
            let after = &cleaned[index + keyword.len()..];
            if !after.is_empty() {
                let app_name = after.trim_end_matches('.').trim();
                open_app(app_name)?;
                return Ok(LocalIntentMatch {
                    matched: true,
                    intent: "open_app".into(),
                    message: format!("Opening {app_name}"),
                });
            }
        }
    }

    Ok(LocalIntentMatch {
        matched: false,
        intent: "(no match)".into(),
        message: String::new(),
    })
}
