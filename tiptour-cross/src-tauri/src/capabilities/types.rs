// Shared types for the capability graph and tool registry. Mirrored on the
// TS side in `src/capabilities/types.ts` — field names are camelCase because
// every struct here uses #[serde(rename_all = "camelCase")] so the JSON
// shape that crosses the Tauri bridge matches TS idioms directly.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SafetyClassification {
    Safe,
    Destructive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    #[serde(rename_all = "camelCase")]
    Click { element_name: String, element_path: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Shortcut { chord_raw: String },
    #[serde(rename_all = "camelCase")]
    Type { target_element_name: String, text_parameter_name: String },
    #[serde(rename_all = "camelCase")]
    Scroll { direction: String, magnitude: i32 },
    #[serde(rename_all = "camelCase")]
    Navigate { menu_path: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateNode {
    pub state_id: String,
    pub foreground_window_title: Option<String>,
    pub element_count: usize,
    pub discovered_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from_state_id: String,
    pub to_state_id: String,
    pub action: Action,
    pub safety: SafetyClassification,
    // One-way edges are recorded when the explorer could not return to the
    // origin state by undo or back-navigation — useful for the planner so it
    // does not pick a return path that doesn't exist.
    pub is_one_way: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSchema {
    // JSON-Schema-shaped param definition — kept as a flat map to avoid
    // pulling a full JSON-Schema crate. Each entry's value is an object
    // with "type" and "description" keys.
    pub properties: HashMap<String, serde_json::Value>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub capability_id: String,
    pub canonical_name: String,
    pub description: String,
    pub parameter_schema: ParameterSchema,
    pub safety: SafetyClassification,
    // Replay recipe is a sequence of edges (by index into the graph) that
    // reproduces the capability. We persist the actions themselves so the
    // recipe survives graph compaction.
    pub replay_actions: Vec<Action>,
    // Bag-of-words searchable tags lifted from element names, menu paths
    // and the canonical name. TODO: replace with embeddings.
    pub keywords: Vec<String>,
    // Per-app state preconditions. Each entry is an opaque token (often a
    // `state_<hash>` reachability tag, but also app-specific predicates
    // like "document_loaded") — the registry drops capabilities whose
    // preconditions are unmet at retrieval time. Capabilities discovered
    // from the root state carry an empty list and are always retrievable.
    #[serde(default)]
    pub preconditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityGraph {
    pub app_identifier: String,
    pub app_version: Option<String>,
    pub nodes: Vec<StateNode>,
    pub edges: Vec<Edge>,
    pub captured_at_unix_ms: i64,
}

// ToolSchema is what we hand to Gemini. It's a leaner projection of
// Capability that strips replay internals and matches Gemini's tool format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchema {
    pub tool_id: String,
    pub name: String,
    pub description: String,
    pub parameter_schema: ParameterSchema,
    pub safety: SafetyClassification,
}

// Returned from invoke_capability — a concrete sequence of executable
// actions with the user-supplied parameter values substituted in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPlan {
    pub tool_id: String,
    pub actions: Vec<Action>,
    pub safety: SafetyClassification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreSummary {
    pub app_identifier: String,
    pub nodes_discovered: usize,
    pub edges_discovered: usize,
    pub capabilities_extracted: usize,
    pub stopped_reason: String,
    pub elapsed_ms: u128,
}
