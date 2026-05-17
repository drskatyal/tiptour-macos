// Per-session conversation history.
//
// One JSON file per Gemini Live session, keyed on the session UUID
// the frontend mints at the moment the user opens push-to-talk.
// Each turn (user transcript or model reply) is appended on the fly
// so a hard quit or model error never loses what was already said.
//
// Storage shape (one file per session):
//   {
//     "session_id": "...",
//     "started_at": "<RFC3339>",
//     "title": "<first user line, trimmed to ~60 chars>",
//     "turns": [{ "role": "user"|"model", "text": "...", "at": "<RFC3339>" }]
//   }
//
// The UI lists every session newest-first and lets the user open or
// delete one. Sessions are NOT concatenated into a single rolling
// transcript — keeping them sliced lets us seed only the most recent
// session's tail into a brand-new Live setup without dragging months
// of context into every conversation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const FILENAME_PREFIX: &str = "session-";
const FILENAME_SUFFIX: &str = ".json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: String,
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    pub session_id: String,
    pub started_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub turns: Vec<ConversationTurn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub started_at: String,
    pub title: String,
    pub turn_count: usize,
}

fn sessions_dir() -> Result<PathBuf, String> {
    let mut path =
        dirs::data_local_dir().ok_or_else(|| "no data_local_dir available".to_string())?;
    path.push("TipTour");
    path.push("sessions");
    std::fs::create_dir_all(&path).map_err(|error| format!("create sessions dir: {error}"))?;
    Ok(path)
}

fn session_file_path(session_id: &str) -> Result<PathBuf, String> {
    // Hard-stop path traversal: only accept session ids that look like
    // a UUID-shaped slug (lowercase hex + dashes). The frontend mints
    // these via crypto.randomUUID so this never trips on legit input.
    if !session_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
        || session_id.is_empty()
        || session_id.len() > 64
    {
        return Err(format!("invalid session id: {session_id}"));
    }
    let mut path = sessions_dir()?;
    path.push(format!("{FILENAME_PREFIX}{session_id}{FILENAME_SUFFIX}"));
    Ok(path)
}

fn read_session_file(path: &PathBuf) -> Result<ConversationSession, String> {
    let raw = std::fs::read_to_string(path).map_err(|error| format!("read: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("parse: {error}"))
}

fn write_session_file(path: &PathBuf, session: &ConversationSession) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(session)
        .map_err(|error| format!("serialize: {error}"))?;
    std::fs::write(path, raw).map_err(|error| format!("write: {error}"))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Append one turn to the session file, creating the file if this is
/// the first turn we've seen for this session id.
#[tauri::command]
pub fn append_session_turn(
    session_id: String,
    role: String,
    text: String,
) -> Result<(), String> {
    let path = session_file_path(&session_id)?;
    let mut session = if path.exists() {
        read_session_file(&path)?
    } else {
        ConversationSession {
            session_id: session_id.clone(),
            started_at: now_rfc3339(),
            title: String::new(),
            turns: Vec::new(),
        }
    };
    // First non-trivial user line doubles as the session title so
    // the history list shows something readable instead of a UUID.
    if session.title.is_empty() && role == "user" {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let title: String = trimmed.chars().take(60).collect();
            session.title = title;
        }
    }
    session.turns.push(ConversationTurn {
        role,
        text,
        at: now_rfc3339(),
    });
    write_session_file(&path, &session)
}

/// List every saved session, newest first. Each entry is a summary —
/// no turn contents — so the list view stays cheap to render.
#[tauri::command]
pub fn list_sessions() -> Result<Vec<SessionSummary>, String> {
    let dir = sessions_dir()?;
    let mut summaries: Vec<SessionSummary> = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|error| format!("read dir: {error}"))?;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(FILENAME_PREFIX) || !file_name.ends_with(FILENAME_SUFFIX) {
            continue;
        }
        let path = entry.path();
        if let Ok(session) = read_session_file(&path) {
            summaries.push(SessionSummary {
                session_id: session.session_id,
                started_at: session.started_at,
                title: if session.title.is_empty() {
                    "Untitled session".into()
                } else {
                    session.title
                },
                turn_count: session.turns.len(),
            });
        }
    }
    summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(summaries)
}

/// Return the full turn log for a single session — used by the
/// history viewer.
#[tauri::command]
pub fn get_session_history(session_id: String) -> Result<ConversationSession, String> {
    let path = session_file_path(&session_id)?;
    if !path.exists() {
        return Err(format!("session {session_id} not found"));
    }
    read_session_file(&path)
}

#[tauri::command]
pub fn delete_session(session_id: String) -> Result<(), String> {
    let path = session_file_path(&session_id)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| format!("remove: {error}"))?;
    }
    Ok(())
}

/// Return the last N turns of the most-recently-touched session.
/// The Gemini Live setup uses this to seed continuity ("remember the
/// thing I asked earlier") without dragging every past session in.
#[tauri::command]
pub fn get_last_session_tail(max_turns: Option<usize>) -> Result<Vec<ConversationTurn>, String> {
    let limit = max_turns.unwrap_or(6).clamp(1, 30);
    let summaries = list_sessions()?;
    let Some(latest) = summaries.first() else {
        return Ok(Vec::new());
    };
    let session = get_session_history(latest.session_id.clone())?;
    let start = session.turns.len().saturating_sub(limit);
    Ok(session.turns[start..].to_vec())
}
