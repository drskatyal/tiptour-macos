// Recordings tab — opt-in demonstrations only. Never exposes passive
// traces. Click a row to expand into a screenshot grid.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";

interface DemonstrationSummary {
  id: string;
  title: string;
  createdAtUnixMs: number;
  traceEntryCount: number;
  hasNarrationAudio: boolean;
}

export async function renderRecordingsTab(paneElement: HTMLElement): Promise<void> {
  paneElement.innerHTML = `
    <h2>Recordings</h2>
    <p class="lede">
      When you opt in to input recording on the General tab, TipTour stores
      keystrokes, clicks, and the timing of demonstrations you save as
      flows. Only your explicit recordings show up here — passive day-to-day
      activity is never written to disk. Recordings stay on your machine,
      are named by you, and can be deleted any time below.
    </p>
    <ul id="recordings-list" class="list-rows"></ul>
    <div id="recordings-status-banner" class="flag-banner success" hidden></div>
  `;

  const recordingsListElement = paneElement.querySelector<HTMLUListElement>("#recordings-list")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#recordings-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  async function refreshRecordingsList(): Promise<void> {
    let demonstrations: DemonstrationSummary[] = [];
    try {
      // Coerce null/undefined to empty list so a no-op handler in
      // dev/mock environments doesn't trip the .length lookup below.
      demonstrations =
        (await invoke<DemonstrationSummary[] | null>("list_demonstrations")) ?? [];
    } catch (listError) {
      flashBanner(`List failed: ${errorMessageOf(listError)}`);
      return;
    }
    recordingsListElement.innerHTML = "";
    if (demonstrations.length === 0) {
      const emptyRow = document.createElement("li");
      emptyRow.className = "list-row disabled empty-state";
      emptyRow.textContent = "No recordings yet. Record a flow from the panel to see it here.";
      recordingsListElement.appendChild(emptyRow);
      return;
    }
    for (const demonstration of demonstrations) {
      recordingsListElement.appendChild(
        renderDemonstrationRow(demonstration, refreshRecordingsList, flashBanner),
      );
    }
  }

  await refreshRecordingsList();
}

function renderDemonstrationRow(
  demonstration: DemonstrationSummary,
  refreshRecordingsList: () => Promise<void>,
  flashBanner: (message: string) => void,
): HTMLElement {
  const rowElement = document.createElement("li");
  rowElement.className = "list-row recording-row";
  const createdLabel = new Date(demonstration.createdAtUnixMs).toLocaleString();
  const audioBadge = demonstration.hasNarrationAudio ? "audio narration" : "no audio";
  rowElement.innerHTML = `
    <div class="recording-row-top">
      <div class="row-main">
        <span class="row-title">${escapeHtml(demonstration.title)}</span>
        <span class="row-sub">${demonstration.traceEntryCount} events · ${escapeHtml(
          createdLabel,
        )} · ${escapeHtml(audioBadge)}</span>
      </div>
      <div class="row-actions">
        <button data-action="expand">View screenshots</button>
        <button data-action="adopt" class="primary">Convert to flow</button>
        <button data-action="delete" class="danger">Delete</button>
      </div>
    </div>
    <div data-region="screenshots" class="screenshots-grid" hidden></div>
  `;

  const screenshotsRegionElement = rowElement.querySelector<HTMLDivElement>(
    "div[data-region='screenshots']",
  )!;

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='expand']")!
    .addEventListener("click", async () => {
      if (!screenshotsRegionElement.hidden) {
        screenshotsRegionElement.hidden = true;
        return;
      }
      try {
        const screenshotAbsolutePaths = await invoke<string[]>(
          "list_demonstration_screenshots",
          { demonstrationId: demonstration.id },
        );
        screenshotsRegionElement.innerHTML = "";
        if (screenshotAbsolutePaths.length === 0) {
          screenshotsRegionElement.textContent = "No screenshots captured during this recording.";
        } else {
          for (const absolutePath of screenshotAbsolutePaths) {
            const imageElement = document.createElement("img");
            imageElement.src = convertFileSrc(absolutePath);
            imageElement.alt = "demonstration screenshot";
            screenshotsRegionElement.appendChild(imageElement);
          }
        }
        screenshotsRegionElement.hidden = false;
      } catch (screenshotsError) {
        flashBanner(`Load screenshots failed: ${errorMessageOf(screenshotsError)}`);
      }
    });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='adopt']")!
    .addEventListener("click", async () => {
      const flowName = window.prompt("Flow name:", demonstration.title);
      if (!flowName) return;
      try {
        await invoke("adopt_demonstration_as_flow", {
          demonstrationId: demonstration.id,
          name: flowName,
        });
        flashBanner("Adopted as flow.");
      } catch (adoptError) {
        flashBanner(`Adopt failed: ${errorMessageOf(adoptError)}`);
      }
    });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='delete']")!
    .addEventListener("click", async () => {
      if (!window.confirm(`Delete recording "${demonstration.title}"?`)) return;
      try {
        await invoke("delete_demonstration", { demonstrationId: demonstration.id });
        flashBanner("Deleted.");
        await refreshRecordingsList();
      } catch (deleteError) {
        flashBanner(`Delete failed: ${errorMessageOf(deleteError)}`);
      }
    });

  return rowElement;
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
