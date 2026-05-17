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

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(name: &str, aliases: &[&str]) -> FlowSummary {
        FlowSummary {
            flow_id: format!("id-{name}"),
            name: name.to_string(),
            created_at_unix_ms: 0,
            step_count: 1,
            trigger_aliases: aliases.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn exact_match_returns_confident_score() {
        let flows = vec![flow("morning routine", &[])];
        let (entry, score) = best_match_for_query("morning routine", &flows).expect("match");
        assert_eq!(entry.name, "morning routine");
        assert!(score >= MATCH_CONFIDENT_THRESHOLD);
    }

    #[test]
    fn normalized_punctuation_still_matches() {
        let flows = vec![flow("Slack + Notion start", &[])];
        let (entry, score) =
            best_match_for_query("Slack, Notion start!!", &flows).expect("match despite punct");
        assert_eq!(entry.name, "Slack + Notion start");
        assert!(score >= MATCH_CONFIDENT_THRESHOLD);
    }

    #[test]
    fn alias_match_beats_canonical_name() {
        // Canonical name shares no useful tokens with the query, but an
        // alias does. The alias path should win.
        let flows = vec![flow("Slack + Notion start", &["morning routine"])];
        let (entry, _score) = best_match_for_query(
            "do my morning routine please",
            &flows,
        )
        .expect("alias match");
        assert_eq!(entry.name, "Slack + Notion start");
    }

    #[test]
    fn rejects_query_below_threshold() {
        let flows = vec![flow("morning routine", &[])];
        // A totally unrelated query should fall under MATCH_ACCEPT_THRESHOLD.
        let result = best_match_for_query("compile the kernel", &flows);
        assert!(result.is_none());
    }

    #[test]
    fn picks_highest_scoring_among_multiple() {
        let flows = vec![
            flow("morning routine", &[]),
            flow("evening shutdown", &[]),
        ];
        let (entry, _score) =
            best_match_for_query("evening shutdown", &flows).expect("match");
        assert_eq!(entry.name, "evening shutdown");
    }
}
