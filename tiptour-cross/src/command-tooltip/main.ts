// Command tooltip — small floating overlay that reports the result
// of a push-to-talk quick-command. Three visible states:
//
//   listening   — mic is open, dot pulses green
//   thinking    — audio sent to flash-lite, dot pulses blue
//   result      — model returned text or adapter ran; show + auto-fade
//   error       — show until dismissed or 6s
//
// The Rust side emits `quick_voice_state` ("listening"/"thinking"/
// "idle"/"error") and `quick_voice_result` (the full QuickVoiceResult
// payload). We listen + render. The window auto-hides itself when
// state lands back at "idle".

import { listen } from "@tauri-apps/api/event";

interface QuickVoiceResult {
  displayText: string;
  actionTaken: boolean;
  model: string;
  elapsedMs: number;
  inputTokens?: number;
  outputTokens?: number;
}

const rootElement = document.getElementById("command-tooltip-root")!;
const stateElement = document.getElementById("command-tooltip-state")!;
const bodyElement = document.getElementById("command-tooltip-body-text")!;
const footerElement = document.getElementById("command-tooltip-footer")!;

let autoHideTimer: number | null = null;

function clearAutoHide(): void {
  if (autoHideTimer) {
    clearTimeout(autoHideTimer);
    autoHideTimer = null;
  }
}

function scheduleAutoHide(milliseconds: number): void {
  clearAutoHide();
  autoHideTimer = window.setTimeout(async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch (hideError) {
      console.warn("[tooltip] hide failed:", hideError);
    }
  }, milliseconds);
}

async function showWindowIfHidden(): Promise<void> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const window = getCurrentWindow();
    if (!(await window.isVisible())) await window.show();
  } catch (showError) {
    console.warn("[tooltip] show failed:", showError);
  }
}

function setState(newState: string, label: string): void {
  rootElement.dataset.state = newState;
  stateElement.textContent = label;
}

await listen<string>("quick_voice_state", async (event) => {
  const next = event.payload;
  if (next === "listening") {
    clearAutoHide();
    setState("listening", "Listening…");
    bodyElement.textContent = "";
    footerElement.textContent = "";
    await showWindowIfHidden();
  } else if (next === "thinking") {
    clearAutoHide();
    setState("thinking", "Thinking…");
  } else if (next === "error") {
    setState("error", "Couldn't catch that");
    scheduleAutoHide(6000);
  } else if (next === "idle") {
    // Only auto-hide if no result has populated the body yet —
    // otherwise the result render below will manage the timer.
    if (!bodyElement.textContent) {
      scheduleAutoHide(800);
    }
  }
});

await listen<QuickVoiceResult>("quick_voice_result", (event) => {
  const payload = event.payload;
  if (!payload) return;
  setState(payload.actionTaken ? "ok" : "reply", payload.actionTaken ? "Done" : "Reply");
  bodyElement.textContent = payload.displayText;
  const tokens =
    payload.inputTokens !== undefined && payload.outputTokens !== undefined
      ? ` · ${payload.inputTokens}+${payload.outputTokens}t`
      : "";
  footerElement.textContent = `${payload.model} · ${payload.elapsedMs}ms${tokens}`;
  // Result tooltips stay visible long enough to read (1.5s per ~30
  // chars of text, min 2s, max 8s).
  const stayMs = Math.min(8000, Math.max(2000, payload.displayText.length * 60));
  scheduleAutoHide(stayMs);
});
