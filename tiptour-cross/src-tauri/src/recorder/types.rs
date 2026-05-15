// Shared types for the demonstration recorder + passive pattern miner.
// Mirrored on the TS side in `src/recorder/types.ts` so the frontend can
// render saved demonstrations and surface "you've done this N times" prompts
// without re-deriving the schema.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InputEvent {
    #[serde(rename_all = "camelCase")]
    KeyDown { key_code: u32, key_name: String },
    #[serde(rename_all = "camelCase")]
    KeyUp { key_code: u32, key_name: String },
    #[serde(rename_all = "camelCase")]
    MouseClick {
        button: String,
        x: f64,
        y: f64,
    },
    #[serde(rename_all = "camelCase")]
    MouseMove { x: f64, y: f64 },
    #[serde(rename_all = "camelCase")]
    Scroll { delta_x: f64, delta_y: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusedElementFingerprint {
    pub role: Option<String>,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub control_type: Option<String>,
    pub is_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub foreground_app_bundle_identifier: Option<String>,
    pub foreground_app_executable_path: Option<String>,
    pub foreground_app_display_name: Option<String>,
    pub foreground_window_title: Option<String>,
    pub uia_tree_fingerprint: Option<String>,
    pub focused_element: Option<FocusedElementFingerprint>,
    pub selected_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceEntry {
    pub timestamp_unix_ms: i64,
    pub event: InputEvent,
    pub snapshot_before: Option<StateSnapshot>,
    pub snapshot_after: Option<StateSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingMode {
    Passive,
    Demonstration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Demonstration {
    pub id: String,
    pub title: String,
    pub narration_audio_path: Option<String>,
    pub trace: Vec<TraceEntry>,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemonstrationSummary {
    pub id: String,
    pub title: String,
    pub created_at_unix_ms: i64,
    pub trace_entry_count: usize,
    pub has_narration_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPattern {
    pub occurrences: usize,
    pub mean_interval_ms: i64,
    pub suggested_name: String,
    pub representative_event_kinds: Vec<String>,
    pub k_gram_length: usize,
    // 1.0 = exact-match k-gram (legacy `mine_patterns_exact`). Lower
    // values reflect the fuzzy miner's centroid similarity across the
    // group, computed as `1.0 - mean_normalized_levenshtein_distance`.
    pub similarity_score: f32,
}
