// Bug-report exporter. Bundles non-sensitive diagnostic context into a
// zip the user can attach to a GitHub issue or email. Strict allowlist:
// app metadata, redacted settings, recent log lines, on-disk counts.
// We deliberately omit memory contents, recordings, traces, and the
// API key — none of those help triage and all of them are sensitive.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use once_cell::sync::Lazy;
use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const BUG_REPORT_SUBDIRECTORY_NAME: &str = "bug_reports";
const RING_BUFFER_CAPACITY: usize = 200;

static LOG_RING_BUFFER: Lazy<Mutex<VecDeque<String>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(RING_BUFFER_CAPACITY)));

/// Append a single line to the in-memory ring buffer. Modules that
/// already use `eprintln!` can mirror the same string here when they
/// want it captured by the bug-report exporter.
#[allow(dead_code)]
pub fn append_log_line(line: impl Into<String>) {
    if let Ok(mut guard) = LOG_RING_BUFFER.lock() {
        if guard.len() >= RING_BUFFER_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(line.into());
    }
}

fn snapshot_recent_logs() -> Vec<String> {
    LOG_RING_BUFFER
        .lock()
        .map(|guard| guard.iter().cloned().collect())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
struct BugReportMeta {
    app_version: String,
    target_os: String,
    target_arch: String,
    build_date: String,
    git_commit: Option<String>,
    exported_at_utc: String,
}

fn data_dir() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    Some(path)
}

fn read_settings_redacted() -> String {
    // Read the user's settings.json (if present) and ensure no key field
    // ever lands in the report. Today's settings file doesn't carry the
    // API key (it lives in the keychain) but we redact defensively.
    let mut path = match data_dir() {
        Some(path) => path,
        None => return "{}".to_string(),
    };
    path.push("settings.json");
    let raw_contents = fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value =
        serde_json::from_str(&raw_contents).unwrap_or_else(|_| serde_json::json!({}));
    redact_keylike_fields(&mut parsed);
    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| "{}".to_string())
}

fn redact_keylike_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key_name, child_value) in map.iter_mut() {
                let key_lower = key_name.to_ascii_lowercase();
                let looks_sensitive = key_lower.contains("key")
                    || key_lower.contains("secret")
                    || key_lower.contains("token")
                    || key_lower.contains("password");
                if looks_sensitive {
                    *child_value = serde_json::Value::String("<redacted>".to_string());
                } else {
                    redact_keylike_fields(child_value);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_keylike_fields(item);
            }
        }
        _ => {}
    }
}

fn count_lines_or_entries_at(path: &PathBuf, json_array_field: &str) -> usize {
    let Ok(contents) = fs::read_to_string(path) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return 0;
    };
    parsed
        .get(json_array_field)
        .and_then(|value| value.as_array())
        .map(|array| array.len())
        .unwrap_or(0)
}

fn count_tasks() -> usize {
    let Some(mut path) = data_dir() else { return 0 };
    path.push("tasks.json");
    count_lines_or_entries_at(&path, "tasks")
}

fn count_flows() -> usize {
    let Some(mut path) = data_dir() else { return 0 };
    path.push("multiflow");
    path.push("flows.json");
    count_lines_or_entries_at(&path, "entries")
}

fn count_memories() -> usize {
    // The JSON-backed agent memory store names this file `memories.json`
    // historically; we tolerate either layout.
    let Some(base) = data_dir() else { return 0 };
    for candidate_name in ["agent_memory.json", "memories.json"] {
        let mut candidate_path = base.clone();
        candidate_path.push(candidate_name);
        if candidate_path.exists() {
            return count_lines_or_entries_at(&candidate_path, "memories");
        }
    }
    0
}

#[tauri::command]
pub fn export_bug_report() -> Result<String, String> {
    let target_dir = data_dir().ok_or_else(|| "no data dir".to_string())?;
    let mut out_dir = target_dir.clone();
    out_dir.push(BUG_REPORT_SUBDIRECTORY_NAME);
    fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;

    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let zip_path = out_dir.join(format!("tiptour-bug-report-{timestamp}.zip"));

    let zip_file = fs::File::create(&zip_path).map_err(|error| error.to_string())?;
    let mut zip_writer = ZipWriter::new(zip_file);
    let zip_options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let meta = BugReportMeta {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        target_os: std::env::consts::OS.to_string(),
        target_arch: std::env::consts::ARCH.to_string(),
        build_date: option_env!("TIPTOUR_BUILD_DATE")
            .unwrap_or("unknown")
            .to_string(),
        git_commit: option_env!("GIT_COMMIT").map(|sha| sha.to_string()),
        exported_at_utc: Utc::now().to_rfc3339(),
    };

    zip_writer
        .start_file("meta.json", zip_options)
        .map_err(|error| error.to_string())?;
    zip_writer
        .write_all(
            serde_json::to_string_pretty(&meta)
                .unwrap_or_else(|_| "{}".to_string())
                .as_bytes(),
        )
        .map_err(|error| error.to_string())?;

    zip_writer
        .start_file("settings.json", zip_options)
        .map_err(|error| error.to_string())?;
    zip_writer
        .write_all(read_settings_redacted().as_bytes())
        .map_err(|error| error.to_string())?;

    zip_writer
        .start_file("recent_logs.txt", zip_options)
        .map_err(|error| error.to_string())?;
    let logs_blob = snapshot_recent_logs().join("\n");
    zip_writer
        .write_all(logs_blob.as_bytes())
        .map_err(|error| error.to_string())?;

    zip_writer
        .start_file("tasks_count.txt", zip_options)
        .map_err(|error| error.to_string())?;
    zip_writer
        .write_all(count_tasks().to_string().as_bytes())
        .map_err(|error| error.to_string())?;

    zip_writer
        .start_file("flows_count.txt", zip_options)
        .map_err(|error| error.to_string())?;
    zip_writer
        .write_all(count_flows().to_string().as_bytes())
        .map_err(|error| error.to_string())?;

    zip_writer
        .start_file("memories_count.txt", zip_options)
        .map_err(|error| error.to_string())?;
    zip_writer
        .write_all(count_memories().to_string().as_bytes())
        .map_err(|error| error.to_string())?;

    zip_writer.finish().map_err(|error| error.to_string())?;

    // Reveal the containing folder in the OS file browser so the user
    // can drag the zip somewhere useful.
    reveal_path_in_os_file_browser(&out_dir);

    Ok(zip_path.to_string_lossy().into_owned())
}

fn reveal_path_in_os_file_browser(path: &PathBuf) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
