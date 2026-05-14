// Grounding layer entry point. Exposes a unified `ElementResolver` to
// the rest of the app and surfaces three Tauri commands so the TS layer
// can pre-warm the cache on hotkey press, ground a Gemini-emitted label
// at tool-call time, and read the shortcut index back for prompt context.

pub mod persistence;
pub mod resolver;
pub mod target_app;
pub mod types;

#[cfg(target_os = "macos")]
pub mod ax_macos;

#[cfg(target_os = "windows")]
pub mod uia_windows;

use types::{ResolvedTarget, ShortcutBinding};

#[tauri::command]
pub async fn prefetch_target_app() -> Result<(), String> {
    // Run the prefetch off the main thread so the hotkey handler returns
    // immediately. The cache it warms is consulted by `resolve_label`
    // milliseconds later — overlap with Gemini session setup is the win.
    tauri::async_runtime::spawn_blocking(|| {
        let target_app = match target_app::current_target_app() {
            Some(app) => app,
            None => return,
        };

        #[cfg(target_os = "macos")]
        {
            ax_macos::prefetch_for_app(target_app.process_id);
        }

        #[cfg(target_os = "windows")]
        {
            match uia_windows::index_foreground_application(&target_app) {
                Ok(shortcut_index) => {
                    let _ = persistence::save_shortcut_index(&shortcut_index);
                }
                Err(_error) => {
                    // TODO: surface UIA errors via analytics. Swallowing
                    // here keeps the hotkey path resilient — a failed
                    // index walk shouldn't block voice capture.
                }
            }
        }

        let _ = target_app;
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_label(label: String) -> Result<Option<ResolvedTarget>, String> {
    let target_app = target_app::current_target_app();
    tauri::async_runtime::spawn_blocking(move || resolver::resolve(&label, target_app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_shortcut_index(app_id: String) -> Result<Vec<ShortcutBinding>, String> {
    tauri::async_runtime::spawn_blocking(move || resolver::shortcut_index_for_application(&app_id))
        .await
        .map_err(|error| error.to_string())
}
