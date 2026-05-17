// Alias generation. Each discovered app gets a short list of pronounceable
// phrases the Vosk grammar can inject under "open <alias>" etc. We bias
// toward the brand-stripped short form ("Google Chrome" → "chrome") because
// that's how users actually speak the command, while still keeping the
// full display name as a safety net.

use std::collections::HashSet;

/// Brand prefixes we strip when generating the short alias. Order matters:
/// longer multi-word brands first so "Microsoft Visual Studio" hits
/// "microsoft visual studio" before the single-word "microsoft" prefix.
const BRAND_PREFIXES_TO_STRIP: &[&str] = &[
    "microsoft visual studio",
    "microsoft",
    "google",
    "apple",
    "adobe",
    "jetbrains",
    "amazon",
    "logitech",
];

/// Common explicit shortenings users say but the display name doesn't
/// contain. Keyed on lowercased display name fragments.
const SHORTHAND_OVERRIDES: &[(&str, &[&str])] = &[
    ("visual studio code", &["vs code", "code"]),
    ("visual studio", &["vs code", "code"]),
    ("google chrome", &["chrome"]),
    ("microsoft edge", &["edge"]),
    ("microsoft word", &["word"]),
    ("microsoft excel", &["excel"]),
    ("microsoft powerpoint", &["powerpoint", "ppt"]),
    ("microsoft outlook", &["outlook"]),
    ("microsoft teams", &["teams"]),
    ("whatsapp", &["whatsapp", "whats app"]),
    ("slack", &["slack"]),
    ("notion", &["notion"]),
    ("spotify", &["spotify"]),
    ("cursor", &["cursor"]),
    ("figma", &["figma"]),
];

pub fn generate_aliases(display_name: &str) -> Vec<String> {
    let lowercased = display_name.trim().to_lowercase();
    if lowercased.is_empty() {
        return Vec::new();
    }

    let mut collected: Vec<String> = Vec::new();
    collected.push(lowercased.clone());

    for prefix in BRAND_PREFIXES_TO_STRIP {
        if lowercased.starts_with(prefix) {
            let remainder = lowercased[prefix.len()..].trim().to_string();
            if !remainder.is_empty() {
                collected.push(remainder);
            }
        }
    }

    for (needle, replacements) in SHORTHAND_OVERRIDES {
        if lowercased.contains(needle) {
            for replacement in *replacements {
                collected.push((*replacement).to_string());
            }
        }
    }

    // Strip trailing ".app" or " desktop" suffixes that survive in some
    // display names (Slack.app, Figma Desktop).
    let mut additional: Vec<String> = Vec::new();
    for alias in &collected {
        if let Some(stripped) = alias.strip_suffix(" desktop") {
            additional.push(stripped.to_string());
        }
        if let Some(stripped) = alias.strip_suffix(".app") {
            additional.push(stripped.to_string());
        }
    }
    collected.extend(additional);

    // Dedup while preserving order. Empty strings and pure-symbol aliases
    // would just inflate the Vosk grammar without ever matching speech.
    let mut seen: HashSet<String> = HashSet::new();
    collected
        .into_iter()
        .filter_map(|alias| {
            let trimmed = alias.trim().to_string();
            if trimmed.is_empty() {
                return None;
            }
            if !trimmed.chars().any(|character| character.is_alphabetic()) {
                return None;
            }
            if seen.insert(trimmed.clone()) {
                Some(trimmed)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_brand_prefix() {
        let aliases = generate_aliases("Google Chrome");
        assert!(aliases.contains(&"google chrome".to_string()));
        assert!(aliases.contains(&"chrome".to_string()));
    }

    #[test]
    fn applies_shorthand_for_vs_code() {
        let aliases = generate_aliases("Visual Studio Code");
        assert!(aliases.contains(&"vs code".to_string()));
        assert!(aliases.contains(&"code".to_string()));
    }
}
