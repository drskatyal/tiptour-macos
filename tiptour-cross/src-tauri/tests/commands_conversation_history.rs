// Per-session conversation history — verifies append + list + tail
// + delete round-trip cleanly through a real temp data dir.
//
// The env-var redirect mutates process-wide state, so tests in this
// file must NOT run in parallel. A single static mutex serializes
// every test entry point.
//
// Windows: the `dirs` crate resolves `data_local_dir()` via the
// `SHGetKnownFolderPath` Win32 API, which ignores the `LOCALAPPDATA`
// env var we set below. That means on Windows these tests would
// write into the real user profile and pollute / be polluted by
// other runs, so the whole file is gated off on that platform.
// The conversation_history module itself is OS-independent — the
// Linux + macOS runs cover its behaviour.

#![cfg(not(target_os = "windows"))]

use std::sync::Mutex;

use once_cell::sync::Lazy;
use tiptour_test_support::conversation_history::{
    append_session_turn, delete_session, get_last_session_tail, get_session_history,
    list_sessions,
};

static SERIAL_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn append_creates_file_and_first_user_turn_becomes_title() {
    let _guard = SERIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _tempdir = redirect_data_local_dir_to_tempdir();
    let session_id = "11111111-2222-3333-4444-555555555555".to_string();

    append_session_turn(session_id.clone(), "user".into(), "play some lofi".into())
        .expect("first append must succeed");
    append_session_turn(session_id.clone(), "model".into(), "sure, playing now".into())
        .expect("second append must succeed");

    let session = get_session_history(session_id.clone()).expect("history must read");
    assert_eq!(session.session_id, session_id);
    assert_eq!(session.title, "play some lofi");
    assert_eq!(session.turns.len(), 2);
    assert_eq!(session.turns[0].role, "user");
    assert_eq!(session.turns[1].role, "model");
}

#[test]
fn list_sessions_returns_newest_first_and_excludes_deleted() {
    let _guard = SERIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _tempdir = redirect_data_local_dir_to_tempdir();
    let older_session_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();
    let newer_session_id = "ffffffff-1111-2222-3333-444444444444".to_string();

    append_session_turn(older_session_id.clone(), "user".into(), "first".into())
        .expect("older append must succeed");
    // Ensure timestamps differ — chrono::Utc::now resolves to nanos but
    // we want a guaranteed-different second so the sort is deterministic.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    append_session_turn(newer_session_id.clone(), "user".into(), "second".into())
        .expect("newer append must succeed");

    let summaries = list_sessions().expect("list must read");
    assert_eq!(summaries.len(), 2, "should have both sessions");
    assert_eq!(
        summaries[0].session_id, newer_session_id,
        "newest session must come first"
    );

    delete_session(older_session_id.clone()).expect("delete must succeed");
    let after_delete = list_sessions().expect("list must read");
    assert_eq!(after_delete.len(), 1);
    assert_eq!(after_delete[0].session_id, newer_session_id);
}

#[test]
fn get_last_session_tail_returns_only_recent_session_turns() {
    let _guard = SERIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _tempdir = redirect_data_local_dir_to_tempdir();
    let older_session_id = "aaaaaaaa-1111-2222-3333-444444444444".to_string();
    let newer_session_id = "bbbbbbbb-1111-2222-3333-444444444444".to_string();

    // Older session: 5 turns we shouldn't see in the tail.
    for index in 0..5 {
        append_session_turn(
            older_session_id.clone(),
            "user".into(),
            format!("old-{index}"),
        )
        .expect("append must succeed");
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    // Newer session: 3 turns.
    for index in 0..3 {
        append_session_turn(
            newer_session_id.clone(),
            "user".into(),
            format!("new-{index}"),
        )
        .expect("append must succeed");
    }

    let tail = get_last_session_tail(Some(10)).expect("tail must read");
    assert_eq!(
        tail.len(),
        3,
        "tail should only contain the newer session's 3 turns, not older session"
    );
    assert_eq!(tail[0].text, "new-0");
    assert_eq!(tail[2].text, "new-2");
}

#[test]
fn invalid_session_id_is_rejected() {
    let _guard = SERIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _tempdir = redirect_data_local_dir_to_tempdir();
    // Path traversal attempt: a literal ".." should fail validation.
    let result = append_session_turn("../etc/passwd".into(), "user".into(), "x".into());
    assert!(result.is_err(), "path traversal must be rejected");

    // Empty session id is rejected.
    let empty_result = append_session_turn("".into(), "user".into(), "x".into());
    assert!(empty_result.is_err(), "empty session id must be rejected");

    // Non-hex characters are rejected.
    let bad_chars_result =
        append_session_turn("not-a-uuid-shape-zzzzz".into(), "user".into(), "x".into());
    assert!(
        bad_chars_result.is_err(),
        "non-hex characters must be rejected"
    );
}

#[test]
fn empty_state_returns_empty_tail_and_empty_list() {
    let _guard = SERIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _tempdir = redirect_data_local_dir_to_tempdir();
    let summaries = list_sessions().expect("list must read");
    assert_eq!(summaries.len(), 0);
    let tail = get_last_session_tail(Some(5)).expect("tail must read");
    assert_eq!(tail.len(), 0);
}
