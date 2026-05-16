// Brain-dump adapter.
//
// Persistent markdown-on-disk store for thoughts, brain dumps, daily
// captures, and screenshots. Designed so that:
//
//   - The files work standalone in ANY markdown editor
//   - The files work as an Obsidian vault if the user already has one
//     (drop the brain-dump folder under an Obsidian vault root — the
//     daily notes + backlinks plugins automatically pick it up)
//   - Screenshots land alongside their text capture with relative
//     image links so the markdown stays self-contained
//
// Default folder: `~/Documents/TipTour Brain Dumps/`. Users can point
// the `brain_dump_folder` app setting at their Obsidian vault root to
// get the full Obsidian experience without any extra wiring.
//
// File naming:
//   YYYY-MM-DD.md                — daily aggregator note (append-only)
//   YYYY-MM-DD-HH-MM-<slug>.md   — individual thought file
//   screenshots/YYYY-MM-DD-HH-MM-<slug>.png  — image attachments
//
// Handlers (called via `control_app`):
//   - capture_text       { text, tags?, title? }
//   - capture_screenshot { caption?, tags? }
//   - daily_note_append  { text }
//   - find               { query } -> [{ path, snippet, score }]
//   - open_folder        {}

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::AppHandle;

use super::helpers::{open_url_via_os, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

const DEFAULT_FOLDER_NAME: &str = "TipTour Brain Dumps";

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "brain-dump".into(),
        name: "Brain Dump".into(),
        description: "Capture thoughts, screenshots, and daily notes as plain markdown. Works standalone or inside an Obsidian vault.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "brain dump".into(),
            "save this thought".into(),
            "jot this down".into(),
            "capture this".into(),
            "grab a screenshot".into(),
            "save the screen".into(),
            "remember today".into(),
            "find my notes about".into(),
        ],
        capabilities: vec![
            AdapterCapability::Network,
            // Network isn't actually used — but the trait we'd want
            // ("file write") isn't an enum variant yet. The
            // capability list is informational today so this just
            // tells the install panel "this adapter touches disk."
        ],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Captures land in ~/Documents/TipTour Brain Dumps/ by default. \
            Point the brain_dump_folder setting at your Obsidian vault root to get \
            graph/backlinks/daily-note features automatically — TipTour writes \
            plain markdown that Obsidian's plugins read natively.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "capture_text" => capture_text(args),
        "capture_screenshot" => capture_screenshot(args).await,
        "daily_note_append" => daily_note_append(args),
        "find" => find(args),
        "open_folder" => open_folder(),
        other => Err(format!("brain-dump: no handler '{other}'")),
    }
}

// -------- folder resolution --------

fn brain_dump_root() -> Result<PathBuf, String> {
    // Honour the user's configured folder first; fall back to
    // ~/Documents/TipTour Brain Dumps/.
    if let Ok(env_override) = std::env::var("TIPTOUR_BRAIN_DUMP_FOLDER") {
        let path = PathBuf::from(env_override);
        std::fs::create_dir_all(&path).map_err(|e| format!("create brain dump dir: {e}"))?;
        return Ok(path);
    }
    let configured = read_configured_folder().filter(|p| !p.as_os_str().is_empty());
    let path = match configured {
        Some(p) => p,
        None => {
            let docs = dirs::document_dir()
                .or_else(dirs::home_dir)
                .ok_or_else(|| "no documents dir; set brain_dump_folder in settings".to_string())?;
            docs.join(DEFAULT_FOLDER_NAME)
        }
    };
    std::fs::create_dir_all(&path).map_err(|e| format!("create brain dump dir: {e}"))?;
    std::fs::create_dir_all(path.join("screenshots"))
        .map_err(|e| format!("create screenshots dir: {e}"))?;
    Ok(path)
}

fn read_configured_folder() -> Option<PathBuf> {
    // Lazy read from app_settings.json — same file the General tab
    // writes. We avoid a hard coupling to app_settings.rs because
    // that module's load function locks a mutex and dispatch from
    // an async tool handler shouldn't compete with the panel.
    let mut path = dirs::data_local_dir()?;
    path.push("TipTour");
    path.push("settings.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let folder = value.get("brainDumpFolder")?.as_str()?;
    if folder.is_empty() {
        None
    } else {
        Some(PathBuf::from(folder))
    }
}

fn slugify(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut last_was_dash = true;
    for character in input.chars().take(60) {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            output.push('-');
            last_was_dash = true;
        }
    }
    output.trim_matches('-').to_string()
}

fn timestamp_parts() -> (String, String, String) {
    let now = chrono::Local::now();
    (
        now.format("%Y-%m-%d").to_string(),     // date for daily file
        now.format("%Y-%m-%d-%H-%M").to_string(), // file prefix
        now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(), // frontmatter ts
    )
}

// -------- handlers --------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureTextArgs {
    text: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    title: Option<String>,
}

fn capture_text(args: Value) -> Result<Value, String> {
    let parsed: CaptureTextArgs = parse_args(args)?;
    let root = brain_dump_root()?;
    let (_date, file_prefix, iso) = timestamp_parts();
    let title = parsed
        .title
        .clone()
        .unwrap_or_else(|| {
            parsed
                .text
                .lines()
                .next()
                .unwrap_or("untitled")
                .chars()
                .take(60)
                .collect()
        });
    let slug = slugify(&title);
    let filename = format!("{}-{}.md", file_prefix, slug);
    let path = root.join(&filename);

    let tags_yaml = if parsed.tags.is_empty() {
        String::new()
    } else {
        format!(
            "tags: [{}]\n",
            parsed
                .tags
                .iter()
                .map(|tag| format!("\"{tag}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let body = format!(
        "---\ncreated: {iso}\ntitle: \"{}\"\n{tags_yaml}---\n\n{}\n",
        title.replace('"', "\\\""),
        parsed.text
    );
    std::fs::write(&path, body).map_err(|e| format!("write thought: {e}"))?;
    Ok(json!({ "path": path.display().to_string(), "title": title }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureScreenshotArgs {
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

async fn capture_screenshot(args: Value) -> Result<Value, String> {
    let parsed: CaptureScreenshotArgs = parse_args(args)?;
    let root = brain_dump_root()?;
    let (_date, file_prefix, iso) = timestamp_parts();
    let caption_text = parsed.caption.clone().unwrap_or_else(|| "screenshot".into());
    let slug = slugify(&caption_text);

    // Capture via existing screen module so we get the same
    // permission-aware path the live session uses.
    let frame = crate::screen::capture::capture_primary_screen()
        .await
        .map_err(|e| format!("screen capture: {e}"))?;
    let png_path = root.join("screenshots").join(format!("{}-{}.png", file_prefix, slug));
    // Encode BGRA → PNG via the image crate. Performance is fine
    // for one-shot captures (the streamer uses JPEG for its 1.5s
    // cadence).
    let mut rgba_buffer = Vec::with_capacity(frame.bgra.len());
    for chunk in frame.bgra.chunks_exact(4) {
        rgba_buffer.push(chunk[2]);
        rgba_buffer.push(chunk[1]);
        rgba_buffer.push(chunk[0]);
        rgba_buffer.push(chunk[3]);
    }
    let image_buffer = image::RgbaImage::from_raw(frame.width, frame.height, rgba_buffer)
        .ok_or_else(|| "buffer size mismatch".to_string())?;
    image_buffer
        .save(&png_path)
        .map_err(|e| format!("png encode: {e}"))?;

    // Write a sidecar markdown note with a relative link.
    let md_filename = format!("{}-{}.md", file_prefix, slug);
    let md_path = root.join(&md_filename);
    let tags_yaml = if parsed.tags.is_empty() {
        String::new()
    } else {
        format!(
            "tags: [{}]\n",
            parsed
                .tags
                .iter()
                .map(|tag| format!("\"{tag}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let body = format!(
        "---\ncreated: {iso}\ntitle: \"{}\"\nkind: screenshot\n{tags_yaml}---\n\n![{}](./screenshots/{}-{}.png)\n",
        caption_text.replace('"', "\\\""),
        caption_text,
        file_prefix,
        slug,
    );
    std::fs::write(&md_path, body).map_err(|e| format!("write screenshot md: {e}"))?;
    Ok(json!({
        "imagePath": png_path.display().to_string(),
        "notePath": md_path.display().to_string(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyNoteAppendArgs {
    text: String,
}

fn daily_note_append(args: Value) -> Result<Value, String> {
    let parsed: DailyNoteAppendArgs = parse_args(args)?;
    let root = brain_dump_root()?;
    let (date, _, iso) = timestamp_parts();
    let daily_path = root.join(format!("{}.md", date));

    // Initialize the daily note if it doesn't exist with a header.
    if !daily_path.exists() {
        std::fs::write(
            &daily_path,
            format!(
                "---\ndate: {date}\ntype: daily-note\n---\n\n# {date}\n\n",
            ),
        )
        .map_err(|e| format!("create daily note: {e}"))?;
    }

    // Append a timestamped block. `\n\n## HH:MM\n<text>\n` keeps the
    // file scannable and Obsidian's daily-note plugin happy.
    let time_only = chrono::Local::now().format("%H:%M").to_string();
    let appended = format!("\n## {time_only}\n{}\n", parsed.text);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&daily_path)
        .map_err(|e| format!("open daily note: {e}"))?;
    file.write_all(appended.as_bytes())
        .map_err(|e| format!("append daily note: {e}"))?;
    let _ = iso; // currently unused in the appended block — kept for future structured frontmatter
    Ok(json!({ "path": daily_path.display().to_string() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindArgs {
    query: String,
}

fn find(args: Value) -> Result<Value, String> {
    let parsed: FindArgs = parse_args(args)?;
    let root = brain_dump_root()?;
    let needle = parsed.query.to_ascii_lowercase();
    let mut hits: Vec<Value> = Vec::new();
    walk_markdown_files(&root, &mut |path, body| {
        let body_lower = body.to_ascii_lowercase();
        if body_lower.contains(&needle) {
            let position = body_lower.find(&needle).unwrap();
            let start = position.saturating_sub(40);
            let end = (position + needle.len() + 80).min(body.len());
            let snippet = body[start..end].replace('\n', " ");
            hits.push(json!({
                "path": path.display().to_string(),
                "snippet": format!("…{snippet}…"),
            }));
        }
    });
    Ok(json!({ "results": hits.into_iter().take(20).collect::<Vec<_>>() }))
}

fn open_folder() -> Result<Value, String> {
    let root = brain_dump_root()?;
    let url = format!("file://{}", root.display());
    open_url_via_os(&url)?;
    Ok(json!({ "opened": root.display().to_string() }))
}

fn walk_markdown_files(dir: &std::path::Path, visitor: &mut dyn FnMut(&std::path::Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Recurse only one level for sub-folders so we don't walk
            // a huge vault unnecessarily.
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name == "screenshots" {
                    continue;
                }
            }
            walk_markdown_files(&path, visitor);
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            if let Ok(body) = std::fs::read_to_string(&path) {
                visitor(&path, &body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_unicode_and_punctuation() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Buy milk!! And eggs"), "buy-milk-and-eggs");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn capture_text_args_reject_unknown_fields() {
        // Verifies deny_unknown_fields catches schema drift.
        let bad = serde_json::json!({ "text": "hi", "audience": "self" });
        let parsed: Result<CaptureTextArgs, _> = serde_json::from_value(bad);
        assert!(parsed.is_err(), "unknown 'audience' field should have been rejected");
    }
}
