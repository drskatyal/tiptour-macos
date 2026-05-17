// Overlay webview. Drives the cursor SVG via the mocked event bus and
// asserts that:
//   - `overlay/cursor_fly_to` updates the cursor's transform within
//     200ms of the event firing
//   - `overlay/response_show` makes the bubble visible

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

test.describe("overlay webview", () => {
  test("cursor_fly_to updates the cursor SVG transform", async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/overlay.html");
    // Give the module's `await listen(...)` calls time to register.
    await page.waitForFunction(() => {
      // Two listen calls happen at module top-level — wait until both
      // have hit the mock invoke recorder so we know the callbacks
      // are actually subscribed before we fire events.
      return window.__mockInvokeCalls.filter(
        (call) => call.cmd === "plugin:event|listen",
      ).length >= 2;
    });
    await page.evaluate(() => {
      window.__mockEmit("overlay/cursor_fly_to", { x: 240, y: 380 });
    });
    // The overlay applies `translate(x-6, y-4)` so the tip aligns.
    await expect
      .poll(
        async () =>
          await page.evaluate(() => {
            const cursor = document.getElementById("cursor-svg");
            return cursor?.style.transform ?? "";
          }),
        { timeout: 200 },
      )
      .toBe("translate(234px, 376px)");
  });

  test("response_show event makes the bubble visible with the given text", async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/overlay.html");
    await page.waitForFunction(() => {
      return window.__mockInvokeCalls.filter(
        (call) => call.cmd === "plugin:event|listen",
      ).length >= 2;
    });
    await page.evaluate(() => {
      window.__mockEmit("overlay/response_show", {
        text: "Opening Calendar.",
        anchorX: 100,
        anchorY: 100,
      });
    });
    const bubble = page.locator("#response-bubble");
    await expect(bubble).toBeVisible();
    await expect(page.locator("#response-text")).toHaveText("Opening Calendar.");
  });
});
