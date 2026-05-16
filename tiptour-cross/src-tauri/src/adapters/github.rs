// GitHub adapter — REST API with personal access token.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "github".into(),
        name: "GitHub".into(),
        description: "Create issues, view your assigned PRs, search repos.".into(),
        category: "dev".into(),
        voice_triggers: vec![
            "create github issue".into(),
            "open issue on github".into(),
            "what prs need my review".into(),
            "search github".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "GitHub personal access token".into(),
            help_url: "https://github.com/settings/tokens?type=beta".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Create a fine-grained PAT with 'repo' read+write scope on the repos you care about. Classic PATs also work — needs at minimum 'repo' + 'read:user'.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let token = keychain::read_provider_key("github")?
        .ok_or_else(|| "GitHub not installed. Paste a token first.".to_string())?;
    match handler {
        "create_issue" => create_issue(&token, args).await,
        "my_review_requests" => my_review_requests(&token).await,
        other => Err(format!("github: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
struct CreateIssueArgs {
    /// "owner/repo".
    repo: String,
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
}

async fn create_issue(token: &str, args: Value) -> Result<Value, String> {
    let parsed: CreateIssueArgs = parse_args(args)?;
    let url = format!("https://api.github.com/repos/{}/issues", parsed.repo);
    let payload = json!({
        "title": parsed.title,
        "body": parsed.body.unwrap_or_default(),
        "labels": parsed.labels,
    });
    let response = http_client()?
        .post(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "TipTour")
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "github {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(json!({
        "number": value.get("number"),
        "url": value.get("html_url"),
    }))
}

async fn my_review_requests(token: &str) -> Result<Value, String> {
    let response = http_client()?
        .get("https://api.github.com/search/issues?q=is:pr+is:open+review-requested:@me")
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "TipTour")
        .send()
        .await
        .map_err(|error| format!("github search: {error}"))?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    let items: Vec<Value> = value
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|pr| {
            json!({
                "title": pr.get("title"),
                "url": pr.get("html_url"),
                "repository": pr.pointer("/repository_url").and_then(|v| v.as_str()).map(|u| u.replace("https://api.github.com/repos/", "")),
            })
        })
        .collect();
    Ok(json!({ "prs": items }))
}
