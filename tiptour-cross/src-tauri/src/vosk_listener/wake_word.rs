// Two-state wake-word + short-command dispatcher.
//
// Default mode `Listening` runs a wake-word-only Vosk recognizer with a
// tiny grammar so out-of-band speech can't false-trigger TipTour. On a
// match we flip to `Awake`, swap in a short-command recognizer built
// from the current saved-flow titles + static control verbs, and wait
// up to `AWAKE_TIMEOUT_MS` for a Final result before snapping back to
// `Listening`.
//
// We rebuild the command grammar every time we enter `Awake` instead of
// caching it because the saved-flow list can change between wakes
// (user records a new flow). Building the recognizer is cheap relative
// to the user's pause between wake word and command.

#![cfg(feature = "vosk")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::app_discovery;
use crate::multiflow;

use super::grammar::{build_command_grammar_with_app_aliases, build_wake_grammar};
use super::recognizer::{LocalRecognizer, RecognitionEvent};

const SAMPLE_RATE_HZ: f32 = 16_000.0;
const AWAKE_TIMEOUT_MS: u64 = 4_000;

/// High-level state of the wake-word dispatcher loop.
enum DispatcherMode {
    Listening,
    Awake { entered_at: Instant },
}

pub struct WakeWordDispatcher {
    app_handle: AppHandle,
    model_path: PathBuf,
    mode: DispatcherMode,
    active_recognizer: LocalRecognizer,
}

impl WakeWordDispatcher {
    pub fn new(app_handle: AppHandle, model_path: PathBuf) -> Result<Self, String> {
        let wake_grammar_json = build_wake_grammar();
        // Vosk wants the grammar as `&[&str]` — we parse our own JSON
        // back out to keep the grammar builder a single source of truth.
        let wake_phrases: Vec<String> =
            serde_json::from_str(&wake_grammar_json).map_err(|error| error.to_string())?;
        let wake_phrase_refs: Vec<&str> = wake_phrases.iter().map(|s| s.as_str()).collect();
        let active_recognizer =
            LocalRecognizer::new(&model_path, Some(&wake_phrase_refs), SAMPLE_RATE_HZ)?;
        Ok(Self {
            app_handle,
            model_path,
            mode: DispatcherMode::Listening,
            active_recognizer,
        })
    }

    /// Feed one PCM16 chunk. Drives mode transitions and emits Tauri
    /// events as recognition results come in.
    pub fn accept_pcm16(&mut self, samples: &[i16]) {
        let event = self.active_recognizer.accept_pcm16(samples);

        if let Some(RecognitionEvent::Partial { text }) = &event {
            let _ = self
                .app_handle
                .emit("vosk_partial_transcript", text.clone());
        }

        match &self.mode {
            DispatcherMode::Listening => {
                if let Some(RecognitionEvent::Final { text }) = event {
                    if text_contains_wake_word(&text) {
                        let _ = self.app_handle.emit("vosk_wake_detected", text.clone());
                        // Also fire the same hotkey event so a hands-free
                        // user gets a Gemini session immediately if they
                        // don't follow up with a short command.
                        let _ = self.app_handle.emit("push_to_talk_toggled", ());
                        self.enter_awake_mode();
                    }
                }
            }
            DispatcherMode::Awake { entered_at } => {
                // Time out back to Listening if the user didn't follow
                // through with a short command — protects against the
                // recognizer sitting in command mode forever after a
                // stray wake-word hit.
                if entered_at.elapsed() > Duration::from_millis(AWAKE_TIMEOUT_MS) {
                    self.return_to_listening();
                    return;
                }
                if let Some(RecognitionEvent::Final { text }) = event {
                    self.handle_command_phrase(&text);
                    self.return_to_listening();
                }
            }
        }
    }

    fn enter_awake_mode(&mut self) {
        let flow_entries = multiflow::list_flows().unwrap_or_default();
        let flow_titles: Vec<String> = flow_entries.iter().map(|f| f.name.clone()).collect();
        let flow_aliases: Vec<Vec<String>> = flow_entries
            .iter()
            .map(|f| f.trigger_aliases.clone())
            .collect();
        // Pull the user-enabled discovered apps so the grammar can fire
        // launch phrases entirely locally — no Gemini round-trip needed
        // for "open chrome" / "launch slack" / etc.
        let mut installed_app_aliases: Vec<String> = Vec::new();
        for discovered_app in app_discovery::enabled_apps_for_grammar() {
            installed_app_aliases.extend(discovered_app.aliases);
        }
        let command_grammar_json = build_command_grammar_with_app_aliases(
            &flow_titles,
            &flow_aliases,
            &installed_app_aliases,
        );
        let Ok(command_phrases) = serde_json::from_str::<Vec<String>>(&command_grammar_json) else {
            return;
        };
        let command_phrase_refs: Vec<&str> = command_phrases.iter().map(|s| s.as_str()).collect();
        match LocalRecognizer::new(&self.model_path, Some(&command_phrase_refs), SAMPLE_RATE_HZ) {
            Ok(recognizer) => {
                self.active_recognizer = recognizer;
                self.mode = DispatcherMode::Awake {
                    entered_at: Instant::now(),
                };
            }
            Err(error) => {
                eprintln!("[vosk] failed to build command recognizer: {error}");
            }
        }
    }

    fn return_to_listening(&mut self) {
        let wake_grammar_json = build_wake_grammar();
        let Ok(wake_phrases) = serde_json::from_str::<Vec<String>>(&wake_grammar_json) else {
            return;
        };
        let wake_phrase_refs: Vec<&str> = wake_phrases.iter().map(|s| s.as_str()).collect();
        match LocalRecognizer::new(&self.model_path, Some(&wake_phrase_refs), SAMPLE_RATE_HZ) {
            Ok(recognizer) => {
                self.active_recognizer = recognizer;
                self.mode = DispatcherMode::Listening;
            }
            Err(error) => {
                eprintln!("[vosk] failed to rebuild wake recognizer: {error}");
            }
        }
    }

    fn handle_command_phrase(&self, raw_phrase: &str) {
        let normalized = raw_phrase.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized == "[unk]" {
            return;
        }

        // Surface every grammar match as a Voice Command indicator. The
        // recognized phrase comes from the static command grammar the
        // user authored (or the discovered-apps alias set), not from
        // raw open-ended dictation, so it's safe to put in the pill.
        crate::indicators::emit(
            &self.app_handle,
            crate::indicators::IndicatorKind::VoiceCommand,
            "Voice command".to_string(),
            Some(normalized.clone()),
            None,
        );

        // Static control verbs first. Match by `starts_with` to tolerate
        // recognizer adding stray trailing words inside the grammar set.
        if normalized == "stop" || normalized == "cancel" {
            let _ = self.app_handle.emit("vosk_command_stop", ());
            return;
        }
        if normalized == "pause" {
            let _ = self.app_handle.emit("vosk_command_pause", ());
            return;
        }

        // "open X" / "launch X" / "start X" / "go to X" → look up the
        // alias in the discovered-apps cache and dispatch a local launch
        // via the same code path the workflow runner uses for
        // `ExecutableAction::LaunchApp`. Stays entirely on-device — the
        // Gemini Live session is never opened for this branch.
        let launch_query_opt = normalized
            .strip_prefix("open ")
            .or_else(|| normalized.strip_prefix("launch "))
            .or_else(|| normalized.strip_prefix("start "))
            .or_else(|| normalized.strip_prefix("go to "));
        if let Some(launch_query) = launch_query_opt {
            let trimmed_launch_query = launch_query.trim().to_string();
            if !trimmed_launch_query.is_empty() {
                if let Some(matched_app) =
                    app_discovery::find_enabled_app_by_alias(&trimmed_launch_query)
                {
                    let launch_identifier = matched_app.launch_identifier().to_string();
                    let display_name_for_event = matched_app.display_name.clone();
                    let app_handle_for_event = self.app_handle.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(launch_error) =
                            crate::executor::workflow_runner::launch_app_public(&launch_identifier)
                        {
                            eprintln!(
                                "[vosk] local launch '{display_name_for_event}' failed: {launch_error}"
                            );
                        }
                        let _ = app_handle_for_event
                            .emit("vosk_local_launch_fired", display_name_for_event);
                    });
                    return;
                }
                // Alias didn't match anything in the cache — fall through
                // to the saved-flow fuzzy match below in case the user
                // recorded a flow whose trigger phrase starts with "open"
                // (e.g. "open daily standup notes").
            }
        }

        // "do X" / "run X" → fuzzy-match against saved flows.
        let flow_query_opt = normalized
            .strip_prefix("do ")
            .or_else(|| normalized.strip_prefix("run "));
        if let Some(flow_query) = flow_query_opt {
            let flow_query_owned = flow_query.trim().to_string();
            if flow_query_owned.is_empty() {
                return;
            }
            let app_handle_for_task = self.app_handle.clone();
            tauri::async_runtime::spawn(async move {
                // `find_flow_by_voice_query` does the same fuzzy-match
                // the Gemini tool-call path uses, so a successful hit
                // here behaves identically to a model-driven recall.
                let matched =
                    multiflow::find_flow_by_voice_query(flow_query_owned.clone()).await;
                match matched {
                    Ok(Some(flow)) => {
                        let _ = multiflow::run_flow_by_name(
                            flow.name.clone(),
                            app_handle_for_task.clone(),
                        )
                        .await;
                    }
                    _ => {
                        // No saved flow matched — fall back to opening a
                        // Gemini Live session so the user's query still
                        // reaches the model.
                        let _ = app_handle_for_task.emit("push_to_talk_toggled", ());
                    }
                }
            });
        }
    }
}

fn text_contains_wake_word(recognized_text: &str) -> bool {
    let normalized = recognized_text.to_ascii_lowercase();
    normalized.contains("tiptour") || normalized.contains("hey tiptour")
}
