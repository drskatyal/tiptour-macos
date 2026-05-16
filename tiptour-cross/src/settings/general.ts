// General tab — Gemini API key, voice, model, push-to-talk hotkey,
// operating mode, always-on listening.

import { invoke } from "@tauri-apps/api/core";

interface AppSettingsShape {
  schemaVersion: number;
  geminiVoice: string;
  geminiModel: string;
  pushToTalkChord: string;
  theme: string;
}

const GEMINI_VOICE_CHOICES = ["Kore", "Aoede", "Charon", "Puck", "Fenrir"];
const GEMINI_MODEL_CHOICES = [
  { value: "gemini-3.1-flash-live-preview", label: "gemini-3.1-flash-live-preview (default)" },
  { value: "gemini-2.0-flash-exp", label: "gemini-2.0-flash-exp (legacy)" },
];

export async function renderGeneralTab(paneElement: HTMLElement): Promise<void> {
  const storedApiKey = await invoke<string | null>("get_api_key").catch(() => null);
  const currentAppSettings = await invoke<AppSettingsShape>("get_app_settings");
  const currentOperatingMode = await invoke<string>("get_operating_mode").catch(
    () => "autopilot",
  );
  const currentRecordingEnabled = await invoke<boolean>("is_recording_enabled").catch(
    () => false,
  );
  const currentListenerEnabled = await invoke<boolean>("is_listener_enabled").catch(
    () => false,
  );

  paneElement.innerHTML = `
    <h2>General</h2>
    <p class="lede">Core voice + automation behavior. Changes save immediately.</p>

    <div class="settings-row">
      <label for="general-api-key">Gemini API key</label>
      <div>
        <input id="general-api-key" type="password" autocomplete="off" spellcheck="false" />
        <button id="general-api-key-save" class="primary">Save</button>
        <span class="row-hint">Stored in your OS keychain. Never logged.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="general-voice">Voice</label>
      <div>
        <select id="general-voice">
          ${GEMINI_VOICE_CHOICES.map(
            (voiceName) =>
              `<option value="${voiceName}">${voiceName}</option>`,
          ).join("")}
        </select>
        <span class="row-hint">Applied on the next session open.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="general-model">Model</label>
      <div>
        <select id="general-model">
          ${GEMINI_MODEL_CHOICES.map(
            (modelChoice) =>
              `<option value="${modelChoice.value}">${modelChoice.label}</option>`,
          ).join("")}
        </select>
        <span class="row-hint">2.0-flash-exp is retired upstream — use only for debugging.</span>
      </div>
    </div>

    <div class="settings-row">
      <label>Push-to-talk hotkey</label>
      <div>
        <button class="hotkey-chip" id="general-hotkey-chip">${escapeHtml(
          currentAppSettings.pushToTalkChord,
        )}</button>
        <span class="row-hint">Click and press a new chord. Default Alt+X.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="general-theme">Theme</label>
      <div>
        <select id="general-theme">
          <option value="auto">Auto (match system)</option>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
        </select>
        <span class="row-hint">Applied immediately across all windows.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="general-mode">Operating mode</label>
      <div>
        <select id="general-mode">
          <option value="autopilot">Autopilot — TipTour clicks for me</option>
          <option value="teaching">Teaching — point, I click</option>
        </select>
      </div>
    </div>

    <div class="settings-row">
      <label>Always-on listening</label>
      <div>
        <label><input type="checkbox" id="general-listener-toggle" /> Enable "tiptour" wake word</label>
        <span class="row-hint">Audio never leaves your machine in this mode.</span>
      </div>
    </div>

    <div class="settings-row">
      <label>Input recording</label>
      <div>
        <label><input type="checkbox" id="general-recording-toggle" /> Required to save flows.</label>
      </div>
    </div>

    <div id="general-status-banner" class="flag-banner success" hidden></div>
  `;

  const apiKeyInputElement = paneElement.querySelector<HTMLInputElement>("#general-api-key")!;
  const apiKeySaveButton = paneElement.querySelector<HTMLButtonElement>("#general-api-key-save")!;
  const voiceSelectElement = paneElement.querySelector<HTMLSelectElement>("#general-voice")!;
  const modelSelectElement = paneElement.querySelector<HTMLSelectElement>("#general-model")!;
  const hotkeyChipButton = paneElement.querySelector<HTMLButtonElement>("#general-hotkey-chip")!;
  const modeSelectElement = paneElement.querySelector<HTMLSelectElement>("#general-mode")!;
  const themeSelectElement = paneElement.querySelector<HTMLSelectElement>("#general-theme")!;
  const listenerToggleElement = paneElement.querySelector<HTMLInputElement>(
    "#general-listener-toggle",
  )!;
  const recordingToggleElement = paneElement.querySelector<HTMLInputElement>(
    "#general-recording-toggle",
  )!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>("#general-status-banner")!;

  if (storedApiKey) apiKeyInputElement.value = storedApiKey;
  voiceSelectElement.value = GEMINI_VOICE_CHOICES.includes(currentAppSettings.geminiVoice)
    ? currentAppSettings.geminiVoice
    : "Kore";
  modelSelectElement.value =
    GEMINI_MODEL_CHOICES.find((choice) => choice.value === currentAppSettings.geminiModel)?.value ??
    GEMINI_MODEL_CHOICES[0].value;
  modeSelectElement.value = currentOperatingMode;
  themeSelectElement.value = ["auto", "dark", "light"].includes(currentAppSettings.theme)
    ? currentAppSettings.theme
    : "auto";
  listenerToggleElement.checked = currentListenerEnabled;
  recordingToggleElement.checked = currentRecordingEnabled;

  function flashSavedBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1500);
  }

  apiKeySaveButton.addEventListener("click", async () => {
    try {
      await invoke("set_api_key", { key: apiKeyInputElement.value.trim() });
      flashSavedBanner("API key saved to keychain.");
    } catch (saveError) {
      flashSavedBanner(`Save failed: ${errorMessageOf(saveError)}`);
    }
  });

  async function persistAppSettingsFromUi(): Promise<void> {
    const updatedSettings: AppSettingsShape = {
      schemaVersion: currentAppSettings.schemaVersion || 1,
      geminiVoice: voiceSelectElement.value,
      geminiModel: modelSelectElement.value,
      pushToTalkChord: hotkeyChipButton.textContent?.trim() || "Alt+X",
      theme: themeSelectElement.value,
    };
    try {
      await invoke("set_app_settings_with_broadcast", { settings: updatedSettings });
      flashSavedBanner("Saved.");
    } catch (settingsError) {
      flashSavedBanner(`Save failed: ${errorMessageOf(settingsError)}`);
    }
  }

  voiceSelectElement.addEventListener("change", () => void persistAppSettingsFromUi());
  modelSelectElement.addEventListener("change", () => void persistAppSettingsFromUi());
  themeSelectElement.addEventListener("change", () => void persistAppSettingsFromUi());

  modeSelectElement.addEventListener("change", async () => {
    try {
      await invoke("set_operating_mode", { mode: modeSelectElement.value });
      flashSavedBanner("Mode updated.");
    } catch (modeError) {
      flashSavedBanner(`Mode failed: ${errorMessageOf(modeError)}`);
    }
  });

  listenerToggleElement.addEventListener("change", async () => {
    try {
      if (listenerToggleElement.checked) {
        await invoke("download_vosk_model_if_needed");
      }
      await invoke("set_listener_enabled", { enabled: listenerToggleElement.checked });
      flashSavedBanner("Listener updated.");
    } catch (listenerError) {
      listenerToggleElement.checked = !listenerToggleElement.checked;
      flashSavedBanner(`Listener failed: ${errorMessageOf(listenerError)}`);
    }
  });

  recordingToggleElement.addEventListener("change", async () => {
    try {
      await invoke("set_recording_enabled", { enabled: recordingToggleElement.checked });
      flashSavedBanner("Recording updated.");
    } catch (recordingError) {
      recordingToggleElement.checked = !recordingToggleElement.checked;
      flashSavedBanner(`Recording failed: ${errorMessageOf(recordingError)}`);
    }
  });

  hotkeyChipButton.addEventListener("click", () => {
    captureHotkeyChord(hotkeyChipButton, async (newChordString: string) => {
      try {
        await invoke("reregister_push_to_talk_hotkey", { chordString: newChordString });
        flashSavedBanner(`Hotkey set to ${newChordString}.`);
      } catch (hotkeyError) {
        flashSavedBanner(`Hotkey failed: ${errorMessageOf(hotkeyError)}`);
      }
    });
  });
}

function captureHotkeyChord(
  chipButtonElement: HTMLButtonElement,
  onCapturedChord: (chordString: string) => void,
): void {
  const previousChordLabel = chipButtonElement.textContent;
  chipButtonElement.dataset.capturing = "true";
  chipButtonElement.textContent = "Press a new chord…";

  function keyDownListener(keyboardEvent: KeyboardEvent) {
    keyboardEvent.preventDefault();
    keyboardEvent.stopPropagation();

    // Ignore standalone modifier presses — wait for a real key.
    if (
      keyboardEvent.key === "Control" ||
      keyboardEvent.key === "Alt" ||
      keyboardEvent.key === "Shift" ||
      keyboardEvent.key === "Meta"
    ) {
      return;
    }

    const chordTokens: string[] = [];
    if (keyboardEvent.ctrlKey) chordTokens.push("Ctrl");
    if (keyboardEvent.altKey) chordTokens.push("Alt");
    if (keyboardEvent.shiftKey) chordTokens.push("Shift");
    if (keyboardEvent.metaKey) chordTokens.push("Cmd");
    const keyCharacter =
      keyboardEvent.key.length === 1
        ? keyboardEvent.key.toUpperCase()
        : keyboardEvent.key;
    chordTokens.push(keyCharacter);
    const chordString = chordTokens.join("+");

    cleanup();
    chipButtonElement.textContent = chordString;
    onCapturedChord(chordString);
  }

  function blurListener() {
    cleanup();
    chipButtonElement.textContent = previousChordLabel;
  }

  function cleanup() {
    window.removeEventListener("keydown", keyDownListener, true);
    chipButtonElement.removeEventListener("blur", blurListener);
    chipButtonElement.dataset.capturing = "false";
  }

  window.addEventListener("keydown", keyDownListener, true);
  chipButtonElement.addEventListener("blur", blurListener);
  chipButtonElement.focus();
}

function escapeHtml(rawString: string): string {
  return rawString
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
