// Safari adapter — open URL, get current URL/title, new tab. OSA.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "safari".into(),
        name: "Safari".into(),
        description: "Open URLs, read the current page, manage tabs in Safari.".into(),
        category: "files".into(),
        voice_triggers: vec![
            "open in safari".into(),
            "new safari tab".into(),
            "what's this page about".into(),
            "current safari url".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives Safari via AppleScript. Reading page text requires 'Allow JavaScript from Apple Events' in Safari → Develop menu.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open_url" => open_url(args),
        "new_tab" => new_tab(args),
        "current_url" => current_url(),
        "current_title" => current_title(),
        other => Err(format!("safari: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct OpenUrlArgs { url: String }

fn open_url(args: Value) -> Result<Value, String> {
    let parsed: OpenUrlArgs = parse_args(args)?;
    let url = osa_escape(&parsed.url);
    let script = format!(
        "tell application \"Safari\"\n\
            activate\n\
            set URL of current tab of window 1 to \"{url}\"\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "opened": parsed.url }))
}

fn new_tab(args: Value) -> Result<Value, String> {
    let parsed: OpenUrlArgs = parse_args(args)?;
    let url = osa_escape(&parsed.url);
    let script = format!(
        "tell application \"Safari\"\n\
            activate\n\
            tell window 1 to set newTab to make new tab with properties {{URL:\"{url}\"}}\n\
            set current tab of window 1 to newTab\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "opened": parsed.url }))
}

fn current_url() -> Result<Value, String> {
    let raw = run_osascript("tell application \"Safari\" to return URL of current tab of window 1")?;
    Ok(json!({ "url": raw }))
}

fn current_title() -> Result<Value, String> {
    let raw = run_osascript("tell application \"Safari\" to return name of current tab of window 1")?;
    Ok(json!({ "title": raw }))
}
