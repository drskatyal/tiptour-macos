// Browser adapter — Chrome DevTools Protocol over WebSocket.
//
// One adapter unlocks every Chromium tab: Chrome, Edge, Arc, Brave,
// Vivaldi. Works against any browser launched with the
// `--remote-debugging-port=9222` flag (the standard one Puppeteer
// and Playwright use). The browser exposes:
//
//   GET  http://localhost:9222/json          -> list of tab targets
//   GET  http://localhost:9222/json/version  -> browser version + ws URL
//   WS   ws://localhost:9222/devtools/page/{targetId}
//
// On the WebSocket the protocol is JSON-RPC: every command is
// `{ id, method, params }` and the server replies with
// `{ id, result }` or `{ id, error }`. We open a fresh socket per
// call to keep state simple — long-lived connections would need an
// event loop and a reconnect supervisor.
//
// Handlers (so Gemini's `control_app` picks the right one):
//   - list_tabs       : {} -> [{ id, url, title }]
//   - open_url        : { url, new_tab?: bool } -> { tabId, url }
//   - current_url     : {} -> { url }
//   - current_title   : {} -> { title }
//   - read_page_text  : { tab_match?: string } -> { text }
//   - click_text      : { text, tab_match?: string } -> { clicked: bool }
//   - fill_field      : { selector, value, tab_match?: string } -> { filled: bool }
//   - execute_js      : { expression, tab_match?: string } -> { result }
//
// `tab_match` is an optional substring matched against either the
// tab's URL or its title. Omit it and we pick the most-recently-
// activated page (heuristic: first non-extension target in the
// /json list, which Chrome returns in front-to-back order).

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::AppHandle;
use tokio_tungstenite::tungstenite::Message;

use super::helpers::{http_client, parse_args};
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

const DEFAULT_DEBUG_PORT: u16 = 9222;
static RPC_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "browser".into(),
        name: "Browser (Chrome / Edge / Arc / Brave)".into(),
        description: "Control any Chromium-based browser tab — open URLs, read page text, click elements, fill forms. Works with Chrome, Edge, Arc, Brave, and Vivaldi.".into(),
        category: "files".into(),
        voice_triggers: vec![
            "open this in browser".into(),
            "read this page".into(),
            "what does this page say".into(),
            "click on the page".into(),
            "summarize this page".into(),
            "fill this form".into(),
        ],
        capabilities: vec![
            AdapterCapability::Network,
            AdapterCapability::Spawn,
        ],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into(), "linux".into()],
        setup_notes: "Launch your browser with the DevTools port open. \
            macOS: `open -na \"Google Chrome\" --args --remote-debugging-port=9222`. \
            Windows: `chrome.exe --remote-debugging-port=9222`. \
            Or use Chrome flags page: chrome://flags. \
            TipTour talks to http://localhost:9222 — no auth required for local debugging.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "list_tabs" => list_tabs().await,
        "open_url" => open_url(args).await,
        "current_url" => current_url(args).await,
        "current_title" => current_title(args).await,
        "read_page_text" => read_page_text(args).await,
        "click_text" => click_text(args).await,
        "fill_field" => fill_field(args).await,
        "execute_js" => execute_js(args).await,
        other => Err(format!("browser: no handler '{other}'")),
    }
}

// ---------- tab discovery ----------

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Tab {
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default, rename = "type")]
    target_type: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    ws_url: String,
}

/// Fetch every open tab. Returns only `type:"page"` targets (drops
/// service workers, background pages, etc).
async fn fetch_tabs() -> Result<Vec<Tab>, String> {
    let response = http_client()?
        .get(&format!("http://localhost:{DEFAULT_DEBUG_PORT}/json"))
        .send()
        .await
        .map_err(|error| {
            format!(
                "Couldn't reach Chrome DevTools on port {DEFAULT_DEBUG_PORT}. \
                 Launch your browser with --remote-debugging-port={DEFAULT_DEBUG_PORT}. ({error})"
            )
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "DevTools port responded {} — is the browser running with debugging on?",
            response.status()
        ));
    }
    let tabs: Vec<Tab> = response.json().await.map_err(|e| e.to_string())?;
    Ok(tabs.into_iter().filter(|t| t.target_type == "page").collect())
}

async fn list_tabs() -> Result<Value, String> {
    let tabs = fetch_tabs().await?;
    let entries: Vec<Value> = tabs
        .into_iter()
        .map(|t| json!({ "id": t.id, "url": t.url, "title": t.title }))
        .collect();
    Ok(json!({ "tabs": entries }))
}

/// Pick a tab by optional substring matched against URL or title.
/// `None` => first page tab (most-recently-activated).
async fn pick_tab(tab_match: Option<&str>) -> Result<Tab, String> {
    let tabs = fetch_tabs().await?;
    if tabs.is_empty() {
        return Err("No browser tabs are open. Open at least one tab and try again.".to_string());
    }
    match tab_match {
        Some(needle) if !needle.is_empty() => {
            let lower = needle.to_ascii_lowercase();
            tabs.into_iter()
                .find(|t| {
                    t.url.to_ascii_lowercase().contains(&lower)
                        || t.title.to_ascii_lowercase().contains(&lower)
                })
                .ok_or_else(|| {
                    format!("No tab matches \"{needle}\". Use list_tabs to see what's open.")
                })
        }
        _ => Ok(tabs.into_iter().next().unwrap()),
    }
}

// ---------- CDP WebSocket plumbing ----------

/// Send one CDP command and await its response. Opens a fresh
/// socket per call — fine for our single-command workflow and
/// avoids state-management complexity.
async fn cdp_call(ws_url: &str, method: &str, params: Value) -> Result<Value, String> {
    let (mut socket, _response) =
        tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|error| format!("CDP connect {ws_url}: {error}"))?;

    let id = RPC_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let request = json!({ "id": id, "method": method, "params": params });
    socket
        .send(Message::Text(request.to_string()))
        .await
        .map_err(|error| format!("CDP send: {error}"))?;

    // Loop until we see the response with matching id. CDP socket
    // streams events too (Page.frameNavigated etc); we ignore
    // anything that isn't our reply.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("CDP {method} timed out after 10s"));
        }
        let next = tokio::time::timeout(remaining, socket.next()).await;
        let Ok(Some(message)) = next else {
            return Err(format!("CDP {method} socket closed before reply"));
        };
        let Ok(message) = message else {
            return Err(format!("CDP {method} socket error"));
        };
        let Message::Text(text) = message else { continue };
        let Ok(value): Result<Value, _> = serde_json::from_str(&text) else {
            continue;
        };
        if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
            if let Some(error) = value.get("error") {
                return Err(format!("CDP {method}: {error}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

// ---------- handlers ----------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OpenUrlArgs {
    url: String,
    #[serde(default)]
    new_tab: bool,
}

async fn open_url(args: Value) -> Result<Value, String> {
    let parsed: OpenUrlArgs = parse_args(args)?;
    if parsed.new_tab {
        // /json/new?url=... creates a new tab via the DevTools HTTP
        // endpoint — simpler than driving Target.createTarget over WS.
        let url = format!(
            "http://localhost:{DEFAULT_DEBUG_PORT}/json/new?{}",
            urlencoding(&parsed.url)
        );
        let response = http_client()?
            .put(&url)
            .send()
            .await
            .map_err(|error| format!("CDP new tab: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("CDP new tab {}", response.status()));
        }
        let tab: Tab = response.json().await.map_err(|e| e.to_string())?;
        return Ok(json!({ "tabId": tab.id, "url": tab.url }));
    }
    let tab = pick_tab(None).await?;
    cdp_call(&tab.ws_url, "Page.navigate", json!({ "url": parsed.url })).await?;
    Ok(json!({ "tabId": tab.id, "url": parsed.url }))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TabMatchArgs {
    #[serde(default)]
    tab_match: Option<String>,
}

async fn current_url(args: Value) -> Result<Value, String> {
    let parsed: TabMatchArgs = parse_args(args).unwrap_or_default();
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    Ok(json!({ "url": tab.url }))
}

async fn current_title(args: Value) -> Result<Value, String> {
    let parsed: TabMatchArgs = parse_args(args).unwrap_or_default();
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    Ok(json!({ "title": tab.title }))
}

async fn read_page_text(args: Value) -> Result<Value, String> {
    let parsed: TabMatchArgs = parse_args(args).unwrap_or_default();
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    let result = cdp_call(
        &tab.ws_url,
        "Runtime.evaluate",
        json!({
            "expression": "document.body.innerText",
            "returnByValue": true,
        }),
    )
    .await?;
    let text = result
        .pointer("/result/value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(json!({ "text": text, "url": tab.url, "title": tab.title }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClickTextArgs {
    text: String,
    #[serde(default)]
    tab_match: Option<String>,
}

async fn click_text(args: Value) -> Result<Value, String> {
    let parsed: ClickTextArgs = parse_args(args)?;
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    // Find an element whose visible text contains the needle, scroll
    // it into view, and click. XPath case-insensitive contains is
    // verbose but works on every page including SPAs.
    let needle_lower = parsed.text.to_ascii_lowercase().replace('"', "\\\"");
    let expression = format!(
        "(() => {{\n\
            const xp = `//*[contains(translate(normalize-space(.), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz'), \"{needle}\")][not(self::script or self::style)]`;\n\
            const r = document.evaluate(xp, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null);\n\
            const el = r.singleNodeValue;\n\
            if (!el) return false;\n\
            el.scrollIntoView({{ block: 'center' }});\n\
            el.click();\n\
            return true;\n\
        }})()",
        needle = needle_lower
    );
    let result = cdp_call(
        &tab.ws_url,
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true }),
    )
    .await?;
    let clicked = result
        .pointer("/result/value")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !clicked {
        return Err(format!(
            "Couldn't find an element matching \"{}\" on this page.",
            parsed.text
        ));
    }
    Ok(json!({ "clicked": true }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillFieldArgs {
    /// CSS selector for the input. e.g. 'input[name="q"]' or '#search'.
    selector: String,
    value: String,
    #[serde(default)]
    tab_match: Option<String>,
}

async fn fill_field(args: Value) -> Result<Value, String> {
    let parsed: FillFieldArgs = parse_args(args)?;
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    let selector = parsed.selector.replace('"', "\\\"");
    let value = parsed.value.replace('"', "\\\"");
    let expression = format!(
        "(() => {{\n\
            const el = document.querySelector(\"{selector}\");\n\
            if (!el) return false;\n\
            const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;\n\
            const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;\n\
            setter.call(el, \"{value}\");\n\
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));\n\
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));\n\
            return true;\n\
        }})()"
    );
    let result = cdp_call(
        &tab.ws_url,
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true }),
    )
    .await?;
    let filled = result
        .pointer("/result/value")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !filled {
        return Err(format!("No element matched selector \"{}\"", parsed.selector));
    }
    Ok(json!({ "filled": true }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecuteJsArgs {
    expression: String,
    #[serde(default)]
    tab_match: Option<String>,
}

async fn execute_js(args: Value) -> Result<Value, String> {
    let parsed: ExecuteJsArgs = parse_args(args)?;
    let tab = pick_tab(parsed.tab_match.as_deref()).await?;
    let result = cdp_call(
        &tab.ws_url,
        "Runtime.evaluate",
        json!({
            "expression": parsed.expression,
            "returnByValue": true,
            "awaitPromise": true,
        }),
    )
    .await?;
    Ok(json!({ "result": result.pointer("/result/value").cloned().unwrap_or(Value::Null) }))
}

/// Minimal URL encoder for the /json/new?url=… query string. We
/// only need it for that single endpoint — `urlencoding` crate
/// would be overkill for one call.
fn urlencoding(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoder_preserves_url_safe_chars() {
        assert_eq!(urlencoding("https://example.com/path"), "https://example.com/path");
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("?q=foo"), "%3Fq%3Dfoo");
    }

    #[test]
    fn manifest_supports_every_platform() {
        let m = manifest();
        // The browser adapter is the most cross-platform thing in
        // the bundle — Chrome runs on everything. If we ever drop a
        // platform from this list, the e2e UX for the long-tail of
        // webapps breaks for those users.
        for os in ["macos", "windows", "linux"] {
            assert!(
                m.supported_platforms.iter().any(|p| p == os),
                "browser adapter missing {os}"
            );
        }
    }
}
