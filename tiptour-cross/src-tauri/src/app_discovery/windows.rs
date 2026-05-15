// Windows application scan. Two complementary sources:
//   1. Start Menu .lnk shortcuts under the per-user and machine-wide
//      Start Menu Programs directories. The `lnk` crate resolves each
//      shortcut to its target executable.
//   2. Uninstall registry entries under HKLM Uninstall + the WOW6432Node
//      sibling for 32-bit installers on 64-bit Windows. Provides display
//      names + install locations for apps that exist outside the Start
//      Menu (Steam games, dev tools installed without shortcuts).
//
// Results are merged and deduped by canonical_id (lowercased exe stem).

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::aliases::generate_aliases;
use super::types::{DiscoveredApp, LaunchTarget};

pub fn scan() -> Vec<DiscoveredApp> {
    let mut accumulator: HashMap<String, DiscoveredApp> = HashMap::new();

    scan_start_menu(&mut accumulator);
    scan_uninstall_registry(&mut accumulator);

    let mut discovered: Vec<DiscoveredApp> = accumulator.into_values().collect();
    discovered.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    discovered
}

fn scan_start_menu(accumulator: &mut HashMap<String, DiscoveredApp>) {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Ok(programdata) = std::env::var("PROGRAMDATA") {
        roots.push(PathBuf::from(programdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    for root in roots {
        walk_for_lnk_files(&root, accumulator, 0);
    }
}

fn walk_for_lnk_files(
    directory: &Path,
    accumulator: &mut HashMap<String, DiscoveredApp>,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry_result in entries {
        let Ok(entry) = entry_result else { continue };
        let path = entry.path();
        if path.is_dir() {
            walk_for_lnk_files(&path, accumulator, depth + 1);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()).map(str::to_lowercase) != Some("lnk".into()) {
            continue;
        }
        if let Some(app) = parse_lnk_shortcut(&path) {
            accumulator.entry(app.canonical_id.clone()).or_insert(app);
        }
    }
}

fn parse_lnk_shortcut(lnk_path: &Path) -> Option<DiscoveredApp> {
    let parsed = lnk::ShellLink::open(lnk_path).ok()?;
    // `relative_path` / `link_info.local_base_path` are both possible
    // sources of the resolved target. Try the most authoritative first.
    let resolved_target_path = parsed
        .link_info()
        .as_ref()
        .and_then(|info| info.local_base_path().clone())
        .or_else(|| parsed.relative_path().clone())?;
    if !resolved_target_path.to_lowercase().ends_with(".exe") {
        return None;
    }

    let display_name = lnk_path
        .file_stem()
        .and_then(OsStr::to_str)
        .map(|s| s.to_string())?;

    let exe_stem_lowercase = Path::new(&resolved_target_path)
        .file_stem()
        .and_then(OsStr::to_str)
        .map(|s| s.to_lowercase())?;

    let aliases = generate_aliases(&display_name);
    if aliases.is_empty() {
        return None;
    }

    Some(DiscoveredApp {
        canonical_id: exe_stem_lowercase,
        display_name,
        launch_target: LaunchTarget::ExecutablePath {
            value: resolved_target_path,
        },
        aliases,
        is_user_enabled: true,
    })
}

fn scan_uninstall_registry(accumulator: &mut HashMap<String, DiscoveredApp>) {
    // We deliberately read through both 64-bit and 32-bit views via the
    // `windows` crate's RegOpenKeyEx + KEY_WOW64_* flags. Errors at this
    // layer are non-fatal — the Start Menu pass already covers the
    // common cases.
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_SAM_FLAGS,
    };

    let views: &[REG_SAM_FLAGS] = &[KEY_WOW64_64KEY, KEY_WOW64_32KEY];
    let subkey_path = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall");

    for view in views {
        let mut uninstall_root: HKEY = HKEY::default();
        let open_result = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                subkey_path,
                0,
                KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE | *view,
                &mut uninstall_root,
            )
        };
        if open_result != ERROR_SUCCESS {
            continue;
        }

        let mut subkey_index: u32 = 0;
        loop {
            let mut name_buffer = [0u16; 512];
            let mut name_length: u32 = name_buffer.len() as u32;
            let enum_result = unsafe {
                RegEnumKeyExW(
                    uninstall_root,
                    subkey_index,
                    windows::core::PWSTR(name_buffer.as_mut_ptr()),
                    &mut name_length,
                    None,
                    windows::core::PWSTR::null(),
                    None,
                    None,
                )
            };
            if enum_result != ERROR_SUCCESS {
                break;
            }
            subkey_index += 1;
            let subkey_name = String::from_utf16_lossy(&name_buffer[..name_length as usize]);
            let subkey_wide: Vec<u16> = subkey_name
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let mut entry_handle: HKEY = HKEY::default();
            let entry_open = unsafe {
                RegOpenKeyExW(
                    uninstall_root,
                    PCWSTR(subkey_wide.as_ptr()),
                    0,
                    KEY_QUERY_VALUE | *view,
                    &mut entry_handle,
                )
            };
            if entry_open != ERROR_SUCCESS {
                continue;
            }

            let display_name = read_registry_string(entry_handle, "DisplayName");
            let install_location = read_registry_string(entry_handle, "InstallLocation");
            let display_icon = read_registry_string(entry_handle, "DisplayIcon");
            unsafe {
                let _ = RegCloseKey(entry_handle);
            }

            let display_name = match display_name {
                Some(value) if !value.trim().is_empty() => value,
                _ => continue,
            };
            // Prefer DisplayIcon (often points at the exe directly);
            // fall back to InstallLocation as a directory hint.
            let exe_path = display_icon
                .as_deref()
                .and_then(extract_exe_path_from_icon_field)
                .or_else(|| {
                    install_location
                        .as_deref()
                        .and_then(|location| guess_exe_in_directory(Path::new(location)))
                });
            let Some(exe_path) = exe_path else { continue };

            let exe_stem_lowercase = Path::new(&exe_path)
                .file_stem()
                .and_then(OsStr::to_str)
                .map(|s| s.to_lowercase());
            let Some(canonical_id) = exe_stem_lowercase else {
                continue;
            };

            let aliases = generate_aliases(&display_name);
            if aliases.is_empty() {
                continue;
            }
            accumulator
                .entry(canonical_id.clone())
                .or_insert(DiscoveredApp {
                    canonical_id,
                    display_name,
                    launch_target: LaunchTarget::ExecutablePath { value: exe_path },
                    aliases,
                    is_user_enabled: true,
                });
        }

        unsafe {
            let _ = RegCloseKey(uninstall_root);
        }
        let _ = subkey_index;
    }
}

fn read_registry_string(handle: windows::Win32::System::Registry::HKEY, value_name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegQueryValueExW, REG_VALUE_TYPE};

    let name_wide: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = [0u16; 1024];
    let mut buffer_byte_length: u32 = (buffer.len() * 2) as u32;
    let mut value_type: REG_VALUE_TYPE = REG_VALUE_TYPE::default();
    let result = unsafe {
        RegQueryValueExW(
            handle,
            PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut value_type),
            Some(buffer.as_mut_ptr() as *mut u8),
            Some(&mut buffer_byte_length),
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let char_count = (buffer_byte_length as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..char_count]))
}

fn extract_exe_path_from_icon_field(display_icon_value: &str) -> Option<String> {
    // DisplayIcon entries look like `C:\Path\app.exe,0` — strip the icon
    // index suffix and quote wrappers before returning.
    let stripped = display_icon_value.trim().trim_matches('"');
    let without_index = match stripped.rfind(',') {
        Some(index) => &stripped[..index],
        None => stripped,
    };
    if without_index.to_lowercase().ends_with(".exe") {
        Some(without_index.to_string())
    } else {
        None
    }
}

fn guess_exe_in_directory(install_directory: &Path) -> Option<String> {
    if !install_directory.exists() {
        return None;
    }
    let entries = std::fs::read_dir(install_directory).ok()?;
    for entry_result in entries {
        let Ok(entry) = entry_result else { continue };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()).map(str::to_lowercase) == Some("exe".into()) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}
