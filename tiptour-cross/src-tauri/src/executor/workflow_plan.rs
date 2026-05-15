// Serde schema for the workflow plans Gemini emits via the
// `submit_workflow_plan` tool call. Mirrors `TipTour/WorkflowPlan.swift`.
//
// We accept the strict shape and a flexible fallback shape (LLMs sometimes
// emit `action` instead of `type`, `target`/`element` instead of `label`,
// etc.). The flexible payload is normalized into the strict shape so the
// runner only has to handle one schema downstream.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StepType {
    Click,
    RightClick,
    DoubleClick,
    OpenApp,
    OpenUrl,
    KeyboardShortcut,
    PressKey,
    Type,
    SetValue,
    Scroll,
    WaitForState,
    Observe,
}

impl StepType {
    /// Tolerant parser — Gemini occasionally drifts on the casing or splits
    /// words differently. We strip punctuation and match against a known
    /// alias table the same way Swift's `StepType.normalized(from:)` does.
    pub fn normalized(raw: Option<&str>) -> StepType {
        let raw = raw.unwrap_or("click");
        let compact: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match compact.as_str() {
            "openapp" | "launchapp" | "launchapplication" | "openapplication" => StepType::OpenApp,
            "openurl" | "url" | "openlink" | "openwebsite" | "openfile" | "openfolder" => {
                StepType::OpenUrl
            }
            "rightclick" | "secondaryclick" | "contextclick" => StepType::RightClick,
            "doubleclick" => StepType::DoubleClick,
            "keyboardshortcut" | "hotkey" | "shortcut" => StepType::KeyboardShortcut,
            "presskey" | "key" => StepType::PressKey,
            "type" | "typetext" | "inputtext" | "entertext" => StepType::Type,
            "setvalue" | "set" => StepType::SetValue,
            "scroll" => StepType::Scroll,
            "waitforstate" | "wait" => StepType::WaitForState,
            "observe" => StepType::Observe,
            "click" => StepType::Click,
            _ => StepType::Click,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TargetContext {
    VisibleElement,
    CurrentHighlight,
    CurrentSelection,
    FocusedElement,
}

impl TargetContext {
    pub fn normalized(raw: Option<&str>) -> Option<TargetContext> {
        let raw = raw?;
        let compact: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match compact.as_str() {
            "highlight" | "currenthighlight" | "focusedhighlight" | "paintedhighlight" => {
                Some(TargetContext::CurrentHighlight)
            }
            "selection" | "currentselection" | "selectedtext" | "textselection" => {
                Some(TargetContext::CurrentSelection)
            }
            "focus" | "focused" | "focusedelement" | "activefield" => {
                Some(TargetContext::FocusedElement)
            }
            "visible" | "visibleelement" | "screen" | "onscreen" => {
                Some(TargetContext::VisibleElement)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    #[serde(default)]
    pub id: Option<String>,

    /// The step kind. We deserialize loosely via the raw_type field
    /// (`#[serde(default)]`) and normalize on entry.
    #[serde(rename = "type", default)]
    pub raw_type: Option<String>,

    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub amount: Option<i64>,
    #[serde(default)]
    pub by: Option<String>,

    #[serde(default, rename = "targetContext", alias = "target_context")]
    pub raw_target_context: Option<String>,

    #[serde(default)]
    pub hint: Option<String>,

    /// LLM coordinate hint (screenshot pixel space).
    #[serde(default, rename = "hintX", alias = "x")]
    pub hint_x: Option<i64>,
    #[serde(default, rename = "hintY", alias = "y")]
    pub hint_y: Option<i64>,

    /// Original normalized box from Gemini: [y1, x1, y2, x2], 0..=1000.
    #[serde(default, rename = "box2DNormalized", alias = "box_2d")]
    pub box_2d_normalized: Option<Vec<i64>>,

    #[serde(default, rename = "screenNumber", alias = "screen")]
    pub screen_number: Option<i64>,

    /// Text payload — for `.type` steps. Sometimes the LLM uses `value`
    /// for this; we accept both and fall back at the runner level.
    #[serde(default)]
    pub text: Option<String>,

    /// Key chord — for `.keyboardShortcut` steps. Strings like "Cmd+S"
    /// or arrays like ["Ctrl","Shift","P"].
    #[serde(default, rename = "keyChord", alias = "key_chord")]
    pub key_chord: Option<serde_json::Value>,
}

impl WorkflowStep {
    pub fn step_type(&self) -> StepType {
        StepType::normalized(self.raw_type.as_deref())
    }

    pub fn target_context(&self) -> Option<TargetContext> {
        TargetContext::normalized(self.raw_target_context.as_deref())
    }

    /// Convenience: normalized [y1,x1,y2,x2] -> Some([y1,x1,y2,x2]) as u32.
    pub fn box_2d_u32(&self) -> Option<[u32; 4]> {
        let raw = self.box_2d_normalized.as_ref()?;
        if raw.len() != 4 {
            return None;
        }
        Some([
            raw[0].max(0) as u32,
            raw[1].max(0) as u32,
            raw[2].max(0) as u32,
            raw[3].max(0) as u32,
        ])
    }

    /// Convert the optional `key_chord` JSON value into a flat `Vec<String>`
    /// of modifier-and-key tokens. Accepts both array-of-strings shape
    /// (`["Cmd","S"]`) and chord-string shape (`"Cmd+S"`).
    pub fn key_chord_tokens(&self) -> Vec<String> {
        match &self.key_chord {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect(),
            Some(serde_json::Value::String(chord)) => chord
                .split(['+', '-'])
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPlan {
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
}
