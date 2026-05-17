// Rust port of `src/gemini/GeminiLiveClient.ts`. Speaks the same wire
// format Google's Gemini Live BidiGenerateContent endpoint expects.
//
// Why a Rust port: sub-agent conversational loops run server-side
// (inside the Tauri host process) so they keep working when the user
// closes the panel webview. The TS client still drives the user-facing
// panel session — both paths share the same wire format intentionally
// so a fix on one side ports straight across.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const GEMINI_LIVE_WS_URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

const SETUP_COMPLETE_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone)]
pub enum InboundMessage {
    SetupComplete,
    AudioChunk { pcm_24khz: Vec<u8> },
    InputTranscript { text: String },
    OutputTranscript { text: String },
    TurnComplete,
    ToolCall {
        name: String,
        args: Value,
        tool_call_id: String,
    },
    UsageReport {
        input_tokens: u32,
        output_tokens: u32,
    },
    Error { message: String },
    Closed { reason: String },
}

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub api_key: String,
    pub model_id: String,
    pub voice_name: String,
    pub system_instruction: String,
    /// JSON value matching `[{ functionDeclarations: [...] }]`. Pass an
    /// empty array `[]` to disable tools entirely.
    pub tool_declarations: Value,
    /// When true, request `responseModalities: ["TEXT"]` and skip the
    /// speech config so the server returns transcript-only responses.
    /// Sub-agents use this; the panel session leaves it false.
    pub disable_audio_output: bool,
}

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;

pub struct GeminiLiveClient {
    // Writer half of the websocket. Reader half is owned by the
    // background reader task spawned in `connect`.
    sink: Arc<Mutex<WsSink>>,
}

impl GeminiLiveClient {
    /// Open the websocket, send the `setup` frame, and wait for
    /// `setupComplete`. Returns the client plus a channel that streams
    /// inbound messages parsed from server frames.
    pub async fn connect(
        options: ClientOptions,
    ) -> Result<(Self, mpsc::Receiver<InboundMessage>), String> {
        let url = format!(
            "{}?key={}",
            GEMINI_LIVE_WS_URL,
            urlencode(&options.api_key)
        );

        let (ws_stream, _response) = connect_async(&url)
            .await
            .map_err(|websocket_connect_error| {
                format!("websocket connect failed: {websocket_connect_error}")
            })?;
        let (mut sink, mut stream) = ws_stream.split();

        // Build and send the setup frame BEFORE we hand the sink off to
        // the shared Arc<Mutex<_>>, so we don't contend on the lock with
        // the reader's setupComplete wait.
        let setup_payload = build_setup_payload(&options);
        sink.send(Message::Text(setup_payload.to_string()))
            .await
            .map_err(|send_error| format!("send setup frame: {send_error}"))?;

        let sink_arc = Arc::new(Mutex::new(sink));
        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundMessage>(64);
        let (setup_complete_tx, setup_complete_rx) = tokio::sync::oneshot::channel::<()>();

        // Background reader task. Translates raw frames into our typed
        // `InboundMessage` enum and pushes them down the mpsc channel.
        // Also fires the setup-complete oneshot the first time it sees
        // a `setupComplete` server frame.
        let inbound_tx_for_reader = inbound_tx.clone();
        tokio::spawn(async move {
            let mut setup_complete_signal = Some(setup_complete_tx);
            while let Some(next_frame) = stream.next().await {
                match next_frame {
                    Ok(Message::Text(text)) => {
                        dispatch_server_frame(
                            &text,
                            &inbound_tx_for_reader,
                            &mut setup_complete_signal,
                        )
                        .await;
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            dispatch_server_frame(
                                text,
                                &inbound_tx_for_reader,
                                &mut setup_complete_signal,
                            )
                            .await;
                        }
                    }
                    Ok(Message::Close(close_frame)) => {
                        let reason = close_frame
                            .map(|c| format!("code {}: {}", c.code, c.reason))
                            .unwrap_or_else(|| "no close frame".to_string());
                        let _ = inbound_tx_for_reader
                            .send(InboundMessage::Closed { reason })
                            .await;
                        break;
                    }
                    Ok(_) => {
                        // Ping/Pong/Frame — ignore.
                    }
                    Err(websocket_read_error) => {
                        let _ = inbound_tx_for_reader
                            .send(InboundMessage::Error {
                                message: format!("ws read: {websocket_read_error}"),
                            })
                            .await;
                        break;
                    }
                }
            }
        });

        // Wait for setupComplete with a timeout. If the server rejects
        // the setup (bad key, retired model) it closes the socket and
        // the reader task's Closed message arrives — but we'd still time
        // out here, so the timeout doubles as a generic "session never
        // got off the ground" error.
        let setup_outcome = tokio::time::timeout(
            std::time::Duration::from_millis(SETUP_COMPLETE_TIMEOUT_MS),
            setup_complete_rx,
        )
        .await;
        match setup_outcome {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err("setup oneshot dropped before completion".to_string()),
            Err(_) => return Err(
                "Gemini setupComplete timeout — bad key, retired model, or network".to_string(),
            ),
        }

        Ok((Self { sink: sink_arc }, inbound_rx))
    }

    /// Send a textual user turn over the live socket. Used for both the
    /// first message (the task description) and for follow-ups, since
    /// sub-agents have no microphone stream.
    pub async fn send_text_turn(&self, text: &str) -> Result<(), String> {
        let payload = json!({
            "clientContent": {
                "turns": [
                    {
                        "role": "user",
                        "parts": [{ "text": text }],
                    }
                ],
                "turnComplete": true,
            }
        });
        self.send_json(payload).await
    }

    /// Acknowledge a tool call back to Gemini. The model treats this as
    /// the result of the function it requested and continues reasoning.
    pub async fn send_tool_response(
        &self,
        tool_call_id: &str,
        response: Value,
    ) -> Result<(), String> {
        let payload = json!({
            "toolResponse": {
                "functionResponses": [
                    {
                        "id": tool_call_id,
                        "response": response,
                    }
                ]
            }
        });
        self.send_json(payload).await
    }

    pub async fn close(&self) {
        let mut sink = self.sink.lock().await;
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
    }

    async fn send_json(&self, payload: Value) -> Result<(), String> {
        let mut sink = self.sink.lock().await;
        sink.send(Message::Text(payload.to_string()))
            .await
            .map_err(|send_error| format!("ws send: {send_error}"))
    }
}

fn build_setup_payload(options: &ClientOptions) -> Value {
    let response_modalities = if options.disable_audio_output {
        json!(["TEXT"])
    } else {
        json!(["AUDIO"])
    };
    let mut generation_config = json!({
        "responseModalities": response_modalities,
        "mediaResolution": "MEDIA_RESOLUTION_MEDIUM",
    });
    if !options.disable_audio_output {
        generation_config["speechConfig"] = json!({
            "voiceConfig": {
                "prebuiltVoiceConfig": { "voiceName": options.voice_name }
            }
        });
    }
    let mut setup_object = json!({
        "model": format!("models/{}", options.model_id),
        "generationConfig": generation_config,
        "systemInstruction": {
            "parts": [{ "text": options.system_instruction }]
        },
        // Long-session safety: have the server slide-compress old turns
        // once the accumulated context approaches the limit, rather
        // than dropping responses silently. Same numbers as the TS
        // client uses.
        "contextWindowCompression": {
            "triggerTokens": 104857,
            "slidingWindow": { "targetTokens": 52428 }
        }
    });
    // Text-only sub-agents don't get audio transcription stanzas — they
    // have no audio in or out. The panel session keeps them.
    if !options.disable_audio_output {
        setup_object["inputAudioTranscription"] = json!({});
        setup_object["outputAudioTranscription"] = json!({});
    }
    // Tools are optional: an empty array disables function calling.
    let tools_array = options.tool_declarations.clone();
    let has_tools = matches!(&tools_array, Value::Array(arr) if !arr.is_empty());
    if has_tools {
        setup_object["tools"] = tools_array;
    }
    json!({ "setup": setup_object })
}

async fn dispatch_server_frame(
    text: &str,
    inbound_tx: &mpsc::Sender<InboundMessage>,
    setup_complete_signal: &mut Option<tokio::sync::oneshot::Sender<()>>,
) {
    let parsed: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            let _ = inbound_tx
                .send(InboundMessage::Error {
                    message: "non-JSON frame".to_string(),
                })
                .await;
            return;
        }
    };

    if parsed.get("setupComplete").is_some() {
        if let Some(signal) = setup_complete_signal.take() {
            let _ = signal.send(());
        }
        let _ = inbound_tx.send(InboundMessage::SetupComplete).await;
        return;
    }

    if let Some(server_content) = parsed.get("serverContent") {
        if let Some(model_turn) = server_content.get("modelTurn") {
            if let Some(parts) = model_turn.get("parts").and_then(|p| p.as_array()) {
                for part in parts {
                    if let Some(inline_data) = part.get("inlineData") {
                        let mime_type = inline_data
                            .get("mimeType")
                            .and_then(|m| m.as_str())
                            .unwrap_or("");
                        if mime_type.starts_with("audio/pcm") {
                            if let Some(b64) = inline_data.get("data").and_then(|d| d.as_str()) {
                                if let Ok(decoded) = BASE64_STANDARD.decode(b64) {
                                    let _ = inbound_tx
                                        .send(InboundMessage::AudioChunk {
                                            pcm_24khz: decoded,
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                    // Text parts come through here too for TEXT modality.
                    if let Some(text_part) =
                        part.get("text").and_then(|t| t.as_str())
                    {
                        if !text_part.is_empty() {
                            let _ = inbound_tx
                                .send(InboundMessage::OutputTranscript {
                                    text: text_part.to_string(),
                                })
                                .await;
                        }
                    }
                }
            }
        }
        if let Some(input_transcript_text) = server_content
            .get("inputTranscription")
            .and_then(|t| t.get("text"))
            .and_then(|t| t.as_str())
        {
            let _ = inbound_tx
                .send(InboundMessage::InputTranscript {
                    text: input_transcript_text.to_string(),
                })
                .await;
        }
        if let Some(output_transcript_text) = server_content
            .get("outputTranscription")
            .and_then(|t| t.get("text"))
            .and_then(|t| t.as_str())
        {
            let _ = inbound_tx
                .send(InboundMessage::OutputTranscript {
                    text: output_transcript_text.to_string(),
                })
                .await;
        }
        if server_content
            .get("turnComplete")
            .and_then(|tc| tc.as_bool())
            .unwrap_or(false)
        {
            let _ = inbound_tx.send(InboundMessage::TurnComplete).await;
        }
    }

    if let Some(tool_call) = parsed.get("toolCall") {
        if let Some(function_calls) =
            tool_call.get("functionCalls").and_then(|f| f.as_array())
        {
            for function_call in function_calls {
                let name = function_call
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = function_call.get("args").cloned().unwrap_or(Value::Null);
                let tool_call_id = function_call
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = inbound_tx
                    .send(InboundMessage::ToolCall {
                        name,
                        args,
                        tool_call_id,
                    })
                    .await;
            }
        }
    }

    // Usage metadata appears on most frames once tokens have been
    // consumed. We re-emit it as a typed message so the conversational
    // loop can enforce its dollar budget.
    if let Some(usage_metadata) = parsed.get("usageMetadata") {
        let input_tokens = usage_metadata
            .get("promptTokenCount")
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let output_tokens = usage_metadata
            .get("responseTokenCount")
            .and_then(|t| t.as_u64())
            .or_else(|| usage_metadata.get("candidatesTokenCount").and_then(|t| t.as_u64()))
            .unwrap_or(0) as u32;
        if input_tokens > 0 || output_tokens > 0 {
            let _ = inbound_tx
                .send(InboundMessage::UsageReport {
                    input_tokens,
                    output_tokens,
                })
                .await;
        }
    }
}

// Minimal percent-encoder for the API key query param. We avoid pulling
// in a full url-encoding crate for one call site.
fn urlencode(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if is_unreserved {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}
