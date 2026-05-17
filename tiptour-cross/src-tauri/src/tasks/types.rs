// Task types for the kanban subsystem. Mirrors the macOS Swift app's
// philosophy of small, ergonomic value types with explicit field names.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Backlog,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub created_at_unix_seconds: i64,
    #[serde(default)]
    pub started_at_unix_seconds: Option<i64>,
    #[serde(default)]
    pub completed_at_unix_seconds: Option<i64>,
    #[serde(default)]
    pub assigned_subagent_id: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub assigned_subagent_id: Option<String>,
    pub tags: Option<Vec<String>>,
}
