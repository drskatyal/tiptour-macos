// Background screen-frame streamer.
//
// A single tokio task captures a frame every `CAPTURE_INTERVAL`, runs the
// dHash dedup so we don't pay JPEG + IPC + Gemini-input-token cost on
// unchanged scenes, encodes the frame to JPEG, and emits a Tauri event
// named `screen_frame` whose payload carries the base64-encoded JPEG.
// The TypeScript session listens for that event and forwards the bytes
// to Gemini Live via `realtimeInput.mediaChunks`.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::capture::capture_primary_screen;
use super::dhash::{perceptual_hash, should_skip, DEFAULT_SAME_SCENE_THRESHOLD};
use super::jpeg::encode_frame_to_jpeg;

const CAPTURE_INTERVAL: Duration = Duration::from_millis(1_500);

#[derive(Serialize, Clone)]
struct ScreenFramePayload {
    #[serde(rename = "jpegBase64")]
    jpeg_base64: String,
    width: u32,
    height: u32,
}

struct StreamerState {
    handle: Option<JoinHandle<()>>,
    cancel: Option<oneshot::Sender<()>>,
}

static STREAMER: Lazy<Mutex<StreamerState>> = Lazy::new(|| {
    Mutex::new(StreamerState {
        handle: None,
        cancel: None,
    })
});

pub fn start_streaming(app: AppHandle) -> Result<(), String> {
    let mut state = STREAMER.lock();
    if state.handle.is_some() {
        // Already running. Treat as idempotent.
        return Ok(());
    }

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

    let app_for_task = app.clone();
    let handle = tokio::spawn(async move {
        let mut previous_hash: Option<u64> = None;
        let mut ticker = tokio::time::interval(CAPTURE_INTERVAL);
        // Capture once immediately, then on the configured cadence.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    break;
                }
                _ = ticker.tick() => {
                    let frame = match capture_primary_screen().await {
                        Ok(frame) => frame,
                        Err(error) => {
                            eprintln!("[screen] capture failed: {error}");
                            // Permission-denied or transient errors — back
                            // off briefly so we don't spin retrying.
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                    };

                    let current_hash = perceptual_hash(&frame);
                    if should_skip(previous_hash, current_hash, DEFAULT_SAME_SCENE_THRESHOLD) {
                        continue;
                    }
                    previous_hash = Some(current_hash);

                    let jpeg = match encode_frame_to_jpeg(&frame) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            eprintln!("[screen] jpeg encode failed: {error}");
                            continue;
                        }
                    };
                    let base64 = BASE64_STANDARD.encode(&jpeg);
                    let payload = ScreenFramePayload {
                        jpeg_base64: base64,
                        width: frame.width,
                        height: frame.height,
                    };
                    if let Err(error) = app_for_task.emit("screen_frame", payload) {
                        eprintln!("[screen] emit screen_frame failed: {error}");
                    }
                }
            }
        }
    });

    state.handle = Some(handle);
    state.cancel = Some(cancel_tx);
    Ok(())
}

pub fn stop_streaming() -> Result<(), String> {
    let mut state = STREAMER.lock();
    if let Some(cancel) = state.cancel.take() {
        let _ = cancel.send(());
    }
    if let Some(handle) = state.handle.take() {
        handle.abort();
    }
    Ok(())
}
