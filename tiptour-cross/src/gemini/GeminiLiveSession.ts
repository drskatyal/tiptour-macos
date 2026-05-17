// High-level orchestrator that ties the Gemini Live WebSocket client to the
// Rust audio bridge and surfaces a small set of UI callbacks.
//
// Mirrors TipTour/GeminiLiveSession.swift in shape. Phase 0 wires only voice
// in / voice out; tool calls are received but rejected with a "not yet"
// response so the model knows the surface exists. Phase 1+ will fill in
// submit_workflow_plan and friends.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { GeminiLiveClient, GeminiInboundMessage } from "./GeminiLiveClient";

export type SessionStatus = "idle" | "listening" | "speaking" | "error";

export interface GeminiLiveSessionOptions {
  apiKey: string;
  /// Optional voice name pulled from settings.json by the panel.
  voiceName?: string;
  /// Optional model short id pulled from settings.json by the panel.
  modelShortId?: string;
  onStatusChange: (status: SessionStatus) => void;
  onUserTranscript: (text: string) => void;
  onModelTranscript: (text: string) => void;
  onError: (message: string) => void;
}

interface ScreenFramePayload {
  jpegBase64: string;
  width: number;
  height: number;
}

// Mirrors the `WorkflowProgress` enum on the Rust side. We keep the
// surface intentionally narrow — the panel only needs a one-line summary
// per transition, not the full structured payload.
type WorkflowProgressEvent =
  | { kind: "started"; workflowId: string; goal?: string | null; totalSteps: number }
  | { kind: "stepStarted"; stepIndex: number; label?: string | null; stepType: string }
  | {
      kind: "stepResolved";
      stepIndex: number;
      // Coordinates are absent when the runner short-circuited a click
      // step into a keyboard shortcut — `shortcutKeys` carries the
      // chord tokens in that case.
      targetX?: number;
      targetY?: number;
      label?: string | null;
      shortcutKeys?: string[];
    }
  | { kind: "stepFinished"; stepIndex: number; result: unknown }
  | { kind: "paused"; reason: string }
  | { kind: "completed" }
  | { kind: "failed"; message: string };

function formatWorkflowProgressEvent(event: WorkflowProgressEvent): string | null {
  switch (event.kind) {
    case "started":
      return `\n[plan] ${event.goal ?? "running"} (${event.totalSteps} steps)`;
    case "stepFinished": {
      const result = event.result as { kind?: string; reason?: string } | undefined;
      if (result?.kind === "executed") {
        return `\n[step ${event.stepIndex + 1}] done`;
      }
      if (result?.kind === "actionFailed") {
        return `\n[step ${event.stepIndex + 1}] failed: ${result.reason ?? "unknown"}`;
      }
      if (result?.kind === "unresolved") {
        return `\n[step ${event.stepIndex + 1}] unresolved: ${result.reason ?? "no match"}`;
      }
      return null;
    }
    case "paused":
      return `\n[plan paused] ${event.reason}`;
    case "completed":
      return `\n[plan done]`;
    case "failed":
      return `\n[plan failed] ${event.message}`;
    default:
      return null;
  }
}

// Mirrors `ReplayProgress` on the Rust side. The kind is tagged so we
// can branch on transition type when summarizing into the transcript.
type MultiflowProgressEvent = {
  flowId: string;
  replayId: string;
  stepIndex: number;
  totalSteps: number;
  kind:
    | { kind: "started" }
    | { kind: "inputReplayed" }
    | { kind: "appLaunched" }
    | { kind: "waited" }
    | { kind: "paused"; reason: string }
    | { kind: "completed" }
    | { kind: "failed"; message: string };
};

function formatMultiflowProgressEvent(event: MultiflowProgressEvent): string | null {
  switch (event.kind.kind) {
    case "started":
      return `\n[flow] replaying (${event.totalSteps} inputs)`;
    case "appLaunched":
      return `\n[flow] app switch detected, waiting for new app`;
    case "paused":
      return `\n[flow paused] ${event.kind.reason}`;
    case "completed":
      return `\n[flow done]`;
    case "failed":
      return `\n[flow failed] ${event.kind.message}`;
    default:
      return null;
  }
}

export class GeminiLiveSession {
  private client: GeminiLiveClient | null = null;
  private micUnlisten: UnlistenFn | null = null;
  private screenFrameUnlisten: UnlistenFn | null = null;
  private workflowProgressUnlisten: UnlistenFn | null = null;
  private multiflowProgressUnlisten: UnlistenFn | null = null;
  private readonly options: GeminiLiveSessionOptions;
  // Each opened session gets a fresh UUID. Every user/model turn is
  // appended to a file keyed on this id so the history list shows one
  // entry per push-to-talk conversation, not one giant rolling log.
  private readonly sessionId: string =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now().toString(16)}-${Math.random().toString(16).slice(2, 10)}`;

  constructor(options: GeminiLiveSessionOptions) {
    this.options = options;
  }

  async open(): Promise<void> {
    this.client = new GeminiLiveClient({
      apiKey: this.options.apiKey,
      voiceName: this.options.voiceName,
      modelShortId: this.options.modelShortId,
      onMessage: (m) => this.handleInbound(m),
      onClose: (reason) => {
        console.warn("[session] websocket closed:", reason);
        this.options.onStatusChange("idle");
      },
    });

    try {
      await this.client.open();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error("[session] open failed:", message);
      this.options.onError(message);
      this.options.onStatusChange("error");
      throw error;
    }

    // Seed the new session with the tail of the previous one so the
    // user can say "remember what I asked earlier" and Gemini has
    // recent context. We deliberately pull from ONE prior session
    // (not all of them) — concatenating every past session would
    // blow the context budget and confuse the model with stale topics.
    try {
      const tail = await invoke<Array<{ role: string; text: string; at: string }>>(
        "get_last_session_tail",
        { maxTurns: 6 },
      );
      if (tail && tail.length > 0) {
        const formatted = tail
          .map((turn) => `${turn.role === "user" ? "User" : "Assistant"}: ${turn.text}`)
          .join("\n");
        this.client.sendTextTurn(
          `Earlier this user said:\n${formatted}\n\nUse this for continuity only if they reference it; otherwise treat this as a fresh conversation.`,
          false,
        );
      }
    } catch (error) {
      console.warn("[session] failed to seed prior-session tail:", error);
    }

    // Counters so the panel debug strip can prove (or disprove)
    // that audio + screenshots are actually reaching Gemini. The
    // most common "Gemini just says hi back" failure mode is the
    // mic loop never emitting a single chunk because permission
    // is denied — the server-side sees an empty audio stream and
    // greets the silence.
    let micChunkCount = 0;
    let screenFrameCount = 0;
    const reportFlow = () => {
      window.dispatchEvent(
        new CustomEvent("session-flow-stats", {
          detail: { micChunkCount, screenFrameCount },
        }),
      );
    };
    setInterval(reportFlow, 2000);

    this.micUnlisten = await listen<number[]>("mic_chunk", (event) => {
      const pcm = Uint8Array.from(event.payload);
      this.client?.sendMicChunk(pcm);
      micChunkCount += 1;
    });

    // Workflow progress stream: the Rust executor emits one event per
    // step transition. We append a short summary to the model transcript
    // so the user sees the plan unfolding alongside Gemini's voice reply.
    this.workflowProgressUnlisten = await listen<WorkflowProgressEvent>(
      "workflow_progress",
      (event) => {
        const payload = event.payload;
        // Drive the overlay cursor in lockstep with the runner. The
        // animation duration in CSS matches the click-settle delay so the
        // companion cursor lands at the same moment the real cursor does.
        if (payload.kind === "stepResolved") {
          // Only fly the overlay cursor when the runner emitted a
          // coordinate target. Shortcut-grounded steps have no on-screen
          // destination so the overlay stays put.
          if (typeof payload.targetX === "number" && typeof payload.targetY === "number") {
            void invoke("overlay_fly_cursor_to", {
              x: payload.targetX,
              y: payload.targetY,
              label: payload.label ?? null,
            });
          }
        }
        if (payload.kind === "completed" || payload.kind === "failed" || payload.kind === "paused") {
          void invoke("overlay_hide_response");
        }
        const summaryLine = formatWorkflowProgressEvent(payload);
        if (summaryLine) {
          this.options.onModelTranscript(summaryLine);
        }
      },
    );

    // Multiflow replay progress: separate event stream from
    // workflow_progress so the panel can render the two states
    // differently if it wants to.
    this.multiflowProgressUnlisten = await listen<MultiflowProgressEvent>(
      "multiflow_progress",
      (event) => {
        const summaryLine = formatMultiflowProgressEvent(event.payload);
        if (summaryLine) {
          this.options.onModelTranscript(summaryLine);
        }
      },
    );

    // Inject the top-importance memories as priming context so the
    // agent starts each session knowing the user's recurring facts.
    // Best-effort: a memory-store failure shouldn't block the voice
    // session from opening.
    try {
      const topMemories = await invoke<Array<{ key: string; value: string }>>(
        "list_top_importance_memories",
        { limit: 10 },
      );
      if (topMemories.length > 0) {
        const memoryLines = topMemories
          .map((m) => `- ${m.key}: ${m.value}`)
          .join("\n");
        this.client.sendTextTurn(
          `Context from previous sessions (do not read aloud, just remember):\n${memoryLines}`,
          false,
        );
      }
    } catch (memoryError) {
      console.warn("[session] memory priming skipped:", memoryError);
    }

    try {
      await invoke("start_mic_capture");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error("[session] mic capture failed:", message);
      this.options.onError("Mic capture failed: " + message);
      this.options.onStatusChange("error");
      throw error;
    }

    // Wire up the screen capture stream. Screen recording is best-effort:
    // if the user hasn't granted permission yet the Rust side will emit
    // an error per tick that we just log — voice still works without
    // vision, so we don't tear down the whole session for a vision-only
    // failure.
    this.screenFrameUnlisten = await listen<ScreenFramePayload>(
      "screen_frame",
      (event) => {
        this.client?.sendScreenshot(event.payload.jpegBase64);
        screenFrameCount += 1;
      },
    );
    try {
      await invoke("start_screen_stream");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.warn("[session] screen stream failed (continuing without vision):", message);
      this.options.onError("Screen capture unavailable — grant screen recording permission. " + message);
    }

    this.options.onStatusChange("listening");
  }

  async close(): Promise<void> {
    try {
      await invoke("stop_mic_capture");
    } catch {
      // best-effort
    }
    try {
      await invoke("stop_screen_stream");
    } catch {
      // best-effort — the streamer may never have started
    }
    this.micUnlisten?.();
    this.micUnlisten = null;
    this.screenFrameUnlisten?.();
    this.screenFrameUnlisten = null;
    this.workflowProgressUnlisten?.();
    this.workflowProgressUnlisten = null;
    this.multiflowProgressUnlisten?.();
    this.multiflowProgressUnlisten = null;
    this.client?.close();
    this.client = null;
    void invoke("overlay_hide");
    void invoke("overlay_set_speaking", { speaking: false });
    this.options.onStatusChange("idle");
  }

  private handleInbound(message: GeminiInboundMessage): void {
    switch (message.kind) {
      case "setup_complete":
        return;
      case "audio_chunk":
        this.options.onStatusChange("speaking");
        void invoke("overlay_set_speaking", { speaking: true });
        void invoke("play_audio_chunk", { pcm: Array.from(message.pcm24kHz) });
        return;
      case "input_transcript":
        this.options.onUserTranscript(message.text);
        void invoke("append_session_turn", {
          sessionId: this.sessionId,
          role: "user",
          text: message.text,
        }).catch((error) => {
          console.warn("[session] append user turn failed:", error);
        });
        return;
      case "output_transcript":
        this.options.onModelTranscript(message.text);
        void invoke("append_session_turn", {
          sessionId: this.sessionId,
          role: "model",
          text: message.text,
        }).catch((error) => {
          console.warn("[session] append model turn failed:", error);
        });
        // Stream model text into the overlay bubble so the user can read
        // the reply right next to the companion cursor.
        void invoke("overlay_show_response", {
          text: message.text,
          appendMode: true,
        });
        return;
      case "turn_complete":
        this.options.onStatusChange("listening");
        void invoke("overlay_set_speaking", { speaking: false });
        // Hide the overlay reply bubble at end-of-turn so the next turn's
        // streamed `output_transcript` doesn't accumulate on top of the
        // previous reply. Without this, the bubble keeps growing across
        // turns (overlay.ts uses appendMode: true and only resets on
        // hideResponseBubble).
        void invoke("overlay_hide_response");
        return;
      case "tool_call":
        void this.handleToolCall(message.name, message.args, message.toolCallId);
        return;
      case "error":
        console.error("Gemini Live error:", message.message);
        this.options.onError(message.message);
        this.options.onStatusChange("error");
        return;
    }
  }

  /// Dispatch a Gemini tool call to the Rust-side central dispatcher.
  /// The Rust `dispatch_tool_call` command knows every tool; we just
  /// forward the name + args and pipe the structured response straight
  /// back to Gemini. This keeps the tool surface in one place (Rust)
  /// instead of duplicating per-tool branches across TS and Rust.
  private async handleToolCall(
    name: string,
    args: unknown,
    toolCallId: string,
  ): Promise<void> {
    try {
      const response = await invoke<Record<string, unknown>>("dispatch_tool_call", {
        name,
        args: (args ?? {}) as Record<string, unknown>,
      });
      this.client?.sendToolResponse(toolCallId, response);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error("[session] dispatch_tool_call failed:", message);
      this.client?.sendToolResponse(toolCallId, {
        status: "error",
        message,
      });
    }
  }
}
