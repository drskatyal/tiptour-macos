// Thin wrappers around the Tauri commands exposed by the Rust recorder
// module. The frontend should always go through this file rather than
// calling invoke() directly so command-name typos surface as TS errors.

import { invoke } from "@tauri-apps/api/core";

import type {
  Demonstration,
  DemonstrationSummary,
  WorkflowPattern,
} from "./types";

export async function isRecordingEnabled(): Promise<boolean> {
  return await invoke<boolean>("is_recording_enabled");
}

export async function setRecordingEnabled(enabled: boolean): Promise<void> {
  await invoke("set_recording_enabled", { enabled });
}

export async function startPassiveRecording(): Promise<void> {
  await invoke("start_passive_recording");
}

export async function stopPassiveRecording(): Promise<void> {
  await invoke("stop_passive_recording");
}

export async function startDemonstration(title: string): Promise<string> {
  return await invoke<string>("start_demonstration", { title });
}

export async function stopDemonstration(): Promise<Demonstration> {
  return await invoke<Demonstration>("stop_demonstration");
}

export async function appendNarrationAudioChunk(
  pcm16LittleEndianBytes: Uint8Array,
): Promise<void> {
  await invoke("append_narration_audio_chunk", {
    pcm: Array.from(pcm16LittleEndianBytes),
  });
}

export async function listDemonstrations(): Promise<DemonstrationSummary[]> {
  return await invoke<DemonstrationSummary[]>("list_demonstrations");
}

export async function loadDemonstration(
  demonstrationId: string,
): Promise<Demonstration> {
  return await invoke<Demonstration>("load_demonstration", {
    demonstrationId,
  });
}

export async function minePatterns(
  minOccurrences: number,
): Promise<WorkflowPattern[]> {
  return await invoke<WorkflowPattern[]>("mine_patterns", { minOccurrences });
}
