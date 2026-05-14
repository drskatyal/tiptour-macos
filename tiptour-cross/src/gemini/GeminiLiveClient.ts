// Low-level WebSocket client for Google's Gemini Live API.
//
// Mirrors the shape of TipTour/GeminiLiveClient.swift on the macOS side so
// session logic ports straight across. This file owns only the wire format:
// frame in, frame out, no audio device handling, no UI state, no tool
// dispatch — those live in GeminiLiveSession.ts and the Rust audio bridge.

import { base64ToPcm16, pcm16ToBase64 } from "./audio";

const GEMINI_LIVE_WS_URL =
  "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

const MODEL = "models/gemini-2.0-flash-exp";

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
  onClose: () => void;
}

export class GeminiLiveClient {
  private socket: WebSocket | null = null;
  private readonly options: GeminiLiveClientOptions;

  constructor(options: GeminiLiveClientOptions) {
    this.options = options;
  }

  async open(): Promise<void> {
    const url = `${GEMINI_LIVE_WS_URL}?key=${encodeURIComponent(this.options.apiKey)}`;
    this.socket = new WebSocket(url);
    this.socket.binaryType = "arraybuffer";

    await new Promise<void>((resolve, reject) => {
      if (!this.socket) return reject(new Error("Socket gone"));
      this.socket.onopen = () => resolve();
      this.socket.onerror = () => reject(new Error("WebSocket failed to open"));
    });

    this.socket!.onmessage = (event) => this.handleRawMessage(event.data);
    this.socket!.onclose = () => this.options.onClose();

    this.sendSetup();
  }

  close(): void {
    this.socket?.close();
    this.socket = null;
  }

  sendMicChunk(pcm16: Uint8Array): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
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
    // Phase 0 setup: voice in/out only. Tool declarations come in Phase 1+.
    const setup = {
      setup: {
        model: MODEL,
        generationConfig: {
          responseModalities: ["AUDIO"],
          speechConfig: {
            voiceConfig: { prebuiltVoiceConfig: { voiceName: "Aoede" } },
          },
        },
        inputAudioTranscription: {},
        outputAudioTranscription: {},
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
