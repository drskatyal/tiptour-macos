// Playwright config for the TipTour Tauri 2 frontend.
//
// We don't launch the real Tauri shell — the runtime needs WKWebView /
// WebView2 / a real OS event loop that the Linux CI sandbox doesn't
// have. Instead we serve the production `dist/` bundle through a tiny
// static file server and inject a mocked `window.__TAURI_INTERNALS__`
// before any module evaluates. Each spec sets up per-command mocks and
// asserts the resulting DOM state — covering panel/overlay/settings/
// indicators event handlers, tab navigation, form validation, and
// button enablement.

import { defineConfig } from "@playwright/test";

const STATIC_BASE_URL = "http://127.0.0.1:4173";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  // No flaky-test retries — every test is hermetic against mocked
  // invoke/listen. A failure here means the asserted contract drifted.
  retries: 0,
  // Single worker keeps stdout readable; the static file server is
  // tiny and shared across all tests via webServer below.
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: STATIC_BASE_URL,
    // Headless Chromium runs identically on every OS — no need for a
    // matrix here.
    browserName: "chromium",
    headless: true,
    // Capture a trace on the first retry only; with retries: 0 this
    // effectively means "trace on failure" if the harness retries.
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
  },
  webServer: {
    // Serve the already-built dist/ folder. `npm run build` produces
    // it and CI runs that before kicking off Playwright.
    command: "npm run serve:dist",
    url: STATIC_BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
