// Cross-platform workflow runner. Port of `TipTour/WorkflowRunner.swift`'s
// autopilot path — consumes plans emitted by Gemini's `submit_workflow_plan`
// tool call and drives them end-to-end through the action executor.
//
// What we keep from the Swift version:
//   * Operation token stamped on each plan so callbacks from a stale plan
//     can't mutate the current one after a rapid restart.
//   * 350ms settle window after click-like steps before we look for the
//     next UI state (post-click AX fingerprint validation is a TODO).
//   * Modal-dialog pause hook (stub for now — Phase 2 follow-up).
//   * App-switch pause hook (stub for now — wired to platform-specific
//     foreground change events later).
//   * Progress streaming so the UI can render a checklist as steps land.
//
// What we don't try to port yet:
//   * Teaching mode (point-only). The runner only knows Autopilot.
//   * Browser CDP grounding. Falls out for free via `GroundingResolver`.
//   * Focus-highlight binding for `currentHighlight`/`currentSelection`.
//     The runner records the target context on each step so the caller's
//     grounding implementation can specialize — see `GroundingResolver`.

use std::time::Duration;
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::executor::action::{ExecutableAction, MouseButton};
use crate::executor::clipboard_paste;
use crate::executor::cross_platform_input;
use crate::executor::workflow_plan::{StepType, TargetContext, WorkflowPlan, WorkflowStep};
use crate::executor::GroundingResolver;

/// Streamed progress update emitted as the runner walks through a plan.
/// Caller wires these into Tauri events so the TS layer can render a
/// live checklist alongside Gemini's voice replies.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkflowProgress {
    Started {
        workflow_id: String,
        goal: Option<String>,
        total_steps: usize,
    },
    StepStarted {
        step_index: usize,
        label: Option<String>,
        step_type: String,
    },
    StepFinished {
        step_index: usize,
        result: StepResult,
    },
    Paused {
        reason: String,
    },
    Completed,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StepResult {
    Executed,
    /// Grounding returned no match for the step's label or target context.
    /// We surface this as a soft failure so the UI can prompt the user
    /// instead of silently abandoning the plan.
    Unresolved { reason: String },
    /// Action delivery itself failed (enigo error, etc.). Distinct from
    /// `Unresolved` because the user can usually retry.
    ActionFailed { reason: String },
    Skipped { reason: String },
}

/// Final summary returned to the caller after the runner finishes (or
/// aborts). Used by tests and the Tauri command to surface a one-shot
/// outcome to JS.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOutcome {
    pub workflow_id: String,
    pub completed: bool,
    pub executed_step_count: usize,
    pub failure_message: Option<String>,
}

/// Default settle window between click-like steps. The real fingerprint
/// validator should replace this once Phase 2 lands AX/UIA polling — for
/// now it's a flat sleep that gives the OS enough time to dispatch the
/// click before the next step's grounding query runs.
const POST_CLICK_SETTLE: Duration = Duration::from_millis(350);

/// Drive a plan to completion. Returns a final outcome; intermediate
/// updates flow over `progress`.
pub async fn run_workflow_plan(
    plan: WorkflowPlan,
    grounding: &mut dyn GroundingResolver,
    progress: Sender<WorkflowProgress>,
) -> Result<WorkflowOutcome, String> {
    let workflow_id = Uuid::new_v4().to_string();
    // The operation token guards against stale callbacks if a future
    // version of this runner exposes restart. Today it's effectively
    // unused — we keep it stamped so the value flows through the
    // progress events for client-side correlation.
    let _operation_token: Uuid = Uuid::new_v4();

    let total_steps = plan.steps.len();
    let _ = progress
        .send(WorkflowProgress::Started {
            workflow_id: workflow_id.clone(),
            goal: plan.goal.clone(),
            total_steps,
        })
        .await;

    let mut executed_step_count: usize = 0;

    for (step_index, step) in plan.steps.iter().enumerate() {
        // TODO(Phase 2): modal dialog detection. The Swift runner pauses
        // when an `AXSheet`/`AXDialog` appears mid-workflow — port that
        // here against UIA's `ControlType.Window` modal flag on Windows
        // and AX role on macOS.
        if modal_dialog_blocking() {
            let _ = progress
                .send(WorkflowProgress::Paused {
                    reason: "modal_dialog_detected".to_string(),
                })
                .await;
            return Ok(WorkflowOutcome {
                workflow_id,
                completed: false,
                executed_step_count,
                failure_message: Some("Paused on modal dialog".to_string()),
            });
        }

        // TODO(Phase 2): pause-on-app-switch. Hook into
        // `NSWorkspace.didActivateApplicationNotification` (mac) and
        // `EVENT_SYSTEM_FOREGROUND` (windows) and trip a flag the runner
        // polls between steps. Until then the runner stays naive.
        if app_switched_during_run() {
            let _ = progress
                .send(WorkflowProgress::Paused {
                    reason: "user_changed_focus".to_string(),
                })
                .await;
            return Ok(WorkflowOutcome {
                workflow_id,
                completed: false,
                executed_step_count,
                failure_message: Some("Paused on app switch".to_string()),
            });
        }

        let step_type = step.step_type();
        let _ = progress
            .send(WorkflowProgress::StepStarted {
                step_index,
                label: step.label.clone(),
                step_type: format!("{:?}", step_type),
            })
            .await;

        let result = execute_step(step, step_type, grounding).await;

        let is_executed = matches!(result, StepResult::Executed);
        let _ = progress
            .send(WorkflowProgress::StepFinished {
                step_index,
                result: result.clone(),
            })
            .await;

        if is_executed {
            executed_step_count += 1;
            // Settle window — gives the click time to propagate before we
            // ground the next step's label against a fresh AX/UIA snapshot.
            // Only matters for click-like steps; cheap to apply uniformly.
            if step_needs_post_action_settle(step_type) {
                tokio::time::sleep(POST_CLICK_SETTLE).await;
            }
        } else if matches!(result, StepResult::ActionFailed { .. }) {
            // Hard failure: surface and stop. The user can ask the agent
            // to retry; we don't want to half-execute the rest of a plan
            // when input delivery is broken.
            let failure_message = match &result {
                StepResult::ActionFailed { reason } => reason.clone(),
                _ => "unknown".to_string(),
            };
            let _ = progress
                .send(WorkflowProgress::Failed {
                    message: failure_message.clone(),
                })
                .await;
            return Ok(WorkflowOutcome {
                workflow_id,
                completed: false,
                executed_step_count,
                failure_message: Some(failure_message),
            });
        }
        // For `Unresolved`/`Skipped` we keep walking — the LLM often
        // emits an observational step that has no grounding target.
    }

    let _ = progress.send(WorkflowProgress::Completed).await;
    Ok(WorkflowOutcome {
        workflow_id,
        completed: true,
        executed_step_count,
        failure_message: None,
    })
}

fn step_needs_post_action_settle(step_type: StepType) -> bool {
    matches!(
        step_type,
        StepType::Click
            | StepType::RightClick
            | StepType::DoubleClick
            | StepType::KeyboardShortcut
            | StepType::PressKey
            | StepType::OpenApp
            | StepType::OpenUrl
    )
}

async fn execute_step(
    step: &WorkflowStep,
    step_type: StepType,
    grounding: &mut dyn GroundingResolver,
) -> StepResult {
    match step_type {
        StepType::Click | StepType::RightClick | StepType::DoubleClick => {
            let target_context = step.target_context();
            let (x, y) = match resolve_step_coordinate(step, target_context, grounding) {
                Some(point) => point,
                None => {
                    return StepResult::Unresolved {
                        reason: format!(
                            "no grounding for label `{}`",
                            step.label.as_deref().unwrap_or("<none>")
                        ),
                    };
                }
            };
            let action = match step_type {
                StepType::DoubleClick => ExecutableAction::DoubleClick { x, y },
                StepType::RightClick => ExecutableAction::RightClick { x, y },
                _ => ExecutableAction::Click {
                    x,
                    y,
                    button: MouseButton::Left,
                },
            };
            deliver(action)
        }

        StepType::KeyboardShortcut => {
            let tokens = step.key_chord_tokens();
            if tokens.is_empty() {
                return StepResult::Unresolved {
                    reason: "keyboardShortcut step missing keys".to_string(),
                };
            }
            deliver(ExecutableAction::KeyboardShortcut { keys: tokens })
        }

        StepType::PressKey => {
            // Single-key press is just a one-token chord.
            let key = step
                .key_chord_tokens()
                .into_iter()
                .next()
                .or_else(|| step.value.clone())
                .or_else(|| step.text.clone());
            match key {
                Some(token) => deliver(ExecutableAction::KeyboardShortcut { keys: vec![token] }),
                None => StepResult::Unresolved {
                    reason: "pressKey step missing key".to_string(),
                },
            }
        }

        StepType::Type | StepType::SetValue => {
            // `type` and `setValue` collapse to the same executor surface:
            // both push text into whatever has focus. Differentiation
            // happens inside the executor, which prefers direct AX value
            // writes before falling back to clipboard paste.
            let text = step.text.clone().or_else(|| step.value.clone());
            let text = match text {
                Some(value) if !value.is_empty() => value,
                _ => {
                    return StepResult::Unresolved {
                        reason: "type step missing text".to_string(),
                    };
                }
            };
            let into_focused = matches!(step_type, StepType::SetValue);
            if matches!(step.target_context(), Some(TargetContext::CurrentSelection))
                || matches!(step.target_context(), Some(TargetContext::CurrentHighlight))
            {
                deliver(ExecutableAction::SetSelectedText { text })
            } else {
                deliver(ExecutableAction::Type { text, into_focused })
            }
        }

        StepType::OpenApp => {
            let id = step
                .value
                .clone()
                .or_else(|| step.label.clone())
                .unwrap_or_default();
            if id.is_empty() {
                return StepResult::Unresolved {
                    reason: "openApp step missing identifier".to_string(),
                };
            }
            deliver(ExecutableAction::LaunchApp {
                bundle_id_or_exe: id,
            })
        }

        StepType::OpenUrl => {
            let url = step
                .value
                .clone()
                .or_else(|| step.label.clone())
                .unwrap_or_default();
            if url.is_empty() {
                return StepResult::Unresolved {
                    reason: "openUrl step missing url".to_string(),
                };
            }
            deliver(ExecutableAction::OpenUrl { url })
        }

        StepType::Scroll => {
            let target_context = step.target_context();
            let (x, y) =
                resolve_step_coordinate(step, target_context, grounding).unwrap_or((0.0, 0.0));
            // Convert direction/amount into a dx/dy pair. `amount` is in
            // the LLM's loosely-defined "ticks"; we pass it through as
            // enigo's notch count.
            let amount = step.amount.unwrap_or(3) as f64;
            let (dx, dy) = match step.direction.as_deref().map(|s| s.to_ascii_lowercase()) {
                Some(ref direction) if direction == "up" => (0.0, -amount),
                Some(ref direction) if direction == "down" => (0.0, amount),
                Some(ref direction) if direction == "left" => (-amount, 0.0),
                Some(ref direction) if direction == "right" => (amount, 0.0),
                _ => (0.0, amount),
            };
            deliver(ExecutableAction::Scroll { x, y, dx, dy })
        }

        StepType::WaitForState => {
            // No state observer wired yet — treat as a short pause so the
            // plan stays well-formed and we don't fail loudly on a step
            // type the LLM occasionally emits.
            tokio::time::sleep(Duration::from_millis(500)).await;
            StepResult::Skipped {
                reason: "waitForState not implemented".to_string(),
            }
        }

        StepType::Observe => StepResult::Skipped {
            reason: "observe step (no action)".to_string(),
        },
    }
}

/// Resolve a step's coordinate using grounding when a label/box is present
/// and we're targeting a visible element. Returns `None` for context
/// modes that grounding doesn't cover today (highlight/selection/focus —
/// those are handled inside the action layer, not by point lookup).
fn resolve_step_coordinate(
    step: &WorkflowStep,
    target_context: Option<TargetContext>,
    grounding: &mut dyn GroundingResolver,
) -> Option<(f64, f64)> {
    match target_context {
        // For highlight/selection/focus the click coordinate isn't the
        // right grounding output — the action layer should send the key
        // chord or paste action to whatever the OS reports as focused.
        // We deliberately fall back to box_2d if Gemini supplied it so
        // the runner remains useful before the focus-binding wiring lands.
        Some(TargetContext::CurrentHighlight)
        | Some(TargetContext::CurrentSelection)
        | Some(TargetContext::FocusedElement) => {
            // TODO(focus binding): resolve through AX/UIA focused element.
            return step.box_2d_u32().and_then(|_| {
                // Fall through to grounding for now.
                let label = step.label.as_deref()?;
                grounding.resolve_label(label, step.box_2d_u32())
            });
        }
        _ => {}
    }

    // Try grounding first by label; if that fails, fall back to the raw
    // box_2d hint from Gemini. Same priority order as ElementResolver.swift.
    if let Some(label) = step.label.as_deref() {
        if let Some(point) = grounding.resolve_label(label, step.box_2d_u32()) {
            return Some(point);
        }
    }

    // Pure box_2d fallback: convert the normalized [y1,x1,y2,x2] center
    // back into screen pixels. We don't know the screenshot resolution
    // here, so we treat the box as already-pixel coordinates when the
    // values exceed the normalized range (Gemini sometimes emits raw
    // pixels). This matches Swift's hintCoordinate fallback.
    if let Some(box_2d) = step.box_2d_u32() {
        let center_x = (box_2d[1] as f64 + box_2d[3] as f64) / 2.0;
        let center_y = (box_2d[0] as f64 + box_2d[2] as f64) / 2.0;
        // We can't usefully scale here without screen dims; the calling
        // layer is expected to provide grounding that knows the capture
        // resolution. Return None to avoid clicking the wrong pixel.
        let _ = (center_x, center_y);
    }

    if let (Some(hx), Some(hy)) = (step.hint_x, step.hint_y) {
        return Some((hx as f64, hy as f64));
    }

    None
}

/// Bridge from `ExecutableAction` to the cross-platform input synth layer.
/// All branches return a `StepResult` so the caller can decide whether to
/// continue, pause, or surface the failure.
fn deliver(action: ExecutableAction) -> StepResult {
    let result = match action {
        ExecutableAction::LaunchApp { bundle_id_or_exe } => launch_app(&bundle_id_or_exe),
        ExecutableAction::OpenUrl { url } => open_url(&url),
        ExecutableAction::Click { x, y, button } => cross_platform_input::click_at(x, y, button),
        ExecutableAction::DoubleClick { x, y } => cross_platform_input::double_click_at(x, y),
        ExecutableAction::RightClick { x, y } => {
            cross_platform_input::click_at(x, y, MouseButton::Right)
        }
        ExecutableAction::KeyboardShortcut { keys } => {
            let refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            cross_platform_input::keyboard_shortcut(&refs)
        }
        ExecutableAction::Type {
            text,
            into_focused: _,
        } => {
            // TODO: prefer AX/UIA selected-text insertion when `into_focused`
            // is true. For now we always fall back to clipboard paste for
            // multi-line / long text and direct keystroke synthesis for
            // short text. The heuristic mirrors the Swift implementation,
            // which uses paste for >80 chars or strings containing newlines.
            if text.len() > 80 || text.contains('\n') {
                clipboard_paste::paste_text(&text)
            } else {
                cross_platform_input::type_text(&text)
            }
        }
        ExecutableAction::SetSelectedText { text } => {
            // TODO: apply armed AXSelectedTextRange before pasting (mac)
            // and the UIA TextPattern equivalent on Windows. Without the
            // range restore this falls back to a plain paste, which only
            // works when the user hasn't moved focus since the highlight.
            clipboard_paste::paste_text(&text)
        }
        ExecutableAction::Scroll { x, y, dx, dy } => cross_platform_input::scroll(x, y, dx, dy),
    };

    match result {
        Ok(()) => StepResult::Executed,
        Err(reason) => StepResult::ActionFailed { reason },
    }
}

/// Launch an app via the OS's native open machinery. On macOS this means
/// `open -b <bundle_id>` (or `open -a <name>`); on Windows we hand the
/// string to `cmd /C start`, which handles both executables on PATH and
/// shell-registered URIs. We keep this as a subprocess shell-out because
/// it lets the OS apply all its usual handler resolution.
fn launch_app(identifier: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let is_bundle_id = identifier.contains('.')
            && !identifier.contains('/')
            && !identifier.ends_with(".app");
        let mut command = std::process::Command::new("open");
        if is_bundle_id {
            command.args(["-b", identifier]);
        } else {
            command.args(["-a", identifier]);
        }
        command
            .status()
            .map_err(|error| format!("open failed: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("open exited with status {status}"))
                }
            })
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", identifier])
            .status()
            .map_err(|error| format!("start failed: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("start exited with status {status}"))
                }
            })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = identifier;
        Err("launch_app not supported on this platform".to_string())
    }
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .status()
            .map_err(|error| format!("open url failed: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("open url exited with status {status}"))
                }
            })
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .map_err(|error| format!("start url failed: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("start url exited with status {status}"))
                }
            })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        Err("open_url not supported on this platform".to_string())
    }
}

/// Stub: modal-dialog detection. Real implementation polls AX/UIA for a
/// front-and-center sheet/dialog window. See TODO at the call site.
fn modal_dialog_blocking() -> bool {
    false
}

/// Stub: pause-on-app-switch. Real implementation listens for the OS
/// foreground-change notification and toggles a flag the runner reads
/// between steps. See TODO at the call site.
fn app_switched_during_run() -> bool {
    false
}
