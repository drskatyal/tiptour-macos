// Microsoft To Do adapter — Graph API.
//
// v1 uses a paste-a-token install (user generates a Graph access
// token via Azure Portal or graph-explorer) so we don't have to ship
// the full OAuth dance. Full PKCE flow lands in v2 alongside Outlook
// + Teams + GCal which all need the same scaffolding.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "ms-to-do".into(),
        name: "Microsoft To Do".into(),
        description: "Add tasks to your default Microsoft To Do list.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "add to my to do".into(),
            "microsoft todo".into(),
            "add to ms to do".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Microsoft Graph access token".into(),
            help_url: "https://developer.microsoft.com/en-us/graph/graph-explorer".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "v1: paste a Graph access token with Tasks.ReadWrite scope. Tokens expire hourly — full PKCE OAuth flow lands in v2.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let token = keychain::read_provider_key("ms-to-do")?.ok_or_else(|| {
        "Microsoft To Do not installed. Paste a Graph token first.".to_string()
    })?;
    match handler {
        "add" => add(&token, args).await,
        other => Err(format!("ms-to-do: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct AddArgs {
    title: String,
    #[serde(default)]
    notes: Option<String>,
}

async fn add(token: &str, args: Value) -> Result<Value, String> {
    let parsed: AddArgs = parse_args(args)?;
    // Resolve the default list id ("Tasks") first. Graph returns
    // it as the first item in /me/todo/lists.
    let list_response = http_client()?
        .get("https://graph.microsoft.com/v1.0/me/todo/lists?$top=1")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("ms-to-do lists: {e}"))?;
    if !list_response.status().is_success() {
        return Err(format!(
            "ms-to-do lists {}: {}",
            list_response.status(),
            list_response.text().await.unwrap_or_default()
        ));
    }
    let list_value: Value = list_response.json().await.map_err(|e| e.to_string())?;
    let list_id = list_value
        .pointer("/value/0/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "ms-to-do: no default list found".to_string())?
        .to_string();

    let url = format!(
        "https://graph.microsoft.com/v1.0/me/todo/lists/{}/tasks",
        list_id
    );
    let mut payload = json!({ "title": parsed.title });
    if let Some(notes) = parsed.notes {
        payload["body"] = json!({ "content": notes, "contentType": "text" });
    }
    let response = http_client()?
        .post(&url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("ms-to-do add: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "ms-to-do add {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(json!({ "id": value.get("id"), "added": true }))
}
