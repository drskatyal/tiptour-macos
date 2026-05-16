// macOS Mail.app adapter via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "mail-macos".into(),
        name: "Mail.app".into(),
        description: "Compose and send email via the built-in Mail app you're already signed into.".into(),
        category: "communication".into(),
        voice_triggers: vec![
            "send email".into(),
            "compose email".into(),
            "draft email".into(),
            "new mail".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives Mail.app via AppleScript. No additional setup — the message goes from your default account.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "compose" => compose(args, /* send_immediately */ false),
        "send" => compose(args, /* send_immediately */ true),
        other => Err(format!("mail-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct ComposeArgs {
    to: String,
    #[serde(default)]
    cc: Option<String>,
    subject: String,
    body: String,
}

fn compose(args: Value, send_immediately: bool) -> Result<Value, String> {
    let parsed: ComposeArgs = parse_args(args)?;
    let to = osa_escape(&parsed.to);
    let subject = osa_escape(&parsed.subject);
    let body = osa_escape(&parsed.body);
    let cc_line = match parsed.cc.as_deref() {
        Some(cc) if !cc.is_empty() => format!(
            "make new cc recipient at end of cc recipients with properties {{address:\"{}\"}}",
            osa_escape(cc)
        ),
        _ => String::new(),
    };
    let send_line = if send_immediately { "send newMessage" } else { "activate" };
    let script = format!(
        "tell application \"Mail\"\n\
            set newMessage to make new outgoing message with properties {{subject:\"{subject}\", content:\"{body}\", visible:true}}\n\
            tell newMessage\n\
                make new to recipient at end of to recipients with properties {{address:\"{to}\"}}\n\
                {cc_line}\n\
            end tell\n\
            {send_line}\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "sent": send_immediately }))
}
