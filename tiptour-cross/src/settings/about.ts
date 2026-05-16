// About tab — version metadata + data-folder + reset + quit actions.

import { invoke } from "@tauri-apps/api/core";

interface AppMetadata {
  version: string;
  targetOs: string;
  targetArch: string;
  buildDate: string;
  gitCommit: string | null;
}

export async function renderAboutTab(paneElement: HTMLElement): Promise<void> {
  const metadata = await invoke<AppMetadata>("get_app_metadata");

  paneElement.innerHTML = `
    <h2>About</h2>
    <p class="lede">TipTour cross-platform build.</p>

    <section class="about-meta">
      <dl>
        <dt>Version</dt><dd>${escapeHtml(metadata.version)}</dd>
        <dt>Platform</dt><dd>${escapeHtml(metadata.targetOs)} / ${escapeHtml(
          metadata.targetArch,
        )}</dd>
        <dt>Build date</dt><dd>${escapeHtml(metadata.buildDate)}</dd>
        <dt>Commit</dt><dd>${escapeHtml(metadata.gitCommit ?? "local dev")}</dd>
      </dl>
    </section>

    <h3>Actions</h3>
    <div class="button-stack">
      <button id="about-open-data-folder">Open data folder</button>
      <button id="about-export-bug-report">Export bug report</button>
      <button id="about-show-onboarding-again">Show onboarding again</button>
      <button id="about-reset-settings" class="danger">Reset all settings</button>
      <button id="about-quit" class="danger">Quit TipTour</button>
    </div>
    <div id="about-status-banner" class="flag-banner success" hidden></div>
  `;

  const openDataFolderButton = paneElement.querySelector<HTMLButtonElement>(
    "#about-open-data-folder",
  )!;
  const resetSettingsButton = paneElement.querySelector<HTMLButtonElement>(
    "#about-reset-settings",
  )!;
  const showOnboardingAgainButton = paneElement.querySelector<HTMLButtonElement>(
    "#about-show-onboarding-again",
  )!;
  const quitButton = paneElement.querySelector<HTMLButtonElement>("#about-quit")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#about-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 2000);
  }

  const exportBugReportButton = paneElement.querySelector<HTMLButtonElement>(
    "#about-export-bug-report",
  )!;
  exportBugReportButton.addEventListener("click", async () => {
    try {
      const zipPath = await invoke<string>("export_bug_report");
      flashBanner(`Bug report saved to ${zipPath}`);
    } catch (exportError) {
      flashBanner(`Export failed: ${errorMessageOf(exportError)}`);
    }
  });

  openDataFolderButton.addEventListener("click", async () => {
    try {
      await invoke("open_data_folder_in_os_file_browser");
    } catch (openError) {
      flashBanner(`Open folder failed: ${errorMessageOf(openError)}`);
    }
  });

  resetSettingsButton.addEventListener("click", async () => {
    if (
      !window.confirm(
        "This deletes settings.json, the operating-mode setting, the recorder opt-in, and Vosk settings, and removes the Gemini API key from your keychain. Continue?",
      )
    )
      return;
    try {
      await invoke("reset_all_settings");
      await invoke("clear_api_key_from_keychain");
      flashBanner("Settings reset. Restart TipTour for a fully clean state.");
    } catch (resetError) {
      flashBanner(`Reset failed: ${errorMessageOf(resetError)}`);
    }
  });

  showOnboardingAgainButton.addEventListener("click", async () => {
    // Reset the persisted completion flag and tell the user to open
    // the panel — the next time the panel webview boots it will see
    // the unset flag and rerun the wizard. We can't reload the panel
    // webview from inside the settings webview, so we surface a
    // banner instead and let the user trigger the panel manually.
    try {
      await invoke("reset_first_run");
      flashBanner("Onboarding will reappear the next time you open the panel.");
    } catch (resetError) {
      flashBanner(`Could not reset onboarding: ${errorMessageOf(resetError)}`);
    }
  });

  quitButton.addEventListener("click", async () => {
    try {
      await invoke("quit_app_gracefully");
    } catch (quitError) {
      flashBanner(`Quit failed: ${errorMessageOf(quitError)}`);
    }
  });
}

function escapeHtml(rawString: string | null | undefined): string {
  // Treat null/undefined as empty string. Without this guard a missing
  // field (e.g. an older `get_app_metadata` impl that doesn't return
  // `gitCommit`, or a test mock that omits any AppMetadata field)
  // crashes the entire About tab with "Cannot read properties of
  // undefined (reading 'replace')".
  if (rawString === null || rawString === undefined) return "";
  return String(rawString)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
