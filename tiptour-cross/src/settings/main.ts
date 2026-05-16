// Entry point for the Settings webview. Owns left-rail nav state and
// delegates to per-tab modules to render the active pane.

import { renderGeneralTab } from "./general";
import { renderCommandsTab } from "./commands";
import { renderFlowsTab } from "./flows";
import { renderRecordingsTab } from "./recordings";
import { renderCapabilitiesTab } from "./capabilities";
import { renderPermissionsTab } from "./permissions";
import { renderAboutTab } from "./about";
import { renderIndicatorsTab } from "./indicators";
import { renderMemoryTab } from "./memory";
import { renderTasksTab } from "./tasks";
import { renderPersonasTab } from "./personas";
import { installThemeBridge } from "../theme";

void installThemeBridge();

type TabName =
  | "general"
  | "commands"
  | "flows"
  | "recordings"
  | "capabilities"
  | "indicators"
  | "memory"
  | "tasks"
  | "personas"
  | "permissions"
  | "about";

const tabRenderers: Record<TabName, (paneElement: HTMLElement) => Promise<void>> = {
  general: renderGeneralTab,
  commands: renderCommandsTab,
  flows: renderFlowsTab,
  recordings: renderRecordingsTab,
  capabilities: renderCapabilitiesTab,
  indicators: renderIndicatorsTab,
  memory: renderMemoryTab,
  tasks: renderTasksTab,
  personas: renderPersonasTab,
  permissions: renderPermissionsTab,
  about: renderAboutTab,
};

const isMacOsPlatform = navigator.platform.toLowerCase().includes("mac");

const settingsNavElement = document.getElementById("settings-nav")!;
const settingsPaneElement = document.getElementById("settings-pane")!;
const permissionsTabItemElement = document.getElementById("permissions-tab-item")!;

// Show the Permissions tab only on macOS — Windows/Linux don't gate
// input synthesis, UIA reads, or desktop capture for user-mode apps.
if (isMacOsPlatform) {
  permissionsTabItemElement.hidden = false;
}

const tabButtonElements = Array.from(
  settingsNavElement.querySelectorAll<HTMLButtonElement>(".settings-tab"),
);

async function switchToTab(targetTabName: TabName) {
  for (const tabButtonElement of tabButtonElements) {
    const buttonTabName = tabButtonElement.dataset.tab as TabName | undefined;
    tabButtonElement.dataset.active = buttonTabName === targetTabName ? "true" : "false";
  }
  // Skeleton placeholder: a header-shaped block + a lede line + three
  // row-shaped shimmers. The structure matches the most common tab
  // layout (h2 + lede + list/rows) so the visual identity of the pane
  // doesn't change when content arrives.
  settingsPaneElement.innerHTML = `
    <div class="settings-loading" aria-hidden="true">
      <div class="settings-loading-row"></div>
      <div class="settings-loading-row"></div>
      <div class="settings-loading-row"></div>
    </div>
  `;
  try {
    await tabRenderers[targetTabName](settingsPaneElement);
  } catch (renderError) {
    const errorMessage =
      renderError instanceof Error ? renderError.message : String(renderError);
    settingsPaneElement.innerHTML = `<div class="flag-banner">Failed to render ${targetTabName}: ${errorMessage}</div>`;
  }
}

for (const tabButtonElement of tabButtonElements) {
  tabButtonElement.addEventListener("click", () => {
    const tabName = tabButtonElement.dataset.tab as TabName | undefined;
    if (tabName) void switchToTab(tabName);
  });
}

void switchToTab("general");
