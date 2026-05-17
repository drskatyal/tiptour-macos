// macOS Terminal.app adapter — new window with a command. OSA.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "terminal-macos".into(),
        name: "Terminal".into(),
        description: "Open a new Terminal window and run a command.".into(),
        category: "dev".into(),
        voice_triggers: vec!["open terminal".into(), "run in terminal".into(), "new terminal".into()],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives Terminal.app via AppleScript. The command runs in a new window/tab.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "run" => run(args),
        "open_cwd" => open_cwd(args),
        other => Err(format!("terminal-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs { command: String, #[serde(default)] cwd: Option<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CwdArgs { path: String }

fn run(args: Value) -> Result<Value, String> {
    let parsed: RunArgs = parse_args(args)?;
    let cmd = osa_escape(&parsed.command);
    let prelude = match parsed.cwd.as_deref() {
        Some(cwd) if !cwd.is_empty() => format!("cd {} && ", osa_escape(cwd)),
        _ => String::new(),
    };
    let full = format!("{}{}", prelude, cmd);
    let script = format!(
        "tell application \"Terminal\"\n\
            activate\n\
            do script \"{}\"\n\
        end tell",
        osa_escape(&full)
    );
    run_osascript(&script)?;
    Ok(json!({ "ran": parsed.command }))
}

fn open_cwd(args: Value) -> Result<Value, String> {
    let parsed: CwdArgs = parse_args(args)?;
    let path = osa_escape(&parsed.path);
    let script = format!(
        "tell application \"Terminal\"\n\
            activate\n\
            do script \"cd \\\"{path}\\\"\"\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "opened": parsed.path }))
}
