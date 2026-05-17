// Side-edge tab — single button that emits `panel_tab_clicked`,
// which the Rust setup hook wires up to restore the panel and hide
// this tab.

import { emit } from "@tauri-apps/api/event";

console.info("[panel-tab] webview loaded");

const buttonElement = document.getElementById(
  "panel-tab-button",
) as HTMLButtonElement | null;
if (!buttonElement) {
  console.error("[panel-tab] button element missing from DOM");
}

buttonElement?.addEventListener("click", async () => {
  console.info("[panel-tab] click → emitting panel_tab_clicked");
  try {
    await emit("panel_tab_clicked");
  } catch (emitError) {
    console.error("[panel-tab] emit failed:", emitError);
  }
  // Belt-and-suspenders fallback — also drive the windows directly
  // from here in case the Rust event listener races or doesn't fire.
  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const panelWindow = await WebviewWindow.getByLabel("panel");
    if (panelWindow) {
      await panelWindow.show();
      await panelWindow.setFocus();
    }
    const selfTabWindow = await WebviewWindow.getByLabel("panel-tab");
    if (selfTabWindow) {
      await selfTabWindow.hide();
    }
  } catch (windowError) {
    console.error("[panel-tab] direct window swap failed:", windowError);
  }
});
