// macOS Messages.app adapter — iMessage / SMS via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "messages-macos".into(),
        name: "Messages".into(),
        description: "Send iMessage / SMS through the built-in Messages app you're signed into.".into(),
        category: "communication".into(),
        voice_triggers: vec![
            "send message".into(),
            "imessage".into(),
            "send text".into(),
            "text someone".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Requires Messages.app signed into iCloud. Sending via SMS needs Text Message Forwarding from your iPhone.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "send" => send(args),
        other => Err(format!("messages-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct SendArgs {
    /// Phone, email, or contact name (Messages.app resolves names
    /// against the Contacts app on its own).
    to: String,
    text: String,
}

fn send(args: Value) -> Result<Value, String> {
    let parsed: SendArgs = parse_args(args)?;
    let to = osa_escape(&parsed.to);
    let text = osa_escape(&parsed.text);
    // Messages 'send' takes either a buddy ref or a buddy id. We
    // resolve by phone/email via Messages' own buddy lookup.
    let script = format!(
        "tell application \"Messages\"\n\
            set targetService to 1st account whose service type = iMessage\n\
            set targetBuddy to participant \"{to}\" of targetService\n\
            send \"{text}\" to targetBuddy\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "sent": true }))
}
