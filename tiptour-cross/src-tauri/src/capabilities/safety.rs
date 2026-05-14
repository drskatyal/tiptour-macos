// Deny-list-based safety classifier. The explorer consults this before
// EXECUTING an action; the action is still RECORDED in the graph so the
// planner can offer it later behind a user confirmation prompt.

use super::types::{Action, SafetyClassification};

pub const DESTRUCTIVE_KEYWORDS: &[&str] = &[
    "delete",
    "send",
    "finalize",
    "publish",
    "submit",
    "purge",
    "transfer",
    "buy",
    "sell",
    "drop",
    "remove",
    "format",
    "wipe",
    "shutdown",
    "restart",
    "quit",
    "close",
    "exit",
    "save as",
    "print",
    "export",
];

pub fn is_destructive(action: &Action, element_name: &str) -> bool {
    if element_name_matches_denylist(element_name) {
        return true;
    }

    match action {
        Action::Click { element_name: inner, .. } => element_name_matches_denylist(inner),
        Action::Navigate { menu_path } => menu_path
            .iter()
            .any(|segment| element_name_matches_denylist(segment)),
        Action::Shortcut { .. } => {
            // Shortcuts can fire destructive commands (Cmd+Q, Cmd+W,
            // Ctrl+P) but we can't tell from the chord alone. We classify
            // them as Unknown via classify_action below; this fn returns
            // false because the explorer wants a strict "definitely
            // destructive" signal here.
            false
        }
        Action::Type { .. } | Action::Scroll { .. } => false,
    }
}

pub fn classify_action(action: &Action, element_name: &str) -> SafetyClassification {
    if is_destructive(action, element_name) {
        return SafetyClassification::Destructive;
    }
    match action {
        Action::Shortcut { .. } => SafetyClassification::Unknown,
        _ => SafetyClassification::Safe,
    }
}

fn element_name_matches_denylist(name: &str) -> bool {
    let lowered = name.to_lowercase();
    DESTRUCTIVE_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_delete_button() {
        let action = Action::Click {
            element_name: "Delete account".into(),
            element_path: vec![],
        };
        assert!(is_destructive(&action, "Delete account"));
    }

    #[test]
    fn allows_benign_button() {
        let action = Action::Click {
            element_name: "New tab".into(),
            element_path: vec![],
        };
        assert!(!is_destructive(&action, "New tab"));
    }

    #[test]
    fn flags_destructive_menu_path() {
        let action = Action::Navigate {
            menu_path: vec!["File".into(), "Save As...".into()],
        };
        assert!(is_destructive(&action, ""));
    }

    #[test]
    fn shortcut_alone_is_unknown_not_destructive() {
        let action = Action::Shortcut {
            chord_raw: "Ctrl+P".into(),
        };
        assert!(!is_destructive(&action, ""));
        assert_eq!(classify_action(&action, ""), SafetyClassification::Unknown);
    }
}
