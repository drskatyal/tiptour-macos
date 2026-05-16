// Tasks tab — kanban grid over the Rust task store. Drag a card between
// columns to flip status. Click a card to open the detail drawer. Live
// sub-agent status badges piggyback on `subagent_progress` events.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

type TaskStatus = "backlog" | "inProgress" | "blocked" | "done" | "cancelled";
type TaskPriority = "low" | "medium" | "high" | "urgent";

interface Task {
  id: string;
  title: string;
  description: string;
  status: TaskStatus;
  priority: TaskPriority;
  createdAtUnixSeconds: number;
  startedAtUnixSeconds: number | null;
  completedAtUnixSeconds: number | null;
  assignedSubagentId: string | null;
  parentTaskId: string | null;
  tags: string[];
}

interface Subagent {
  id: string;
  name: string;
  status: "pending" | "running" | "paused" | "done" | "failed";
  lastProgressMessage: string | null;
}

interface SubagentProgressEvent {
  subagentId: string;
  status: Subagent["status"];
  message: string;
}

const COLUMN_DEFINITIONS: { status: TaskStatus; label: string }[] = [
  { status: "backlog", label: "Backlog" },
  { status: "inProgress", label: "In Progress" },
  { status: "blocked", label: "Blocked" },
  { status: "done", label: "Done" },
  { status: "cancelled", label: "Cancelled" },
];

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatRelativeAge(unixSeconds: number): string {
  const ageSeconds = Math.max(0, Math.floor(Date.now() / 1000 - unixSeconds));
  if (ageSeconds < 60) return `${ageSeconds}s`;
  if (ageSeconds < 3600) return `${Math.floor(ageSeconds / 60)}m`;
  if (ageSeconds < 86400) return `${Math.floor(ageSeconds / 3600)}h`;
  return `${Math.floor(ageSeconds / 86400)}d`;
}

function escapeHtml(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

let subagentProgressUnlisten: UnlistenFn | null = null;

export async function renderTasksTab(paneElement: HTMLElement): Promise<void> {
  // Tear down any previous listener when the tab re-renders.
  subagentProgressUnlisten?.();
  subagentProgressUnlisten = null;

  paneElement.innerHTML = `
    <h2>Tasks</h2>
    <p class="lede">A kanban for things the agent's working on. Drag cards between columns. Tasks can be dispatched to a parallel sub-agent.</p>
    <div style="display:flex;gap:8px;margin-bottom:12px;align-items:center;flex-wrap:wrap">
      <button id="task-new" class="primary">New task</button>
      <input id="task-filter-tag" type="search" placeholder="Filter by tag…" style="padding:6px 10px;border-radius:6px;border:1px solid var(--color-border)" />
      <label style="display:flex;gap:4px;align-items:center;font-size:12px">
        <input id="task-filter-subagent" type="checkbox" /> With sub-agent only
      </label>
    </div>
    <div id="kanban-empty-state-host" class="empty-state" hidden style="padding:14px;text-align:center;opacity:0.7;font-size:12px;border:1px dashed rgba(255,255,255,0.12);border-radius:8px;margin-bottom:8px">
      No tasks yet. Create one from the panel or ask the agent:
      "remind me to ship the demo this week".
    </div>
    <div id="kanban-grid" style="display:grid;grid-template-columns:repeat(5,minmax(180px,1fr));gap:12px;align-items:start"></div>
    <div id="task-status-banner" class="flag-banner success" hidden></div>
    <div id="task-drawer" hidden></div>
  `;

  const kanbanGridElement = paneElement.querySelector<HTMLDivElement>("#kanban-grid")!;
  const newButtonElement = paneElement.querySelector<HTMLButtonElement>("#task-new")!;
  const tagFilterElement = paneElement.querySelector<HTMLInputElement>("#task-filter-tag")!;
  const subagentFilterElement = paneElement.querySelector<HTMLInputElement>("#task-filter-subagent")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>("#task-status-banner")!;
  const drawerElement = paneElement.querySelector<HTMLDivElement>("#task-drawer")!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  let cachedTasks: Task[] = [];
  let cachedSubagentsById = new Map<string, Subagent>();

  async function refresh(): Promise<void> {
    try {
      cachedTasks = await invoke<Task[]>("list_tasks", { statusFilter: null, tagFilter: null });
      const subagents = await invoke<Subagent[]>("list_subagents");
      cachedSubagentsById = new Map(subagents.map((s) => [s.id, s]));
      renderColumns();
    } catch (listError) {
      flashBanner(`List failed: ${errorMessageOf(listError)}`);
    }
  }

  function renderColumns(): void {
    const tagNeedle = tagFilterElement.value.trim().toLowerCase();
    const subagentOnly = subagentFilterElement.checked;
    const visible = cachedTasks.filter((task) => {
      if (subagentOnly && !task.assignedSubagentId) return false;
      if (tagNeedle && !task.tags.some((t) => t.toLowerCase().includes(tagNeedle))) return false;
      return true;
    });

    // Surface the "no tasks yet" nudge ABOVE the kanban (in a separate
    // pre-grid element so the columns themselves always render). This
    // matters for both UX (empty columns are inviting drop targets) and
    // testability (the e2e harness asserts there are always 5 columns).
    const emptyStateHostElement = paneElement.querySelector<HTMLDivElement>(
      "#kanban-empty-state-host",
    );
    if (emptyStateHostElement) {
      emptyStateHostElement.hidden = cachedTasks.length !== 0;
    }

    kanbanGridElement.innerHTML = COLUMN_DEFINITIONS.map(
      (column) => `
        <div class="kanban-column" data-status="${column.status}"
             style="background:var(--color-surface-alt,#1d1d22);border-radius:8px;padding:8px;min-height:160px">
          <div style="font-weight:600;font-size:12px;text-transform:uppercase;letter-spacing:0.04em;margin-bottom:6px;opacity:0.75">
            ${column.label} (${visible.filter((t) => t.status === column.status).length})
          </div>
          <div class="kanban-cards" data-column-status="${column.status}"></div>
        </div>
      `,
    ).join("");

    for (const column of COLUMN_DEFINITIONS) {
      const cardContainer = kanbanGridElement.querySelector<HTMLDivElement>(
        `.kanban-cards[data-column-status="${column.status}"]`,
      )!;
      const tasksForColumn = visible.filter((task) => task.status === column.status);
      for (const task of tasksForColumn) {
        cardContainer.appendChild(buildCardElement(task));
      }
      installColumnDropTarget(cardContainer, column.status);
    }
  }

  function buildCardElement(task: Task): HTMLDivElement {
    const cardElement = document.createElement("div");
    cardElement.className = "kanban-card";
    cardElement.draggable = true;
    cardElement.dataset.taskId = task.id;
    cardElement.style.cssText =
      "background:var(--color-surface);border:1px solid var(--color-border);border-radius:6px;padding:8px;margin-bottom:6px;cursor:grab;font-size:12px";

    const subagentForCard = task.assignedSubagentId
      ? cachedSubagentsById.get(task.assignedSubagentId) ?? null
      : null;
    const subagentChip = subagentForCard
      ? `<span class="chip" style="font-size:10px;background:var(--color-accent);color:#fff">
           agent: ${escapeHtml(subagentForCard.name)} · ${subagentForCard.status}
         </span>`
      : "";

    cardElement.innerHTML = `
      <div style="font-weight:600;margin-bottom:4px">${escapeHtml(task.title)}</div>
      ${task.description ? `<div style="opacity:0.75;margin-bottom:4px">${escapeHtml(task.description.slice(0, 80))}${task.description.length > 80 ? "…" : ""}</div>` : ""}
      <div style="display:flex;gap:4px;flex-wrap:wrap;align-items:center">
        <span class="chip" style="font-size:10px">${task.priority}</span>
        <span class="chip" style="font-size:10px">${formatRelativeAge(task.createdAtUnixSeconds)} old</span>
        ${task.tags.map((tag) => `<span class="chip" style="font-size:10px">${escapeHtml(tag)}</span>`).join("")}
        ${subagentChip}
      </div>
    `;
    cardElement.addEventListener("dragstart", (event) => {
      event.dataTransfer?.setData("text/plain", task.id);
      cardElement.style.opacity = "0.5";
    });
    cardElement.addEventListener("dragend", () => {
      cardElement.style.opacity = "1";
    });
    cardElement.addEventListener("click", () => openDrawerForTask(task));
    return cardElement;
  }

  function installColumnDropTarget(cardContainer: HTMLElement, targetStatus: TaskStatus): void {
    cardContainer.addEventListener("dragover", (event) => {
      event.preventDefault();
    });
    cardContainer.addEventListener("drop", async (event) => {
      event.preventDefault();
      const droppedTaskId = event.dataTransfer?.getData("text/plain");
      if (!droppedTaskId) return;
      try {
        await invoke("update_task_status", { id: droppedTaskId, status: targetStatus });
        await refresh();
      } catch (moveError) {
        flashBanner(`Move failed: ${errorMessageOf(moveError)}`);
      }
    });
  }

  function openDrawerForTask(task: Task): void {
    const subagentForTask = task.assignedSubagentId
      ? cachedSubagentsById.get(task.assignedSubagentId) ?? null
      : null;
    drawerElement.hidden = false;
    drawerElement.style.cssText =
      "position:fixed;top:0;right:0;height:100vh;width:380px;background:var(--color-surface);border-left:1px solid var(--color-border);padding:16px;overflow-y:auto;box-shadow:-4px 0 12px rgba(0,0,0,0.3);z-index:1000";
    drawerElement.innerHTML = `
      <button id="drawer-close" style="float:right">×</button>
      <h3>${escapeHtml(task.title)}</h3>
      <p style="opacity:0.8;white-space:pre-wrap">${escapeHtml(task.description || "(no description)")}</p>
      <div style="display:flex;gap:6px;flex-wrap:wrap;margin:8px 0">
        <span class="chip">${task.status}</span>
        <span class="chip">${task.priority}</span>
        ${task.tags.map((t) => `<span class="chip">${escapeHtml(t)}</span>`).join("")}
      </div>
      <p style="font-size:11px;opacity:0.7">Created ${formatRelativeAge(task.createdAtUnixSeconds)} ago</p>
      ${
        subagentForTask
          ? `<div style="margin-top:12px;padding:8px;background:var(--color-surface-alt,#1d1d22);border-radius:6px">
               <strong>Sub-agent:</strong> ${escapeHtml(subagentForTask.name)}<br/>
               Status: ${subagentForTask.status}<br/>
               ${subagentForTask.lastProgressMessage ? `<em>${escapeHtml(subagentForTask.lastProgressMessage)}</em><br/>` : ""}
               <button id="drawer-open-transcript" style="margin-top:6px">View transcript path</button>
             </div>`
          : `<button id="drawer-dispatch" class="primary" style="margin-top:8px">Dispatch to sub-agent</button>`
      }
      <hr style="margin:12px 0;border-color:var(--color-border)" />
      <button id="drawer-delete" style="color:#ff5d5d">Delete task</button>
    `;
    drawerElement.querySelector<HTMLButtonElement>("#drawer-close")!.addEventListener("click", () => {
      drawerElement.hidden = true;
    });
    drawerElement.querySelector<HTMLButtonElement>("#drawer-delete")!.addEventListener("click", async () => {
      if (!window.confirm("Delete this task?")) return;
      try {
        await invoke("delete_task", { id: task.id });
        drawerElement.hidden = true;
        await refresh();
      } catch (deleteError) {
        flashBanner(`Delete failed: ${errorMessageOf(deleteError)}`);
      }
    });
    const dispatchButton = drawerElement.querySelector<HTMLButtonElement>("#drawer-dispatch");
    if (dispatchButton) {
      dispatchButton.addEventListener("click", async () => {
        try {
          await invoke<string>("dispatch_task_to_subagent", {
            taskId: task.id,
            systemPromptOverride: null,
          });
          flashBanner("Sub-agent dispatched");
          drawerElement.hidden = true;
          await refresh();
        } catch (dispatchError) {
          flashBanner(`Dispatch failed: ${errorMessageOf(dispatchError)}`);
        }
      });
    }
    const transcriptButton = drawerElement.querySelector<HTMLButtonElement>("#drawer-open-transcript");
    if (transcriptButton && task.assignedSubagentId) {
      transcriptButton.addEventListener("click", async () => {
        try {
          const path = await invoke<string>("get_subagent_transcript_path", {
            subagentId: task.assignedSubagentId,
          });
          window.alert(`Transcript: ${path}`);
        } catch (pathError) {
          flashBanner(`Transcript path failed: ${errorMessageOf(pathError)}`);
        }
      });
    }
  }

  newButtonElement.addEventListener("click", async () => {
    const title = window.prompt("Task title:");
    if (!title) return;
    const description = window.prompt("Description (optional):") ?? "";
    try {
      await invoke("create_task", {
        title,
        description,
        priority: "medium",
        parentTaskId: null,
        tags: [],
      });
      flashBanner("Task created");
      await refresh();
    } catch (createError) {
      flashBanner(`Create failed: ${errorMessageOf(createError)}`);
    }
  });

  tagFilterElement.addEventListener("input", renderColumns);
  subagentFilterElement.addEventListener("change", renderColumns);

  // Live-update sub-agent badges as progress events fire.
  subagentProgressUnlisten = await listen<SubagentProgressEvent>(
    "subagent_progress",
    (event) => {
      const existing = cachedSubagentsById.get(event.payload.subagentId);
      if (existing) {
        existing.status = event.payload.status;
        existing.lastProgressMessage = event.payload.message;
      }
      renderColumns();
    },
  );

  await refresh();
}
