// Spotify adapter — Web API.
//
// Auth: user pastes a Spotify Web API access token (we don't run the
// OAuth dance in v1 because the redirect flow needs a server-side
// callback; an in-app PKCE flow is v2). The user generates a token
// via https://developer.spotify.com/console/get-search-item/ or via
// any other Spotify OAuth helper they prefer.
//
// Handlers (called by the orchestrator's tool router):
//   - `play_track`  : { query: "song name + artist" } -> { track, artist }
//   - `pause`       : {} -> {}
//   - `resume`      : {} -> {}
//   - `next`        : {} -> {}
//   - `previous`    : {} -> {}
//   - `current`     : {} -> { track, artist, isPlaying }
//
// Token storage: keychain entry under slug "spotify" via the
// existing multi-key keychain machinery.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::adapters::{AdapterAuth, AdapterCapability, AdapterManifest};
use crate::keychain;

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "spotify".into(),
        name: "Spotify".into(),
        description: "Play, pause, search, and queue tracks by voice.".into(),
        category: "music".into(),
        voice_triggers: vec![
            "play spotify".into(),
            "play on spotify".into(),
            "spotify play".into(),
            "pause music".into(),
            "skip song".into(),
            "next song".into(),
            "previous song".into(),
            "what's playing".into(),
        ],
        capabilities: vec![AdapterCapability::Network, AdapterCapability::Keychain],
        auth: AdapterAuth::ApiKey {
            label: "Spotify Web API access token".into(),
            help_url: "https://developer.spotify.com/console/get-search-item/".into(),
        },
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes:
            "Generate a Web API access token at the linked page (scopes: \
             user-read-playback-state, user-modify-playback-state, user-read-currently-playing). \
             Paste the token here. Tokens expire after 1 hour — re-paste when expired."
                .into(),
    }
}

pub async fn dispatch(
    _app: AppHandle,
    handler: &str,
    args: Value,
) -> Result<Value, String> {
    let token = keychain::read_provider_key("spotify")?
        .ok_or_else(|| "Spotify not installed. Add a token in Settings → Connected apps.".to_string())?;
    match handler {
        "play_track" => play_track(&token, args).await,
        "pause" => transport(&token, "pause").await,
        "resume" => transport(&token, "play").await,
        "next" => transport(&token, "next").await,
        "previous" => transport(&token, "previous").await,
        "current" => current(&token).await,
        other => Err(format!("Spotify adapter has no handler '{other}'")),
    }
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| format!("http: {error}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayTrackArgs {
    /// Free-form search query — e.g. "Lover by Diljit". The handler
    /// runs a search, picks the top track, and starts it on the
    /// user's active device.
    query: String,
}

async fn play_track(token: &str, args: Value) -> Result<Value, String> {
    let parsed: PlayTrackArgs =
        serde_json::from_value(args).map_err(|error| format!("play_track args: {error}"))?;

    // 1. Search.
    let search_response = http()?
        .get("https://api.spotify.com/v1/search")
        .bearer_auth(token)
        .query(&[("q", parsed.query.as_str()), ("type", "track"), ("limit", "1")])
        .send()
        .await
        .map_err(|error| format!("spotify search: {error}"))?;
    if !search_response.status().is_success() {
        let status = search_response.status();
        let text = search_response.text().await.unwrap_or_default();
        return Err(format!("spotify search {status}: {text}"));
    }
    let search_value: Value = search_response
        .json()
        .await
        .map_err(|error| format!("spotify search parse: {error}"))?;
    let track_uri = search_value
        .pointer("/tracks/items/0/uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Spotify found no tracks for query \"{}\"", parsed.query))?
        .to_string();
    let track_name = search_value
        .pointer("/tracks/items/0/name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)")
        .to_string();
    let artist_name = search_value
        .pointer("/tracks/items/0/artists/0/name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)")
        .to_string();

    // 2. Start playback. PUT to /me/player/play with a uris array.
    let play_response = http()?
        .put("https://api.spotify.com/v1/me/player/play")
        .bearer_auth(token)
        .json(&json!({ "uris": [track_uri] }))
        .send()
        .await
        .map_err(|error| format!("spotify play: {error}"))?;
    if !play_response.status().is_success() {
        let status = play_response.status();
        let text = play_response.text().await.unwrap_or_default();
        // The most common 404 here is "no active device" — surface
        // it specifically so the user opens Spotify on a device
        // instead of debugging "spotify 404".
        if status.as_u16() == 404 {
            return Err(
                "No active Spotify device. Open Spotify on your computer or phone first."
                    .to_string(),
            );
        }
        return Err(format!("spotify play {status}: {text}"));
    }
    Ok(json!({ "track": track_name, "artist": artist_name }))
}

async fn transport(token: &str, action: &str) -> Result<Value, String> {
    // pause + play + next + previous all live under /me/player/<action>.
    // pause / play = PUT; next / previous = POST.
    let url = format!("https://api.spotify.com/v1/me/player/{action}");
    let client = http()?;
    let response = match action {
        "pause" | "play" => client.put(&url).bearer_auth(token).send().await,
        "next" | "previous" => client.post(&url).bearer_auth(token).send().await,
        other => return Err(format!("unsupported transport action: {other}")),
    }
    .map_err(|error| format!("spotify {action}: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            return Err(
                "No active Spotify device. Open Spotify on your computer or phone first."
                    .to_string(),
            );
        }
        return Err(format!("spotify {action} {status}: {text}"));
    }
    Ok(json!({ "ok": true }))
}

async fn current(token: &str) -> Result<Value, String> {
    let response = http()?
        .get("https://api.spotify.com/v1/me/player/currently-playing")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("spotify currently-playing: {error}"))?;
    // 204 = nothing playing. Return a clear shape instead of an error.
    if response.status().as_u16() == 204 {
        return Ok(json!({ "isPlaying": false }));
    }
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("spotify currently-playing {status}: {text}"));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| format!("spotify currently-playing parse: {error}"))?;
    Ok(json!({
        "isPlaying": value.pointer("/is_playing").and_then(|v| v.as_bool()).unwrap_or(false),
        "track": value.pointer("/item/name").and_then(|v| v.as_str()).unwrap_or("(unknown)"),
        "artist": value.pointer("/item/artists/0/name").and_then(|v| v.as_str()).unwrap_or("(unknown)"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_required_capabilities() {
        let m = manifest();
        assert!(m.capabilities.contains(&AdapterCapability::Network));
        assert!(m.capabilities.contains(&AdapterCapability::Keychain));
        assert_eq!(m.slug, "spotify");
        // Triggers must include the obvious "play spotify" phrase
        // or the voice router won't find the adapter.
        assert!(
            m.voice_triggers.iter().any(|t| t.contains("play spotify") || t.contains("play on spotify")),
            "spotify manifest must include a 'play spotify' trigger"
        );
    }
}
