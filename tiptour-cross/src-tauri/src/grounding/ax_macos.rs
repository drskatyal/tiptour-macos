// macOS Accessibility (AX) prefetch + element resolver. Ports the *concept*
// of TipTour/AccessibilityTreeResolver.swift to Rust: warm the AX tree for
// a target pid on hotkey press, then answer label lookups against a cached
// snapshot so the first CUA click resolves against warm data.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

#[derive(Debug, Clone)]
struct CachedElement {
    label: String,
    center_x: f64,
    center_y: f64,
}

#[derive(Debug, Default)]
struct AccessibilityCache {
    elements_by_pid: HashMap<i32, Vec<CachedElement>>,
}

static ACCESSIBILITY_CACHE: Lazy<Mutex<AccessibilityCache>> =
    Lazy::new(|| Mutex::new(AccessibilityCache::default()));

pub fn prefetch_for_app(process_id: i32) {
    // TODO: real AX walk via accessibility-sys.
    //
    // The Swift implementation builds an AXUIElement for the pid via
    // `AXUIElementCreateApplication(pid)`, then walks AXChildren recursively
    // using `AXUIElementCopyMultipleAttributeValues` for ~3-10× speedup.
    // Each element contributes (AXTitle ?? AXDescription ?? AXValue) →
    // AXPosition + AXSize in global AppKit coords.
    //
    // For Phase 1 scaffolding we install the empty cache slot so the rest
    // of the resolver pipeline runs end-to-end on macOS.
    let mut guard = match ACCESSIBILITY_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.elements_by_pid.entry(process_id).or_insert_with(Vec::new);

    // TODO: AXManualAccessibility equivalent — set on Electron apps so they
    // populate their full webpage AX tree. Same trick the Swift app uses on
    // every `NSWorkspace.didActivateApplicationNotification`.

    // TODO: AXUIElementSetMessagingTimeout(0.4) on the system-wide element
    // + per-app on activation, so a single hung query can't stall the
    // resolver longer than 400ms.
}

pub fn find_element_by_label(process_id: i32, label: &str) -> Option<(f64, f64)> {
    let guard = ACCESSIBILITY_CACHE.lock().ok()?;
    let elements = guard.elements_by_pid.get(&process_id)?;

    let normalized_label = label.trim().to_lowercase();
    elements
        .iter()
        .find(|element| element.label.trim().to_lowercase() == normalized_label)
        .map(|element| (element.center_x, element.center_y))
}

// Internal helper so the future real AX walk can drop results into the cache
// without exposing the lock to callers. Kept pub(super) so the eventual
// production implementation in this same module can populate the cache.
#[allow(dead_code)]
pub(super) fn store_cached_elements(process_id: i32, elements: Vec<(String, f64, f64)>) {
    let mut guard = match ACCESSIBILITY_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let cached = elements
        .into_iter()
        .map(|(label, center_x, center_y)| CachedElement {
            label,
            center_x,
            center_y,
        })
        .collect();
    guard.elements_by_pid.insert(process_id, cached);
}
