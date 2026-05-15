// Real Gemini Live conversational loop for a single sub-agent. Owns its
// own websocket end-to-end inside the Tauri host process so the sub-agent
// keeps running even if the user closes the panel webview.
//
// Flow per sub-agent:
//   1. Build a `ClientOptions` with `disable_audio_output = true`
//      (sub-agents are text-only — no mic, no TTS).
//   2. Connect to Gemini Live and wait for setupComplete.
//   3. Send `task_description` as the first user turn.
//   4. Pump inbound messages forever:
//      - Output transcript text → append to transcript JSONL +
//        progress event.
//      - Tool call → dispatch through `tool_dispatch`, send response
//        back to Gemini.
//      - UsageReport → accumulate dollar cost; if budget exceeded,
//        cancel + flip status to Failed.
//      - TurnComplete → if the last model turn looks done, flip to Done.
//      - Error / Closed → fail the sub-agent.
//   5. Between turns, check the per-sub-agent cancel flag and bail if
//      the user (or the budget enforcer) flipped it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use super::pool::{DEFAULT_TOKEN_BUDGET_USD, SUBAGENT_POOL};
use super::runner::append_transcript_line;
use super::types::{SubagentProgressEvent, SubagentStatus};
use crate::gemini_live_client::{ClientOptions, GeminiLiveClient, InboundMessage};
use crate::tool_dispatch::dispatch_subagent_tool_call;

// Per-token Gemini Flash Live pricing snapshot used for budget enforcement.
// Numbers come from the user's brief; keep them in one place so a price
// change is a single edit. Values are USD per token.
const INPUT_TOKEN_USD: f64 = 0.000003;
const OUTPUT_TOKEN_USD: f64 = 0.000015;

const DEFAULT_SUBAGENT_MODEL_SHORT_ID: &str = "gemini-3.1-flash-live-preview";

// Registry of per-sub-agent cancel flags. The `cancel_subagent` Tauri
// command flips the flag; the conversational loop checks it between
// inbound messages and bails out cleanly.
static CANCEL_FLAGS: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn install_cancel_flag_for(subagent_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut guard = CANCEL_FLAGS.lock().await;
    guard.insert(subagent_id.to_string(), flag.clone());
    flag
}

pub async fn request_cancel(subagent_id: &str) {
    let guard = CANCEL_FLAGS.lock().await;
    if let Some(flag) = guard.get(subagent_id) {
        flag.store(true, Ordering::SeqCst);
    }
}

async fn clear_cancel_flag(subagent_id: &str) {
    let mut guard = CANCEL_FLAGS.lock().await;
    guard.remove(subagent_id);
}

#[derive(Debug, Clone)]
pub enum SubagentOutcome {
    CompletedSuccessfully,
    Cancelled,
    BudgetExceeded,
    SessionError(String),
}

/// Drive one sub-agent's Gemini Live conversation from start to finish.
/// Returns the outcome so `finalize_subagent` can write the right final
/// status to the pool.
pub async fn run_subagent_conversational_loop(
    subagent_id: String,
    task_description: String,
    system_prompt: String,
    api_key: String,
    parent_depth: u32,
    token_budget_usd: f64,
    app: AppHandle,
) -> SubagentOutcome {
    let cancel_flag = install_cancel_flag_for(&subagent_id).await;

    // The conversational-loop entry recursive depth cap. The dispatcher
    // also enforces this on `spawn_subagent`, but checking here catches
    // any future programmatic entry points that bypass the dispatcher.
    if parent_depth > super::pool::MAX_SUBAGENT_DEPTH {
        let message = format!(
            "subagent depth {parent_depth} exceeds max {}",
            super::pool::MAX_SUBAGENT_DEPTH
        );
        append_transcript_line(&subagent_id, "system", &message);
        clear_cancel_flag(&subagent_id).await;
        return SubagentOutcome::SessionError(message);
    }

    let tool_declarations = build_subagent_tool_declarations();

    let client_options = ClientOptions {
        api_key,
        model_id: DEFAULT_SUBAGENT_MODEL_SHORT_ID.to_string(),
        // Voice name is unused with disable_audio_output=true, but the
        // setup payload accepts an arbitrary string so we pass a sane
        // placeholder.
        voice_name: "Aoede".to_string(),
        system_instruction: system_prompt,
        tool_declarations,
        disable_audio_output: true,
    };

    let (client, mut inbound) = match GeminiLiveClient::connect(client_options).await {
        Ok(pair) => pair,
        Err(connect_error) => {
            append_transcript_line(
                &subagent_id,
                "system",
                &format!("connect failed: {connect_error}"),
            );
            emit_progress(&app, &subagent_id, SubagentStatus::Failed, &connect_error);
            clear_cancel_flag(&subagent_id).await;
            return SubagentOutcome::SessionError(connect_error);
        }
    };

    // First turn: deliver the task to Gemini.
    append_transcript_line(&subagent_id, "user", &task_description);
    if let Err(send_error) = client.send_text_turn(&task_description).await {
        append_transcript_line(
            &subagent_id,
            "system",
            &format!("first turn send failed: {send_error}"),
        );
        client.close().await;
        clear_cancel_flag(&subagent_id).await;
        return SubagentOutcome::SessionError(send_error);
    }
    emit_progress(&app, &subagent_id, SubagentStatus::Running, "task dispatched");

    // Loop state.
    let mut accumulated_input_tokens: u64 = 0;
    let mut accumulated_output_tokens: u64 = 0;
    // Tracks whether the model emitted at least one tool call during the
    // current turn so we can interpret a "no tool call this turn" as the
    // model deciding it's done.
    let mut tool_calls_emitted_this_turn: u32 = 0;
    let mut last_model_text_this_turn = String::new();

    let outcome = loop {
        // Cancel check between messages — fast, no contention beyond a
        // tokio mutex acquire that only runs at message boundaries.
        if cancel_flag.load(Ordering::SeqCst) {
            append_transcript_line(&subagent_id, "system", "cancelled");
            break SubagentOutcome::Cancelled;
        }

        let inbound_message = match inbound.recv().await {
            Some(message) => message,
            None => {
                // Channel closed — Gemini hung up or the reader task
                // exited. Treat as an error rather than a graceful end.
                break SubagentOutcome::SessionError("inbound channel closed".to_string());
            }
        };

        match inbound_message {
            InboundMessage::SetupComplete => {
                // Already handled inside connect(); ignore the duplicate.
            }
            InboundMessage::AudioChunk { .. } => {
                // Sub-agents request TEXT modality so this shouldn't fire.
                // If it does, drop silently — playing audio for a
                // background sub-agent would be confusing UX.
            }
            InboundMessage::InputTranscript { text } => {
                append_transcript_line(&subagent_id, "user", &text);
            }
            InboundMessage::OutputTranscript { text } => {
                last_model_text_this_turn.push_str(&text);
                append_transcript_line(&subagent_id, "model", &text);
                emit_progress(&app, &subagent_id, SubagentStatus::Running, &text);
            }
            InboundMessage::ToolCall {
                name,
                args,
                tool_call_id,
            } => {
                tool_calls_emitted_this_turn += 1;
                append_transcript_line(
                    &subagent_id,
                    "tool_call",
                    &format!("{name} {args}"),
                );
                let response_value = dispatch_subagent_tool_call(
                    &subagent_id,
                    &name,
                    args,
                    parent_depth,
                    app.clone(),
                )
                .await;
                append_transcript_line(
                    &subagent_id,
                    "tool_response",
                    &response_value.to_string(),
                );
                if let Err(send_error) =
                    client.send_tool_response(&tool_call_id, response_value).await
                {
                    append_transcript_line(
                        &subagent_id,
                        "system",
                        &format!("tool response send failed: {send_error}"),
                    );
                    break SubagentOutcome::SessionError(send_error);
                }
            }
            InboundMessage::UsageReport {
                input_tokens,
                output_tokens,
            } => {
                // The server reports cumulative-per-frame totals on most
                // frames once context exists. We accumulate the max
                // observed to guard against double-counting if the server
                // ever emits a non-monotonic delta.
                accumulated_input_tokens =
                    accumulated_input_tokens.max(input_tokens as u64);
                accumulated_output_tokens =
                    accumulated_output_tokens.max(output_tokens as u64);
                let total_cost_usd = (accumulated_input_tokens as f64) * INPUT_TOKEN_USD
                    + (accumulated_output_tokens as f64) * OUTPUT_TOKEN_USD;
                update_spent_usd(&subagent_id, total_cost_usd);
                if total_cost_usd > token_budget_usd {
                    append_transcript_line(
                        &subagent_id,
                        "system",
                        &format!(
                            "budget exceeded: ${total_cost_usd:.4} > ${token_budget_usd:.4}"
                        ),
                    );
                    break SubagentOutcome::BudgetExceeded;
                }
            }
            InboundMessage::TurnComplete => {
                // Heuristic for "the agent is finished": no tool call this
                // turn, the last model text non-empty, and ends with a
                // sentence terminator. We avoid declaring done on a
                // model turn that was purely a tool call because Gemini
                // typically wants another round-trip after a tool result.
                let trimmed = last_model_text_this_turn.trim();
                let looks_final = tool_calls_emitted_this_turn == 0
                    && !trimmed.is_empty()
                    && (trimmed.ends_with('.')
                        || trimmed.ends_with('!')
                        || trimmed.ends_with('?')
                        || trimmed.to_lowercase().contains("task_completed"));
                tool_calls_emitted_this_turn = 0;
                last_model_text_this_turn.clear();
                if looks_final {
                    break SubagentOutcome::CompletedSuccessfully;
                }
            }
            InboundMessage::Error { message } => {
                append_transcript_line(
                    &subagent_id,
                    "system",
                    &format!("ws error: {message}"),
                );
                break SubagentOutcome::SessionError(message);
            }
            InboundMessage::Closed { reason } => {
                append_transcript_line(
                    &subagent_id,
                    "system",
                    &format!("ws closed: {reason}"),
                );
                // A clean close after the model declared itself done
                // would normally not arrive here because TurnComplete
                // already broke us out. If we get here it means the
                // server hung up unexpectedly.
                break SubagentOutcome::SessionError(format!("ws closed: {reason}"));
            }
        }
    };

    client.close().await;
    clear_cancel_flag(&subagent_id).await;
    outcome
}

/// Apply the dollar-cost total to the sub-agent's pool record. This is a
/// best-effort update — a poisoned mutex shouldn't kill an in-flight
/// session, since the loop's own budget enforcement runs against its
/// local accumulator regardless.
fn update_spent_usd(subagent_id: &str, total_cost_usd: f64) {
    let pool_guard = SUBAGENT_POOL.lock();
    let mut pool = match pool_guard {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if let Some(subagent_mut) = pool.find_by_id_mut(subagent_id) {
        subagent_mut.spent_usd = total_cost_usd as f32;
        subagent_mut.last_heartbeat_unix_seconds = Utc::now().timestamp();
    }
}

fn emit_progress(app: &AppHandle, subagent_id: &str, status: SubagentStatus, message: &str) {
    let event = SubagentProgressEvent {
        subagent_id: subagent_id.to_string(),
        status,
        message: message.to_string(),
        unix_seconds: Utc::now().timestamp(),
    };
    let _ = app.emit("subagent_progress", event);
}

/// Build the function-declarations JSON the sub-agent's Gemini session
/// gets at setup time. Mirrors the panel session's surface (see
/// `src/gemini/GeminiLiveClient.ts`) so a sub-agent has every tool the
/// user-facing session has, including recursive `spawn_subagent`.
fn build_subagent_tool_declarations() -> Value {
    json!([
        {
            "functionDeclarations": [
                {
                    "name": "submit_workflow_plan",
                    "description": "Submit a multi-step CUA plan to drive the user's computer.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "goal": { "type": "string" },
                            "app": { "type": "string" },
                            "steps": { "type": "array", "items": { "type": "object" } }
                        },
                        "required": ["steps"]
                    }
                },
                {
                    "name": "remember",
                    "description": "Save a fact to the agent's persistent memory.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "value": { "type": "string" },
                            "tags": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["key", "value"]
                    }
                },
                {
                    "name": "recall",
                    "description": "Semantic search over the agent's memory.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "top_k": { "type": "number" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "forget",
                    "description": "Soft-delete a memory by id.",
                    "parameters": {
                        "type": "object",
                        "properties": { "memory_id": { "type": "string" } },
                        "required": ["memory_id"]
                    }
                },
                {
                    "name": "list_memories",
                    "description": "List stored memories, optionally filtered by tag.",
                    "parameters": {
                        "type": "object",
                        "properties": { "tag": { "type": "string" } }
                    }
                },
                {
                    "name": "spawn_subagent",
                    "description": "Spawn a parallel sub-agent for an independent task.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "task_description": { "type": "string" }
                        },
                        "required": ["name", "task_description"]
                    }
                },
                {
                    "name": "list_subagents",
                    "description": "List active and recent sub-agents.",
                    "parameters": { "type": "object", "properties": {} }
                },
                {
                    "name": "cancel_subagent",
                    "description": "Cancel a running sub-agent by id.",
                    "parameters": {
                        "type": "object",
                        "properties": { "subagent_id": { "type": "string" } },
                        "required": ["subagent_id"]
                    }
                },
                {
                    "name": "create_task",
                    "description": "Create a kanban task for later work.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["title"]
                    }
                },
                {
                    "name": "update_task_status",
                    "description": "Move a task between kanban columns.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "task_id": { "type": "string" },
                            "status": { "type": "string" }
                        },
                        "required": ["task_id", "status"]
                    }
                },
                {
                    "name": "list_tasks",
                    "description": "List tasks, optionally filtered by status.",
                    "parameters": {
                        "type": "object",
                        "properties": { "status": { "type": "string" } }
                    }
                },
                {
                    "name": "run_saved_flow",
                    "description": "Replay a previously-recorded flow by name.",
                    "parameters": {
                        "type": "object",
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"]
                    }
                }
            ]
        }
    ])
}

/// Default token budget exposed for callers that want the same default
/// the pool uses but in `f64`.
#[allow(dead_code)]
pub fn default_token_budget_usd_as_f64() -> f64 {
    DEFAULT_TOKEN_BUDGET_USD as f64
}
