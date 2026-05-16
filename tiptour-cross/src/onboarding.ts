// First-run onboarding wizard. Runs ONLY when:
//   1. There's no Gemini API key in the OS keychain, AND
//   2. `is_first_run` Tauri command returns true (no completion flag).
// Step 3 (permissions) is macOS-only — Windows skips straight from API
// key to "Done" since Windows doesn't gate AX/screen capture behind TCC.
//
// On completion writes the `tiptour_first_run_done.json` flag via
// `mark_first_run_complete` so subsequent launches go straight to the
// normal panel. The About-tab "Show onboarding again" button calls
// `reset_first_run` and reloads to re-trigger the wizard.

import { invoke } from "@tauri-apps/api/core";

const API_KEY_HELP_LINK = "https://aistudio.google.com/apikey";

export interface OnboardingHooks {
  /// Invoked once after the wizard finishes so the host (main.ts) can
  /// hydrate the normal panel from disk now that a key + permissions
  /// might exist. Without this, the panel state captured *before* the
  /// wizard runs is stale and the user has to flip a toggle to refresh.
  onComplete: () => void | Promise<void>;
}

const isMacPlatform = navigator.platform.toLowerCase().includes("mac");

/// Top-level entry. Returns true when the wizard took control of the
/// panel (caller should not boot normal panel logic until `onComplete`
/// fires). Returns false when the user has already onboarded — the
/// caller should proceed normally.
export async function showOnboardingIfNeeded(hooks: OnboardingHooks): Promise<boolean> {
  let hasExistingKey = false;
  try {
    const storedKey = await invoke<string | null>("get_api_key");
    hasExistingKey = !!storedKey && storedKey.length > 0;
  } catch (lookupError) {
    console.warn("[onboarding] get_api_key lookup failed:", lookupError);
  }

  let hasFirstRunFlag = false;
  try {
    const isFirstRunActive = await invoke<boolean>("is_first_run");
    hasFirstRunFlag = !isFirstRunActive;
  } catch (firstRunError) {
    console.warn("[onboarding] is_first_run lookup failed:", firstRunError);
  }

  // The wizard runs only when BOTH conditions hold (no key AND no flag).
  // A user who pasted a key in a previous launch but quit before the
  // flag was written should still skip onboarding the next time —
  // hence the OR check.
  const shouldSkipWizard = hasExistingKey || hasFirstRunFlag;
  if (shouldSkipWizard) return false;

  await renderWizard(hooks);
  return true;
}

/// Programmatic re-trigger from the About tab. Resets the flag and
/// reloads so the panel reboots through `showOnboardingIfNeeded`.
export async function resetAndReopenOnboarding(): Promise<void> {
  try {
    await invoke("reset_first_run");
  } catch (resetError) {
    console.warn("[onboarding] reset_first_run failed:", resetError);
  }
  // Reload the panel webview so main.ts re-runs through the gate.
  window.location.reload();
}

type WizardStep = "welcome" | "api-key" | "permissions";

interface WizardState {
  currentStep: WizardStep;
  hasJustGrantedScreenRecording: boolean;
}

async function renderWizard(hooks: OnboardingHooks): Promise<void> {
  let wizardElement = document.getElementById("onboarding-wizard");
  if (!wizardElement) {
    // Lazy-create when the markup wasn't pre-baked into index.html (it
    // is, but we tolerate either path so this module stays self-contained).
    wizardElement = document.createElement("div");
    wizardElement.id = "onboarding-wizard";
    wizardElement.className = "onboarding-wizard";
    document.body.appendChild(wizardElement);
  }
  wizardElement.classList.add("onboarding-wizard");
  // index.html ships the wizard div with `hidden` set so first paint
  // doesn't leak the unfinished onboarding card. CRITICAL: we have to
  // remove it here, otherwise the `.onboarding-wizard[hidden]
  // { display:none }` rule keeps the card invisible while
  // `data-onboarding-active="true"` simultaneously hides the normal
  // .panel — the user sees an empty dark rectangle with no controls
  // and no way to close.
  wizardElement.removeAttribute("hidden");
  document.body.dataset.onboardingActive = "true";

  const wizardState: WizardState = {
    currentStep: "welcome",
    hasJustGrantedScreenRecording: false,
  };

  function rerender(): void {
    if (wizardState.currentStep === "welcome") {
      paintWelcomeStep(wizardElement!, () => {
        wizardState.currentStep = "api-key";
        rerender();
      });
    } else if (wizardState.currentStep === "api-key") {
      paintApiKeyStep(
        wizardElement!,
        () => {
          if (isMacPlatform) {
            wizardState.currentStep = "permissions";
            rerender();
          } else {
            void finishWizard();
          }
        },
        () => {
          wizardState.currentStep = "welcome";
          rerender();
        },
      );
    } else {
      paintPermissionsStep(
        wizardElement!,
        wizardState,
        () => {
          void finishWizard();
        },
        () => {
          wizardState.currentStep = "api-key";
          rerender();
        },
        rerender,
      );
    }
  }

  async function finishWizard(): Promise<void> {
    try {
      await invoke("mark_first_run_complete");
    } catch (completionError) {
      console.warn(
        "[onboarding] mark_first_run_complete failed:",
        completionError,
      );
    }
    document.body.dataset.onboardingActive = "false";
    wizardElement!.innerHTML = "";
    // Re-hide so the next mount of this webview (e.g. after a
    // reload) doesn't briefly flash an empty wizard card on top of
    // the panel. data-onboardingActive=false alone wouldn't be
    // enough — the wizard's flex layout is its default state.
    wizardElement!.setAttribute("hidden", "");
    await hooks.onComplete();
  }

  rerender();
}

function paintWelcomeStep(
  wizardElement: HTMLElement,
  onContinue: () => void,
): void {
  wizardElement.innerHTML = `
    <div class="onboarding-drag-strip" data-tauri-drag-region></div>
    <div class="onboarding-step-counter">Step 1 of ${isMacPlatform ? 3 : 2}</div>
    <h2 class="onboarding-step-title">Welcome to TipTour</h2>
    <p class="onboarding-step-subtitle">
      A voice agent that lives in your menu bar, sees your screen, and runs your
      computer for you.
    </p>
    <ul class="onboarding-feature-list">
      <li><strong>Talk naturally.</strong> Press Option+X (or Alt+X) and speak.</li>
      <li><strong>It sees what you see.</strong> Streams your screen to Gemini Live.</li>
      <li><strong>Click highlight.</strong> Hold Ctrl+Shift to brush a region for context.</li>
      <li><strong>Multiflow.</strong> Record cross-app routines once, recall by voice.</li>
      <li><strong>Wake word.</strong> Optional on-device "tiptour" trigger.</li>
      <li><strong>Memory.</strong> Long-running facts the agent remembers.</li>
    </ul>
    <div class="onboarding-actions">
      <span></span>
      <button class="primary" id="onboarding-welcome-continue">Continue →</button>
    </div>
  `;
  wizardElement
    .querySelector<HTMLButtonElement>("#onboarding-welcome-continue")!
    .addEventListener("click", onContinue);
}

function paintApiKeyStep(
  wizardElement: HTMLElement,
  onSavedAndContinue: () => void,
  onBack: () => void,
): void {
  wizardElement.innerHTML = `
    <div class="onboarding-drag-strip" data-tauri-drag-region></div>
    <div class="onboarding-step-counter">Step 2 of ${isMacPlatform ? 3 : 2}</div>
    <h2 class="onboarding-step-title">Add your Gemini key</h2>
    <p class="onboarding-step-subtitle">
      Paste your Gemini API key — it stays in your OS keychain and never leaves
      your machine except to talk to Google's API.
    </p>
    <div class="onboarding-key-field">
      <input
        id="onboarding-api-key-input"
        type="password"
        placeholder="Paste your Gemini API key — starts with AIza…"
        autocomplete="off"
        spellcheck="false"
      />
      <a
        class="onboarding-link"
        href="${API_KEY_HELP_LINK}"
        target="_blank"
        rel="noreferrer noopener"
      >Get a free key →</a>
    </div>
    <div class="onboarding-actions">
      <button class="secondary" id="onboarding-key-back">← Back</button>
      <button class="primary" id="onboarding-key-save" aria-label="Save Gemini API key and continue to permissions" disabled>Save &amp; continue</button>
    </div>
  `;

  const keyInput = wizardElement.querySelector<HTMLInputElement>(
    "#onboarding-api-key-input",
  )!;
  const saveButton = wizardElement.querySelector<HTMLButtonElement>(
    "#onboarding-key-save",
  )!;
  keyInput.addEventListener("input", () => {
    saveButton.disabled = keyInput.value.trim().length === 0;
  });
  keyInput.focus();

  saveButton.addEventListener("click", async () => {
    const trimmedKey = keyInput.value.trim();
    if (!trimmedKey) return;
    saveButton.disabled = true;
    saveButton.textContent = "Saving…";
    try {
      await invoke("set_api_key", { key: trimmedKey });
      onSavedAndContinue();
    } catch (saveError) {
      saveButton.disabled = false;
      saveButton.textContent = "Save & continue";
      const errorMessage =
        saveError instanceof Error ? saveError.message : String(saveError);
      window.alert(`Could not save key: ${errorMessage}`);
    }
  });

  wizardElement
    .querySelector<HTMLButtonElement>("#onboarding-key-back")!
    .addEventListener("click", onBack);
}

function paintPermissionsStep(
  wizardElement: HTMLElement,
  wizardState: WizardState,
  onDone: () => void,
  onBack: () => void,
  rerender: () => void,
): void {
  wizardElement.innerHTML = `
    <div class="onboarding-drag-strip" data-tauri-drag-region></div>
    <div class="onboarding-step-counter">Step 3 of 3</div>
    <h2 class="onboarding-step-title">Grant permissions</h2>
    <p class="onboarding-step-subtitle">
      macOS gates Accessibility (clicks &amp; typing) and Screen Recording
      (so Gemini can see) per-app. Grant both, then come back.
    </p>
    ${
      wizardState.hasJustGrantedScreenRecording
        ? `<div class="onboarding-restart-banner">
             Screen Recording was just granted. macOS only applies it to the
             next launch — quit and relaunch TipTour after finishing onboarding.
           </div>`
        : ""
    }
    <div id="onboarding-permission-rows"></div>
    <div id="onboarding-permissions-status"></div>
    <div class="onboarding-actions">
      <button class="secondary" id="onboarding-permissions-back">← Back</button>
      <button class="primary" id="onboarding-permissions-done">Done</button>
    </div>
  `;

  const permissionRowsContainer = wizardElement.querySelector<HTMLDivElement>(
    "#onboarding-permission-rows",
  )!;
  const statusContainer = wizardElement.querySelector<HTMLDivElement>(
    "#onboarding-permissions-status",
  )!;

  async function refreshPermissionRows(): Promise<void> {
    let hasAccessibility = false;
    let hasScreenRecording = false;
    try {
      [hasAccessibility, hasScreenRecording] = await Promise.all([
        invoke<boolean>("check_accessibility_permission"),
        invoke<boolean>("check_screen_recording_permission"),
      ]);
    } catch (permissionError) {
      console.warn("[onboarding] permission check failed:", permissionError);
    }

    permissionRowsContainer.innerHTML = `
      <div class="onboarding-permission-row" data-granted="${hasAccessibility}">
        <span>Accessibility</span>
        <span>
          ${
            hasAccessibility
              ? '<span class="granted-check">Granted</span>'
              : '<button class="primary" id="onboarding-grant-accessibility">Grant</button>'
          }
        </span>
      </div>
      <div class="onboarding-permission-row" data-granted="${hasScreenRecording}">
        <span>Screen recording</span>
        <span>
          ${
            hasScreenRecording
              ? '<span class="granted-check">Granted</span>'
              : '<button class="primary" id="onboarding-grant-screen-recording">Grant</button>'
          }
        </span>
      </div>
    `;

    statusContainer.innerHTML =
      hasAccessibility && hasScreenRecording
        ? '<div class="onboarding-all-set">All set! 🎉</div>'
        : "";

    permissionRowsContainer
      .querySelector<HTMLButtonElement>("#onboarding-grant-accessibility")
      ?.addEventListener("click", async () => {
        try {
          await invoke("request_accessibility_permission");
        } finally {
          setTimeout(() => void refreshPermissionRows(), 800);
        }
      });

    permissionRowsContainer
      .querySelector<HTMLButtonElement>("#onboarding-grant-screen-recording")
      ?.addEventListener("click", async () => {
        let alreadyGrantedBeforeClick = false;
        try {
          alreadyGrantedBeforeClick = await invoke<boolean>(
            "check_screen_recording_permission",
          );
        } catch {
          // Treat as not-granted; that path triggers the restart banner
          // when the kernel-level grant flips on after the user returns.
        }
        try {
          await invoke("request_screen_recording_permission");
        } catch (grantError) {
          console.warn("[onboarding] grant screen recording threw:", grantError);
        }
        // Poll for ~30s for a not-granted → granted transition. When
        // detected, set the restart banner flag and re-render.
        let elapsedSeconds = 0;
        const pollerHandle = window.setInterval(async () => {
          elapsedSeconds += 1;
          try {
            const nowGranted = await invoke<boolean>(
              "check_screen_recording_permission",
            );
            if (nowGranted && !alreadyGrantedBeforeClick) {
              wizardState.hasJustGrantedScreenRecording = true;
              window.clearInterval(pollerHandle);
              rerender();
            }
          } catch {
            /* ignore — keep polling */
          }
          if (elapsedSeconds >= 30) {
            window.clearInterval(pollerHandle);
            void refreshPermissionRows();
          }
        }, 1000);
      });
  }

  wizardElement
    .querySelector<HTMLButtonElement>("#onboarding-permissions-back")!
    .addEventListener("click", onBack);
  wizardElement
    .querySelector<HTMLButtonElement>("#onboarding-permissions-done")!
    .addEventListener("click", onDone);

  void refreshPermissionRows();
}
