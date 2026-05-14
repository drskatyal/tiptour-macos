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

        collect_menu_items(automation, &menu_item, current_path, output, depth + 1);
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
