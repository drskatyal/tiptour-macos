// Disk persistence for capability graphs and tool registries. Layout:
//   $LOCAL_DATA/TipTour/capabilities/{app_identifier}/{version}/graph.json
//   $LOCAL_DATA/TipTour/capabilities/{app_identifier}/{version}/tools.json
//
// We sanitize the app identifier and version into filesystem-safe segments
// so an identifier like "com.foo/bar" can't escape the capabilities root.

use std::fs;
use std::path::PathBuf;

use super::types::{Capability, CapabilityGraph};

const DEFAULT_VERSION_FOLDER: &str = "unversioned";

pub fn capabilities_root() -> Result<PathBuf, String> {
    let mut base = dirs::data_local_dir()
        .ok_or_else(|| "No local data dir available".to_string())?;
    base.push("TipTour");
    base.push("capabilities");
    Ok(base)
}

pub fn app_version_directory(
    app_identifier: &str,
    app_version: Option<&str>,
) -> Result<PathBuf, String> {
    let mut path = capabilities_root()?;
    path.push(sanitize_path_segment(app_identifier));
    path.push(sanitize_path_segment(
        app_version.unwrap_or(DEFAULT_VERSION_FOLDER),
    ));
    Ok(path)
}

pub fn save_graph(graph: &CapabilityGraph) -> Result<(), String> {
    let dir = app_version_directory(&graph.app_identifier, graph.app_version.as_deref())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join("graph.json");
    let json = serde_json::to_string_pretty(graph).map_err(|error| error.to_string())?;
    fs::write(&path, json).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn load_graph(
    app_identifier: &str,
    app_version: Option<&str>,
) -> Result<Option<CapabilityGraph>, String> {
    let dir = app_version_directory(app_identifier, app_version)?;
    let path = dir.join("graph.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let graph = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    Ok(Some(graph))
}

pub fn save_capabilities(
    app_identifier: &str,
    app_version: Option<&str>,
    capabilities: &[Capability],
) -> Result<(), String> {
    let dir = app_version_directory(app_identifier, app_version)?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join("tools.json");
    let json = serde_json::to_string_pretty(capabilities).map_err(|error| error.to_string())?;
    fs::write(&path, json).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn load_capabilities(
    app_identifier: &str,
    app_version: Option<&str>,
) -> Result<Vec<Capability>, String> {
    let dir = app_version_directory(app_identifier, app_version)?;
    let path = dir.join("tools.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let parsed = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    Ok(parsed)
}

// Load capabilities across every known app — the registry pulls all
// installed apps' tools into one search index so a free-form voice query
// can be matched without first knowing which app the user means.
pub fn load_all_capabilities() -> Result<Vec<Capability>, String> {
    let root = match capabilities_root() {
        Ok(path) => path,
        Err(_) => return Ok(Vec::new()),
    };
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut all = Vec::new();
    let app_dirs = fs::read_dir(&root).map_err(|error| error.to_string())?;
    for app_entry in app_dirs.flatten() {
        let version_dirs = match fs::read_dir(app_entry.path()) {
            Ok(iter) => iter,
            Err(_) => continue,
        };
        for version_entry in version_dirs.flatten() {
            let tools_path = version_entry.path().join("tools.json");
            if !tools_path.exists() {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&tools_path) {
                if let Ok(mut parsed) = serde_json::from_str::<Vec<Capability>>(&raw) {
                    all.append(&mut parsed);
                }
            }
        }
    }
    Ok(all)
}

fn sanitize_path_segment(segment: &str) -> String {
    segment
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
