// Low-level WebSocket client for Google's Gemini Live API.
//
// Mirrors the shape of TipTour/GeminiLiveClient.swift on the macOS side so
// session logic ports straight across. This file owns only the wire format:
// frame in, frame out, no audio device handling, no UI state, no tool
// dispatch — those live in GeminiLiveSession.ts and the Rust audio bridge.

import { base64ToPcm16, pcm16ToBase64 } from "./audio";

const GEMINI_LIVE_WS_URL =
  "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

// Matches the model used by the Swift TipTour app (May 2026). The old
// gemini-2.0-flash-exp endpoint is retired and silently rejects sessions.
const MODEL = "models/gemini-3.1-flash-live-preview";

// "Kore" is the voice 3.1 Flash Live accepts; older voices like "Aoede"
// cause the server to close the socket before setupComplete.
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

    this.sendSetup();

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

  private sendSetup(): void {
    const setup = {
      setup: {
        model: MODEL,
        generationConfig: {
          responseModalities: ["AUDIO"],
          mediaResolution: "MEDIA_RESOLUTION_MEDIUM",
          speechConfig: {
            voiceConfig: { prebuiltVoiceConfig: { voiceName: DEFAULT_VOICE } },
          },
        },
        systemInstruction: {
          parts: [
            {
              text: "You are TipTour, a helpful voice companion. Reply concisely.",
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
