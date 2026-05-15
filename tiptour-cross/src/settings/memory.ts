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
    <p class="lede">Persistent facts the agent remembers across sessions. Recalled by semantic search.</p>
    <div class="button-stack" style="display:flex;gap:8px;align-items:center;margin-bottom:12px">
      <input id="memory-search" type="search" placeholder="Filter memories…" style="flex:1;padding:6px 10px;border-radius:6px;border:1px solid var(--color-border)" />
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
        <div class="empty-state" style="padding:24px;text-align:center;opacity:0.7">
          No memories yet. The agent will fill this in as you talk, or you can add one manually.
        </div>
      `;
      return;
    }
    const rowsHtml = records
      .map((record) => {
        const importancePercent = Math.round(record.importance * 100);
        const tagsHtml = record.tags
          .map((tag) => `<span class="chip" style="margin-right:4px">${escapeHtml(tag)}</span>`)
          .join("");
        return `
          <tr data-memory-id="${escapeHtml(record.id)}">
            <td><strong>${escapeHtml(record.key)}</strong></td>
            <td>${escapeHtml(record.value)}</td>
            <td>${tagsHtml}</td>
            <td>${escapeHtml(record.source ?? "—")}</td>
            <td title="${importancePercent}%">
              <div style="background:var(--color-border);height:6px;border-radius:3px;width:60px">
                <div style="background:var(--color-accent);height:100%;border-radius:3px;width:${importancePercent}%"></div>
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
      <table style="width:100%;border-collapse:collapse;font-size:13px">
        <thead>
          <tr style="text-align:left;border-bottom:1px solid var(--color-border)">
            <th style="padding:6px">Key</th>
            <th style="padding:6px">Value</th>
            <th style="padding:6px">Tags</th>
            <th style="padding:6px">Source</th>
            <th style="padding:6px">Importance</th>
            <th style="padding:6px">Last recalled</th>
            <th style="padding:6px"></th>
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
