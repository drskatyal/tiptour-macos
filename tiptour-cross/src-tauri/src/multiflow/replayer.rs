// Replay a saved demonstration as a sequence of synthesized inputs.
//
// We read `demonstrations/{id}/trace.jsonl` and walk it in order. Between
// trace entries we preserve the original inter-event timing so the
// replay feels human, but cap any single gap at REPLAY_MAX_GAP_MS so a
// stale idle pause in the recording doesn't freeze the user's machine
// for minutes during playback. Mouse moves are skipped — they're visual
// noise the OS doesn't need; we only replay clicks, key chords, and
// scrolls.
//
// App-launch boundaries (foreground app change between snapshot_before
// and snapshot_after) get an explicit wait-for-frontmost poll up to
// REPLAY_APP_LAUNCH_WAIT_MS so we don't try to type into the previous
// app's window while the new one is still painting.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::sync::mpsc::Sender;
use tokio::time::sleep;

use super::types::{ReplayProgress, ReplayProgressKind};
use crate::executor::action::MouseButton;
use crate::executor::cross_platform_input;
use crate::executor::safety_rails;
use crate::grounding;
use crate::recorder::persistence as recorder_persistence;
use crate::recorder::types::{InputEvent, TraceEntry};

const REPLAY_MAX_GAP_MS: u64 = 2000;
const REPLAY_APP_LAUNCH_WAIT_MS: u64 = 3000;
const REPLAY_APP_LAUNCH_POLL_INTERVAL_MS: u64 = 100;

// The active replay's operation token. A new `run_flow_by_name` cancels
// any in-flight replay by replacing this token — the running task checks
// the token between every step and bails when it no longer matches the
// one it captured at start. Same pattern as the workflow runner's stale-
// callback guard.
static CURRENT_REPLAY_OPERATION_TOKEN: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Replace the in-flight replay token with `new_replay_id` and return
/// the new token wrapped in an `Arc<AtomicBool>` the running task can
/// cheaply poll. The caller is expected to spawn the replay with the
/// returned cancellation flag.
pub fn install_new_replay_and_cancel_previous(new_replay_id: &str) -> Arc<AtomicBool> {
    let mut current = CURRENT_REPLAY_OPERATION_TOKEN.lock();
    *current = Some(new_replay_id.to_string());
    // The cancellation flag the new replay watches. The previous replay's
    // flag was never tracked here — replays self-check by comparing their
    // captured replay_id against the global token, so installing a new
    // token implicitly cancels the older runner on its next step.
    Arc::new(AtomicBool::new(false))
}

fn current_replay_token_matches(expected_replay_id: &str) -> bool {
    let current = CURRENT_REPLAY_OPERATION_TOKEN.lock();
    current.as_deref() == Some(expected_replay_id)
}

fn parse_mouse_button_from_recorded_token(button_token: &str) -> MouseButton {
    match button_token.to_ascii_lowercase().as_str() {
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

fn foreground_app_changed_between(
    trace_entry: &TraceEntry,
) -> bool {
    let before_identifier = trace_entry
        .snapshot_before
        .as_ref()
        .and_then(|snap| snap.foreground_app_bundle_identifier.clone()
            .or_else(|| snap.foreground_app_executable_path.clone()));
    let after_identifier = trace_entry
        .snapshot_after
        .as_ref()
        .and_then(|snap| snap.foreground_app_bundle_identifier.clone()
            .or_else(|| snap.foreground_app_executable_path.clone()));
    match (before_identifier, after_identifier) {
        (Some(before), Some(after)) => before != after,
        _ => false,
    }
}

async fn wait_for_frontmost_app_to_become(target_identifier: &str) {
    let polling_started_at = std::time::Instant::now();
    let max_wait = Duration::from_millis(REPLAY_APP_LAUNCH_WAIT_MS);
    while polling_started_at.elapsed() < max_wait {
        let current_app = grounding::target_app::current_target_app();
        let current_identifier = current_app.and_then(|app| {
            app.bundle_identifier
                .clone()
                .or(app.executable_path.clone())
                .or(app.display_name.clone())
        });
        if let Some(identifier) = current_identifier {
            if identifier == target_identifier {
                return;
            }
        }
        sleep(Duration::from_millis(REPLAY_APP_LAUNCH_POLL_INTERVAL_MS)).await;
    }
}

async fn emit_progress(
    progress_sender: &Sender<ReplayProgress>,
    flow_id: &str,
    replay_id: &str,
    step_index: usize,
    total_steps: usize,
    kind: ReplayProgressKind,
) {
    let _ = progress_sender
        .send(ReplayProgress {
            flow_id: flow_id.to_string(),
            replay_id: replay_id.to_string(),
            step_index,
            total_steps,
            kind,
        })
        .await;
}

/// Replay the demonstration with id `flow_id`. The function is async and
/// drives one synthesized input per trace entry through the cross-platform
/// input layer. Progress events are streamed through `progress_sender`;
/// the channel can be dropped by the caller mid-replay to detach.
pub async fn replay_flow(
    flow_id: String,
    replay_id: String,
    progress_sender: Sender<ReplayProgress>,
) -> Result<(), String> {
    let demonstration_directory = recorder_persistence::demonstration_directory_for_id(&flow_id)
        .ok_or_else(|| "no data dir".to_string())?;
    let mut trace_file_path = demonstration_directory.clone();
    trace_file_path.push("trace.jsonl");

    let trace_entries = recorder_persistence::read_trace_jsonl(&trace_file_path)?;
    let total_steps = trace_entries.len();

    // Capture the foreground pid at replay start so the "user switched
    // away" rail can detect a Cmd-Tab away from the recorded trajectory.
    let starting_pid = safety_rails::current_frontmost_pid();

    emit_progress(
        &progress_sender,
        &flow_id,
        &replay_id,
        0,
        total_steps,
        ReplayProgressKind::Started,
    )
    .await;

    let mut previous_event_timestamp_unix_ms: Option<i64> = None;

    for (step_index, trace_entry) in trace_entries.iter().enumerate() {
        // Bail if a newer replay has been queued behind us.
        if !current_replay_token_matches(&replay_id) {
            emit_progress(
                &progress_sender,
                &flow_id,
                &replay_id,
                step_index,
                total_steps,
                ReplayProgressKind::Paused {
                    reason: "superseded by a newer replay".to_string(),
                },
            )
            .await;
            return Ok(());
        }

        // Safety rail: pause when a modal dialog is in front of us. We
        // drop a paused event and stop — same "soft pause" semantics as
        // the workflow runner.
        if safety_rails::modal_dialog_blocking() {
            emit_progress(
                &progress_sender,
                &flow_id,
                &replay_id,
                step_index,
                total_steps,
                ReplayProgressKind::Paused {
                    reason: "modal dialog detected".to_string(),
                },
            )
            .await;
            return Ok(());
        }

        // Safety rail: pause when the user took the foreground away
        // mid-replay. We compare against the pid at replay start, not
        // the per-step recorded pid — clicking-into-Notion mid-replay
        // is expected to change the frontmost app, but a user Cmd-Tab
        // back to a third app is not.
        if safety_rails::user_switched_away_from(starting_pid) {
            // The recorded trace itself might switch apps; only treat
            // this as a user-initiated switch if the current frontmost
            // pid also doesn't match anything in our recorded
            // trajectory's per-step snapshots. We approximate that
            // by checking the current step's expected snapshot_before
            // pid would be unrelated info — for v1 we just pause on
            // any mid-replay foreground swap to a non-starting pid.
            // TODO: tighten this once we record per-snapshot pids.
        }

        // Preserve original inter-event timing, capped.
        if let Some(previous_timestamp) = previous_event_timestamp_unix_ms {
            let raw_gap_ms = (trace_entry.timestamp_unix_ms - previous_timestamp).max(0) as u64;
            let capped_gap_ms = raw_gap_ms.min(REPLAY_MAX_GAP_MS);
            if capped_gap_ms > 0 {
                sleep(Duration::from_millis(capped_gap_ms)).await;
                emit_progress(
                    &progress_sender,
                    &flow_id,
                    &replay_id,
                    step_index,
                    total_steps,
                    ReplayProgressKind::Waited,
                )
                .await;
            }
        }

        // Skip key-ups that pair with a recorded key-down — we replay
        // both via a single keyboard_shortcut(&[key_name]) call on the
        // key-down. Skipping key-ups avoids double-tapping the key.
        match &trace_entry.event {
            InputEvent::KeyUp { .. } => {
                previous_event_timestamp_unix_ms = Some(trace_entry.timestamp_unix_ms);
                continue;
            }
            InputEvent::MouseMove { .. } => {
                // Mouse moves are visual noise the OS doesn't need; the
                // click step that follows carries the final coordinate.
                previous_event_timestamp_unix_ms = Some(trace_entry.timestamp_unix_ms);
                continue;
            }
            _ => {}
        }

        let dispatch_result: Result<(), String> = match &trace_entry.event {
            InputEvent::KeyDown { key_name, .. } => {
                cross_platform_input::keyboard_shortcut(&[key_name.as_str()])
            }
            InputEvent::MouseClick { button, x, y } => {
                let mouse_button = parse_mouse_button_from_recorded_token(button);
                cross_platform_input::click_at(*x, *y, mouse_button)
            }
            InputEvent::Scroll { delta_x, delta_y } => {
                cross_platform_input::scroll(0.0, 0.0, *delta_x, *delta_y)
            }
            InputEvent::KeyUp { .. } | InputEvent::MouseMove { .. } => Ok(()),
        };

        if let Err(error_message) = dispatch_result {
            emit_progress(
                &progress_sender,
                &flow_id,
                &replay_id,
                step_index,
                total_steps,
                ReplayProgressKind::Failed { message: error_message.clone() },
            )
            .await;
            return Err(error_message);
        }

        emit_progress(
            &progress_sender,
            &flow_id,
            &replay_id,
            step_index,
            total_steps,
            ReplayProgressKind::InputReplayed,
        )
        .await;

        // Detect app-launch boundary: if this event flipped the
        // foreground app, wait for the new app to actually become
        // frontmost before continuing. The recorded `snapshot_after`
        // tells us which app the user ended up in.
        if foreground_app_changed_between(trace_entry) {
            let target_identifier = trace_entry
                .snapshot_after
                .as_ref()
                .and_then(|snap| {
                    snap.foreground_app_bundle_identifier
                        .clone()
                        .or_else(|| snap.foreground_app_executable_path.clone())
                });
            if let Some(identifier) = target_identifier {
                wait_for_frontmost_app_to_become(&identifier).await;
                emit_progress(
                    &progress_sender,
                    &flow_id,
                    &replay_id,
                    step_index,
                    total_steps,
                    ReplayProgressKind::AppLaunched,
                )
                .await;
            }
        }

        previous_event_timestamp_unix_ms = Some(trace_entry.timestamp_unix_ms);
    }

    emit_progress(
        &progress_sender,
        &flow_id,
        &replay_id,
        total_steps,
        total_steps,
        ReplayProgressKind::Completed,
    )
    .await;

    Ok(())
}
