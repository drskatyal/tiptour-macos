// iWork suite — Pages / Numbers / Keynote via OSA. No auth.
//
// All three apps share the same minimal command surface: open a
// document, create a new document. Each ships as a separate manifest
// so the settings UI groups them next to the matching Office apps.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

fn iwork_manifest(slug: &str, name: &str, description: &str, triggers: Vec<String>) -> AdapterManifest {
    AdapterManifest {
        slug: slug.into(),
        name: name.into(),
        description: description.into(),
        category: "productivity".into(),
        voice_triggers: triggers,
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives the iWork app via AppleScript. No setup — works with the version that shipped with macOS.".into(),
    }
}

pub fn pages_manifest() -> AdapterManifest {
    iwork_manifest(
        "pages",
        "Pages",
        "Create and open Pages documents.",
        vec!["new pages document".into(), "open pages".into(), "create a doc in pages".into()],
    )
}
pub fn numbers_manifest() -> AdapterManifest {
    iwork_manifest(
        "numbers",
        "Numbers",
        "Create and open Numbers spreadsheets.",
        vec!["new numbers spreadsheet".into(), "open numbers".into(), "create a sheet in numbers".into()],
    )
}
pub fn keynote_manifest() -> AdapterManifest {
    iwork_manifest(
        "keynote",
        "Keynote",
        "Create and open Keynote presentations.",
        vec!["new keynote".into(), "open keynote".into(), "start a presentation".into()],
    )
}

pub async fn dispatch_pages(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    dispatch_app("Pages", handler, args)
}
pub async fn dispatch_numbers(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    dispatch_app("Numbers", handler, args)
}
pub async fn dispatch_keynote(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    dispatch_app("Keynote", handler, args)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenArgs {
    path: String,
}

fn dispatch_app(app_name: &str, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "new_document" => {
            let script = format!(
                "tell application \"{app_name}\"\n\
                    activate\n\
                    make new document\n\
                end tell"
            );
            run_osascript(&script)?;
            Ok(json!({ "created": true }))
        }
        "open" => {
            let parsed: OpenArgs = parse_args(args)?;
            let path = osa_escape(&parsed.path);
            let script = format!(
                "tell application \"{app_name}\"\n\
                    activate\n\
                    open (POSIX file \"{path}\")\n\
                end tell"
            );
            run_osascript(&script)?;
            Ok(json!({ "opened": parsed.path }))
        }
        other => Err(format!("{}: no handler '{other}'", app_name.to_lowercase())),
    }
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placeholder {}
