// Web search adapter — Brave Search API.
//
// Closes the knowledge-cutoff gap. Without this, Gemini fabricates or
// refuses for "current info" queries. Brave is the cheapest free-tier
// general-web search with a reasonable result quality + a clean
// JSON shape that's easy to summarize.
//
// Handlers:
//   - search { query, count? } -> { results: [{title, url, snippet}] }
//   - news   { query, count? } -> same shape, news.search endpoint
//
// API key stored under the "brave-search" keychain slot. Free tier
// gives 2000 queries/month — more than enough for a personal
// assistant.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

const BRAVE_WEB_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const BRAVE_NEWS_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/news/search";

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "web-search".into(),
        name: "Web search (Brave)".into(),
        description: "Live web search so the agent can answer 'what time does X close', 'who is Y', 'what's the latest on Z' — anything past Gemini's knowledge cutoff.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "search the web for".into(),
            "look up".into(),
            "what's the latest".into(),
            "find me information about".into(),
            "what time does".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Brave Search API token".into(),
            help_url: "https://api.search.brave.com/app/keys".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Free tier: 2000 queries/month, no card required. Sign up at the linked URL, create a 'Free' plan token, paste it here.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let api_key = keychain::read_provider_key("brave-search")?
        .ok_or_else(|| "No Brave Search API key. Add one in Settings → Connected apps.".to_string())?;
    match handler {
        "search" => web_search(&api_key, args, BRAVE_WEB_SEARCH_URL, "web").await,
        "news" => web_search(&api_key, args, BRAVE_NEWS_SEARCH_URL, "news").await,
        other => Err(format!("web-search: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    /// 1-20, default 5. Trim to keep Gemini's context budget healthy
    /// — 5 result snippets is usually enough to answer most queries.
    #[serde(default)]
    count: Option<u32>,
}

async fn web_search(
    api_key: &str,
    args: Value,
    endpoint: &str,
    pointer_root: &str,
) -> Result<Value, String> {
    let parsed: SearchArgs = parse_args(args)?;
    let count = parsed.count.unwrap_or(5).clamp(1, 20);

    let response = http_client()?
        .get(endpoint)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .query(&[
            ("q", parsed.query.as_str()),
            ("count", &count.to_string()),
            ("safesearch", "moderate"),
        ])
        .send()
        .await
        .map_err(|error| format!("brave-search: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "brave-search {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let value: Value = response.json().await.map_err(|e| format!("parse: {e}"))?;
    // Brave returns: { web: { results: [{title, url, description, ...}] } }
    // News:        { results: [{title, url, description, age, source}] }
    let results_array = value
        .pointer(&format!("/{pointer_root}/results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let trimmed: Vec<Value> = results_array
        .into_iter()
        .take(count as usize)
        .map(|item| {
            json!({
                "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "url": item.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "snippet": item.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                // News-only fields surface when present, ignored otherwise.
                "age": item.get("age").and_then(|v| v.as_str()),
                "source": item.get("source").and_then(|v| v.as_str()),
            })
        })
        .collect();
    Ok(json!({ "results": trimmed, "query": parsed.query }))
}
