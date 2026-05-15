import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { GeminiLiveSession, SessionStatus } from "./gemini/GeminiLiveSession";

const statusDot = document.querySelector<HTMLElement>(".dot")!;
const statusLabel = document.getElementById("status-label")!;
const apiKeyInput = document.getElementById("api-key-input") as HTMLInputElement;
const apiKeySaveButton = document.getElementById("api-key-save") as HTMLButtonElement;
const startButton = document.getElementById("start-listening") as HTMLButtonElement;
const stopButton = document.getElementById("stop-listening") as HTMLButtonElement;
const transcriptEl = document.getElementById("transcript")!;
const hotkeyDisplay = document.getElementById("hotkey-display")!;
const errorBanner = document.getElementById("error-banner")!;

const isMac = navigator.platform.toLowerCase().includes("mac");
hotkeyDisplay.textContent = isMac ? "Option + X" : "Alt + X";

const permissionsSection = document.getElementById("permissions-section")!;
const axPermissionRow = document.getElementById("ax-permission-row")!;
const screenPermissionRow = document.getElementById("screen-permission-row")!;
const grantAccessibilityButton = document.getElementById("grant-accessibility")!;
const grantScreenRecordingButton = document.getElementById("grant-screen-recording")!;

async function refreshPermissions() {
  if (!isMac) return;
  try {
    const [hasAccessibility, hasScreenRecording] = await Promise.all([
      invoke<boolean>("check_accessibility_permission"),
      invoke<boolean>("check_screen_recording_permission"),
    ]);
    axPermissionRow.hidden = hasAccessibility;
    screenPermissionRow.hidden = hasScreenRecording;
    permissionsSection.hidden = hasAccessibility && hasScreenRecording;
  } catch (error) {
    console.warn("[panel] permission check failed:", error);
  }
}

grantAccessibilityButton.addEventListener("click", async () => {
  try {
    await invoke("request_accessibility_permission");
  } finally {
    setTimeout(() => void refreshPermissions(), 500);
  }
});

grantScreenRecordingButton.addEventListener("click", async () => {
  try {
    await invoke("request_screen_recording_permission");
  } finally {
    setTimeout(() => void refreshPermissions(), 500);
  }
});

let session: GeminiLiveSession | null = null;

function setStatus(status: SessionStatus) {
  statusDot.dataset.status = status;
  statusLabel.textContent =
    status === "idle"
      ? "Idle"
      : status === "listening"
        ? "Listening"
        : status === "speaking"
          ? "Speaking"
          : "Error";

  const isActive = status === "listening" || status === "speaking";
  startButton.hidden = isActive;
  stopButton.hidden = !isActive;
}

function showError(message: string) {
  errorBanner.textContent = message;
  errorBanner.hidden = false;
}

function clearError() {
  errorBanner.hidden = true;
  errorBanner.textContent = "";
}

function appendTranscript(role: "user" | "model", text: string) {
  const line = document.createElement("div");
  line.textContent = (role === "user" ? "> " : "  ") + text;
  transcriptEl.appendChild(line);
  transcriptEl.scrollTop = transcriptEl.scrollHeight;
}

async function loadStoredApiKey() {
  try {
    const key = await invoke<string | null>("get_api_key");
    if (key) apiKeyInput.value = key;
  } catch (error) {
    console.warn("[panel] no stored key:", error);
  }
}

apiKeySaveButton.addEventListener("click", async () => {
  const key = apiKeyInput.value.trim();
  if (!key) {
    showError("Paste a Gemini API key first.");
    return;
  }
  try {
    await invoke("set_api_key", { key });
    clearError();
    apiKeySaveButton.textContent = "Saved";
    setTimeout(() => (apiKeySaveButton.textContent = "Save"), 1200);
  } catch (error) {
    showError("Failed to save key: " + (error instanceof Error ? error.message : String(error)));
  }
});

async function startSession() {
  const key = apiKeyInput.value.trim();
  if (!key) {
    showError("Paste a Gemini API key first.");
    return;
  }
  clearError();
  console.info("[panel] starting session");

  session = new GeminiLiveSession({
    apiKey: key,
    onStatusChange: setStatus,
    onUserTranscript: (text) => appendTranscript("user", text),
    onModelTranscript: (text) => appendTranscript("model", text),
    onError: (message) => showError(message),
  });

  try {
    await session.open();
  } catch (error) {
    console.error("[panel] session.open threw:", error);
    session = null;
  }
}

async function stopSession() {
  if (!session) return;
  console.info("[panel] stopping session");
  await session.close();
  session = null;
}

async function togglePushToTalk() {
  if (session) {
    await stopSession();
  } else {
    await startSession();
  }
}

startButton.addEventListener("click", () => void startSession());
stopButton.addEventListener("click", () => void stopSession());

await listen("push_to_talk_toggled", () => {
  console.info("[panel] hotkey fired");
  void togglePushToTalk();
});

await loadStoredApiKey();
await refreshPermissions();
setStatus("idle");
console.info("[panel] ready");
