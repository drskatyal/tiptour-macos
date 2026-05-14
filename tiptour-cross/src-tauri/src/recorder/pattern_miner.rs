// Simple k-gram frequency miner over passive traces. For every k in [2, 8]
// we slide a window across each trace, hash the event-kind sequence, and
// count how often each window appears across the corpus. Patterns whose
// occurrence count clears `min_occurrences` are returned, sorted by count.
//
// TODO: replace the exact-string k-gram with sequence-alignment that allows
// small edit distance, so "Cmd+T, type, Enter" and "Cmd+T, type, Tab, Enter"
// fold into one pattern. The simplest workable version ships first.

use std::collections::HashMap;
use std::path::PathBuf;

use super::persistence::{list_passive_trace_files, read_trace_jsonl};
use super::types::{InputEvent, TraceEntry, WorkflowPattern};

const MIN_K_GRAM_LENGTH: usize = 2;
const MAX_K_GRAM_LENGTH: usize = 8;

pub fn mine_patterns_from_all_passive_traces(
    min_occurrences: usize,
) -> Result<Vec<WorkflowPattern>, String> {
    let trace_file_paths = list_passive_trace_files()?;
    let mut all_traces: Vec<Vec<TraceEntry>> = Vec::new();
    for trace_file_path in trace_file_paths {
        let trace_entries = read_trace_jsonl(&trace_file_path).unwrap_or_default();
        if !trace_entries.is_empty() {
            all_traces.push(trace_entries);
        }
    }
    Ok(mine_patterns(&all_traces, min_occurrences))
}

pub fn mine_patterns(
    all_traces: &[Vec<TraceEntry>],
    min_occurrences: usize,
) -> Vec<WorkflowPattern> {
    let mut k_gram_occurrence_counts: HashMap<String, KGramAccumulator> = HashMap::new();

    for single_trace in all_traces {
        let event_kind_sequence: Vec<&str> = single_trace
            .iter()
            .map(|entry| event_kind_token(&entry.event))
            .collect();
        let timestamp_sequence: Vec<i64> = single_trace
            .iter()
            .map(|entry| entry.timestamp_unix_ms)
            .collect();

        for k_gram_length in MIN_K_GRAM_LENGTH..=MAX_K_GRAM_LENGTH {
            if event_kind_sequence.len() < k_gram_length {
                continue;
            }
            for window_start_index in 0..=event_kind_sequence.len() - k_gram_length {
                let window_end_index = window_start_index + k_gram_length;
                let window_slice = &event_kind_sequence[window_start_index..window_end_index];
                let window_key = window_slice.join("|");
                let window_duration_ms = timestamp_sequence[window_end_index - 1]
                    - timestamp_sequence[window_start_index];

                let accumulator = k_gram_occurrence_counts
                    .entry(window_key)
                    .or_insert_with(|| KGramAccumulator {
                        occurrences: 0,
                        total_duration_ms: 0,
                        representative_event_kinds: window_slice
                            .iter()
                            .map(|kind| (*kind).to_string())
                            .collect(),
                        k_gram_length,
                    });
                accumulator.occurrences += 1;
                accumulator.total_duration_ms += window_duration_ms.max(0);
            }
        }
    }

    let mut workflow_patterns: Vec<WorkflowPattern> = k_gram_occurrence_counts
        .into_iter()
        .filter(|(_, accumulator)| accumulator.occurrences >= min_occurrences)
        .map(|(_, accumulator)| {
            let mean_interval_ms = if accumulator.occurrences > 0 {
                accumulator.total_duration_ms / accumulator.occurrences as i64
            } else {
                0
            };
            WorkflowPattern {
                occurrences: accumulator.occurrences,
                mean_interval_ms,
                suggested_name: suggest_name_for_kinds(&accumulator.representative_event_kinds),
                representative_event_kinds: accumulator.representative_event_kinds,
                k_gram_length: accumulator.k_gram_length,
            }
        })
        .collect();

    workflow_patterns.sort_by(|left, right| {
        right
            .occurrences
            .cmp(&left.occurrences)
            .then(right.k_gram_length.cmp(&left.k_gram_length))
    });
    workflow_patterns
}

struct KGramAccumulator {
    occurrences: usize,
    total_duration_ms: i64,
    representative_event_kinds: Vec<String>,
    k_gram_length: usize,
}

fn event_kind_token(event: &InputEvent) -> &'static str {
    match event {
        InputEvent::KeyDown { .. } => "keyDown",
        InputEvent::KeyUp { .. } => "keyUp",
        InputEvent::MouseClick { .. } => "mouseClick",
        InputEvent::MouseMove { .. } => "mouseMove",
        InputEvent::Scroll { .. } => "scroll",
    }
}

fn suggest_name_for_kinds(event_kinds: &[String]) -> String {
    format!("Pattern: {}", event_kinds.join(" → "))
}

pub fn passive_trace_file_path_for_session(session_start_unix_ms: i64) -> Option<PathBuf> {
    let mut path = super::persistence::passive_traces_directory()?;
    path.push(format!("{session_start_unix_ms}.jsonl"));
    Some(path)
}
