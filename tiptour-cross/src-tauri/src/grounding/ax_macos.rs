// macOS Accessibility (AX) prefetch + element resolver. Ports the *concept*
// of TipTour/AccessibilityTreeResolver.swift to Rust: warm the AX tree for
// a target pid on hotkey press, then answer label lookups against a cached
// snapshot so the first CUA click resolves against warm data.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::sync::Mutex;

use accessibility_sys::{
    kAXChildrenAttribute, kAXDescriptionAttribute, kAXManualAccessibility,
    kAXPositionAttribute, kAXRoleAttribute, kAXSizeAttribute, kAXTitleAttribute,
    kAXValueAttribute, kAXValueTypeCGPoint, kAXValueTypeCGSize, AXError,
    AXUIElementCopyAttributeValue, AXUIElementCopyMultipleAttributeValues,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetAttributeValue,
    AXUIElementSetMessagingTimeout, AXValueGetType, AXValueGetValue, AXValueRef,
};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFType, CFTypeID, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::{CFString, CFStringRef};
use once_cell::sync::Lazy;

const AX_WALK_MAX_DEPTH: usize = 20;
const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 0.4;

// We declare the missing `kAXManualAccessibility` symbol as a plain string
// because accessibility-sys may not export it consistently across versions.
// The Swift app sets this attribute on Electron apps so they populate
// their full webpage AX tree.
#[allow(dead_code)]
const MANUAL_ACCESSIBILITY_ATTRIBUTE: &str = "AXManualAccessibility";

#[derive(Debug, Clone)]
struct CachedElement {
    label: String,
    center_x: f64,
    center_y: f64,
    #[allow(dead_code)]
    role: String,
}

#[derive(Debug, Default)]
struct AccessibilityCache {
    elements_by_pid: HashMap<i32, Vec<CachedElement>>,
}

static ACCESSIBILITY_CACHE: Lazy<Mutex<AccessibilityCache>> =
    Lazy::new(|| Mutex::new(AccessibilityCache::default()));

pub fn prefetch_for_app(process_id: i32) {
    // Build the AX root for the application. AXUIElementCreateApplication
    // never fails — it just hands back an element that errors on every
    // attribute call if the pid is bogus, so we still need defensive checks
    // inside the walk.
    let application_element: AXUIElementRef = unsafe { AXUIElementCreateApplication(process_id) };
    if application_element.is_null() {
        return;
    }

    unsafe {
        // Cap any single AX query at 400ms. Without this, Electron and
        // Xcode trees can hang the resolver indefinitely while waiting for
        // a slow renderer process to answer an attribute read.
        AXUIElementSetMessagingTimeout(application_element, AX_MESSAGING_TIMEOUT_SECONDS);

        // AXManualAccessibility makes Electron-based apps (Slack, Discord,
        // VS Code, Cursor, Notion, Figma) expose their full webpage AX
        // tree. Non-Electron apps return kAXErrorAttributeUnsupported,
        // which we silently ignore.
        let manual_attribute = CFString::new(MANUAL_ACCESSIBILITY_ATTRIBUTE);
        let _ = AXUIElementSetAttributeValue(
            application_element,
            manual_attribute.as_concrete_TypeRef(),
            CFBoolean::true_value().as_CFTypeRef(),
        );
    }

    let mut collected_elements: Vec<CachedElement> = Vec::new();
    walk_element_recursively(application_element, 0, &mut collected_elements);

    {
        let mut guard = match ACCESSIBILITY_CACHE.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.elements_by_pid.insert(process_id, collected_elements);
    }

    unsafe {
        // The AX application element is created by us and must be released
        // (Create rule). Subsequent children obtained via copy attribute
        // calls are released inside the recursive walker.
        CFRelease(application_element as CFTypeRef);
    }
}

pub fn find_element_by_label(process_id: i32, label: &str) -> Option<(f64, f64)> {
    let guard = ACCESSIBILITY_CACHE.lock().ok()?;
    let elements = guard.elements_by_pid.get(&process_id)?;

    let needle_trimmed = label.trim();
    if needle_trimmed.is_empty() {
        return None;
    }
    let needle_lowercased = needle_trimmed.to_lowercase();

    if let Some(exact_match) = elements.iter().find(|element| {
        element.label.trim().to_lowercase() == needle_lowercased
    }) {
        return Some((exact_match.center_x, exact_match.center_y));
    }

    elements
        .iter()
        .find(|element| {
            element
                .label
                .trim()
                .to_lowercase()
                .contains(&needle_lowercased)
        })
        .map(|element| (element.center_x, element.center_y))
}

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
            role: String::new(),
        })
        .collect();
    guard.elements_by_pid.insert(process_id, cached);
}

// -- Internal walker ---------------------------------------------------------

struct BatchedNodeAttributes {
    role: String,
    title: String,
    description: String,
    value: String,
    frame_center: Option<(f64, f64)>,
}

fn walk_element_recursively(
    element: AXUIElementRef,
    current_depth: usize,
    output: &mut Vec<CachedElement>,
) {
    if current_depth >= AX_WALK_MAX_DEPTH || element.is_null() {
        return;
    }

    // Batch-read role + label-candidate attributes + geometry in a single IPC.
    if let Some(batched_attributes) = batch_read_node_attributes(element) {
        let preferred_label = pick_preferred_label(
            &batched_attributes.title,
            &batched_attributes.description,
            &batched_attributes.value,
        );
        if let (Some(label_text), Some(center)) =
            (preferred_label, batched_attributes.frame_center)
        {
            output.push(CachedElement {
                label: label_text,
                center_x: center.0,
                center_y: center.1,
                role: batched_attributes.role,
            });
        }
    }

    // Recurse into children. Children come back as a CFArray of AXUIElement
    // refs; we copy each ref by retaining it so the recursive call owns its
    // own reference and the array release at the end is safe.
    let children_attribute = CFString::new(kAXChildrenAttribute);
    let mut children_raw: CFTypeRef = std::ptr::null_mut();
    let copy_status: AXError = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            children_attribute.as_concrete_TypeRef(),
            &mut children_raw,
        )
    };
    if copy_status != 0 || children_raw.is_null() {
        return;
    }

    unsafe {
        let array_ref = children_raw as CFArrayRef;
        let array: CFArray<CFType> = CFArray::wrap_under_create_rule(array_ref);
        let child_count = array.len();
        for child_index in 0..child_count {
            if let Some(child_type_ref) = array.get(child_index) {
                let child_element_ref =
                    child_type_ref.as_CFTypeRef() as AXUIElementRef;
                walk_element_recursively(child_element_ref, current_depth + 1, output);
            }
        }
    }
}

fn batch_read_node_attributes(element: AXUIElementRef) -> Option<BatchedNodeAttributes> {
    // The order here is positional — readers below index into the result
    // array by these positions so do not reorder without updating them.
    let attribute_names: Vec<CFString> = vec![
        CFString::new(kAXRoleAttribute),
        CFString::new(kAXTitleAttribute),
        CFString::new(kAXDescriptionAttribute),
        CFString::new(kAXValueAttribute),
        CFString::new(kAXPositionAttribute),
        CFString::new(kAXSizeAttribute),
    ];
    let attribute_name_refs: Vec<CFStringRef> = attribute_names
        .iter()
        .map(|name| name.as_concrete_TypeRef())
        .collect();

    let names_array = CFArray::from_copyable(&attribute_name_refs);

    let mut values_array_ref: CFArrayRef = std::ptr::null_mut();
    let status: AXError = unsafe {
        AXUIElementCopyMultipleAttributeValues(
            element,
            names_array.as_concrete_TypeRef(),
            0, // do not stop on per-attribute error
            &mut values_array_ref,
        )
    };
    if status != 0 || values_array_ref.is_null() {
        return None;
    }

    unsafe {
        let values_array: CFArray<CFType> = CFArray::wrap_under_create_rule(values_array_ref);
        if values_array.len() < attribute_names.len() as isize {
            return None;
        }

        let string_at = |index: isize| -> String {
            match values_array.get(index) {
                Some(type_ref) => extract_string_from_cf_type(type_ref.as_CFTypeRef()),
                None => String::new(),
            }
        };

        let role = string_at(0);
        let title = string_at(1);
        let description = string_at(2);
        let value = string_at(3);

        let frame_center =
            extract_frame_center(values_array.get(4), values_array.get(5));

        Some(BatchedNodeAttributes {
            role,
            title,
            description,
            value,
            frame_center,
        })
    }
}

fn pick_preferred_label(title: &str, description: &str, value: &str) -> Option<String> {
    if !title.trim().is_empty() {
        return Some(title.to_string());
    }
    if !description.trim().is_empty() {
        return Some(description.to_string());
    }
    if !value.trim().is_empty() {
        return Some(value.to_string());
    }
    None
}

unsafe fn extract_string_from_cf_type(cf_type_ref: CFTypeRef) -> String {
    if cf_type_ref.is_null() {
        return String::new();
    }
    let type_id: CFTypeID = core_foundation::base::CFGetTypeID(cf_type_ref);
    if type_id != CFString::type_id() {
        return String::new();
    }
    let cf_string: CFString = CFString::wrap_under_get_rule(cf_type_ref as CFStringRef);
    cf_string.to_string()
}

unsafe fn extract_frame_center(
    position_value: Option<CFType>,
    size_value: Option<CFType>,
) -> Option<(f64, f64)> {
    let position_type = position_value?;
    let size_type = size_value?;

    let position_value_ref = position_type.as_CFTypeRef() as AXValueRef;
    let size_value_ref = size_type.as_CFTypeRef() as AXValueRef;

    if AXValueGetType(position_value_ref) != kAXValueTypeCGPoint {
        return None;
    }
    if AXValueGetType(size_value_ref) != kAXValueTypeCGSize {
        return None;
    }

    #[repr(C)]
    struct CGPointRepresentation {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    struct CGSizeRepresentation {
        width: f64,
        height: f64,
    }

    let mut position = CGPointRepresentation { x: 0.0, y: 0.0 };
    let mut size = CGSizeRepresentation {
        width: 0.0,
        height: 0.0,
    };

    let got_position = AXValueGetValue(
        position_value_ref,
        kAXValueTypeCGPoint,
        &mut position as *mut _ as *mut std::ffi::c_void,
    );
    let got_size = AXValueGetValue(
        size_value_ref,
        kAXValueTypeCGSize,
        &mut size as *mut _ as *mut std::ffi::c_void,
    );
    if !got_position || !got_size {
        return None;
    }
    if size.width <= 0.0 || size.height <= 0.0 {
        return None;
    }
    Some((
        position.x + size.width / 2.0,
        position.y + size.height / 2.0,
    ))
}
