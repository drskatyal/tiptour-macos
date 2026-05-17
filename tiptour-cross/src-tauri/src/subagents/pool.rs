// Concurrency-bounded sub-agent pool. Caps concurrent Running agents at
// `MAX_CONCURRENT_RUNNING` to keep token-spend predictable and to keep
// API rate-limits within Gemini Live's per-key quota. Pending sub-agents
// queue FIFO until a slot frees.

use chrono::Utc;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use super::types::{Subagent, SubagentStatus};

pub const MAX_CONCURRENT_RUNNING: usize = 3;
pub const MAX_SUBAGENT_DEPTH: u32 = 2;
pub const HEARTBEAT_TIMEOUT_SECONDS: i64 = 5 * 60;
pub const DEFAULT_TOKEN_BUDGET_USD: f32 = 0.50;

pub struct SubagentPool {
    pub subagents: Vec<Subagent>,
    pub pending_queue: VecDeque<String>,
}

impl SubagentPool {
    pub fn new() -> Self {
        Self {
            subagents: Vec::new(),
            pending_queue: VecDeque::new(),
        }
    }

    pub fn currently_running_count(&self) -> usize {
        self.subagents
            .iter()
            .filter(|s| s.status == SubagentStatus::Running)
            .count()
    }

    pub fn find_by_id(&self, id: &str) -> Option<&Subagent> {
        self.subagents.iter().find(|s| s.id == id)
    }

    pub fn find_by_id_mut(&mut self, id: &str) -> Option<&mut Subagent> {
        self.subagents.iter_mut().find(|s| s.id == id)
    }

    pub fn promote_pending_if_room(&mut self) -> Option<String> {
        if self.currently_running_count() >= MAX_CONCURRENT_RUNNING {
            return None;
        }
        let next_id = self.pending_queue.pop_front()?;
        let now = Utc::now().timestamp();
        if let Some(subagent_mut) = self.find_by_id_mut(&next_id) {
            subagent_mut.status = SubagentStatus::Running;
            subagent_mut.started_at_unix_seconds = now;
            subagent_mut.last_heartbeat_unix_seconds = now;
            return Some(next_id);
        }
        None
    }
}

pub static SUBAGENT_POOL: Lazy<Mutex<SubagentPool>> = Lazy::new(|| Mutex::new(SubagentPool::new()));

pub fn subagent_traces_directory() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir().ok_or("no data_local_dir on this OS")?;
    Ok(base.join("TipTour").join("subagent_traces"))
}
