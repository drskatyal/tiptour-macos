// Settings → Tasks tab kanban. Asserts:
//   - 5 columns render (Backlog, In Progress, Blocked, Done, Cancelled)
//   - dragging a card between columns dispatches `update_task_status`
//     with the destination column's status

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

const FIXTURE_TASK = {
  id: "task-1",
  title: "Wire the kanban end-to-end",
  description: "",
  status: "backlog",
  priority: "medium",
  createdAtUnixSeconds: 1_700_000_000,
  startedAtUnixSeconds: null,
  completedAtUnixSeconds: null,
  assignedSubagentId: null,
  parentTaskId: null,
  tags: [],
};

test.describe("settings tasks tab", () => {
  test("renders 5 kanban columns", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        get_api_key: () => null,
        get_app_settings: () => ({
          schemaVersion: 1,
          geminiVoice: "Kore",
          geminiModel: "gemini-3.1-flash-live-preview",
          pushToTalkChord: "Alt+X",
        }),
        get_operating_mode: () => "autopilot",
        is_recording_enabled: () => false,
        is_listener_enabled: () => false,
        list_tasks: () => [],
        list_subagents: () => [],
      },
    });
    await page.goto("/settings.html");
    await page.locator(".settings-tab[data-tab='tasks']").click();
    await expect(page.locator(".kanban-column")).toHaveCount(5);
  });

  test("dragging a card to In Progress dispatches update_task_status with inProgress", async ({
    page,
  }) => {
    await installTauriMock(page, {
      handlers: {
        get_api_key: () => null,
        get_app_settings: () => ({
          schemaVersion: 1,
          geminiVoice: "Kore",
          geminiModel: "gemini-3.1-flash-live-preview",
          pushToTalkChord: "Alt+X",
        }),
        get_operating_mode: () => "autopilot",
        is_recording_enabled: () => false,
        is_listener_enabled: () => false,
        list_tasks: () => [
          {
            id: "task-1",
            title: "Wire the kanban end-to-end",
            description: "",
            status: "backlog",
            priority: "medium",
            createdAtUnixSeconds: 1_700_000_000,
            startedAtUnixSeconds: null,
            completedAtUnixSeconds: null,
            assignedSubagentId: null,
            parentTaskId: null,
            tags: [],
          },
        ],
        list_subagents: () => [],
        // Echo back a tweaked task so the post-drop refresh path
        // doesn't blow up.
        update_task_status: (args) => ({
          ...{
            id: "task-1",
            title: "Wire the kanban end-to-end",
            description: "",
            status: "backlog",
            priority: "medium",
            createdAtUnixSeconds: 1_700_000_000,
            startedAtUnixSeconds: null,
            completedAtUnixSeconds: null,
            assignedSubagentId: null,
            parentTaskId: null,
            tags: [],
          },
          status: args?.status ?? "backlog",
        }),
      },
    });
    await page.goto("/settings.html");
    await page.locator(".settings-tab[data-tab='tasks']").click();

    const cardLocator = page.locator(`.kanban-card[data-task-id="${FIXTURE_TASK.id}"]`);
    await expect(cardLocator).toBeVisible();
    const inProgressDropTarget = page.locator(
      `.kanban-cards[data-column-status="inProgress"]`,
    );

    // Playwright's dragTo synthesizes the right HTML5 dragstart/drop
    // events that the tab's installColumnDropTarget listens for.
    await cardLocator.dragTo(inProgressDropTarget);

    // Allow the post-drop async refresh to run.
    await page.waitForTimeout(100);
    const recordedInvokes = await page.evaluate(() => window.__mockInvokeCalls);
    const updateCalls = recordedInvokes.filter((call) => call.cmd === "update_task_status");
    expect(updateCalls.length).toBeGreaterThan(0);
    expect(updateCalls[updateCalls.length - 1].args).toMatchObject({
      id: FIXTURE_TASK.id,
      status: "inProgress",
    });
  });
});
