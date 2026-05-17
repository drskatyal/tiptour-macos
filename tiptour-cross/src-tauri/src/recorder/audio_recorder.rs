// Narration audio recorder for demonstration mode. Writes 16 kHz mono PCM16
// to a WAV file using `hound`. We deliberately do NOT seize the mic from the
// main audio module — if the main Gemini-Live mic capture is already running,
// this module buffers incoming `mic_chunk` payloads pushed in from TS and
// writes them to disk. That keeps the recorder additive instead of conflict-
// prone with the realtime voice path.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hound::{SampleFormat, WavSpec, WavWriter};

const NARRATION_SAMPLE_RATE_HZ: u32 = 16_000;
const NARRATION_CHANNEL_COUNT: u16 = 1;
const NARRATION_BITS_PER_SAMPLE: u16 = 16;

pub struct AudioRecorder {
    output_file_path: PathBuf,
    wav_writer: Mutex<Option<WavWriter<BufWriter<File>>>>,
}

impl AudioRecorder {
    pub fn create(output_file_path: PathBuf) -> Result<Self, String> {
        if let Some(parent_directory) = output_file_path.parent() {
            std::fs::create_dir_all(parent_directory).map_err(|error| error.to_string())?;
        }
        let wav_specification = WavSpec {
            channels: NARRATION_CHANNEL_COUNT,
            sample_rate: NARRATION_SAMPLE_RATE_HZ,
            bits_per_sample: NARRATION_BITS_PER_SAMPLE,
            sample_format: SampleFormat::Int,
        };
        let wav_writer = WavWriter::create(&output_file_path, wav_specification)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            output_file_path,
            wav_writer: Mutex::new(Some(wav_writer)),
        })
    }

    pub fn output_file_path(&self) -> &Path {
        &self.output_file_path
    }

    pub fn append_pcm16_chunk(&self, pcm16_little_endian_bytes: &[u8]) -> Result<(), String> {
        let mut writer_guard = self.wav_writer.lock().map_err(|error| error.to_string())?;
        let writer = match writer_guard.as_mut() {
            Some(writer) => writer,
            None => return Ok(()),
        };
        let mut byte_iterator = pcm16_little_endian_bytes.chunks_exact(2);
        for two_byte_chunk in byte_iterator.by_ref() {
            let sample_value = i16::from_le_bytes([two_byte_chunk[0], two_byte_chunk[1]]);
            writer
                .write_sample(sample_value)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn finalize(&self) -> Result<(), String> {
        let mut writer_guard = self.wav_writer.lock().map_err(|error| error.to_string())?;
        if let Some(writer) = writer_guard.take() {
            writer.finalize().map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
