// Windows File Explorer adapter — `explorer.exe`. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::parse_args;
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "file-explorer".into(),
        name: "File Explorer".into(),
        description: "Open and reveal files and folders in Windows Explorer.".into(),
        category: "files".into(),
        voice_triggers: vec![
            "open folder".into(),
            "open explorer".into(),
            "show in explorer".into(),
            "reveal in explorer".into(),
        ],
        capabilities: vec![AdapterCapability::Spawn],
        auth: AdapterAuth::None,
        supported_platforms: vec!["windows".into()],
        setup_notes: "Uses explorer.exe — no setup needed.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open_path" => open_path(args),
        "reveal_path" => reveal_path(args),
        other => Err(format!("file-explorer: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct PathArgs {
    path: String,
}

fn open_path(args: Value) -> Result<Value, String> {
    let parsed: PathArgs = parse_args(args)?;
    // `explorer.exe <path>` opens the folder, or for a file opens
    // its containing folder. Use the silent /n flag for a clean window.
    std::process::Command::new("explorer.exe")
        .args(["/n,", &parsed.path])
        .spawn()
        .map_err(|error| format!("explorer.exe: {error}"))?;
    Ok(json!({ "opened": parsed.path }))
}

fn reveal_path(args: Value) -> Result<Value, String> {
    let parsed: PathArgs = parse_args(args)?;
    // /select,<path> selects the item inside its parent folder.
    std::process::Command::new("explorer.exe")
        .args(["/select,", &parsed.path])
        .spawn()
        .map_err(|error| format!("explorer.exe /select: {error}"))?;
    Ok(json!({ "revealed": parsed.path }))
}
