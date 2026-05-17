// Permissions tab — macOS-only. Surfaces Accessibility, Screen
// Recording, and Microphone status with grant buttons and help text.

import { invoke } from "@tauri-apps/api/core";

const isMacOsPlatform = navigator.platform.toLowerCase().includes("mac");

interface PermissionRowDefinition {
  domId: string;
  label: string;
  helpText: string;
  checkCommandName: string;
  requestCommandName: string;
  /// Some macOS permissions only take effect after a relaunch — surface
  /// that inline so the user isn't left puzzled.
  requiresRelaunchOnGrant: boolean;
}

const PERMISSION_ROWS: PermissionRowDefinition[] = [
  {
    domId: "permission-accessibility",
    label: "Accessibility",
    helpText:
      "Lets TipTour read the AX tree (to find buttons) and synthesize clicks + keystrokes.",
    checkCommandName: "check_accessibility_permission",
    requestCommandName: "request_accessibility_permission",
    requiresRelaunchOnGrant: false,
  },
  {
    domId: "permission-screen",
    label: "Screen Recording",
    helpText:
      "So Gemini can see your screen. macOS caches this per-process — you must relaunch TipTour after granting.",
    checkCommandName: "check_screen_recording_permission",
    requestCommandName: "request_screen_recording_permission",
    requiresRelaunchOnGrant: true,
  },
];

export async function renderPermissionsTab(paneElement: HTMLElement): Promise<void> {
  if (!isMacOsPlatform) {
    paneElement.innerHTML = `
      <h2>Permissions</h2>
      <p class="lede">Permissions are macOS-specific. Windows and Linux don't gate input
      synthesis, UIA reads, or capture for user-mode apps.</p>
    `;
    return;
  }

  paneElement.innerHTML = `
    <h2>Permissions</h2>
    <p class="lede">macOS gates these by default. Grant what TipTour needs to work.</p>
    <ul id="permissions-list" class="list-rows"></ul>
    <div class="list-row">
      <div class="row-main">
        <span class="row-title"><span class="permission-indicator" data-granted="false"></span>Microphone</span>
        <span class="row-sub">
          So TipTour can hear you. Grant via System Settings → Privacy & Security → Microphone.
        </span>
      </div>
    </div>
    <div id="permissions-status-banner" class="flag-banner success" hidden></div>
  `;

  const permissionsListElement = paneElement.querySelector<HTMLUListElement>(
    "#permissions-list",
  )!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#permissions-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 2500);
  }

  for (const permissionRowDefinition of PERMISSION_ROWS) {
    const isGranted = await invoke<boolean>(permissionRowDefinition.checkCommandName).catch(
      () => false,
    );
    const rowElement = document.createElement("li");
    rowElement.className = "list-row";
    rowElement.id = permissionRowDefinition.domId;
    rowElement.innerHTML = `
      <div class="row-main">
        <span class="row-title"><span class="permission-indicator" data-granted="${isGranted}"></span>${escapeHtml(
          permissionRowDefinition.label,
        )}</span>
        <span class="row-sub">${escapeHtml(permissionRowDefinition.helpText)}</span>
      </div>
      <div class="row-actions">
        <button data-action="grant" ${isGranted ? "disabled" : ""}>${
          isGranted ? "Granted" : "Grant"
        }</button>
      </div>
    `;
    rowElement
      .querySelector<HTMLButtonElement>("button[data-action='grant']")!
      .addEventListener("click", async () => {
        try {
          await invoke(permissionRowDefinition.requestCommandName);
          if (permissionRowDefinition.requiresRelaunchOnGrant) {
            flashBanner(
              "Permission requested. Quit and reopen TipTour for the change to take effect.",
            );
          } else {
            flashBanner("Permission requested.");
          }
        } catch (requestError) {
          flashBanner(`Request failed: ${errorMessageOf(requestError)}`);
        }
      });
    permissionsListElement.appendChild(rowElement);
  }
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
