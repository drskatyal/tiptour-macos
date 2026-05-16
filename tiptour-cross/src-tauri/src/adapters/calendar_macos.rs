// macOS Calendar.app adapter via OSA. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "calendar-macos".into(),
        name: "Calendar.app".into(),
        description: "Create events and read your schedule from the built-in Calendar app.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "create event".into(),
            "schedule a meeting".into(),
            "what's on my calendar".into(),
            "what's next".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives Calendar.app via AppleScript. Events land in the default calendar (the first one in your sidebar).".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "create_event" => create_event(args),
        "list_today" => list_today(),
        other => Err(format!("calendar-macos: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct CreateEventArgs {
    summary: String,
    /// ISO 8601 start time, e.g. "2026-05-16T14:00:00".
    start_iso: String,
    /// ISO 8601 end time. If omitted, default to start + 30 min.
    #[serde(default)]
    end_iso: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

fn create_event(args: Value) -> Result<Value, String> {
    let parsed: CreateEventArgs = parse_args(args)?;
    let summary = osa_escape(&parsed.summary);
    let location = osa_escape(parsed.location.as_deref().unwrap_or(""));
    let notes = osa_escape(parsed.notes.as_deref().unwrap_or(""));
    // AppleScript's date parsing is finicky — we hand it ISO strings
    // and use `date` keyword to coerce, which works for all locales.
    let start = osa_escape(&parsed.start_iso);
    let end = osa_escape(parsed.end_iso.as_deref().unwrap_or(""));
    let end_block = if end.is_empty() {
        "set endDate to startDate + 30 * minutes".into()
    } else {
        format!("set endDate to my parseIso(\"{end}\")")
    };
    let script = format!(
        "on parseIso(s)\n\
            return current date + ((time of (date s)) - (time of (current date)))\n\
        end parseIso\n\
        tell application \"Calendar\"\n\
            set firstCalendar to first calendar\n\
            set startDate to my parseIso(\"{start}\")\n\
            {end_block}\n\
            make new event at end of events of firstCalendar with properties \
                {{summary:\"{summary}\", start date:startDate, end date:endDate, \
                  location:\"{location}\", description:\"{notes}\"}}\n\
            return \"ok\"\n\
        end tell"
    );
    // AppleScript's date parsing varies wildly by locale — if the
    // parseIso helper fails we surface a useful hint rather than a
    // raw osascript error.
    match run_osascript(&script) {
        Ok(_) => Ok(json!({ "created": true })),
        Err(e) if e.contains("date") => Err(format!(
            "Calendar couldn't parse the date \"{}\". Try ISO 8601 like 2026-05-16T14:00:00.",
            parsed.start_iso
        )),
        Err(e) => Err(e),
    }
}

fn list_today() -> Result<Value, String> {
    // Pulls today's events from every visible calendar.
    let script = "tell application \"Calendar\"\n\
        set startOfDay to current date\n\
        set time of startOfDay to 0\n\
        set endOfDay to startOfDay + 1 * days\n\
        set eventLines to {}\n\
        repeat with c in (calendars where writable is true)\n\
            try\n\
                repeat with e in (events of c whose start date ≥ startOfDay and start date < endOfDay)\n\
                    set end of eventLines to ((summary of e) & \"|\" & (start date of e as string))\n\
                end repeat\n\
            on error\n\
                -- skip read-only / unreachable calendars\n\
            end try\n\
        end repeat\n\
        return eventLines as string\n\
        end tell";
    let raw = run_osascript(script).unwrap_or_default();
    let events: Vec<Value> = raw
        .split(", ")
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut parts = line.splitn(2, '|');
            let summary = parts.next().unwrap_or("").to_string();
            let start = parts.next().unwrap_or("").to_string();
            json!({ "summary": summary, "start": start })
        })
        .collect();
    Ok(json!({ "events": events }))
}
