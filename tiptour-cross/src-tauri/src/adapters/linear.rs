// Linear adapter — GraphQL API with personal API key.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "linear".into(),
        name: "Linear".into(),
        description: "Create issues and view your queue in Linear.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "create linear issue".into(),
            "new linear ticket".into(),
            "file a bug in linear".into(),
            "what's on my linear queue".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Linear personal API key".into(),
            help_url: "https://linear.app/settings/api".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Create a personal API key at the linked page (starts with 'lin_api_'). It has access to every team and project you have permission for.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    let token = keychain::read_provider_key("linear")?
        .ok_or_else(|| "Linear not installed. Paste an API key first.".to_string())?;
    match handler {
        "create_issue" => create_issue(&token, args).await,
        "my_issues" => my_issues(&token).await,
        other => Err(format!("linear: no handler '{other}'")),
    }
}

async fn graphql(token: &str, query: &str, variables: Value) -> Result<Value, String> {
    let response = http_client()?
        .post("https://api.linear.app/graphql")
        .header("Authorization", token)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|error| format!("linear: {error}"))?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if let Some(errors) = value.get("errors") {
        return Err(format!("linear errors: {errors}"));
    }
    Ok(value.get("data").cloned().unwrap_or_default())
}

#[derive(Deserialize)]
struct CreateIssueArgs {
    team_key: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    /// 0 = no priority, 1 = urgent, 2 = high, 3 = medium, 4 = low.
    #[serde(default)]
    priority: Option<u32>,
}

async fn create_issue(token: &str, args: Value) -> Result<Value, String> {
    let parsed: CreateIssueArgs = parse_args(args)?;
    // Resolve team id from team key (LIN-…) first — Linear's mutation
    // requires the UUID. One round-trip per call; cache lands in v2.
    let team_query = "query($key: String!) { teams(filter:{ key:{ eq:$key }}) { nodes { id } } }";
    let teams = graphql(token, team_query, json!({ "key": parsed.team_key })).await?;
    let team_id = teams
        .pointer("/teams/nodes/0/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("No Linear team with key '{}'.", parsed.team_key))?
        .to_string();

    let mutation = "
        mutation($teamId: String!, $title: String!, $description: String, $priority: Int) {
            issueCreate(input:{ teamId:$teamId, title:$title, description:$description, priority:$priority }) {
                issue { id identifier url }
            }
        }";
    let result = graphql(
        token,
        mutation,
        json!({
            "teamId": team_id,
            "title": parsed.title,
            "description": parsed.description.unwrap_or_default(),
            "priority": parsed.priority.unwrap_or(0),
        }),
    )
    .await?;
    Ok(json!({
        "identifier": result.pointer("/issueCreate/issue/identifier"),
        "url": result.pointer("/issueCreate/issue/url"),
    }))
}

async fn my_issues(token: &str) -> Result<Value, String> {
    let q = "{ viewer { assignedIssues(first:10, filter:{state:{type:{neq:\"completed\"}}}) { nodes { identifier title state{name} } } } }";
    let data = graphql(token, q, json!({})).await?;
    Ok(data.pointer("/viewer/assignedIssues/nodes").cloned().unwrap_or(json!([])))
}
