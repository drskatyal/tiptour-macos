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

use crate::grounding;

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

#[tauri::command]
pub async fn explore_app(app_identifier: String) -> Result<ExploreSummary, String> {
    // Exploration is CPU-and-IO heavy and can run for minutes — push it to
    // a blocking task so the Tauri runtime stays responsive for the panel.
    let identifier = app_identifier.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut provider = grounding::make_grounding_provider();
        let config = explorer::ExploreConfig::default();
        // Stamp the persisted graph + tools with the current executable
        // version so cache loaders can pick the right vintage on next launch
        // (see registry::ToolRegistry::load_all).
        let app_version = grounding::target_app::current_target_app()
            .and_then(|target_app| target_app.file_version);
        let outcome = explorer::explore(
            &identifier,
            app_version.clone(),
            provider.as_mut(),
            &config,
        )?;
        persistence::save_graph(&outcome.graph)?;
        persistence::save_capabilities(&identifier, app_version.as_deref(), &outcome.capabilities)?;
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
        // Best-effort current foreground fingerprint; if grounding can't
        // reach the AX/UIA tree (Linux dev host, denied permissions), we
        // pass None — capabilities with no preconditions stay visible.
        let mut provider = grounding::make_grounding_provider();
        let current_state_hash = provider
            .snapshot_foreground_app()
            .ok()
            .map(|snapshot| fingerprint::fingerprint(&snapshot));
        Ok::<_, String>(registry.retrieve_with_state(
            &query,
            top_k,
            current_state_hash.as_deref(),
        ))
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

// Deliberate non-goal: embedding-backed retrieval to replace the bag-of-words
// ranker. Switching would require shipping an ONNX runtime + model (~100MB)
// or a network round-trip per query, both of which are a net loss for the
// instant-feel grounding goal. The bag-of-words ranker is good enough until
// the user shows the tool registry doesn't surface the right capability for
// real voice queries.
