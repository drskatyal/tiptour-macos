// Breadth-first explorer that builds the capability graph. The explorer
// asks a GroundingProvider (defined in mod.rs) to snapshot the foreground
// app, enumerate candidate actions, and execute / undo them. The explorer
// itself owns scheduling, deduplication, deny-list enforcement, and caps.
//
// Design notes:
// - Destructive actions are still RECORDED as outgoing edges from the
//   current state. We just don't traverse them, so we never discover what
//   they reveal. The metadata is enough for the planner to surface them
//   behind a confirmation later.
// - When the provider reports that undo failed, we mark the edge one_way.
//   The BFS continues from the new state — that's still progress.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::fingerprint::{self, TreeSnapshot};
use super::safety::{classify_action, is_destructive};
use super::types::{
    Action, Capability, CapabilityGraph, Edge, ExploreSummary, ParameterSchema,
    SafetyClassification, StateNode,
};
use super::GroundingProvider;

pub struct ExploreConfig {
    pub max_nodes: usize,
    pub max_duration: Duration,
    pub max_edges_per_node: usize,
}

impl Default for ExploreConfig {
    fn default() -> Self {
        Self {
            max_nodes: 500,
            max_duration: Duration::from_secs(20 * 60),
            max_edges_per_node: 64,
        }
    }
}

pub struct ExploreOutcome {
    pub graph: CapabilityGraph,
    pub capabilities: Vec<Capability>,
    pub summary: ExploreSummary,
}

pub fn explore(
    app_identifier: &str,
    app_version: Option<String>,
    provider: &mut dyn GroundingProvider,
    config: &ExploreConfig,
) -> Result<ExploreOutcome, String> {
    let start_instant = Instant::now();
    let started_at_unix_ms = now_unix_ms();

    let mut graph = CapabilityGraph {
        app_identifier: app_identifier.to_string(),
        app_version,
        nodes: Vec::new(),
        edges: Vec::new(),
        captured_at_unix_ms: started_at_unix_ms,
    };

    let mut seen_state_ids: HashSet<String> = HashSet::new();
    let mut frontier: VecDeque<String> = VecDeque::new();

    let initial_snapshot = provider
        .snapshot_foreground_app()
        .map_err(|error| format!("snapshot failed: {error}"))?;
    let initial_state_id = register_state(&initial_snapshot, &mut graph, &mut seen_state_ids);
    frontier.push_back(initial_state_id);

    let mut stopped_reason = String::from("frontier-exhausted");

    while let Some(current_state_id) = frontier.pop_front() {
        if graph.nodes.len() >= config.max_nodes {
            stopped_reason = "max-nodes".into();
            break;
        }
        if start_instant.elapsed() >= config.max_duration {
            stopped_reason = "max-duration".into();
            break;
        }

        let returned = provider
            .return_to_state(&current_state_id)
            .map_err(|error| format!("return_to_state failed: {error}"))?;
        if !returned {
            // We couldn't physically navigate back to this node; any edges
            // we'd record would be misleading. Skip — the BFS will reach
            // peers from the state we actually landed in.
            continue;
        }

        let candidate_actions = provider
            .enumerate_candidate_actions()
            .map_err(|error| format!("enumerate failed: {error}"))?;

        for (action_index, action) in candidate_actions
            .into_iter()
            .take(config.max_edges_per_node)
            .enumerate()
        {
            let element_name = element_name_for_action(&action);
            let safety = classify_action(&action, &element_name);

            if is_destructive(&action, &element_name) {
                // Record the edge with a synthetic destination so the
                // capability extractor still sees the action. We use
                // a state id derived from the action so repeat encounters
                // collapse cleanly without claiming we visited it.
                let destination = format!("destructive::{current_state_id}::{action_index}");
                graph.edges.push(Edge {
                    from_state_id: current_state_id.clone(),
                    to_state_id: destination,
                    action,
                    safety,
                    is_one_way: true,
                });
                continue;
            }

            let execute_result = provider.execute_action(&action);
            let post_snapshot = match execute_result {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => continue,
                Err(_error) => {
                    // Treat execution failures as one-way unknown edges so
                    // the planner knows not to rely on this transition.
                    continue;
                }
            };

            let post_state_id =
                register_state(&post_snapshot, &mut graph, &mut seen_state_ids);
            let undo_success = provider
                .undo_last_action()
                .map_err(|error| format!("undo failed: {error}"))?;

            graph.edges.push(Edge {
                from_state_id: current_state_id.clone(),
                to_state_id: post_state_id.clone(),
                action,
                safety,
                is_one_way: !undo_success,
            });

            if !seen_already(&post_state_id, &graph) || !undo_success {
                // Only enqueue if we discovered a new state, or if we lost
                // the ability to come back — in the second case we want to
                // explore forward from where we landed.
                if !frontier.contains(&post_state_id) {
                    frontier.push_back(post_state_id);
                }
            }
        }
    }

    let capabilities = extract_capabilities(&graph);
    let summary = ExploreSummary {
        app_identifier: app_identifier.to_string(),
        nodes_discovered: graph.nodes.len(),
        edges_discovered: graph.edges.len(),
        capabilities_extracted: capabilities.len(),
        stopped_reason,
        elapsed_ms: start_instant.elapsed().as_millis(),
    };

    Ok(ExploreOutcome { graph, capabilities, summary })
}

fn register_state(
    snapshot: &TreeSnapshot,
    graph: &mut CapabilityGraph,
    seen: &mut HashSet<String>,
) -> String {
    let state_id = fingerprint::fingerprint(snapshot);
    if seen.insert(state_id.clone()) {
        graph.nodes.push(StateNode {
            state_id: state_id.clone(),
            foreground_window_title: snapshot.foreground_window_title.clone(),
            element_count: fingerprint::element_count(snapshot),
            discovered_at_unix_ms: now_unix_ms(),
        });
    }
    state_id
}

fn seen_already(state_id: &str, graph: &CapabilityGraph) -> bool {
    graph.nodes.iter().any(|node| node.state_id == state_id)
}

fn element_name_for_action(action: &Action) -> String {
    match action {
        Action::Click { element_name, .. } => element_name.clone(),
        Action::Type { target_element_name, .. } => target_element_name.clone(),
        Action::Navigate { menu_path } => menu_path.join(" > "),
        Action::Shortcut { chord_raw } => chord_raw.clone(),
        Action::Scroll { direction, .. } => format!("scroll-{direction}"),
    }
}

// Capability extraction collapses each edge into a callable tool. For a
// first cut, every safe or unknown edge becomes its own capability — the
// retrieval layer ranks them. A future revision should fold sequences
// (open menu → click item → confirm) into single multi-step capabilities.
fn extract_capabilities(graph: &CapabilityGraph) -> Vec<Capability> {
    let mut capabilities = Vec::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        if matches!(edge.safety, SafetyClassification::Destructive) {
            // Destructive edges are intentionally left out of the auto-built
            // tool set. They live in the graph for transparency and can be
            // promoted to capabilities behind an explicit user opt-in.
            continue;
        }

        let element_name = element_name_for_action(&edge.action);
        let canonical_name = canonicalize_name(&element_name);
        let mut parameter_schema = ParameterSchema::default();
        if let Action::Type { text_parameter_name, .. } = &edge.action {
            parameter_schema.properties.insert(
                text_parameter_name.clone(),
                serde_json::json!({
                    "type": "string",
                    "description": "Text to type"
                }),
            );
            parameter_schema.required.push(text_parameter_name.clone());
        }

        let keywords = derive_keywords(&element_name);
        let description = format!("Performs {} via the discovered UI path.", element_name);
        let capability_id = format!("{}::{edge_index}", graph.app_identifier);

        capabilities.push(Capability {
            capability_id,
            canonical_name,
            description,
            parameter_schema,
            safety: edge.safety.clone(),
            replay_actions: vec![edge.action.clone()],
            keywords,
        });
    }
    capabilities
}

fn canonicalize_name(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace(['.', ','], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

fn derive_keywords(raw: &str) -> Vec<String> {
    raw.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty() && token.len() > 1)
        .map(|token| token.to_string())
        .collect()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
