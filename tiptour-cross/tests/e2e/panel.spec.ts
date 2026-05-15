// Panel webview — root index.html. Asserts the visible pieces a brand
// new user touches first: API key field, Start button validation, mode
// dropdown persistence, recording opt-in toggle.

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

test.describe("panel webview", () => {
  test("renders with empty api key field by default and shows error when starting without a key", async ({
    page,
  }) => {
    // No `get_api_key` handler → returns undefined → the panel leaves
    // the input empty, which is the contract we want to assert on.
    await installTauriMock(page, {
      handlers: {
        // Permission probes default to "granted" so the permissions
        // box stays hidden and doesn't pollute this assertion.
        check_accessibility_permission: () => true,
        check_screen_recording_permission: () => true,
        get_app_settings: () => ({
          schemaVersion: 1,
          geminiVoice: "Kore",
          geminiModel: "gemini-3.1-flash-live-preview",
          pushToTalkChord: "Alt+X",
        }),
        get_operating_mode: () => "autopilot",
        is_recording_enabled: () => false,
        is_listener_enabled: () => false,
        list_flows: () => [],
        list_tasks: () => [],
        get_api_key: () => null,
      },
    });
    await page.goto("/index.html");
    await expect(page.locator("#api-key-input")).toHaveValue("");
    // Click Start without a key — should surface an error banner.
    await page.locator("#start-listening").click();
    const errorBanner = page.locator("#error-banner");
    await expect(errorBanner).toBeVisible();
    await expect(errorBanner).toContainText("Paste a Gemini API key");
  });

  test("recording opt-in toggle persists via set_recording_enabled", async ({ page }) => {
    let recordingFlagOnDisk = false;
    await installTauriMock(page, {
      handlers: {
        check_accessibility_permission: () => true,
        check_screen_recording_permission: () => true,
        get_app_settings: () => ({
          schemaVersion: 1,
          geminiVoice: "Kore",
          geminiModel: "gemini-3.1-flash-live-preview",
          pushToTalkChord: "Alt+X",
        }),
        get_operating_mode: () => "autopilot",
        list_flows: () => [],
        is_recording_enabled: () => false,
        is_listener_enabled: () => false,
        get_api_key: () => null,
      },
    });
    await page.goto("/index.html");
    // Override the set_recording_enabled handler post-load so we can
    // capture the value the click sends.
    await page.evaluate(() => {
      window.__mockSetHandler("set_recording_enabled", (args) => {
        // Stash the recieved arg on a window prop so the spec can
        // assert on it without relying on call ordering.
        (window as unknown as { __lastRecordingFlag: unknown }).__lastRecordingFlag =
          args?.enabled;
        return undefined;
      });
    });
    const recordingToggle = page.locator("#recording-opt-in-toggle");
    await recordingToggle.check();
    const observed = await page.evaluate(
      () => (window as unknown as { __lastRecordingFlag: unknown }).__lastRecordingFlag,
    );
    expect(observed).toBe(true);
    // Avoid lint complaint about unused outer var; document why it's
    // present (mirrors what the Rust backend would persist).
    void recordingFlagOnDisk;
  });

  test("mode dropdown writes new mode through set_operating_mode", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        check_accessibility_permission: () => true,
        check_screen_recording_permission: () => true,
        get_app_settings: () => ({
          schemaVersion: 1,
          geminiVoice: "Kore",
          geminiModel: "gemini-3.1-flash-live-preview",
          pushToTalkChord: "Alt+X",
        }),
        // Pre-set autopilot so we can assert the dropdown reflects it
        // before the user changes it.
        get_operating_mode: () => "autopilot",
        list_flows: () => [],
        is_recording_enabled: () => false,
        is_listener_enabled: () => false,
        get_api_key: () => null,
      },
    });
    await page.goto("/index.html");
    const modeSelect = page.locator("#mode-select");
    await expect(modeSelect).toHaveValue("autopilot");
    await modeSelect.selectOption("teaching");
    // Walk the recorded invokes to find the most recent set_operating_mode.
    const recordedInvokes = await page.evaluate(() => window.__mockInvokeCalls);
    const lastSetOperatingMode = [...recordedInvokes]
      .reverse()
      .find((call) => call.cmd === "set_operating_mode");
    expect(lastSetOperatingMode?.args).toMatchObject({ mode: "teaching" });
  });
});
