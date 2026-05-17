// Slack adapter — Web API with user or bot token.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "slack".into(),
        name: "Slack".into(),
        description: "Post messages and DMs to your Slack workspace.".into(),
        category: "communication".into(),
        voice_triggers: vec![
            "send slack".into(),
            "post to slack".into(),
            "message on slack".into(),
            "slack dm".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Slack user OAuth token (xoxp-…)".into(),
            help_url: "https://api.slack.com/apps".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Create a Slack app, install it to your workspace, copy the user OAuth token (needs chat:write scope). User tokens post as you; bot tokens post as the app.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let token = keychain::read_provider_key("slack")?
        .ok_or_else(|| "Slack not installed. Paste a token first.".to_string())?;
    match handler {
        "post_message" => post_message(&token, args).await,
        "post_dm" => post_dm(&token, args).await,
        other => Err(format!("slack: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostMessageArgs {
    /// Channel id (C…) or channel name (#general — Slack resolves).
    channel: String,
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostDmArgs {
    /// User id (U…) — DMs require the user id, not display name.
    user_id: String,
    text: String,
}

async fn post_message(token: &str, args: Value) -> Result<Value, String> {
    let parsed: PostMessageArgs = parse_args(args)?;
    post(token, &parsed.channel, &parsed.text).await
}

async fn post_dm(token: &str, args: Value) -> Result<Value, String> {
    let parsed: PostDmArgs = parse_args(args)?;
    // conversations.open returns a DM channel id for the user, then
    // we post into that channel.
    let open = http_client()?
        .post("https://slack.com/api/conversations.open")
        .bearer_auth(token)
        .json(&json!({ "users": parsed.user_id }))
        .send()
        .await
        .map_err(|e| format!("slack open: {e}"))?;
    let open_value: Value = open.json().await.map_err(|e| e.to_string())?;
    if !open_value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(format!("slack conversations.open: {open_value}"));
    }
    let channel_id = open_value
        .pointer("/channel/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "slack: no DM channel id".to_string())?
        .to_string();
    post(token, &channel_id, &parsed.text).await
}

async fn post(token: &str, channel: &str, text: &str) -> Result<Value, String> {
    let response = http_client()?
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(token)
        .json(&json!({ "channel": channel, "text": text }))
        .send()
        .await
        .map_err(|e| format!("slack post: {e}"))?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(format!(
            "slack: {}",
            value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ));
    }
    Ok(json!({ "ts": value.get("ts"), "channel": value.get("channel") }))
}
