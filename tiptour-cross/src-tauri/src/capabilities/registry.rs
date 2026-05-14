// Tool registry. Loads persisted capabilities, exposes a retrieval API
// keyed on a free-form voice query (bag-of-words for now — TODO: swap to
// an embedding-based ranker), and resolves a chosen tool into a concrete
// ResolvedPlan of executable actions with parameter values bound.

use std::collections::HashMap;

use super::persistence;
use super::types::{Action, Capability, ResolvedPlan, ToolSchema};

pub struct ToolRegistry {
    capabilities: Vec<Capability>,
}

impl ToolRegistry {
    pub fn load_all() -> Result<Self, String> {
        let capabilities = persistence::load_all_capabilities()?;
        Ok(Self { capabilities })
    }

    pub fn load_for_app(
        app_identifier: &str,
        app_version: Option<&str>,
    ) -> Result<Self, String> {
        let capabilities = persistence::load_capabilities(app_identifier, app_version)?;
        Ok(Self { capabilities })
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn retrieve(&self, query: &str, top_k: usize) -> Vec<ToolSchema> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return self
                .capabilities
                .iter()
                .take(top_k)
                .map(capability_to_tool_schema)
                .collect();
        }

        let mut scored: Vec<(f32, &Capability)> = self
            .capabilities
            .iter()
            .map(|capability| (score_capability(capability, &query_tokens), capability))
            .filter(|(score, _)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(_, capability)| capability_to_tool_schema(capability))
            .collect()
    }

    pub fn invoke(
        &self,
        tool_id: &str,
        params: serde_json::Value,
    ) -> Result<ResolvedPlan, String> {
        let capability = self
            .capabilities
            .iter()
            .find(|candidate| candidate.capability_id == tool_id)
            .ok_or_else(|| format!("unknown tool id: {tool_id}"))?;

        let parameter_map: HashMap<String, serde_json::Value> = match params {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            serde_json::Value::Null => HashMap::new(),
            other => {
                return Err(format!(
                    "params must be a JSON object, got: {}",
                    other
                ))
            }
        };

        for required in &capability.parameter_schema.required {
            if !parameter_map.contains_key(required) {
                return Err(format!("missing required parameter: {required}"));
            }
        }

        let bound_actions = capability
            .replay_actions
            .iter()
            .map(|action| bind_action_parameters(action, &parameter_map))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ResolvedPlan {
            tool_id: capability.capability_id.clone(),
            actions: bound_actions,
            safety: capability.safety.clone(),
        })
    }
}

fn bind_action_parameters(
    action: &Action,
    parameters: &HashMap<String, serde_json::Value>,
) -> Result<Action, String> {
    match action {
        Action::Type { target_element_name, text_parameter_name } => {
            // We keep the parameter name in the replay action; binding here
            // only validates that the value is present and a string. The
            // ActionExecutor downstream reads the param map directly.
            let value = parameters
                .get(text_parameter_name)
                .ok_or_else(|| format!("missing parameter {text_parameter_name}"))?;
            if !value.is_string() {
                return Err(format!(
                    "parameter {text_parameter_name} must be a string"
                ));
            }
            Ok(Action::Type {
                target_element_name: target_element_name.clone(),
                text_parameter_name: text_parameter_name.clone(),
            })
        }
        other => Ok(other.clone()),
    }
}

fn capability_to_tool_schema(capability: &Capability) -> ToolSchema {
    ToolSchema {
        tool_id: capability.capability_id.clone(),
        name: capability.canonical_name.clone(),
        description: capability.description.clone(),
        parameter_schema: capability.parameter_schema.clone(),
        safety: capability.safety.clone(),
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty() && token.len() > 1)
        .map(|token| token.to_string())
        .collect()
}

fn score_capability(capability: &Capability, query_tokens: &[String]) -> f32 {
    let mut score = 0.0;
    for query_token in query_tokens {
        if capability.keywords.iter().any(|keyword| keyword == query_token) {
            score += 2.0;
        } else if capability
            .keywords
            .iter()
            .any(|keyword| keyword.contains(query_token) || query_token.contains(keyword))
        {
            score += 1.0;
        }
        if capability.canonical_name.contains(query_token) {
            score += 1.5;
        }
        if capability.description.to_lowercase().contains(query_token) {
            score += 0.5;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::types::{
        Action, Capability, ParameterSchema, SafetyClassification,
    };

    fn make_capability(id: &str, name: &str, keywords: &[&str]) -> Capability {
        Capability {
            capability_id: id.into(),
            canonical_name: name.into(),
            description: format!("Performs {name}"),
            parameter_schema: ParameterSchema::default(),
            safety: SafetyClassification::Safe,
            replay_actions: vec![Action::Click {
                element_name: name.into(),
                element_path: vec![],
            }],
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
        }
    }

    #[test]
    fn retrieve_ranks_keyword_matches() {
        let registry = ToolRegistry {
            capabilities: vec![
                make_capability("a", "new_tab", &["new", "tab"]),
                make_capability("b", "close_window", &["close", "window"]),
            ],
        };
        let results = registry.retrieve("open a new tab", 2);
        assert_eq!(results[0].tool_id, "a");
    }
}
