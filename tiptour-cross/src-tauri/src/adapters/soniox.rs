// Soniox transcription adapter.
//
// The actual WebSocket plumbing lives in `crate::soniox_transcribe`.
// This file just exposes a `control_app(soniox, …)` surface so voice
// commands like "start transcribing" and "stop transcribing" route
// the same way every other adapter call does.
//
// Handlers:
//   start  → start_soniox_transcription
//   stop   → stop_soniox_transcription
//   toggle → toggle_soniox_transcription
//   state  → get_soniox_state

use serde_json::{json, Value};
use tauri::AppHandle;

use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "soniox".into(),
        name: "Soniox transcription".into(),
        description: "Real-time speech-to-text into the focused field. Streams via Soniox's WebSocket API.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "start transcribing".into(),
            "transcribe".into(),
            "stop transcribing".into(),
            "type what i say".into(),
        ],
        capabilities: vec![
            AdapterCapability::Network,
            AdapterCapability::Keychain,
            AdapterCapability::Keystrokes,
            AdapterCapability::Clipboard,
        ],
        auth: AdapterAuth::ApiKey {
            label: "Soniox API key".into(),
            help_url: "https://console.soniox.com/".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Get a Soniox API key at console.soniox.com (free tier covers typical use). \
            Paste it here, then trigger via Alt+Z, the dock 'Transcribe' button, or by saying \
            'start transcribing'. Tokens stream into whatever field has focus; non-final \
            tokens auto-correct as Soniox refines.".into(),
    }
}

pub async fn dispatch(app: AppHandle, handler: &str, _args: Value) -> Result<Value, String> {
    // Each handler is a thin pass-through to the soniox_transcribe
    // module's tauri::command-marked functions. We don't double-mark
    // them here; the module already exposes them as Tauri commands
    // for the dock + main panel.
    match handler {
        "start" => {
            let state = crate::soniox_transcribe::start_soniox_transcription(app).await?;
            Ok(json!({ "started": true, "state": state }))
        }
        "stop" => {
            let state = crate::soniox_transcribe::stop_soniox_transcription(app).await?;
            Ok(json!({ "stopped": true, "state": state }))
        }
        "toggle" => {
            let state = crate::soniox_transcribe::toggle_soniox_transcription(app).await?;
            Ok(json!({ "state": state }))
        }
        "state" => {
            let state = crate::soniox_transcribe::get_soniox_state();
            Ok(json!({ "state": state }))
        }
        other => Err(format!("soniox: no handler '{other}'")),
    }
}
