// Shared theme application helper used by every webview. Reads the
// persisted theme from app_settings on boot and listens for the
// `theme_changed` Tauri event so a settings change in one webview
// propagates to all the others without a reload.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type ThemeChoice = "auto" | "dark" | "light";

interface AppSettingsShape {
  theme?: string;
}

function applyTheme(themeValue: string): void {
  const normalized: ThemeChoice =
    themeValue === "light" || themeValue === "dark" ? themeValue : "auto";
  document.documentElement.dataset.theme = normalized;
}

export async function installThemeBridge(): Promise<void> {
  try {
    const settings = await invoke<AppSettingsShape>("get_app_settings");
    applyTheme(settings.theme ?? "auto");
  } catch (loadError) {
    console.warn("[theme] could not load initial theme:", loadError);
    applyTheme("auto");
  }
  await listen<string>("theme_changed", (event) => {
    applyTheme(event.payload ?? "auto");
  });
}
