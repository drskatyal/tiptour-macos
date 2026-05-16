// macOS Finder adapter — open / reveal / select via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "finder".into(),
        name: "Finder".into(),
        description: "Open and reveal files and folders in Finder.".into(),
        category: "files".into(),
        voice_triggers: vec![
            "open folder".into(),
            "open in finder".into(),
            "reveal in finder".into(),
            "show in finder".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript, AdapterCapability::OpenUri],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Paths can be POSIX (/Users/you/…) or HFS-style (Macintosh HD:Users:you:…).".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open_path" => open_path(args),
        "reveal_path" => reveal_path(args),
        other => Err(format!("finder: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct PathArgs {
    path: String,
}

fn open_path(args: Value) -> Result<Value, String> {
    let parsed: PathArgs = parse_args(args)?;
    let path = osa_escape(&parsed.path);
    let script = format!(
        "tell application \"Finder\" to open (POSIX file \"{path}\")"
    );
    run_osascript(&script)?;
    Ok(json!({ "opened": parsed.path }))
}

fn reveal_path(args: Value) -> Result<Value, String> {
    let parsed: PathArgs = parse_args(args)?;
    let path = osa_escape(&parsed.path);
    let script = format!(
        "tell application \"Finder\"\n\
            activate\n\
            reveal (POSIX file \"{path}\")\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "revealed": parsed.path }))
}
