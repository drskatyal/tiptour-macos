// Floating dock — small grey dash that expands to a toolbar on hover.
// All six buttons dispatch into existing Rust paths so the dock is
// just a visual front for things the panel + hotkey already do.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const rootElement = document.getElementById("dock-root")!;

function bind(id: string, fn: () => void | Promise<void>): void {
  const button = document.getElementById(id);
  if (!button) return;
  button.addEventListener("click", (event) => {
    event.preventDefault();
    void fn();
  });
}

bind("dock-quick", async () => {
  // Equivalent to pressing the global push-to-talk hotkey — toggles
  // a quick capture or ends one if already armed.
  await invoke("emit_panel_push_to_talk").catch(async () => {
    // Fallback: emit the event ourselves via the panel-side handler.
    const { emit } = await import("@tauri-apps/api/event");
    await emit("push_to_talk_toggled");
  });
});

bind("dock-transcribe", async () => {
  // Toggle Soniox transcription. Reads state from the Rust side so
  // pressing the button twice starts/stops cleanly.
  await invoke("toggle_soniox_transcription").catch((error) => {
    console.warn("[dock] toggle_soniox_transcription failed:", error);
  });
});

bind("dock-screenshot", async () => {
  await invoke("dispatch_adapter_command", {
    slug: "brain-dump",
    handler: "capture_screenshot",
    args: { caption: "dock-snap" },
  }).catch((error) => console.warn("[dock] screenshot failed:", error));
});

bind("dock-brain-dump", async () => {
  // Quick "capture a thought" via a one-shot voice command — same
  // path as the hotkey but armed by the toolbar instead of Alt+X.
  await invoke("emit_panel_push_to_talk").catch(async () => {
    const { emit } = await import("@tauri-apps/api/event");
    await emit("push_to_talk_toggled");
  });
});

bind("dock-open-panel", async () => {
  await invoke("show_panel_window").catch(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    // Fallback: show via the WebviewWindow API directly.
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const panel = await WebviewWindow.getByLabel("panel");
    if (panel) {
      await panel.show();
      await panel.setFocus();
    }
    void getCurrentWindow;
  });
});

bind("dock-open-settings", async () => {
  await invoke("open_settings_window").catch((error) => {
    console.warn("[dock] open_settings_window failed:", error);
  });
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
