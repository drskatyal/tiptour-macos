// Cross-platform `ElementResolver`. The Swift app's `ElementResolver.swift`
// is a three-tier lookup (AX → browser CDP → Gemini box_2d). On Windows
// the same role is played by UIA → shortcut-index fallback. This module
// unifies both behind a single API the rest of the app calls.

use super::persistence;
use super::types::{ResolvedTarget, ShortcutBinding, TargetApp};

#[cfg(target_os = "windows")]
use super::uia_windows;

#[cfg(target_os = "macos")]
use super::ax_macos;

pub fn resolve(label: &str, app_hint: Option<TargetApp>) -> Option<ResolvedTarget> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        // Prefer keyboard shortcuts over coordinates — a key chord is more
        // reliable than a click, doesn't move the cursor, and survives
        // window resizes. Match the shortcut index first.
        if let Some(target_app) = app_hint.as_ref() {
            let application_identifier = uia_windows::build_application_identifier(target_app);
            if let Some(shortcut_binding) =
                lookup_shortcut_by_label(&application_identifier, trimmed)
            {
                return Some(ResolvedTarget::Shortcut {
                    chord: shortcut_binding.accelerator,
                });
            }
        }

        if let Some((x, y)) = uia_windows::find_element_coordinate_by_label(trimmed) {
            return Some(ResolvedTarget::Coordinate { x, y });
        }

        return None;
    }

    #[cfg(target_os = "macos")]
    {
        let process_id = app_hint.as_ref().map(|app| app.process_id).unwrap_or(0);
        if let Some((x, y)) = ax_macos::find_element_by_label(process_id, trimmed) {
            return Some(ResolvedTarget::Coordinate { x, y });
        }
        // TODO: Phase 1 only ships the AX tier. Browser CDP + Gemini box_2d
        // fallbacks will land alongside the screen-capture + overlay work.
        return None;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = app_hint;
        None
    }
}

#[cfg(target_os = "windows")]
fn lookup_shortcut_by_label(
    application_identifier: &str,
    label: &str,
) -> Option<ShortcutBinding> {
    let shortcut_index = persistence::load_shortcut_index(application_identifier)?;
    let needle = label.trim().to_lowercase();
    shortcut_index
        .bindings
        .into_iter()
        .find(|binding| binding.label.trim().to_lowercase() == needle)
}

pub fn shortcut_index_for_application(application_identifier: &str) -> Vec<ShortcutBinding> {
    persistence::load_shortcut_index(application_identifier)
        .map(|index| index.bindings)
        .unwrap_or_default()
}
