// Action executor and workflow runner. Port of TipTour's Swift
// `ActionExecutor.swift` + `WorkflowRunner.swift`, repurposed for the
// cross-platform Tauri build.
//
// The Gemini Live `submit_workflow_plan` tool call lands here: the TS
// layer hands us the raw arguments JSON, we parse it into a `WorkflowPlan`,
// build a `GroundingResolver` adapter against the existing grounding
// layer, run the plan on a background task, and stream progress to the
// frontend via `workflow_progress` Tauri events.

pub mod action;
pub mod clipboard_paste;
pub mod cross_platform_input;
pub mod safety_rails;
pub mod workflow_plan;
pub mod workflow_runner;

use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::grounding;
use workflow_plan::WorkflowPlan;
use workflow_runner::{run_workflow_plan, WorkflowProgress};

/// Trait the workflow runner uses to translate `(label, optional box_2d)`
/// pairs into global screen pixel coordinates. We deliberately keep this
/// tiny so the grounding implementation can evolve under the runner
/// without breaking the contract.
pub trait GroundingResolver: Send {
    fn resolve_label(&mut self, label: &str, box_2d: Option<[u32; 4]>) -> Option<(f64, f64)>;

    /// When the grounding layer prefers a keyboard chord for this label
    /// (menu-item shortcut bindings on Windows, AX menu accelerators on
    /// macOS), surface the modifier-and-key tokens here. The workflow
    /// runner uses this to short-circuit click steps into keyboard
    /// shortcuts before it bothers with coordinate resolution — far more
    /// reliable than dispatching synthetic clicks at menu items.
    fn resolve_shortcut(&mut self, _label: &str) -> Option<Vec<String>> {
        None
    }
}

/// Default grounding implementation. Delegates to the existing
/// `grounding::resolver::resolve` function and converts its
/// `ResolvedTarget::Coordinate` variant into the (x, y) tuple the runner
/// expects. The `Shortcut` variant doesn't fit the coordinate-only
/// trait — when it shows up we ignore it and let the runner fall back
/// to box_2d. Phase 2 should expand `GroundingResolver` to surface
/// shortcuts so click-like steps can prefer key chords on Windows.
pub struct DefaultGroundingResolver {
    target_app: Option<grounding::types::TargetApp>,
}

impl DefaultGroundingResolver {
    pub fn new() -> Self {
        Self {
            target_app: grounding::target_app::current_target_app(),
        }
    }
}

impl GroundingResolver for DefaultGroundingResolver {
    fn resolve_label(&mut self, label: &str, _box_2d: Option<[u32; 4]>) -> Option<(f64, f64)> {
        let resolved = grounding::resolver::resolve(label, self.target_app.clone())?;
        match resolved {
            grounding::types::ResolvedTarget::Coordinate { x, y } => Some((x, y)),
            // Shortcut binding flows through `resolve_shortcut` below; we
            // return None here so the runner's coordinate path doesn't
            // pretend it succeeded.
            grounding::types::ResolvedTarget::Shortcut { .. } => None,
        }
    }

    fn resolve_shortcut(&mut self, label: &str) -> Option<Vec<String>> {
        let resolved = grounding::resolver::resolve(label, self.target_app.clone())?;
        match resolved {
            grounding::types::ResolvedTarget::Shortcut { chord } => {
                // Flatten ["Cmd","Shift"] + "S" into the flat token list
                // the cross-platform input layer expects.
                let mut chord_tokens: Vec<String> =
                    chord.modifiers.into_iter().collect();
                chord_tokens.push(chord.key);
                Some(chord_tokens)
            }
            grounding::types::ResolvedTarget::Coordinate { .. } => None,
        }
    }
}

/// Fallback resolver that returns `None` for every label. The runner
/// then falls back to Gemini's `box_2d` hint, which is the design intent
/// for environments where the grounding layer isn't available yet.
pub struct NullGroundingResolver;

impl GroundingResolver for NullGroundingResolver {
    fn resolve_label(&mut self, _label: &str, _box_2d: Option<[u32; 4]>) -> Option<(f64, f64)> {
        None
    }
}

/// Tauri command — entry point for the TS layer when Gemini's tool call
/// arrives. Spawns the runner on a Tokio task, returns the workflow id
/// immediately, and emits `workflow_progress` events as steps land.
#[tauri::command]
pub async fn execute_workflow_plan(
    plan_json: Value,
    app: AppHandle,
) -> Result<String, String> {
    let plan: WorkflowPlan =
        serde_json::from_value(plan_json).map_err(|error| format!("plan parse: {error}"))?;

    let (progress_sender, mut progress_receiver) = mpsc::channel::<WorkflowProgress>(64);
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let workflow_id_clone = workflow_id.clone();

    // Forwarder task: drain progress events from the runner channel and
    // re-emit them as Tauri events the panel listens for.
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(progress) = progress_receiver.recv().await {
            let _ = app_clone.emit("workflow_progress", progress);
        }
    });

    // Runner task: instantiate the default grounding resolver and walk
    // the plan. We don't await this — execution is fire-and-forget at the
    // TS boundary so Gemini's tool response can come back immediately.
    tauri::async_runtime::spawn(async move {
        let mut resolver = DefaultGroundingResolver::new();
        let outcome = run_workflow_plan(plan, &mut resolver, progress_sender.clone()).await;
        if let Err(error) = outcome {
            let _ = progress_sender
                .send(WorkflowProgress::Failed { message: error })
                .await;
        }
        // Sender dropped here, closing the channel and letting the
        // forwarder task exit.
    });

    Ok(workflow_id_clone)
}
