// Floating dock — small grey dash that expands to a toolbar on hover.
// All six buttons dispatch into existing Rust paths so the dock is
// just a visual front for things the panel + hotkey already do.

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

const rootElement = document.getElementById("dock-root")!;

// First-launch discoverability: the collapsed dash is intentionally
// subtle (6px translucent capsule), which is great once the user
// knows it's there but invisible on a busy desktop the first time.
// Auto-expand the toolbar for ~3s on launch so the user sees the
// full surface, then fall back to the dash. The data-state attribute
// already drives the CSS animation, so flipping it is enough.
rootElement.dataset.state = "toolbar";
window.setTimeout(() => {
  if (rootElement.dataset.state === "toolbar" && !rootElement.matches(":hover")) {
    rootElement.dataset.state = "dash";
  }
}, 3000);

// Tray "Show dock" / external request to peek the toolbar. Emits the
// same expansion + auto-collapse so the user can flash the dock from
// the menu when they forget where it lives.
void listen("dock_flash", () => {
  rootElement.dataset.state = "toolbar";
  window.setTimeout(() => {
    if (rootElement.dataset.state === "toolbar" && !rootElement.matches(":hover")) {
      rootElement.dataset.state = "dash";
    }
  }, 3500);
});

// Action pill tooltip — floats above the toolbar centered on the
// hovered button. Reads data-action + data-shortcut off each button
// so the markup stays the source of truth for label + chord.
const actionPillElement = document.getElementById(
  "dock-action-pill",
) as HTMLDivElement | null;
const actionLabelElement = document.getElementById(
  "dock-action-label",
) as HTMLSpanElement | null;
const actionShortcutElement = document.getElementById(
  "dock-action-shortcut",
) as HTMLSpanElement | null;

function showActionPillForButton(buttonElement: HTMLElement): void {
  if (!actionPillElement || !actionLabelElement || !actionShortcutElement) return;
  const actionLabel = buttonElement.dataset.action ?? "";
  const actionShortcut = buttonElement.dataset.shortcut ?? "";
  if (!actionLabel) return;
  actionLabelElement.textContent = actionLabel;
  actionShortcutElement.textContent = actionShortcut;
  // Center horizontally on the hovered button.
  const buttonRect = buttonElement.getBoundingClientRect();
  const rootRect = rootElement.getBoundingClientRect();
  const buttonCenterX = buttonRect.left + buttonRect.width / 2 - rootRect.left;
  actionPillElement.hidden = false;
  actionPillElement.style.left = `${buttonCenterX}px`;
  // Use rAF so the browser commits hidden=false before we flip the
  // visible state — otherwise the CSS transition skips the in-frame.
  requestAnimationFrame(() => {
    actionPillElement.dataset.visible = "true";
  });
}

function hideActionPill(): void {
  if (!actionPillElement) return;
  actionPillElement.dataset.visible = "false";
  // Match the CSS opacity transition before fully hiding so quick
  // mouse moves between adjacent buttons don't flicker the pill.
  window.setTimeout(() => {
    if (actionPillElement.dataset.visible !== "true") {
      actionPillElement.hidden = true;
    }
  }, 180);
}

document.querySelectorAll<HTMLButtonElement>(".dock-button").forEach((buttonElement) => {
  buttonElement.addEventListener("mouseenter", () => showActionPillForButton(buttonElement));
  buttonElement.addEventListener("focus", () => showActionPillForButton(buttonElement));
  buttonElement.addEventListener("mouseleave", hideActionPill);
  buttonElement.addEventListener("blur", hideActionPill);
});

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

// Focused-app pip: refreshed on every hover-expand so the user
// sees which app TipTour will implicitly target when they say
// "this" / "here". Cheap enough to read on demand (~30ms via
// osascript on Mac, sub-ms via GetForegroundWindow on Windows).
const focusedPipElement = document.getElementById("dock-focused-app");
// 2-letter badge keeps the pip the same shape + size as the
// circular icon chips. Capitalized initials work well for app
// names ("Chrome" → "CH", "VS Code" → "VS", "Slack" → "SL").
function makeTwoLetterBadge(appName: string): string {
  const tokens = appName.split(/\s+/).filter((t) => t.length > 0);
  if (tokens.length >= 2) {
    return (tokens[0][0] + tokens[1][0]).toUpperCase();
  }
  return appName.slice(0, 2).toUpperCase();
}
async function refreshFocusedAppPip(): Promise<void> {
  if (!focusedPipElement) return;
  try {
    const appName = await invoke<string | null>("get_frontmost_app_name");
    if (appName && appName.length > 0 && appName !== "TipTour") {
      focusedPipElement.textContent = makeTwoLetterBadge(appName);
      focusedPipElement.title = `Targeting ${appName}`;
      focusedPipElement.hidden = false;
    } else {
      // Hide rather than show "TipTour" — when the dock itself is
      // frontmost the pip is meaningless.
      focusedPipElement.hidden = true;
    }
  } catch {
    focusedPipElement.hidden = true;
  }
}
rootElement.addEventListener("mouseenter", () => {
  void refreshFocusedAppPip();
});

// Transient confirmation chip: shows after each adapter dispatch,
// fades after ~1.6s. Lives in the toolbar's right edge so it
// doesn't push other controls.
const dispatchChipElement = document.getElementById("dock-dispatch-chip");
let dispatchChipTimeoutId: number | null = null;
await listen<{ slug: string; handler: string; ok: boolean; message: string }>(
  "adapter_dispatched",
  (event) => {
    if (!dispatchChipElement) return;
    const payload = event.payload;
    dispatchChipElement.dataset.ok = payload.ok ? "true" : "false";
    dispatchChipElement.textContent = payload.ok
      ? `✓ ${payload.slug} · ${payload.handler}`
      : `⚠ ${payload.message.slice(0, 60)}`;
    dispatchChipElement.hidden = false;
    if (dispatchChipTimeoutId) clearTimeout(dispatchChipTimeoutId);
    dispatchChipTimeoutId = window.setTimeout(() => {
      if (dispatchChipElement) dispatchChipElement.hidden = true;
    }, 1800);
  },
);

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
