// Apple Music / Music.app adapter — OSA on macOS.
//
// Free macOS counterpart to the Spotify adapter. Drives Music.app
// (formerly iTunes) via AppleScript. No auth.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::{osa_escape, parse_args, run_osascript};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "apple-music".into(),
        name: "Apple Music".into(),
        description: "Play, pause, skip, and search your Music library by voice.".into(),
        category: "music".into(),
        voice_triggers: vec![
            "play apple music".into(),
            "play music".into(),
            "music app".into(),
            "play song in music".into(),
        ],
        capabilities: vec![AdapterCapability::Osascript],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into()],
        setup_notes: "Drives the Music.app you're already signed into. No additional setup needed.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "play_track" => play_track(args),
        "pause" => transport("pause"),
        "resume" => transport("play"),
        "next" => transport("next track"),
        "previous" => transport("previous track"),
        "current" => current(),
        other => Err(format!("apple-music: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayTrackArgs {
    /// Track name or "track by artist". We pass straight to
    /// Music.app's search/play; it does substring match on track
    /// name + album + artist.
    query: String,
}

fn play_track(args: Value) -> Result<Value, String> {
    let parsed: PlayTrackArgs = parse_args(args)?;
    let q = osa_escape(&parsed.query);
    // Search the library for the query and play the first match.
    // `play first track of …` raises if zero matches, which we
    // turn into a clean error.
    let script = format!(
        "tell application \"Music\"\n\
            set theResults to (search library 1 for \"{q}\" only songs)\n\
            if (count of theResults) is 0 then\n\
                return \"no-match\"\n\
            end if\n\
            play (item 1 of theResults)\n\
            return (name of current track) & \" — \" & (artist of current track)\n\
        end tell"
    );
    let output = run_osascript(&script)?;
    if output == "no-match" {
        return Err(format!("Apple Music found no tracks for \"{}\"", parsed.query));
    }
    Ok(json!({ "result": output }))
}

fn transport(verb: &str) -> Result<Value, String> {
    let script = format!("tell application \"Music\" to {verb}");
    run_osascript(&script)?;
    Ok(json!({ "ok": true }))
}

fn current() -> Result<Value, String> {
    let script = "tell application \"Music\"\n\
        if player state is playing then\n\
            return \"playing|\" & (name of current track) & \"|\" & (artist of current track)\n\
        else\n\
            return \"idle\"\n\
        end if\n\
        end tell";
    let raw = run_osascript(script)?;
    if raw == "idle" {
        return Ok(json!({ "isPlaying": false }));
    }
    let mut parts = raw.splitn(3, '|');
    let _state = parts.next();
    let track = parts.next().unwrap_or("");
    let artist = parts.next().unwrap_or("");
    Ok(json!({ "isPlaying": true, "track": track, "artist": artist }))
}
