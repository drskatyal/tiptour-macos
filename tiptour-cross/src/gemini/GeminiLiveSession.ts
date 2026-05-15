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

export class GeminiLiveSession {
  private client: GeminiLiveClient | null = null;
  private micUnlisten: UnlistenFn | null = null;
  private screenFrameUnlisten: UnlistenFn | null = null;
  private readonly options: GeminiLiveSessionOptions;

  constructor(options: GeminiLiveSessionOptions) {
    this.options = options;
  }

  async open(): Promise<void> {
    this.client = new GeminiLiveClient({
      apiKey: this.options.apiKey,
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

    this.micUnlisten = await listen<number[]>("mic_chunk", (event) => {
      const pcm = Uint8Array.from(event.payload);
      this.client?.sendMicChunk(pcm);
    });

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
    this.client?.close();
    this.client = null;
    this.options.onStatusChange("idle");
  }

  private handleInbound(message: GeminiInboundMessage): void {
    switch (message.kind) {
      case "setup_complete":
        return;
      case "audio_chunk":
        this.options.onStatusChange("speaking");
        void invoke("play_audio_chunk", { pcm: Array.from(message.pcm24kHz) });
        return;
      case "input_transcript":
        this.options.onUserTranscript(message.text);
        return;
      case "output_transcript":
        this.options.onModelTranscript(message.text);
        return;
      case "turn_complete":
        this.options.onStatusChange("listening");
        return;
      case "tool_call":
        // Phase 0 stub: acknowledge but refuse. Phase 1 wires the real
        // submit_workflow_plan handler through the same channel.
        this.client?.sendToolResponse(message.toolCallId, {
          status: "not_implemented",
          message: "Tool calling is not enabled in Phase 0.",
        });
        return;
      case "error":
        console.error("Gemini Live error:", message.message);
        this.options.onError(message.message);
        this.options.onStatusChange("error");
        return;
    }
  }
}
