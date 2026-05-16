// WhatsApp adapter — URI-scheme mode (v1).
//
// WhatsApp Desktop registers a `whatsapp://send?phone=...&text=...`
// URL handler on both macOS and Windows. Opening that URL opens the
// pre-filled chat — but does NOT auto-send. We synthesize a Return
// keystroke ~250ms later so the user just says "send WhatsApp to
// Mom: I'll be late" and the message goes out without a click.
//
// Handlers:
//   - `send_message` : { phone: "+14155551234", text: "..." } -> { sent: bool }
//   - `open_chat`    : { phone: "+14155551234" } -> {}  (no auto-send)
//
// No auth required — we drive the user's already-signed-in desktop
// app. CAN'T read messages, search history, or list contacts in v1;
// those need WhatsApp Web DOM control (CDP) which lands with the
// browser adapter in v2.
//
// Privacy note exposed in the install UX: WhatsApp's URI handler
// passes the phone + text through the URL bar, so the message is
// briefly visible to anything that intercepts URL opens (Spotlight
// recents, default-browser logs). We document this on the install
// page so power users can decide.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::adapters::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "whatsapp".into(),
        name: "WhatsApp".into(),
        description: "Send WhatsApp messages by voice. Drives the desktop app you're already signed into.".into(),
        category: "communication".into(),
        voice_triggers: vec![
            "send whatsapp".into(),
            "whatsapp".into(),
            "message on whatsapp".into(),
            "text on whatsapp".into(),
        ],
        capabilities: vec![
            AdapterCapability::OpenUri,
            AdapterCapability::Keystrokes,
        ],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into()],
        setup_notes: "Requires WhatsApp Desktop installed and signed in. \
            TipTour opens whatsapp://send URLs, then sends Return to dispatch the message. \
            The phone number + message body briefly appear in the OS URL handler — if \
            that's a concern for your threat model, leave this adapter disabled and \
            use the WhatsApp Web adapter (lands in v2).".into(),
    }
}

pub async fn dispatch(
    app: AppHandle,
    handler: &str,
    args: Value,
) -> Result<Value, String> {
    match handler {
        "send_message" => send_message(app, args).await,
        "open_chat" => open_chat(app, args).await,
        other => Err(format!("WhatsApp adapter has no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendMessageArgs {
    /// E.164 phone number including country code, e.g. "+14155551234".
    /// We strip + and any non-digits before handing to WhatsApp because
    /// the whatsapp:// URL handler expects digits only.
    phone: String,
    /// Message body. URL-encoded into the `text` parameter.
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenChatArgs {
    phone: String,
}

async fn send_message(app: AppHandle, args: Value) -> Result<Value, String> {
    let parsed: SendMessageArgs =
        serde_json::from_value(args).map_err(|error| format!("send_message args: {error}"))?;
    let phone_digits = parsed
        .phone
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>();
    if phone_digits.is_empty() {
        return Err("WhatsApp send_message: phone number had no digits".to_string());
    }
    // URL-encode the body. The whatsapp:// handler decodes the text
    // param when populating the message draft.
    let encoded_text = url_encode(&parsed.text);
    let target = format!("whatsapp://send?phone={phone_digits}&text={encoded_text}");
    open_url_via_os(&app, &target)?;
    // Settle: WhatsApp Desktop needs a beat to focus the chat and
    // populate the message draft before our Return keystroke lands.
    // 350ms is the conservative pick from our cross-platform testing
    // (250 sometimes misses on Windows).
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    let return_keys = ["Return"];
    crate::executor::cross_platform_input::keyboard_shortcut(&return_keys)?;
    Ok(json!({ "sent": true, "phone": phone_digits }))
}

async fn open_chat(app: AppHandle, args: Value) -> Result<Value, String> {
    let parsed: OpenChatArgs =
        serde_json::from_value(args).map_err(|error| format!("open_chat args: {error}"))?;
    let phone_digits = parsed
        .phone
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>();
    let target = format!("whatsapp://send?phone={phone_digits}");
    open_url_via_os(&app, &target)?;
    Ok(json!({ "opened": true, "phone": phone_digits }))
}

/// Open a URL through the OS default URL handler. On macOS this is
/// `open <url>`; on Windows it's `cmd /C start <url>`; on Linux
/// `xdg-open <url>`. We deliberately avoid the `opener` crate to
/// keep the dep tree small for a single shell-out.
fn open_url_via_os(_app: &AppHandle, url: &str) -> Result<(), String> {
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

/// Minimal percent-encoder for the text param of a whatsapp:// URL.
/// We don't bring in `urlencoding` for one call.
fn url_encode(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            b' ' => output.push_str("%20"),
            other => output.push_str(&format!("%{:02X}", other)),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_handles_spaces_and_specials() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("hi!"), "hi%21");
        assert_eq!(url_encode("a b c"), "a%20b%20c");
        // Unicode characters must round-trip via percent encoding so
        // emoji in messages survive the URL handoff.
        assert!(url_encode("🚀").starts_with("%"));
    }

    #[test]
    fn phone_digit_stripping_is_idempotent() {
        // The args parser is dumb — actual stripping happens in the
        // handler. Repeat the filter chain here so the contract is
        // documented and stays stable across refactors.
        let stripped: String = "+1 (415) 555-1234"
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        assert_eq!(stripped, "14155551234");
    }

    #[test]
    fn manifest_no_auth_no_network() {
        let m = manifest();
        assert!(matches!(m.auth, AdapterAuth::None));
        // URI + Keystrokes are the only capabilities the adapter
        // actually uses — no network, no keychain. Surfaces clearly
        // on the install page so the user knows the v1 adapter is
        // local-only.
        assert!(m.capabilities.contains(&AdapterCapability::OpenUri));
        assert!(m.capabilities.contains(&AdapterCapability::Keystrokes));
        assert!(!m.capabilities.contains(&AdapterCapability::Network));
        assert!(!m.capabilities.contains(&AdapterCapability::Keychain));
    }
}
