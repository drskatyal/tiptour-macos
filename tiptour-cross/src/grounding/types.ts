// Mirror of src-tauri/src/grounding/types.rs. Field names are camelCase
// because the Rust side uses #[serde(rename_all = "camelCase")] so the
// JSON shape that crosses the Tauri bridge already matches TS idioms.

export interface TargetApp {
  processId: number;
  bundleIdentifier: string | null;
  executablePath: string | null;
  displayName: string | null;
  fileVersion: string | null;
}

export interface KeyChord {
  raw: string;
  modifiers: string[];
  key: string;
}

export interface ShortcutBinding {
  menuPath: string[];
  label: string;
  accelerator: KeyChord;
}

export interface ShortcutIndex {
  applicationIdentifier: string;
  executablePath: string | null;
  fileVersion: string | null;
  capturedAtUnixMs: number;
  bindings: ShortcutBinding[];
}

export type ResolvedTarget =
  | { kind: "coordinate"; x: number; y: number }
  | { kind: "shortcut"; chord: KeyChord };
