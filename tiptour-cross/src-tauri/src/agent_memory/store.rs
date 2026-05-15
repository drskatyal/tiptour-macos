// Default memory backend: a JSON-on-disk store with a bag-of-words
// cosine ranker. Zero native dependencies — works on every platform out
// of the box. Good enough for personal-scale (tens of thousands of
// memories) and used whenever the `agent-memory-vector` feature is off
// or the vector backend fails to initialize.
//
// File path: `dirs::data_local_dir()/TipTour/memory.json`
// Atomic writes: tmp file + rename, like every other store in this app.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ranker::{bumped_importance, combined_score};
use super::MemoryBackend;

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const MEMORY_FILE_NAME: &str = "memory.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;
// Soft-delete sentinel: forget() zeroes importance instead of removing
// the row so an accidental "forget X" can be reversed manually. The
// ranker treats anything ≤ this threshold as deleted.
const DELETED_IMPORTANCE_THRESHOLD: f32 = 0.001;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub created_at_unix_seconds: i64,
    #[serde(default)]
    pub last_recalled_at_unix_seconds: Option<i64>,
    #[serde(default)]
    pub recall_count: i32,
    #[serde(default = "default_importance")]
    pub importance: f32,
}

fn default_importance() -> f32 {
    0.5
}

#[derive(Debug, Default, Clone)]
pub struct MemoryUpdate {
    pub key: Option<String>,
    pub value: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoryFile {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    records: Vec<MemoryRecord>,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

pub struct JsonBagOfWordsBackend {
    file_path: PathBuf,
    records: Vec<MemoryRecord>,
}

impl JsonBagOfWordsBackend {
    pub fn open_default() -> Result<Self, String> {
        let directory = default_memory_directory()?;
        fs::create_dir_all(&directory).map_err(|e| format!("create memory dir: {e}"))?;
        let file_path = directory.join(MEMORY_FILE_NAME);
        let records = if file_path.exists() {
            let raw = fs::read_to_string(&file_path).map_err(|e| format!("read memory: {e}"))?;
            let parsed: MemoryFile =
                serde_json::from_str(&raw).map_err(|e| format!("parse memory: {e}"))?;
            parsed.records
        } else {
            Vec::new()
        };
        Ok(Self { file_path, records })
    }

    fn persist(&self) -> Result<(), String> {
        let file = MemoryFile {
            schema_version: CURRENT_SCHEMA_VERSION,
            records: self.records.clone(),
        };
        let serialized = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("serialize memory: {e}"))?;
        let temporary_file_path = self.file_path.with_extension("json.tmp");
        {
            let mut handle = fs::File::create(&temporary_file_path)
                .map_err(|e| format!("create tmp memory file: {e}"))?;
            handle
                .write_all(serialized.as_bytes())
                .map_err(|e| format!("write tmp memory file: {e}"))?;
            handle
                .sync_all()
                .map_err(|e| format!("fsync tmp memory file: {e}"))?;
        }
        fs::rename(&temporary_file_path, &self.file_path)
            .map_err(|e| format!("rename tmp memory file: {e}"))?;
        Ok(())
    }
}

pub fn default_memory_directory() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir().ok_or("no data_local_dir on this OS")?;
    Ok(base.join(ROOT_DIRECTORY_NAME))
}

impl MemoryBackend for JsonBagOfWordsBackend {
    fn remember(
        &mut self,
        key: &str,
        value: &str,
        tags: &[String],
        source: Option<&str>,
    ) -> Result<MemoryRecord, String> {
        // If a memory with the same key exists and isn't soft-deleted,
        // update it in place. Saves the agent from accumulating dupes
        // when it re-asserts a fact each session.
        let now_unix_seconds = Utc::now().timestamp();
        if let Some(existing_index) = self
            .records
            .iter()
            .position(|r| r.key == key && r.importance > DELETED_IMPORTANCE_THRESHOLD)
        {
            let existing = &mut self.records[existing_index];
            existing.value = value.to_string();
            existing.tags = tags.to_vec();
            existing.source = source.map(|s| s.to_string()).or(existing.source.clone());
            existing.importance = bumped_importance(existing.importance);
            let copy = existing.clone();
            self.persist()?;
            return Ok(copy);
        }
        let new_record = MemoryRecord {
            id: Uuid::new_v4().to_string(),
            key: key.to_string(),
            value: value.to_string(),
            tags: tags.to_vec(),
            source: source.map(|s| s.to_string()),
            created_at_unix_seconds: now_unix_seconds,
            last_recalled_at_unix_seconds: None,
            recall_count: 0,
            importance: default_importance(),
        };
        self.records.push(new_record.clone());
        self.persist()?;
        Ok(new_record)
    }

    fn recall(&mut self, query: &str, top_k: usize) -> Result<Vec<MemoryRecord>, String> {
        let query_token_vector = bag_of_words_vector(query);
        if query_token_vector.is_empty() {
            return Ok(Vec::new());
        }
        let mut scored_hits: Vec<(f32, usize)> = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, record)| record.importance > DELETED_IMPORTANCE_THRESHOLD)
            .map(|(index, record)| {
                let document_vector =
                    bag_of_words_vector(&format!("{} {}", record.key, record.value));
                let similarity = cosine_similarity(&query_token_vector, &document_vector);
                let score =
                    combined_score(similarity, record.importance, record.created_at_unix_seconds);
                (score, index)
            })
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored_hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored_hits.truncate(top_k);

        let now_unix_seconds = Utc::now().timestamp();
        let mut result_records = Vec::with_capacity(scored_hits.len());
        for (_, index) in &scored_hits {
            let record = &mut self.records[*index];
            record.recall_count += 1;
            record.last_recalled_at_unix_seconds = Some(now_unix_seconds);
            record.importance = bumped_importance(record.importance);
            result_records.push(record.clone());
        }
        if !scored_hits.is_empty() {
            self.persist()?;
        }
        Ok(result_records)
    }

    fn forget(&mut self, id: &str) -> Result<(), String> {
        let mut found = false;
        for record in self.records.iter_mut() {
            if record.id == id {
                record.importance = 0.0;
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("no memory with id {id}"));
        }
        self.persist()?;
        Ok(())
    }

    fn list_memories(&self, tag_filter: Option<&str>) -> Result<Vec<MemoryRecord>, String> {
        let mut filtered: Vec<MemoryRecord> = self
            .records
            .iter()
            .filter(|r| r.importance > DELETED_IMPORTANCE_THRESHOLD)
            .filter(|r| match tag_filter {
                Some(tag) => r.tags.iter().any(|t| t == tag),
                None => true,
            })
            .cloned()
            .collect();
        // Stable ordering: most-recently-created first so the Settings
        // table doesn't shuffle on every load.
        filtered.sort_by(|a, b| b.created_at_unix_seconds.cmp(&a.created_at_unix_seconds));
        Ok(filtered)
    }

    fn update_memory(
        &mut self,
        id: &str,
        fields: MemoryUpdate,
    ) -> Result<MemoryRecord, String> {
        let target_index = self
            .records
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("no memory with id {id}"))?;
        let record = &mut self.records[target_index];
        if let Some(new_key) = fields.key {
            record.key = new_key;
        }
        if let Some(new_value) = fields.value {
            record.value = new_value;
        }
        if let Some(new_tags) = fields.tags {
            record.tags = new_tags;
        }
        let updated = record.clone();
        self.persist()?;
        Ok(updated)
    }

    fn list_top_importance(&self, limit: usize) -> Result<Vec<MemoryRecord>, String> {
        let mut alive: Vec<MemoryRecord> = self
            .records
            .iter()
            .filter(|r| r.importance > DELETED_IMPORTANCE_THRESHOLD)
            .cloned()
            .collect();
        alive.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(std::cmp::Ordering::Equal));
        alive.truncate(limit);
        Ok(alive)
    }
}

// Simple bag-of-words tokenizer: lowercase, split on non-alphanumerics,
// drop tokens shorter than 2 chars (kills articles and most filler
// without a stopword list). Counts duplicates so "meeting meeting"
// scores higher than "meeting" on a "meeting" query.
fn bag_of_words_vector(text: &str) -> HashMap<String, f32> {
    let mut counts: HashMap<String, f32> = HashMap::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            if current.len() >= 2 {
                *counts.entry(current.clone()).or_insert(0.0) += 1.0;
            }
            current.clear();
        }
    }
    if !current.is_empty() && current.len() >= 2 {
        *counts.entry(current).or_insert(0.0) += 1.0;
    }
    counts
}

fn cosine_similarity(a: &HashMap<String, f32>, b: &HashMap<String, f32>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot_product = 0.0_f32;
    for (token, weight_in_a) in a {
        if let Some(weight_in_b) = b.get(token) {
            dot_product += weight_in_a * weight_in_b;
        }
    }
    let magnitude_a = a.values().map(|v| v * v).sum::<f32>().sqrt();
    let magnitude_b = b.values().map(|v| v * v).sum::<f32>().sqrt();
    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }
    dot_product / (magnitude_a * magnitude_b)
}
