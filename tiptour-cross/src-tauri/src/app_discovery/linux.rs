// Linux is not a target platform; this stub keeps the cross-platform
// build clean so `cargo check` on Linux compiles every dependent module.

#![cfg(target_os = "linux")]

use super::types::DiscoveredApp;

pub fn scan() -> Vec<DiscoveredApp> {
    Vec::new()
}
