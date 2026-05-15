// Overlay window controller. Receives Tauri events from the Rust shell and
// drives a transparent always-on-top window's cursor + response bubble.
//
// Coordinates from the Rust side are GLOBAL screen pixels (the union of all
// monitors). The overlay window is positioned at (0,0) of the virtual
// screen, so child elements are placed at the raw global coordinates with
// no translation. Multi-monitor support comes from the window itself
// spanning the full virtual desktop.

import { listen } from "@tauri-apps/api/event";

const cursor = document.getElementById("cursor-svg") as SVGElement | null;
const responseBubble = document.getElementById("response-bubble") as HTMLElement | null;
const responseText = document.getElementById("response-text") as HTMLElement | null;
const waveform = document.getElementById("waveform") as HTMLElement | null;

let lastCursorX = 0;
let lastCursorY = 0;
let bubbleVisible = false;
let accumulatedText = "";

interface CursorFlyToEvent {
  x: number;
  y: number;
  label?: string;
}

interface ResponseBubbleEvent {
  text: string;
  appendMode?: boolean;
  attachToCursor?: boolean;
  anchorX?: number;
  anchorY?: number;
}

function setCursorPosition(x: number, y: number) {
  if (!cursor) return;
  cursor.classList.add("visible");
  // SVG is 44x44 — offset by half so the tip sits at (x,y).
  cursor.style.transform = `translate(${x - 6}px, ${y - 4}px)`;
  lastCursorX = x;
  lastCursorY = y;
  positionBubbleNearCursor();
}

function positionBubbleNearCursor() {
  if (!responseBubble) return;
  if (!bubbleVisible) return;
  // Right-and-down offset like the Swift overlay — sits beside the cursor
  // so it doesn't occlude the click target.
  const offsetX = 28;
  const offsetY = 8;
  responseBubble.style.transform = `translate(${lastCursorX + offsetX}px, ${lastCursorY + offsetY}px)`;
}

function showResponseBubble() {
  if (!responseBubble) return;
  responseBubble.hidden = false;
  bubbleVisible = true;
  positionBubbleNearCursor();
}

function hideResponseBubble() {
  if (!responseBubble) return;
  responseBubble.hidden = true;
  bubbleVisible = false;
  accumulatedText = "";
  if (responseText) responseText.textContent = "";
}

function setWaveformVisible(visible: boolean) {
  if (!waveform) return;
  waveform.style.display = visible ? "flex" : "none";
}

await listen<CursorFlyToEvent>("overlay/cursor_fly_to", (event) => {
  setCursorPosition(event.payload.x, event.payload.y);
});

await listen("overlay/cursor_hide", () => {
  if (cursor) cursor.classList.remove("visible");
});

await listen<ResponseBubbleEvent>("overlay/response_show", (event) => {
  if (!responseText) return;
  if (event.payload.appendMode) {
    accumulatedText += event.payload.text;
  } else {
    accumulatedText = event.payload.text;
  }
  responseText.textContent = accumulatedText;
  if (event.payload.anchorX !== undefined && event.payload.anchorY !== undefined) {
    lastCursorX = event.payload.anchorX;
    lastCursorY = event.payload.anchorY;
  }
  showResponseBubble();
});

await listen("overlay/response_hide", () => {
  hideResponseBubble();
});

await listen<{ speaking: boolean }>("overlay/waveform", (event) => {
  setWaveformVisible(event.payload.speaking);
});

console.info("[overlay] ready");
