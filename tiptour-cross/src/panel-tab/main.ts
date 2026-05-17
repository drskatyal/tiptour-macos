// Side-edge tab — single button that emits `panel_tab_clicked`,
// which the panel + Rust setup wire up to restore the panel and
// hide this tab.

import { emit } from "@tauri-apps/api/event";

const buttonElement = document.getElementById(
  "panel-tab-button",
) as HTMLButtonElement | null;

buttonElement?.addEventListener("click", () => {
  void emit("panel_tab_clicked");
});
