// Kanban task store round-trip.
//
// One test function in this file covers create → list → update_status
// (column transitions) → delete. We deliberately keep a single test
// because the production module wires its store behind a `once_cell::
// Lazy<Mutex<TaskStore>>` — once initialized, it sticks for the
// lifetime of the test binary, so two parallel tests would clobber
// each other's state. Cargo gives every `tests/*.rs` file its own
// binary; isolating per-file is the cleanest way to keep the
// production code untouched while still asserting a meaningful
// happy path.

use tiptour_test_support::tasks::{
    self, TaskPriority, TaskStatus,
};

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn task_lifecycle_create_update_status_list_then_delete_round_trips_correctly() {
    // Set the env BEFORE the first call to anything in `tasks::*`,
    // since `TASK_STORE` is a Lazy that captures the path on first
    // access. If we set this after a previous call, isolation is gone.
    let _tempdir = redirect_data_local_dir_to_tempdir();

    let created = tasks::create_task(
        "Wire the kanban end-to-end".to_string(),
        Some("test description".to_string()),
        Some(TaskPriority::High),
        None,
        Some(vec!["test-tag".to_string()]),
    )
    .expect("create_task must succeed");
    assert_eq!(created.status, TaskStatus::Backlog);
    assert_eq!(created.priority, TaskPriority::High);
    assert_eq!(created.tags, vec!["test-tag".to_string()]);

    // List should include exactly the one we created.
    let initial_listing = tasks::list_tasks(None, None).expect("list_tasks must succeed");
    assert_eq!(initial_listing.len(), 1);
    assert_eq!(initial_listing[0].id, created.id);

    // Move it through the kanban columns. The store stamps lifecycle
    // timestamps as we go — assert those too.
    let in_progress = tasks::update_task_status(created.id.clone(), TaskStatus::InProgress)
        .expect("flip to InProgress must succeed");
    assert_eq!(in_progress.status, TaskStatus::InProgress);
    assert!(
        in_progress.started_at_unix_seconds.is_some(),
        "InProgress must stamp started_at_unix_seconds the first time"
    );
    assert!(in_progress.completed_at_unix_seconds.is_none());

    let done = tasks::update_task_status(created.id.clone(), TaskStatus::Done)
        .expect("flip to Done must succeed");
    assert_eq!(done.status, TaskStatus::Done);
    assert!(done.completed_at_unix_seconds.is_some());

    // Filtered listing — Done column only.
    let done_only =
        tasks::list_tasks(Some(TaskStatus::Done), None).expect("list_tasks Done must succeed");
    assert_eq!(done_only.len(), 1);

    // Tag filter hit + miss.
    let tag_hit = tasks::list_tasks(None, Some("test-tag".to_string()))
        .expect("list_tasks tag hit must succeed");
    assert_eq!(tag_hit.len(), 1);
    let tag_miss = tasks::list_tasks(None, Some("nonexistent".to_string()))
        .expect("list_tasks tag miss must succeed");
    assert_eq!(tag_miss.len(), 0);

    // Delete + verify gone.
    tasks::delete_task(created.id.clone()).expect("delete_task must succeed");
    let post_delete = tasks::list_tasks(None, None).expect("list after delete must succeed");
    assert_eq!(post_delete.len(), 0);
    // Re-deleting must error rather than silently succeed.
    let re_delete_result = tasks::delete_task(created.id.clone());
    assert!(re_delete_result.is_err(), "deleting twice must surface an error");
}

// TODO: dispatch_task_to_subagent test is not included because it
// requires a Tauri AppHandle and the subagents pool. That command
// needs `tauri::test::mock_app()` plus a stubbed sub-agent runtime,
// which is the next milestone for this harness.
#[test]
#[ignore = "dispatch_task_to_subagent needs a real AppHandle + subagent pool"]
fn dispatch_task_to_subagent_emits_progress_events() {
    // intentionally empty — see the TODO above.
}
