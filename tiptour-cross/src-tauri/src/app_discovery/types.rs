// Discovered-application records shared across the macOS / Windows / Linux
// scanners. Aliases are the user-pronounceable launch phrases the Vosk
// grammar injects under "open <alias>" / "launch <alias>" / etc.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LaunchTarget {
    BundleId { value: String },
    ExecutablePath { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredApp {
    pub canonical_id: String,
    pub display_name: String,
    pub launch_target: LaunchTarget,
    pub aliases: Vec<String>,
    pub is_user_enabled: bool,
}

impl DiscoveredApp {
    /// Convert the launch target into the single-string identifier the
    /// existing `executor::ExecutableAction::LaunchApp { bundle_id_or_exe }`
    /// path consumes. The shape of the string (dotted bundle id vs path)
    /// is what the existing `launch_app` helper keys off, so we don't need
    /// a separate enum branch downstream.
    pub fn launch_identifier(&self) -> &str {
        match &self.launch_target {
            LaunchTarget::BundleId { value } => value,
            LaunchTarget::ExecutablePath { value } => value,
        }
    }
}
