// Cross-platform workflow runner. Port of `TipTour/WorkflowRunner.swift`'s
// autopilot path — consumes plans emitted by Gemini's `submit_workflow_plan`
// tool call and drives them end-to-end through the action executor.
//
// What we keep from the Swift version:
//   * Operation token stamped on each plan so callbacks from a stale plan
//     can't mutate the current one after a rapid restart.
//   * 350ms settle window after click-like steps before we look for the
//     next UI state, with post-click AX/UIA fingerprint validation
//     supplied by `safety_rails::settle_until_ui_stable`.
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
use crate::executor::safety_rails::{
    modal_dialog_blocking, current_frontmost_pid, settle_until_ui_stable, user_switched_away_from,
};
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
    StepResolved {
        step_index: usize,
        // Resolved click target in global screen coordinates. Emitted just
        // before the synthetic click lands, so the overlay can fly its
        // companion cursor in lockstep with the actual cursor movement.
        // Both coordinates are None when the runner short-circuited the
        // click into a keyboard shortcut — `shortcut_keys` carries the
        // chord tokens in that case.
        #[serde(skip_serializing_if = "Option::is_none")]
        target_x: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_y: Option<f64>,
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        shortcut_keys: Option<Vec<String>>,
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

    // Capture the foreground pid at workflow start so we can detect when
    // the user Cmd-Tabs away mid-plan. None just disables the check —
    // safer than falsely tripping when there is no foreground app.
    let workflow_starting_foreground_pid: Option<i32> = current_frontmost_pid();

    for (step_index, step) in plan.steps.iter().enumerate() {
        // Pause the workflow whenever a modal dialog/sheet pops up. The
        // detector caches its answer for 100ms internally so polling
        // between every step stays cheap.
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

        // Pause when the user moves focus to a different app than the
        // one the plan started in — except when the new frontmost is
        // TipTour itself, since the panel/tray naturally grabs focus
        // during voice activity and we don't want that to abort plans.
        if user_switched_away_from(workflow_starting_foreground_pid) {
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

        let result = execute_step(step, step_index, step_type, grounding, &progress).await;

        let is_executed = matches!(result, StepResult::Executed);
        let _ = progress
            .send(WorkflowProgress::StepFinished {
                step_index,
                result: result.clone(),
            })
            .await;

        if is_executed {
            executed_step_count += 1;
            // Post-action settle. For UI-mutating step types
            // (Click/Type/Shortcut/PressKey/SetValue) this polls the
            // foreground AX/UIA fingerprint until three consecutive
            // samples match, capped at 600ms — so the next step's
            // grounding query runs against a tree that has actually
            // repainted. For LaunchApp/OpenUrl/Scroll it's a flat 350ms.
            settle_until_ui_stable(step_type).await;
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

async fn execute_step(
    step: &WorkflowStep,
    step_index: usize,
    step_type: StepType,
    grounding: &mut dyn GroundingResolver,
    progress: &Sender<WorkflowProgress>,
) -> StepResult {
    match step_type {
        StepType::Click | StepType::RightClick | StepType::DoubleClick => {
            // Prefer keyboard shortcut grounding when the resolver knows
            // a chord for this label (menu accelerators are deterministic;
            // synthetic clicks on menu items are not). Skip the
            // coordinate resolution entirely in that case.
            if let Some(label) = step.label.as_deref() {
                if let Some(shortcut_tokens) = grounding.resolve_shortcut(label) {
                    let _ = progress
                        .send(WorkflowProgress::StepResolved {
                            step_index,
                            target_x: None,
                            target_y: None,
                            label: step.label.clone(),
                            shortcut_keys: Some(shortcut_tokens.clone()),
                        })
                        .await;
                    if matches!(
                        crate::mode::current_operating_mode(),
                        crate::mode::OperatingMode::Teaching
                    ) {
                        return StepResult::Executed;
                    }
                    return deliver(ExecutableAction::KeyboardShortcut {
                        keys: shortcut_tokens,
                    });
                }
            }

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
            // Tell the overlay where the cursor is about to go, before the
            // real cursor moves. The animation duration on the overlay
            // matches the system click delay so the two cursors arrive at
            // the same time.
            let _ = progress
                .send(WorkflowProgress::StepResolved {
                    step_index,
                    target_x: Some(x),
                    target_y: Some(y),
                    label: step.label.clone(),
                    shortcut_keys: None,
                })
                .await;
            // Teaching mode: the overlay still points, but TipTour does NOT
            // perform the click — the user does. Return early with a result
            // that surfaces as "user-driven" in the progress stream.
            if matches!(crate::mode::current_operating_mode(), crate::mode::OperatingMode::Teaching) {
                return StepResult::Executed;
            }
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
        Some(TargetContext::CurrentHighlight) => {
            // The brush module owns the latest painted region. Anchor
            // point is the last hovered point on the painted polyline —
            // exactly what we want as the click target.
            if let Some(highlight_context) = crate::highlight::current_highlight_context() {
                if let Some(anchor_point) = highlight_context.anchor_point {
                    return Some(anchor_point);
                }
            }
        }
        Some(TargetContext::CurrentSelection) => {
            if let Some(point) = current_selection_center_point() {
                return Some(point);
            }
        }
        Some(TargetContext::FocusedElement) => {
            if let Some(point) = focused_element_center_point() {
                return Some(point);
            }
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
            into_focused,
        } => {
            // When the LLM marked this step as `into_focused`, try the
            // deterministic AX/UIA path first. It preserves the user's
            // clipboard and won't race with their physical keyboard. The
            // helper returns Err on any platform/element mismatch, in
            // which case we drop through to the length-heuristic fallback
            // (paste for >80 chars / multi-line, keystrokes otherwise).
            if into_focused {
                if crate::executor::ax_text_write::attempt_ax_text_write(&text).is_ok() {
                    Ok(())
                } else if text.len() > 80 || text.contains('\n') {
                    clipboard_paste::paste_text(&text)
                } else {
                    cross_platform_input::type_text(&text)
                }
            } else if text.len() > 80 || text.contains('\n') {
                clipboard_paste::paste_text(&text)
            } else {
                cross_platform_input::type_text(&text)
            }
        }
        ExecutableAction::SetSelectedText { text } => {
            // Replace the highlighted run. On macOS we first restore the
            // armed AXSelectedTextRange captured at paint-end so the paste
            // lands inside the originally-highlighted span even if the
            // user has since clicked elsewhere; only then do we paste. On
            // Windows we don't have the armed-range capture yet, so the
            // plain paste path is used.
            #[cfg(target_os = "macos")]
            {
                restore_armed_selection_range_on_macos();
            }
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


/// Center point of the user's current text selection (in global screen
/// coordinates), or None if nothing is selected. Used to bind a step's
/// `targetContext: "currentSelection"` to an actual click target so
/// follow-up clicks land inside the selected range.
fn current_selection_center_point() -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        return macos_current_selection_center();
    }
    #[cfg(target_os = "windows")]
    {
        return windows_current_selection_center();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Center point of the system-wide focused UI element, or None if no
/// element reports focus. Lets the runner bind `targetContext:
/// "focusedElement"` steps to the field the OS reports as accepting
/// input right now.
fn focused_element_center_point() -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        return macos_focused_element_center();
    }
    #[cfg(target_os = "windows")]
    {
        return windows_focused_element_center();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_focused_element_center() -> Option<(f64, f64)> {
    use accessibility_sys::{
        kAXFocusedUIElementAttribute, kAXPositionAttribute, kAXSizeAttribute,
        kAXValueTypeCGPoint, kAXValueTypeCGSize, AXUIElementCopyAttributeValue,
        AXUIElementCreateSystemWide, AXUIElementRef, AXUIElementSetMessagingTimeout,
        AXValueGetType, AXValueGetValue, AXValueRef,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;

    unsafe {
        let system_wide_element: AXUIElementRef = AXUIElementCreateSystemWide();
        if system_wide_element.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system_wide_element, 0.4);
        let focused_attribute = CFString::new(kAXFocusedUIElementAttribute);
        let mut focused_raw: CFTypeRef = std::ptr::null_mut();
        let status = AXUIElementCopyAttributeValue(
            system_wide_element,
            focused_attribute.as_concrete_TypeRef(),
            &mut focused_raw,
        );
        CFRelease(system_wide_element as CFTypeRef);
        if status != 0 || focused_raw.is_null() {
            return None;
        }
        let focused_element_ref = focused_raw as AXUIElementRef;
        AXUIElementSetMessagingTimeout(focused_element_ref, 0.4);

        let position = read_ax_point(focused_element_ref, kAXPositionAttribute, kAXValueTypeCGPoint);
        let size = read_ax_size(focused_element_ref, kAXSizeAttribute, kAXValueTypeCGSize);
        CFRelease(focused_raw);

        let (origin_x, origin_y) = position?;
        let (width, height) = size?;
        Some((origin_x + width / 2.0, origin_y + height / 2.0))
    }
}

#[cfg(target_os = "macos")]
fn macos_current_selection_center() -> Option<(f64, f64)> {
    use accessibility_sys::{
        kAXBoundsForRangeParameterizedAttribute, kAXFocusedUIElementAttribute,
        kAXSelectedTextRangeAttribute, kAXValueTypeCGRect, AXUIElementCopyAttributeValue,
        AXUIElementCopyParameterizedAttributeValue, AXUIElementCreateSystemWide,
        AXUIElementRef, AXUIElementSetMessagingTimeout, AXValueGetType, AXValueGetValue,
        AXValueRef,
    };
    use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use core_graphics::geometry::CGRect;

    unsafe {
        let system_wide_element: AXUIElementRef = AXUIElementCreateSystemWide();
        if system_wide_element.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system_wide_element, 0.4);
        let focused_attribute = CFString::new(kAXFocusedUIElementAttribute);
        let mut focused_raw: CFTypeRef = std::ptr::null_mut();
        let focused_status = AXUIElementCopyAttributeValue(
            system_wide_element,
            focused_attribute.as_concrete_TypeRef(),
            &mut focused_raw,
        );
        CFRelease(system_wide_element as CFTypeRef);
        if focused_status != 0 || focused_raw.is_null() {
            return None;
        }
        let focused_element_ref = focused_raw as AXUIElementRef;
        AXUIElementSetMessagingTimeout(focused_element_ref, 0.4);

        // Pull the AXSelectedTextRange (CFRange-style AXValue) from the
        // focused element. If nothing is selected the read fails or the
        // range length is zero and we bail.
        let selected_range_attribute = CFString::new(kAXSelectedTextRangeAttribute);
        let mut selected_range_raw: CFTypeRef = std::ptr::null_mut();
        let range_status = AXUIElementCopyAttributeValue(
            focused_element_ref,
            selected_range_attribute.as_concrete_TypeRef(),
            &mut selected_range_raw,
        );
        if range_status != 0 || selected_range_raw.is_null() {
            CFRelease(focused_raw);
            return None;
        }

        // Convert that range into screen-space bounds via the
        // AXBoundsForRange parameterized attribute. The returned AXValue
        // wraps a CGRect we can use directly.
        let bounds_attribute = CFString::new(kAXBoundsForRangeParameterizedAttribute);
        let mut bounds_raw: CFTypeRef = std::ptr::null_mut();
        let bounds_status = AXUIElementCopyParameterizedAttributeValue(
            focused_element_ref,
            bounds_attribute.as_concrete_TypeRef(),
            selected_range_raw,
            &mut bounds_raw,
        );
        CFRelease(selected_range_raw);
        CFRelease(focused_raw);
        if bounds_status != 0 || bounds_raw.is_null() {
            return None;
        }

        let value_ref = bounds_raw as AXValueRef;
        let value_type = AXValueGetType(value_ref);
        if value_type != kAXValueTypeCGRect {
            CFRelease(bounds_raw);
            return None;
        }
        let mut rect = CGRect::new(
            &core_graphics::geometry::CGPoint::new(0.0, 0.0),
            &core_graphics::geometry::CGSize::new(0.0, 0.0),
        );
        let got_value = AXValueGetValue(
            value_ref,
            value_type,
            &mut rect as *mut CGRect as *mut std::ffi::c_void,
        );
        CFRelease(bounds_raw);
        if !got_value {
            return None;
        }
        let _ = CFGetTypeID;
        Some((
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        ))
    }
}

#[cfg(target_os = "macos")]
unsafe fn read_ax_point(
    element: accessibility_sys::AXUIElementRef,
    attribute_name: &str,
    expected_type: u32,
) -> Option<(f64, f64)> {
    use accessibility_sys::{
        AXUIElementCopyAttributeValue, AXValueGetType, AXValueGetValue, AXValueRef,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use core_graphics::geometry::CGPoint;

    let attribute = CFString::new(attribute_name);
    let mut raw: CFTypeRef = std::ptr::null_mut();
    let status = AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut raw);
    if status != 0 || raw.is_null() {
        return None;
    }
    let value_ref = raw as AXValueRef;
    if AXValueGetType(value_ref) != expected_type {
        CFRelease(raw);
        return None;
    }
    let mut cg_point = CGPoint::new(0.0, 0.0);
    let ok = AXValueGetValue(
        value_ref,
        expected_type,
        &mut cg_point as *mut CGPoint as *mut std::ffi::c_void,
    );
    CFRelease(raw);
    if !ok {
        return None;
    }
    Some((cg_point.x, cg_point.y))
}

#[cfg(target_os = "macos")]
unsafe fn read_ax_size(
    element: accessibility_sys::AXUIElementRef,
    attribute_name: &str,
    expected_type: u32,
) -> Option<(f64, f64)> {
    use accessibility_sys::{
        AXUIElementCopyAttributeValue, AXValueGetType, AXValueGetValue, AXValueRef,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use core_graphics::geometry::CGSize;

    let attribute = CFString::new(attribute_name);
    let mut raw: CFTypeRef = std::ptr::null_mut();
    let status = AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut raw);
    if status != 0 || raw.is_null() {
        return None;
    }
    let value_ref = raw as AXValueRef;
    if AXValueGetType(value_ref) != expected_type {
        CFRelease(raw);
        return None;
    }
    let mut cg_size = CGSize::new(0.0, 0.0);
    let ok = AXValueGetValue(
        value_ref,
        expected_type,
        &mut cg_size as *mut CGSize as *mut std::ffi::c_void,
    );
    CFRelease(raw);
    if !ok {
        return None;
    }
    Some((cg_size.width, cg_size.height))
}

/// Restore the AX text range captured when the user finished painting the
/// highlight. Writing `kAXSelectedTextRangeAttribute` back onto the
/// focused element re-selects the original span, so the subsequent paste
/// replaces *that* run instead of whatever happens to be selected (or
/// nothing) at the moment the workflow step fires.
#[cfg(target_os = "macos")]
fn restore_armed_selection_range_on_macos() {
    use accessibility_sys::{
        kAXFocusedUIElementAttribute, kAXSelectedTextRangeAttribute, kAXValueTypeCFRange,
        AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide, AXUIElementRef,
        AXUIElementSetAttributeValue, AXUIElementSetMessagingTimeout, AXValueCreate,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;

    // Pull the armed range from the latest highlight context. If the user
    // hasn't painted anything (or the painter never recorded a range),
    // there's nothing to restore and the caller will plain-paste.
    let highlight_context = match crate::highlight::current_highlight_context() {
        Some(context) => context,
        None => return,
    };
    let (range_location, range_length) = match (
        highlight_context.armed_text_range_location,
        highlight_context.armed_text_range_length,
    ) {
        (Some(location), Some(length)) if length > 0 => (location, length),
        _ => return,
    };

    #[repr(C)]
    struct CoreFoundationRange {
        location: core::ffi::c_long,
        length: core::ffi::c_long,
    }
    let mut cf_range = CoreFoundationRange {
        location: range_location as core::ffi::c_long,
        length: range_length as core::ffi::c_long,
    };

    unsafe {
        let system_wide_element: AXUIElementRef = AXUIElementCreateSystemWide();
        if system_wide_element.is_null() {
            return;
        }
        AXUIElementSetMessagingTimeout(system_wide_element, 0.4);
        let focused_attribute = CFString::new(kAXFocusedUIElementAttribute);
        let mut focused_raw: CFTypeRef = std::ptr::null_mut();
        let focused_status = AXUIElementCopyAttributeValue(
            system_wide_element,
            focused_attribute.as_concrete_TypeRef(),
            &mut focused_raw,
        );
        CFRelease(system_wide_element as CFTypeRef);
        if focused_status != 0 || focused_raw.is_null() {
            return;
        }
        let focused_element_ref = focused_raw as AXUIElementRef;
        AXUIElementSetMessagingTimeout(focused_element_ref, 0.4);

        let ax_value_ref = AXValueCreate(
            kAXValueTypeCFRange,
            &mut cf_range as *mut CoreFoundationRange as *mut std::ffi::c_void,
        );
        if ax_value_ref.is_null() {
            CFRelease(focused_raw);
            return;
        }

        let selected_range_attribute = CFString::new(kAXSelectedTextRangeAttribute);
        let set_status = AXUIElementSetAttributeValue(
            focused_element_ref,
            selected_range_attribute.as_concrete_TypeRef(),
            ax_value_ref as CFTypeRef,
        );
        CFRelease(ax_value_ref as CFTypeRef);
        CFRelease(focused_raw);
        if set_status != 0 {
            // Range write failed — the focused element probably changed
            // since paint-end. Caller's plain paste is the safest action.
            eprintln!(
                "workflow_runner: AXSelectedTextRange restore failed with status {set_status}; falling back to plain paste"
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_focused_element_center() -> Option<(f64, f64)> {
    use uiautomation::UIAutomation;
    let automation = UIAutomation::new().ok()?;
    let focused_element = automation.get_focused_element().ok()?;
    let bounding_rect = focused_element.get_bounding_rectangle().ok()?;
    let left = bounding_rect.get_left() as f64;
    let top = bounding_rect.get_top() as f64;
    let right = bounding_rect.get_right() as f64;
    let bottom = bounding_rect.get_bottom() as f64;
    Some(((left + right) / 2.0, (top + bottom) / 2.0))
}

#[cfg(target_os = "windows")]
fn windows_current_selection_center() -> Option<(f64, f64)> {
    // The `uiautomation` crate (0.16) doesn't expose
    // ITextRangeProvider::GetBoundingRectangles, so we fall back to the
    // selection's enclosing element bounding rect — for selections
    // inside text controls this gives us a click target inside the
    // field, which is good enough for "click in this selection" intent.
    use uiautomation::patterns::UITextPattern;
    use uiautomation::UIAutomation;
    let automation = UIAutomation::new().ok()?;
    let focused_element = automation.get_focused_element().ok()?;
    let text_pattern = focused_element.get_pattern::<UITextPattern>().ok()?;
    let selection_ranges = text_pattern.get_selection().ok()?;
    let first_range = selection_ranges.into_iter().next()?;
    let enclosing_element = first_range.get_enclosing_element().ok()?;
    let bounding_rect = enclosing_element.get_bounding_rectangle().ok()?;
    let left = bounding_rect.get_left() as f64;
    let top = bounding_rect.get_top() as f64;
    let right = bounding_rect.get_right() as f64;
    let bottom = bounding_rect.get_bottom() as f64;
    Some(((left + right) / 2.0, (top + bottom) / 2.0))
}
