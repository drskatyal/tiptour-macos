// Cost meter — recording usage updates session totals + today's
// rolling-history entry, and the JSON file round-trips.

use tiptour_test_support::cost_meter::{
    get_cost_history, get_session_cost, get_today_cost, record_usage,
    reset_in_memory_state_for_tests,
};

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn cost_meter_round_trip_and_session_today_history() {
    let _tempdir = redirect_data_local_dir_to_tempdir();
    reset_in_memory_state_for_tests();

    // Empty before any usage.
    let initial_session = get_session_cost().expect("session cost must read");
    assert_eq!(initial_session.input_tokens, 0);
    assert_eq!(initial_session.output_tokens, 0);
    assert_eq!(initial_session.usd_cost, 0.0);

    // Record a usage frame: 1M input + 1M output → $3 + $15 = $18.
    record_usage(1_000_000, 1_000_000).expect("record_usage must succeed");

    let after_one = get_session_cost().expect("session must read");
    assert_eq!(after_one.input_tokens, 1_000_000);
    assert_eq!(after_one.output_tokens, 1_000_000);
    assert!((after_one.usd_cost - 18.0).abs() < 0.0001, "cost should be ~ $18");

    let today = get_today_cost().expect("today must read");
    assert_eq!(today.input_tokens, 1_000_000);
    assert!((today.usd_cost - 18.0).abs() < 0.0001);

    // Second frame accumulates onto the same day + session.
    record_usage(500_000, 0).expect("record_usage must succeed");
    let after_two = get_session_cost().expect("session must read");
    assert_eq!(after_two.input_tokens, 1_500_000);
    assert!((after_two.usd_cost - 19.5).abs() < 0.0001, "cost should be ~ $19.50");

    let history = get_cost_history(30).expect("history must read");
    assert!(!history.is_empty(), "should have at least today's entry");
    assert!(
        history.iter().any(|entry| (entry.usd_cost - 19.5).abs() < 0.0001),
        "today's entry should reflect accumulated cost"
    );
}
