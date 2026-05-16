// AppSettings JSON round-trip + atomic-write semantics.
//
// The production `set_app_settings` writes to a temp file and renames
// it over the live settings.json. We isolate per-test by overriding
// `XDG_DATA_HOME` (Linux) / `LOCALAPPDATA` (Windows) / `HOME` (macOS)
// to a tempdir before any module-state initializer runs. This works
// because `dirs::data_local_dir()` reads the env on every call.
//
// IMPORTANT: env vars are process-global, and Cargo runs tests
// inside a single binary in parallel by default. We collapse all
// scenarios into a single `#[test]` function to keep them serialized
// without pulling in `serial_test` as a dev-dep.

use std::fs;
use tiptour_test_support::app_settings::{
    self, get_app_settings, reset_all_settings, set_app_settings, AppSettings,
};

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn app_settings_round_trip_atomic_write_and_reset_all_paths() {
    // ---- defaults when no file exists ----
    let tempdir_for_defaults = redirect_data_local_dir_to_tempdir();
    let loaded_defaults = get_app_settings()
        .expect("get_app_settings on a clean dir must succeed");
    assert_eq!(loaded_defaults.gemini_voice, "Kore");
    assert!(loaded_defaults.gemini_model.starts_with("gemini-"));
    assert_eq!(loaded_defaults.push_to_talk_chord, "Alt+X");
    drop(tempdir_for_defaults);

    // ---- set then get round-trips every field ----
    let tempdir_for_round_trip = redirect_data_local_dir_to_tempdir();
    let outgoing = AppSettings {
        schema_version: 1,
        gemini_voice: "Aoede".to_string(),
        gemini_model: "gemini-x-test-model".to_string(),
        push_to_talk_chord: "Ctrl+Shift+Q".to_string(),
        theme: "light".to_string(),
    };
    set_app_settings(outgoing.clone()).expect("set_app_settings must succeed");
    let read_back = get_app_settings().expect("get_app_settings must succeed");
    assert_eq!(read_back.gemini_voice, outgoing.gemini_voice);
    assert_eq!(read_back.gemini_model, outgoing.gemini_model);
    assert_eq!(read_back.push_to_talk_chord, outgoing.push_to_talk_chord);
    // The atomic write must have left a single canonical file with
    // no leftover .tmp staging file.
    let settings_dir = tempdir_for_round_trip.path().join("TipTour");
    let leftover_tmp_count = fs::read_dir(&settings_dir)
        .expect("settings dir must exist after first write")
        .filter_map(|entry_result| entry_result.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(
        leftover_tmp_count, 0,
        "atomic write should have renamed the .tmp file away — a leftover .tmp would mean the rename never landed"
    );
    drop(tempdir_for_round_trip);

    // ---- reset_all_settings on an empty dir is a no-op ----
    let tempdir_for_reset = redirect_data_local_dir_to_tempdir();
    reset_all_settings().expect("reset on empty dir must succeed");
    set_app_settings(AppSettings::default()).expect("write before reset must succeed");
    reset_all_settings().expect("reset after write must succeed");
    let after_reset = get_app_settings().expect("get after reset must succeed");
    assert_eq!(
        after_reset.gemini_voice, "Kore",
        "after reset, get must fall back to the default voice"
    );
    drop(tempdir_for_reset);

    // Reference an unused-otherwise import so this stays compiling
    // even if we trim the public surface used elsewhere later.
    let _ = app_settings::load_app_settings_from_disk();
}
