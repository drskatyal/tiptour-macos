// Token + cost accounting. Each Gemini Live `usageMetadata` frame
// arrives with input/output token counts; we accumulate session totals
// in memory and append to a per-day rolling history on disk.
//
// Pricing assumes Gemini Live preview rates ($3/M input, $15/M output)
// — easy to swap if Google updates them. The numbers shown to the user
// are estimates only; the authoritative bill comes from the Cloud
// console.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const COST_HISTORY_FILE_NAME: &str = "cost_history.json";
const ROLLING_HISTORY_DAYS: usize = 30;

const INPUT_USD_PER_MILLION_TOKENS: f64 = 3.0;
const OUTPUT_USD_PER_MILLION_TOKENS: f64 = 15.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd_cost: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyEntry {
    /// `YYYY-MM-DD` UTC.
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd_cost: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostHistoryFile {
    pub days: Vec<DailyEntry>,
}

static SESSION_COST: Lazy<Mutex<CostSnapshot>> = Lazy::new(|| Mutex::new(CostSnapshot::default()));
static HISTORY_CACHE: Lazy<Mutex<Option<CostHistoryFile>>> = Lazy::new(|| Mutex::new(None));

fn cost_history_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(COST_HISTORY_FILE_NAME);
    Some(path)
}

fn load_history_from_disk() -> CostHistoryFile {
    let Some(path) = cost_history_path() else {
        return CostHistoryFile::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return CostHistoryFile::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

fn save_history_to_disk(history: &CostHistoryFile) -> Result<(), String> {
    let path = cost_history_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(history).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

fn with_history<R>(handler: impl FnOnce(&mut CostHistoryFile) -> R) -> R {
    let mut guard = HISTORY_CACHE.lock().expect("cost history mutex poisoned");
    if guard.is_none() {
        *guard = Some(load_history_from_disk());
    }
    handler(guard.as_mut().expect("just initialized"))
}

fn cost_for(input_tokens: u64, output_tokens: u64) -> f64 {
    let input_cost = (input_tokens as f64) * INPUT_USD_PER_MILLION_TOKENS / 1_000_000.0;
    let output_cost = (output_tokens as f64) * OUTPUT_USD_PER_MILLION_TOKENS / 1_000_000.0;
    input_cost + output_cost
}

#[tauri::command]
pub fn record_usage(input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    let added_cost = cost_for(input_tokens, output_tokens);

    {
        let mut session = SESSION_COST.lock().map_err(|_| "session mutex poisoned".to_string())?;
        session.input_tokens = session.input_tokens.saturating_add(input_tokens);
        session.output_tokens = session.output_tokens.saturating_add(output_tokens);
        session.usd_cost += added_cost;
    }

    let today_key = Utc::now().format("%Y-%m-%d").to_string();
    with_history(|file| {
        // Update or insert today's entry.
        if let Some(entry) = file.days.iter_mut().find(|e| e.date == today_key) {
            entry.input_tokens = entry.input_tokens.saturating_add(input_tokens);
            entry.output_tokens = entry.output_tokens.saturating_add(output_tokens);
            entry.usd_cost += added_cost;
        } else {
            file.days.push(DailyEntry {
                date: today_key.clone(),
                input_tokens,
                output_tokens,
                usd_cost: added_cost,
            });
        }
        // Trim to the last `ROLLING_HISTORY_DAYS` distinct dates.
        // Sort ascending by date string (YYYY-MM-DD sorts lexicographically).
        file.days.sort_by(|a, b| a.date.cmp(&b.date));
        if file.days.len() > ROLLING_HISTORY_DAYS {
            let drop_count = file.days.len() - ROLLING_HISTORY_DAYS;
            file.days.drain(0..drop_count);
        }
        save_history_to_disk(file)
    })
}

#[tauri::command]
pub fn get_session_cost() -> Result<CostSnapshot, String> {
    let session = SESSION_COST.lock().map_err(|_| "session mutex poisoned".to_string())?;
    Ok(session.clone())
}

#[tauri::command]
pub fn get_today_cost() -> Result<CostSnapshot, String> {
    let today_key = Utc::now().format("%Y-%m-%d").to_string();
    Ok(with_history(|file| {
        file.days
            .iter()
            .find(|e| e.date == today_key)
            .map(|e| CostSnapshot {
                input_tokens: e.input_tokens,
                output_tokens: e.output_tokens,
                usd_cost: e.usd_cost,
            })
            .unwrap_or_default()
    }))
}

#[tauri::command]
pub fn get_cost_history(days: usize) -> Result<Vec<DailyEntry>, String> {
    Ok(with_history(|file| {
        let mut sorted = file.days.clone();
        sorted.sort_by(|a, b| b.date.cmp(&a.date));
        sorted.into_iter().take(days).collect()
    }))
}

/// Test helper — reset the in-memory caches so tests starting from a
/// fresh tempdir don't see stale state from a previous test in the
/// same process. Not a Tauri command; only the tests link against it.
#[allow(dead_code)]
pub fn reset_in_memory_state_for_tests() {
    if let Ok(mut session) = SESSION_COST.lock() {
        *session = CostSnapshot::default();
    }
    if let Ok(mut cache) = HISTORY_CACHE.lock() {
        *cache = None;
    }
}

// Keep the unused-import suppression happy for the HashMap re-export
// some callers might expect as the module grows.
#[allow(dead_code)]
fn _suppress_unused_hashmap_warning() -> HashMap<String, u64> {
    HashMap::new()
}
