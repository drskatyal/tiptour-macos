// Soniox real-time transcription.
//
// Streams PCM16 mic audio over Soniox's WebSocket API
// (wss://stt-rt.soniox.com/transcribe-websocket) and pipes the
// resulting transcript directly into the focused text field via the
// existing clipboard-paste path. Final (non-revisable) tokens get
// pasted; non-final tokens get buffered and replaced on each update.
//
// Three trigger paths:
//   1. Dock "Transcribe" button → toggle_soniox_transcription
//   2. Global hotkey (Ctrl+Shift+T by default)
//   3. Voice command via control_app(soniox, start_transcribe)
//
// API key: stored under the "soniox" keychain slot via the
// existing multi-key keychain machinery. Free tier on Soniox has
// generous limits; users without a key see a clear error.

use futures_util::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::executor::clipboard_paste;
use crate::keychain;

const SONIOX_WS_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
const SAMPLE_RATE_HZ: u32 = 16_000;

/// Channel for piping mic chunks from the frontend into the active
/// Soniox WebSocket sender. Replaced (Some → None) when transcription
/// stops, so stray chunks after stop fall on the floor instead of
/// reconnecting.
struct SonioxSession {
    chunk_sender: mpsc::UnboundedSender<Vec<u8>>,
    /// Total characters dispatched into the focused field this
    /// session — surfaced to the cost meter so users see the volume.
    chars_typed: u64,
}

static ACTIVE_SESSION: Lazy<Mutex<Option<SonioxSession>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SonioxState {
    pub is_active: bool,
    pub chars_typed: u64,
}

#[tauri::command]
pub fn get_soniox_state() -> SonioxState {
    let guard = ACTIVE_SESSION.lock();
    match &*guard {
        Some(s) => SonioxState {
            is_active: true,
            chars_typed: s.chars_typed,
        },
        None => SonioxState::default(),
    }
}

#[tauri::command]
pub async fn toggle_soniox_transcription(app: AppHandle) -> Result<SonioxState, String> {
    let is_active = ACTIVE_SESSION.lock().is_some();
    if is_active {
        stop_soniox_transcription_impl(app).await
    } else {
        start_soniox_transcription_impl(app).await
    }
}

#[tauri::command]
pub async fn start_soniox_transcription(app: AppHandle) -> Result<SonioxState, String> {
    if ACTIVE_SESSION.lock().is_some() {
        return Err("transcription already running — stop it first".into());
    }
    start_soniox_transcription_impl(app).await
}

#[tauri::command]
pub async fn stop_soniox_transcription(app: AppHandle) -> Result<SonioxState, String> {
    stop_soniox_transcription_impl(app).await
}

async fn start_soniox_transcription_impl(app: AppHandle) -> Result<SonioxState, String> {
    let api_key = keychain::read_provider_key("soniox")?.ok_or_else(|| {
        "No Soniox API key. Add one in Settings → Personas → API keys (free tier at soniox.com).".to_string()
    })?;

    let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Replace any prior session (defensive — the public toggle path
    // already checks but a racy call shouldn't double-open).
    *ACTIVE_SESSION.lock() = Some(SonioxSession {
        chunk_sender: chunk_tx,
        chars_typed: 0,
    });

    let _ = app.emit("soniox_state", "transcribing");

    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        match run_soniox_session(&app_for_task, &api_key, chunk_rx).await {
            Ok(()) => {}
            Err(error) => {
                eprintln!("[soniox] session ended with error: {error}");
                let _ = app_for_task.emit(
                    "soniox_error",
                    format!("Soniox session failed: {error}"),
                );
            }
        }
        // Drop session state on exit so the next toggle starts clean.
        *ACTIVE_SESSION.lock() = None;
        let _ = app_for_task.emit("soniox_state", "idle");
    });

    // The mic stream lives on the panel/dock side — they already pump
    // chunks via `append_soniox_audio_chunk`. We don't start the mic
    // capture here because Soniox can interleave with other voice
    // flows (the user might want both quick-command + transcribe
    // ready). The mic_chunk listener on the frontend forwards while
    // ACTIVE_SESSION is Some.
    crate::audio::start_mic_capture(app).map_err(|e| format!("mic start: {e}"))?;

    Ok(SonioxState {
        is_active: true,
        chars_typed: 0,
    })
}

async fn stop_soniox_transcription_impl(app: AppHandle) -> Result<SonioxState, String> {
    let mut guard = ACTIVE_SESSION.lock();
    let chars_typed = guard.as_ref().map(|s| s.chars_typed).unwrap_or(0);
    *guard = None;
    drop(guard);
    let _ = crate::audio::stop_mic_capture();
    let _ = app.emit("soniox_state", "idle");
    Ok(SonioxState {
        is_active: false,
        chars_typed,
    })
}

#[tauri::command]
pub fn append_soniox_audio_chunk(pcm_bytes: Vec<u8>) -> Result<(), String> {
    let guard = ACTIVE_SESSION.lock();
    if let Some(session) = guard.as_ref() {
        let _ = session.chunk_sender.send(pcm_bytes);
    }
    // Silently drop when no active session — happens during the
    // brief window between stop_soniox_transcription and the
    // frontend's mic_chunk listener unbinding.
    Ok(())
}

async fn run_soniox_session(
    app: &AppHandle,
    api_key: &str,
    mut chunk_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<(), String> {
    // Soniox auths via the Authorization header on the WS upgrade —
    // the older "api_key in first JSON frame" path still works on
    // their current endpoint but the header form is what their
    // docs recommend and what their newer SDKs use, so we send via
    // header to stay forward-compatible. Live test against a real
    // key returned the same 403 shape for both forms, confirming
    // the header is parsed at the upgrade layer.
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(SONIOX_WS_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        // tokio-tungstenite would normally populate these for us;
        // when we hand it a manual Request via builder() we have to
        // include them ourselves or it errors with "missing required
        // websocket headers" before the upgrade attempt.
        .header("Host", "stt-rt.soniox.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .map_err(|e| format!("Soniox request build: {e}"))?;

    let (mut socket, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| {
            // Map the most common error to a clear remedy. "Host not
            // in allowlist" is a Soniox console setting (IP allowlist
            // on the key) — surface it directly so the user knows
            // exactly where to fix it.
            let raw = e.to_string();
            if raw.contains("403") {
                format!(
                    "Soniox returned 403. Most likely your key has an IP allowlist set in the Soniox console; either disable the allowlist or add your machine's IP. Detail: {raw}"
                )
            } else {
                format!("Soniox connect: {raw}")
            }
        })?;

    // First binary frame after the upgrade is the JSON config: no
    // api_key (now in the header) but the audio + model knobs.
    let config = json!({
        "audio_format": "pcm_s16le",
        "sample_rate": SAMPLE_RATE_HZ,
        "num_channels": 1,
        "model": "stt-rt-preview",
        "language_hints": ["en"],
        "enable_language_identification": false,
        "include_nonfinal": true,
        "client_reference_id": "tiptour-cross",
    });
    socket
        .send(Message::Text(config.to_string()))
        .await
        .map_err(|e| format!("Soniox config send: {e}"))?;

    // Spawn the audio-pump task that forwards chunks from the
    // frontend's mpsc channel onto the WebSocket.
    let (mut ws_writer, mut ws_reader) = socket.split();
    let pump_handle = tauri::async_runtime::spawn(async move {
        while let Some(chunk) = chunk_rx.recv().await {
            if ws_writer.send(Message::Binary(chunk)).await.is_err() {
                break;
            }
        }
        // Send the empty-binary EOF Soniox expects when the client
        // is done streaming — they flush the final transcript on
        // receiving this and close the socket cleanly.
        let _ = ws_writer.send(Message::Binary(Vec::new())).await;
    });

    // Outstanding non-final length (number of characters we've
    // pasted from the most recent non-final transcript). Each new
    // non-final replaces the last by sending a backspace burst
    // before pasting the fresh substring. Final tokens are kept and
    // the cursor is left after them.
    let mut nonfinal_pasted_len: usize = 0;
    let mut final_buffer = String::new();

    while let Some(message_result) = ws_reader.next().await {
        let message = match message_result {
            Ok(m) => m,
            Err(error) => return Err(format!("Soniox read: {error}")),
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(error_message) = parsed.get("error_message").and_then(|v| v.as_str()) {
            return Err(format!("Soniox error: {error_message}"));
        }
        // Soniox token shape: { tokens: [{ text, is_final }, ...] }
        let Some(tokens) = parsed.get("tokens").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut new_final = String::new();
        let mut new_nonfinal = String::new();
        for token in tokens {
            let token_text = token.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let is_final = token.get("is_final").and_then(|v| v.as_bool()).unwrap_or(false);
            if is_final {
                new_final.push_str(token_text);
            } else {
                new_nonfinal.push_str(token_text);
            }
        }

        if !new_final.is_empty() || !new_nonfinal.is_empty() {
            // 1. Erase the previously-pasted non-final substring.
            if nonfinal_pasted_len > 0 {
                let backspaces = "\x08".repeat(nonfinal_pasted_len);
                // Use the existing typing path so we drive the
                // foreground app's input rather than fiddling with
                // its AX state.
                let _ = clipboard_paste::paste_text(&backspaces);
            }
            // 2. Paste the new final + non-final.
            let combined = format!("{new_final}{new_nonfinal}");
            if !combined.is_empty() {
                let _ = clipboard_paste::paste_text(&combined);
            }
            nonfinal_pasted_len = new_nonfinal.chars().count();

            if !new_final.is_empty() {
                final_buffer.push_str(&new_final);
                // Update the active session's chars-typed counter for
                // the cost meter.
                if let Some(session) = ACTIVE_SESSION.lock().as_mut() {
                    session.chars_typed = session
                        .chars_typed
                        .saturating_add(new_final.chars().count() as u64);
                }
                let _ = app.emit("soniox_final_chunk", &new_final);
            }
        }

        // If the active session was cleared externally (toggle off),
        // bail out of the read loop. The pump_handle will see the
        // channel closed and exit too.
        if ACTIVE_SESSION.lock().is_none() {
            break;
        }
    }
    drop(pump_handle);
    let _ = final_buffer;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_serializes_camelcase() {
        let json = serde_json::to_string(&SonioxState {
            is_active: true,
            chars_typed: 42,
        })
        .unwrap();
        assert!(json.contains("isActive"));
        assert!(json.contains("charsTyped"));
    }
}
