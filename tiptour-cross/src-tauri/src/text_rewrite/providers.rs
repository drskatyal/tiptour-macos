// Multi-provider LLM dispatcher.
//
// Strategy: every modern fast-inference vendor is OpenAI-compatible
// at `<base>/v1/chat/completions`, so we implement one client and
// vary only base URL + auth header style. Gemini and Anthropic have
// their own schemas and get bespoke impls.
//
// Provider catalog is the single source of truth — the frontend
// reads it via `list_providers` so the settings UI never drifts.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// (id, display_name, supports_reasoning_toggle, suggested_models)
///
/// `supports_reasoning_toggle` controls whether the Personas UI
/// shows the "Reasoning" switch — Grok 4 fast and Gemini 2.5 Pro
/// expose it; Cerebras / Groq / Together / Fireworks do not (their
/// models are non-thinking variants). Anthropic Claude 4.x has
/// thinking but it's per-model so we leave it false here and let
/// the request layer enable it for opus/sonnet ids.
pub const CATALOG: &[(&str, &str, bool, &[&str])] = &[
    (
        "gemini",
        "Google Gemini",
        true,
        &[
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "gemini-2.5-flash-lite",
            "gemini-3.1-flash-live-preview",
        ],
    ),
    (
        "grok",
        "xAI Grok",
        true,
        &[
            "grok-4-fast-reasoning",
            "grok-4-fast-non-reasoning",
            "grok-4",
            "grok-3",
        ],
    ),
    (
        "groq",
        "Groq (LPU inference)",
        false,
        &[
            "llama-3.3-70b-versatile",
            "llama-3.1-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
            "deepseek-r1-distill-llama-70b",
        ],
    ),
    (
        "cerebras",
        "Cerebras (CS-3 inference)",
        false,
        &[
            "llama3.1-8b",
            "llama-3.3-70b",
            "llama-4-scout-17b-16e-instruct",
            "qwen-3-32b",
        ],
    ),
    (
        "together",
        "Together AI",
        false,
        &[
            "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
            "Qwen/Qwen2.5-72B-Instruct-Turbo",
            "deepseek-ai/DeepSeek-V3",
        ],
    ),
    (
        "fireworks",
        "Fireworks AI",
        false,
        &[
            "accounts/fireworks/models/llama-v3p3-70b-instruct",
            "accounts/fireworks/models/llama-v3p1-8b-instruct",
            "accounts/fireworks/models/qwen2p5-72b-instruct",
            "accounts/fireworks/models/deepseek-v3",
        ],
    ),
    (
        "anthropic",
        "Anthropic Claude",
        true,
        &[
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-haiku-4-5-20251001",
        ],
    ),
    (
        "openai",
        "OpenAI",
        true,
        &[
            "gpt-5",
            "gpt-5-mini",
            "gpt-4o",
            "o3-mini",
        ],
    ),
];

pub struct CompletionRequest {
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub temperature: f32,
    pub reasoning_enabled: bool,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<String, String>;
}

pub fn resolve(provider_id: &str) -> Option<Box<dyn LlmProvider>> {
    match provider_id.to_ascii_lowercase().as_str() {
        "gemini" => Some(Box::new(GeminiProvider)),
        "anthropic" => Some(Box::new(AnthropicProvider)),
        "grok" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.x.ai/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: true,
        })),
        "groq" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.groq.com/openai/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: false,
        })),
        "cerebras" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.cerebras.ai/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: false,
        })),
        "together" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.together.xyz/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: false,
        })),
        "fireworks" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.fireworks.ai/inference/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: false,
        })),
        "openai" => Some(Box::new(OpenAiCompat {
            base_url: "https://api.openai.com/v1",
            auth_style: AuthStyle::BearerToken,
            include_reasoning_param: true,
        })),
        _ => None,
    }
}

// ----------- shared HTTP client -----------

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| format!("http client build: {error}"))
}

#[derive(Clone, Copy)]
pub enum AuthStyle {
    BearerToken,
}

// ----------- Gemini -----------

pub struct GeminiProvider;

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<String, String> {
        // Gemini's `generateContent` REST endpoint. System prompt
        // goes in `systemInstruction`; user prompt is a single
        // `user` role part. Thinking budget only added when the
        // persona asked for reasoning.
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            request.model
        );
        let mut body = json!({
            "systemInstruction": { "role": "system", "parts": [{ "text": request.system_prompt }] },
            "contents": [{ "role": "user", "parts": [{ "text": request.user_prompt }] }],
            "generationConfig": {
                "temperature": request.temperature,
            }
        });
        if request.reasoning_enabled {
            // 2.5 Pro reasoning models accept `thinkingConfig` to
            // expose chain-of-thought budget. Conservative budget
            // keeps rewrite latency under a few seconds.
            body["generationConfig"]["thinkingConfig"] = json!({ "thinkingBudget": 2048 });
        }
        let response = http_client()?
            .post(&url)
            .header("x-goog-api-key", request.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("gemini POST: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("gemini {status}: {text}"));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("gemini parse: {error}"))?;
        let text = value
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                format!(
                    "gemini response missing /candidates/0/content/parts/0/text — got {value}"
                )
            })?;
        Ok(text.trim().to_string())
    }
}

// ----------- Anthropic -----------

pub struct AnthropicProvider;

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<String, String> {
        let mut body = json!({
            "model": request.model,
            "max_tokens": 4096,
            "temperature": request.temperature,
            "system": request.system_prompt,
            "messages": [
                { "role": "user", "content": request.user_prompt }
            ]
        });
        if request.reasoning_enabled {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": 2048 });
        }
        let response = http_client()?
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", request.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("anthropic POST: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("anthropic {status}: {text}"));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("anthropic parse: {error}"))?;
        // Walk the content array, concatenating any `type == "text"`
        // blocks. Skips thinking blocks (which we don't want to leak
        // to the user as visible draft text).
        let mut buffer = String::new();
        if let Some(content) = value.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        buffer.push_str(text);
                    }
                }
            }
        }
        if buffer.is_empty() {
            return Err(format!("anthropic response had no text blocks: {value}"));
        }
        Ok(buffer.trim().to_string())
    }
}

// ----------- OpenAI-compatible (Grok, Groq, Cerebras, Together, Fireworks, OpenAI) -----------

pub struct OpenAiCompat {
    pub base_url: &'static str,
    pub auth_style: AuthStyle,
    pub include_reasoning_param: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageOwned,
}
#[derive(Deserialize)]
struct ChatMessageOwned {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<String>,
}

#[async_trait]
impl LlmProvider for OpenAiCompat {
    async fn complete(&self, request: CompletionRequest) -> Result<String, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let messages = vec![
            ChatMessage {
                role: "system",
                content: &request.system_prompt,
            },
            ChatMessage {
                role: "user",
                content: &request.user_prompt,
            },
        ];
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "temperature": request.temperature,
        });
        if self.include_reasoning_param && request.reasoning_enabled {
            // xAI Grok + OpenAI o-series accept this. Other providers
            // ignore unknown params, so this is safe even if our
            // catalog gets out of sync with a provider's actual
            // surface area.
            body["reasoning_effort"] = json!("medium");
        }
        let mut http_request = http_client()?.post(&url).json(&body);
        match self.auth_style {
            AuthStyle::BearerToken => {
                http_request =
                    http_request.header("Authorization", format!("Bearer {}", request.api_key));
            }
        }
        let response = http_request
            .send()
            .await
            .map_err(|error| format!("openai-compat POST: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("openai-compat {status} at {}: {text}", self.base_url));
        }
        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|error| format!("openai-compat parse: {error}"))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| format!("openai-compat response missing choices[0].message.content"))?;
        Ok(content.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_all_expected_providers() {
        // Make sure the user can pick every provider we documented
        // in the PR — if someone deletes one by mistake the test
        // fails loud instead of the UI silently losing options.
        let ids: Vec<&str> = CATALOG.iter().map(|(id, ..)| *id).collect();
        for expected in [
            "gemini", "grok", "groq", "cerebras", "together", "fireworks", "anthropic", "openai",
        ] {
            assert!(
                ids.contains(&expected),
                "provider {expected} missing from catalog (have {ids:?})"
            );
        }
    }

    #[test]
    fn resolve_returns_some_for_every_catalog_entry() {
        for (id, ..) in CATALOG {
            assert!(
                resolve(id).is_some(),
                "resolve({id}) returned None but {id} is in CATALOG"
            );
        }
    }

    #[test]
    fn resolve_returns_none_for_unknown_provider() {
        assert!(resolve("unknown-vendor").is_none());
    }
}
