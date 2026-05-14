import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { GeminiLiveSession, SessionStatus } from "./gemini/GeminiLiveSession";

const statusDot = document.querySelector<HTMLElement>(".dot")!;
const statusLabel = document.getElementById("status-label")!;
const apiKeyInput = document.getElementById("api-key-input") as HTMLInputElement;
const apiKeySaveButton = document.getElementById("api-key-save")!;
const transcriptEl = document.getElementById("transcript")!;
const hotkeyDisplay = document.getElementById("hotkey-display")!;

const isMac = navigator.platform.toLowerCase().includes("mac");
hotkeyDisplay.textContent = isMac ? "Ctrl + Option" : "Ctrl + Alt";

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
  } catch {
    // First run; no key stored yet.
  }
}

apiKeySaveButton.addEventListener("click", async () => {
  const key = apiKeyInput.value.trim();
  if (!key) return;
  await invoke("set_api_key", { key });
  apiKeySaveButton.textContent = "Saved";
  setTimeout(() => (apiKeySaveButton.textContent = "Save"), 1200);
});

async function togglePushToTalk() {
  const key = apiKeyInput.value.trim();
  if (!key) {
    setStatus("error");
    appendTranscript("model", "Set a Gemini API key first.");
    return;
  }

  if (session) {
    await session.close();
    session = null;
    setStatus("idle");
    return;
  }

  session = new GeminiLiveSession({
    apiKey: key,
    onStatusChange: setStatus,
    onUserTranscript: (text) => appendTranscript("user", text),
    onModelTranscript: (text) => appendTranscript("model", text),
  });
  await session.open();
}

await listen("push_to_talk_toggled", () => {
  void togglePushToTalk();
});

await loadStoredApiKey();
setStatus("idle");
