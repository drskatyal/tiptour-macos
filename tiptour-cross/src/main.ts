import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { GeminiLiveSession, SessionStatus } from "./gemini/GeminiLiveSession";
import { showOnboardingIfNeeded } from "./onboarding";
import { installThemeBridge } from "./theme";

void installThemeBridge();

// Fail-loud panel: when ANY uncaught error or unhandled promise
// rejection slips through, slap a banner across the panel + log to
// console so we can debug live. Banner includes a Dismiss button so
// the user can clear it once they've copied the message.
function renderFatalErrorOverlay(message: string): void {
  console.error("[panel-error]", message);
  let overlay = document.getElementById("panel-fatal-error-overlay");
  if (!overlay) {
    overlay = document.createElement("div");
    overlay.id = "panel-fatal-error-overlay";
    overlay.style.cssText = [
      "position:fixed",
      "top:0",
      "left:0",
      "right:0",
      "z-index:2147483647",
      "padding:10px 36px 10px 14px",
      "background:#7a1c1c",
      "color:#fff",
      "font:12px/1.4 ui-monospace,monospace",
      "white-space:pre-wrap",
      "border-bottom:1px solid #ff8a8a",
      "max-height:40vh",
      "overflow:auto",
    ].join(";");
    const dismissButton = document.createElement("button");
    dismissButton.textContent = "×";
    dismissButton.setAttribute("aria-label", "Dismiss error");
    dismissButton.style.cssText = [
      "position:absolute",
      "top:4px",
      "right:8px",
      "background:transparent",
      "border:none",
      "color:#fff",
      "font:16px/1 sans-serif",
      "cursor:pointer",
    ].join(";");
    dismissButton.addEventListener("click", () => overlay?.remove());
    overlay.appendChild(dismissButton);
    document.body.appendChild(overlay);
  }
  const line = document.createElement("div");
  line.textContent = message;
  overlay.appendChild(line);
}
window.addEventListener("error", (event) => {
  renderFatalErrorOverlay(`${event.message} @ ${event.filename}:${event.lineno}`);
});
window.addEventListener("unhandledrejection", (event) => {
  const reason =
    event.reason instanceof Error
      ? (event.reason.stack ?? event.reason.message)
      : typeof event.reason === "object"
        ? JSON.stringify(event.reason)
        : String(event.reason);
  renderFatalErrorOverlay(`Unhandled rejection: ${reason}`);
});

// Wraps an `await listen(...)` call so a single failure (e.g. the
// Tauri events plugin denying a label that's missing from
// capabilities) doesn't blow up the whole panel boot.
// Signature kept compatible with `@tauri-apps/api/event` listen: the
// callback receives the full Event<T> object, not just `{ payload }`.
async function safeListen<T>(
  eventName: string,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  callback: (event: any) => void | Promise<void>,
): Promise<void> {
  try {
    await listen<T>(eventName, callback);
  } catch (listenError) {
    console.warn(`[panel] listen(${eventName}) failed:`, listenError);
  }
}

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

// macOS caches Screen Recording entitlement per-pid in TCC. Even after
// the user flips the toggle in System Settings the running TipTour
// process can't see the change — capture keeps failing until the app
// is relaunched. The old flow showed a transient error toast right
// after the user clicked Grant, which fired before they'd even left
// for System Settings and felt like a bug ("I clicked Grant and got
// an error?"). The new flow:
//   1. Snapshot the current permission state at click time.
//   2. Open System Settings via request_screen_recording_permission.
//   3. Poll every 1s for up to 30s. If the permission flips
//      false→true, the kernel granted it but THIS process can't see it
//      until relaunch — surface a persistent "Restart TipTour" banner
//      with a one-click restart button. Far less ambiguous than a
//      vanishing toast.
const restartRequiredBanner = document.getElementById("restart-required-banner")!;
const restartNowButton = document.getElementById("restart-now-button")!;

let screenRecordingPollerHandle: number | null = null;

function showRestartRequiredBanner() {
  restartRequiredBanner.hidden = false;
}

restartNowButton.addEventListener("click", async () => {
  try {
    await invoke("quit_app_gracefully");
  } catch (restartError) {
    showError(
      "Could not auto-restart. Quit TipTour from the menu bar and reopen it. " +
        (restartError instanceof Error ? restartError.message : String(restartError)),
    );
  }
});

async function watchForScreenRecordingPermissionFlip(initiallyHadPermission: boolean) {
  if (screenRecordingPollerHandle !== null) {
    window.clearInterval(screenRecordingPollerHandle);
    screenRecordingPollerHandle = null;
  }
  let elapsedSeconds = 0;
  screenRecordingPollerHandle = window.setInterval(async () => {
    elapsedSeconds += 1;
    try {
      const nowGranted = await invoke<boolean>("check_screen_recording_permission");
      // Permission flipped from "not granted" to "granted" in System
      // Settings. The kernel has the entitlement; THIS pid still can't
      // see captured frames until a relaunch. Surface the banner.
      if (nowGranted && !initiallyHadPermission) {
        showRestartRequiredBanner();
        if (screenRecordingPollerHandle !== null) {
          window.clearInterval(screenRecordingPollerHandle);
          screenRecordingPollerHandle = null;
        }
        return;
      }
    } catch {
      // ignore — keep polling
    }
    if (elapsedSeconds >= 30) {
      // Give up after 30s. The user either granted it (caught above)
      // or decided not to right now. We don't keep polling forever.
      if (screenRecordingPollerHandle !== null) {
        window.clearInterval(screenRecordingPollerHandle);
        screenRecordingPollerHandle = null;
      }
    }
  }, 1000);
}

grantScreenRecordingButton.addEventListener("click", async () => {
  // Capture the current state *before* opening System Settings so we
  // can detect the false→true transition specifically (vs the user
  // having already granted it from a previous attempt).
  let alreadyHadPermissionBeforeGrantClick = false;
  try {
    alreadyHadPermissionBeforeGrantClick = await invoke<boolean>(
      "check_screen_recording_permission",
    );
  } catch {
    // assume not granted — that's the worse-but-safe default
  }
  try {
    await invoke("request_screen_recording_permission");
  } catch (grantError) {
    showError(
      "Could not open Screen Recording settings: " +
        (grantError instanceof Error ? grantError.message : String(grantError)),
    );
    return;
  }
  // Refresh the row immediately in case the user had already granted
  // before clicking, then start polling for the flip.
  setTimeout(() => void refreshPermissions(), 500);
  void watchForScreenRecordingPermissionFlip(alreadyHadPermissionBeforeGrantClick);
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

  // Mirror the panel status into the indicator/tray channel so the
  // Rust side can swap the menu bar icon to its "active" variant
  // whenever a Gemini session is live. We send the canonical lowercase
  // status string and let the Rust handler decide whether to swap.
  void invoke("emit_indicator_from_frontend", {
    kind: "sessionStatus",
    title: status,
    subtitle: "",
    sourceId: null,
  }).catch(() => undefined);

  // Broadcast to the dock so it pulses the pill blue while any
  // session is active. The dock listens for this event and flips
  // data-active accordingly. Without this the dock had no idea a
  // Live session was open.
  void import("@tauri-apps/api/event").then(({ emit }) =>
    emit("panel_session_state", isActive ? "listening" : "idle"),
  );

  // Mirror into the tray so the Start/Stop session toggle reflects
  // the live session state without polling.
  void invoke("set_tray_session_active", { active: isActive }).catch(() => undefined);
}

function showError(message: string) {
  errorBanner.textContent = message;
  errorBanner.hidden = false;
  // Mirror every panel-surfaced error into the side-of-screen indicator
  // strip so the user notices even when the panel isn't visible. The
  // Rust emit handler honors the per-type enable flag, so users who
  // disabled error pills won't see anything.
  void invoke("emit_indicator_from_frontend", {
    kind: "error",
    title: "TipTour error",
    subtitle: message,
    sourceId: null,
  }).catch(() => undefined);
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

// "Need a key?" inline help row — visible while the field is empty and
// the user hasn't saved a key yet, hidden once the keychain has one.
const apiKeyHelpRow = document.getElementById("api-key-help-row") as HTMLDivElement | null;
function refreshApiKeyHelpRowVisibility(): void {
  if (!apiKeyHelpRow) return;
  apiKeyHelpRow.hidden = apiKeyInput.value.trim().length > 0;
}
apiKeyInput.addEventListener("input", refreshApiKeyHelpRowVisibility);

async function loadStoredApiKey() {
  try {
    const key = await invoke<string | null>("get_api_key");
    if (key) apiKeyInput.value = key;
  } catch (error) {
    console.warn("[panel] no stored key:", error);
  }
  refreshApiKeyHelpRowVisibility();
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
    refreshApiKeyHelpRowVisibility();
    showSavedToast("Key saved");
  } catch (error) {
    showError("Failed to save key: " + (error instanceof Error ? error.message : String(error)));
  }
});

// Lightweight "Saved" toast. Auto-dismisses after 1.5s. Lives in a
// fixed overlay so it doesn't reflow the panel layout. We dedupe rapid
// successive calls by reusing the same toast element rather than
// stacking — a user mashing save shouldn't get a vertical pile.
let savedToastElement: HTMLDivElement | null = null;
let savedToastDismissTimer: number | null = null;
function showSavedToast(message: string): void {
  if (!savedToastElement) {
    savedToastElement = document.createElement("div");
    savedToastElement.className = "saved-toast";
    document.body.appendChild(savedToastElement);
  }
  savedToastElement.textContent = message;
  savedToastElement.dataset.visible = "true";
  if (savedToastDismissTimer !== null) {
    window.clearTimeout(savedToastDismissTimer);
  }
  savedToastDismissTimer = window.setTimeout(() => {
    if (savedToastElement) savedToastElement.dataset.visible = "false";
    savedToastDismissTimer = null;
  }, 1500);
}

interface PanelAppSettings {
  schemaVersion: number;
  geminiVoice: string;
  geminiModel: string;
  pushToTalkChord: string;
  theme: string;
}

// Visible debug strip that prints each step of the session start
// flow into the panel itself. The user couldn't see anything when
// Start listening did nothing — now every step lands on screen so
// the failure point is obvious.
function debugTrace(message: string): void {
  console.info("[panel-trace]", message);
  let strip = document.getElementById("panel-debug-strip");
  if (!strip) {
    strip = document.createElement("div");
    strip.id = "panel-debug-strip";
    strip.style.cssText = [
      "position:fixed",
      "left:8px",
      "right:8px",
      "bottom:8px",
      "z-index:2147483646",
      "padding:8px 10px",
      "background:rgba(15,15,20,0.88)",
      "color:#9be0ff",
      "font:11px/1.35 ui-monospace,monospace",
      "border:1px solid rgba(155,224,255,0.30)",
      "border-radius:8px",
      "max-height:140px",
      "overflow:auto",
      "pointer-events:none",
    ].join(";");
    document.body.appendChild(strip);
  }
  const line = document.createElement("div");
  const stamp = new Date().toLocaleTimeString();
  line.textContent = `${stamp}  ${message}`;
  strip.appendChild(line);
  strip.scrollTop = strip.scrollHeight;
}

async function startSession() {
  debugTrace("startSession() called");
  const key = apiKeyInput.value.trim();
  if (!key) {
    debugTrace("startSession: no key in input");
    showError("Paste a Gemini API key first.");
    return;
  }
  debugTrace(`startSession: key length=${key.length}`);
  clearError();
  console.info("[panel] starting session");

  // Pull the user's selected voice/model from settings.json so the
  // dashboard's General-tab pickers actually take effect. Failing to
  // read settings shouldn't block a session — we just fall back to the
  // client's compile-time defaults.
  let panelAppSettings: PanelAppSettings | null = null;
  try {
    panelAppSettings = await invoke<PanelAppSettings>("get_app_settings");
  } catch (settingsError) {
    console.warn("[panel] get_app_settings failed:", settingsError);
  }

  debugTrace(`startSession: model=${panelAppSettings?.geminiModel ?? "(default)"}`);
  session = new GeminiLiveSession({
    apiKey: key,
    voiceName: panelAppSettings?.geminiVoice,
    modelShortId: panelAppSettings?.geminiModel,
    onStatusChange: (newStatus) => {
      debugTrace(`session status -> ${newStatus}`);
      setStatus(newStatus);
    },
    onUserTranscript: (text) => {
      debugTrace(`user transcript: ${text.slice(0, 60)}`);
      appendTranscript("user", text);
    },
    onModelTranscript: (text) => {
      debugTrace(`model transcript: ${text.slice(0, 60)}`);
      appendTranscript("model", text);
    },
    onError: (message) => {
      debugTrace(`session ERROR: ${message}`);
      showError(message);
    },
  });

  try {
    debugTrace("session.open() awaiting…");
    await session.open();
    debugTrace("session.open() resolved");
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    debugTrace(`session.open() threw: ${detail}`);
    console.error("[panel] session.open threw:", error);
    showError(
      `Couldn't open Gemini session: ${detail}. ` +
        `Verify the API key is valid and that you have network access.`,
    );
    setStatus("error");
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

startButton.addEventListener("click", () => {
  debugTrace("Start listening clicked");
  void startSession();
});

// Live counters from the running Gemini Live session — exposes
// whether mic audio + screen frames are actually reaching Gemini.
// "Gemini just says hi back" almost always means micChunkCount is
// staying 0 (mic permission denied or wrong device).
window.addEventListener("session-flow-stats", (event) => {
  const detail = (event as CustomEvent<{ micChunkCount: number; screenFrameCount: number }>)
    .detail;
  debugTrace(
    `flow: mic chunks=${detail.micChunkCount}  screen frames=${detail.screenFrameCount}`,
  );
});
stopButton.addEventListener("click", () => {
  debugTrace("Stop clicked");
  void stopSession();
});

// Gear icon in the panel header opens the deep-edit Settings window.
const openSettingsButton = document.getElementById(
  "open-settings-button",
) as HTMLButtonElement | null;
openSettingsButton?.addEventListener("click", async () => {
  try {
    await invoke("open_settings_window");
  } catch (openSettingsError) {
    showError(
      "Could not open settings window: " +
        (openSettingsError instanceof Error
          ? openSettingsError.message
          : String(openSettingsError)),
    );
  }
});

// Close button in the panel header hides this window. The tray icon
// re-shows it. We hide rather than destroy so panel state (current
// session, transcript, mode, persona) survives the next reopen.
const hidePanelButton = document.getElementById(
  "hide-panel-button",
) as HTMLButtonElement | null;
hidePanelButton?.addEventListener("click", async () => {
  // Collapse to the side-edge tab instead of fully hiding so the
  // user has a one-click affordance to bring the panel back without
  // hunting for the tray icon. The Rust setup hook listens for
  // panel_collapse_to_tab and orchestrates hide(panel) + show(tab).
  try {
    const { emit } = await import("@tauri-apps/api/event");
    await emit("panel_collapse_to_tab");
  } catch (hidePanelError) {
    showError(
      "Could not collapse panel: " +
        (hidePanelError instanceof Error
          ? hidePanelError.message
          : String(hidePanelError)),
    );
  }
});

// ---------------------------------------------------------------------
// Push-to-talk hotkey routing
// ---------------------------------------------------------------------
// Two voice modes share the hotkey:
//   quick (default) — one-shot REST round-trip to gemini-2.5-flash-lite
//                     with audio + tools, result lands in the floating
//                     command tooltip. No TTS. ~3-7s per command.
//                     Cheap and zero-state.
//   live           — open a streaming WebSocket Gemini Live session
//                     with TTS reply for real conversation. Costs more
//                     and stays open until the user stops it.
//
// Mode is read from app_settings.voiceMode and defaults to "quick"
// because the typical hotkey use case is a single command, not a
// dialogue.

type QuickCaptureState = "idle" | "armed";
let quickCaptureState: QuickCaptureState = "idle";
let quickMicChunkUnlisten: (() => void) | null = null;

async function beginQuickCaptureLoop(): Promise<void> {
  debugTrace("quick-capture: begin");
  quickCaptureState = "armed";
  try {
    await invoke("begin_quick_voice_capture");
    debugTrace("quick-capture: begin_quick_voice_capture OK");
  } catch (error) {
    debugTrace(`quick-capture: begin_quick_voice_capture FAILED ${String(error)}`);
    quickCaptureState = "idle";
    throw error;
  }
  try {
    await invoke("start_mic_capture");
    debugTrace("quick-capture: start_mic_capture OK");
  } catch (error) {
    debugTrace(`quick-capture: start_mic_capture FAILED ${String(error)}`);
    quickCaptureState = "idle";
    throw error;
  }
  quickMicChunkUnlisten = await listen<number[]>("mic_chunk", (event) => {
    if (quickCaptureState !== "armed") return;
    void invoke("append_quick_voice_chunk", { pcmBytes: event.payload });
  });
  debugTrace("quick-capture: mic_chunk listener attached");
}

async function endQuickCaptureLoop(): Promise<void> {
  debugTrace("quick-capture: end");
  quickCaptureState = "idle";
  try {
    await invoke("stop_mic_capture");
    debugTrace("quick-capture: stop_mic_capture OK");
  } catch (stopError) {
    debugTrace(`quick-capture: stop_mic_capture FAILED ${String(stopError)}`);
  }
  if (quickMicChunkUnlisten) {
    quickMicChunkUnlisten();
    quickMicChunkUnlisten = null;
  }
  try {
    const reply = await invoke<string | null>("end_quick_voice_capture_and_dispatch");
    debugTrace(`quick-capture: dispatch OK reply=${String(reply).slice(0, 80)}`);
  } catch (dispatchError) {
    debugTrace(`quick-capture: dispatch FAILED ${String(dispatchError)}`);
    showError(
      `Quick command failed: ${
        dispatchError instanceof Error ? dispatchError.message : String(dispatchError)
      }`,
    );
  }
}

async function handleHotkeyToggle(): Promise<void> {
  // Read mode every press so a settings flip applies immediately —
  // no relaunch required.
  let voiceMode = "live";
  try {
    const s = await invoke<{ voiceMode?: string }>("get_app_settings");
    if (s?.voiceMode === "live" || s?.voiceMode === "quick") voiceMode = s.voiceMode;
  } catch (modeError) {
    console.warn("[panel] get_app_settings failed, defaulting to live:", modeError);
  }

  if (voiceMode === "quick") {
    if (quickCaptureState === "armed") {
      await endQuickCaptureLoop();
    } else {
      await beginQuickCaptureLoop();
    }
    return;
  }

  // Live mode — existing path
  await togglePushToTalk();
}

// Quick/Live mode + hotkey behavior selectors in the panel. Sync
// straight back to settings.json so the change applies live without
// a settings-window roundtrip. Read on boot to reflect what's
// already persisted.
const voiceModeSelect = document.getElementById(
  "voice-mode-select",
) as HTMLSelectElement | null;
const hotkeyBehaviorSelect = document.getElementById(
  "hotkey-behavior-select",
) as HTMLSelectElement | null;
const presenceModeSelect = document.getElementById(
  "presence-mode-select",
) as HTMLSelectElement | null;

interface VoiceAndHotkeySettings {
  voiceMode?: string;
  hotkeyBehavior?: string;
  presenceMode?: string;
}

async function loadVoiceAndHotkeyPanelControls(): Promise<void> {
  try {
    const s = await invoke<VoiceAndHotkeySettings>("get_app_settings");
    if (voiceModeSelect && (s?.voiceMode === "live" || s?.voiceMode === "quick")) {
      voiceModeSelect.value = s.voiceMode;
    }
    if (
      hotkeyBehaviorSelect &&
      (s?.hotkeyBehavior === "toggle" || s?.hotkeyBehavior === "hold")
    ) {
      hotkeyBehaviorSelect.value = s.hotkeyBehavior;
    }
    if (
      presenceModeSelect &&
      (s?.presenceMode === "dock" ||
        s?.presenceMode === "cursor-buddy" ||
        s?.presenceMode === "notch")
    ) {
      presenceModeSelect.value = s.presenceMode;
    }
  } catch (loadError) {
    console.warn("[panel] could not load voice/hotkey settings:", loadError);
  }
}

presenceModeSelect?.addEventListener("change", async () => {
  try {
    await invoke("set_app_setting_field", {
      field: "presence_mode",
      value: presenceModeSelect.value,
    });
    // Tell the running app to swap the active presence surface live
    // (show/hide dock, kick off cursor buddy, etc.) — the Rust side
    // listens for this event and reconfigures windows.
    const { emit } = await import("@tauri-apps/api/event");
    await emit("presence_mode_changed", presenceModeSelect.value);
  } catch (saveError) {
    showError(
      "Could not change presence mode: " +
        (saveError instanceof Error ? saveError.message : String(saveError)),
    );
  }
});

voiceModeSelect?.addEventListener("change", async () => {
  try {
    await invoke("set_app_setting_field", {
      field: "voice_mode",
      value: voiceModeSelect.value,
    });
    // Picking Bidirectional or Unidirectional carries an implicit
    // model — Live needs a *-live-preview, Lite needs flash-lite.
    // Sync the model so the user never ends up in the broken state
    // where Voice=Live but Model=flash-lite (which the server
    // silently rejects with 'not supported for bidiGenerateContent').
    const matchedModel =
      voiceModeSelect.value === "live"
        ? "gemini-3.1-flash-live-preview"
        : "gemini-2.5-flash-lite";
    await invoke("set_app_setting_field", {
      field: "gemini_model",
      value: matchedModel,
    });
    debugTrace(`voice mode -> ${voiceModeSelect.value}, model -> ${matchedModel}`);
  } catch (saveError) {
    showError(
      "Could not save voice mode: " +
        (saveError instanceof Error ? saveError.message : String(saveError)),
    );
  }
});

hotkeyBehaviorSelect?.addEventListener("change", async () => {
  try {
    await invoke("set_app_setting_field", {
      field: "hotkey_behavior",
      value: hotkeyBehaviorSelect.value,
    });
  } catch (saveError) {
    showError(
      "Could not save hotkey behavior: " +
        (saveError instanceof Error ? saveError.message : String(saveError)),
    );
  }
});

// When the user picks "Hold to record" we ignore press-toggle and
// only act on the press→release transition (begin on press, stop +
// dispatch on release). Default "toggle" semantics ignore release
// entirely so a long hold reads as a single press.
let lastHotkeyBehavior: "toggle" | "hold" = "toggle";
async function refreshLastHotkeyBehavior(): Promise<void> {
  try {
    const s = await invoke<{ hotkeyBehavior?: string }>("get_app_settings");
    lastHotkeyBehavior = s?.hotkeyBehavior === "hold" ? "hold" : "toggle";
  } catch (_err) {
    lastHotkeyBehavior = "toggle";
  }
}
void refreshLastHotkeyBehavior();
// Re-read on every focus so a settings flip applies live without
// having to restart the app.
window.addEventListener("focus", () => void refreshLastHotkeyBehavior());

await safeListen("push_to_talk_toggled", () => {
  console.info("[panel] hotkey press");
  debugTrace("hotkey press received (Alt+X)");
  // Refresh once per press so a settings flip is picked up on the
  // very next hotkey use, not the one after that.
  void refreshLastHotkeyBehavior().then(() => {
    if (lastHotkeyBehavior === "hold") {
      // Hold-to-record: start capture on press; release will end it.
      if (quickCaptureState === "idle") {
        void beginQuickCaptureLoop().catch((startError) => {
          showError(
            "Could not start capture: " +
              (startError instanceof Error ? startError.message : String(startError)),
          );
        });
      }
    } else {
      void handleHotkeyToggle();
    }
  });
});

await safeListen("push_to_talk_released", () => {
  console.info("[panel] hotkey release");
  if (lastHotkeyBehavior === "hold" && quickCaptureState === "armed") {
    void endQuickCaptureLoop().catch((endError) => {
      showError(
        "Could not end capture: " +
          (endError instanceof Error ? endError.message : String(endError)),
      );
    });
  }
});

// Transcribe hotkey (default Alt+Z) — toggles Soniox real-time
// transcription into whatever has focus. Independent of the
// push-to-talk path; same listener whether the chord fired from the
// global registration, the dock button, or a future voice command.
await safeListen("transcribe_toggled", async () => {
  console.info("[panel] transcribe hotkey fired");
  try {
    await invoke("toggle_soniox_transcription");
  } catch (transcribeError) {
    showError(
      `Transcription toggle failed: ${
        transcribeError instanceof Error
          ? transcribeError.message
          : String(transcribeError)
      }`,
    );
  }
});

// Soniox real-time transcription — when the Rust side is in an
// active session, every mic chunk should also forward to the Soniox
// pipeline so it can stream tokens out. A separate listener so it
// runs independently of quick-voice and live-session paths.
let sonioxState: "idle" | "transcribing" = "idle";
await safeListen<string>("soniox_state", (event) => {
  if (event.payload === "transcribing") {
    sonioxState = "transcribing";
  } else {
    sonioxState = "idle";
  }
});
await safeListen<number[]>("mic_chunk", (event) => {
  if (sonioxState === "transcribing") {
    void invoke("append_soniox_audio_chunk", { pcmBytes: event.payload });
  }
});

// If the OS refused to register the global hotkey (most common cause:
// macOS Accessibility permission not granted, or another app stole the
// chord), surface it as a panel error banner so users stop pressing
// Alt+X expecting silence to mean "broken app".
await safeListen<string>("hotkey_registration_failed", (event) => {
  const detail = event.payload ?? "unknown";
  showError(
    `Push-to-talk hotkey couldn't register (${detail}). ` +
      `On macOS, grant Accessibility in System Settings → Privacy. ` +
      `Or change the chord in Settings → General → Push-to-talk.`,
  );
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

// Names of flows the user has just kicked off but whose multiflow_progress
// terminal event we haven't yet observed. Used to disable Run on a flow
// that's already running so a second click doesn't silently cancel the
// first via the token-supersession path.
const flowNamesCurrentlyReplaying = new Set<string>();

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
    runButton.className = "saved-flow-run";
    const isAlreadyReplaying = flowNamesCurrentlyReplaying.has(flow.name);
    runButton.textContent = isAlreadyReplaying ? "Running…" : "Run";
    runButton.disabled = isAlreadyReplaying;
    runButton.addEventListener("click", async () => {
      if (flowNamesCurrentlyReplaying.has(flow.name)) {
        showError(`"${flow.name}" is already running. Wait for it to finish or say "pause".`);
        return;
      }
      flowNamesCurrentlyReplaying.add(flow.name);
      void refreshSavedFlowsList();
      try {
        await invoke<string>("run_flow_by_name", { name: flow.name });
      } catch (error) {
        flowNamesCurrentlyReplaying.delete(flow.name);
        void refreshSavedFlowsList();
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

await safeListen<string>("vosk_partial_transcript", (event) => {
  if (!alwaysOnListeningHeardElement) return;
  const heardText = event.payload?.trim();
  if (!heardText) {
    alwaysOnListeningHeardElement.hidden = true;
    return;
  }
  alwaysOnListeningHeardElement.hidden = false;
  alwaysOnListeningHeardElement.textContent = `heard: "${heardText}"`;
});

await safeListen("vosk_wake_detected", () => {
  console.info("[panel] vosk wake word detected");
});

await safeListen("vosk_command_stop", () => {
  console.info("[panel] vosk stop command");
  void stopSession();
});

await safeListen("vosk_command_pause", () => {
  // Pause cancels any in-flight multiflow replay without closing the
  // Gemini Live session, so the user can keep conversing while the
  // automation halts. Stop, in contrast, tears down the whole session.
  console.info("[panel] vosk pause command");
  void invoke("pause_active_replay");
});

// Panel-level multiflow_progress listener — distinct from the one inside
// GeminiLiveSession, which only runs while a session is open. Voice
// triggers run flows without an active session too, and we need the Run
// button state to clear regardless of how the replay was started.
interface PanelMultiflowProgressEvent {
  flowId: string;
  kind:
    | { kind: "started" }
    | { kind: "inputReplayed" }
    | { kind: "appLaunched" }
    | { kind: "waited" }
    | { kind: "paused"; reason: string }
    | { kind: "completed" }
    | { kind: "failed"; message: string };
}

await safeListen<PanelMultiflowProgressEvent>("multiflow_progress", async (event) => {
  const kind = event.payload.kind.kind;
  if (kind === "completed" || kind === "failed" || kind === "paused") {
    // We don't carry the flow name through the progress event; just
    // empty the running set on any terminal event. Multiple concurrent
    // replays aren't supported anyway (token supersession), so this is
    // safe.
    if (flowNamesCurrentlyReplaying.size > 0) {
      flowNamesCurrentlyReplaying.clear();
      await refreshSavedFlowsList();
    }
  }
});

// Live "Tasks: N in progress" pill under the saved-flows list. Polls
// every 8s — cheap because the count is a tiny read off the in-memory
// task store. Clicking opens the Tasks tab.
const tasksInProgressLineElement = document.getElementById(
  "tasks-in-progress-line",
) as HTMLDivElement | null;

async function refreshTasksInProgressLine(): Promise<void> {
  if (!tasksInProgressLineElement) return;
  try {
    const count = await invoke<number>("count_tasks_in_progress");
    if (count === 0) {
      tasksInProgressLineElement.hidden = true;
      return;
    }
    tasksInProgressLineElement.hidden = false;
    tasksInProgressLineElement.textContent = `Tasks: ${count} in progress →`;
  } catch (error) {
    console.warn("[panel] count_tasks_in_progress failed:", error);
  }
}

// Cost meter footer — polls every 6s while a session is open and once
// on boot. Cheap (two in-memory + one file read on the Rust side).
const costMeterLineElement = document.getElementById(
  "cost-meter-line",
) as HTMLDivElement | null;
const costMeterTextElement = document.getElementById(
  "cost-meter-text",
) as HTMLSpanElement | null;

interface CostSnapshot {
  inputTokens: number;
  outputTokens: number;
  usdCost: number;
}

function formatUsd(usd: number): string {
  // Three decimals when under a penny so a single tool call doesn't
  // render as "$0.00" and feel broken.
  if (usd < 0.01) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

async function refreshCostMeterLine(): Promise<void> {
  if (!costMeterLineElement || !costMeterTextElement) return;
  try {
    const [sessionCost, todayCost] = await Promise.all([
      invoke<CostSnapshot>("get_session_cost"),
      invoke<CostSnapshot>("get_today_cost"),
    ]);
    if (sessionCost.usdCost <= 0 && todayCost.usdCost <= 0) {
      costMeterLineElement.hidden = true;
      return;
    }
    costMeterLineElement.hidden = false;
    costMeterTextElement.textContent = `${formatUsd(
      sessionCost.usdCost,
    )} this session · ${formatUsd(todayCost.usdCost)} today`;
  } catch (costError) {
    console.warn("[panel] cost meter refresh failed:", costError);
  }
}

void refreshCostMeterLine();
setInterval(() => void refreshCostMeterLine(), 6000);

tasksInProgressLineElement?.addEventListener("click", async () => {
  try {
    await invoke("open_settings_window");
  } catch (error) {
    console.warn("[panel] open_settings_window failed:", error);
  }
});

// Sub-agent spawn handler. The Rust pool emits `subagent_spawn_request`
// when a queued sub-agent gets promoted to Running and needs an actual
// Gemini Live socket. We stub this for now: log + report a synthetic
// "started" progress event so the registry and UI both reflect the
// "running" state. End-to-end Gemini chat for sub-agents will be wired
// in a follow-up — the registry, transcripts, budget enforcement, and
// UI all work in the meantime.
interface SubagentSpawnRequest {
  subagentId: string;
  name: string;
  taskDescription: string;
  systemPrompt: string | null;
  tokenBudgetUsd: number;
}

await safeListen<SubagentSpawnRequest>("subagent_spawn_request", async (event) => {
  const request = event.payload;
  console.info(
    "[panel] subagent spawn request:",
    request.subagentId,
    request.name,
  );
  // Mark as running so the kanban badge advances; the actual
  // conversational loop is not yet wired. We append a transcript line
  // so the on-disk trace reflects the lifecycle truthfully.
  try {
    await invoke("append_subagent_transcript", {
      subagentId: request.subagentId,
      role: "system",
      text: `Spawned: ${request.name} — ${request.taskDescription}`,
    });
    await invoke("report_subagent_progress", {
      subagentId: request.subagentId,
      status: "running",
      message: "started (panel stub runner; full Gemini Live socket not yet wired)",
      spentUsdDelta: null,
    });
  } catch (reportError) {
    console.warn("[panel] subagent spawn report failed:", reportError);
  }
});

await safeListen<string>("subagent_cancel_request", (event) => {
  console.info("[panel] subagent cancel request:", event.payload);
  // No active socket to close in the stub runner.
});

// Crash-recovery banner. Rust emits `previous_session_crashed` ~1.5s
// after boot when the last run didn't shut down cleanly. Send routes
// to export_bug_report and shows the path in the OS file browser.
const crashRecoveryBanner = document.getElementById("crash-recovery-banner")!;
const crashRecoverySendButton = document.getElementById(
  "crash-recovery-send-button",
) as HTMLButtonElement;
const crashRecoveryDismissButton = document.getElementById(
  "crash-recovery-dismiss-button",
) as HTMLButtonElement;

crashRecoverySendButton?.addEventListener("click", async () => {
  try {
    const zipPath = await invoke<string>("export_bug_report");
    showError(`Bug report saved to ${zipPath}`);
  } catch (exportError) {
    showError(
      "Export failed: " +
        (exportError instanceof Error ? exportError.message : String(exportError)),
    );
  } finally {
    crashRecoveryBanner.hidden = true;
  }
});
crashRecoveryDismissButton?.addEventListener("click", () => {
  crashRecoveryBanner.hidden = true;
});

await safeListen("previous_session_crashed", () => {
  crashRecoveryBanner.hidden = false;
});

async function bootNormalPanelState(): Promise<void> {
  await loadStoredApiKey();
  await loadOperatingMode();
  await loadVoiceAndHotkeyPanelControls();
  await refreshPermissions();
  await refreshSavedFlowsList();
  await loadRecordingOptInInitialState();
  await loadAlwaysOnListenerInitialState();
  await refreshTasksInProgressLine();
  setInterval(() => void refreshTasksInProgressLine(), 8000);
  setStatus("idle");
  console.info("[panel] ready");
}

// Run the first-launch wizard in front of the normal panel when the
// user has no key + no completion flag. The wizard owns the panel
// surface until the user clicks Done; on completion we boot the normal
// panel state so the freshly-saved key + permissions show up live.
const wizardTookOver = await showOnboardingIfNeeded({
  onComplete: async () => {
    await bootNormalPanelState();
  },
});
if (!wizardTookOver) {
  await bootNormalPanelState();
}

