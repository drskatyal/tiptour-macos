// Saved Flows tab — full management surface for multiflow entries.

import { invoke } from "@tauri-apps/api/core";

interface FlowSummary {
  flowId: string;
  name: string;
  createdAtUnixMs: number;
  stepCount: number;
  triggerAliases: string[];
}

export async function renderFlowsTab(paneElement: HTMLElement): Promise<void> {
  paneElement.innerHTML = `
    <h2>Saved Flows</h2>
    <p class="lede">
      A saved flow is a multi-step routine you've demonstrated once — open
      Slack, click #standup, type yesterday's wins — that TipTour can replay
      on voice command later. Each flow has trigger phrases ("run my
      morning routine") that the wake-word listener matches against, and
      can be re-recorded any time. Import a flow file shared by a
      teammate, or hit Record from the panel to start a new one.
    </p>
    <div class="button-stack">
      <button id="flows-import" class="primary">Import flow…</button>
    </div>
    <ul id="flows-list" class="list-rows" style="margin-top:12px"></ul>
    <div id="flows-status-banner" class="flag-banner success" hidden></div>
  `;

  const flowsListElement = paneElement.querySelector<HTMLUListElement>("#flows-list")!;
  const importButtonElement = paneElement.querySelector<HTMLButtonElement>("#flows-import")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#flows-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  async function refreshFlowsList(): Promise<void> {
    let flows: FlowSummary[] = [];
    try {
      flows = await invoke<FlowSummary[]>("list_flows");
    } catch (listError) {
      flashBanner(`List flows failed: ${errorMessageOf(listError)}`);
      return;
    }
    flowsListElement.innerHTML = "";
    if (flows.length === 0) {
      const emptyRow = document.createElement("li");
      emptyRow.className = "list-row disabled empty-state";
      emptyRow.textContent = "No saved flows yet. Record one from the panel to recall it by voice.";
      flowsListElement.appendChild(emptyRow);
      return;
    }
    for (const flow of flows) {
      flowsListElement.appendChild(renderFlowRow(flow, refreshFlowsList, flashBanner));
    }
  }

  importButtonElement.addEventListener("click", async () => {
    // Open a hidden file input so we don't need a Tauri-dialog plugin.
    const fileInputElement = document.createElement("input");
    fileInputElement.type = "file";
    fileInputElement.accept = ".json,.tiptour-flow.json,application/json";
    fileInputElement.addEventListener("change", async () => {
      const pickedFile = fileInputElement.files?.[0];
      if (!pickedFile) return;
      const fileText = await pickedFile.text();
      try {
        await invoke<FlowSummary>("import_flow", { exportedBlob: fileText });
        flashBanner("Flow imported.");
        await refreshFlowsList();
      } catch (importError) {
        flashBanner(`Import failed: ${errorMessageOf(importError)}`);
      }
    });
    fileInputElement.click();
  });

  await refreshFlowsList();
}

function renderFlowRow(
  flow: FlowSummary,
  refreshFlowsList: () => Promise<void>,
  flashBanner: (message: string) => void,
): HTMLElement {
  const rowElement = document.createElement("li");
  rowElement.className = "list-row";
  const createdLabel = new Date(flow.createdAtUnixMs).toLocaleString();
  const aliasesText = flow.triggerAliases.join(", ");
  rowElement.innerHTML = `
    <div class="row-main">
      <span class="row-title">${escapeHtml(flow.name)}</span>
      <span class="row-sub">${flow.stepCount} steps · created ${escapeHtml(createdLabel)}</span>
      <input type="text" placeholder="trigger aliases (comma-separated)" value="${escapeHtml(
        aliasesText,
      )}" data-field="aliases" style="margin-top:6px; width:100%; box-sizing:border-box" />
    </div>
    <div class="row-actions">
      <button data-action="run" class="primary">Run</button>
      <button data-action="rename">Rename</button>
      <button data-action="export">Export</button>
      <button data-action="delete" class="danger">Delete</button>
    </div>
  `;

  const aliasesInputElement = rowElement.querySelector<HTMLInputElement>(
    "input[data-field='aliases']",
  )!;
  aliasesInputElement.addEventListener("blur", async () => {
    const aliases = aliasesInputElement.value
      .split(",")
      .map((token) => token.trim())
      .filter((token) => token.length > 0);
    try {
      await invoke("set_flow_trigger_aliases", {
        flowId: flow.flowId,
        triggerAliases: aliases,
      });
      flashBanner("Aliases saved.");
    } catch (aliasError) {
      flashBanner(`Aliases failed: ${errorMessageOf(aliasError)}`);
    }
  });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='run']")!
    .addEventListener("click", async () => {
      try {
        await invoke<string>("run_flow_by_name", { name: flow.name });
        flashBanner(`Running ${flow.name}…`);
      } catch (runError) {
        flashBanner(`Run failed: ${errorMessageOf(runError)}`);
      }
    });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='rename']")!
    .addEventListener("click", async () => {
      const proposedName = window.prompt("New name for this flow:", flow.name);
      if (!proposedName) return;
      try {
        await invoke("rename_flow", { flowId: flow.flowId, newName: proposedName });
        flashBanner("Renamed.");
        await refreshFlowsList();
      } catch (renameError) {
        flashBanner(`Rename failed: ${errorMessageOf(renameError)}`);
      }
    });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='export']")!
    .addEventListener("click", async () => {
      try {
        const exportedBlob = await invoke<string>("export_flow", { flowId: flow.flowId });
        const downloadBlob = new Blob([exportedBlob], { type: "application/json" });
        const downloadUrl = URL.createObjectURL(downloadBlob);
        const downloadAnchor = document.createElement("a");
        downloadAnchor.href = downloadUrl;
        downloadAnchor.download = `${flow.name.replace(/[^a-z0-9-_]+/gi, "_")}.tiptour-flow.json`;
        downloadAnchor.click();
        setTimeout(() => URL.revokeObjectURL(downloadUrl), 1500);
        flashBanner("Exported.");
      } catch (exportError) {
        flashBanner(`Export failed: ${errorMessageOf(exportError)}`);
      }
    });

  rowElement
    .querySelector<HTMLButtonElement>("button[data-action='delete']")!
    .addEventListener("click", async () => {
      if (!window.confirm(`Delete flow "${flow.name}"? This removes the recording.`)) return;
      try {
        await invoke("delete_flow", { name: flow.flowId });
        flashBanner("Deleted.");
        await refreshFlowsList();
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
