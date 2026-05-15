// Round-trip integration test for the OS keychain commands.
//
// IGNORED in CI by default. The `keyring` crate on Linux backs onto
// the Secret Service over D-Bus; CI runners and most container
// sandboxes don't expose one, which makes the test deterministically
// fail with "platform secret store not available". On a real
// developer machine (mac/win/Linux with gnome-keyring or kwallet
// running) the test rounds the value through and back.
//
// NOTE: we don't unset/clear the production keychain entry here —
// the test calls `set_api_key` which would clobber a developer's
// real Gemini key. The `#[ignore]` is partly for that reason too;
// run it explicitly with `cargo test -- --ignored
// keychain_round_trip` only when you understand the side effect.

use tiptour_test_support::keychain;

#[test]
#[ignore = "needs OS keychain (Secret Service / Keychain.app / Credential Manager) and would clobber the developer's real Gemini key"]
fn keychain_round_trip_persists_then_reads_back_the_same_string() {
    let probe_value = format!("test-key-{}", std::process::id());
    keychain::set_api_key(probe_value.clone()).expect("set_api_key must succeed");
    let read_back = keychain::get_api_key().expect("get_api_key must succeed");
    assert_eq!(read_back.as_deref(), Some(probe_value.as_str()));
}
