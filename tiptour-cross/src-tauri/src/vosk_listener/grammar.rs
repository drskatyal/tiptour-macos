// Grammar JSON builders for the Vosk constrained-vocabulary recognizers.
//
// Vosk's `Recognizer::new_with_grammar` takes a JSON array of phrases
// the recognizer is allowed to emit. Constraining to a small set both
// massively speeds up decoding and eliminates false positives like
// "tip the tour" being heard as "tiptour".
//
// We always append `"[unk]"` so out-of-vocabulary speech routes to the
// unknown sink instead of being force-snapped onto the closest in-set
// phrase.

#![cfg_attr(not(feature = "vosk"), allow(dead_code))]

/// Phrases accepted in the always-listening wake mode.
pub fn build_wake_grammar() -> String {
    let phrases = vec!["tiptour", "hey tiptour", "[unk]"];
    serde_json::to_string(&phrases).expect("wake grammar should always serialize")
}

/// Phrases accepted in the short-command mode after the wake word fires.
/// Includes the static control verbs plus every saved-flow trigger
/// (canonical name + each alias) wrapped in "do ..." / "run ..." so the
/// caller doesn't have to enumerate both verb forms.
pub fn build_command_grammar(flow_titles: &[String], flow_aliases: &[Vec<String>]) -> String {
    build_command_grammar_with_app_aliases(flow_titles, flow_aliases, &[])
}

/// Variant of `build_command_grammar` that also injects launch phrases
/// ("open <alias>", "launch <alias>", "start <alias>", "go to <alias>")
/// for every discovered-and-enabled app alias the caller passes in.
/// Kept separate so the existing tests covering "do <flow>" / "run <flow>"
/// behavior don't need a discovered-apps fixture.
pub fn build_command_grammar_with_app_aliases(
    flow_titles: &[String],
    flow_aliases: &[Vec<String>],
    installed_app_aliases: &[String],
) -> String {
    let mut phrases: Vec<String> = vec![
        "stop".to_string(),
        "cancel".to_string(),
        "pause".to_string(),
    ];

    // Collect every voice-target string (canonical + aliases) and wrap
    // each with both "do" and "run" verb prefixes.
    let mut voice_target_phrases: Vec<String> = Vec::new();
    for (index, title) in flow_titles.iter().enumerate() {
        let trimmed_title = title.trim();
        if !trimmed_title.is_empty() {
            voice_target_phrases.push(trimmed_title.to_lowercase());
        }
        if let Some(aliases_for_flow) = flow_aliases.get(index) {
            for alias in aliases_for_flow {
                let trimmed_alias = alias.trim();
                if !trimmed_alias.is_empty() {
                    voice_target_phrases.push(trimmed_alias.to_lowercase());
                }
            }
        }
    }

    for target_phrase in &voice_target_phrases {
        phrases.push(format!("do {target_phrase}"));
        phrases.push(format!("run {target_phrase}"));
    }

    // Each discovered + enabled installed app contributes four launch
    // verbs. We accept all four ("open"/"launch"/"start"/"go to") because
    // users phrase the same intent in any of them and Vosk grammar size
    // grows linearly without hurting recognition quality at this scale.
    for installed_app_alias in installed_app_aliases {
        let trimmed = installed_app_alias.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        phrases.push(format!("open {trimmed}"));
        phrases.push(format!("launch {trimmed}"));
        phrases.push(format!("start {trimmed}"));
        phrases.push(format!("go to {trimmed}"));
    }

    phrases.push("[unk]".to_string());

    // Dedup while preserving order — the same alias appearing under two
    // flows would otherwise inflate the grammar without effect.
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<&String> = phrases.iter().filter(|p| seen.insert((*p).clone())).collect();

    serde_json::to_string(&deduped).expect("command grammar should always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_grammar_is_array_with_unk() {
        let grammar = build_wake_grammar();
        assert!(grammar.contains("tiptour"));
        assert!(grammar.contains("[unk]"));
    }

    #[test]
    fn command_grammar_wraps_do_and_run() {
        let titles = vec!["morning routine".to_string()];
        let aliases = vec![vec!["start of day".to_string()]];
        let grammar = build_command_grammar(&titles, &aliases);
        assert!(grammar.contains("do morning routine"));
        assert!(grammar.contains("run morning routine"));
        assert!(grammar.contains("do start of day"));
        assert!(grammar.contains("stop"));
        assert!(grammar.contains("[unk]"));
    }
}
