// Floating dock — small grey dash that expands to a toolbar on hover.
// All six buttons dispatch into existing Rust paths so the dock is
// just a visual front for things the panel + hotkey already do.

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

const rootElement = document.getElementById("dock-root")!;

function bind(id: string, fn: () => void | Promise<void>): void {
  const button = document.getElementById(id);
  if (!button) return;
  button.addEventListener("click", (event) => {
    event.preventDefault();
    void fn();
  });
}

// Buttons dispatch directly into the same paths the panel/hotkey use,
// so the dock is purely a visual front. No new Rust commands needed.

bind("dock-quick", async () => {
  // Same as the global push-to-talk hotkey: toggles a quick voice
  // capture in whichever mode (quick/live) is configured. The panel
  // listens for this event regardless of whether it's visible, so the
  // dock works even when the panel is hidden.
  await emit("push_to_talk_toggled");
});

bind("dock-brain-dump", async () => {
  // For now reuses the same quick-voice path — the user speaks "save
  // this thought…" and Gemini routes via control_app(brain-dump,
  // capture_text). Could later become a dedicated dictation flow.
  await emit("push_to_talk_toggled");
});

bind("dock-transcribe", async () => {
  // Toggle Soniox real-time transcription into the focused field.
  try {
    await invoke("toggle_soniox_transcription");
  } catch (error) {
    console.warn("[dock] toggle_soniox_transcription failed:", error);
  }
});

bind("dock-screenshot", async () => {
  // Direct brain-dump screenshot capture — no Gemini round-trip.
  try {
    await invoke("dispatch_adapter_command", {
      slug: "brain-dump",
      handler: "capture_screenshot",
      args: { caption: "dock-snap" },
    });
  } catch (error) {
    console.warn("[dock] screenshot failed:", error);
  }
});

bind("dock-open-panel", async () => {
  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const panel = await WebviewWindow.getByLabel("panel");
    if (panel) {
      await panel.show();
      await panel.setFocus();
    }
  } catch (error) {
    console.warn("[dock] open panel failed:", error);
  }
});

bind("dock-open-settings", async () => {
  try {
    await invoke("open_settings_window");
  } catch (error) {
    console.warn("[dock] open_settings_window failed:", error);
  }
});

// Reflect the session/transcribe state via data-active so the dash
// glows blue when something's running. Rust pushes events for both.
await listen<string>("quick_voice_state", (event) => {
  const state = event.payload;
  if (state === "listening" || state === "thinking") {
    rootElement.dataset.active = "listening";
  } else {
    rootElement.dataset.active = "";
  }
});

await listen<string>("soniox_state", (event) => {
  if (event.payload === "transcribing") {
    rootElement.dataset.active = "transcribing";
  } else {
    rootElement.dataset.active = "";
  }
});
