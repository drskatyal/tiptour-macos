// Memory tab — search, edit, forget the agent's persistent memories.

import { invoke } from "@tauri-apps/api/core";

interface MemoryRecord {
  id: string;
  key: string;
  value: string;
  tags: string[];
  source: string | null;
  createdAtUnixSeconds: number;
  lastRecalledAtUnixSeconds: number | null;
  recallCount: number;
  importance: number;
}

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatRelativeTimestamp(unixSeconds: number | null): string {
  if (unixSeconds == null) return "—";
  const nowMs = Date.now();
  const thenMs = unixSeconds * 1000;
  const ageSeconds = Math.max(0, Math.floor((nowMs - thenMs) / 1000));
  if (ageSeconds < 60) return `${ageSeconds}s ago`;
  if (ageSeconds < 3600) return `${Math.floor(ageSeconds / 60)}m ago`;
  if (ageSeconds < 86400) return `${Math.floor(ageSeconds / 3600)}h ago`;
  return `${Math.floor(ageSeconds / 86400)}d ago`;
}

export async function renderMemoryTab(paneElement: HTMLElement): Promise<void> {
  paneElement.innerHTML = `
    <h2>Memory</h2>
    <p class="lede">
      Memories are facts you tell TipTour explicitly — "my GitHub handle is X",
      "Sara's email is …", "I prefer Cerebras for quick rewrites" — that the
      agent recalls in future sessions via semantic search. Unlike chat
      history, memories outlive sessions and surface across personas. Add
      one below or just tell TipTour mid-conversation ("remember that I
      deploy on Fridays only"); both routes land in the same store.
    </p>
    <div class="filter-bar">
      <input id="memory-search" type="search" class="filter-bar-input" placeholder="Filter memories…" />
      <button id="memory-add" class="primary">Add memory</button>
    </div>
    <div id="memory-table-wrap"></div>
    <div id="memory-status-banner" class="flag-banner success" hidden></div>
  `;

  const searchInputElement = paneElement.querySelector<HTMLInputElement>("#memory-search")!;
  const addButtonElement = paneElement.querySelector<HTMLButtonElement>("#memory-add")!;
  const tableWrapElement = paneElement.querySelector<HTMLDivElement>("#memory-table-wrap")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#memory-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  let cachedRecords: MemoryRecord[] = [];

  function applyFilterAndRender(): void {
    const needle = searchInputElement.value.trim().toLowerCase();
    const filtered = !needle
      ? cachedRecords
      : cachedRecords.filter((record) => {
          return (
            record.key.toLowerCase().includes(needle) ||
            record.value.toLowerCase().includes(needle) ||
            record.tags.some((tag) => tag.toLowerCase().includes(needle))
          );
        });
    renderTable(filtered);
  }

  function renderTable(records: MemoryRecord[]): void {
    if (records.length === 0) {
      tableWrapElement.innerHTML = `
        <div class="memory-empty-state">
          No memories yet. The agent will store useful facts here as you use it.
          Try asking it to remember something specific.
        </div>
      `;
      return;
    }
    const rowsHtml = records
      .map((record) => {
        const importancePercent = Math.round(record.importance * 100);
        const tagsHtml = record.tags
          .map((tag) => `<span class="chip memory-tag-chip">${escapeHtml(tag)}</span>`)
          .join("");
        return `
          <tr data-memory-id="${escapeHtml(record.id)}">
            <td><strong>${escapeHtml(record.key)}</strong></td>
            <td>${escapeHtml(record.value)}</td>
            <td>${tagsHtml}</td>
            <td>${escapeHtml(record.source ?? "—")}</td>
            <td title="${importancePercent}%">
              <div class="importance-bar-container">
                <div class="importance-bar-fill" style="width:${importancePercent}%"></div>
              </div>
            </td>
            <td>${formatRelativeTimestamp(record.lastRecalledAtUnixSeconds)}</td>
            <td>
              <button class="memory-edit-btn" data-memory-id="${escapeHtml(record.id)}">Edit</button>
              <button class="memory-forget-btn" data-memory-id="${escapeHtml(record.id)}">Forget</button>
            </td>
          </tr>
        `;
      })
      .join("");
    tableWrapElement.innerHTML = `
      <table class="memory-table">
        <thead>
          <tr>
            <th>Key</th>
            <th>Value</th>
            <th>Tags</th>
            <th>Source</th>
            <th>Importance</th>
            <th>Last recalled</th>
            <th></th>
          </tr>
        </thead>
        <tbody>${rowsHtml}</tbody>
      </table>
    `;
    for (const editButton of tableWrapElement.querySelectorAll<HTMLButtonElement>(".memory-edit-btn")) {
      editButton.addEventListener("click", () => {
        const memoryId = editButton.dataset.memoryId!;
        void promptEditMemory(memoryId);
      });
    }
    for (const forgetButton of tableWrapElement.querySelectorAll<HTMLButtonElement>(".memory-forget-btn")) {
      forgetButton.addEventListener("click", () => {
        const memoryId = forgetButton.dataset.memoryId!;
        void forgetMemory(memoryId);
      });
    }
  }

  async function refresh(): Promise<void> {
    try {
      cachedRecords = await invoke<MemoryRecord[]>("list_memories", { tagFilter: null });
      applyFilterAndRender();
    } catch (listError) {
      flashBanner(`List memories failed: ${errorMessageOf(listError)}`);
    }
  }

  async function promptEditMemory(memoryId: string): Promise<void> {
    const existing = cachedRecords.find((r) => r.id === memoryId);
    if (!existing) return;
    const newKey = window.prompt("Memory key:", existing.key);
    if (newKey == null) return;
    const newValue = window.prompt("Memory value:", existing.value);
    if (newValue == null) return;
    const newTagsRaw = window.prompt("Tags (comma-separated):", existing.tags.join(", "));
    if (newTagsRaw == null) return;
    const newTags = newTagsRaw.split(",").map((t) => t.trim()).filter(Boolean);
    try {
      await invoke("update_memory", {
        id: memoryId,
        key: newKey,
        value: newValue,
        tags: newTags,
      });
      flashBanner("Memory updated");
      await refresh();
    } catch (updateError) {
      flashBanner(`Update failed: ${errorMessageOf(updateError)}`);
    }
  }

  async function forgetMemory(memoryId: string): Promise<void> {
    if (!window.confirm("Forget this memory? The agent will lose this fact.")) return;
    try {
      await invoke("forget", { id: memoryId });
      flashBanner("Forgotten");
      await refresh();
    } catch (forgetError) {
      flashBanner(`Forget failed: ${errorMessageOf(forgetError)}`);
    }
  }

  addButtonElement.addEventListener("click", async () => {
    const newKey = window.prompt("Memory key (short label):");
    if (!newKey) return;
    const newValue = window.prompt("Memory value (the fact itself):");
    if (!newValue) return;
    const tagsRaw = window.prompt("Tags (comma-separated, optional):") ?? "";
    const tags = tagsRaw.split(",").map((t) => t.trim()).filter(Boolean);
    try {
      await invoke("remember", {
        key: newKey,
        value: newValue,
        tags,
        source: "user",
      });
      flashBanner("Memory added");
      await refresh();
    } catch (addError) {
      flashBanner(`Add failed: ${errorMessageOf(addError)}`);
    }
  });

  searchInputElement.addEventListener("input", applyFilterAndRender);

  await refresh();
}

function escapeHtml(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
