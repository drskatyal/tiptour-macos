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
    // macOS caches Screen Recording entitlement per-pid in TCC. Even
    // after the user flips the toggle in System Settings, the
    // currently running TipTour process won't pick it up — capture
    // will keep failing until the app is relaunched. Surface that
    // explicitly so the user isn't left wondering why Gemini still
    // can't see the screen after a successful permission flow.
    showError(
      "Screen recording permission requested. Quit and reopen TipTour for the change to take effect.",
    );
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

// Serializes overlapping push-to-talk toggles. Without this guard rapid
// Alt+X presses could fire `stopSession` while `startSession`'s
// `await session.open()` is still installing mic + screen listeners,
// leaving stranded listeners attached after close (they only get
// unregistered inside `session.close()` AFTER `open()` completes).
let pushToTalkInFlightPromise: Promise<void> | null = null;

async function togglePushToTalk() {
  if (pushToTalkInFlightPromise) {
    // A previous toggle is mid-flight (open or close). Wait for it to
    // settle so the resulting `session` state reflects truth before we
    // decide what to do next.
    await pushToTalkInFlightPromise;
  }
  const nextTransition = session ? stopSession() : startSession();
  pushToTalkInFlightPromise = nextTransition.finally(() => {
    pushToTalkInFlightPromise = null;
  });
  await pushToTalkInFlightPromise;
}

startButton.addEventListener("click", () => void startSession());
stopButton.addEventListener("click", () => void stopSession());

await listen("push_to_talk_toggled", () => {
  console.info("[panel] hotkey fired");
  void togglePushToTalk();
});

const modeSelect = document.getElementById("mode-select") as HTMLSelectElement | null;
async function loadOperatingMode() {
  if (!modeSelect) return;
  try {
    const current = await invoke<string>("get_operating_mode");
    modeSelect.value = current;
  } catch (error) {
    console.warn("[panel] failed to load operating mode:", error);
  }
}
modeSelect?.addEventListener("change", async () => {
  if (!modeSelect) return;
  try {
    await invoke("set_operating_mode", { mode: modeSelect.value });
  } catch (error) {
    showError("Failed to set mode: " + (error instanceof Error ? error.message : String(error)));
  }
});

// ----------------------------------------------------------------------
// Saved flows (multiflow) panel section
// ----------------------------------------------------------------------

interface FlowSummary {
  flowId: string;
  name: string;
  createdAtUnixMs: number;
  stepCount: number;
  triggerAliases: string[];
}

const recordFlowToggleButton = document.getElementById(
  "record-flow-toggle",
) as HTMLButtonElement | null;
const recordFlowNamingRow = document.getElementById(
  "record-flow-naming",
) as HTMLDivElement | null;
const recordFlowNameInput = document.getElementById(
  "record-flow-name-input",
) as HTMLInputElement | null;
const recordFlowConfirmButton = document.getElementById(
  "record-flow-confirm",
) as HTMLButtonElement | null;
const recordFlowCancelButton = document.getElementById(
  "record-flow-cancel",
) as HTMLButtonElement | null;
const savedFlowsListElement = document.getElementById(
  "saved-flows-list",
) as HTMLUListElement | null;

let isRecordingFlowInProgress = false;

// Recording opt-in toggle. The recorder backend refuses to start a
// demonstration unless `set_recording_enabled(true)` was called and
// persisted. Without this UI wire-up the checkbox added in index.html
// would be cosmetic and "Record new flow" would always error out with
// "recording is not enabled — user must opt in".
const recordingOptInToggle = document.getElementById(
  "recording-opt-in-toggle",
) as HTMLInputElement | null;

async function loadRecordingOptInInitialState() {
  if (!recordingOptInToggle) return;
  try {
    const isEnabled = await invoke<boolean>("is_recording_enabled");
    recordingOptInToggle.checked = isEnabled;
  } catch (error) {
    console.warn("[panel] is_recording_enabled failed:", error);
  }
}

recordingOptInToggle?.addEventListener("change", async () => {
  if (!recordingOptInToggle) return;
  const userWantsRecordingEnabled = recordingOptInToggle.checked;
  try {
    await invoke("set_recording_enabled", { enabled: userWantsRecordingEnabled });
  } catch (error) {
    // Revert the visible state so the toggle reflects truth on failure.
    recordingOptInToggle.checked = !userWantsRecordingEnabled;
    showError(
      "Set recording enabled failed: " +
        (error instanceof Error ? error.message : String(error)),
    );
  }
});

function setRecordingButtonState(isRecording: boolean) {
  if (!recordFlowToggleButton) return;
  isRecordingFlowInProgress = isRecording;
  recordFlowToggleButton.textContent = isRecording ? "Stop recording" : "Record new flow";
  recordFlowToggleButton.dataset.recording = isRecording ? "true" : "false";
}

function showFlowNamingRow() {
  if (!recordFlowNamingRow || !recordFlowNameInput) return;
  recordFlowNamingRow.hidden = false;
  recordFlowNameInput.value = "";
  recordFlowNameInput.focus();
}

function hideFlowNamingRow() {
  if (!recordFlowNamingRow) return;
  recordFlowNamingRow.hidden = true;
}

async function refreshSavedFlowsList() {
  if (!savedFlowsListElement) return;
  let flows: FlowSummary[] = [];
  try {
    flows = await invoke<FlowSummary[]>("list_flows");
  } catch (error) {
    console.warn("[panel] list_flows failed:", error);
  }

  savedFlowsListElement.innerHTML = "";
  if (flows.length === 0) {
    const emptyLine = document.createElement("li");
    emptyLine.className = "saved-flows-empty";
    emptyLine.textContent = "No saved flows yet. Record one to recall by voice.";
    savedFlowsListElement.appendChild(emptyLine);
    return;
  }

  for (const flow of flows) {
    const flowRowElement = document.createElement("li");
    flowRowElement.className = "saved-flow-row";

    const headerElement = document.createElement("div");
    headerElement.className = "saved-flow-row-header";

    const nameElement = document.createElement("span");
    nameElement.className = "saved-flow-name";
    nameElement.textContent = flow.name;
    nameElement.title = flow.name;

    const stepCountElement = document.createElement("span");
    stepCountElement.className = "saved-flow-steps";
    stepCountElement.textContent = `${flow.stepCount} steps`;

    const buttonsElement = document.createElement("div");
    buttonsElement.className = "saved-flow-buttons";

    const runButton = document.createElement("button");
    runButton.textContent = "Run";
    runButton.className = "saved-flow-run";
    runButton.addEventListener("click", async () => {
      try {
        await invoke<string>("run_flow_by_name", { name: flow.name });
      } catch (error) {
        showError("Run flow failed: " + (error instanceof Error ? error.message : String(error)));
      }
    });

    const deleteButton = document.createElement("button");
    deleteButton.textContent = "Delete";
    deleteButton.addEventListener("click", async () => {
      try {
        await invoke("delete_flow", { name: flow.flowId });
        await refreshSavedFlowsList();
      } catch (error) {
        showError(
          "Delete flow failed: " + (error instanceof Error ? error.message : String(error)),
        );
      }
    });

    buttonsElement.appendChild(runButton);
    buttonsElement.appendChild(deleteButton);
    headerElement.appendChild(nameElement);
    headerElement.appendChild(stepCountElement);
    headerElement.appendChild(buttonsElement);
    flowRowElement.appendChild(headerElement);

    // Trigger aliases mini-input. User edits a comma-separated list and
    // we persist on blur so the panel doesn't spam the backend per keystroke.
    const aliasesWrapperElement = document.createElement("div");
    aliasesWrapperElement.className = "saved-flow-aliases";
    const aliasesLabel = document.createElement("label");
    aliasesLabel.textContent = "Triggers (comma-separated)";
    const aliasesInput = document.createElement("input");
    aliasesInput.type = "text";
    aliasesInput.value = flow.triggerAliases.join(", ");
    aliasesInput.placeholder = "morning routine, start of day";
    aliasesInput.addEventListener("blur", async () => {
      const aliasesArray = aliasesInput.value
        .split(",")
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0);
      try {
        await invoke("set_flow_trigger_aliases", {
          flowId: flow.flowId,
          triggerAliases: aliasesArray,
        });
      } catch (error) {
        showError(
          "Save aliases failed: " + (error instanceof Error ? error.message : String(error)),
        );
      }
    });
    aliasesWrapperElement.appendChild(aliasesLabel);
    aliasesWrapperElement.appendChild(aliasesInput);
    flowRowElement.appendChild(aliasesWrapperElement);

    savedFlowsListElement.appendChild(flowRowElement);
  }
}

async function startRecordingNewFlow(flowName: string) {
  try {
    await invoke<string>("start_recording_flow", { name: flowName });
    setRecordingButtonState(true);
    hideFlowNamingRow();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    // The recorder requires opt-in via set_recording_enabled; surface that
    // hint inline rather than just dropping the error blindly.
    showError(
      `Start recording failed: ${message}. ` +
        `If recording is disabled, enable it via the recorder settings first.`,
    );
  }
}

async function stopRecordingFlow() {
  try {
    await invoke<FlowSummary>("stop_recording_flow");
    setRecordingButtonState(false);
    await refreshSavedFlowsList();
  } catch (error) {
    showError(
      "Stop recording failed: " + (error instanceof Error ? error.message : String(error)),
    );
  }
}

recordFlowToggleButton?.addEventListener("click", () => {
  if (isRecordingFlowInProgress) {
    void stopRecordingFlow();
  } else {
    showFlowNamingRow();
  }
});

recordFlowConfirmButton?.addEventListener("click", () => {
  const flowName = recordFlowNameInput?.value.trim() ?? "";
  if (!flowName) {
    showError("Give the flow a name first.");
    return;
  }
  void startRecordingNewFlow(flowName);
});

recordFlowCancelButton?.addEventListener("click", () => {
  hideFlowNamingRow();
});

// ----------------------------------------------------------------------
// Always-on Vosk listener panel section
// ----------------------------------------------------------------------

type VoskListenerUiState = "disabled" | "downloading" | "ready" | "listening" | "error";

const alwaysOnListeningToggle = document.getElementById(
  "always-on-listening-toggle",
) as HTMLInputElement | null;
const alwaysOnListeningStatusElement = document.getElementById(
  "always-on-listening-status",
) as HTMLElement | null;
const alwaysOnListeningHeardElement = document.getElementById(
  "always-on-listening-heard",
) as HTMLElement | null;

function setAlwaysOnListeningUiState(uiState: VoskListenerUiState, statusLabelOverride?: string) {
  if (!alwaysOnListeningStatusElement) return;
  alwaysOnListeningStatusElement.dataset.state = uiState;
  alwaysOnListeningStatusElement.textContent =
    statusLabelOverride ??
    (uiState === "disabled"
      ? "Disabled"
      : uiState === "downloading"
        ? "Downloading model"
        : uiState === "ready"
          ? "Ready"
          : uiState === "listening"
            ? "Listening"
            : "Error");
}

async function loadAlwaysOnListenerInitialState() {
  if (!alwaysOnListeningToggle) return;
  try {
    const persistedEnabled = await invoke<boolean>("is_listener_enabled");
    alwaysOnListeningToggle.checked = persistedEnabled;
    setAlwaysOnListeningUiState(persistedEnabled ? "listening" : "disabled");
  } catch (error) {
    console.warn("[panel] is_listener_enabled failed:", error);
    setAlwaysOnListeningUiState("disabled");
  }
}

alwaysOnListeningToggle?.addEventListener("change", async () => {
  if (!alwaysOnListeningToggle) return;
  const userWantsEnabled = alwaysOnListeningToggle.checked;
  if (!userWantsEnabled) {
    try {
      await invoke("set_listener_enabled", { enabled: false });
      setAlwaysOnListeningUiState("disabled");
    } catch (error) {
      showError(
        "Disable listener failed: " + (error instanceof Error ? error.message : String(error)),
      );
    }
    return;
  }

  // Enabling for the first time: pull the model first, then flip
  // the persisted enabled flag (set_listener_enabled also boots the
  // background mic stream when given true).
  setAlwaysOnListeningUiState("downloading");
  try {
    await invoke("download_vosk_model_if_needed");
  } catch (error) {
    setAlwaysOnListeningUiState("error", "Model download failed");
    alwaysOnListeningToggle.checked = false;
    showError(
      "Vosk model download failed: " + (error instanceof Error ? error.message : String(error)),
    );
    return;
  }

  try {
    await invoke("set_listener_enabled", { enabled: true });
    setAlwaysOnListeningUiState("listening");
  } catch (error) {
    setAlwaysOnListeningUiState("error", "Start failed");
    alwaysOnListeningToggle.checked = false;
    showError(
      "Start listener failed: " + (error instanceof Error ? error.message : String(error)),
    );
  }
});

await listen<string>("vosk_partial_transcript", (event) => {
  if (!alwaysOnListeningHeardElement) return;
  const heardText = event.payload?.trim();
  if (!heardText) {
    alwaysOnListeningHeardElement.hidden = true;
    return;
  }
  alwaysOnListeningHeardElement.hidden = false;
  alwaysOnListeningHeardElement.textContent = `heard: "${heardText}"`;
});

await listen("vosk_wake_detected", () => {
  console.info("[panel] vosk wake word detected");
});

await listen("vosk_command_stop", () => {
  console.info("[panel] vosk stop command");
  void stopSession();
});

await listen("vosk_command_pause", () => {
  // Pause cancels any in-flight multiflow replay without closing the
  // Gemini Live session, so the user can keep conversing while the
  // automation halts. Stop, in contrast, tears down the whole session.
  console.info("[panel] vosk pause command");
  void invoke("pause_active_replay");
});

await loadStoredApiKey();
await loadOperatingMode();
await refreshPermissions();
await refreshSavedFlowsList();
await loadRecordingOptInInitialState();
await loadAlwaysOnListenerInitialState();
setStatus("idle");
console.info("[panel] ready");
