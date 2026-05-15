// Test-support library target. Re-exports a curated subset of the
// production modules under their original module paths so the
// `tests/*.rs` integration tests can call command functions directly.
//
// This file is NOT under `src-tauri/src/` and is wired in via
// Cargo.toml's `[lib]` entry. The binary target (`src/main.rs`) is
// completely independent — both compile their own copy of these
// modules. By design: the test lib never gets linked into the
// shipped app, and it doesn't need (and shouldn't carry) Tauri
// command-handler glue.
//
// We deliberately exclude modules that pull in platform-specific
// crates (uiautomation on Windows, accessibility-sys on macOS, cpal
// audio devices, rdev input taps, screencapturekit) — those need real
// hardware and are covered by `#[ignore]`'d tests with TODOs.

#[path = "src/keychain.rs"]
pub mod keychain;

#[path = "src/app_settings.rs"]
pub mod app_settings;

#[path = "src/personas/mod.rs"]
pub mod personas;

#[path = "src/mode.rs"]
pub mod mode;

// `tasks` and `agent_memory` are inline modules that point at the
// production source dir via `#[path]` so their nested `mod types;` /
// `mod store;` declarations resolve to the real files via the same
// `<dir>/<name>.rs` rule Rust normally uses for file-backed mods.
#[path = "src/tasks"]
pub mod tasks {
    pub mod types;
    pub mod store;

    use chrono::Utc;
    use once_cell::sync::Lazy;
    use std::sync::Mutex;
    use uuid::Uuid;
    pub use types::{Task, TaskPriority, TaskStatus, TaskUpdate};

    static TASK_STORE: Lazy<Mutex<store::TaskStore>> = Lazy::new(|| {
        Mutex::new(
            store::TaskStore::open_default()
                .expect("[tasks] task store init must succeed in test"),
        )
    });

    fn lock_store() -> Result<std::sync::MutexGuard<'static, store::TaskStore>, String> {
        TASK_STORE
            .lock()
            .map_err(|_| "task store mutex poisoned".to_string())
    }

    pub fn create_task(
        title: String,
        description: Option<String>,
        priority: Option<TaskPriority>,
        parent_task_id: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Task, String> {
        let new_task = Task {
            id: Uuid::new_v4().to_string(),
            title,
            description: description.unwrap_or_default(),
            status: TaskStatus::Backlog,
            priority: priority.unwrap_or(TaskPriority::Medium),
            created_at_unix_seconds: Utc::now().timestamp(),
            started_at_unix_seconds: None,
            completed_at_unix_seconds: None,
            assigned_subagent_id: None,
            parent_task_id,
            tags: tags.unwrap_or_default(),
        };
        let mut guard = lock_store()?;
        guard.tasks_mut().push(new_task.clone());
        guard.persist()?;
        Ok(new_task)
    }

    pub fn update_task_status(id: String, status: TaskStatus) -> Result<Task, String> {
        let mut guard = lock_store()?;
        let now = Utc::now().timestamp();
        let task_index = guard
            .tasks()
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| format!("no task with id {id}"))?;
        {
            let task_mut = &mut guard.tasks_mut()[task_index];
            match status {
                TaskStatus::InProgress => {
                    if task_mut.started_at_unix_seconds.is_none() {
                        task_mut.started_at_unix_seconds = Some(now);
                    }
                }
                TaskStatus::Done | TaskStatus::Cancelled => {
                    task_mut.completed_at_unix_seconds = Some(now);
                }
                _ => {}
            }
            task_mut.status = status;
        }
        let updated = guard.tasks()[task_index].clone();
        guard.persist()?;
        Ok(updated)
    }

    pub fn delete_task(id: String) -> Result<(), String> {
        let mut guard = lock_store()?;
        let starting_len = guard.tasks().len();
        guard.tasks_mut().retain(|t| t.id != id);
        if guard.tasks().len() == starting_len {
            return Err(format!("no task with id {id}"));
        }
        guard.persist()?;
        Ok(())
    }

    pub fn list_tasks(
        status_filter: Option<TaskStatus>,
        tag_filter: Option<String>,
    ) -> Result<Vec<Task>, String> {
        let guard = lock_store()?;
        let tasks: Vec<Task> = guard
            .tasks()
            .iter()
            .filter(|t| status_filter.map_or(true, |s| t.status == s))
            .filter(|t| match &tag_filter {
                Some(tag) => t.tags.iter().any(|task_tag| task_tag == tag),
                None => true,
            })
            .cloned()
            .collect();
        Ok(tasks)
    }
}

#[path = "src/agent_memory"]
pub mod agent_memory {
    pub mod ranker;
    pub mod store;

    pub use store::{JsonBagOfWordsBackend, MemoryRecord, MemoryUpdate};

    pub trait MemoryBackend: Send + Sync {
        fn remember(
            &mut self,
            key: &str,
            value: &str,
            tags: &[String],
            source: Option<&str>,
        ) -> Result<MemoryRecord, String>;
        fn recall(&mut self, query: &str, top_k: usize) -> Result<Vec<MemoryRecord>, String>;
        fn forget(&mut self, id: &str) -> Result<(), String>;
        fn list_memories(&self, tag_filter: Option<&str>) -> Result<Vec<MemoryRecord>, String>;
        fn update_memory(
            &mut self,
            id: &str,
            fields: MemoryUpdate,
        ) -> Result<MemoryRecord, String>;
        fn list_top_importance(&self, limit: usize) -> Result<Vec<MemoryRecord>, String>;
    }
}
