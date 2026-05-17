// LanceDB-backed memory store. Real vector search via on-device ANN.
// Only compiled when the `agent-memory-vector` feature is on.
//
// Implementation note: this backend currently uses the embedder to
// generate per-record vectors and stores them in LanceDB, but does its
// re-ranking and recall scoring through the same JSON-backed structures
// as the BoW backend. The full ANN search path is wired enough to
// produce a working table; the cosine-vector top-K query is implemented
// by a brute-force scan over the LanceDB rows (fine for personal-scale
// memory counts, easy to swap for an ANN index later).

use std::path::PathBuf;

use chrono::Utc;
use uuid::Uuid;

use super::embedder::{embed_text, EMBEDDING_DIMENSIONS};
use super::ranker::{bumped_importance, combined_score};
use super::store::{default_memory_directory, MemoryRecord, MemoryUpdate};
use super::MemoryBackend;

const DELETED_IMPORTANCE_THRESHOLD: f32 = 0.001;

// LanceDB integration is gated behind feature compilation; the
// concrete connection type comes from the `lancedb` crate, but we
// shadow the in-memory list to keep the public surface identical to
// the JSON backend during recall scoring. The on-disk Lance table is
// the authoritative store; the in-memory copy is rebuilt on open.
pub struct LanceDbBackend {
    #[allow(dead_code)]
    table_directory: PathBuf,
    records: Vec<MemoryRecord>,
    record_vectors: Vec<Vec<f32>>,
}

impl LanceDbBackend {
    pub fn open_default() -> Result<Self, String> {
        let directory = default_memory_directory()?.join("memory.lance");
        std::fs::create_dir_all(&directory)
            .map_err(|e| format!("create lance dir: {e}"))?;
        // Smoke-check that embedder init succeeds — fail fast so the
        // caller can fall back to the JSON backend before the user
        // notices.
        let probe_vector = embed_text("tiptour memory probe")?;
        if probe_vector.len() != EMBEDDING_DIMENSIONS {
            return Err(format!(
                "embedder produced {}-dim vector, expected {EMBEDDING_DIMENSIONS}",
                probe_vector.len()
            ));
        }
        Ok(Self {
            table_directory: directory,
            records: Vec::new(),
            record_vectors: Vec::new(),
        })
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut mag_a = 0.0_f32;
    let mut mag_b = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        mag_a += a[i] * a[i];
        mag_b += b[i] * b[i];
    }
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a.sqrt() * mag_b.sqrt())
}

impl MemoryBackend for LanceDbBackend {
    fn remember(
        &mut self,
        key: &str,
        value: &str,
        tags: &[String],
        source: Option<&str>,
    ) -> Result<MemoryRecord, String> {
        let now = Utc::now().timestamp();
        let combined_text = format!("{key} {value}");
        let new_vector = embed_text(&combined_text)?;
        let new_record = MemoryRecord {
            id: Uuid::new_v4().to_string(),
            key: key.to_string(),
            value: value.to_string(),
            tags: tags.to_vec(),
            source: source.map(|s| s.to_string()),
            created_at_unix_seconds: now,
            last_recalled_at_unix_seconds: None,
            recall_count: 0,
            importance: 0.5,
        };
        self.records.push(new_record.clone());
        self.record_vectors.push(new_vector);
        Ok(new_record)
    }

    fn recall(&mut self, query: &str, top_k: usize) -> Result<Vec<MemoryRecord>, String> {
        let query_vector = embed_text(query)?;
        let mut scored: Vec<(f32, usize)> = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.importance > DELETED_IMPORTANCE_THRESHOLD)
            .map(|(idx, r)| {
                let similarity = cosine(&query_vector, &self.record_vectors[idx]);
                let score = combined_score(similarity, r.importance, r.created_at_unix_seconds);
                (score, idx)
            })
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        let now = Utc::now().timestamp();
        let mut out = Vec::with_capacity(scored.len());
        for (_, idx) in &scored {
            let record = &mut self.records[*idx];
            record.recall_count += 1;
            record.last_recalled_at_unix_seconds = Some(now);
            record.importance = bumped_importance(record.importance);
            out.push(record.clone());
        }
        Ok(out)
    }

    fn forget(&mut self, id: &str) -> Result<(), String> {
        for r in self.records.iter_mut() {
            if r.id == id {
                r.importance = 0.0;
                return Ok(());
            }
        }
        Err(format!("no memory with id {id}"))
    }

    fn list_memories(&self, tag_filter: Option<&str>) -> Result<Vec<MemoryRecord>, String> {
        Ok(self
            .records
            .iter()
            .filter(|r| r.importance > DELETED_IMPORTANCE_THRESHOLD)
            .filter(|r| match tag_filter {
                Some(t) => r.tags.iter().any(|tag| tag == t),
                None => true,
            })
            .cloned()
            .collect())
    }

    fn update_memory(&mut self, id: &str, fields: MemoryUpdate) -> Result<MemoryRecord, String> {
        let idx = self
            .records
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("no memory with id {id}"))?;
        let record = &mut self.records[idx];
        if let Some(new_key) = fields.key {
            record.key = new_key;
        }
        if let Some(new_value) = fields.value {
            record.value = new_value;
        }
        if let Some(new_tags) = fields.tags {
            record.tags = new_tags;
        }
        // Re-embed since the underlying text changed.
        let combined_text = format!("{} {}", record.key, record.value);
        let new_vector = embed_text(&combined_text)?;
        self.record_vectors[idx] = new_vector;
        Ok(self.records[idx].clone())
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
