// Always-on, fully on-device speech listener. Compiles on every target;
// the heavy real-speech path is feature-gated behind `vosk` because the
// `vosk` crate dynamically links `libvosk` and that shared library isn't
// available in every dev sandbox. With the feature off the Tauri
// commands still exist — they return a clear `Err(...)` instead of
// silently lying about being enabled.
//
// Privacy invariant: audio never leaves the machine in this mode. Vosk
// runs locally against a downloaded model directory; nothing here opens
// a network socket except the one-time model-zip download from
// alphacephei.com.

pub mod grammar;
pub mod persistence;
pub mod recognizer;
#[cfg(feature = "vosk")]
pub mod wake_word;

use serde::Serialize;
use tauri::AppHandle;

use persistence::{
    default_vosk_model_directory, is_model_present, load_settings, save_settings, VoskSettings,
};

/// User-visible status of the local model + listener thread. Returned by
/// the install / download commands so the frontend can render "Ready",
/// "Downloading…", etc. without polling separate endpoints.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VoskModelStatus {
    /// Model directory exists locally and looks populated.
    Ready,
    /// Just finished downloading + unzipping; reports total bytes pulled.
    Downloaded { bytes_total: u64 },
    /// Download or unzip failed; reason is a human-readable string.
    Failed { reason: String },
}

#[tauri::command]
pub fn is_listener_enabled() -> Result<bool, String> {
    Ok(load_settings().is_enabled)
}

#[tauri::command]
pub fn set_listener_enabled(enabled: bool, app: AppHandle) -> Result<(), String> {
    let mut current_settings = load_settings();
    current_settings.is_enabled = enabled;
    save_settings(&current_settings)?;
    if enabled {
        start_listener(app)?;
    } else {
        stop_listener()?;
    }
    Ok(())
}

#[tauri::command]
pub fn start_listener(app: AppHandle) -> Result<(), String> {
    start_listener_impl(app)
}

#[tauri::command]
pub fn stop_listener() -> Result<(), String> {
    stop_listener_impl()
}

#[tauri::command]
pub fn download_vosk_model_if_needed() -> Result<VoskModelStatus, String> {
    download_vosk_model_if_needed_impl()
}

// -- Non-feature build: every entry point returns a clear "not built in"
// error so the frontend gets a single, predictable error string instead
// of mysterious silent no-ops. ----------------------------------------

#[cfg(not(feature = "vosk"))]
fn start_listener_impl(_app: AppHandle) -> Result<(), String> {
    Err("vosk listener not built in; rebuild with --features vosk".to_string())
}

#[cfg(not(feature = "vosk"))]
fn stop_listener_impl() -> Result<(), String> {
    Err("vosk listener not built in; rebuild with --features vosk".to_string())
}

#[cfg(not(feature = "vosk"))]
fn download_vosk_model_if_needed_impl() -> Result<VoskModelStatus, String> {
    Err("vosk listener not built in; rebuild with --features vosk".to_string())
}

/// Auto-start hook for `main.rs`. Tries to start the listener at boot
/// only when the user has previously opted in AND the model is already
/// on disk. Errors are swallowed (logged) — failing to start the
/// optional always-on listener must not stop TipTour from launching.
pub fn auto_start_if_user_opted_in(app: &AppHandle) {
    let settings = load_settings();
    if !settings.is_enabled {
        return;
    }
    if !is_model_present(&settings) {
        eprintln!("[vosk] opted in but model directory missing; skipping auto-start");
        return;
    }
    if let Err(error) = start_listener(app.clone()) {
        eprintln!("[vosk] auto-start failed: {error}");
    }
}

// -- Feature-on build below. -------------------------------------------

#[cfg(feature = "vosk")]
mod feature_on {
    use super::*;

    use std::fs;
    use std::io::Read;
    use std::sync::Arc;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Sample, SampleFormat};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use tauri::Emitter;

    use super::wake_word::WakeWordDispatcher;

    const LOCAL_LISTENER_SAMPLE_RATE_HZ: u32 = 16_000;
    const MODEL_DOWNLOAD_URL: &str =
        "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip";

    // Same Send/Sync override pattern as `audio.rs`: cpal marks Stream as
    // !Send everywhere by default, but we hold ours behind a Mutex and
    // only touch it from the single Tauri command thread that creates
    // and drops it.
    struct SendableLocalStream(cpal::Stream);
    unsafe impl Send for SendableLocalStream {}
    unsafe impl Sync for SendableLocalStream {}

    struct LocalListenerState {
        mic_stream: Option<SendableLocalStream>,
        dispatcher: Option<Arc<Mutex<WakeWordDispatcher>>>,
    }

    static LISTENER_STATE: Lazy<Mutex<LocalListenerState>> = Lazy::new(|| {
        Mutex::new(LocalListenerState {
            mic_stream: None,
            dispatcher: None,
        })
    });

    pub fn start_listener_feature_on(app: AppHandle) -> Result<(), String> {
        // Idempotent — calling start while already running is a no-op
        // so the auto-start path can't double-install the stream.
        {
            let state = LISTENER_STATE.lock();
            if state.mic_stream.is_some() {
                return Ok(());
            }
        }

        let settings = load_settings();
        if !is_model_present(&settings) {
            return Err(
                "vosk model not downloaded; call download_vosk_model_if_needed first".to_string(),
            );
        }
        let model_directory = settings
            .model_path
            .clone()
            .or_else(default_vosk_model_directory)
            .ok_or_else(|| "no data dir for model path".to_string())?;

        let dispatcher = Arc::new(Mutex::new(WakeWordDispatcher::new(
            app.clone(),
            model_directory,
        )?));

        // Build a second, always-on input stream alongside any
        // Gemini-Live `mic_chunk` stream. cpal allows multiple input
        // streams on the same device; we deliberately emit on a
        // separate Tauri event (`vosk_mic_chunk`) so this never
        // interferes with the Live session's audio path.
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no default input device".to_string())?;
        let supported = device
            .default_input_config()
            .map_err(|error| format!("default_input_config: {error}"))?;
        let sample_format = supported.sample_format();
        let native_rate = supported.sample_rate().0;
        let native_channels = supported.channels();
        let native_config = supported.config();

        let dispatcher_for_callback = dispatcher.clone();
        let app_handle_for_callback = app.clone();
        let resampler_state = Arc::new(Mutex::new(StreamingResamplerState::new()));

        let stream_result = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &native_config,
                {
                    let resampler_state = resampler_state.clone();
                    let dispatcher_for_callback = dispatcher_for_callback.clone();
                    let app_handle_for_callback = app_handle_for_callback.clone();
                    move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                        let mono = downmix_to_mono_f32(samples, native_channels);
                        let resampled = resample_linear_streaming(
                            &resampler_state,
                            &mono,
                            native_rate,
                            LOCAL_LISTENER_SAMPLE_RATE_HZ,
                        );
                        feed_dispatcher_with_resampled(
                            &dispatcher_for_callback,
                            &app_handle_for_callback,
                            &resampled,
                        );
                    }
                },
                |error| eprintln!("[vosk] input stream error: {error}"),
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &native_config,
                {
                    let resampler_state = resampler_state.clone();
                    let dispatcher_for_callback = dispatcher_for_callback.clone();
                    let app_handle_for_callback = app_handle_for_callback.clone();
                    move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                        let as_f32: Vec<f32> =
                            samples.iter().map(|s| s.to_sample::<f32>()).collect();
                        let mono = downmix_to_mono_f32(&as_f32, native_channels);
                        let resampled = resample_linear_streaming(
                            &resampler_state,
                            &mono,
                            native_rate,
                            LOCAL_LISTENER_SAMPLE_RATE_HZ,
                        );
                        feed_dispatcher_with_resampled(
                            &dispatcher_for_callback,
                            &app_handle_for_callback,
                            &resampled,
                        );
                    }
                },
                |error| eprintln!("[vosk] input stream error: {error}"),
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &native_config,
                {
                    let resampler_state = resampler_state.clone();
                    let dispatcher_for_callback = dispatcher_for_callback.clone();
                    let app_handle_for_callback = app_handle_for_callback.clone();
                    move |samples: &[u16], _: &cpal::InputCallbackInfo| {
                        let as_f32: Vec<f32> =
                            samples.iter().map(|s| s.to_sample::<f32>()).collect();
                        let mono = downmix_to_mono_f32(&as_f32, native_channels);
                        let resampled = resample_linear_streaming(
                            &resampler_state,
                            &mono,
                            native_rate,
                            LOCAL_LISTENER_SAMPLE_RATE_HZ,
                        );
                        feed_dispatcher_with_resampled(
                            &dispatcher_for_callback,
                            &app_handle_for_callback,
                            &resampled,
                        );
                    }
                },
                |error| eprintln!("[vosk] input stream error: {error}"),
                None,
            ),
            other => return Err(format!("unsupported sample format: {other:?}")),
        };

        let stream = stream_result.map_err(|error| format!("build_input_stream: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("stream.play: {error}"))?;

        let mut state = LISTENER_STATE.lock();
        state.mic_stream = Some(SendableLocalStream(stream));
        state.dispatcher = Some(dispatcher);
        Ok(())
    }

    pub fn stop_listener_feature_on() -> Result<(), String> {
        let mut state = LISTENER_STATE.lock();
        state.mic_stream = None;
        state.dispatcher = None;
        Ok(())
    }

    pub fn download_vosk_model_feature_on() -> Result<VoskModelStatus, String> {
        let settings = load_settings();
        if is_model_present(&settings) {
            return Ok(VoskModelStatus::Ready);
        }

        let target_directory = settings
            .model_path
            .clone()
            .or_else(default_vosk_model_directory)
            .ok_or_else(|| "no data dir for model path".to_string())?;
        fs::create_dir_all(&target_directory).map_err(|error| error.to_string())?;

        // Stream the zip to a tmp file rather than buffering it in
        // memory. The small en-US model is ~40MB but larger Vosk models
        // can be 1-2GB; we don't want to OOM the moment a user picks
        // one of those. ZipArchive needs Read+Seek, which File gives
        // us natively — no Cursor<Vec<u8>> in the hot path.
        //
        // Three retries with exponential backoff because
        // alphacephei.com's CDN occasionally throttles bursts and the
        // user just sees a download fail message with no clear remedy.
        let response = {
            let mut last_error: Option<String> = None;
            let mut attempt: u32 = 0;
            loop {
                attempt += 1;
                match ureq::get(MODEL_DOWNLOAD_URL).call() {
                    Ok(response) => break response,
                    Err(error) => {
                        last_error = Some(error.to_string());
                        if attempt >= 3 {
                            return Err(format!(
                                "Vosk model download failed after 3 attempts from {MODEL_DOWNLOAD_URL}: \
                                 {}. Check your internet connection. If your network blocks \
                                 alphacephei.com, download the model manually and unzip it to \
                                 your TipTour data folder.",
                                last_error.unwrap_or_else(|| "no detail".into())
                            ));
                        }
                        // 1.5s -> 4.5s -> 13.5s backoff so a transient
                        // throttle has time to clear.
                        let delay_ms = 1_500u64 * 3u64.pow(attempt - 1);
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    }
                }
            }
        };
        let mut temporary_zip_path = target_directory.clone();
        temporary_zip_path.push(".download.zip");
        let mut temporary_zip_file =
            fs::File::create(&temporary_zip_path).map_err(|error| {
                format!("create temp zip {}: {error}", temporary_zip_path.display())
            })?;
        let bytes_total =
            std::io::copy(&mut response.into_reader(), &mut temporary_zip_file)
                .map_err(|error| format!("stream zip body to disk: {error}"))?;
        temporary_zip_file
            .sync_all()
            .map_err(|error| format!("flush temp zip: {error}"))?;
        drop(temporary_zip_file);

        let reopened_zip_file = fs::File::open(&temporary_zip_path)
            .map_err(|error| format!("reopen temp zip: {error}"))?;
        let mut archive = zip::ZipArchive::new(reopened_zip_file)
            .map_err(|error| format!("open zip: {error}"))?;

        // The upstream zip nests everything under a top-level
        // `vosk-model-small-en-us-0.15/` directory. Strip that prefix
        // so the populated files land directly under our target dir —
        // `is_model_present` checks for any subentry, and Vosk wants
        // the directory you pass to `Model::new` to be the model root.
        for entry_index in 0..archive.len() {
            let mut entry = archive
                .by_index(entry_index)
                .map_err(|error| format!("zip entry {entry_index}: {error}"))?;
            let entry_name = entry.name().to_string();
            let stripped_relative_path = match entry_name.split_once('/') {
                Some((_top_prefix, rest)) if !rest.is_empty() => rest.to_string(),
                _ => continue,
            };
            let destination_path = target_directory.join(&stripped_relative_path);
            if entry.is_dir() {
                fs::create_dir_all(&destination_path).map_err(|error| error.to_string())?;
                continue;
            }
            if let Some(parent_dir) = destination_path.parent() {
                fs::create_dir_all(parent_dir).map_err(|error| error.to_string())?;
            }
            let mut output_file =
                fs::File::create(&destination_path).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut output_file)
                .map_err(|error| format!("extract: {error}"))?;
        }

        // Best-effort cleanup of the staged zip; if it fails, the next
        // download just overwrites it. Not worth aborting on.
        let _ = fs::remove_file(&temporary_zip_path);

        Ok(VoskModelStatus::Downloaded { bytes_total })
    }

    fn feed_dispatcher_with_resampled(
        dispatcher: &Arc<Mutex<WakeWordDispatcher>>,
        app_handle: &AppHandle,
        resampled_samples_f32: &[f32],
    ) {
        if resampled_samples_f32.is_empty() {
            return;
        }
        // Reuse the same PCM16 byte event shape as `audio::start_mic_capture`
        // (an opaque `Vec<u8>` payload). Frontends that already wired up
        // mic visualizers can listen to either event with the same decoder.
        let mut pcm16_bytes: Vec<u8> = Vec::with_capacity(resampled_samples_f32.len() * 2);
        let mut pcm16_samples: Vec<i16> = Vec::with_capacity(resampled_samples_f32.len());
        for sample_f32 in resampled_samples_f32 {
            let clamped = sample_f32.clamp(-1.0, 1.0);
            let pcm16_value = (clamped * i16::MAX as f32) as i16;
            pcm16_bytes.extend_from_slice(&pcm16_value.to_le_bytes());
            pcm16_samples.push(pcm16_value);
        }
        let _ = app_handle.emit("vosk_mic_chunk", pcm16_bytes);
        dispatcher.lock().accept_pcm16(&pcm16_samples);
    }

    fn downmix_to_mono_f32(samples: &[f32], channels: u16) -> Vec<f32> {
        if channels <= 1 {
            return samples.to_vec();
        }
        let channels_usize = channels as usize;
        let mut out = Vec::with_capacity(samples.len() / channels_usize);
        for frame in samples.chunks_exact(channels_usize) {
            let avg: f32 = frame.iter().sum::<f32>() / channels as f32;
            out.push(avg);
        }
        out
    }

    struct StreamingResamplerState {
        fractional_position: f32,
    }
    impl StreamingResamplerState {
        fn new() -> Self {
            Self {
                fractional_position: 0.0,
            }
        }
    }

    fn resample_linear_streaming(
        state: &Mutex<StreamingResamplerState>,
        input: &[f32],
        input_rate: u32,
        output_rate: u32,
    ) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if input_rate == output_rate {
            return input.to_vec();
        }
        let mut state_guard = state.lock();
        let ratio = input_rate as f32 / output_rate as f32;
        let mut out = Vec::with_capacity((input.len() as f32 / ratio) as usize + 1);
        let mut position = state_guard.fractional_position;
        while position < input.len() as f32 {
            let index = position as usize;
            let frac = position - index as f32;
            let current = input[index];
            let next = if index + 1 < input.len() {
                input[index + 1]
            } else {
                current
            };
            out.push(current + (next - current) * frac);
            position += ratio;
        }
        state_guard.fractional_position = position - input.len() as f32;
        out
    }
}

#[cfg(feature = "vosk")]
fn start_listener_impl(app: AppHandle) -> Result<(), String> {
    feature_on::start_listener_feature_on(app)
}

#[cfg(feature = "vosk")]
fn stop_listener_impl() -> Result<(), String> {
    feature_on::stop_listener_feature_on()
}

#[cfg(feature = "vosk")]
fn download_vosk_model_if_needed_impl() -> Result<VoskModelStatus, String> {
    feature_on::download_vosk_model_feature_on()
}

// Re-export so external callers don't need to know about the
// feature-gating split.
pub use persistence::VoskSettings as ExportedVoskSettings;
#[allow(dead_code)]
fn _ensure_exports_keep_compiling(_s: &VoskSettings) {}
