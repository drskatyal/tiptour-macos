// `get_app_metadata` Tauri command for the About tab.
//
// Reads the version from Cargo's compile-time `CARGO_PKG_VERSION` env so
// we don't have to parse Cargo.toml at runtime, and the target os/arch
// from compile-time `TARGET`-derived `cfg` macros.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMetadata {
    pub version: String,
    pub target_os: String,
    pub target_arch: String,
    /// ISO-8601 build timestamp set at compile time by the build script
    /// when available, else `unknown`.
    pub build_date: String,
    /// Short git commit SHA when the build environment exposes
    /// `GIT_COMMIT`; `None` otherwise (e.g. local `tauri dev`).
    pub git_commit: Option<String>,
}

#[tauri::command]
pub fn get_app_metadata() -> AppMetadata {
    let target_os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
    .to_string();

    let target_arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    }
    .to_string();

    AppMetadata {
        version: env!("CARGO_PKG_VERSION").to_string(),
        target_os,
        target_arch,
        build_date: option_env!("TIPTOUR_BUILD_DATE")
            .unwrap_or("unknown")
            .to_string(),
        git_commit: option_env!("GIT_COMMIT").map(|sha| sha.to_string()),
    }
}

/// Open the TipTour data directory in the OS file browser (Finder on
/// macOS, Explorer on Windows). Called by the About tab's "Open data
/// folder" button.
#[tauri::command]
pub fn open_data_folder_in_os_file_browser() -> Result<(), String> {
    let mut path = dirs::data_local_dir().ok_or_else(|| "no data dir".to_string())?;
    path.push("TipTour");
    // Make sure the directory exists so the OS doesn't error out trying
    // to reveal a non-existent path on a fresh install.
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Trigger the graceful shutdown path the tray's Quit menu item already
/// uses. Lets the About tab's Quit button reuse the same cleanup
/// sequence so an in-flight replay isn't cut off mid-keystroke.
#[tauri::command]
pub fn quit_app_gracefully(app: tauri::AppHandle) {
    crate::multiflow::replayer::set_active_replay_token("__user_quit__");
    let _ = crate::audio::stop_mic_capture();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        app_clone.exit(0);
    });
}

/// Clear the persisted Gemini API key from the OS keychain. Companion
/// to `reset_all_settings` for the About-tab reset flow.
#[tauri::command]
pub fn clear_api_key_from_keychain() -> Result<(), String> {
    use keyring::Entry;
    const SERVICE: &str = "com.tiptour.cross";
    const ACCOUNT: &str = "gemini-api-key";
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(|error| error.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        // No entry = already cleared; treat as success so the caller
        // doesn't need to special-case first-time resets.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
