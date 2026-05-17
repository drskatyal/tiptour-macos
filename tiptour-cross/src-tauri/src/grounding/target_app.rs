// Resolves "what app is the user pointing at?" — the app under the mouse
// at hotkey press time, falling back to the frontmost app. Mirrors the
// Swift app's hover-window-then-frontmost-fallback policy so prefetch
// targets the app the user was actually pointing at, not TipTour itself.

use super::types::TargetApp;

#[cfg(target_os = "windows")]
pub fn current_target_app() -> Option<TargetApp> {
    use std::os::windows::ffi::OsStringExt;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let foreground_window: HWND = GetForegroundWindow();
        if foreground_window.0.is_null() {
            return None;
        }

        let mut process_id_raw: u32 = 0;
        let _thread_id = GetWindowThreadProcessId(foreground_window, Some(&mut process_id_raw));
        if process_id_raw == 0 {
            return None;
        }

        let process_handle = match OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id_raw,
        ) {
            Ok(handle) => handle,
            Err(_) => return None,
        };

        let mut path_buffer: Vec<u16> = vec![0u16; MAX_PATH as usize];
        let mut path_length: u32 = path_buffer.len() as u32;
        let query_result = QueryFullProcessImageNameW(
            process_handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(path_buffer.as_mut_ptr()),
            &mut path_length,
        );
        let _ = CloseHandle(process_handle);

        if query_result.is_err() || path_length == 0 {
            return Some(TargetApp {
                process_id: process_id_raw as i32,
                bundle_identifier: None,
                executable_path: None,
                display_name: None,
                file_version: None,
            });
        }

        let executable_path_string = OsString::from_wide(&path_buffer[..path_length as usize])
            .to_string_lossy()
            .to_string();
        let executable_path = PathBuf::from(&executable_path_string);
        let display_name = executable_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string());

        let file_version = read_file_version(&path_buffer[..path_length as usize]);

        Some(TargetApp {
            process_id: process_id_raw as i32,
            bundle_identifier: None,
            executable_path: Some(executable_path_string),
            display_name,
            file_version,
        })
    }
}

#[cfg(target_os = "windows")]
fn read_file_version(executable_path_wide: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    // Build a null-terminated wide copy because the version APIs expect PCWSTR.
    let mut path_terminated: Vec<u16> = executable_path_wide.to_vec();
    if !path_terminated.last().map_or(false, |last| *last == 0) {
        path_terminated.push(0);
    }
    let path_pointer = PCWSTR(path_terminated.as_ptr());

    unsafe {
        let mut handle: u32 = 0;
        let info_size = GetFileVersionInfoSizeW(path_pointer, Some(&mut handle));
        if info_size == 0 {
            return None;
        }

        let mut info_buffer: Vec<u8> = vec![0u8; info_size as usize];
        let get_info_result = GetFileVersionInfoW(
            path_pointer,
            0,
            info_size,
            info_buffer.as_mut_ptr() as *mut _,
        );
        if get_info_result.is_err() {
            return None;
        }

        // Query the language/codepage translation list to pick a string table.
        let translation_subblock: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let mut translation_pointer: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut translation_length: u32 = 0;
        let translation_query_ok = VerQueryValueW(
            info_buffer.as_ptr() as *const _,
            PCWSTR(translation_subblock.as_ptr()),
            &mut translation_pointer,
            &mut translation_length,
        );

        let (language_id, code_page): (u16, u16) = if translation_query_ok.as_bool()
            && !translation_pointer.is_null()
            && translation_length >= 4
        {
            let translations = std::slice::from_raw_parts(
                translation_pointer as *const u16,
                (translation_length / 2) as usize,
            );
            (translations[0], translations[1])
        } else {
            // Fall back to US English Unicode if the translation table is missing.
            (0x0409, 0x04B0)
        };

        let sub_block_string =
            format!("\\StringFileInfo\\{:04x}{:04x}\\ProductVersion", language_id, code_page);
        let sub_block_wide: Vec<u16> = sub_block_string
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut value_pointer: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut value_length: u32 = 0;
        let version_query_ok = VerQueryValueW(
            info_buffer.as_ptr() as *const _,
            PCWSTR(sub_block_wide.as_ptr()),
            &mut value_pointer,
            &mut value_length,
        );
        if !version_query_ok.as_bool() || value_pointer.is_null() || value_length == 0 {
            return None;
        }

        let utf16_slice = std::slice::from_raw_parts(
            value_pointer as *const u16,
            value_length as usize,
        );
        // Trim the trailing NUL the version table embeds.
        let trimmed_slice: &[u16] = match utf16_slice.iter().position(|c| *c == 0) {
            Some(nul_index) => &utf16_slice[..nul_index],
            None => utf16_slice,
        };
        Some(String::from_utf16_lossy(trimmed_slice))
    }
}

#[cfg(target_os = "macos")]
pub fn current_target_app() -> Option<TargetApp> {
    // Two-tier lookup: first the window directly under the cursor (so the
    // prefetch keys on the app the user pointed at, not TipTour's own panel),
    // then fall back to the frontmost application.
    if let Some(target_app) = window_under_mouse_target_app() {
        return Some(target_app);
    }
    frontmost_application_target_app()
}

#[cfg(target_os = "macos")]
fn window_under_mouse_target_app() -> Option<TargetApp> {
    use core_foundation::array::CFArray;
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::display::{
        kCGNullWindowID, kCGWindowListOptionOnScreenOnly, CGWindowListCopyWindowInfo,
    };

    let mouse_location = current_mouse_location_in_appkit_coordinates()?;

    unsafe {
        let window_info_array_ref =
            CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID);
        if window_info_array_ref.is_null() {
            return None;
        }
        let window_info_array: CFArray<CFDictionary> =
            CFArray::wrap_under_create_rule(window_info_array_ref as *const _);

        for window_dictionary in window_info_array.iter() {
            // The CGWindowList dictionaries are typed as `CFDictionary<*const c_void, *const c_void>`
            // because they're untyped at the CG level. core-foundation 0.10's `.find()` wants
            // the key to impl `ToVoid<K>` where K = `*const c_void`. `&CFString` doesn't impl
            // that combination directly (and tao/cocoa pull in core-foundation 0.9 transitively,
            // multiplying the trait-version confusion). We sidestep both problems by calling
            // the raw CFDictionaryGetValue with a cast `CFStringRef` — same wire-level call,
            // no trait gymnastics, identical lifetime semantics.
            let owner_pid_key = CFString::new("kCGWindowOwnerPID");
            let bounds_key = CFString::new("kCGWindowBounds");
            let layer_key = CFString::new("kCGWindowLayer");

            let owner_pid_raw = cf_dict_get_value_raw(&window_dictionary, &owner_pid_key);
            let owner_pid_value: CFNumber = match owner_pid_raw {
                Some(value_ref) => CFNumber::wrap_under_get_rule(value_ref as _),
                None => continue,
            };
            let process_id: i32 = match owner_pid_value.to_i32() {
                Some(pid) => pid,
                None => continue,
            };
            if process_id == std::process::id() as i32 {
                continue;
            }

            // Only consider normal-layer (layer == 0) windows; menu bars,
            // docks and overlays sit at non-zero layers and would otherwise
            // swallow the hit-test result.
            if let Some(layer_value_ref) = cf_dict_get_value_raw(&window_dictionary, &layer_key) {
                let layer_value: CFNumber =
                    CFNumber::wrap_under_get_rule(layer_value_ref as _);
                if layer_value.to_i32().unwrap_or(1) != 0 {
                    continue;
                }
            }

            let bounds_dictionary_ref = match cf_dict_get_value_raw(&window_dictionary, &bounds_key) {
                Some(value_ref) => value_ref,
                None => continue,
            };
            let bounds_dictionary: CFDictionary =
                CFDictionary::wrap_under_get_rule(bounds_dictionary_ref as _);

            let bounds_x = number_from_dictionary(&bounds_dictionary, "X").unwrap_or(0.0);
            let bounds_y = number_from_dictionary(&bounds_dictionary, "Y").unwrap_or(0.0);
            let bounds_width = number_from_dictionary(&bounds_dictionary, "Width").unwrap_or(0.0);
            let bounds_height =
                number_from_dictionary(&bounds_dictionary, "Height").unwrap_or(0.0);

            if mouse_location.0 < bounds_x
                || mouse_location.0 > bounds_x + bounds_width
                || mouse_location.1 < bounds_y
                || mouse_location.1 > bounds_y + bounds_height
            {
                continue;
            }

            return Some(target_app_for_pid(process_id));
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn cf_dict_get_value_raw(
    dictionary: &core_foundation::dictionary::CFDictionary,
    key: &core_foundation::string::CFString,
) -> Option<core_foundation::base::CFTypeRef> {
    use core_foundation::base::TCFType;
    use core_foundation_sys::dictionary::CFDictionaryGetValue;

    unsafe {
        let value = CFDictionaryGetValue(
            dictionary.as_concrete_TypeRef(),
            key.as_concrete_TypeRef() as *const std::ffi::c_void,
        );
        if value.is_null() {
            None
        } else {
            Some(value as core_foundation::base::CFTypeRef)
        }
    }
}

#[cfg(target_os = "macos")]
fn number_from_dictionary(
    dictionary: &core_foundation::dictionary::CFDictionary,
    key_name: &str,
) -> Option<f64> {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;

    let key = CFString::new(key_name);
    let value_ref = cf_dict_get_value_raw(dictionary, &key)?;
    unsafe {
        let number: CFNumber = CFNumber::wrap_under_get_rule(value_ref as _);
        number.to_f64().or_else(|| number.to_i64().map(|integer| integer as f64))
    }
}

#[cfg(target_os = "macos")]
fn current_mouse_location_in_appkit_coordinates() -> Option<(f64, f64)> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    unsafe {
        let event_class = AnyClass::get("NSEvent")?;
        // NSEvent.mouseLocation returns an NSPoint in screen coordinates
        // (bottom-left origin). CGWindowListCopyWindowInfo bounds are in
        // CG coordinates (top-left origin) — we convert below using the
        // main screen height.
        let mouse_point: objc2_foundation::NSPoint = msg_send![event_class, mouseLocation];

        let screen_class = AnyClass::get("NSScreen")?;
        let main_screen: *mut objc2::runtime::AnyObject = msg_send![screen_class, mainScreen];
        if main_screen.is_null() {
            return Some((mouse_point.x, mouse_point.y));
        }
        let frame: objc2_foundation::NSRect = msg_send![main_screen, frame];
        let cg_y = frame.size.height - mouse_point.y;
        Some((mouse_point.x, cg_y))
    }
}

#[cfg(target_os = "macos")]
fn frontmost_application_target_app() -> Option<TargetApp> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    unsafe {
        let workspace_class = AnyClass::get("NSWorkspace")?;
        let workspace: *mut objc2::runtime::AnyObject =
            msg_send![workspace_class, sharedWorkspace];
        if workspace.is_null() {
            return None;
        }
        let running_application: *mut objc2::runtime::AnyObject =
            msg_send![workspace, frontmostApplication];
        if running_application.is_null() {
            return None;
        }
        let process_id: i32 = msg_send![running_application, processIdentifier];
        Some(target_app_for_running_application(running_application, process_id))
    }
}

#[cfg(target_os = "macos")]
fn target_app_for_pid(process_id: i32) -> TargetApp {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    unsafe {
        if let Some(class) = AnyClass::get("NSRunningApplication") {
            let running_application: *mut objc2::runtime::AnyObject =
                msg_send![class, runningApplicationWithProcessIdentifier: process_id];
            if !running_application.is_null() {
                return target_app_for_running_application(running_application, process_id);
            }
        }
    }
    TargetApp {
        process_id,
        bundle_identifier: None,
        executable_path: None,
        display_name: None,
        file_version: None,
    }
}

#[cfg(target_os = "macos")]
fn target_app_for_running_application(
    running_application: *mut objc2::runtime::AnyObject,
    process_id: i32,
) -> TargetApp {
    use objc2::msg_send;

    unsafe {
        let bundle_identifier = nsstring_property_to_string(running_application, "bundleIdentifier");
        let localized_name = nsstring_property_to_string(running_application, "localizedName");

        let executable_url: *mut objc2::runtime::AnyObject =
            msg_send![running_application, executableURL];
        let executable_path = if executable_url.is_null() {
            None
        } else {
            nsstring_property_to_string(executable_url, "path")
        };

        let bundle_url: *mut objc2::runtime::AnyObject =
            msg_send![running_application, bundleURL];
        let file_version = if bundle_url.is_null() {
            None
        } else {
            read_short_version_from_bundle(bundle_url)
        };

        TargetApp {
            process_id,
            bundle_identifier,
            executable_path,
            display_name: localized_name,
            file_version,
        }
    }
}

#[cfg(target_os = "macos")]
fn nsstring_property_to_string(
    object: *mut objc2::runtime::AnyObject,
    selector_name: &str,
) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::Sel;

    unsafe {
        let selector = Sel::register(selector_name);
        let nsstring: *mut objc2::runtime::AnyObject = msg_send![object, performSelector: selector];
        if nsstring.is_null() {
            return None;
        }
        nsstring_to_rust_string(nsstring)
    }
}

#[cfg(target_os = "macos")]
fn nsstring_to_rust_string(nsstring: *mut objc2::runtime::AnyObject) -> Option<String> {
    use objc2::msg_send;

    unsafe {
        let utf8_pointer: *const std::os::raw::c_char = msg_send![nsstring, UTF8String];
        if utf8_pointer.is_null() {
            return None;
        }
        let c_string = std::ffi::CStr::from_ptr(utf8_pointer);
        c_string.to_str().ok().map(|borrowed| borrowed.to_string())
    }
}

#[cfg(target_os = "macos")]
fn read_short_version_from_bundle(
    bundle_url: *mut objc2::runtime::AnyObject,
) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    unsafe {
        let bundle_class = AnyClass::get("NSBundle")?;
        let bundle: *mut objc2::runtime::AnyObject =
            msg_send![bundle_class, bundleWithURL: bundle_url];
        if bundle.is_null() {
            return None;
        }

        // Build an NSString key for CFBundleShortVersionString.
        let key_nsstring = ns_string_literal("CFBundleShortVersionString")?;
        let value: *mut objc2::runtime::AnyObject =
            msg_send![bundle, objectForInfoDictionaryKey: key_nsstring];
        if value.is_null() {
            return None;
        }
        nsstring_to_rust_string(value)
    }
}

#[cfg(target_os = "macos")]
fn ns_string_literal(rust_string: &str) -> Option<*mut objc2::runtime::AnyObject> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    unsafe {
        let nsstring_class = AnyClass::get("NSString")?;
        let c_string = std::ffi::CString::new(rust_string).ok()?;
        let nsstring: *mut objc2::runtime::AnyObject =
            msg_send![nsstring_class, stringWithUTF8String: c_string.as_ptr()];
        if nsstring.is_null() {
            None
        } else {
            Some(nsstring)
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn current_target_app() -> Option<TargetApp> {
    None
}
