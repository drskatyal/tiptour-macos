// Thin wrapper over `vosk::Recognizer`. Compiled only when the `vosk`
// feature is enabled — the upstream crate dynamically links `libvosk`,
// which isn't present in the Linux dev sandbox.

#![cfg(feature = "vosk")]

use std::path::Path;

use vosk::{DecodingState, Model, Recognizer};

#[derive(Debug, Clone)]
pub enum RecognitionEvent {
    Partial { text: String },
    Final { text: String },
}

pub struct LocalRecognizer {
    // The model is owned by the recognizer; we keep it alive for the
    // lifetime of the recognizer so dropping the recognizer also drops
    // the model handle.
    _model: Model,
    recognizer: Recognizer,
}

impl LocalRecognizer {
    pub fn new(
        model_path: &Path,
        allowed_grammar: Option<&[&str]>,
        sample_rate: f32,
    ) -> Result<Self, String> {
        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| "model path must be valid UTF-8".to_string())?;
        let model = Model::new(model_path_str)
            .ok_or_else(|| format!("vosk failed to load model at {model_path_str}"))?;

        // Constraining the recognizer to a known phrase set makes
        // decoding dramatically faster and removes false positives. We
        // build a fresh recognizer per grammar instead of trying to
        // re-set vocab on a live one — Vosk's safe surface area treats
        // these as separate objects.
        let recognizer = match allowed_grammar {
            Some(phrases) => Recognizer::new_with_grammar(&model, sample_rate, phrases)
                .ok_or_else(|| "Recognizer::new_with_grammar returned None".to_string())?,
            None => Recognizer::new(&model, sample_rate)
                .ok_or_else(|| "Recognizer::new returned None".to_string())?,
        };

        Ok(Self {
            _model: model,
            recognizer,
        })
    }

    /// Feed one chunk of PCM16 samples. Returns Some(event) when the
    /// recognizer has either a partial hypothesis worth showing or a
    /// finalized utterance — None otherwise.
    pub fn accept_pcm16(&mut self, samples: &[i16]) -> Option<RecognitionEvent> {
        // `accept_waveform` returns Result<DecodingState, _> in current
        // vosk; treat any error as "no event this chunk" so the caller
        // can keep feeding audio instead of getting stuck.
        let decoding_state = match self.recognizer.accept_waveform(samples) {
            Ok(state) => state,
            Err(_) => return None,
        };
        match decoding_state {
            DecodingState::Finalized => {
                let final_result = self.recognizer.result().single().map(|r| r.text.to_string());
                final_result.map(|text| RecognitionEvent::Final { text })
            }
            DecodingState::Running => {
                let partial_text = self.recognizer.partial_result().partial.to_string();
                if partial_text.trim().is_empty() {
                    None
                } else {
                    Some(RecognitionEvent::Partial { text: partial_text })
                }
            }
            DecodingState::Failed => None,
        }
    }

    pub fn reset(&mut self) {
        self.recognizer.reset();
    }
}
