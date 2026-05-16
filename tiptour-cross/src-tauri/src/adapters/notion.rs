// Notion adapter — Web API with integration token. v1: create-page +
// append-to-page; search lands in v2.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

const NOTION_VERSION: &str = "2022-06-28";

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "notion".into(),
        name: "Notion".into(),
        description: "Create pages and append blocks to your Notion workspace.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "add to notion".into(),
            "append to notion".into(),
            "create notion page".into(),
            "new notion".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Notion integration token".into(),
            help_url: "https://www.notion.so/my-integrations".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Create an internal integration at the linked URL, copy the secret token, and share the target page or database with the integration so it has write access.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let token = keychain::read_provider_key("notion")?.ok_or_else(|| {
        "Notion not installed. Paste a token in Settings → Connected apps.".to_string()
    })?;
    match handler {
        "create_page" => create_page(&token, args).await,
        "append_text" => append_text(&token, args).await,
        other => Err(format!("notion: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePageArgs {
    /// Parent page id (UUID with no dashes is fine — Notion accepts both).
    parent_page_id: String,
    title: String,
    /// Markdown-ish body. We split lines into Notion paragraph blocks.
    body: String,
}

async fn create_page(token: &str, args: Value) -> Result<Value, String> {
    let parsed: CreatePageArgs = parse_args(args)?;
    let blocks: Vec<Value> = parsed
        .body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            json!({
                "object": "block",
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [{ "type": "text", "text": { "content": line } }]
                }
            })
        })
        .collect();
    let body = json!({
        "parent": { "page_id": parsed.parent_page_id },
        "properties": {
            "title": [{ "type": "text", "text": { "content": parsed.title } }]
        },
        "children": blocks,
    });
    let response = http_client()?
        .post("https://api.notion.com/v1/pages")
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("notion: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "notion {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(json!({ "id": value.get("id"), "url": value.get("url") }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendTextArgs {
    page_id: String,
    text: String,
}

async fn append_text(token: &str, args: Value) -> Result<Value, String> {
    let parsed: AppendTextArgs = parse_args(args)?;
    let blocks: Vec<Value> = parsed
        .text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            json!({
                "object": "block",
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [{ "type": "text", "text": { "content": line } }]
                }
            })
        })
        .collect();
    let body = json!({ "children": blocks });
    let url = format!("https://api.notion.com/v1/blocks/{}/children", parsed.page_id);
    let response = http_client()?
        .patch(&url)
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("notion append: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "notion append {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    Ok(json!({ "appended": true }))
}
