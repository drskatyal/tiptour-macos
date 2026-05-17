// Kanban task subsystem. CRUD over a small file-backed store, plus a
// "dispatch this task to a fresh sub-agent" command that wires into the
// `subagents` module.
//
// Gemini gets a narrow read+write surface: it can create tasks, flip
// their status, and list them. Deletion is intentionally user-only so a
// runaway agent can't wipe the user's board.

mod store;
mod types;

use chrono::Utc;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use tauri::AppHandle;
use uuid::Uuid;

pub use types::{Task, TaskPriority, TaskStatus, TaskUpdate};

static TASK_STORE: Lazy<Mutex<store::TaskStore>> = Lazy::new(|| {
    Mutex::new(
        store::TaskStore::open_default()
            .expect("[tasks] task store init must succeed — local FS unwritable"),
    )
});

fn lock_store() -> Result<std::sync::MutexGuard<'static, store::TaskStore>, String> {
    TASK_STORE
        .lock()
        .map_err(|_| "task store mutex poisoned".to_string())
}

#[tauri::command]
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

#[tauri::command]
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
        // Stamp lifecycle timestamps as the task moves through columns
        // so the UI can show "started 3m ago" / "done 1h ago" without
        // tracking the transitions externally.
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

#[tauri::command]
pub fn update_task(id: String, fields: TaskUpdate) -> Result<Task, String> {
    let mut guard = lock_store()?;
    let task_index = guard
        .tasks()
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| format!("no task with id {id}"))?;
    {
        let task_mut = &mut guard.tasks_mut()[task_index];
        if let Some(new_title) = fields.title {
            task_mut.title = new_title;
        }
        if let Some(new_description) = fields.description {
            task_mut.description = new_description;
        }
        if let Some(new_status) = fields.status {
            task_mut.status = new_status;
        }
        if let Some(new_priority) = fields.priority {
            task_mut.priority = new_priority;
        }
        if let Some(new_assigned) = fields.assigned_subagent_id {
            task_mut.assigned_subagent_id = Some(new_assigned);
        }
        if let Some(new_tags) = fields.tags {
            task_mut.tags = new_tags;
        }
    }
    let updated = guard.tasks()[task_index].clone();
    guard.persist()?;
    Ok(updated)
}

#[tauri::command]
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

#[tauri::command]
pub fn list_tasks(
    status_filter: Option<TaskStatus>,
    tag_filter: Option<String>,
) -> Result<Vec<Task>, String> {
    let guard = lock_store()?;
    let mut tasks: Vec<Task> = guard
        .tasks()
        .iter()
        .filter(|t| status_filter.map_or(true, |s| t.status == s))
        .filter(|t| match &tag_filter {
            Some(tag) => t.tags.iter().any(|task_tag| task_tag == tag),
            None => true,
        })
        .cloned()
        .collect();
    // Newest-first for stable display in the kanban columns.
    tasks.sort_by(|a, b| b.created_at_unix_seconds.cmp(&a.created_at_unix_seconds));
    Ok(tasks)
}

#[tauri::command]
pub fn count_tasks_in_progress() -> Result<usize, String> {
    let guard = lock_store()?;
    Ok(guard
        .tasks()
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress)
        .count())
}

#[tauri::command]
pub fn dispatch_task_to_subagent(
    app: AppHandle,
    task_id: String,
    system_prompt_override: Option<String>,
) -> Result<String, String> {
    // Lookup the task first so we can build the agent task description
    // from its title + description, then mark it In Progress.
    let task_snapshot = {
        let guard = lock_store()?;
        guard
            .tasks()
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
            .ok_or_else(|| format!("no task with id {task_id}"))?
    };
    let combined_task_description = if task_snapshot.description.is_empty() {
        task_snapshot.title.clone()
    } else {
        format!("{}\n\n{}", task_snapshot.title, task_snapshot.description)
    };
    let subagent_id = crate::subagents::spawn_subagent(
        app,
        format!("Task: {}", task_snapshot.title),
        combined_task_description,
        None,
        system_prompt_override,
        None,
    )?;
    // Link the task to the new subagent and flip to InProgress.
    {
        let mut guard = lock_store()?;
        let task_index = guard
            .tasks()
            .iter()
            .position(|t| t.id == task_id)
            .ok_or_else(|| format!("no task with id {task_id}"))?;
        let now = Utc::now().timestamp();
        let task_mut = &mut guard.tasks_mut()[task_index];
        task_mut.assigned_subagent_id = Some(subagent_id.clone());
        task_mut.status = TaskStatus::InProgress;
        if task_mut.started_at_unix_seconds.is_none() {
            task_mut.started_at_unix_seconds = Some(now);
        }
        guard.persist()?;
    }
    Ok(subagent_id)
}
