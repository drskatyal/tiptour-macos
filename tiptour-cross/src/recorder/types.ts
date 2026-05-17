// Mirror of src-tauri/src/recorder/types.rs. Field names are camelCase
// because the Rust side uses #[serde(rename_all = "camelCase")] so the
// JSON shape that crosses the Tauri bridge already matches TS idioms.

export type InputEvent =
  | { kind: "keyDown"; keyCode: number; keyName: string }
  | { kind: "keyUp"; keyCode: number; keyName: string }
  | { kind: "mouseClick"; button: string; x: number; y: number }
  | { kind: "mouseMove"; x: number; y: number }
  | { kind: "scroll"; deltaX: number; deltaY: number };

export interface FocusedElementFingerprint {
  role: string | null;
  name: string | null;
  automationId: string | null;
  controlType: string | null;
  isPassword: boolean;
}

export interface StateSnapshot {
  foregroundAppBundleIdentifier: string | null;
  foregroundAppExecutablePath: string | null;
  foregroundAppDisplayName: string | null;
  foregroundWindowTitle: string | null;
  uiaTreeFingerprint: string | null;
  focusedElement: FocusedElementFingerprint | null;
  selectedText: string | null;
}

export interface TraceEntry {
  timestampUnixMs: number;
  event: InputEvent;
  snapshotBefore: StateSnapshot | null;
  snapshotAfter: StateSnapshot | null;
}

export type RecordingMode = "passive" | "demonstration";

export interface Demonstration {
  id: string;
  title: string;
  narrationAudioPath: string | null;
  trace: TraceEntry[];
  createdAtUnixMs: number;
}

export interface DemonstrationSummary {
  id: string;
  title: string;
  createdAtUnixMs: number;
  traceEntryCount: number;
  hasNarrationAudio: boolean;
}

export interface WorkflowPattern {
  occurrences: number;
  meanIntervalMs: number;
  suggestedName: string;
  representativeEventKinds: string[];
  kGramLength: number;
}
