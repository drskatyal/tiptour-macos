// macOS Reminders.app adapter via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "reminders-macos".into(),
        name: "Reminders".into(),
        description: "Add reminders by voice to the default Reminders list.".into(),
        category: "productivity".into(),
        voice_triggers: vec!["remind me".into(), "add reminder".into(), "create reminder".into()],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Adds reminders to the first list in Reminders.app.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "add" => add(args),
        "list_today" => list_today(),
        other => Err(format!("reminders-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    text: String,
    /// Optional ISO 8601 due date.
    #[serde(default)]
    due_iso: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

fn add(args: Value) -> Result<Value, String> {
    let parsed: AddArgs = parse_args(args)?;
    let text = osa_escape(&parsed.text);
    let notes = osa_escape(parsed.notes.as_deref().unwrap_or(""));
    let due_block = match parsed.due_iso.as_deref() {
        Some(due) if !due.is_empty() => format!(
            "set due date of newReminder to (date \"{}\")",
            osa_escape(due)
        ),
        _ => String::new(),
    };
    let script = format!(
        "tell application \"Reminders\"\n\
            set defaultList to first list\n\
            set newReminder to make new reminder at end of reminders of defaultList \
                with properties {{name:\"{text}\", body:\"{notes}\"}}\n\
            {due_block}\n\
            return \"ok\"\n\
        end tell"
    );
    run_osascript(&script)?;
    Ok(json!({ "added": true }))
}

fn list_today() -> Result<Value, String> {
    let script = "tell application \"Reminders\"\n\
        set incompleteToday to {}\n\
        repeat with r in (every reminder of first list whose completed is false)\n\
            set end of incompleteToday to (name of r)\n\
        end repeat\n\
        return incompleteToday as string\n\
        end tell";
    let raw = run_osascript(script).unwrap_or_default();
    let items: Vec<&str> = raw.split(", ").filter(|s| !s.is_empty()).collect();
    Ok(json!({ "items": items }))
}
