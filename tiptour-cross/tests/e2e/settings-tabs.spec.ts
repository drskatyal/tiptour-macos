// Settings webview navigation. Asserts every one of the 9 visible
// tabs renders without throwing into the catch block (which would
// surface as a `flag-banner` div) and without writing console.error.

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

const ALL_TAB_NAMES = [
  "general",
  "commands",
  "flows",
  "recordings",
  "capabilities",
  "indicators",
  "memory",
  "tasks",
  "about",
] as const;

test.describe("settings tabs", () => {
  test("each of the 9 tabs renders without console.error or fallback banner", async ({ page }) => {
    // Generic broad mock: every command we know each tab calls returns
    // an empty/sensible default so no tab dies on missing handlers.
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
        list_custom_voice_commands: () => [],
        list_flows: () => [],
        list_demonstrations: () => [],
        list_capability_index_summaries: () => [],
        list_capabilities: () => [],
        get_destructive_keywords: () => [],
        get_indicators_settings: () => ({
          edge: "right",
          density: "normal",
          maxVisible: 4,
          autoDismissMs: 5000,
          sound: "silent",
          enabledKinds: ["step", "flow", "voice", "app", "screenshot", "error"],
        }),
        list_memories: () => [],
        list_top_importance_memories: () => [],
        list_tasks: () => [],
        list_subagents: () => [],
        get_app_metadata: () => ({
          appVersion: "0.0.1",
          tauriVersion: "2",
          osName: "linux",
          dataFolderPath: "/tmp/fake",
          buildHash: "deadbeef",
        }),
        list_discovered_apps: () => [],
      },
    });

    const consoleErrors: string[] = [];
    page.on("console", (consoleMessage) => {
      if (consoleMessage.type() === "error") {
        consoleErrors.push(consoleMessage.text());
      }
    });
    await page.goto("/settings.html");
    // Wait for the default General tab to render so we know boot succeeded.
    await expect(page.locator(".settings-tab[data-tab='general']")).toHaveAttribute(
      "data-active",
      "true",
    );
    for (const tabName of ALL_TAB_NAMES) {
      await page.locator(`.settings-tab[data-tab='${tabName}']`).click();
      await expect(page.locator(`.settings-tab[data-tab='${tabName}']`)).toHaveAttribute(
        "data-active",
        "true",
      );
      // The pane is replaced with `<div class="flag-banner">Failed to
      // render ...` on a thrown render error. If that ever appears the
      // tab is broken — fail loud.
      const fallbackBannerCount = await page.locator(".flag-banner").count();
      expect(
        fallbackBannerCount,
        `tab ${tabName} fell back to its error banner`,
      ).toBe(0);
    }
    expect(
      consoleErrors,
      "console.error must not fire while switching tabs",
    ).toEqual([]);
  });
});
