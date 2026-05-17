// Shared building blocks for adapters: AppleScript runner, URI opener,
// CLI shell-out, HTTP client. Keeping these here means each adapter
// file is just its manifest + a thin dispatch table.

use serde_json::Value;

#[cfg(target_os = "macos")]
pub fn run_osascript(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("osascript spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "osascript: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn run_osascript(_script: &str) -> Result<String, String> {
    // Non-macOS path is a soft fail so the adapter surface still
    // compiles on Linux/Windows CI. Adapters that depend on OSA
    // declare macos as the only supported_platform.
    Err("osascript is macOS-only".to_string())
}

/// Escape a string for safe interpolation inside an AppleScript
/// double-quoted literal. AppleScript's quoting rules are simple
/// but every adapter needs them so we centralize.
pub fn osa_escape(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn open_url_via_os(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("open {url}: {error}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|error| format!("start {url}: {error}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("xdg-open {url}: {error}"))?;
    }
    Ok(())
}

/// Spawn a CLI tool and capture stdout. We use blocking std::process
/// here rather than tokio::process because adapters call this from
/// async fns and a single shell-out per adapter call is fine.
pub fn spawn_cli(program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program} spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("http: {error}"))
}

/// Coerce `serde_json::Value` -> typed args via serde_json, returning
/// a clean error if the shape is wrong. Adapters use this for every
/// handler so the dispatch boundary doesn't litter `.map_err` calls.
pub fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|error| format!("invalid args: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osa_escape_handles_quotes_and_backslashes() {
        assert_eq!(osa_escape("hello"), "hello");
        assert_eq!(osa_escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(osa_escape("c:\\path"), "c:\\\\path");
    }
}
