// Text rewrite + agentic LLM dispatch.
//
// This module is the "sub-agent" arm of the orchestrator pattern:
// Gemini Live (voice/vision/realtime) runs the dispatch loop and
// calls `rewrite_selection` as a tool whenever the user wants prose
// transformed. Each call hits a stateless text-only LLM picked per
// the active persona — Gemini, Grok (xAI), Groq, Cerebras, Together,
// Fireworks, Anthropic, or OpenAI — so we get fast inference + per-
// task model picks without burning Live-channel tokens.
//
// Public surface:
//   - `RewriteRequest` — serialized command from the frontend
//   - `rewrite_selection` — tauri command, returns the rewritten markdown
//   - `list_providers` — tauri command, exposes the provider catalog
//                       to the Personas settings UI
//   - `open_drafting_window_with_text` — tauri command that opens the
//     drafting webview and pre-fills it with markdown
//
// The clipboard rich-text helper that pastes back into Word / Docs /
// Mail lives in `clipboard_rich.rs`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::keychain;
use crate::personas;

pub mod clipboard_rich;
pub mod providers;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteRequest {
    /// Persona id to dispatch to. Resolves the system prompt, model,
    /// provider, reasoning, and temperature. If empty, falls back to
    /// the currently active persona.
    #[serde(default)]
    pub persona_id: String,
    /// User instruction (e.g. "rewrite this as a formal email").
    pub instruction: String,
    /// Raw selected text from the foreground app. Pasted into the
    /// prompt verbatim.
    pub selection: String,
    /// Optional override for the response format. "markdown" (default)
    /// makes the model emit markdown, which we then convert to HTML
    /// for the rich-text clipboard payload.
    #[serde(default = "default_format")]
    pub output_format: String,
}

fn default_format() -> String {
    "markdown".to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteResponse {
    /// Markdown the model emitted. Already trimmed of any preamble
    /// the provider added ("Here's your rewrite:" etc).
    pub markdown: String,
    /// HTML rendering of the markdown — what we put on the
    /// clipboard's `text/html` slot so Word / Docs / Mail receive
    /// styled paste output.
    pub html: String,
    /// Provider + model the request actually hit, surfaced so the UI
    /// can show a "via Cerebras / llama3.1-8b" badge under the draft.
    pub provider: String,
    pub model: String,
    /// Milliseconds end-to-end. Useful for the UI to show speed
    /// badges and for the audit harness to detect regressions.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalogEntry {
    pub id: String,
    pub display_name: String,
    /// Whether the provider supports an explicit reasoning toggle.
    /// Used by the Personas UI to enable/disable the "Reasoning"
    /// switch per provider/model combo.
    pub supports_reasoning: bool,
    /// Suggested model ids — the UI shows these in a dropdown but
    /// the user can type a custom id since providers add models
    /// faster than we can ship token-sheet updates.
    pub suggested_models: Vec<String>,
    /// Keychain-account key used for this provider, surfaced so the
    /// Personas UI knows which entry to read/write when the user
    /// adds an API key for a non-Gemini provider.
    pub keychain_account: String,
}

#[tauri::command]
pub fn list_providers() -> Vec<ProviderCatalogEntry> {
    providers::CATALOG
        .iter()
        .map(|(id, name, reasoning, models)| ProviderCatalogEntry {
            id: (*id).to_string(),
            display_name: (*name).to_string(),
            supports_reasoning: *reasoning,
            suggested_models: models.iter().map(|m| (*m).to_string()).collect(),
            keychain_account: format!("{}-api-key", id.to_ascii_lowercase()),
        })
        .collect()
}

#[tauri::command]
pub async fn rewrite_selection(
    app: AppHandle,
    request: RewriteRequest,
) -> Result<RewriteResponse, String> {
    let started_at = std::time::Instant::now();

    // 1. Resolve the persona. Empty persona_id => use whatever is
    //    currently active so the frontend doesn't have to fetch it
    //    just to pass it back in.
    let persona = if request.persona_id.is_empty() {
        personas::get_active_persona()?
    } else {
        personas::list_personas()?
            .into_iter()
            .find(|p| p.id == request.persona_id)
            .ok_or_else(|| format!("unknown persona id: {}", request.persona_id))?
    };

    // 2. Read the provider's API key from the keychain. Surfacing
    //    this error clearly is important because most provider
    //    failures we see in practice are a missing key, not a bad
    //    response.
    let api_key = keychain::read_provider_key(&persona.model_provider)?
        .ok_or_else(|| {
            format!(
                "No API key configured for provider '{}'. Add one in Settings → Personas.",
                persona.model_provider
            )
        })?;

    // 3. Dispatch. Each provider impl returns plain text (no
    //    streaming yet — streaming is wired separately for the
    //    drafting window via `rewrite_selection_streaming`).
    let provider = providers::resolve(&persona.model_provider)
        .ok_or_else(|| format!("unsupported provider: {}", persona.model_provider))?;

    let markdown = provider
        .complete(providers::CompletionRequest {
            api_key: api_key.clone(),
            model: persona.model_id.clone(),
            // Prepend ISO-8601 current datetime so the model resolves
            // relative dates ("tomorrow", "next Friday at 2pm") without
            // asking the user to clarify. Live test confirmed this is a
            // real gap when tool calls don't carry user-clock context.
            system_prompt: format!(
                "Current date and time: {}.\n\n{}",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
                persona.system_prompt
            ),
            user_prompt: format!(
                "{}\n\n---\nSelection:\n{}",
                request.instruction.trim(),
                request.selection
            ),
            temperature: persona.temperature,
            reasoning_enabled: persona.reasoning_enabled,
        })
        .await?;

    // 4. Convert to HTML for the rich-text clipboard payload.
    let html = markdown_to_html(&markdown);

    // 5. Emit a progress event so the drafting window (and any other
    //    listening webview) can pick up the result without polling.
    let response = RewriteResponse {
        markdown: markdown.clone(),
        html: html.clone(),
        provider: persona.model_provider.clone(),
        model: persona.model_id.clone(),
        elapsed_ms: started_at.elapsed().as_millis() as u64,
    };
    let _ = app.emit("rewrite_complete", &response);

    Ok(response)
}

/// Convert markdown to HTML using pulldown-cmark with sensible
/// extensions (tables + footnotes + strikethrough). The output is
/// safe to put on the `text/html` clipboard slot.
pub fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut buffer = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut buffer, parser);
    buffer
}

/// Open (or show + focus) the drafting webview and seed it with the
/// given markdown. Called by `rewrite_selection_into_drafting_window`
/// and from the frontend "Open in editor" button.
#[tauri::command]
pub async fn open_drafting_window_with_text(
    app: AppHandle,
    markdown: String,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("drafting") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit("drafting_seed_text", &markdown);
        return Ok(());
    }
    // The drafting window is declared in tauri.conf.json; if it's
    // missing, fail loudly so misconfigured builds get caught.
    Err("drafting window not found in app config".to_string())
}

/// End-to-end helper: rewrite + open + clipboard. The frontend
/// command-palette / hotkey path calls this so a single voice
/// command goes from selection → editor → clipboard.
#[tauri::command]
pub async fn rewrite_selection_into_drafting_window(
    app: AppHandle,
    request: RewriteRequest,
) -> Result<RewriteResponse, String> {
    let response = rewrite_selection(app.clone(), request).await?;
    clipboard_rich::put_markdown_and_html(&response.markdown, &response.html)?;
    open_drafting_window_with_text(app, response.markdown.clone()).await?;
    Ok(response)
}

/// Paste-from-drafting helper. The drafting window's "Replace
/// selection" button calls this after putting the rich payload on
/// the clipboard.
///
/// The catch: when the user clicks the button, the drafting window
/// itself has focus — so a naive Cmd+V would paste into the
/// editor. We hide the drafting window first so the OS's normal
/// "focus the previously-focused window" handler kicks in, then
/// wait a short settle interval, then fire Cmd+V. The drafting
/// window comes back when the user re-opens it via the panel or
/// the next rewrite.
#[tauri::command]
pub fn paste_from_drafting_window(app: AppHandle) -> Result<(), String> {
    use crate::executor::cross_platform_input;

    if let Some(window) = app.get_webview_window("drafting") {
        // Hide rather than close so the next rewrite can re-show
        // the same window with its draft contents preserved.
        let _ = window.hide();
    }
    // Window-server settle. The OS needs ~150ms to flip focus back
    // to the previously-active app; firing Cmd+V too early lands
    // the paste in whatever Tauri's parent window happens to be.
    std::thread::sleep(std::time::Duration::from_millis(180));
    let paste_keys = ["Cmd", "V"];
    cross_platform_input::keyboard_shortcut(&paste_keys)
}
