// Mic capture (16 kHz PCM16 → Tauri event) and playback (24 kHz PCM16 chunks).
//
// We deliberately keep audio on the Rust side: cpal gives us low-latency
// device access on both OSes and avoids the WebView's audio quirks. Frames
// cross the IPC boundary as JSON byte arrays for Phase 0; a future revision
// can swap to raw IPC channels if the JSON encode/decode shows up in profiles.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, StreamConfig};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

const UPLINK_SAMPLE_RATE_HZ: u32 = 16_000;
const DOWNLINK_SAMPLE_RATE_HZ: u32 = 24_000;

// cpal marks Stream as !Send/!Sync on every platform to discourage misuse,
// but our access is strictly serialized through the surrounding Mutex and
// streams are created/dropped from the same Tauri command thread. Wrap the
// streams in a Send/Sync-asserting newtype so we can hold them in a static.
struct SendableStream(cpal::Stream);
unsafe impl Send for SendableStream {}
unsafe impl Sync for SendableStream {}

struct AudioState {
    input_stream: Option<SendableStream>,
    output_stream: Option<SendableStream>,
    playback_queue: Arc<Mutex<Vec<i16>>>,
}

static STATE: Lazy<Mutex<AudioState>> = Lazy::new(|| {
    Mutex::new(AudioState {
        input_stream: None,
        output_stream: None,
        playback_queue: Arc::new(Mutex::new(Vec::with_capacity(48_000))),
    })
});

#[derive(Serialize, Clone)]
struct MicChunkPayload(Vec<u8>);

#[tauri::command]
pub fn start_mic_capture(app: AppHandle) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No input device".to_string())?;

    let supported = device
        .default_input_config()
        .map_err(|error| error.to_string())?;

    let sample_format = supported.sample_format();
    let config: StreamConfig = StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(UPLINK_SAMPLE_RATE_HZ),
        buffer_size: cpal::BufferSize::Default,
    };

    let app_for_callback = app.clone();
    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                &config,
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    let pcm16 = f32_to_pcm16_bytes(samples);
                    let _ = app_for_callback.emit("mic_chunk", MicChunkPayload(pcm16));
                },
                |error| eprintln!("input stream error: {error}"),
                None,
            )
            .map_err(|error| error.to_string())?,
        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                    let pcm16 = i16_to_pcm16_bytes(samples);
                    let _ = app_for_callback.emit("mic_chunk", MicChunkPayload(pcm16));
                },
                |error| eprintln!("input stream error: {error}"),
                None,
            )
            .map_err(|error| error.to_string())?,
        other => return Err(format!("Unsupported sample format: {other:?}")),
    };

    stream.play().map_err(|error| error.to_string())?;
    STATE.lock().input_stream = Some(SendableStream(stream));

    ensure_output_stream()?;

    Ok(())
}

#[tauri::command]
pub fn stop_mic_capture() -> Result<(), String> {
    let mut state = STATE.lock();
    state.input_stream = None;
    state.playback_queue.lock().clear();
    Ok(())
}

#[tauri::command]
pub fn play_audio_chunk(pcm: Vec<u8>) -> Result<(), String> {
    // pcm is little-endian PCM16 @ 24 kHz from Gemini.
    let mut samples = Vec::with_capacity(pcm.len() / 2);
    let mut iter = pcm.chunks_exact(2);
    while let Some(chunk) = iter.next() {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    let queue = STATE.lock().playback_queue.clone();
    queue.lock().extend(samples);
    Ok(())
}

fn ensure_output_stream() -> Result<(), String> {
    let mut state = STATE.lock();
    if state.output_stream.is_some() {
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "No output device".to_string())?;

    let config: StreamConfig = StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(DOWNLINK_SAMPLE_RATE_HZ),
        buffer_size: cpal::BufferSize::Default,
    };

    let queue = state.playback_queue.clone();
    let stream = device
        .build_output_stream(
            &config,
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut q = queue.lock();
                let take = output.len().min(q.len());
                for (slot, sample) in output.iter_mut().zip(q.drain(..take)) {
                    *slot = sample.to_sample::<f32>();
                }
                for slot in output[take..].iter_mut() {
                    *slot = 0.0;
                }
            },
            |error| eprintln!("output stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())?;

    stream.play().map_err(|error| error.to_string())?;
    state.output_stream = Some(SendableStream(stream));
    Ok(())
}

fn f32_to_pcm16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn i16_to_pcm16_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}
