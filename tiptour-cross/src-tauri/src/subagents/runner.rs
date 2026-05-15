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

use super::conversational_loop::{run_subagent_conversational_loop, SubagentOutcome};
use super::pool::{
    subagent_traces_directory, HEARTBEAT_TIMEOUT_SECONDS, SUBAGENT_POOL,
};
use super::types::{Subagent, SubagentProgressEvent, SubagentStatus};

const DEFAULT_SUBAGENT_SYSTEM_PROMPT: &str = "You are a sub-agent of TipTour, working in the background on a focused task. You can call tools to read/write memory, spawn child sub-agents, manage tasks, run saved flows, and submit CUA workflow plans. Work efficiently. When you finish the task, summarize the result in plain text and end your final message with a period. Do not request a tool call on your final turn.";

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSpawnRequest {
    pub subagent_id: String,
    pub name: String,
    pub task_description: String,
    pub system_prompt: Option<String>,
    pub token_budget_usd: f32,
}

/// Promote a pending sub-agent into a real running Gemini Live session
/// inside the Tauri host process. We spawn the conversational loop on a
/// Tokio task and let it drive its own websocket; the pool record is
/// updated by `finalize_subagent` when the loop returns. Also emits a
/// `subagent_spawn_request` event so any UI listener that wants to know
/// when a sub-agent starts can react (the previous TS-driven contract).
pub fn request_panel_run_subagent(app: &AppHandle, subagent: &Subagent) -> Result<(), String> {
    let request = SubagentSpawnRequest {
        subagent_id: subagent.id.clone(),
        name: subagent.name.clone(),
        task_description: subagent.task_description.clone(),
        system_prompt: subagent.system_prompt.clone(),
        token_budget_usd: subagent.token_budget_usd,
    };
    // Informational only — the actual conversation runs in-process.
    let _ = app.emit("subagent_spawn_request", request);

    // Pull the Gemini API key out of the OS keychain. Without one we
    // can't connect, so the sub-agent flips to Failed immediately.
    let api_key = match crate::keychain::get_api_key() {
        Ok(Some(key)) if !key.is_empty() => key,
        Ok(_) => {
            finalize_subagent(
                app,
                &subagent.id,
                SubagentOutcome::SessionError("no Gemini API key in keychain".to_string()),
            );
            return Ok(());
        }
        Err(keychain_error) => {
            finalize_subagent(
                app,
                &subagent.id,
                SubagentOutcome::SessionError(format!("keychain read: {keychain_error}")),
            );
            return Ok(());
        }
    };

    let subagent_id = subagent.id.clone();
    let task_description = subagent.task_description.clone();
    let resolved_system_prompt = subagent
        .system_prompt
        .clone()
        .unwrap_or_else(|| DEFAULT_SUBAGENT_SYSTEM_PROMPT.to_string());
    let parent_depth = subagent.depth;
    let token_budget_usd = subagent.token_budget_usd as f64;
    let app_for_loop = app.clone();
    let app_for_finalize = app.clone();
    let subagent_id_for_finalize = subagent_id.clone();

    tauri::async_runtime::spawn(async move {
        let outcome = run_subagent_conversational_loop(
            subagent_id.clone(),
            task_description,
            resolved_system_prompt,
            api_key,
            parent_depth,
            token_budget_usd,
            app_for_loop,
        )
        .await;
        finalize_subagent(&app_for_finalize, &subagent_id_for_finalize, outcome);
    });

    Ok(())
}

/// Apply the loop's outcome to the pool record, stamp the end time, and
/// emit a final `subagent_progress` event so the kanban UI can flip to
/// Done / Failed.
pub fn finalize_subagent(app: &AppHandle, subagent_id: &str, outcome: SubagentOutcome) {
    let now = Utc::now().timestamp();
    let (final_status, final_message) = match &outcome {
        SubagentOutcome::CompletedSuccessfully => (SubagentStatus::Done, "completed".to_string()),
        SubagentOutcome::Cancelled => (SubagentStatus::Failed, "cancelled".to_string()),
        SubagentOutcome::BudgetExceeded => {
            (SubagentStatus::Failed, "budget exceeded".to_string())
        }
        SubagentOutcome::SessionError(message) => {
            (SubagentStatus::Failed, format!("session error: {message}"))
        }
    };
    if let Ok(mut pool_guard) = SUBAGENT_POOL.lock() {
        if let Some(subagent_mut) = pool_guard.find_by_id_mut(subagent_id) {
            subagent_mut.status = final_status;
            subagent_mut.ended_at_unix_seconds = Some(now);
            subagent_mut.last_progress_message = Some(final_message.clone());
            // If a task is linked to this sub-agent, flip the task's
            // status to match so the kanban view updates in lockstep.
            if let Some(linked_task_id) =
                find_task_linked_to_subagent(subagent_id)
            {
                let next_task_status = match final_status {
                    SubagentStatus::Done => crate::tasks::TaskStatus::Done,
                    _ => crate::tasks::TaskStatus::Blocked,
                };
                let _ = crate::tasks::update_task_status(linked_task_id, next_task_status);
            }
        }
    }
    let event = SubagentProgressEvent {
        subagent_id: subagent_id.to_string(),
        status: final_status,
        message: final_message,
        unix_seconds: now,
    };
    let _ = app.emit("subagent_progress", event);
    // Free a concurrency slot for any pending sub-agent.
    let _ = super::promote_and_dispatch_pending_for_runner(app);
}

/// Look up whether any kanban task points at the given sub-agent id.
/// Used by `finalize_subagent` to mirror Done/Failed onto the linked
/// task without forcing every caller of the loop to know about tasks.
fn find_task_linked_to_subagent(subagent_id: &str) -> Option<String> {
    let all_tasks = crate::tasks::list_tasks(None, None).ok()?;
    all_tasks
        .into_iter()
        .find(|t| t.assigned_subagent_id.as_deref() == Some(subagent_id))
        .map(|t| t.id)
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
