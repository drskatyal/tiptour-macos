// Windows UIA-driven menu bar walker. Harvests every MenuItem under the
// foreground app's MenuBar control, captures its AcceleratorKey, and
// returns a flat list of `ShortcutBinding`s keyed by the menu hierarchy
// path. The output is what we persist to disk and surface to Gemini.

#![cfg(target_os = "windows")]

use std::time::{SystemTime, UNIX_EPOCH};

use uiautomation::controls::ControlType;
use uiautomation::types::TreeScope;
use uiautomation::{UIAutomation, UIElement};

use super::types::{KeyChord, ShortcutBinding, ShortcutIndex, TargetApp};

const MAX_MENU_DEPTH: usize = 6;

pub fn index_foreground_application(target_app: &TargetApp) -> Result<ShortcutIndex, String> {
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;

    // Anchor on the focused window for the requested process so we don't
    // accidentally walk a different app's menu bar.
    let root = automation
        .get_focused_element()
        .map_err(|error| error.to_string())?;
    let top_window = climb_to_top_window(&root);

    let bindings = walk_menu_bars(&automation, &top_window)?;

    let application_identifier = build_application_identifier(target_app);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);

    Ok(ShortcutIndex {
        application_identifier,
        executable_path: target_app.executable_path.clone(),
        file_version: target_app.file_version.clone(),
        captured_at_unix_ms: now_ms,
        bindings,
    })
}

pub fn build_application_identifier(target_app: &TargetApp) -> String {
    // Combine exe path + file version so different versions cache separately
    // and a self-updating app silently invalidates its stale entry.
    let executable_path = target_app.executable_path.as_deref().unwrap_or("unknown");
    let file_version = target_app.file_version.as_deref().unwrap_or("0");
    format!("{executable_path}@{file_version}")
}

fn climb_to_top_window(element: &UIElement) -> UIElement {
    let mut current = element.clone();
    // UIA's GetParent walks toward the desktop root; stop at the Window node
    // so we scope our menu-bar search to the user's app.
    for _ in 0..32 {
        let control_type = current.get_control_type().unwrap_or(ControlType::Custom);
        if control_type == ControlType::Window {
            return current;
        }
        match current.get_cached_parent() {
            Ok(parent) => current = parent,
            Err(_) => break,
        }
    }
    current
}

fn walk_menu_bars(
    automation: &UIAutomation,
    window: &UIElement,
) -> Result<Vec<ShortcutBinding>, String> {
    let menu_bar_condition = automation
        .create_property_condition(
            uiautomation::variants::UIProperty::ControlType.into(),
            (ControlType::MenuBar as i32).into(),
            None,
        )
        .map_err(|error| error.to_string())?;

    let menu_bars = window
        .find_all(TreeScope::Descendants, &menu_bar_condition)
        .map_err(|error| error.to_string())?;

    let mut bindings: Vec<ShortcutBinding> = Vec::new();
    for menu_bar in menu_bars {
        collect_menu_items(automation, &menu_bar, &mut Vec::new(), &mut bindings, 0);
    }

    Ok(bindings)
}

fn collect_menu_items(
    automation: &UIAutomation,
    parent: &UIElement,
    current_path: &mut Vec<String>,
    output: &mut Vec<ShortcutBinding>,
    depth: usize,
) {
    if depth >= MAX_MENU_DEPTH {
        return;
    }

    // TODO: lazy-submenu expansion. Some apps (Office, Electron) don't
    // populate child MenuItems until the parent fires ExpandCollapsePattern.
    // Phase 1 walks the tree as-is; Phase 1.5 should expand and re-collapse
    // each submenu to harvest deep entries (View → Zoom → ...).
    let children_condition = match automation.create_property_condition(
        uiautomation::variants::UIProperty::ControlType.into(),
        (ControlType::MenuItem as i32).into(),
        None,
    ) {
        Ok(condition) => condition,
        Err(_) => return,
    };

    let menu_items = match parent.find_all(TreeScope::Children, &children_condition) {
        Ok(items) => items,
        Err(_) => return,
    };

    for menu_item in menu_items {
        let label = menu_item.get_name().unwrap_or_default();
        if label.is_empty() {
            continue;
        }

        current_path.push(label.clone());

        let accelerator_text = menu_item.get_accelerator_key().unwrap_or_default();
        if !accelerator_text.is_empty() {
            if let Some(chord) = parse_accelerator_text(&accelerator_text) {
                output.push(ShortcutBinding {
                    menu_path: current_path.clone(),
                    label: label.clone(),
                    accelerator: chord,
                });
            }
        }

        // Some apps (Office, Electron) only realize submenu children when
        // ExpandCollapsePattern.Expand fires — descending the tree as-is
        // misses anything below "View → Zoom →". Try to expand, walk, then
        // collapse so we don't leave the menu visibly hanging open.
        let expand_collapse_pattern = menu_item
            .get_pattern::<uiautomation::patterns::UIExpandCollapsePattern>()
            .ok();
        let needed_expansion = match expand_collapse_pattern.as_ref() {
            Some(pattern) => matches!(
                pattern.get_state(),
                Ok(uiautomation::types::ExpandCollapseState::Collapsed)
            ),
            None => false,
        };
        if needed_expansion {
            if let Some(pattern) = expand_collapse_pattern.as_ref() {
                let _ = pattern.expand();
            }
        }

        collect_menu_items(automation, &menu_item, current_path, output, depth + 1);

        if needed_expansion {
            if let Some(pattern) = expand_collapse_pattern.as_ref() {
                let _ = pattern.collapse();
            }
        }

        current_path.pop();
    }
}

fn parse_accelerator_text(raw_accelerator_text: &str) -> Option<KeyChord> {
    let normalized = raw_accelerator_text.trim();
    if normalized.is_empty() {
        return None;
    }

    let segments: Vec<String> = normalized
        .split('+')
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect();

    if segments.is_empty() {
        return None;
    }

    let (modifiers, key) = segments.split_at(segments.len() - 1);
    let key = key.first().cloned().unwrap_or_default();

    Some(KeyChord {
        raw: normalized.to_string(),
        modifiers: modifiers.to_vec(),
        key,
    })
}

pub fn find_element_coordinate_by_label(label: &str) -> Option<(f64, f64)> {
    let automation = UIAutomation::new().ok()?;
    let root = automation.get_focused_element().ok()?;
    let top_window = climb_to_top_window(&root);

    let name_condition = automation
        .create_property_condition(
            uiautomation::variants::UIProperty::Name.into(),
            label.into(),
            None,
        )
        .ok()?;

    let element = top_window.find_first(TreeScope::Descendants, &name_condition).ok()?;
    let rect = element.get_bounding_rectangle().ok()?;

    // UIA returns physical pixels; the cursor overlay expects logical
    // coordinates. Phase 1 ignores DPI scaling — TODO multi-DPI handling.
    let center_x = rect.get_left() as f64 + rect.get_width() as f64 / 2.0;
    let center_y = rect.get_top() as f64 + rect.get_height() as f64 / 2.0;
    Some((center_x, center_y))
}

// Three-tier lookup against the foreground app's UIA tree: exact Name match,
// case-insensitive Name contains, then AccessibilityName/HelpText fallback.
// Returns the bounding-rectangle center in physical pixels.
pub fn find_element_by_label(_target_app: &TargetApp, label: &str) -> Option<(f64, f64)> {
    let normalized_label = label.trim();
    if normalized_label.is_empty() {
        return None;
    }

    let automation = UIAutomation::new().ok()?;
    let root = automation.get_focused_element().ok()?;
    let top_window = climb_to_top_window(&root);

    // Tier 1: exact Name match.
    if let Ok(exact_condition) = automation.create_property_condition(
        uiautomation::variants::UIProperty::Name.into(),
        normalized_label.into(),
        None,
    ) {
        if let Ok(element) = top_window.find_first(TreeScope::Descendants, &exact_condition) {
            if let Some(center) = center_of_element(&element) {
                return Some(center);
            }
        }
    }

    // Tier 2 + 3: walk all named elements, prefer case-insensitive Name
    // contains, then AccessibilityName/HelpText contains. UIA doesn't expose
    // a "contains" property condition in this crate, so we enumerate every
    // descendant with a non-empty Name once and score in Rust.
    let any_named_condition = automation
        .create_property_condition(
            uiautomation::variants::UIProperty::IsControlElement.into(),
            true.into(),
            None,
        )
        .ok()?;

    let candidate_elements = top_window
        .find_all(TreeScope::Descendants, &any_named_condition)
        .ok()?;

    let needle_lowercased = normalized_label.to_lowercase();
    let mut best_name_contains_match: Option<(f64, f64)> = None;
    let mut best_secondary_match: Option<(f64, f64)> = None;

    for candidate_element in candidate_elements {
        let candidate_name = candidate_element.get_name().unwrap_or_default();
        if !candidate_name.is_empty()
            && candidate_name.to_lowercase().contains(&needle_lowercased)
            && best_name_contains_match.is_none()
        {
            if let Some(center) = center_of_element(&candidate_element) {
                best_name_contains_match = Some(center);
            }
        }

        if best_name_contains_match.is_none() && best_secondary_match.is_none() {
            let accessibility_name = candidate_element.get_localized_control_type().unwrap_or_default();
            let help_text = candidate_element.get_help_text().unwrap_or_default();
            if (accessibility_name.to_lowercase().contains(&needle_lowercased)
                && !accessibility_name.is_empty())
                || (help_text.to_lowercase().contains(&needle_lowercased)
                    && !help_text.is_empty())
            {
                if let Some(center) = center_of_element(&candidate_element) {
                    best_secondary_match = Some(center);
                }
            }
        }
    }

    best_name_contains_match.or(best_secondary_match)
}

// Shallow UIA tree walk anchored at the foreground window. Used by the
// capabilities explorer to fingerprint the current foreground state and
// detect post-action visible changes; we cap depth so trees with thousands
// of descendants (Office, IDEs) don't bloat the snapshot.
pub fn snapshot_foreground_tree(
    max_depth: usize,
) -> Result<crate::capabilities::fingerprint::TreeSnapshot, String> {
    use crate::capabilities::fingerprint::{TreeNodeSnapshot, TreeSnapshot};

    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    let focused_element = automation
        .get_focused_element()
        .map_err(|error| error.to_string())?;
    let top_window = climb_to_top_window(&focused_element);

    let window_title = top_window.get_name().ok().filter(|title| !title.is_empty());
    let root_node = walk_uia_node_shallow(&top_window, 0, max_depth);

    Ok(TreeSnapshot {
        foreground_window_title: window_title,
        root: root_node,
    })
}

fn walk_uia_node_shallow(
    element: &UIElement,
    current_depth: usize,
    max_depth: usize,
) -> crate::capabilities::fingerprint::TreeNodeSnapshot {
    use crate::capabilities::fingerprint::TreeNodeSnapshot;

    let role = element
        .get_control_type()
        .map(|control_type| format!("{:?}", control_type))
        .unwrap_or_else(|_| "Unknown".to_string());
    let name = element.get_name().unwrap_or_default();

    let mut children_snapshots: Vec<TreeNodeSnapshot> = Vec::new();
    if current_depth < max_depth {
        if let Ok(child_elements) = element.get_cached_children() {
            for child_element in child_elements {
                children_snapshots.push(walk_uia_node_shallow(
                    &child_element,
                    current_depth + 1,
                    max_depth,
                ));
            }
        } else if let Some(walker) = UIAutomation::new()
            .ok()
            .and_then(|automation| automation.create_tree_walker().ok())
        {
            // Fall back to a live walk when the element has no cached
            // children — the cached path is faster but isn't populated unless
            // a CacheRequest ran upstream of us.
            let mut current_child = walker.get_first_child(element).ok();
            while let Some(child_element) = current_child {
                children_snapshots.push(walk_uia_node_shallow(
                    &child_element,
                    current_depth + 1,
                    max_depth,
                ));
                current_child = walker.get_next_sibling(&child_element).ok();
            }
        }
    }

    TreeNodeSnapshot {
        role,
        name,
        children: children_snapshots,
    }
}

// Enumerates interactive descendants of the foreground window — buttons,
// menu items, checkboxes, radio buttons, links — so the explorer can use
// them as candidate Click actions. Returns each element's name (which the
// resolver later reuses to ground the click at execution time).
pub fn enumerate_interactive_element_labels() -> Result<Vec<String>, String> {
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    let focused_element = automation
        .get_focused_element()
        .map_err(|error| error.to_string())?;
    let top_window = climb_to_top_window(&focused_element);

    let interactive_control_types: &[ControlType] = &[
        ControlType::Button,
        ControlType::MenuItem,
        ControlType::CheckBox,
        ControlType::RadioButton,
        ControlType::Hyperlink,
    ];

    let mut labels: Vec<String> = Vec::new();
    let mut seen_labels = std::collections::HashSet::new();
    for control_type in interactive_control_types {
        let condition = match automation.create_property_condition(
            uiautomation::variants::UIProperty::ControlType.into(),
            (*control_type as i32).into(),
            None,
        ) {
            Ok(condition) => condition,
            Err(_) => continue,
        };
        let matching_elements = match top_window.find_all(TreeScope::Descendants, &condition) {
            Ok(elements) => elements,
            Err(_) => continue,
        };
        for element in matching_elements {
            let label = element.get_name().unwrap_or_default();
            if label.trim().is_empty() {
                continue;
            }
            if seen_labels.insert(label.clone()) {
                labels.push(label);
            }
        }
    }
    Ok(labels)
}

fn center_of_element(element: &UIElement) -> Option<(f64, f64)> {
    let rect = element.get_bounding_rectangle().ok()?;
    let width = rect.get_width() as f64;
    let height = rect.get_height() as f64;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((
        rect.get_left() as f64 + width / 2.0,
        rect.get_top() as f64 + height / 2.0,
    ))
}
