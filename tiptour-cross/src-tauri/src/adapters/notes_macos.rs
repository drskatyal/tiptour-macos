// macOS Notes.app adapter via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "notes-macos".into(),
        name: "Notes.app".into(),
        description: "Create and append to notes in the built-in Notes app.".into(),
        category: "productivity".into(),
        voice_triggers: vec!["new note".into(), "add note".into(), "create note".into(), "save note".into()],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives Notes.app. Notes land in your iCloud default folder.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "create" => create(args),
        "append" => append(args),
        other => Err(format!("notes-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct CreateArgs {
    title: String,
    body: String,
}
#[derive(Deserialize)]
struct AppendArgs {
    title: String,
    body: String,
}

fn create(args: Value) -> Result<Value, String> {
    let parsed: CreateArgs = parse_args(args)?;
    let title = osa_escape(&parsed.title);
    let body = osa_escape(&parsed.body);
    let script = format!(
        "tell application \"Notes\"\n\
            make new note with properties {{name:\"{title}\", body:\"{body}\"}}\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "created": true }))
}

fn append(args: Value) -> Result<Value, String> {
    let parsed: AppendArgs = parse_args(args)?;
    let title = osa_escape(&parsed.title);
    let body = osa_escape(&parsed.body);
    // Find a note whose name matches; create one if it doesn't exist.
    let script = format!(
        "tell application \"Notes\"\n\
            set matches to (notes whose name is \"{title}\")\n\
            if (count of matches) is 0 then\n\
                make new note with properties {{name:\"{title}\", body:\"{body}\"}}\n\
            else\n\
                set existingNote to item 1 of matches\n\
                set body of existingNote to (body of existingNote) & \"<br>\" & \"{body}\"\n\
            end if\n\
            return \"ok\"\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "appended": true }))
}
