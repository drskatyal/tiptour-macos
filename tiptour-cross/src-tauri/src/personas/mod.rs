// Voice-activated agent personas — system-prompt presets the user can
// switch between. Built-in seeds get written on first read so the JSON
// file is always self-describing on disk; user-authored edits to a
// built-in are persisted alongside the seeds and survive next boot.
//
// The active persona id is held in the same `personas.json` so a single
// atomic write keeps both the catalog and the selection consistent.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const PERSONAS_FILE_NAME: &str = "personas.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    /// Phrases like "switch to coding assistant" the Vosk grammar uses
    /// to flip personas hands-free. Lowercase, no punctuation.
    pub voice_trigger_phrases: Vec<String>,
    /// True for built-in seeds; the UI may show a "Reset" button only
    /// for these and disallow Delete.
    #[serde(default)]
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonasFile {
    pub schema_version: u32,
    pub active_persona_id: String,
    pub personas: Vec<Persona>,
}

impl Default for PersonasFile {
    fn default() -> Self {
        let seeds = built_in_seed_personas();
        let active = seeds[0].id.clone();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            active_persona_id: active,
            personas: seeds,
        }
    }
}

fn built_in_seed_personas() -> Vec<Persona> {
    vec![
        Persona {
            id: "coding-assistant".into(),
            name: "Coding Assistant".into(),
            system_prompt: "You are TipTour in coding-assistant mode. Help the user write, debug, and review code. Reply concisely with code snippets when relevant. Prefer practical fixes over lectures.".into(),
            voice_trigger_phrases: vec!["coding assistant".into(), "developer mode".into()],
            is_built_in: true,
        },
        Persona {
            id: "writing-coach".into(),
            name: "Writing Coach".into(),
            system_prompt: "You are TipTour in writing-coach mode. Help the user write, edit, and improve prose. Suggest tighter phrasing. When the user highlights text, propose specific replacements.".into(),
            voice_trigger_phrases: vec!["writing coach".into(), "writing mode".into()],
            is_built_in: true,
        },
        Persona {
            id: "meeting-assistant".into(),
            name: "Meeting Assistant".into(),
            system_prompt: "You are TipTour in meeting-assistant mode. Take notes, summarize, and pull out action items. Be brief and structured.".into(),
            voice_trigger_phrases: vec!["meeting assistant".into(), "meeting mode".into()],
            is_built_in: true,
        },
        Persona {
            id: "quick-helper".into(),
            name: "Quick Helper".into(),
            system_prompt: "You are TipTour in quick-helper mode. Answer fast, in one or two sentences. No preamble.".into(),
            voice_trigger_phrases: vec!["quick helper".into(), "quick mode".into()],
            is_built_in: true,
        },
        Persona {
            id: "teacher".into(),
            name: "Teacher".into(),
            system_prompt: "You are TipTour in teacher mode. Walk the user through unfamiliar tools step by step. Point at the next thing to click; don't take over the keyboard.".into(),
            voice_trigger_phrases: vec!["teacher".into(), "teacher mode".into(), "show me how".into()],
            is_built_in: true,
        },
    ]
}

fn personas_file_path() -> Option<PathBuf> {
    let mut path = dirs::data_local_dir()?;
    path.push(ROOT_DIRECTORY_NAME);
    path.push(PERSONAS_FILE_NAME);
    Some(path)
}

pub fn load_personas_from_disk() -> PersonasFile {
    let Some(path) = personas_file_path() else {
        return PersonasFile::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        // First-read seed: write the defaults to disk so the catalog is
        // discoverable + editable through the file.
        let defaults = PersonasFile::default();
        let _ = save_personas_to_disk(&defaults);
        return defaults;
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save_personas_to_disk(file: &PersonasFile) -> Result<(), String> {
    let path = personas_file_path().ok_or_else(|| "no data dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    fs::write(&tmp_path, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp_path, &path).map_err(|error| error.to_string())?;
    Ok(())
}

// In-memory cache so list/get calls don't hit disk every time. Kept in
// sync via an atomic load on first access and rewrite on each mutator.
static PERSONAS_CACHE: Lazy<Mutex<Option<PersonasFile>>> = Lazy::new(|| Mutex::new(None));

fn with_loaded<R>(handler: impl FnOnce(&mut PersonasFile) -> R) -> R {
    let mut guard = PERSONAS_CACHE.lock().expect("personas mutex poisoned");
    if guard.is_none() {
        *guard = Some(load_personas_from_disk());
    }
    handler(guard.as_mut().expect("just initialized"))
}

#[tauri::command]
pub fn list_personas() -> Result<Vec<Persona>, String> {
    Ok(with_loaded(|file| file.personas.clone()))
}

#[tauri::command]
pub fn get_active_persona() -> Result<Persona, String> {
    with_loaded(|file| {
        file.personas
            .iter()
            .find(|p| p.id == file.active_persona_id)
            .or_else(|| file.personas.first())
            .cloned()
            .ok_or_else(|| "no personas available".to_string())
    })
}

#[tauri::command]
pub fn set_active_persona(id: String) -> Result<(), String> {
    with_loaded(|file| {
        if !file.personas.iter().any(|p| p.id == id) {
            return Err(format!("no persona with id {id}"));
        }
        file.active_persona_id = id;
        save_personas_to_disk(file)
    })
}

#[tauri::command]
pub fn upsert_custom_persona(persona: Persona) -> Result<Persona, String> {
    with_loaded(|file| {
        // Match the existing entry by id, preserving the built-in flag
        // when the caller forgot to set it (the UI shouldn't be able to
        // strip the flag from a built-in via an edit).
        if let Some(existing) = file.personas.iter_mut().find(|p| p.id == persona.id) {
            let preserved_built_in = existing.is_built_in;
            *existing = persona.clone();
            existing.is_built_in = preserved_built_in;
        } else {
            file.personas.push(persona.clone());
        }
        save_personas_to_disk(file)?;
        Ok(persona)
    })
}

#[tauri::command]
pub fn delete_persona(id: String) -> Result<(), String> {
    with_loaded(|file| {
        let target = file.personas.iter().find(|p| p.id == id);
        if let Some(target) = target {
            if target.is_built_in {
                return Err("cannot delete a built-in persona".to_string());
            }
        } else {
            return Err(format!("no persona with id {id}"));
        }
        file.personas.retain(|p| p.id != id);
        // If we just nuked the active one, fall back to the first.
        if file.active_persona_id == id {
            if let Some(fallback) = file.personas.first() {
                file.active_persona_id = fallback.id.clone();
            }
        }
        save_personas_to_disk(file)
    })
}

/// Match a free-form voice query against persona names + trigger
/// phrases. Returns the persona id if confident; None otherwise.
/// Used by the Vosk dispatcher's "switch to <persona>" path.
#[tauri::command]
pub fn match_persona_by_voice(query: String) -> Result<Option<String>, String> {
    let normalized_query = query.trim().to_ascii_lowercase();
    if normalized_query.is_empty() {
        return Ok(None);
    }
    let stripped_query = normalized_query
        .strip_prefix("switch to ")
        .or_else(|| normalized_query.strip_prefix("become "))
        .unwrap_or(&normalized_query)
        .trim()
        .to_string();
    Ok(with_loaded(|file| {
        for persona in &file.personas {
            let name_lower = persona.name.to_ascii_lowercase();
            if stripped_query == name_lower || stripped_query.contains(&name_lower) {
                return Some(persona.id.clone());
            }
            for trigger in &persona.voice_trigger_phrases {
                let trigger_lower = trigger.to_ascii_lowercase();
                if stripped_query == trigger_lower || stripped_query.contains(&trigger_lower) {
                    return Some(persona.id.clone());
                }
            }
        }
        None
    }))
}
