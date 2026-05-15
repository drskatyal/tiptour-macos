// Centralized Gemini tool-call dispatcher. Both the user-facing panel
// session (driven from TypeScript) and the Rust-side sub-agent
// conversational loop funnel their tool calls through here so the tool
// surface stays in one place.
//
// The Tauri command `dispatch_tool_call` exposes the same dispatcher to
// the TypeScript panel session — that lets `GeminiLiveSession.ts`
// collapse its hand-rolled per-tool branches into one `invoke` call.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::agent_memory;
use crate::executor;
use crate::multiflow;
use crate::subagents;
use crate::tasks;

/// Rust-internal entry point. Called from the sub-agent conversational
/// loop. `parent_depth` is the depth of the *calling* sub-agent so the
/// recursive spawn check can compare `parent_depth + 1` against the cap.
pub async fn dispatch_subagent_tool_call(
    _subagent_id: &str,
    tool_name: &str,
    args: Value,
    parent_depth: u32,
    app: AppHandle,
) -> Value {
    dispatch_inner(tool_name, args, Some(parent_depth), app).await
}

#[tauri::command]
pub async fn dispatch_tool_call(name: String, args: Value, app: AppHandle) -> Value {
    // Top-level (panel) caller has no enclosing sub-agent depth, so any
    // `spawn_subagent` call from here creates a root sub-agent (depth 0).
    dispatch_inner(&name, args, None, app).await
}

async fn dispatch_inner(
    tool_name: &str,
    args: Value,
    parent_depth_if_subagent: Option<u32>,
    app: AppHandle,
) -> Value {
    match tool_name {
        "submit_workflow_plan" => match executor::execute_workflow_plan(args, app).await {
            Ok(workflow_id) => json!({ "status": "started", "workflowId": workflow_id }),
            Err(message) => error_response(&message),
        },

        // Agent memory tools -------------------------------------------------
        "remember" => {
            let key = string_arg(&args, "key");
            let value = string_arg(&args, "value");
            let tags = array_of_strings(&args, "tags");
            match agent_memory::remember(key, value, Some(tags), Some("gemini".to_string())) {
                Ok(record) => json!({ "status": "ok", "record": record }),
                Err(message) => error_response(&message),
            }
        }
        "recall" => {
            let query = string_arg(&args, "query");
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).map(|v| v as usize);
            match agent_memory::recall(query, top_k) {
                Ok(records) => json!({ "status": "ok", "records": records }),
                Err(message) => error_response(&message),
            }
        }
        "forget" => {
            let memory_id = string_arg(&args, "memory_id");
            match agent_memory::forget(memory_id) {
                Ok(()) => json!({ "status": "ok" }),
                Err(message) => error_response(&message),
            }
        }
        "list_memories" => {
            let tag = args.get("tag").and_then(|v| v.as_str()).map(String::from);
            match agent_memory::list_memories(tag) {
                Ok(records) => json!({ "status": "ok", "records": records }),
                Err(message) => error_response(&message),
            }
        }

        // Sub-agent tools ----------------------------------------------------
        "spawn_subagent" => {
            // Depth enforcement: when called from inside a sub-agent, the
            // caller's depth + 1 must be within the max. The
            // `subagents::spawn_subagent` helper performs its own check
            // too, but we duplicate it here so we can emit a clean
            // structured error before doing any state mutation.
            if let Some(parent_depth) = parent_depth_if_subagent {
                let projected_child_depth = parent_depth + 1;
                if projected_child_depth > subagents::MAX_SUBAGENT_DEPTH {
                    return error_response("depth limit reached");
                }
            }
            let name = string_arg(&args, "name");
            let task_description = string_arg(&args, "task_description");
            // When the caller is itself a sub-agent we'd want to thread
            // its id through as `parent_subagent_id`, but the dispatcher
            // only knows the depth, not the id. The conversational loop
            // sets it via the wrapper variant below.
            match subagents::spawn_subagent(app, name, task_description, None, None, None) {
                Ok(subagent_id) => json!({ "status": "ok", "subagentId": subagent_id }),
                Err(message) => error_response(&message),
            }
        }
        "list_subagents" => match subagents::list_subagents() {
            Ok(items) => json!({ "status": "ok", "subagents": items }),
            Err(message) => error_response(&message),
        },
        "cancel_subagent" => {
            let subagent_id = string_arg(&args, "subagent_id");
            match subagents::cancel_subagent(app, subagent_id) {
                Ok(()) => json!({ "status": "ok" }),
                Err(message) => error_response(&message),
            }
        }

        // Task tools ---------------------------------------------------------
        "create_task" => {
            let title = string_arg(&args, "title");
            let description = args
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            match tasks::create_task(title, description, None, None, None) {
                Ok(task) => json!({ "status": "ok", "task": task }),
                Err(message) => error_response(&message),
            }
        }
        "update_task_status" => {
            let task_id = string_arg(&args, "task_id");
            let status_str = string_arg(&args, "status");
            let parsed_status: Result<crate::tasks::TaskStatus, _> =
                serde_json::from_value(Value::String(status_str.clone()));
            match parsed_status {
                Ok(status_value) => match tasks::update_task_status(task_id, status_value) {
                    Ok(task) => json!({ "status": "ok", "task": task }),
                    Err(message) => error_response(&message),
                },
                Err(_) => error_response(&format!("unknown task status '{status_str}'")),
            }
        }
        "list_tasks" => {
            let status_filter = args.get("status").and_then(|v| v.as_str()).and_then(|s| {
                serde_json::from_value::<crate::tasks::TaskStatus>(Value::String(s.to_string())).ok()
            });
            match tasks::list_tasks(status_filter, None) {
                Ok(items) => json!({ "status": "ok", "tasks": items }),
                Err(message) => error_response(&message),
            }
        }

        // Multiflow ----------------------------------------------------------
        "run_saved_flow" => {
            let flow_name = string_arg(&args, "name");
            match multiflow::run_flow_by_name(flow_name, app).await {
                Ok(replay_id) => json!({ "status": "started", "replayId": replay_id }),
                Err(message) => error_response(&message),
            }
        }

        other => json!({
            "status": "unknown_tool",
            "message": format!("Tool '{other}' is not implemented."),
        }),
    }
}

fn error_response(message: &str) -> Value {
    json!({ "status": "error", "message": message })
}

fn string_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn array_of_strings(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
