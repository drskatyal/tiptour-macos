// Shared types for the grounding layer. Mirrored on the TS side in
// `src/grounding/types.ts` so the frontend can speak the same vocabulary
// when feeding context into Gemini and dispatching tool-call results.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetApp {
    pub process_id: i32,
    pub bundle_identifier: Option<String>,
    pub executable_path: Option<String>,
    pub display_name: Option<String>,
    pub file_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyChord {
    // Human-readable form e.g. "Ctrl+Shift+P" — kept verbatim from UIA so
    // we can both render hints in UI and feed back into ActionExecutor.
    pub raw: String,
    pub modifiers: Vec<String>,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub menu_path: Vec<String>,
    pub label: String,
    pub accelerator: KeyChord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutIndex {
    pub application_identifier: String,
    pub executable_path: Option<String>,
    pub file_version: Option<String>,
    pub captured_at_unix_ms: i64,
    pub bindings: Vec<ShortcutBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedTarget {
    #[serde(rename_all = "camelCase")]
    Coordinate { x: f64, y: f64 },
    #[serde(rename_all = "camelCase")]
    Shortcut { chord: KeyChord },
}
