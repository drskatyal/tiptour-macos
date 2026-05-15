// Agent memory subsystem. Persistent semantic recall for TipTour's
// voice agent, exposed both to the user (Settings → Memory tab) and to
// Gemini (the `remember` / `recall` / `forget` / `list_memories` tool
// calls).
//
// Two backends live behind a single `MemoryBackend` trait:
//
//   1. `JsonBagOfWordsBackend` (default) — file-backed store with a
//      bag-of-words cosine ranker. Zero native dependencies; ships in
//      every build. Good enough for tens-of-thousands of memories on a
//      single user's machine, which is the personal-scale this feature
//      targets.
//
//   2. `LanceDbBackend` (feature `agent-memory-vector`) — LanceDB +
//      fastembed-rs (quantized MiniLM-L6-v2). Real embeddings, ANN
//      search, in-process inference. Off by default because the
//      transitive build (Arrow + ONNX Runtime) is heavy and Linux CI
//      sandboxes routinely OOM compiling it.
//
// The public Tauri command surface is identical regardless of which
// backend is wired up, so the UI and Gemini tool dispatch don't care
// which one is active.

mod ranker;
mod store;

#[cfg(feature = "agent-memory-vector")]
mod embedder;
#[cfg(feature = "agent-memory-vector")]
mod lance_backend;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub use store::{MemoryRecord, MemoryUpdate};

// We hide the concrete backend behind a Mutex<Box<dyn MemoryBackend>>
// so the `agent-memory-vector` flag can swap implementations at startup
// without leaking through to the rest of the codebase.
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
    fn update_memory(&mut self, id: &str, fields: MemoryUpdate) -> Result<MemoryRecord, String>;
    fn list_top_importance(&self, limit: usize) -> Result<Vec<MemoryRecord>, String>;
}

static MEMORY_BACKEND: Lazy<Mutex<Box<dyn MemoryBackend>>> = Lazy::new(|| {
    // Try the vector backend if compiled in; fall back to JSON+BoW on
    // any init failure so a busted ONNX download doesn't brick memory.
    #[cfg(feature = "agent-memory-vector")]
    {
        match lance_backend::LanceDbBackend::open_default() {
            Ok(backend) => return Mutex::new(Box::new(backend)),
            Err(open_error) => {
                eprintln!(
                    "[agent_memory] LanceDB backend init failed, falling back to JSON: {open_error}"
                );
            }
        }
    }
    let json_backend = store::JsonBagOfWordsBackend::open_default()
        .expect("[agent_memory] JSON backend init must succeed — local FS is unwritable");
    Mutex::new(Box::new(json_backend))
});

#[derive(Debug, Serialize, Deserialize)]
pub struct RememberArgs {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[tauri::command]
pub fn remember(
    key: String,
    value: String,
    tags: Option<Vec<String>>,
    source: Option<String>,
) -> Result<MemoryRecord, String> {
    let tag_list = tags.unwrap_or_default();
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .remember(&key, &value, &tag_list, source.as_deref())
}

#[tauri::command]
pub fn recall(query: String, top_k: Option<usize>) -> Result<Vec<MemoryRecord>, String> {
    let k = top_k.unwrap_or(5).clamp(1, 50);
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .recall(&query, k)
}

#[tauri::command]
pub fn forget(id: String) -> Result<(), String> {
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .forget(&id)
}

#[tauri::command]
pub fn list_memories(tag_filter: Option<String>) -> Result<Vec<MemoryRecord>, String> {
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .list_memories(tag_filter.as_deref())
}

#[tauri::command]
pub fn update_memory(
    id: String,
    key: Option<String>,
    value: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<MemoryRecord, String> {
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .update_memory(
            &id,
            MemoryUpdate {
                key,
                value,
                tags,
            },
        )
}

/// Session-startup helper: returns the highest-importance memories so the
/// Gemini Live session can inject them as system-instruction context.
/// Kept as a Tauri command so the TS side can call it from session open.
#[tauri::command]
pub fn list_top_importance_memories(limit: Option<usize>) -> Result<Vec<MemoryRecord>, String> {
    let limit_clamped = limit.unwrap_or(10).clamp(1, 50);
    MEMORY_BACKEND
        .lock()
        .map_err(|_| "memory mutex poisoned".to_string())?
        .list_top_importance(limit_clamped)
}
