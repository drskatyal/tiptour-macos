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

// Gemini emits box_2d as [y1, x1, y2, x2] normalized to 0..=1000 with a
// top-left origin. The caller passes screen dimensions so we can convert
// the normalized box back into global AppKit / Windows screen coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Box2dHint {
    pub normalized: [u32; 4],
    pub screen_width: f64,
    pub screen_height: f64,
}

pub fn resolve(label: &str, app_hint: Option<TargetApp>) -> Option<ResolvedTarget> {
    resolve_with_box_hint(label, app_hint, None)
}

pub fn resolve_with_box_hint(
    label: &str,
    app_hint: Option<TargetApp>,
    box_2d_hint: Option<Box2dHint>,
) -> Option<ResolvedTarget> {
    let trimmed_label = label.trim();
    if trimmed_label.is_empty() {
        // An empty label still admits a box_2d hint — Gemini sometimes emits
        // a coordinate without an accompanying label (raw spatial grounding).
        if let Some(box_2d_hint) = box_2d_hint {
            return resolved_target_from_box_hint(box_2d_hint);
        }
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        // Tier 1 (Windows): keyboard shortcut from the cached menu-bar index.
        // A key chord is more reliable than a click, doesn't move the cursor,
        // and survives window resizes.
        if let Some(target_app) = app_hint.as_ref() {
            let application_identifier = uia_windows::build_application_identifier(target_app);
            if let Some(shortcut_binding) =
                lookup_shortcut_by_label(&application_identifier, trimmed_label)
            {
                return Some(ResolvedTarget::Shortcut {
                    chord: shortcut_binding.accelerator,
                });
            }
        }

        // Tier 2 (Windows): live UIA search for an element matching the label.
        if let Some(target_app) = app_hint.as_ref() {
            if let Some((x, y)) = uia_windows::find_element_by_label(target_app, trimmed_label) {
                return Some(ResolvedTarget::Coordinate { x, y });
            }
        }
        if let Some((x, y)) = uia_windows::find_element_coordinate_by_label(trimmed_label) {
            return Some(ResolvedTarget::Coordinate { x, y });
        }

        // Tier 3 (Windows): fall back to Gemini's own box_2d spatial guess.
        if let Some(box_2d_hint) = box_2d_hint {
            return resolved_target_from_box_hint(box_2d_hint);
        }
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        // Tier 1 (macOS): warmed AX cache from prefetch_for_app.
        let process_id = app_hint.as_ref().map(|app| app.process_id).unwrap_or(0);
        if process_id != 0 {
            if let Some((x, y)) = ax_macos::find_element_by_label(process_id, trimmed_label) {
                return Some(ResolvedTarget::Coordinate { x, y });
            }
        }

        // Tier 2 (macOS): Gemini box_2d hint. Browser CDP fallback will land
        // alongside the screen-capture pipeline; for now we trust the
        // model's spatial grounding when AX missed.
        if let Some(box_2d_hint) = box_2d_hint {
            return resolved_target_from_box_hint(box_2d_hint);
        }
        return None;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = app_hint;
        if let Some(box_2d_hint) = box_2d_hint {
            return resolved_target_from_box_hint(box_2d_hint);
        }
        None
    }
}

fn resolved_target_from_box_hint(box_hint: Box2dHint) -> Option<ResolvedTarget> {
    // Gemini's encoding: [y1, x1, y2, x2] in 0..=1000, top-left origin.
    let normalized_y1 = box_hint.normalized[0] as f64;
    let normalized_x1 = box_hint.normalized[1] as f64;
    let normalized_y2 = box_hint.normalized[2] as f64;
    let normalized_x2 = box_hint.normalized[3] as f64;

    if box_hint.screen_width <= 0.0 || box_hint.screen_height <= 0.0 {
        return None;
    }

    let center_x = ((normalized_x1 + normalized_x2) / 2.0 / 1000.0) * box_hint.screen_width;
    let center_y = ((normalized_y1 + normalized_y2) / 2.0 / 1000.0) * box_hint.screen_height;
    Some(ResolvedTarget::Coordinate {
        x: center_x,
        y: center_y,
    })
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
