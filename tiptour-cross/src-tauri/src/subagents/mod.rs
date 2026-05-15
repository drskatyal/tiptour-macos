// Sub-agent fleet subsystem.
//
// Architecture (see `runner.rs` for the full rationale): Rust owns the
// authoritative registry, depth/budget/heartbeat enforcement, transcript
// persistence, and lifecycle commands. The TypeScript panel owns the
// actual Gemini Live WebSocket per sub-agent — it listens for
// `subagent_spawn_request` events and opens fresh sessions. Progress
// events flow back via `report_subagent_progress`.
//
// This split keeps the sub-agent surface honest about what's wired
// end-to-end: registry + lifecycle + persistence are real; the actual
// Gemini conversation runs through the same TS client that powers the
// user-facing session.

mod pool;
mod runner;
mod types;

use chrono::Utc;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub use types::{Subagent, SubagentProgressEvent, SubagentStatus};

use pool::{DEFAULT_TOKEN_BUDGET_USD, MAX_SUBAGENT_DEPTH, SUBAGENT_POOL};

/// Internal helper used by both the Tauri command and the
/// `tasks::dispatch_task_to_subagent` integration. Returns the new
/// sub-agent id.
pub fn spawn_subagent(
    app: AppHandle,
    name: String,
    task_description: String,
    parent_subagent_id: Option<String>,
    system_prompt_override: Option<String>,
    token_budget_usd: Option<f32>,
) -> Result<String, String> {
    let now = Utc::now().timestamp();
    let new_id = Uuid::new_v4().to_string();
    let new_depth = match &parent_subagent_id {
        Some(parent_id) => {
            let pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
            let parent = pool_guard
                .find_by_id(parent_id)
                .ok_or_else(|| format!("parent subagent {parent_id} not found"))?;
            let next = parent.depth + 1;
            if next > MAX_SUBAGENT_DEPTH {
                return Err(format!(
                    "subagent depth {next} exceeds max {MAX_SUBAGENT_DEPTH}"
                ));
            }
            next
        }
        None => 0,
    };
    let new_subagent = Subagent {
        id: new_id.clone(),
        name,
        task_description,
        system_prompt: system_prompt_override,
        status: SubagentStatus::Pending,
        parent_subagent_id: parent_subagent_id.clone(),
        depth: new_depth,
        token_budget_usd: token_budget_usd.unwrap_or(DEFAULT_TOKEN_BUDGET_USD),
        spent_usd: 0.0,
        started_at_unix_seconds: now,
        ended_at_unix_seconds: None,
        last_heartbeat_unix_seconds: now,
        child_subagent_ids: Vec::new(),
        last_progress_message: None,
    };

    {
        let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
        pool_guard.subagents.push(new_subagent.clone());
        pool_guard.pending_queue.push_back(new_id.clone());
        if let Some(parent_id) = &parent_subagent_id {
            if let Some(parent_mut) = pool_guard.find_by_id_mut(parent_id) {
                parent_mut.child_subagent_ids.push(new_id.clone());
            }
        }
    }

    // Promote queued sub-agents up to the concurrency cap. If our new
    // sub-agent immediately gets promoted, ask the panel to run it.
    promote_and_dispatch_pending(&app)?;
    Ok(new_id)
}

fn promote_and_dispatch_pending(app: &AppHandle) -> Result<(), String> {
    loop {
        let promoted_id = {
            let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
            pool_guard.promote_pending_if_room()
        };
        match promoted_id {
            None => break,
            Some(id) => {
                let snapshot = {
                    let pool_guard =
                        SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
                    pool_guard
                        .find_by_id(&id)
                        .cloned()
                        .ok_or_else(|| format!("subagent {id} vanished after promotion"))?
                };
                runner::request_panel_run_subagent(app, &snapshot)?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn spawn_subagent_command(
    app: AppHandle,
    name: String,
    task_description: String,
    parent_subagent_id: Option<String>,
    system_prompt_override: Option<String>,
    token_budget_usd: Option<f32>,
) -> Result<String, String> {
    spawn_subagent(
        app,
        name,
        task_description,
        parent_subagent_id,
        system_prompt_override,
        token_budget_usd,
    )
}

#[tauri::command]
pub fn list_subagents() -> Result<Vec<Subagent>, String> {
    let pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
    let mut output = pool_guard.subagents.clone();
    output.sort_by(|a, b| b.started_at_unix_seconds.cmp(&a.started_at_unix_seconds));
    Ok(output)
}

#[tauri::command]
pub fn get_subagent_status(id: String) -> Result<Subagent, String> {
    let pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
    pool_guard
        .find_by_id(&id)
        .cloned()
        .ok_or_else(|| format!("subagent {id} not found"))
}

#[tauri::command]
pub fn cancel_subagent(app: AppHandle, id: String) -> Result<(), String> {
    let now = Utc::now().timestamp();
    {
        let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
        let subagent = pool_guard
            .find_by_id_mut(&id)
            .ok_or_else(|| format!("subagent {id} not found"))?;
        subagent.status = SubagentStatus::Failed;
        subagent.ended_at_unix_seconds = Some(now);
        subagent.last_progress_message = Some("cancelled by user".to_string());
        // Drop from pending queue if it never got promoted.
        pool_guard.pending_queue.retain(|q_id| q_id != &id);
    }
    let event = SubagentProgressEvent {
        subagent_id: id.clone(),
        status: SubagentStatus::Failed,
        message: "cancelled".to_string(),
        unix_seconds: now,
    };
    let _ = app.emit("subagent_progress", event);
    // Tell the TS side to actually close the socket for this id.
    let _ = app.emit("subagent_cancel_request", id);
    promote_and_dispatch_pending(&app)?;
    Ok(())
}

#[tauri::command]
pub fn pause_subagent(app: AppHandle, id: String) -> Result<(), String> {
    let now = Utc::now().timestamp();
    {
        let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
        let subagent = pool_guard
            .find_by_id_mut(&id)
            .ok_or_else(|| format!("subagent {id} not found"))?;
        if subagent.status != SubagentStatus::Running {
            return Err("can only pause a Running sub-agent".to_string());
        }
        subagent.status = SubagentStatus::Paused;
    }
    let event = SubagentProgressEvent {
        subagent_id: id.clone(),
        status: SubagentStatus::Paused,
        message: "paused".to_string(),
        unix_seconds: now,
    };
    let _ = app.emit("subagent_progress", event);
    promote_and_dispatch_pending(&app)?;
    Ok(())
}

#[tauri::command]
pub fn resume_subagent(app: AppHandle, id: String) -> Result<(), String> {
    let now = Utc::now().timestamp();
    {
        let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
        let subagent = pool_guard
            .find_by_id_mut(&id)
            .ok_or_else(|| format!("subagent {id} not found"))?;
        if subagent.status != SubagentStatus::Paused {
            return Err("can only resume a Paused sub-agent".to_string());
        }
        // Resume goes back to Pending so the pool's concurrency cap
        // re-applies the next promotion sweep.
        subagent.status = SubagentStatus::Pending;
        subagent.last_heartbeat_unix_seconds = now;
        pool_guard.pending_queue.push_back(id.clone());
    }
    promote_and_dispatch_pending(&app)?;
    Ok(())
}

/// Called by the TS side as the sub-agent makes progress. Updates
/// heartbeat + status + transcript on disk, then re-emits as a single
/// canonical `subagent_progress` event for the UI.
#[tauri::command]
pub fn report_subagent_progress(
    app: AppHandle,
    subagent_id: String,
    status: SubagentStatus,
    message: String,
    spent_usd_delta: Option<f32>,
) -> Result<(), String> {
    let now = Utc::now().timestamp();
    let mut budget_exceeded = false;
    {
        let mut pool_guard = SUBAGENT_POOL.lock().map_err(|_| "pool poisoned".to_string())?;
        let subagent = pool_guard
            .find_by_id_mut(&subagent_id)
            .ok_or_else(|| format!("subagent {subagent_id} not found"))?;
        subagent.last_heartbeat_unix_seconds = now;
        subagent.last_progress_message = Some(message.clone());
        if let Some(delta) = spent_usd_delta {
            subagent.spent_usd += delta.max(0.0);
            if subagent.spent_usd > subagent.token_budget_usd {
                budget_exceeded = true;
            }
        }
        match status {
            SubagentStatus::Done | SubagentStatus::Failed => {
                subagent.ended_at_unix_seconds = Some(now);
                subagent.status = status;
            }
            _ => {
                subagent.status = status;
            }
        }
    }
    runner::append_transcript_line(&subagent_id, "progress", &message);
    let event = SubagentProgressEvent {
        subagent_id: subagent_id.clone(),
        status,
        message,
        unix_seconds: now,
    };
    let _ = app.emit("subagent_progress", event);

    if budget_exceeded {
        // Hard-stop: cancel the sub-agent and let the TS side tear down
        // its socket. We log to stderr (not stdout) so the message
        // doesn't pollute release builds' user-visible streams.
        eprintln!(
            "[subagent] {subagent_id} exceeded token budget; cancelling",
        );
        cancel_subagent(app, subagent_id)?;
    } else {
        promote_and_dispatch_pending(&app)?;
    }
    Ok(())
}

#[tauri::command]
pub fn append_subagent_transcript(
    subagent_id: String,
    role: String,
    text: String,
) -> Result<(), String> {
    runner::append_transcript_line(&subagent_id, &role, &text);
    Ok(())
}

#[tauri::command]
pub fn sweep_stalled_subagents_command(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(runner::sweep_stalled_subagents(&app))
}

#[tauri::command]
pub fn get_subagent_transcript_path(subagent_id: String) -> Result<String, String> {
    let directory = runner::transcript_directory_for(&subagent_id)?;
    Ok(directory.join("transcript.jsonl").to_string_lossy().into_owned())
}
