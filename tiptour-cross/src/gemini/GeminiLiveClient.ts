// Low-level WebSocket client for Google's Gemini Live API.
//
// Mirrors the shape of TipTour/GeminiLiveClient.swift on the macOS side so
// session logic ports straight across. This file owns only the wire format:
// frame in, frame out, no audio device handling, no UI state, no tool
// dispatch — those live in GeminiLiveSession.ts and the Rust audio bridge.

import { invoke } from "@tauri-apps/api/core";
import { base64ToPcm16, pcm16ToBase64 } from "./audio";

interface ActivePersonaShape {
  id: string;
  name: string;
  systemPrompt: string;
  voiceTriggerPhrases: string[];
  isBuiltIn: boolean;
}

const GEMINI_LIVE_WS_URL =
  "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

// Default model used by the Swift TipTour app (May 2026). The old
// gemini-2.0-flash-exp endpoint is retired and silently rejects sessions.
// The Settings dashboard can override this at runtime via the model
// dropdown — see `loadGeminiSessionConfigFromSettings`.
const DEFAULT_MODEL_SHORT_ID = "gemini-3.1-flash-live-preview";

// "Kore" is the voice 3.1 Flash Live accepts; older voices like "Aoede"
// cause the server to close the socket before setupComplete on older
// models. We still let the user pick — the model decides whether to
// honour the choice.
const DEFAULT_VOICE = "Kore";

const SETUP_COMPLETE_TIMEOUT_MS = 10_000;

export type GeminiInboundMessage =
  | { kind: "setup_complete" }
  | { kind: "audio_chunk"; pcm24kHz: Uint8Array }
  | { kind: "input_transcript"; text: string }
  | { kind: "output_transcript"; text: string }
  | { kind: "turn_complete" }
  | { kind: "tool_call"; name: string; args: unknown; toolCallId: string }
  | { kind: "error"; message: string };

export interface GeminiLiveClientOptions {
  apiKey: string;
  /// Optional voice name from app settings; falls back to "Kore" when
  /// not supplied so callers that don't read settings keep working.
  voiceName?: string;
  /// Optional model short id (e.g. "gemini-3.1-flash-live-preview").
  /// Falls back to the default.
  modelShortId?: string;
  onMessage: (message: GeminiInboundMessage) => void;
  onClose: (reason: string) => void;
}

export class GeminiLiveClient {
  private socket: WebSocket | null = null;
  private setupComplete = false;
  private setupCompletePromise: Promise<void> | null = null;
  private resolveSetupComplete: (() => void) | null = null;
  private rejectSetupComplete: ((error: Error) => void) | null = null;
  private readonly options: GeminiLiveClientOptions;

  constructor(options: GeminiLiveClientOptions) {
    this.options = options;
  }

  async open(): Promise<void> {
    const url = `${GEMINI_LIVE_WS_URL}?key=${encodeURIComponent(this.options.apiKey)}`;
    console.info("[gemini] opening websocket");
    this.socket = new WebSocket(url);
    this.socket.binaryType = "arraybuffer";

    await new Promise<void>((resolve, reject) => {
      if (!this.socket) return reject(new Error("Socket gone before handshake"));
      this.socket.onopen = () => {
        console.info("[gemini] websocket open");
        resolve();
      };
      this.socket.onerror = (event) => {
        console.error("[gemini] websocket error", event);
        reject(new Error("WebSocket failed to open — check internet and API key"));
      };
    });

    this.socket!.onmessage = (event) => this.handleRawMessage(event.data);
    this.socket!.onclose = (event) => {
      const reason = event.reason || `code ${event.code}`;
      console.warn("[gemini] websocket closed:", reason);
      this.rejectSetupComplete?.(new Error(`Socket closed before setupComplete: ${reason}`));
      this.options.onClose(reason);
    };

    this.setupCompletePromise = new Promise((resolve, reject) => {
      this.resolveSetupComplete = resolve;
      this.rejectSetupComplete = reject;
    });

    await this.sendSetup();

    const timeout = new Promise<never>((_, reject) =>
      setTimeout(
        () => reject(new Error("Gemini setupComplete timeout — bad key or unsupported model?")),
        SETUP_COMPLETE_TIMEOUT_MS,
      ),
    );
    await Promise.race([this.setupCompletePromise, timeout]);
    console.info("[gemini] setup complete");
  }

  close(): void {
    this.socket?.close();
    this.socket = null;
    this.setupComplete = false;
  }

  sendMicChunk(pcm16: Uint8Array): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    if (!this.setupComplete) return; // server rejects audio before setupComplete
    const payload = {
      realtimeInput: {
        mediaChunks: [
          {
            mimeType: "audio/pcm;rate=16000",
            data: pcm16ToBase64(pcm16),
          },
        ],
      },
    };
    this.socket.send(JSON.stringify(payload));
  }

  // Streams a single JPEG screenshot up to Gemini as a realtime media
  // chunk. Same gating as `sendMicChunk`: silently no-ops before
  // setupComplete or after the socket closes, since either case means
  // the server will reject the payload anyway.
  sendScreenshot(jpegBase64: string): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    if (!this.setupComplete) return;
    const payload = {
      realtimeInput: {
        mediaChunks: [
          {
            mimeType: "image/jpeg",
            data: jpegBase64,
          },
        ],
      },
    };
    this.socket.send(JSON.stringify(payload));
  }

  /// Sends a one-shot textual user turn over the live socket. We use
  /// this to inject "here are facts you remembered last time" context
  /// at session open without re-running the full setup payload.
  sendTextTurn(text: string, isUserTurn: boolean): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    if (!this.setupComplete) return;
    const payload = {
      clientContent: {
        turns: [
          {
            role: isUserTurn ? "user" : "model",
            parts: [{ text }],
          },
        ],
        turnComplete: false,
      },
    };
    this.socket.send(JSON.stringify(payload));
  }

  sendToolResponse(toolCallId: string, response: unknown): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    const payload = {
      toolResponse: {
        functionResponses: [
          {
            id: toolCallId,
            response,
          },
        ],
      },
    };
    this.socket.send(JSON.stringify(payload));
  }

  private async sendSetup(): Promise<void> {
    const resolvedModelShortId = this.options.modelShortId ?? DEFAULT_MODEL_SHORT_ID;
    const resolvedVoiceName = this.options.voiceName ?? DEFAULT_VOICE;
    // Pull the active persona so its system prompt is layered onto the
    // baseline TipTour identity. Failing to read should not block the
    // session — fall back to the baseline-only instruction.
    let activePersonaSystemPrompt = "";
    try {
      const activePersona = await invoke<ActivePersonaShape>("get_active_persona");
      activePersonaSystemPrompt = activePersona?.systemPrompt ?? "";
    } catch (personaError) {
      console.warn("[gemini] get_active_persona failed:", personaError);
    }
    const baselineSystemInstruction =
      "You are TipTour, a helpful voice companion. Reply concisely.";
    const composedSystemInstruction = activePersonaSystemPrompt
      ? `${baselineSystemInstruction}\n\n${activePersonaSystemPrompt}`
      : baselineSystemInstruction;
    const setup = {
      setup: {
        model: `models/${resolvedModelShortId}`,
        generationConfig: {
          responseModalities: ["AUDIO"],
          mediaResolution: "MEDIA_RESOLUTION_MEDIUM",
          speechConfig: {
            voiceConfig: { prebuiltVoiceConfig: { voiceName: resolvedVoiceName } },
          },
        },
        systemInstruction: {
          parts: [
            {
              text: composedSystemInstruction,
            },
          ],
        },
        inputAudioTranscription: {},
        outputAudioTranscription: {},
        // Long-session safety: have the server compress old turns once the
        // accumulated context approaches the limit, instead of dropping
        // responses silently.
        contextWindowCompression: {
          triggerTokens: 104857,
          slidingWindow: { targetTokens: 52428 },
        },
        // Tool surface Gemini can call into. Shape matches the JSON
        // schema the Swift app's GeminiLiveClient.swift sends so the
        // model behaves consistently across platforms.
        tools: [
          {
            functionDeclarations: [
              {
                name: "submit_workflow_plan",
                description:
                  "Submit a multi-step CUA plan to drive the user's computer. The Rust executor grounds each step and dispatches it.",
                parameters: {
                  type: "object",
                  properties: {
                    goal: { type: "string", description: "What the user asked for." },
                    app: {
                      type: "string",
                      description: "Target app name or bundle id; optional.",
                    },
                    steps: {
                      type: "array",
                      description: "Ordered list of plan steps.",
                      items: { type: "object" },
                    },
                  },
                  required: ["steps"],
                },
              },
              {
                name: "remember",
                description:
                  "Save a fact to the agent's persistent memory. The fact will be recalled in future sessions by semantic search.",
                parameters: {
                  type: "object",
                  properties: {
                    key: { type: "string", description: "Short label for the fact." },
                    value: { type: "string", description: "The fact itself." },
                    tags: {
                      type: "array",
                      items: { type: "string" },
                      description: "Optional tags for filtering.",
                    },
                  },
                  required: ["key", "value"],
                },
              },
              {
                name: "recall",
                description:
                  "Semantic search over the agent's memory. Returns up to top_k relevant memories.",
                parameters: {
                  type: "object",
                  properties: {
                    query: { type: "string" },
                    top_k: { type: "number" },
                  },
                  required: ["query"],
                },
              },
              {
                name: "forget",
                description: "Soft-delete a memory by id.",
                parameters: {
                  type: "object",
                  properties: {
                    memory_id: { type: "string" },
                  },
                  required: ["memory_id"],
                },
              },
              {
                name: "list_memories",
                description: "List stored memories, optionally filtered by tag.",
                parameters: {
                  type: "object",
                  properties: {
                    tag: { type: "string" },
                  },
                },
              },
              {
                name: "spawn_subagent",
                description:
                  "Spawn a parallel sub-agent to chase down an independent task. Returns the new sub-agent id.",
                parameters: {
                  type: "object",
                  properties: {
                    name: { type: "string" },
                    task_description: { type: "string" },
                  },
                  required: ["name", "task_description"],
                },
              },
              {
                name: "list_subagents",
                description: "List active and recent sub-agents.",
                parameters: { type: "object", properties: {} },
              },
              {
                name: "cancel_subagent",
                description: "Cancel a running sub-agent by id.",
                parameters: {
                  type: "object",
                  properties: { subagent_id: { type: "string" } },
                  required: ["subagent_id"],
                },
              },
              {
                name: "create_task",
                description:
                  "Create a kanban task. Use this when the user mentions something they want to do later.",
                parameters: {
                  type: "object",
                  properties: {
                    title: { type: "string" },
                    description: { type: "string" },
                  },
                  required: ["title"],
                },
              },
              {
                name: "update_task_status",
                description: "Move a task between kanban columns.",
                parameters: {
                  type: "object",
                  properties: {
                    task_id: { type: "string" },
                    status: {
                      type: "string",
                      enum: ["backlog", "inProgress", "blocked", "done", "cancelled"],
                    },
                  },
                  required: ["task_id", "status"],
                },
              },
              {
                name: "list_tasks",
                description: "List tasks, optionally filtered by status.",
                parameters: {
                  type: "object",
                  properties: { status: { type: "string" } },
                },
              },
              {
                name: "run_saved_flow",
                description:
                  "Replay a previously-recorded multi-app flow by name. Use when the user says things like 'do my morning routine' or 'run the standup flow'. The name is fuzzy-matched against saved flows and their trigger aliases.",
                parameters: {
                  type: "object",
                  properties: {
                    name: {
                      type: "string",
                      description:
                        "The flow name or alias phrase the user spoke. Pass the user's exact words; the matcher normalizes filler.",
                    },
                  },
                  required: ["name"],
                },
              },
              {
                name: "control_app",
                description:
                  "Drive an installed third-party adapter (Spotify, WhatsApp, Mail, Calendar, Notion, Linear, GitHub, Slack, Word, Excel, Safari, Finder, etc.). Use this whenever the user asks to do something IN a specific app — 'play X on Spotify', 'send WhatsApp to Mom: …', 'create a Linear issue', 'add to my Notion log', 'open this in VS Code', 'reveal this in Finder'. Look up the adapter slug + handler from the user's intent. The adapter must be installed + enabled (Settings → Connected apps) for the call to succeed; if it isn't, surface the failure verbatim so the user knows to install it. Available slugs include: spotify, apple-music, whatsapp, mail-macos, messages-macos, slack, calendar-macos, reminders-macos, notes-macos, notion, linear, obsidian, ms-to-do, pages, numbers, keynote, word, excel, powerpoint, outlook-desktop, github, vscode, terminal-macos, finder, file-explorer, safari, browser. The `browser` adapter drives any Chromium-based browser (Chrome / Edge / Arc / Brave / Vivaldi) and unlocks every webapp: ChatGPT, Google Docs / Sheets, YouTube, LinkedIn, Maps, internal tools. Its handlers are: list_tabs, open_url{url,new_tab?}, current_url, current_title, read_page_text{tab_match?}, click_text{text,tab_match?}, fill_field{selector,value,tab_match?}, execute_js{expression,tab_match?}. Each adapter exposes a small set of handlers (e.g. spotify: play_track/pause/resume/next/previous/current; whatsapp: send_message/open_chat; calendar-macos: create_event/list_today). Pass handler args under `args`.",
                parameters: {
                  type: "object",
                  properties: {
                    slug: {
                      type: "string",
                      description:
                        "The adapter slug. Lowercase, hyphen-separated. Examples: 'spotify', 'whatsapp', 'mail-macos', 'github', 'word'.",
                    },
                    handler: {
                      type: "string",
                      description:
                        "The handler name within the adapter, e.g. 'play_track', 'send_message', 'create_issue', 'open_path'.",
                    },
                    args: {
                      type: "object",
                      description:
                        "Handler-specific arguments. CRITICAL: use ONLY the exact field names listed below per handler — do NOT invent variants like 'song_name' or 'track_name' or 'artist_name'. The Rust deserializer rejects unknown fields and the call fails silently if you guess.\n" +
                        "spotify.play_track: {query: '<song or song+artist>'}\n" +
                        "spotify.pause / resume / next / previous / current: {}\n" +
                        "apple-music.play_track: {query: '<song name>'}; .pause / resume / next / previous / current: {}\n" +
                        "whatsapp.send_message: {phone: '<E.164 e.g. +14155551234>', text: '<body>'}; .open_chat: {phone}\n" +
                        "mail-macos.compose / send: {to, cc?, subject, body}\n" +
                        "messages-macos.send: {to, text}\n" +
                        "slack.post_message: {channel, text}; .post_dm: {user_id, text}\n" +
                        "calendar-macos.create_event: {summary, start_iso, end_iso?, location?, notes?}; .list_today: {}\n" +
                        "reminders-macos.add: {text, due_iso?, notes?}; .list_today: {}\n" +
                        "notes-macos.create / append: {title, body}\n" +
                        "notion.create_page: {parent_page_id, title, body}; .append_text: {page_id, text}\n" +
                        "linear.create_issue: {team_key, title, description?, priority?}; .my_issues: {}\n" +
                        "obsidian.open_note: {vault, file}; .create_note: {vault, name, content}; .append_to_daily: {vault, text}\n" +
                        "ms-to-do.add: {title, notes?}\n" +
                        "pages / numbers / keynote.new_document: {}; .open: {path}\n" +
                        "word.open: {path}; .new_document: {}; .save_as_pdf: {input_path, output_path}\n" +
                        "excel.open: {path}; .new_workbook: {}\n" +
                        "powerpoint.open: {path}; .new_presentation: {}; .start_slideshow: {}\n" +
                        "outlook-desktop.compose: {to, subject, body}\n" +
                        "github.create_issue: {repo: '<owner/repo>', title, body?, labels?}; .my_review_requests: {}\n" +
                        "vscode.open / open_in_cursor: {path}; .goto_line: {path, line, column?}\n" +
                        "terminal-macos.run: {command, cwd?}; .open_cwd: {path}\n" +
                        "finder.open_path / reveal_path: {path}\n" +
                        "file-explorer.open_path / reveal_path: {path}\n" +
                        "safari.open_url / new_tab: {url}; .current_url / current_title: {}\n" +
                        "browser.list_tabs: {}; .open_url: {url, new_tab?}; .current_url / current_title / read_page_text: {tab_match?}; .click_text: {text, tab_match?}; .fill_field: {selector, value, tab_match?}; .execute_js: {expression, tab_match?}",
                    },
                  },
                  required: ["slug", "handler"],
                },
              },
            ],
          },
        ],
      },
    };
    this.socket!.send(JSON.stringify(setup));
  }

  private handleRawMessage(raw: string | ArrayBuffer): void {
    const text = typeof raw === "string" ? raw : new TextDecoder().decode(raw);
    let parsed: any;
    try {
      parsed = JSON.parse(text);
    } catch {
      this.options.onMessage({ kind: "error", message: "Non-JSON frame from Gemini" });
      return;
    }

    if (parsed.setupComplete) {
      this.setupComplete = true;
      this.resolveSetupComplete?.();
      this.options.onMessage({ kind: "setup_complete" });
      return;
    }

    if (parsed.serverContent) {
      const sc = parsed.serverContent;
      const modelTurn = sc.modelTurn;
      if (modelTurn?.parts) {
        for (const part of modelTurn.parts) {
          if (part.inlineData?.mimeType?.startsWith("audio/pcm")) {
            this.options.onMessage({
              kind: "audio_chunk",
              pcm24kHz: base64ToPcm16(part.inlineData.data),
            });
          }
        }
      }
      if (sc.inputTranscription?.text) {
        this.options.onMessage({
          kind: "input_transcript",
          text: sc.inputTranscription.text,
        });
      }
      if (sc.outputTranscription?.text) {
        this.options.onMessage({
          kind: "output_transcript",
          text: sc.outputTranscription.text,
        });
      }
      if (sc.turnComplete) {
        this.options.onMessage({ kind: "turn_complete" });
      }
      return;
    }

    // Gemini Live emits a `usageMetadata` block whenever the server
    // tallies token consumption for a turn. Forward to the Rust cost
    // meter so the panel footer + history file stay in sync without
    // routing through the model's tool surface.
    if (parsed.usageMetadata) {
      const usage = parsed.usageMetadata;
      const inputTokens = Number(
        usage.promptTokenCount ?? usage.inputTokenCount ?? 0,
      );
      const outputTokens = Number(
        usage.responseTokenCount ?? usage.outputTokenCount ?? usage.candidatesTokenCount ?? 0,
      );
      if (inputTokens > 0 || outputTokens > 0) {
        void invoke("record_usage", {
          inputTokens,
          outputTokens,
        }).catch((recordError) =>
          console.warn("[gemini] record_usage failed:", recordError),
        );
      }
    }

    if (parsed.toolCall?.functionCalls) {
      for (const fc of parsed.toolCall.functionCalls) {
        this.options.onMessage({
          kind: "tool_call",
          name: fc.name,
          args: fc.args,
          toolCallId: fc.id,
        });
      }
    }
  }
}
