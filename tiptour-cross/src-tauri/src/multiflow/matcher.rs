// Fuzzy voice→flow name matcher. Gemini's speech transcripts come in
// with filler ("do my morning routine please") and case/punctuation noise;
// we normalize aggressively then take the best normalized-Levenshtein
// score across the flow name and any of its user-defined trigger aliases.
//
// Threshold 0.65 is the floor — below that we return None rather than
// risk running the wrong flow on a misheard phrase. 0.85+ is the
// "confident match" band the caller can act on without confirmation.

use strsim::normalized_levenshtein;

use super::types::FlowSummary;

const MATCH_ACCEPT_THRESHOLD: f64 = 0.65;
pub const MATCH_CONFIDENT_THRESHOLD: f64 = 0.85;

const FILLER_WORDS: &[&str] = &[
    "do", "my", "run", "please", "now", "the", "a", "an", "kick", "off",
    "start", "begin", "execute", "play", "go", "for", "me", "can", "you",
    "would", "could",
];

/// Normalize a user-spoken phrase or a stored flow name down to a
/// whitespace-separated, lowercased, punctuation-free string with common
/// filler words removed. Two phrases that mean the same thing in casual
/// English should normalize to similar tokens.
fn normalize_phrase_for_matching(raw_phrase: &str) -> String {
    let lowered = raw_phrase.to_ascii_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let kept_tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|token| !FILLER_WORDS.contains(token))
        .collect();
    kept_tokens.join(" ")
}

fn similarity_between(query_phrase: &str, candidate_phrase: &str) -> f64 {
    let normalized_query = normalize_phrase_for_matching(query_phrase);
    let normalized_candidate = normalize_phrase_for_matching(candidate_phrase);
    if normalized_query.is_empty() || normalized_candidate.is_empty() {
        return 0.0;
    }
    normalized_levenshtein(&normalized_query, &normalized_candidate)
}

/// Returns the best-matching flow summary along with its similarity
/// score, or None when nothing clears the accept threshold.
pub fn best_match_for_query<'flows>(
    voice_query: &str,
    flow_entries: &'flows [FlowSummary],
) -> Option<(&'flows FlowSummary, f64)> {
    let mut best_match: Option<(&FlowSummary, f64)> = None;
    for entry in flow_entries {
        // Score against the canonical name and each trigger alias; the
        // user might have registered "morning routine" as an alias for a
        // flow they originally named "Slack + Notion start".
        let mut best_score_for_entry = similarity_between(voice_query, &entry.name);
        for alias in &entry.trigger_aliases {
            let alias_score = similarity_between(voice_query, alias);
            if alias_score > best_score_for_entry {
                best_score_for_entry = alias_score;
            }
        }
        if best_score_for_entry >= MATCH_ACCEPT_THRESHOLD {
            match best_match {
                Some((_, current_best_score)) if current_best_score >= best_score_for_entry => {}
                _ => best_match = Some((entry, best_score_for_entry)),
            }
        }
    }
    best_match
}
