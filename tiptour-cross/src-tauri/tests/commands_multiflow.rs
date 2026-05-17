// Multiflow integration tests.
//
// All tests in this file are `#[ignore]`'d — every meaningful path
// (start_recording_flow → stop_recording_flow → list_flows →
// delete_flow) wires through the recorder's GLOBAL_RECORDER, which
// in turn instantiates the OS input tap (rdev), audio capture (cpal
// for narration), and the screen capture pipeline. Those need real
// hardware that the CI sandbox does not have.
//
// We keep the test scaffolding here so future hardware-having
// developers can wire it up; the multiflow command surface itself
// (FlowSummary shape, on-disk index format, deletion of both index
// entry and demonstration directory) is exercised at the production
// code's per-module unit-test layer that ships alongside the source.

#[test]
#[ignore = "start_recording_flow / stop_recording_flow drive rdev + cpal + screencap — needs a real desktop session"]
fn start_then_stop_recording_flow_returns_a_flow_summary_with_step_count() {
    // Skeleton — see the file-level doc comment for why this is
    // intentionally empty in the headless harness.
}

#[test]
#[ignore = "list_flows + delete_flow round-trip requires a recorded demonstration on disk first"]
fn list_flows_after_delete_flow_drops_both_the_index_entry_and_the_demonstration_directory() {
    // Skeleton — same reason as above.
}
