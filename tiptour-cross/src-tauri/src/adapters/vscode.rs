// VS Code / Cursor adapter — `code` / `cursor` CLI + URL scheme.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{open_url_via_os, parse_args, spawn_cli};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "vscode".into(),
        name: "VS Code / Cursor".into(),
        description: "Open files, folders, and goto-line targets in VS Code or Cursor.".into(),
        category: "dev".into(),
        voice_triggers: vec![
            "open in vs code".into(),
            "open in cursor".into(),
            "open file in editor".into(),
            "go to file".into(),
        ],
        capabilities: vec![AdapterCapability::Spawn, AdapterCapability::OpenUri],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Requires `code` (VS Code) or `cursor` (Cursor) on PATH. Install via the editor's command palette → 'Shell Command: Install code in PATH'.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open" => open(args),
        "open_in_cursor" => open_in_cursor(args),
        "goto_line" => goto_line(args),
        other => Err(format!("vscode: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct OpenArgs {
    path: String,
}
#[derive(Deserialize)]
struct GotoLineArgs {
    path: String,
    line: u32,
    #[serde(default)]
    column: u32,
}

fn open(args: Value) -> Result<Value, String> {
    let parsed: OpenArgs = parse_args(args)?;
    spawn_cli("code", &[&parsed.path]).or_else(|_| {
        // Fallback to the vscode:// URL scheme if the CLI isn't on PATH.
        open_url_via_os(&format!("vscode://file/{}", parsed.path)).map(|_| String::new())
    })?;
    Ok(json!({ "opened": parsed.path }))
}

fn open_in_cursor(args: Value) -> Result<Value, String> {
    let parsed: OpenArgs = parse_args(args)?;
    spawn_cli("cursor", &[&parsed.path]).or_else(|_| {
        open_url_via_os(&format!("cursor://file/{}", parsed.path)).map(|_| String::new())
    })?;
    Ok(json!({ "opened": parsed.path }))
}

fn goto_line(args: Value) -> Result<Value, String> {
    let parsed: GotoLineArgs = parse_args(args)?;
    let column = if parsed.column == 0 { 1 } else { parsed.column };
    let arg = format!("{}:{}:{}", parsed.path, parsed.line, column);
    spawn_cli("code", &["-g", &arg])?;
    Ok(json!({ "opened": parsed.path, "line": parsed.line }))
}
