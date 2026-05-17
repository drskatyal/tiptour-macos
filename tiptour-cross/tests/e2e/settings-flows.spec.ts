// Settings → Saved Flows tab. Asserts that a fixture flow renders with
// its name + step-count, that clicking Run dispatches `run_flow_by_name`
// with the flow's name, and that delete shows a confirmation prompt.

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

const FIXTURE_FLOW = {
  flowId: "flow-uuid-1234",
  name: "morning routine",
  createdAtUnixMs: 1_700_000_000_000,
  stepCount: 7,
  triggerAliases: ["morning", "kickoff"],
};

test.describe("settings flows tab", () => {
  test("renders a fixture flow with name + step count, Run dispatches run_flow_by_name", async ({
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
        list_flows: () => [
          {
            flowId: "flow-uuid-1234",
            name: "morning routine",
            createdAtUnixMs: 1_700_000_000_000,
            stepCount: 7,
            triggerAliases: ["morning", "kickoff"],
          },
        ],
        run_flow_by_name: () => "ok",
      },
    });
    await page.goto("/settings.html");
    await page.locator(".settings-tab[data-tab='flows']").click();
    // Title and step count both visible in the row.
    await expect(page.locator("#flows-list .row-title")).toHaveText(FIXTURE_FLOW.name);
    await expect(page.locator("#flows-list .row-sub")).toContainText(
      `${FIXTURE_FLOW.stepCount} steps`,
    );
    // Click Run — assert run_flow_by_name went out with the flow name.
    await page.locator("#flows-list button[data-action='run']").click();
    const recordedInvokes = await page.evaluate(() => window.__mockInvokeCalls);
    const runCalls = recordedInvokes.filter((call) => call.cmd === "run_flow_by_name");
    expect(runCalls.length).toBeGreaterThan(0);
    expect(runCalls[runCalls.length - 1].args).toMatchObject({ name: FIXTURE_FLOW.name });
  });

  test("delete prompts confirmation; cancel skips delete_flow", async ({ page }) => {
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
        list_flows: () => [
          {
            flowId: "flow-uuid-1234",
            name: "morning routine",
            createdAtUnixMs: 1_700_000_000_000,
            stepCount: 7,
            triggerAliases: [],
          },
        ],
      },
    });
    await page.goto("/settings.html");
    await page.locator(".settings-tab[data-tab='flows']").click();
    // Auto-dismiss the confirm() with cancel — that should suppress the
    // outgoing delete_flow call.
    page.once("dialog", (dialog) => dialog.dismiss());
    await page.locator("#flows-list button[data-action='delete']").click();
    const recordedInvokes = await page.evaluate(() => window.__mockInvokeCalls);
    const deleteCalls = recordedInvokes.filter((call) => call.cmd === "delete_flow");
    expect(deleteCalls.length).toBe(0);
  });
});
