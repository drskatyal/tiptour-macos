// Screen capture pipeline for Gemini Live vision.
//
// Mirrors TipTour/CompanionScreenCaptureUtility.swift in spirit but
// targets two backends instead of one: ScreenCaptureKit on macOS and
// Windows.Graphics.Capture (via the `windows-capture` crate) on Windows.
// On Linux the module compiles as a stub so the rest of the crate keeps
// building during cross-platform development.
//
// The public surface is intentionally small: two Tauri commands —
// `start_screen_stream` and `stop_screen_stream` — and an event named
// `screen_frame` carrying base64-encoded JPEG bytes. Everything else
// (dHash dedup, resizing, encoding, the background ticker) lives in the
// submodules below.

pub mod capture;
pub mod dhash;
pub mod jpeg;
pub mod streamer;

use tauri::AppHandle;

#[tauri::command]
pub async fn start_screen_stream(app: AppHandle) -> Result<(), String> {
    streamer::start_streaming(app)
}

#[tauri::command]
pub async fn stop_screen_stream() -> Result<(), String> {
    streamer::stop_streaming()
}
