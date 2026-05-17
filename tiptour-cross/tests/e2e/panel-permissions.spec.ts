// Permissions yellow box behavior. The panel:
//   - shows the permissions section when at least one of accessibility
//     / screen-recording is missing
//   - hides it when both are granted
//   - exposes a Grant button per row that calls
//     `request_*_permission`
//   - polls screen-recording for a false→true transition and surfaces
//     a "restart required" banner ONLY on that transition (mirroring
//     the macOS TCC per-pid caching reality)

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

test.describe("panel permissions", () => {
  test("shows permissions box with both rows when both checks return false", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        // Force-mac so the panel's `if (!isMac) return` short-circuit
        // doesn't suppress the permission probes. We fake the platform
        // string below before the page boots.
        check_accessibility_permission: () => false,
        check_screen_recording_permission: () => false,
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
    // Pretend to be macOS so the permissions polling code path runs.
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", {
        value: "MacIntel",
        configurable: true,
      });
    });
    await page.goto("/index.html");
    const permissionsSection = page.locator("#permissions-section");
    await expect(permissionsSection).toBeVisible();
    await expect(page.locator("#ax-permission-row")).toBeVisible();
    await expect(page.locator("#screen-permission-row")).toBeVisible();
  });

  test("hides permissions box when both checks return true", async ({ page }) => {
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
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", {
        value: "MacIntel",
        configurable: true,
      });
    });
    await page.goto("/index.html");
    await expect(page.locator("#permissions-section")).toBeHidden();
  });

  test("clicking Grant Accessibility invokes request_accessibility_permission", async ({
    page,
  }) => {
    await installTauriMock(page, {
      handlers: {
        check_accessibility_permission: () => false,
        check_screen_recording_permission: () => false,
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
        request_accessibility_permission: () => undefined,
      },
    });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", {
        value: "MacIntel",
        configurable: true,
      });
    });
    await page.goto("/index.html");
    await page.locator("#grant-accessibility").click();
    const recordedInvokes = await page.evaluate(() => window.__mockInvokeCalls);
    const sawRequest = recordedInvokes.some(
      (call) => call.cmd === "request_accessibility_permission",
    );
    expect(sawRequest).toBe(true);
  });

  test("restart banner appears only after the false→true poll transition", async ({ page }) => {
    // Permission starts false, click Grant, then flips true. The panel
    // should poll, detect the transition, and surface the banner.
    let screenRecordingPermissionState = false;
    await installTauriMock(page, {
      handlers: {
        check_accessibility_permission: () => true,
        check_screen_recording_permission: () => false,
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
        request_screen_recording_permission: () => undefined,
      },
    });
    void screenRecordingPermissionState;
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", {
        value: "MacIntel",
        configurable: true,
      });
    });
    await page.goto("/index.html");
    // The banner must NOT be visible up front.
    const restartBanner = page.locator("#restart-required-banner");
    await expect(restartBanner).toBeHidden();
    // Now flip the handler so the next probe returns "granted", then
    // click Grant. The poller fires every 1s and should pick up the
    // change within a few seconds.
    // Two-phase mock: while `__simulateGranted` is false we report "not
    // granted" (so the click-handler's snapshot reflects the initial
    // state). After the click, the spec flips the flag so the poller's
    // next tick sees the OS-side false→true transition.
    await page.evaluate(() => {
      (window as unknown as { __simulateGranted: boolean }).__simulateGranted = false;
      window.__mockSetHandler("check_screen_recording_permission", () => {
        return (window as unknown as { __simulateGranted: boolean }).__simulateGranted;
      });
    });
    await page.locator("#grant-screen-recording").click();
    // Now flip the flag — the poller fires every 1s and will detect
    // the transition on its next tick.
    await page.evaluate(() => {
      (window as unknown as { __simulateGranted: boolean }).__simulateGranted = true;
    });
    // Poller fires every 1000ms; give it ~3s of wall time to detect.
    await expect(restartBanner).toBeVisible({ timeout: 5_000 });
  });
});
