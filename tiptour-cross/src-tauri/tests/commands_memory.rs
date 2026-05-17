// Agent memory JSON backend round-trip.
//
// One `#[test]` function for the whole journey because env-vars are
// process-global and Cargo runs intra-binary tests in parallel by
// default. Pulling in `serial_test` purely to split four scenarios
// would add a dev-dep we don't otherwise need.
//
// Asserts:
//   - `remember` then `list_memories` returns the row
//   - `recall` returns the row and `list_top_importance` ranks
//     bumped-importance entries first
//   - `forget` soft-deletes (importance drops below the ranker
//     threshold so list_memories hides it)
//   - `update_memory` writes value + tags in place

use tiptour_test_support::agent_memory::{
    JsonBagOfWordsBackend, MemoryBackend, MemoryUpdate,
};

fn fresh_backend() -> JsonBagOfWordsBackend {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    // Leak the tempdir so the directory survives for the full
    // backend lifetime within the test. The OS cleans up on exit.
    std::mem::forget(tempdir);
    JsonBagOfWordsBackend::open_default().expect("backend open must succeed on tempdir")
}

#[test]
fn agent_memory_full_round_trip_remember_recall_forget_list_top_update() {
    // ---- remember + list ----
    let mut backend = fresh_backend();
    let inserted = backend
        .remember(
            "user.name",
            "Ada Lovelace",
            &["identity".to_string()],
            Some("test"),
        )
        .expect("remember must succeed");
    assert_eq!(inserted.key, "user.name");
    assert_eq!(inserted.value, "Ada Lovelace");
    assert_eq!(backend.list_memories(None).unwrap().len(), 1);

    // ---- forget soft-deletes ----
    let to_forget = backend
        .remember("temp.fact", "v1", &[], None)
        .expect("second remember must succeed");
    assert_eq!(backend.list_memories(None).unwrap().len(), 2);
    backend
        .forget(&to_forget.id)
        .expect("forget must succeed");
    assert_eq!(
        backend.list_memories(None).unwrap().len(),
        1,
        "soft-delete must drop importance below the ranker visibility threshold"
    );

    // ---- recall + list_top_importance ranking ----
    backend
        .remember("project.name", "TipTour", &[], None)
        .expect("project.name remember must succeed");
    backend
        .remember("language.preferred", "Rust", &[], None)
        .expect("language.preferred remember must succeed");
    // Repeat project.name twice more — the same-key in-place path
    // bumps importance each time.
    backend
        .remember("project.name", "TipTour", &[], None)
        .expect("repeat must succeed");
    backend
        .remember("project.name", "TipTour", &[], None)
        .expect("repeat must succeed");

    let top = backend
        .list_top_importance(2)
        .expect("list_top_importance must succeed");
    assert!(!top.is_empty(), "top-importance must surface at least one row");
    assert_eq!(
        top[0].key, "project.name",
        "the most-bumped key must rank first"
    );
    let recalled = backend
        .recall("TipTour", 5)
        .expect("recall must succeed");
    assert!(
        recalled.iter().any(|row| row.key == "project.name"),
        "recall on the value text must surface the matching key"
    );

    // ---- update_memory rewrites value + tags in place ----
    let editable = backend
        .remember("editable.fact", "v1", &["original".to_string()], None)
        .expect("editable.fact must succeed");
    let updated = backend
        .update_memory(
            &editable.id,
            MemoryUpdate {
                key: None,
                value: Some("v2".to_string()),
                tags: Some(vec!["updated".to_string()]),
            },
        )
        .expect("update_memory must succeed");
    assert_eq!(updated.value, "v2");
    assert_eq!(updated.tags, vec!["updated".to_string()]);
}
