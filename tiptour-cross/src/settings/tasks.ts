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
    <p class="lede">
      A kanban for work TipTour is doing on your behalf. Each card represents
      one task — research a vendor, draft a status update, summarise a doc —
      and can be dispatched to a parallel sub-agent that runs in the
      background while you keep talking to the main session. Drag cards
      between columns as the work progresses; the agent updates state on
      its own when it finishes.
    </p>
    <div class="filter-bar">
      <button id="task-new" class="primary">New task</button>
      <input id="task-filter-tag" type="search" class="filter-bar-input" placeholder="Filter by tag…" />
      <label class="filter-bar-checkbox-label">
        <input id="task-filter-subagent" type="checkbox" /> With sub-agent only
      </label>
    </div>
    <div id="kanban-empty-state-host" class="kanban-empty-state" hidden>
      No tasks yet. Create one from the panel or ask the agent:
      "remind me to ship the demo this week".
    </div>
    <div id="kanban-grid" class="kanban-grid"></div>
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
        <div class="kanban-column" data-status="${column.status}">
          <div class="kanban-column-header">
            ${column.label} <span class="kanban-column-count">${visible.filter((t) => t.status === column.status).length}</span>
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

    const subagentForCard = task.assignedSubagentId
      ? cachedSubagentsById.get(task.assignedSubagentId) ?? null
      : null;
    const subagentChip = subagentForCard
      ? `<span class="chip chip-accent">agent: ${escapeHtml(subagentForCard.name)} · ${subagentForCard.status}</span>`
      : "";

    cardElement.innerHTML = `
      <div class="kanban-card-title">${escapeHtml(task.title)}</div>
      ${task.description ? `<div class="kanban-card-description">${escapeHtml(task.description.slice(0, 80))}${task.description.length > 80 ? "…" : ""}</div>` : ""}
      <div class="kanban-card-chips">
        <span class="chip">${task.priority}</span>
        <span class="chip">${formatRelativeAge(task.createdAtUnixSeconds)} old</span>
        ${task.tags.map((tag) => `<span class="chip">${escapeHtml(tag)}</span>`).join("")}
        ${subagentChip}
      </div>
    `;
    cardElement.addEventListener("dragstart", (event) => {
      event.dataTransfer?.setData("text/plain", task.id);
      cardElement.classList.add("kanban-card-dragging");
    });
    cardElement.addEventListener("dragend", () => {
      cardElement.classList.remove("kanban-card-dragging");
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
    drawerElement.removeAttribute("hidden");
    drawerElement.className = "task-drawer open";
    drawerElement.innerHTML = `
      <button id="drawer-close" class="task-drawer-close" aria-label="Close task details">×</button>
      <h3>${escapeHtml(task.title)}</h3>
      <p class="task-drawer-description">${escapeHtml(task.description || "(no description)")}</p>
      <div class="task-drawer-chips">
        <span class="chip">${task.status}</span>
        <span class="chip">${task.priority}</span>
        ${task.tags.map((t) => `<span class="chip">${escapeHtml(t)}</span>`).join("")}
      </div>
      <p class="task-drawer-meta">Created ${formatRelativeAge(task.createdAtUnixSeconds)} ago</p>
      ${
        subagentForTask
          ? `<div class="task-drawer-subagent">
               <strong>Sub-agent:</strong> ${escapeHtml(subagentForTask.name)}<br/>
               Status: ${subagentForTask.status}<br/>
               ${subagentForTask.lastProgressMessage ? `<em>${escapeHtml(subagentForTask.lastProgressMessage)}</em><br/>` : ""}
               <button id="drawer-open-transcript">View transcript path</button>
             </div>`
          : `<button id="drawer-dispatch" class="primary">Dispatch to sub-agent</button>`
      }
      <hr class="task-drawer-divider" />
      <button id="drawer-delete" class="danger">Delete task</button>
    `;
    drawerElement.querySelector<HTMLButtonElement>("#drawer-close")!.addEventListener("click", () => {
      drawerElement.classList.remove("open");
    });
    drawerElement.querySelector<HTMLButtonElement>("#drawer-delete")!.addEventListener("click", async () => {
      if (!window.confirm("Delete this task?")) return;
      try {
        await invoke("delete_task", { id: task.id });
        drawerElement.classList.remove("open");
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
          drawerElement.classList.remove("open");
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
