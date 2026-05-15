// Orchestrator that ties the input channel, state provider, audio recorder,
// and privacy gate together. Two modes:
//   - Passive       : UIA-only trace, no audio, no screenshots; recorded
//                     into a per-session JSONL under passive_traces/.
//   - Demonstration : UIA + audio narration + JPEG screenshots at each
//                     visual transition (clicks and key presses, not mouse
//                     moves). Each run lives under demonstrations/{id}/,
//                     with screenshots/{timestamp_unix_ms}.jpg alongside
//                     trace.jsonl and narration.wav.
//
// The recorder thread polls the input event receiver and consults the
// privacy gate on every event. If the gate says "pause", the event is
// discarded — it never touches disk.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use parking_lot::Mutex;

use super::audio_recorder::AudioRecorder;
use super::input_capture::{self, InputCaptureHandle};
use super::pattern_miner;
use super::persistence::{
    self, append_trace_entry, demonstration_directory_for_id, write_demonstration_meta,
};
use super::privacy;
use super::state_capture::{NullStateProvider, StateProvider};
use super::types::{
    Demonstration, DemonstrationSummary, RecordingMode, StateSnapshot, TraceEntry,
};

pub struct ActiveRecording {
    pub mode: RecordingMode,
    pub demonstration_id: Option<String>,
    pub demonstration_title: Option<String>,
    pub trace_file_path: PathBuf,
    // In demonstration mode, screenshots at every transition point land
    // under {demonstration_directory}/screenshots/{timestamp}.jpg. None in
    // passive mode — passive recordings never write pixels.
    pub demonstration_directory: Option<PathBuf>,
    pub audio_recorder: Option<Arc<AudioRecorder>>,
    pub trace_entries_in_memory: Vec<TraceEntry>,
    pub session_started_unix_ms: i64,
    pub should_keep_running: Arc<AtomicBool>,
}

pub struct Recorder {
    state_provider: Arc<dyn StateProvider>,
    active_recording: Mutex<Option<ActiveRecording>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state_provider: Arc::new(NullStateProvider),
            active_recording: Mutex::new(None),
        }
    }

    pub fn set_state_provider(&self, state_provider: Arc<dyn StateProvider>) {
        // Replacing the provider mid-recording is allowed but only takes
        // effect on subsequent events. The trait object is cheap to clone.
        let _ = state_provider;
        // We can't actually mutate `state_provider` through &self with the
        // current Arc<dyn ...> shape; callers that need to swap should hold
        // their own ref and pass it into `start_passive_recording_with_provider`.
        // Keeping this method as a placeholder so the public API mirrors
        // the recorder's intended ownership model.
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.lock().is_some()
    }

    pub fn current_mode(&self) -> Option<RecordingMode> {
        self.active_recording.lock().as_ref().map(|active| active.mode)
    }

    pub fn start_passive_recording(self: &Arc<Self>) -> Result<(), String> {
        let mut active_recording_guard = self.active_recording.lock();
        if active_recording_guard.is_some() {
            return Err("recording already active".to_string());
        }
        let session_started_unix_ms = Utc::now().timestamp_millis();
        let trace_file_path =
            pattern_miner::passive_trace_file_path_for_session(session_started_unix_ms)
                .ok_or_else(|| "no data dir".to_string())?;
        let should_keep_running = Arc::new(AtomicBool::new(true));

        let active_recording = ActiveRecording {
            mode: RecordingMode::Passive,
            demonstration_id: None,
            demonstration_title: None,
            trace_file_path: trace_file_path.clone(),
            demonstration_directory: None,
            audio_recorder: None,
            trace_entries_in_memory: Vec::new(),
            session_started_unix_ms,
            should_keep_running: should_keep_running.clone(),
        };
        *active_recording_guard = Some(active_recording);
        drop(active_recording_guard);

        let input_capture_handle = input_capture::start();
        self.spawn_consume_loop(input_capture_handle, should_keep_running);
        Ok(())
    }

    pub fn stop_passive_recording(&self) -> Result<(), String> {
        let mut active_recording_guard = self.active_recording.lock();
        let active = match active_recording_guard.take() {
            Some(active) if active.mode == RecordingMode::Passive => active,
            Some(other) => {
                *active_recording_guard = Some(other);
                return Err("not in passive mode".to_string());
            }
            None => return Err("no active recording".to_string()),
        };
        active.should_keep_running.store(false, Ordering::SeqCst);
        input_capture::stop();
        Ok(())
    }

    pub fn start_demonstration(self: &Arc<Self>, title: String) -> Result<String, String> {
        let mut active_recording_guard = self.active_recording.lock();
        if active_recording_guard.is_some() {
            return Err("recording already active".to_string());
        }
        let demonstration_id = generate_demonstration_id();
        let session_started_unix_ms = Utc::now().timestamp_millis();

        let demonstration_directory = demonstration_directory_for_id(&demonstration_id)
            .ok_or_else(|| "no data dir".to_string())?;
        std::fs::create_dir_all(&demonstration_directory).map_err(|error| error.to_string())?;

        let mut trace_file_path = demonstration_directory.clone();
        trace_file_path.push("trace.jsonl");

        let mut narration_file_path = demonstration_directory.clone();
        narration_file_path.push("narration.wav");
        let audio_recorder = Arc::new(AudioRecorder::create(narration_file_path)?);

        let initial_summary = DemonstrationSummary {
            id: demonstration_id.clone(),
            title: title.clone(),
            created_at_unix_ms: session_started_unix_ms,
            trace_entry_count: 0,
            has_narration_audio: true,
        };
        write_demonstration_meta(&demonstration_id, &initial_summary)?;

        let should_keep_running = Arc::new(AtomicBool::new(true));
        let active_recording = ActiveRecording {
            mode: RecordingMode::Demonstration,
            demonstration_id: Some(demonstration_id.clone()),
            demonstration_title: Some(title),
            trace_file_path: trace_file_path.clone(),
            demonstration_directory: Some(demonstration_directory.clone()),
            audio_recorder: Some(audio_recorder),
            trace_entries_in_memory: Vec::new(),
            session_started_unix_ms,
            should_keep_running: should_keep_running.clone(),
        };
        *active_recording_guard = Some(active_recording);
        drop(active_recording_guard);

        let input_capture_handle = input_capture::start();
        self.spawn_consume_loop(input_capture_handle, should_keep_running);
        Ok(demonstration_id)
    }

    pub fn stop_demonstration(&self) -> Result<Demonstration, String> {
        let mut active_recording_guard = self.active_recording.lock();
        let active = match active_recording_guard.take() {
            Some(active) if active.mode == RecordingMode::Demonstration => active,
            Some(other) => {
                *active_recording_guard = Some(other);
                return Err("not in demonstration mode".to_string());
            }
            None => return Err("no active recording".to_string()),
        };
        active.should_keep_running.store(false, Ordering::SeqCst);
        input_capture::stop();

        if let Some(audio_recorder) = &active.audio_recorder {
            let _ = audio_recorder.finalize();
        }

        let demonstration_id = active
            .demonstration_id
            .clone()
            .ok_or_else(|| "demonstration id missing".to_string())?;
        let demonstration_title = active
            .demonstration_title
            .clone()
            .unwrap_or_else(|| "Untitled".to_string());

        let final_summary = DemonstrationSummary {
            id: demonstration_id.clone(),
            title: demonstration_title.clone(),
            created_at_unix_ms: active.session_started_unix_ms,
            trace_entry_count: active.trace_entries_in_memory.len(),
            has_narration_audio: active.audio_recorder.is_some(),
        };
        write_demonstration_meta(&demonstration_id, &final_summary)?;

        let narration_audio_path = active
            .audio_recorder
            .as_ref()
            .map(|audio_recorder| audio_recorder.output_file_path().to_string_lossy().to_string());

        Ok(Demonstration {
            id: demonstration_id,
            title: demonstration_title,
            narration_audio_path,
            trace: active.trace_entries_in_memory,
            created_at_unix_ms: active.session_started_unix_ms,
        })
    }

    pub fn append_narration_audio_chunk(
        &self,
        pcm16_little_endian_bytes: &[u8],
    ) -> Result<(), String> {
        let active_recording_guard = self.active_recording.lock();
        let active = match active_recording_guard.as_ref() {
            Some(active) => active,
            None => return Ok(()),
        };
        if let Some(audio_recorder) = &active.audio_recorder {
            audio_recorder.append_pcm16_chunk(pcm16_little_endian_bytes)?;
        }
        Ok(())
    }

    fn spawn_consume_loop(
        self: &Arc<Self>,
        input_capture_handle: InputCaptureHandle,
        should_keep_running: Arc<AtomicBool>,
    ) {
        let recorder_for_thread = self.clone();
        thread::spawn(move || {
            while should_keep_running.load(Ordering::SeqCst) {
                let recv_result = input_capture_handle
                    .event_receiver
                    .recv_timeout(Duration::from_millis(100));
                let input_event = match recv_result {
                    Ok(event) => event,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };

                let snapshot_before = recorder_for_thread.state_provider.current_snapshot();
                if let Some(snapshot) = snapshot_before.as_ref() {
                    if privacy::should_pause(snapshot) {
                        continue;
                    }
                }
                let snapshot_after = recorder_for_thread.state_provider.current_snapshot();

                let trace_entry = TraceEntry {
                    timestamp_unix_ms: Utc::now().timestamp_millis(),
                    event: input_event,
                    snapshot_before,
                    snapshot_after,
                };

                // Snapshot the lightweight fields we need under the lock,
                // then release it before doing any disk I/O. The previous
                // shape held `active_recording.lock()` across
                // `fs::create_dir_all` and the trace append, which under
                // heavy input briefly stalled every other recorder-state
                // query. We push the trace entry back into the in-memory
                // vec after releasing the lock; the order is preserved by
                // the single-threaded consumer loop.
                let (trace_file_path, recording_mode, screenshots_directory) = {
                    let mut active_recording_guard =
                        recorder_for_thread.active_recording.lock();
                    let active = match active_recording_guard.as_mut() {
                        Some(active) => active,
                        None => break,
                    };
                    let trace_file_path = active.trace_file_path.clone();
                    let recording_mode = active.mode;
                    let screenshots_directory = active
                        .demonstration_directory
                        .as_ref()
                        .map(|directory| directory.join("screenshots"));
                    active.trace_entries_in_memory.push(trace_entry.clone());
                    (trace_file_path, recording_mode, screenshots_directory)
                };

                let _ = append_trace_entry(&trace_file_path, &trace_entry);

                // In demonstration mode, every input event is also a
                // visual transition point — capture a screenshot so the
                // saved demo can be replayed visually and so Gemini has
                // pixels to disambiguate ambiguous UIA snapshots. Passive
                // mode never persists screenshots (privacy gate).
                if matches!(recording_mode, RecordingMode::Demonstration)
                    && input_event_is_visual_transition(&trace_entry.event)
                {
                    let timestamp = trace_entry.timestamp_unix_ms;
                    if let Some(target_directory) = screenshots_directory {
                        let _ = std::fs::create_dir_all(&target_directory);
                        // Fire-and-forget the actual capture so the input
                        // hook keeps draining its event channel — a slow
                        // screenshot must not back up the trace.
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                write_demonstration_screenshot(target_directory, timestamp).await
                            {
                                eprintln!(
                                    "recorder: screenshot capture failed at {timestamp}: {error}"
                                );
                            }
                        });
                    }
                }
            }
        });
    }
}

fn generate_demonstration_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[allow(dead_code)]
pub fn list_passive_trace_files_for_test() -> Result<Vec<PathBuf>, String> {
    persistence::list_passive_trace_files()
}

// Suppress unused warnings for fields read indirectly.
#[allow(dead_code)]
fn _touch_state_snapshot(_snapshot: &StateSnapshot) {}

/// Visual transition points worth capturing a screenshot for in
/// demonstration mode. Mouse moves and scrolls fire continuously and
/// would generate hundreds of frames per minute — we don't capture
/// those. Clicks, key presses, and key releases are sparse and
/// meaningful, which makes them the right inflection points to remember.
fn input_event_is_visual_transition(event: &super::types::InputEvent) -> bool {
    use super::types::InputEvent;
    matches!(
        event,
        InputEvent::KeyDown { .. } | InputEvent::KeyUp { .. } | InputEvent::MouseClick { .. }
    )
}

/// Capture one JPEG screenshot of the primary display and write it under
/// the demonstration directory keyed on `timestamp_unix_ms`. Errors are
/// swallowed at the call site — a missed frame can't be allowed to back
/// up the trace pipeline.
async fn write_demonstration_screenshot(
    screenshots_directory: PathBuf,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let raw_frame = crate::screen::capture::capture_primary_screen().await?;
    let jpeg_bytes = crate::screen::jpeg::encode_frame_to_jpeg(&raw_frame)?;
    let mut output_path = screenshots_directory;
    output_path.push(format!("{timestamp_unix_ms}.jpg"));
    std::fs::write(&output_path, &jpeg_bytes).map_err(|error| error.to_string())?;
    // Side-of-screen pill so the user sees a discreet confirmation that
    // a screenshot landed on disk during demonstration recording.
    crate::indicators::emit_via_global(
        crate::indicators::IndicatorKind::Screenshot,
        "Screenshot captured".to_string(),
        None,
        None,
    );
    Ok(())
}
