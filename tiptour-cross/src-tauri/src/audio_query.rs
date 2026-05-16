// Audio-query flow — the push-to-talk hotkey's default behaviour.
//
// User holds (or toggles) the hotkey, speaks a command, releases (or
// re-toggles). The accumulated PCM16 buffer goes to gemini-2.5-flash-
// lite in a single REST call with the full adapter tool surface
// registered. Gemini either:
//
//   1. Emits a `control_app` function call → we dispatch through the
//      existing adapter pipeline and surface the result in the
//      command tooltip window.
//   2. Emits a text reply → we render it in the tooltip and let it
//      fade.
//
// This is intentionally NOT the streaming WebSocket Live session.
// Live is reserved for "Start conversation" — long-running voice
// dialogue with TTS reply. Quick commands go through here because:
//   - 3-7s round-trip is faster than a Live setup+exchange for a
//     one-shot command
//   - No TTS cost (output is text-only)
//   - No screen-streaming cost while user thinks about their command
//   - Cheaper per-call by an order of magnitude
//
// See command_tooltip.rs for the visual surface.

use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::adapters;
use crate::keychain;
use crate::personas;

const FLASH_LITE_MODEL: &str = "gemini-2.5-flash-lite";
const GENERATE_CONTENT_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-lite:generateContent";

/// Holds the PCM16 buffer accumulating during a quick-command capture.
/// Reset on every `begin_quick_voice_capture`; drained + sent on
/// `end_quick_voice_capture_and_dispatch`.
static QUICK_CAPTURE_BUFFER: Mutex<Vec<u8>> = Mutex::new(Vec::new());

#[tauri::command]
pub fn begin_quick_voice_capture(app: AppHandle) -> Result<(), String> {
    QUICK_CAPTURE_BUFFER.lock().clear();
    // Tell the command tooltip to show its "listening…" state. The
    // tooltip handles its own fade-out; we just push state events
    // and let it style accordingly.
    let _ = app.emit("quick_voice_state", "listening");
    Ok(())
}

/// Append a fresh PCM16 chunk (frontend receives it via mic_chunk
/// events from cpal and forwards here). The frontend is the natural
/// place to collect chunks because the existing mic_chunk stream
/// already flows through it; we'd duplicate listening to do it
/// purely in Rust.
#[tauri::command]
pub fn append_quick_voice_chunk(pcm_bytes: Vec<u8>) -> Result<(), String> {
    if pcm_bytes.len() > 32 * 1024 * 1024 {
        // 32MB ~= ~17 minutes at 16kHz PCM16 — way past any quick
        // command. Reject to keep memory bounded.
        return Err("audio chunk too large".to_string());
    }
    QUICK_CAPTURE_BUFFER.lock().extend_from_slice(&pcm_bytes);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickVoiceResult {
    /// What we showed in the tooltip: model text reply OR adapter
    /// confirmation OR error message.
    pub display_text: String,
    /// Whether a control_app function call fired + succeeded.
    pub action_taken: bool,
    /// Provider/model badge for the tooltip footer.
    pub model: String,
    /// Total round-trip including audio upload + Gemini round-trip +
    /// adapter dispatch. Useful for the cost meter.
    pub elapsed_ms: u64,
    /// Tokens used, if returned by the API. Surfaces in the cost
    /// meter so users see budget impact.
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

#[tauri::command]
pub async fn end_quick_voice_capture_and_dispatch(
    app: AppHandle,
) -> Result<QuickVoiceResult, String> {
    let started_at = std::time::Instant::now();
    let pcm_bytes = {
        let mut buffer = QUICK_CAPTURE_BUFFER.lock();
        std::mem::take(&mut *buffer)
    };
    if pcm_bytes.is_empty() {
        let _ = app.emit("quick_voice_state", "idle");
        return Err("no audio captured — release the hotkey while speaking".to_string());
    }
    let _ = app.emit("quick_voice_state", "thinking");

    let api_key = keychain::read_provider_key("gemini")?
        .ok_or_else(|| "Add a Gemini API key in Settings first.".to_string())?;
    let active_persona = personas::get_active_persona()
        .unwrap_or_else(|_| personas::Persona {
            id: "fallback".into(),
            name: "Fallback".into(),
            system_prompt: "You are TipTour. Use control_app to dispatch installed adapters when the user asks for an action. Reply briefly if just answering a question.".into(),
            voice_trigger_phrases: vec![],
            is_built_in: false,
            model_provider: "gemini".into(),
            model_id: FLASH_LITE_MODEL.into(),
            reasoning_enabled: false,
            temperature: 0.2,
        });

    let base64_audio = base64::engine::general_purpose::STANDARD.encode(&pcm_bytes);
    let now_iso = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let body = json!({
        "systemInstruction": {
            "role": "system",
            "parts": [{
                "text": format!(
                    "Current date and time: {now_iso}.\n\n{}\n\nYou hear short voice commands. \
                     If the command names an action — play a song, send a message, add a reminder, \
                     capture a thought, open an app — emit a control_app function call. \
                     If it's a question or chat, reply with one or two sentences. NEVER ramble. \
                     Output is shown as a small text tooltip; keep it under 80 characters when possible.",
                    active_persona.system_prompt
                )
            }]
        },
        "contents": [{
            "role": "user",
            "parts": [
                { "inline_data": { "mime_type": "audio/pcm;rate=16000", "data": base64_audio }},
                { "text": "(audio captured from push-to-talk hotkey above)" }
            ]
        }],
        "generationConfig": { "temperature": active_persona.temperature, "maxOutputTokens": 256 },
        "tools": [{
            "functionDeclarations": [adapter_dispatch_tool_declaration()]
        }],
    });

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("http client: {e}"))?
        .post(GENERATE_CONTENT_URL)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send gemini: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let _ = app.emit("quick_voice_state", "error");
        return Err(format!("gemini {status}: {}", text.chars().take(200).collect::<String>()));
    }

    let value: Value = response.json().await.map_err(|e| format!("parse: {e}"))?;
    let input_tokens = value.pointer("/usageMetadata/promptTokenCount").and_then(|v| v.as_u64()).map(|n| n as u32);
    let output_tokens = value.pointer("/usageMetadata/candidatesTokenCount").and_then(|v| v.as_u64()).map(|n| n as u32);

    // Look for a function call first, fall back to text reply.
    let parts = value.pointer("/candidates/0/content/parts");
    let function_call = parts
        .and_then(|p| p.as_array())
        .and_then(|arr| arr.iter().find_map(|p| p.get("functionCall")));

    let (display_text, action_taken) = if let Some(fc) = function_call {
        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = fc.get("args").cloned().unwrap_or(json!({}));
        if name == "control_app" {
            let slug = args.get("slug").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let handler = args.get("handler").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let inner_args = args.get("args").cloned().unwrap_or(json!({}));
            match adapters::dispatch_adapter_command(app.clone(), slug.clone(), handler.clone(), inner_args).await {
                Ok(result) => (summarize_adapter_result(&slug, &handler, &result), true),
                Err(error) => (format!("Couldn't do that: {error}"), false),
            }
        } else {
            (format!("Unknown function: {name}"), false)
        }
    } else {
        let text_reply = parts
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.iter().find_map(|p| p.get("text").and_then(|t| t.as_str())))
            .unwrap_or("(no reply)")
            .to_string();
        (text_reply, false)
    };

    let result = QuickVoiceResult {
        display_text,
        action_taken,
        model: FLASH_LITE_MODEL.to_string(),
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        input_tokens,
        output_tokens,
    };
    let _ = app.emit("quick_voice_result", &result);
    let _ = app.emit("quick_voice_state", "idle");
    Ok(result)
}

/// Render an adapter dispatch result as a short user-facing line.
/// Different adapters return different shapes so we cherry-pick the
/// most useful field per common case.
fn summarize_adapter_result(slug: &str, handler: &str, result: &Value) -> String {
    // Common shapes the adapters return: { sent: true }, { added: true },
    // { track, artist }, { id, url }, { path }, etc. Pick the friendliest
    // surface or fall back to a compact JSON summary.
    if let Some(track) = result.get("track").and_then(|v| v.as_str()) {
        let artist = result.get("artist").and_then(|v| v.as_str()).unwrap_or("");
        if !artist.is_empty() {
            return format!("Now playing: {track} — {artist}");
        }
        return format!("Now playing: {track}");
    }
    if result.get("sent").and_then(|v| v.as_bool()) == Some(true) {
        return "Sent.".to_string();
    }
    if result.get("added").and_then(|v| v.as_bool()) == Some(true) {
        return "Added.".to_string();
    }
    if result.get("created").and_then(|v| v.as_bool()) == Some(true) {
        return "Created.".to_string();
    }
    if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
        return format!("{slug}: {url}");
    }
    if let Some(path) = result.get("path").and_then(|v| v.as_str()) {
        let basename = std::path::Path::new(path).file_name()
            .and_then(|s| s.to_str()).unwrap_or(path);
        return format!("Saved: {basename}");
    }
    format!("{slug} • {handler} ok")
}

fn adapter_dispatch_tool_declaration() -> Value {
    json!({
        "name": "control_app",
        "description":
            "Drive an installed adapter to take an action on the user's machine. \
             Use this for any imperative — 'play X', 'add to list', 'send message', 'remind me', \
             'open file', 'save thought', 'screenshot'. The runtime dispatches to the correct \
             adapter and shows a one-line confirmation. Slugs you can use: \
             spotify, apple-music, whatsapp, mail-macos, messages-macos, slack, \
             calendar-macos, reminders-macos, notes-macos, notion, linear, obsidian, \
             ms-to-do, pages, numbers, keynote, word, excel, powerpoint, outlook-desktop, \
             github, vscode, terminal-macos, finder, file-explorer, safari, browser, brain-dump. \
             OR use category-relative slugs that resolve to the user's default app: \
             default:tasks, default:music, default:email, default:calendar, default:notes, \
             default:messages. CRITICAL: args field names must match the handler's expected \
             shape exactly; the deserializer rejects unknown fields. \
             Common handler shapes: spotify.play_track:{query}, \
             whatsapp.send_message:{phone,text}, github.create_issue:{repo,title,body?}, \
             brain-dump.capture_text:{text}, brain-dump.capture_screenshot:{caption?}, \
             reminders-macos.add:{text,due_iso?}, calendar-macos.create_event:{summary,start_iso}.",
        "parameters": {
            "type": "object",
            "properties": {
                "slug": { "type": "string" },
                "handler": { "type": "string" },
                "args": { "type": "object" }
            },
            "required": ["slug", "handler", "args"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_picks_track_artist_when_present() {
        let v = json!({ "track": "Lover", "artist": "Diljit" });
        assert_eq!(summarize_adapter_result("spotify", "play_track", &v), "Now playing: Lover — Diljit");
    }

    #[test]
    fn summarize_handles_added() {
        let v = json!({ "added": true });
        assert_eq!(summarize_adapter_result("reminders-macos", "add", &v), "Added.");
    }

    #[test]
    fn summarize_falls_back_to_slug_handler() {
        let v = json!({ "ok": true });
        assert_eq!(summarize_adapter_result("whatever", "doStuff", &v), "whatever • doStuff ok");
    }
}
