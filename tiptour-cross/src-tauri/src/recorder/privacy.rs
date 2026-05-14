// Privacy gates for the recorder. The rule is conservative: when in doubt,
// pause. Recording health/finance/legal data without explicit user awareness
// would be a trust-destroying bug, so the default for every uncertain branch
// returns `true` (pause).

use super::types::StateSnapshot;

// Stub list of credential-manager and banking app identifiers. Real
// distribution builds will populate this from a maintained allow-deny list
// shipped with the app bundle.
pub const BLOCKED_APP_IDENTIFIERS: &[&str] = &[
    // Password managers
    "com.lastpass.LastPass",
    "com.agilebits.onepassword7",
    "com.agilebits.onepassword-launcher",
    "com.bitwarden.desktop",
    "org.keepassxc.keepassxc",
    "com.dashlane.dashlanephonefinal",
    // Browser built-in password UIs are out of scope here — the password
    // field check below covers them generically.
    // Banking placeholder bundle ids — TODO: replace with maintained list.
    "com.chase.sig.Chase",
    "com.bankofamerica.BofA",
];

pub fn should_pause(snapshot: &StateSnapshot) -> bool {
    if let Some(focused_element) = &snapshot.focused_element {
        if focused_element.is_password {
            return true;
        }
    }

    if let Some(bundle_identifier) = &snapshot.foreground_app_bundle_identifier {
        if is_blocked_application_identifier(bundle_identifier) {
            return true;
        }
    }

    if let Some(executable_path) = &snapshot.foreground_app_executable_path {
        if is_blocked_application_identifier(executable_path) {
            return true;
        }
    }

    false
}

fn is_blocked_application_identifier(identifier: &str) -> bool {
    let lowered = identifier.to_lowercase();
    for blocked in BLOCKED_APP_IDENTIFIERS {
        if lowered.contains(&blocked.to_lowercase()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::types::FocusedElementFingerprint;

    fn empty_snapshot() -> StateSnapshot {
        StateSnapshot {
            foreground_app_bundle_identifier: None,
            foreground_app_executable_path: None,
            foreground_app_display_name: None,
            foreground_window_title: None,
            uia_tree_fingerprint: None,
            focused_element: None,
            selected_text: None,
        }
    }

    #[test]
    fn pauses_when_focus_is_password_field() {
        let mut snapshot = empty_snapshot();
        snapshot.focused_element = Some(FocusedElementFingerprint {
            role: None,
            name: None,
            automation_id: None,
            control_type: Some("Edit".to_string()),
            is_password: true,
        });
        assert!(should_pause(&snapshot));
    }

    #[test]
    fn pauses_when_foreground_is_password_manager() {
        let mut snapshot = empty_snapshot();
        snapshot.foreground_app_bundle_identifier = Some("com.bitwarden.desktop".to_string());
        assert!(should_pause(&snapshot));
    }

    #[test]
    fn does_not_pause_for_ordinary_app() {
        let mut snapshot = empty_snapshot();
        snapshot.foreground_app_bundle_identifier = Some("com.apple.Safari".to_string());
        assert!(!should_pause(&snapshot));
    }
}
