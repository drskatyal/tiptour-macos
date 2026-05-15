// Sub-agent value types. A sub-agent is a child Gemini Live session
// spawned by the parent agent (or the user) to chase down an
// independent task in parallel.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubagentStatus {
    Pending,
    Running,
    Paused,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub id: String,
    pub name: String,
    pub task_description: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub status: SubagentStatus,
    #[serde(default)]
    pub parent_subagent_id: Option<String>,
    pub depth: u32,
    pub token_budget_usd: f32,
    #[serde(default)]
    pub spent_usd: f32,
    pub started_at_unix_seconds: i64,
    #[serde(default)]
    pub ended_at_unix_seconds: Option<i64>,
    pub last_heartbeat_unix_seconds: i64,
    #[serde(default)]
    pub child_subagent_ids: Vec<String>,
    #[serde(default)]
    pub last_progress_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProgressEvent {
    pub subagent_id: String,
    pub status: SubagentStatus,
    pub message: String,
    pub unix_seconds: i64,
}
