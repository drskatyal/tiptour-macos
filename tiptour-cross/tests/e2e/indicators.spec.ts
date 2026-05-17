// Indicators rail webview. Drives the side-of-screen pill stack via
// `indicator_event` payloads and asserts:
//   - one pill per kind appears
//   - hover expands a normal-density pill (adds .indicator-pill-expanded)
//   - clicking a pill triggers its leaving animation + DOM removal
//   - autoDismissMs causes timed dismissal

import { test, expect } from "@playwright/test";
import { installTauriMock } from "./_helpers/installTauriMock";

interface IndicatorEventPayload {
  kind: "step" | "flow" | "voice" | "app" | "screenshot" | "error";
  title: string;
  subtitle: string | null;
  sourceId: string | null;
  autoDismissMs: number;
  maxVisible: number;
  density: "compact" | "normal" | "verbose";
  sound: "silent" | "soft-tick";
}

function makeIndicatorPayload(
  overrides: Partial<IndicatorEventPayload>,
): IndicatorEventPayload {
  return {
    kind: "step",
    title: "Step done",
    subtitle: null,
    sourceId: null,
    autoDismissMs: 5_000,
    maxVisible: 8,
    density: "normal",
    sound: "silent",
    ...overrides,
  };
}

test.describe("indicators webview", () => {
  test("emitting one event of each kind appends a pill per kind to the stack", async ({
    page,
  }) => {
    await installTauriMock(page, {
      handlers: {
        indicators_set_click_through: () => undefined,
      },
    });
    await page.goto("/indicators.html");
    // Wait for the listener to install.
    await page.waitForFunction(() => {
      return window.__mockInvokeCalls.some(
        (call) =>
          call.cmd === "plugin:event|listen" &&
          (call.args as { event?: string } | undefined)?.event === "indicator_event",
      );
    });
    const allKinds: IndicatorEventPayload["kind"][] = [
      "step",
      "flow",
      "voice",
      "app",
      "screenshot",
      "error",
    ];
    await page.evaluate((kinds) => {
      for (const kind of kinds) {
        const payload = {
          kind,
          title: `${kind} pill`,
          subtitle: null,
          sourceId: null,
          autoDismissMs: 0, // forever — don't race against test
          maxVisible: 32,
          density: "normal",
          sound: "silent",
        };
        window.__mockEmit("indicator_event", payload);
      }
    }, allKinds);
    await expect(page.locator(".indicator-pill")).toHaveCount(allKinds.length);
    for (const kind of allKinds) {
      await expect(page.locator(`.indicator-pill[data-kind="${kind}"]`)).toHaveCount(1);
    }
  });

  test("hover on a normal-density pill adds the expanded class", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        indicators_set_click_through: () => undefined,
      },
    });
    await page.goto("/indicators.html");
    await page.waitForFunction(() => {
      return window.__mockInvokeCalls.some(
        (call) =>
          call.cmd === "plugin:event|listen" &&
          (call.args as { event?: string } | undefined)?.event === "indicator_event",
      );
    });
    await page.evaluate((payload) => window.__mockEmit("indicator_event", payload), {
      ...makeIndicatorPayload({ autoDismissMs: 0, density: "normal" }),
    });
    const pillLocator = page.locator(".indicator-pill");
    await expect(pillLocator).toHaveCount(1);
    await pillLocator.hover();
    await expect(pillLocator).toHaveClass(/indicator-pill-expanded/);
  });

  test("clicking a pill removes it from the DOM after the leave animation", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        indicators_set_click_through: () => undefined,
      },
    });
    await page.goto("/indicators.html");
    await page.waitForFunction(() => {
      return window.__mockInvokeCalls.some(
        (call) =>
          call.cmd === "plugin:event|listen" &&
          (call.args as { event?: string } | undefined)?.event === "indicator_event",
      );
    });
    await page.evaluate((payload) => window.__mockEmit("indicator_event", payload), {
      ...makeIndicatorPayload({ autoDismissMs: 0 }),
    });
    await page.locator(".indicator-pill").click();
    // The leave animation removes the node 240ms later.
    await expect(page.locator(".indicator-pill")).toHaveCount(0, { timeout: 1_000 });
  });

  test("autoDismissMs causes the pill to disappear on its own", async ({ page }) => {
    await installTauriMock(page, {
      handlers: {
        indicators_set_click_through: () => undefined,
      },
    });
    await page.goto("/indicators.html");
    await page.waitForFunction(() => {
      return window.__mockInvokeCalls.some(
        (call) =>
          call.cmd === "plugin:event|listen" &&
          (call.args as { event?: string } | undefined)?.event === "indicator_event",
      );
    });
    await page.evaluate((payload) => window.__mockEmit("indicator_event", payload), {
      ...makeIndicatorPayload({ autoDismissMs: 200 }),
    });
    await expect(page.locator(".indicator-pill")).toHaveCount(1);
    // Auto-dismiss at 200ms + 240ms exit animation; allow generous slack.
    await expect(page.locator(".indicator-pill")).toHaveCount(0, { timeout: 2_000 });
  });
});
