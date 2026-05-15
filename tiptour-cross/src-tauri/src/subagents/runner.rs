// Sub-agent runner. Each running sub-agent is owned by the TypeScript
// side: the panel listens for `subagent_spawn_request` events and opens
// a fresh `GeminiLiveSession` against the user's Gemini key for each.
// The Rust side keeps the registry, enforces depth + budget + heartbeat
// rules, and persists transcripts.
//
// Why split this way: the existing Gemini Live client lives in
// TypeScript (`src/gemini/GeminiLiveClient.ts`) and the audio bridge
// already terminates on the webview side. Reimplementing the full WS +
// audio + tool-dispatch loop in Rust would double the surface area
// without giving sub-agents any capability the TS client lacks. The
// trade-off is that sub-agent sessions run inside the same webview
// process as the user-facing session — fine for personal-scale (≤ 3
// concurrent), would need rethinking if we ever wanted background
// agents that survive panel close.

use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

use super::pool::{
    subagent_traces_directory, HEARTBEAT_TIMEOUT_SECONDS, SUBAGENT_POOL,
};
use super::types::{Subagent, SubagentProgressEvent, SubagentStatus};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSpawnRequest {
    pub subagent_id: String,
    pub name: String,
    pub task_description: String,
    pub system_prompt: Option<String>,
    pub token_budget_usd: f32,
}

/// Emit a "please open a Gemini Live session for this sub-agent" event
/// to the panel. The panel handler is responsible for actually opening
/// the WebSocket. Returns the same id so the caller can correlate.
pub fn request_panel_run_subagent(app: &AppHandle, subagent: &Subagent) -> Result<(), String> {
    let request = SubagentSpawnRequest {
        subagent_id: subagent.id.clone(),
        name: subagent.name.clone(),
        task_description: subagent.task_description.clone(),
        system_prompt: subagent.system_prompt.clone(),
        token_budget_usd: subagent.token_budget_usd,
    };
    app.emit("subagent_spawn_request", request)
        .map_err(|e| format!("emit subagent_spawn_request: {e}"))?;
    Ok(())
}

/// Append a JSONL line to the sub-agent's transcript file. Best-effort:
/// failures are logged but don't propagate, since transcript loss
/// shouldn't kill an agent run.
pub fn append_transcript_line(subagent_id: &str, role: &str, text: &str) {
    let directory = match transcript_directory_for(subagent_id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[subagent] transcript dir: {e}");
            return;
        }
    };
    if let Err(e) = fs::create_dir_all(&directory) {
        eprintln!("[subagent] mkdir transcript dir: {e}");
        return;
    }
    let path = directory.join("transcript.jsonl");
    let line = serde_json::json!({
        "unix_seconds": Utc::now().timestamp(),
        "role": role,
        "text": text,
    });
    let serialized = match serde_json::to_string(&line) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[subagent] serialize transcript: {e}");
            return;
        }
    };
    match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut handle) => {
            let _ = writeln!(handle, "{serialized}");
        }
        Err(e) => eprintln!("[subagent] open transcript file: {e}"),
    }
}

pub fn transcript_directory_for(subagent_id: &str) -> Result<PathBuf, String> {
    Ok(subagent_traces_directory()?.join(subagent_id))
}

/// Background sweep that the panel can call (or a future scheduled
/// tokio task can call) to fail-out agents whose heartbeat has stalled
/// beyond the timeout. Returns the list of newly-failed ids so the UI
/// can show toasts.
pub fn sweep_stalled_subagents(app: &AppHandle) -> Vec<String> {
    let now = Utc::now().timestamp();
    let mut newly_failed = Vec::new();
    let mut pool = match SUBAGENT_POOL.lock() {
        Ok(g) => g,
        Err(_) => return newly_failed,
    };
    for subagent in pool.subagents.iter_mut() {
        if subagent.status != SubagentStatus::Running {
            continue;
        }
        if now - subagent.last_heartbeat_unix_seconds > HEARTBEAT_TIMEOUT_SECONDS {
            subagent.status = SubagentStatus::Failed;
            subagent.ended_at_unix_seconds = Some(now);
            subagent.last_progress_message =
                Some("heartbeat timeout — no progress for 5 minutes".to_string());
            newly_failed.push(subagent.id.clone());
        }
    }
    drop(pool);
    for failed_id in &newly_failed {
        let event = SubagentProgressEvent {
            subagent_id: failed_id.clone(),
            status: SubagentStatus::Failed,
            message: "heartbeat timeout".to_string(),
            unix_seconds: now,
        };
        let _ = app.emit("subagent_progress", event);
    }
    newly_failed
}
