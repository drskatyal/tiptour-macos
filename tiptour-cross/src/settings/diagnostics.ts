// Diagnostics tab: runs every probe in src-tauri/src/diagnostics.rs
// and renders a uniform pass/fail table. Used to triage "why isn't
// my key working?" reports — also exposes a copy-to-clipboard JSON
// dump the user can paste into a bug report.

import { invoke } from "@tauri-apps/api/core";

type DiagnosticReport = {
  name: string;
  category: string;
  ok: boolean;
  detail: string;
};

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export async function renderDiagnosticsTab(paneElement: HTMLElement): Promise<void> {
  paneElement.innerHTML = `
    <h2>Diagnostics</h2>
    <p class="settings-lede">
      Sanity-checks for the wired-up pieces. Run this when something
      isn't working — it's faster than reading logs.
    </p>
    <div class="diagnostics-toolbar">
      <button class="settings-button" id="diagnostics-run">Run probes</button>
      <button class="settings-button" id="diagnostics-copy">Copy bundle</button>
    </div>
    <div class="diagnostics-table" id="diagnostics-table">
      <p class="sessions-empty">Click "Run probes" to start.</p>
    </div>
  `;

  const runButton = paneElement.querySelector<HTMLButtonElement>("#diagnostics-run")!;
  const copyButton = paneElement.querySelector<HTMLButtonElement>("#diagnostics-copy")!;
  const tableElement = paneElement.querySelector<HTMLDivElement>("#diagnostics-table")!;

  let lastReports: DiagnosticReport[] = [];

  async function runProbes() {
    runButton.disabled = true;
    runButton.textContent = "Running…";
    tableElement.innerHTML = `<div class="settings-loading-row"></div>`;
    try {
      const reports = (await invoke<DiagnosticReport[]>("run_diagnostics")) ?? [];
      lastReports = reports;
      tableElement.innerHTML = `
        <table class="diagnostics-rows">
          <thead>
            <tr><th>Check</th><th>Category</th><th>Result</th><th>Detail</th></tr>
          </thead>
          <tbody>
            ${reports
              .map(
                (report) => `
                  <tr class="${report.ok ? "diagnostic-ok" : "diagnostic-fail"}">
                    <td>${escapeHtml(report.name)}</td>
                    <td>${escapeHtml(report.category)}</td>
                    <td>${report.ok ? "✓ pass" : "✗ fail"}</td>
                    <td>${escapeHtml(report.detail)}</td>
                  </tr>
                `,
              )
              .join("")}
          </tbody>
        </table>
      `;
    } catch (error) {
      tableElement.innerHTML = `<p class="sessions-empty">Probe run failed: ${escapeHtml(String(error))}</p>`;
    } finally {
      runButton.disabled = false;
      runButton.textContent = "Run probes";
    }
  }

  copyButton.addEventListener("click", async () => {
    if (lastReports.length === 0) {
      copyButton.textContent = "Run probes first";
      setTimeout(() => (copyButton.textContent = "Copy bundle"), 1500);
      return;
    }
    try {
      await navigator.clipboard.writeText(JSON.stringify(lastReports, null, 2));
      copyButton.textContent = "Copied!";
    } catch {
      copyButton.textContent = "Clipboard blocked";
    }
    setTimeout(() => (copyButton.textContent = "Copy bundle"), 1500);
  });

  runButton.addEventListener("click", () => void runProbes());

  // Auto-run on open so the user sees something immediately.
  await runProbes();
}
