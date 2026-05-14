// Capability graph + tool registry entry point. Tauri commands here are
// the only surface exposed to the TS side; everything else is plumbing.
//
// The explorer talks to the grounding/AX backend through the
// `GroundingProvider` trait so this module stays decoupled from the
// in-progress Phase 1 grounding implementation. Phase 1 will impl this
// trait against UIA / AX trees later.

pub mod explorer;
pub mod fingerprint;
pub mod persistence;
pub mod registry;
pub mod safety;
pub mod types;

use fingerprint::TreeSnapshot;
use types::{Action, Capability, ExploreSummary, ResolvedPlan, ToolSchema};

// The trait the explorer uses to drive the target app. Phase 1's grounding
// layer will provide a concrete impl. Returning `None` from
// execute_action means "the action ran but produced no visible state
// change" — the explorer just skips edge creation in that case.
pub trait GroundingProvider: Send {
    fn snapshot_foreground_app(&mut self) -> Result<TreeSnapshot, String>;
    fn enumerate_candidate_actions(&mut self) -> Result<Vec<Action>, String>;
    fn execute_action(&mut self, action: &Action) -> Result<Option<TreeSnapshot>, String>;
    fn undo_last_action(&mut self) -> Result<bool, String>;
    // Returns true if the provider managed to navigate back to the given
    // state. False signals the explorer should drop this state from the
    // frontier — it can't be reached again.
    fn return_to_state(&mut self, state_id: &str) -> Result<bool, String>;
}

// TODO: replace the placeholder provider with a real Phase 1-backed impl
// once grounding lands. The placeholder lets the explorer compile and
// lets us unit-test the BFS without a live AX backend.
struct PlaceholderGroundingProvider;

impl GroundingProvider for PlaceholderGroundingProvider {
    fn snapshot_foreground_app(&mut self) -> Result<TreeSnapshot, String> {
        Err("grounding provider not wired yet".into())
    }
    fn enumerate_candidate_actions(&mut self) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
    fn execute_action(&mut self, _action: &Action) -> Result<Option<TreeSnapshot>, String> {
        Ok(None)
    }
    fn undo_last_action(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn return_to_state(&mut self, _state_id: &str) -> Result<bool, String> {
        Ok(false)
    }
}

#[tauri::command]
pub async fn explore_app(app_identifier: String) -> Result<ExploreSummary, String> {
    // Exploration is CPU-and-IO heavy and can run for minutes — push it to
    // a blocking task so the Tauri runtime stays responsive for the panel.
    let identifier = app_identifier.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut provider = PlaceholderGroundingProvider;
        let config = explorer::ExploreConfig::default();
        let outcome = explorer::explore(&identifier, None, &mut provider, &config)?;
        persistence::save_graph(&outcome.graph)?;
        persistence::save_capabilities(&identifier, None, &outcome.capabilities)?;
        Ok::<_, String>(outcome.summary)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_capabilities(app_identifier: String) -> Result<Vec<Capability>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        persistence::load_capabilities(&app_identifier, None)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn retrieve_tools(query: String, top_k: usize) -> Result<Vec<ToolSchema>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let registry = registry::ToolRegistry::load_all()?;
        Ok::<_, String>(registry.retrieve(&query, top_k))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn invoke_capability(
    tool_id: String,
    params: serde_json::Value,
) -> Result<ResolvedPlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let registry = registry::ToolRegistry::load_all()?;
        registry.invoke(&tool_id, params)
    })
    .await
    .map_err(|error| error.to_string())?
}

// Cross-cutting TODOs:
// - Embedding-backed retrieval to replace the bag-of-words ranker.
// - Lazy submenu expansion via UIA ExpandCollapsePattern so the explorer
//   can reach nested menu items without manual hovering.
// - Cache invalidation when the target app's version changes (the version
//   is encoded in the persistence path but not yet checked at load time).
// - Per-app state preconditions ("annotation tools require an open
//   document") so the registry can hide unreachable capabilities until the
//   user satisfies their precondition.
