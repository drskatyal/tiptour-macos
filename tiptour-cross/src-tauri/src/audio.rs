// Mic capture (resampled to 16 kHz PCM16) and playback (24 kHz PCM16 from
// Gemini, resampled to the device's native rate).
//
// We deliberately keep audio on the Rust side: cpal gives us low-latency
// device access on both OSes and avoids the WebView's audio quirks. Frames
// cross the IPC boundary as JSON byte arrays for Phase 0; a future revision
// can swap to raw IPC channels if the JSON encode/decode shows up in profiles.
//
// Windows WASAPI rejects arbitrary sample rates, so we ask the device for
// its native config and resample in software. Linear interpolation is good
// enough for speech; quality-sensitive paths can move to libsamplerate
// later.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
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
    output_native_rate: u32,
    output_channels: u16,
    playback_queue: Arc<Mutex<Vec<f32>>>,
}

static STATE: Lazy<Mutex<AudioState>> = Lazy::new(|| {
    Mutex::new(AudioState {
        input_stream: None,
        output_stream: None,
        output_native_rate: DOWNLINK_SAMPLE_RATE_HZ,
        output_channels: 1,
        playback_queue: Arc::new(Mutex::new(Vec::with_capacity(96_000))),
    })
});

#[derive(Serialize, Clone)]
struct MicChunkPayload(Vec<u8>);

#[tauri::command]
pub fn start_mic_capture(app: AppHandle) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No input device found".to_string())?;
    let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let supported = device
        .default_input_config()
        .map_err(|error| format!("default_input_config: {error}"))?;
    let sample_format = supported.sample_format();
    let native_rate = supported.sample_rate().0;
    let native_channels = supported.channels();
    let native_config = supported.config();

    eprintln!(
        "[audio] mic device='{device_name}' rate={native_rate} channels={native_channels} format={sample_format:?}",
    );

    let app_for_callback = app.clone();
    let resampler_state = Arc::new(Mutex::new(ResamplerState::new()));

    let stream_result = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &native_config,
            {
                let resampler_state = resampler_state.clone();
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono = downmix_to_mono_f32(samples, native_channels);
                    let resampled = resample_linear_f32(
                        &resampler_state,
                        &mono,
                        native_rate,
                        UPLINK_SAMPLE_RATE_HZ,
                    );
                    let bytes = f32_to_pcm16_bytes(&resampled);
                    if !bytes.is_empty() {
                        let _ = app_for_callback.emit("mic_chunk", MicChunkPayload(bytes));
                    }
                }
            },
            |error| eprintln!("[audio] input stream error: {error}"),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &native_config,
            {
                let resampler_state = resampler_state.clone();
                move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                    let as_f32: Vec<f32> = samples.iter().map(|s| s.to_sample::<f32>()).collect();
                    let mono = downmix_to_mono_f32(&as_f32, native_channels);
                    let resampled = resample_linear_f32(
                        &resampler_state,
                        &mono,
                        native_rate,
                        UPLINK_SAMPLE_RATE_HZ,
                    );
                    let bytes = f32_to_pcm16_bytes(&resampled);
                    if !bytes.is_empty() {
                        let _ = app_for_callback.emit("mic_chunk", MicChunkPayload(bytes));
                    }
                }
            },
            |error| eprintln!("[audio] input stream error: {error}"),
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &native_config,
            {
                let resampler_state = resampler_state.clone();
                move |samples: &[u16], _: &cpal::InputCallbackInfo| {
                    let as_f32: Vec<f32> = samples.iter().map(|s| s.to_sample::<f32>()).collect();
                    let mono = downmix_to_mono_f32(&as_f32, native_channels);
                    let resampled = resample_linear_f32(
                        &resampler_state,
                        &mono,
                        native_rate,
                        UPLINK_SAMPLE_RATE_HZ,
                    );
                    let bytes = f32_to_pcm16_bytes(&resampled);
                    if !bytes.is_empty() {
                        let _ = app_for_callback.emit("mic_chunk", MicChunkPayload(bytes));
                    }
                }
            },
            |error| eprintln!("[audio] input stream error: {error}"),
            None,
        ),
        other => return Err(format!("Unsupported sample format: {other:?}")),
    };

    let stream = stream_result.map_err(|error| format!("build_input_stream: {error}"))?;
    stream.play().map_err(|error| format!("stream.play: {error}"))?;
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
    // pcm is little-endian PCM16 mono @ 24 kHz from Gemini.
    let mut incoming: Vec<f32> = Vec::with_capacity(pcm.len() / 2);
    for chunk in pcm.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        incoming.push(sample as f32 / i16::MAX as f32);
    }

    let (queue, native_rate, channels) = {
        let state = STATE.lock();
        (
            state.playback_queue.clone(),
            state.output_native_rate,
            state.output_channels,
        )
    };

    // Upsample from 24 kHz → device native rate, then duplicate the mono
    // signal across each output channel.
    let upsampled = resample_linear_simple(&incoming, DOWNLINK_SAMPLE_RATE_HZ, native_rate);
    let mut interleaved = Vec::with_capacity(upsampled.len() * channels as usize);
    for sample in upsampled {
        for _ in 0..channels {
            interleaved.push(sample);
        }
    }

    queue.lock().extend(interleaved);
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
        .ok_or_else(|| "No output device found".to_string())?;
    let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let supported = device
        .default_output_config()
        .map_err(|error| format!("default_output_config: {error}"))?;
    let native_config = supported.config();
    let native_rate = supported.sample_rate().0;
    let native_channels = supported.channels();

    eprintln!(
        "[audio] speaker device='{device_name}' rate={native_rate} channels={native_channels}",
    );

    state.output_native_rate = native_rate;
    state.output_channels = native_channels;

    let queue = state.playback_queue.clone();
    let stream = device
        .build_output_stream(
            &native_config,
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut q = queue.lock();
                let take = output.len().min(q.len());
                for (slot, sample) in output.iter_mut().zip(q.drain(..take)) {
                    *slot = sample;
                }
                for slot in output[take..].iter_mut() {
                    *slot = 0.0;
                }
            },
            |error| eprintln!("[audio] output stream error: {error}"),
            None,
        )
        .map_err(|error| format!("build_output_stream: {error}"))?;

    stream.play().map_err(|error| format!("stream.play: {error}"))?;
    state.output_stream = Some(SendableStream(stream));
    Ok(())
}

fn downmix_to_mono_f32(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels_usize = channels as usize;
    let mut out = Vec::with_capacity(samples.len() / channels_usize);
    let mut chunks = samples.chunks_exact(channels_usize);
    for frame in chunks.by_ref() {
        let avg: f32 = frame.iter().sum::<f32>() / channels as f32;
        out.push(avg);
    }
    out
}

// Streaming linear resampler with carry-over of fractional position so
// successive callback buffers don't get re-quantized at chunk boundaries.
struct ResamplerState {
    fractional_position: f32,
    last_sample: f32,
}
impl ResamplerState {
    fn new() -> Self {
        Self {
            fractional_position: 0.0,
            last_sample: 0.0,
        }
    }
}

fn resample_linear_f32(
    state: &Mutex<ResamplerState>,
    input: &[f32],
    input_rate: u32,
    output_rate: u32,
) -> Vec<f32> {
    if input_rate == output_rate {
        return input.to_vec();
    }
    if input.is_empty() {
        return Vec::new();
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
    state_guard.last_sample = *input.last().unwrap();
    out
}

// Stateless version for playback (each Gemini chunk is independent enough
// that boundary artifacts at the seam are inaudible for 24 kHz speech).
fn resample_linear_simple(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if input_rate == output_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = input_rate as f32 / output_rate as f32;
    let output_len = (input.len() as f32 / ratio) as usize;
    let mut out = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let position = i as f32 * ratio;
        let index = position as usize;
        let frac = position - index as f32;
        let current = input[index.min(input.len() - 1)];
        let next = if index + 1 < input.len() {
            input[index + 1]
        } else {
            current
        };
        out.push(current + (next - current) * frac);
    }
    out
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
