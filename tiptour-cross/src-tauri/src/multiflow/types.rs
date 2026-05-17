// Multiflow types. A "flow" is a saved demonstration the user can recall
// by voice — same on-disk demonstration directory as the recorder writes,
// but indexed under a human-friendly name with optional trigger aliases.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    /// Matches the underlying demonstration_id so the recorder's on-disk
    /// layout (`demonstrations/{id}/`) doubles as the flow's storage root.
    pub flow_id: String,
    pub name: String,
    pub created_at_unix_ms: i64,
    pub step_count: usize,
    /// Alternate phrases the voice matcher should accept for this flow —
    /// e.g. ["morning routine", "start of day", "kick off the day"].
    pub trigger_aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReplayProgressKind {
    Started,
    InputReplayed,
    AppLaunched,
    Waited,
    Paused { reason: String },
    Completed,
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayProgress {
    pub flow_id: String,
    pub replay_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub kind: ReplayProgressKind,
}
