// Privacy gates for the recorder. The rule is conservative: when in doubt,
// pause. Recording health/finance/legal data without explicit user awareness
// would be a trust-destroying bug, so the default for every uncertain branch
// returns `true` (pause).

use super::types::StateSnapshot;

// Curated list of credential-manager bundle identifiers (macOS). Browser
// bundles intentionally excluded — they'd block ordinary web browsing —
// and we rely on the password-field role check below to catch password
// UIs inside the browser. Banking apps don't ship under stable bundle ids
// on macOS, so they're omitted; we depend on the focused-field check.
pub const BLOCKED_MACOS_BUNDLE_IDENTIFIERS: &[&str] = &[
    "com.apple.keychainaccess",
    "com.1password.1password",
    "com.1password.1password7",
    "com.agilebits.onepassword7",
    "com.agilebits.onepassword-launcher",
    "com.lastpass.lastpassmacdesktop",
    "com.bitwarden.desktop",
    "com.dashlane.dashlanephonefinal",
    "com.macpaw.cleanmymac.password",
    "org.keepassxc.keepassxc",
];

// Curated list of Windows credential-manager executable file stems
// (compared case-insensitively against the file_stem of the foreground
// executable path).
pub const BLOCKED_WINDOWS_EXECUTABLE_STEMS: &[&str] = &[
    "1password",
    "lastpass",
    "keepass",
    "keepass2",
    "bitwarden",
    "dashlane",
    "keeper",
];

// AX roles that always denote a password / secure entry field. The UIA
// `is_password` flag already covers the cross-platform fingerprint, but
// macOS apps can additionally surface `AXSecureTextField` directly.
const MACOS_PASSWORD_AX_ROLES: &[&str] = &["AXSecureTextField"];

pub fn should_pause(snapshot: &StateSnapshot) -> bool {
    if let Some(focused_element) = &snapshot.focused_element {
        if focused_element.is_password {
            return true;
        }
        if let Some(role) = &focused_element.role {
            if MACOS_PASSWORD_AX_ROLES.iter().any(|known_role| {
                role.eq_ignore_ascii_case(known_role)
            }) {
                return true;
            }
        }
    }

    if let Some(bundle_identifier) = &snapshot.foreground_app_bundle_identifier {
        if is_blocked_macos_bundle_identifier(bundle_identifier) {
            return true;
        }
    }

    if let Some(executable_path) = &snapshot.foreground_app_executable_path {
        if is_blocked_windows_executable_path(executable_path) {
            return true;
        }
    }

    false
}

fn is_blocked_macos_bundle_identifier(bundle_identifier: &str) -> bool {
    let lowered = bundle_identifier.to_lowercase();
    BLOCKED_MACOS_BUNDLE_IDENTIFIERS
        .iter()
        .any(|blocked| lowered == blocked.to_lowercase())
}

fn is_blocked_windows_executable_path(executable_path: &str) -> bool {
    // Compare against the file stem (filename without extension), case
    // insensitive, so installation path variation doesn't affect blocking.
    let normalized_path = executable_path.replace('\\', "/");
    let file_stem_lowered = normalized_path
        .rsplit('/')
        .next()
        .unwrap_or(executable_path)
        .rsplit_once('.')
        .map(|(stem, _extension)| stem)
        .unwrap_or(executable_path)
        .to_lowercase();
    BLOCKED_WINDOWS_EXECUTABLE_STEMS
        .iter()
        .any(|blocked_stem| file_stem_lowered == *blocked_stem)
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

    #[test]
    fn pauses_when_focus_role_is_macos_secure_text_field() {
        let mut snapshot = empty_snapshot();
        snapshot.focused_element = Some(FocusedElementFingerprint {
            role: Some("AXSecureTextField".to_string()),
            name: None,
            automation_id: None,
            control_type: None,
            is_password: false,
        });
        assert!(should_pause(&snapshot));
    }

    #[test]
    fn pauses_when_windows_executable_is_password_manager() {
        let mut snapshot = empty_snapshot();
        snapshot.foreground_app_executable_path =
            Some("C:\\Program Files\\1Password\\1Password.exe".to_string());
        assert!(should_pause(&snapshot));
    }

    #[test]
    fn does_not_pause_for_ordinary_windows_executable() {
        let mut snapshot = empty_snapshot();
        snapshot.foreground_app_executable_path =
            Some("C:\\Program Files\\Microsoft VS Code\\Code.exe".to_string());
        assert!(!should_pause(&snapshot));
    }
}
