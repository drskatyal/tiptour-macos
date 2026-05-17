// Crash recovery JSON round-trip + previous-crash detection.
//
// The first boot has no prior state and should report no crash. A
// second boot without an intervening clean-shutdown stamp should
// surface the previous run as crashed. After a clean shutdown stamp,
// the following boot should NOT report a crash.

use tiptour_test_support::crash_recovery::{
    mark_clean_shutdown, record_boot_and_detect_previous_crash,
};

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn crash_recovery_detects_unclean_shutdown_and_round_trips_clean_state() {
    let _tempdir = redirect_data_local_dir_to_tempdir();

    // First boot — no prior state file.
    let first_boot_detected_crash = record_boot_and_detect_previous_crash();
    assert!(
        !first_boot_detected_crash,
        "first-ever boot has no prior state, so no crash should be reported"
    );

    // Simulate a crash by booting again without a clean shutdown.
    let second_boot_detected_crash = record_boot_and_detect_previous_crash();
    assert!(
        second_boot_detected_crash,
        "second boot without clean shutdown should detect the previous run as crashed"
    );

    // Mark clean shutdown then boot again — should NOT detect a crash.
    mark_clean_shutdown();
    let third_boot_detected_crash = record_boot_and_detect_previous_crash();
    assert!(
        !third_boot_detected_crash,
        "boot after a clean shutdown stamp should NOT detect a crash"
    );

    // And again — a boot directly after the third (which itself wrote
    // clean_shutdown=false) should detect a crash, since the third
    // boot never marked itself clean.
    let fourth_boot_detected_crash = record_boot_and_detect_previous_crash();
    assert!(
        fourth_boot_detected_crash,
        "boot after a boot that never marked clean should detect a crash"
    );
}
