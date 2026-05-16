// Indicators tab — controls the side-of-screen pill strip: which edge
// of the primary monitor it lives on (or Disabled to hide it), density,
// auto-dismiss timeout, max visible pill count, optional sound, and
// the six per-kind enable flags.
//
// Every change saves immediately. Changing position triggers a Rust
// reposition so the user gets visual feedback without clicking a
// "save and apply" button.

import { invoke } from "@tauri-apps/api/core";

type IndicatorPositionWire = "right-edge" | "left-edge" | "disabled";
type IndicatorDensityWire = "compact" | "normal" | "verbose";
type IndicatorAutoDismissWire =
  | "three-seconds"
  | "five-seconds"
  | "ten-seconds"
  | "forever";
type IndicatorMaxVisibleWire = "four" | "six" | "ten";
type IndicatorSoundWire = "silent" | "soft-tick";

interface IndicatorTypeTogglesShape {
  workflowStep: boolean;
  flowDone: boolean;
  voiceCommand: boolean;
  appLaunched: boolean;
  screenshot: boolean;
  error: boolean;
}

interface IndicatorSettingsShape {
  schemaVersion: number;
  position: IndicatorPositionWire;
  density: IndicatorDensityWire;
  autoDismiss: IndicatorAutoDismissWire;
  maxVisible: IndicatorMaxVisibleWire;
  sound: IndicatorSoundWire;
  typesEnabled: IndicatorTypeTogglesShape;
}

const POSITION_CHOICES: { value: IndicatorPositionWire; label: string }[] = [
  { value: "right-edge", label: "Right edge of primary display (default)" },
  { value: "left-edge", label: "Left edge of primary display" },
  { value: "disabled", label: "Disabled — hide indicator strip" },
];

const DENSITY_CHOICES: { value: IndicatorDensityWire; label: string }[] = [
  { value: "compact", label: "Compact — icons only, no hover expand" },
  { value: "normal", label: "Normal — collapse on idle, expand on hover" },
  { value: "verbose", label: "Verbose — always expanded" },
];

const AUTO_DISMISS_CHOICES: { value: IndicatorAutoDismissWire; label: string }[] = [
  { value: "three-seconds", label: "3 seconds" },
  { value: "five-seconds", label: "5 seconds (default)" },
  { value: "ten-seconds", label: "10 seconds" },
  { value: "forever", label: "Never — only dismiss on click" },
];

const MAX_VISIBLE_CHOICES: { value: IndicatorMaxVisibleWire; label: string }[] = [
  { value: "four", label: "4 pills" },
  { value: "six", label: "6 pills (default)" },
  { value: "ten", label: "10 pills" },
];

const SOUND_CHOICES: { value: IndicatorSoundWire; label: string }[] = [
  { value: "silent", label: "Silent (default)" },
  { value: "soft-tick", label: "Soft tick" },
];

const TYPE_TOGGLE_DEFINITIONS: {
  flagKey: keyof IndicatorTypeTogglesShape;
  label: string;
  description: string;
}[] = [
  {
    flagKey: "workflowStep",
    label: "Workflow steps",
    description: "One pill per executed step in a Gemini workflow plan.",
  },
  {
    flagKey: "flowDone",
    label: "Saved-flow replays",
    description: "Pill when a recalled saved flow finishes replaying.",
  },
  {
    flagKey: "voiceCommand",
    label: "Voice commands",
    description: "Pill on every recognized wake-word command match.",
  },
  {
    flagKey: "appLaunched",
    label: "App launches",
    description: "Pill when TipTour opens an app for you.",
  },
  {
    flagKey: "screenshot",
    label: "Screenshots",
    description: "Pill when a demonstration screenshot is captured.",
  },
  {
    flagKey: "error",
    label: "Errors",
    description: "Pill when TipTour surfaces an error.",
  },
];

function escapeHtml(unsafeText: string): string {
  return unsafeText
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// Defaults for every IndicatorTypeToggles field. Used to backfill any
// settings JSON written by an older build (or by a test mock) that
// omits the `typesEnabled` block — the renderer dereferences each
// flag and would crash on `undefined.workflowStep` otherwise.
const DEFAULT_TYPE_TOGGLES: IndicatorTypeTogglesShape = {
  workflowStep: true,
  flowDone: true,
  voiceCommand: true,
  appLaunched: true,
  screenshot: true,
  error: true,
};

export async function renderIndicatorsTab(paneElement: HTMLElement): Promise<void> {
  const loadedSettings = await invoke<Partial<IndicatorSettingsShape>>(
    "get_indicators_settings",
  );
  // Merge against safe defaults so any missing field (e.g. typesEnabled
  // on a stale settings file) doesn't throw downstream during render.
  const currentSettings: IndicatorSettingsShape = {
    schemaVersion: loadedSettings.schemaVersion ?? 1,
    position: loadedSettings.position ?? "right-edge",
    density: loadedSettings.density ?? "normal",
    autoDismiss: loadedSettings.autoDismiss ?? "five-seconds",
    maxVisible: loadedSettings.maxVisible ?? "four",
    sound: loadedSettings.sound ?? "silent",
    typesEnabled: {
      ...DEFAULT_TYPE_TOGGLES,
      ...(loadedSettings.typesEnabled ?? {}),
    },
  };

  paneElement.innerHTML = `
    <h2>Indicators</h2>
    <p class="lede">
      A slim strip of dismissable pills sits at the edge of your primary
      display and surfaces noteworthy events as they happen — a workflow
      step finished, a screenshot was captured, an app was launched.
      Hover any pill to expand it; click to dismiss. Use the controls
      below to choose the edge, density, auto-dismiss timeout, and which
      event types deserve a pill. Every change saves immediately.
    </p>

    <div class="settings-row">
      <label for="indicators-position">Position</label>
      <div>
        <select id="indicators-position">
          ${POSITION_CHOICES.map(
            (choice) =>
              `<option value="${choice.value}"${
                choice.value === currentSettings.position ? " selected" : ""
              }>${escapeHtml(choice.label)}</option>`,
          ).join("")}
        </select>
        <span class="row-hint">Disabling hides the indicator window entirely.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="indicators-density">Density</label>
      <div>
        <select id="indicators-density">
          ${DENSITY_CHOICES.map(
            (choice) =>
              `<option value="${choice.value}"${
                choice.value === currentSettings.density ? " selected" : ""
              }>${escapeHtml(choice.label)}</option>`,
          ).join("")}
        </select>
      </div>
    </div>

    <div class="settings-row">
      <label for="indicators-auto-dismiss">Auto-dismiss</label>
      <div>
        <select id="indicators-auto-dismiss">
          ${AUTO_DISMISS_CHOICES.map(
            (choice) =>
              `<option value="${choice.value}"${
                choice.value === currentSettings.autoDismiss ? " selected" : ""
              }>${escapeHtml(choice.label)}</option>`,
          ).join("")}
        </select>
      </div>
    </div>

    <div class="settings-row">
      <label for="indicators-max-visible">Max visible</label>
      <div>
        <select id="indicators-max-visible">
          ${MAX_VISIBLE_CHOICES.map(
            (choice) =>
              `<option value="${choice.value}"${
                choice.value === currentSettings.maxVisible ? " selected" : ""
              }>${escapeHtml(choice.label)}</option>`,
          ).join("")}
        </select>
        <span class="row-hint">Oldest pill drops when over the cap.</span>
      </div>
    </div>

    <div class="settings-row">
      <label for="indicators-sound">Sound</label>
      <div>
        <select id="indicators-sound">
          ${SOUND_CHOICES.map(
            (choice) =>
              `<option value="${choice.value}"${
                choice.value === currentSettings.sound ? " selected" : ""
              }>${escapeHtml(choice.label)}</option>`,
          ).join("")}
        </select>
      </div>
    </div>

    <h3 class="settings-subhead">Per-type toggles</h3>
    <div class="indicators-type-toggle-list">
      ${TYPE_TOGGLE_DEFINITIONS.map(
        (toggleDefinition) => `
        <label class="indicators-type-toggle-row">
          <input
            type="checkbox"
            data-indicator-type-flag="${toggleDefinition.flagKey}"
            ${currentSettings.typesEnabled[toggleDefinition.flagKey] ? "checked" : ""}
          />
          <span class="indicators-type-toggle-text">
            <span class="indicators-type-toggle-label">${escapeHtml(toggleDefinition.label)}</span>
            <span class="indicators-type-toggle-description">${escapeHtml(toggleDefinition.description)}</span>
          </span>
        </label>
      `,
      ).join("")}
    </div>
  `;

  // Wire change handlers — every control reads its value from the DOM,
  // splices it into the in-memory settings object, and persists. We
  // don't merge against the server-side default here because the Rust
  // serde defaults will fill any future-added field.
  const liveSettings: IndicatorSettingsShape = JSON.parse(JSON.stringify(currentSettings));

  async function persistAndApply(): Promise<void> {
    try {
      await invoke("set_indicators_settings", { settings: liveSettings });
    } catch (persistError) {
      console.error("indicators: failed to save settings", persistError);
    }
  }

  const positionSelectElement = paneElement.querySelector<HTMLSelectElement>(
    "#indicators-position",
  )!;
  positionSelectElement.addEventListener("change", () => {
    liveSettings.position = positionSelectElement.value as IndicatorPositionWire;
    void persistAndApply();
  });

  const densitySelectElement = paneElement.querySelector<HTMLSelectElement>(
    "#indicators-density",
  )!;
  densitySelectElement.addEventListener("change", () => {
    liveSettings.density = densitySelectElement.value as IndicatorDensityWire;
    void persistAndApply();
  });

  const autoDismissSelectElement = paneElement.querySelector<HTMLSelectElement>(
    "#indicators-auto-dismiss",
  )!;
  autoDismissSelectElement.addEventListener("change", () => {
    liveSettings.autoDismiss = autoDismissSelectElement.value as IndicatorAutoDismissWire;
    void persistAndApply();
  });

  const maxVisibleSelectElement = paneElement.querySelector<HTMLSelectElement>(
    "#indicators-max-visible",
  )!;
  maxVisibleSelectElement.addEventListener("change", () => {
    liveSettings.maxVisible = maxVisibleSelectElement.value as IndicatorMaxVisibleWire;
    void persistAndApply();
  });

  const soundSelectElement = paneElement.querySelector<HTMLSelectElement>(
    "#indicators-sound",
  )!;
  soundSelectElement.addEventListener("change", () => {
    liveSettings.sound = soundSelectElement.value as IndicatorSoundWire;
    void persistAndApply();
  });

  const typeToggleInputElements = Array.from(
    paneElement.querySelectorAll<HTMLInputElement>("input[data-indicator-type-flag]"),
  );
  for (const toggleInputElement of typeToggleInputElements) {
    toggleInputElement.addEventListener("change", () => {
      const flagKey = toggleInputElement.dataset.indicatorTypeFlag as
        | keyof IndicatorTypeTogglesShape
        | undefined;
      if (!flagKey) return;
      liveSettings.typesEnabled[flagKey] = toggleInputElement.checked;
      void persistAndApply();
    });
  }
}
