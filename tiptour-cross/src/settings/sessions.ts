// History tab: lists every saved Gemini Live session, lets the user
// open one and read the full transcript, or delete it. Sessions are
// stored one file per push-to-talk conversation (see
// src-tauri/src/conversation_history.rs) — this tab is the only
// surface that reads them.

import { invoke } from "@tauri-apps/api/core";

type SessionSummary = {
  session_id: string;
  started_at: string;
  title: string;
  turn_count: number;
};

type ConversationTurn = {
  role: string;
  text: string;
  at: string;
};

type ConversationSession = {
  session_id: string;
  started_at: string;
  title: string;
  turns: ConversationTurn[];
};

function formatTimestamp(rfc3339: string): string {
  const parsed = new Date(rfc3339);
  if (Number.isNaN(parsed.getTime())) return rfc3339;
  return parsed.toLocaleString();
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export async function renderSessionsTab(paneElement: HTMLElement): Promise<void> {
  paneElement.innerHTML = `
    <h2>Conversation history</h2>
    <p class="settings-lede">
      One entry per push-to-talk conversation. The tail of your most
      recent session is silently seeded into the next one for
      continuity — older sessions stay private to this list.
    </p>
    <div class="sessions-layout">
      <div class="sessions-list" id="sessions-list">
        <div class="settings-loading-row"></div>
      </div>
      <div class="sessions-detail" id="sessions-detail">
        <p class="sessions-empty">Pick a session on the left to read it.</p>
      </div>
    </div>
  `;

  const listElement = paneElement.querySelector<HTMLDivElement>("#sessions-list")!;
  const detailElement = paneElement.querySelector<HTMLDivElement>("#sessions-detail")!;

  async function refreshList() {
    const summaries =
      (await invoke<SessionSummary[]>("list_sessions").catch(() => [])) ?? [];
    if (summaries.length === 0) {
      listElement.innerHTML = `<p class="sessions-empty">No saved sessions yet — open push-to-talk and say something to record one.</p>`;
      return;
    }
    listElement.innerHTML = summaries
      .map((summary) => {
        return `
          <div class="sessions-row" data-session-id="${escapeHtml(summary.session_id)}">
            <div class="sessions-row-main">
              <div class="sessions-row-title">${escapeHtml(summary.title)}</div>
              <div class="sessions-row-meta">${formatTimestamp(summary.started_at)} · ${summary.turn_count} turn${summary.turn_count === 1 ? "" : "s"}</div>
            </div>
            <button class="sessions-row-delete" data-delete-session-id="${escapeHtml(summary.session_id)}" title="Delete this session">×</button>
          </div>
        `;
      })
      .join("");
  }

  listElement.addEventListener("click", async (event) => {
    const target = event.target as HTMLElement;
    const deleteId = target.dataset.deleteSessionId;
    if (deleteId) {
      event.stopPropagation();
      await invoke("delete_session", { sessionId: deleteId }).catch(() => {});
      await refreshList();
      detailElement.innerHTML = `<p class="sessions-empty">Pick a session on the left to read it.</p>`;
      return;
    }
    const row = target.closest<HTMLElement>(".sessions-row");
    if (!row) return;
    const sessionId = row.dataset.sessionId;
    if (!sessionId) return;
    try {
      const session = await invoke<ConversationSession>("get_session_history", {
        sessionId,
      });
      detailElement.innerHTML = `
        <div class="sessions-detail-header">
          <h3>${escapeHtml(session.title || "Untitled session")}</h3>
          <div class="sessions-detail-meta">${formatTimestamp(session.started_at)}</div>
        </div>
        <div class="sessions-turns">
          ${session.turns
            .map(
              (turn) => `
                <div class="sessions-turn sessions-turn-${escapeHtml(turn.role)}">
                  <div class="sessions-turn-role">${turn.role === "user" ? "You" : "TipTour"}</div>
                  <div class="sessions-turn-text">${escapeHtml(turn.text)}</div>
                </div>
              `,
            )
            .join("")}
        </div>
      `;
    } catch (error) {
      detailElement.innerHTML = `<p class="sessions-empty">Failed to load: ${escapeHtml(String(error))}</p>`;
    }
  });

  await refreshList();
}
