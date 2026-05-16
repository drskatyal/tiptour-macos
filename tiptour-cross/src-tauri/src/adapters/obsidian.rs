// Obsidian adapter via the obsidian:// URL scheme.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{open_url_via_os, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "obsidian".into(),
        name: "Obsidian".into(),
        description: "Open and append to notes in your Obsidian vault.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "open in obsidian".into(),
            "obsidian note".into(),
            "add to daily note".into(),
            "append to obsidian".into(),
        ],
        capabilities: vec![AdapterCapability::OpenUri],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Requires Obsidian installed. The first command auto-prompts to pick a vault if Obsidian doesn't have a default set.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open_note" => open_note(args),
        "create_note" => create_note(args),
        "append_to_daily" => append_to_daily(args),
        other => Err(format!("obsidian: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct OpenNoteArgs {
    vault: String,
    file: String,
}
#[derive(Deserialize)]
struct CreateNoteArgs {
    vault: String,
    name: String,
    content: String,
}
#[derive(Deserialize)]
struct AppendDailyArgs {
    vault: String,
    text: String,
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

fn open_note(args: Value) -> Result<Value, String> {
    let parsed: OpenNoteArgs = parse_args(args)?;
    let url = format!(
        "obsidian://open?vault={}&file={}",
        percent_encode(&parsed.vault),
        percent_encode(&parsed.file)
    );
    open_url_via_os(&url)?;
    Ok(json!({ "opened": parsed.file }))
}

fn create_note(args: Value) -> Result<Value, String> {
    let parsed: CreateNoteArgs = parse_args(args)?;
    let url = format!(
        "obsidian://new?vault={}&name={}&content={}",
        percent_encode(&parsed.vault),
        percent_encode(&parsed.name),
        percent_encode(&parsed.content)
    );
    open_url_via_os(&url)?;
    Ok(json!({ "created": parsed.name }))
}

fn append_to_daily(args: Value) -> Result<Value, String> {
    let parsed: AppendDailyArgs = parse_args(args)?;
    // Obsidian's `daily-note` action handler requires the Daily Notes
    // plugin enabled in the vault, which is a default install.
    let url = format!(
        "obsidian://daily?vault={}&append={}",
        percent_encode(&parsed.vault),
        percent_encode(&parsed.text)
    );
    open_url_via_os(&url)?;
    Ok(json!({ "appended": true }))
}
