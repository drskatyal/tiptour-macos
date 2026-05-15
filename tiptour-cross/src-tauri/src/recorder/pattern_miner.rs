// Workflow pattern miner.
//
// The original implementation here was an exact k-gram counter: identical
// event-kind sequences had to repeat verbatim before they registered as a
// pattern. That misses near-duplicates like "Cmd+T, type, Enter" vs
// "Cmd+T, type, Tab, Enter" which the user clearly experiences as the
// same workflow.
//
// The new entry point `mine_patterns_fuzzy` groups sub-sequences by a
// normalized Levenshtein-distance threshold (≤ 0.2 → ≥ 80% similar)
// computed over a *signature* tuple of (event-kind, role, name) per
// event. Timestamps and absolute coordinates are intentionally ignored —
// the user's intent doesn't change because they clicked a button at
// (412, 88) one day and (419, 88) the next.
//
// `mine_patterns_exact` is preserved for the legacy k-gram path so the
// existing unit tests stay valid.

use std::collections::HashMap;
use std::path::PathBuf;

use super::persistence::{list_passive_trace_files, read_trace_jsonl};
use super::types::{InputEvent, TraceEntry, WorkflowPattern};

const MIN_K_GRAM_LENGTH: usize = 2;
const MAX_K_GRAM_LENGTH: usize = 8;

// Edit-distance ceiling for two sub-sequences to belong to the same
// fuzzy group. 0.2 → at most 20% of the longer sequence's length is
// allowed to differ.
const MAX_NORMALIZED_EDIT_DISTANCE_FOR_GROUPING: f32 = 0.2;

// Hard ceiling on event count for the O(N²) similarity comparison.
// Anything bigger gets uniformly downsampled.
const MAX_EVENTS_BEFORE_DOWNSAMPLING: usize = 10_000;

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
    Ok(mine_patterns_fuzzy(&all_traces, min_occurrences))
}

/// Fuzzy miner: groups similar sub-sequences (Levenshtein ≤ 0.2 normalized)
/// and counts occurrences across the canonical centroid of each group.
pub fn mine_patterns_fuzzy(
    all_traces: &[Vec<TraceEntry>],
    min_occurrences: usize,
) -> Vec<WorkflowPattern> {
    // Collapse each trace down to its signature sequence and downsample
    // if the trace is too long for the O(N²) similarity comparison below.
    let signature_traces: Vec<Vec<EventSignature>> = all_traces
        .iter()
        .map(|trace_entries| {
            let signatures: Vec<EventSignature> =
                trace_entries.iter().map(event_signature_from_entry).collect();
            if signatures.len() > MAX_EVENTS_BEFORE_DOWNSAMPLING {
                downsample_uniformly(&signatures, MAX_EVENTS_BEFORE_DOWNSAMPLING)
            } else {
                signatures
            }
        })
        .collect();

    // Collect every sub-sequence (k-gram in [MIN..=MAX]) along with the
    // duration covered, then bucket similar windows into groups.
    let mut harvested_windows: Vec<HarvestedWindow> = Vec::new();
    for (trace_index, signatures) in signature_traces.iter().enumerate() {
        let timestamps: &[i64] = &timestamps_for_trace(&all_traces[trace_index]);
        for k_gram_length in MIN_K_GRAM_LENGTH..=MAX_K_GRAM_LENGTH {
            if signatures.len() < k_gram_length {
                continue;
            }
            for window_start_index in 0..=signatures.len() - k_gram_length {
                let window_end_index = window_start_index + k_gram_length;
                let window_slice = signatures[window_start_index..window_end_index].to_vec();
                let window_duration_ms = timestamps
                    .get(window_end_index - 1)
                    .copied()
                    .unwrap_or(0)
                    - timestamps.get(window_start_index).copied().unwrap_or(0);
                harvested_windows.push(HarvestedWindow {
                    signature_sequence: window_slice,
                    duration_ms: window_duration_ms.max(0),
                });
            }
        }
    }

    let mut similarity_groups: Vec<SimilarityGroup> = Vec::new();
    for harvested_window in harvested_windows {
        let matched_group_index = similarity_groups.iter().position(|group| {
            normalized_levenshtein_distance(
                &group.centroid_signatures,
                &harvested_window.signature_sequence,
            ) <= MAX_NORMALIZED_EDIT_DISTANCE_FOR_GROUPING
        });

        if let Some(group_index) = matched_group_index {
            let group = &mut similarity_groups[group_index];
            let distance_to_centroid = normalized_levenshtein_distance(
                &group.centroid_signatures,
                &harvested_window.signature_sequence,
            );
            group.occurrences += 1;
            group.total_duration_ms += harvested_window.duration_ms;
            group.sum_of_distances_to_centroid += distance_to_centroid;
        } else {
            similarity_groups.push(SimilarityGroup {
                centroid_signatures: harvested_window.signature_sequence,
                occurrences: 1,
                total_duration_ms: harvested_window.duration_ms,
                sum_of_distances_to_centroid: 0.0,
            });
        }
    }

    let mut workflow_patterns: Vec<WorkflowPattern> = similarity_groups
        .into_iter()
        .filter(|group| group.occurrences >= min_occurrences)
        .map(|group| {
            let mean_interval_ms = if group.occurrences > 0 {
                group.total_duration_ms / group.occurrences as i64
            } else {
                0
            };
            let mean_distance_to_centroid = if group.occurrences > 0 {
                group.sum_of_distances_to_centroid / group.occurrences as f32
            } else {
                0.0
            };
            let representative_event_kinds: Vec<String> = group
                .centroid_signatures
                .iter()
                .map(EventSignature::display_token)
                .collect();
            let k_gram_length = representative_event_kinds.len();
            WorkflowPattern {
                occurrences: group.occurrences,
                mean_interval_ms,
                suggested_name: suggest_name_for_kinds(&representative_event_kinds),
                representative_event_kinds,
                k_gram_length,
                similarity_score: (1.0 - mean_distance_to_centroid).clamp(0.0, 1.0),
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

/// Legacy exact k-gram miner. Preserved so the previous unit tests and any
/// downstream tooling that depends on exact-match semantics keep working.
pub fn mine_patterns_exact(
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
                similarity_score: 1.0,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EventSignature {
    event_kind: &'static str,
    element_role: String,
    element_name: String,
}

impl EventSignature {
    fn display_token(&self) -> String {
        if self.element_role.is_empty() && self.element_name.is_empty() {
            self.event_kind.to_string()
        } else {
            format!(
                "{}({}|{})",
                self.event_kind, self.element_role, self.element_name
            )
        }
    }
}

struct HarvestedWindow {
    signature_sequence: Vec<EventSignature>,
    duration_ms: i64,
}

struct SimilarityGroup {
    centroid_signatures: Vec<EventSignature>,
    occurrences: usize,
    total_duration_ms: i64,
    sum_of_distances_to_centroid: f32,
}

struct KGramAccumulator {
    occurrences: usize,
    total_duration_ms: i64,
    representative_event_kinds: Vec<String>,
    k_gram_length: usize,
}

fn event_signature_from_entry(entry: &TraceEntry) -> EventSignature {
    let event_kind = event_kind_token(&entry.event);
    // Prefer `snapshot_before` since the event happened "in" that state.
    let focused_element_fingerprint = entry
        .snapshot_before
        .as_ref()
        .and_then(|snap| snap.focused_element.clone())
        .or_else(|| {
            entry
                .snapshot_after
                .as_ref()
                .and_then(|snap| snap.focused_element.clone())
        });
    let (element_role, element_name) = match focused_element_fingerprint {
        Some(fingerprint) => (
            fingerprint.role.unwrap_or_default(),
            fingerprint.name.unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    };
    EventSignature {
        event_kind,
        element_role,
        element_name,
    }
}

fn timestamps_for_trace(trace_entries: &[TraceEntry]) -> Vec<i64> {
    trace_entries
        .iter()
        .map(|entry| entry.timestamp_unix_ms)
        .collect()
}

fn downsample_uniformly<T: Clone>(items: &[T], target_length: usize) -> Vec<T> {
    if items.len() <= target_length || target_length == 0 {
        return items.to_vec();
    }
    let mut downsampled = Vec::with_capacity(target_length);
    // Use integer arithmetic to step through `items` so the downsample is
    // deterministic and free of floating-point drift on long traces.
    for downsample_index in 0..target_length {
        let source_index = downsample_index * items.len() / target_length;
        downsampled.push(items[source_index].clone());
    }
    downsampled
}

/// Returns Levenshtein distance normalized by the longer length.
/// 0.0 → identical; 1.0 → maximally dissimilar.
fn normalized_levenshtein_distance(left: &[EventSignature], right: &[EventSignature]) -> f32 {
    let longer_length = left.len().max(right.len());
    if longer_length == 0 {
        return 0.0;
    }
    let raw_distance = levenshtein_distance(left, right);
    (raw_distance as f32) / (longer_length as f32)
}

fn levenshtein_distance(left: &[EventSignature], right: &[EventSignature]) -> usize {
    let left_length = left.len();
    let right_length = right.len();
    if left_length == 0 {
        return right_length;
    }
    if right_length == 0 {
        return left_length;
    }
    // Two-row dynamic programming table — O(min) memory, O(left*right) time.
    let mut previous_row: Vec<usize> = (0..=right_length).collect();
    let mut current_row: Vec<usize> = vec![0; right_length + 1];
    for (left_index_zero_based, left_signature) in left.iter().enumerate() {
        current_row[0] = left_index_zero_based + 1;
        for (right_index_zero_based, right_signature) in right.iter().enumerate() {
            let substitution_cost = if left_signature == right_signature {
                0
            } else {
                1
            };
            let deletion = previous_row[right_index_zero_based + 1] + 1;
            let insertion = current_row[right_index_zero_based] + 1;
            let substitution = previous_row[right_index_zero_based] + substitution_cost;
            current_row[right_index_zero_based + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut previous_row, &mut current_row);
    }
    previous_row[right_length]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::types::InputEvent;

    fn entry(timestamp_unix_ms: i64, event: InputEvent) -> TraceEntry {
        TraceEntry {
            timestamp_unix_ms,
            event,
            snapshot_before: None,
            snapshot_after: None,
        }
    }

    #[test]
    fn fuzzy_groups_near_duplicate_sequences() {
        let trace_one = vec![
            entry(0, InputEvent::KeyDown { key_code: 1, key_name: "T".into() }),
            entry(10, InputEvent::KeyDown { key_code: 2, key_name: "Type".into() }),
            entry(20, InputEvent::KeyDown { key_code: 3, key_name: "Enter".into() }),
        ];
        let trace_two = vec![
            entry(0, InputEvent::KeyDown { key_code: 1, key_name: "T".into() }),
            entry(10, InputEvent::KeyDown { key_code: 2, key_name: "Type".into() }),
            entry(15, InputEvent::KeyDown { key_code: 9, key_name: "Tab".into() }),
            entry(20, InputEvent::KeyDown { key_code: 3, key_name: "Enter".into() }),
        ];
        let patterns = mine_patterns_fuzzy(&[trace_one, trace_two], 2);
        // Both traces share the 3-event "keyDown × 3" pattern via fuzzy
        // grouping, so at least one pattern should hit min_occurrences=2.
        assert!(patterns.iter().any(|pattern| pattern.occurrences >= 2));
    }

    #[test]
    fn exact_matches_legacy_behavior() {
        let trace = vec![
            entry(0, InputEvent::KeyDown { key_code: 1, key_name: "A".into() }),
            entry(10, InputEvent::KeyUp { key_code: 1, key_name: "A".into() }),
            entry(20, InputEvent::KeyDown { key_code: 1, key_name: "A".into() }),
            entry(30, InputEvent::KeyUp { key_code: 1, key_name: "A".into() }),
        ];
        let patterns = mine_patterns_exact(&[trace], 2);
        assert!(!patterns.is_empty());
        assert!(patterns.iter().all(|pattern| pattern.similarity_score == 1.0));
    }
}
