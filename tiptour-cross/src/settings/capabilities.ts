// Capabilities tab — per-app capability indexes plus the deny-list of
// destructive keywords the explorer never executes.

import { invoke } from "@tauri-apps/api/core";

interface CapabilityIndexSummary {
  appIdentifier: string;
  appVersion: string | null;
  capabilityCount: number;
  lastExploredUnixMs: number | null;
}

export async function renderCapabilitiesTab(paneElement: HTMLElement): Promise<void> {
  const destructiveKeywords = await invoke<string[]>("get_destructive_keywords").catch(
    () => [] as string[],
  );

  paneElement.innerHTML = `
    <h2>Capabilities</h2>
    <p class="lede">
      Capabilities are the action surface TipTour has mapped for each app it
      knows — the buttons and menus on screen, the AppleScript / UI Automation
      commands available, and the keyboard shortcuts it can drive. The
      registry is built by exploring apps the first time you use them and
      refreshed when the app updates. Re-explore from here if a recent app
      update broke a flow you'd recorded.
    </p>

    <div class="flag-banner">
      <strong>Deny-listed during exploration</strong>: actions whose element name or menu
      path contains any of these keywords are recorded but never auto-executed.
      <div class="error-details-code">
        ${destructiveKeywords.map((keyword) => escapeHtml(keyword)).join(" · ")}
      </div>
    </div>

    <h3>Indexed apps</h3>
    <p class="section-helper">
      One row per app TipTour has walked. The capability count is how many
      distinct buttons, menus, and shortcuts the registry knows for that app.
      Click Re-explore on any row after an app update so the registry stays
      accurate.
    </p>
    <ul id="capabilities-list" class="list-rows"></ul>
    <div id="capabilities-status-banner" class="flag-banner success" hidden></div>
  `;

  const capabilitiesListElement = paneElement.querySelector<HTMLUListElement>(
    "#capabilities-list",
  )!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#capabilities-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  async function refreshCapabilitiesList(): Promise<void> {
    let indexSummaries: CapabilityIndexSummary[] = [];
    try {
      // Coerce null to empty list so a no-op handler doesn't crash
      // the .map() below.
      indexSummaries =
        (await invoke<CapabilityIndexSummary[] | null>(
          "list_capability_index_summaries",
        )) ?? [];
    } catch (listError) {
      flashBanner(`List failed: ${errorMessageOf(listError)}`);
      return;
    }
    capabilitiesListElement.innerHTML = "";
    if (indexSummaries.length === 0) {
      const emptyRow = document.createElement("li");
      emptyRow.className = "list-row disabled empty-state";
      emptyRow.textContent = "No apps explored yet. Open an app and click 'Explore' to map its commands.";
      capabilitiesListElement.appendChild(emptyRow);
      return;
    }
    for (const indexSummary of indexSummaries) {
      const rowElement = document.createElement("li");
      rowElement.className = "list-row";
      const lastExploredLabel = indexSummary.lastExploredUnixMs
        ? new Date(indexSummary.lastExploredUnixMs).toLocaleString()
        : "never";
      rowElement.innerHTML = `
        <div class="row-main">
          <span class="row-title">${escapeHtml(indexSummary.appIdentifier)}</span>
          <span class="row-sub">
            ${indexSummary.capabilityCount} capabilities ·
            version ${escapeHtml(indexSummary.appVersion ?? "unknown")} ·
            last explored ${escapeHtml(lastExploredLabel)}
          </span>
        </div>
        <div class="row-actions">
          <button data-action="re-explore">Re-explore</button>
          <button data-action="clear" class="danger">Clear cache</button>
        </div>
      `;
      rowElement
        .querySelector<HTMLButtonElement>("button[data-action='re-explore']")!
        .addEventListener("click", async () => {
          flashBanner(`Exploring ${indexSummary.appIdentifier}… this can take a while.`);
          try {
            await invoke("explore_app", { appIdentifier: indexSummary.appIdentifier });
            flashBanner("Re-exploration finished.");
            await refreshCapabilitiesList();
          } catch (exploreError) {
            flashBanner(`Explore failed: ${errorMessageOf(exploreError)}`);
          }
        });
      rowElement
        .querySelector<HTMLButtonElement>("button[data-action='clear']")!
        .addEventListener("click", async () => {
          if (
            !window.confirm(
              `Clear capability cache for "${indexSummary.appIdentifier}"? You'll need to re-explore before the registry has tools for it again.`,
            )
          )
            return;
          try {
            await invoke("clear_capability_cache", {
              appIdentifier: indexSummary.appIdentifier,
            });
            flashBanner("Cleared.");
            await refreshCapabilitiesList();
          } catch (clearError) {
            flashBanner(`Clear failed: ${errorMessageOf(clearError)}`);
          }
        });
      capabilitiesListElement.appendChild(rowElement);
    }
  }

  await refreshCapabilitiesList();
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
