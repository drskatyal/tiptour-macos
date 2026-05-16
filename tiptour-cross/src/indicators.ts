// Indicator-pill webview. Receives `indicator_event` from the Rust
// side, prepends a pill into the stack, plays the entry spring,
// optionally auto-dismisses after a configurable timeout, and toggles
// OS-level click-through when the mouse hovers a pill (so the rest of
// the strip never blocks clicks on the user's real apps behind it).

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { installThemeBridge } from "./theme";

void installThemeBridge();

type IndicatorKindWire =
  | "step"
  | "flow"
  | "voice"
  | "app"
  | "screenshot"
  | "error";

type DensityWire = "compact" | "normal" | "verbose";

interface IndicatorEventPayload {
  kind: IndicatorKindWire;
  title: string;
  subtitle?: string | null;
  sourceId?: string | null;
  autoDismissMs: number;
  maxVisible: number;
  density: DensityWire;
  sound: "silent" | "soft-tick";
}

// Unicode glyph per kind. Keeps things dependency-free; if we later
// want SVGs we can swap them in without touching layout.
const ICON_GLYPH_BY_KIND: Record<IndicatorKindWire, string> = {
  step: "✓",
  flow: "▶",
  voice: "🎙",
  app: "↗",
  screenshot: "📷",
  error: "!",
};

const indicatorStackElement = document.getElementById("indicator-stack") as HTMLDivElement;

// Track active pills so we can enforce max-visible by dropping oldest.
interface ActivePill {
  pillElement: HTMLDivElement;
  autoDismissTimerId: number | null;
}
const activePills: ActivePill[] = [];

// Grace timer for mouseleave on the whole stack — re-enable
// click-through ~100ms after the user stops hovering any pill, so
// quickly moving from one pill to another doesn't strobe the flag.
let stackLeaveGraceTimerId: number | null = null;

async function setOsClickThrough(isClickThrough: boolean): Promise<void> {
  try {
    await invoke("indicators_set_click_through", { isClickThrough });
  } catch (clickThroughError) {
    // Non-fatal — the worst case is a brief click-blocking, which the
    // mouseleave path will clear. Log and continue.
    console.warn("indicators: failed to toggle click-through", clickThroughError);
  }
}

function buildPillElement(payload: IndicatorEventPayload): HTMLDivElement {
  const pillElement = document.createElement("div");
  pillElement.className = "indicator-pill indicator-pill-entering";
  pillElement.dataset.kind = payload.kind;

  const iconElement = document.createElement("div");
  iconElement.className = "indicator-pill-icon";
  iconElement.textContent = ICON_GLYPH_BY_KIND[payload.kind] ?? "?";

  const bodyElement = document.createElement("div");
  bodyElement.className = "indicator-pill-body";

  const titleElement = document.createElement("div");
  titleElement.className = "indicator-pill-title";
  titleElement.textContent = payload.title;

  const subtitleText = payload.subtitle ?? "";
  if (subtitleText.length > 0) {
    const subtitleElement = document.createElement("div");
    subtitleElement.className = "indicator-pill-subtitle";
    subtitleElement.textContent = subtitleText;
    bodyElement.appendChild(titleElement);
    bodyElement.appendChild(subtitleElement);
  } else {
    bodyElement.appendChild(titleElement);
  }

  pillElement.appendChild(iconElement);
  pillElement.appendChild(bodyElement);

  // Compact density keeps the pill collapsed always; Verbose keeps it
  // expanded always; Normal toggles on hover.
  if (payload.density === "verbose") {
    pillElement.classList.add("indicator-pill-expanded");
  }

  // Hover behavior — only in normal density does hover affect anything.
  pillElement.addEventListener("mouseenter", () => {
    void setOsClickThrough(false);
    if (payload.density === "normal") {
      pillElement.classList.add("indicator-pill-expanded");
    }
    // Cancel any pending leave grace — we're still hovering something.
    if (stackLeaveGraceTimerId !== null) {
      window.clearTimeout(stackLeaveGraceTimerId);
      stackLeaveGraceTimerId = null;
    }
  });
  pillElement.addEventListener("mouseleave", () => {
    if (payload.density === "normal") {
      pillElement.classList.remove("indicator-pill-expanded");
    }
  });

  pillElement.addEventListener("click", () => {
    dismissPill(pillElement);
  });

  return pillElement;
}

function dismissPill(pillElement: HTMLDivElement): void {
  const activePillIndex = activePills.findIndex(
    (active) => active.pillElement === pillElement,
  );
  if (activePillIndex < 0) {
    return;
  }
  const [removedActivePill] = activePills.splice(activePillIndex, 1);
  if (removedActivePill.autoDismissTimerId !== null) {
    window.clearTimeout(removedActivePill.autoDismissTimerId);
  }
  pillElement.classList.remove("indicator-pill-entering");
  pillElement.classList.add("indicator-pill-leaving");
  // Wait for the exit animation before removing from the DOM so the
  // browser actually paints the fade-out.
  window.setTimeout(() => {
    pillElement.remove();
  }, 240);
}

function nudgeExistingPills(): void {
  // Re-run the nudge animation on every currently-stacked pill by
  // briefly toggling the class. Removing then re-adding inside a
  // requestAnimationFrame restarts the keyframe animation.
  for (const active of activePills) {
    active.pillElement.classList.remove("indicator-pill-nudging");
    // Force a reflow so the animation restarts on re-add.
    void active.pillElement.offsetWidth;
    active.pillElement.classList.add("indicator-pill-nudging");
  }
}

function handleIndicatorEvent(payload: IndicatorEventPayload): void {
  const pillElement = buildPillElement(payload);

  // Stack-shift nudge on existing pills before inserting the new one.
  nudgeExistingPills();

  // Prepend so newest pill sits at the top of the stack.
  indicatorStackElement.prepend(pillElement);

  const activePill: ActivePill = {
    pillElement,
    autoDismissTimerId: null,
  };
  activePills.unshift(activePill);

  // Enforce max-visible — drop the oldest pill if we're over the cap.
  while (activePills.length > payload.maxVisible) {
    const oldestActivePill = activePills.pop();
    if (oldestActivePill) {
      dismissPill(oldestActivePill.pillElement);
    }
  }

  // Auto-dismiss timer. autoDismissMs === 0 means "forever".
  if (payload.autoDismissMs > 0) {
    activePill.autoDismissTimerId = window.setTimeout(() => {
      dismissPill(pillElement);
    }, payload.autoDismissMs);
  }

  // Optional soft tick. Synthesized via WebAudio so we don't ship an
  // audio asset. Honor user preference; silent is the default.
  if (payload.sound === "soft-tick") {
    playSoftTickSound();
  }
}

let cachedAudioContext: AudioContext | null = null;
function playSoftTickSound(): void {
  try {
    if (!cachedAudioContext) {
      const AudioContextConstructor =
        window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      cachedAudioContext = new AudioContextConstructor();
    }
    const oscillatorNode = cachedAudioContext.createOscillator();
    const gainNode = cachedAudioContext.createGain();
    oscillatorNode.type = "sine";
    oscillatorNode.frequency.value = 880;
    gainNode.gain.setValueAtTime(0.0001, cachedAudioContext.currentTime);
    gainNode.gain.exponentialRampToValueAtTime(
      0.08,
      cachedAudioContext.currentTime + 0.01,
    );
    gainNode.gain.exponentialRampToValueAtTime(
      0.0001,
      cachedAudioContext.currentTime + 0.12,
    );
    oscillatorNode.connect(gainNode).connect(cachedAudioContext.destination);
    oscillatorNode.start();
    oscillatorNode.stop(cachedAudioContext.currentTime + 0.14);
  } catch (audioError) {
    console.warn("indicators: soft tick failed", audioError);
  }
}

// Re-enable click-through whenever the cursor leaves the entire stack
// container, with a 100ms grace so brushing between pills doesn't
// re-block.
indicatorStackElement.addEventListener("mouseleave", () => {
  if (stackLeaveGraceTimerId !== null) {
    window.clearTimeout(stackLeaveGraceTimerId);
  }
  stackLeaveGraceTimerId = window.setTimeout(() => {
    void setOsClickThrough(true);
    stackLeaveGraceTimerId = null;
  }, 100);
});

void listen<IndicatorEventPayload>("indicator_event", (event) => {
  // Apply the current edge preference to the stack alignment. The
  // emit payload doesn't carry position (the window itself is moved
  // server-side), but we still read it from the document body class
  // applied by the settings change handler.
  handleIndicatorEvent(event.payload);
});

// Tell Rust we're ready in case it wants to flush queued events. No
// such queue exists today, but this gives us a hook for later.
window.addEventListener("DOMContentLoaded", () => {
  void setOsClickThrough(true);
});
