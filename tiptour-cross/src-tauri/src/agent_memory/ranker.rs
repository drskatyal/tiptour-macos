// Re-ranks vector-store top-K hits by combining raw similarity with
// importance and recency decay. Importance encodes "the user/agent
// reinforced this fact" (it bumps each time the memory is recalled),
// and recency_decay is `exp(-age_days / 30)` so things the agent
// learned six months ago bubble down unless they keep getting recalled.
//
// Lives separately from `store.rs` so both backends (JSON+BoW and
// LanceDB) share one ranker.

use chrono::Utc;

pub const IMPORTANCE_RECALL_BUMP: f32 = 0.05;
pub const IMPORTANCE_CAP: f32 = 1.0;
pub const RECENCY_HALF_LIFE_DAYS: f32 = 30.0;

pub fn recency_decay_score(created_at_unix_seconds: i64) -> f32 {
    let now_unix_seconds = Utc::now().timestamp();
    let age_seconds = (now_unix_seconds - created_at_unix_seconds).max(0) as f32;
    let age_days = age_seconds / 86_400.0;
    (-age_days / RECENCY_HALF_LIFE_DAYS).exp()
}

pub fn combined_score(similarity: f32, importance: f32, created_at_unix_seconds: i64) -> f32 {
    let importance_clamped = importance.clamp(0.0, IMPORTANCE_CAP);
    let recency = recency_decay_score(created_at_unix_seconds);
    similarity * importance_clamped.max(0.1) * recency
}

pub fn bumped_importance(current_importance: f32) -> f32 {
    (current_importance + IMPORTANCE_RECALL_BUMP).min(IMPORTANCE_CAP)
}
