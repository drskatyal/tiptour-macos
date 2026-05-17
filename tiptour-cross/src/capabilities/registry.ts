// Thin wrapper around the Tauri commands exposed in
// src-tauri/src/capabilities/mod.rs. Keeping the IPC boundary in one
// place makes it easy to mock for tests and swap transports later.

import { invoke } from "@tauri-apps/api/core";

import type {
  Capability,
  ExploreSummary,
  ResolvedPlan,
  ToolSchema,
} from "./types";

export async function exploreApp(appIdentifier: string): Promise<ExploreSummary> {
  return invoke<ExploreSummary>("explore_app", { appIdentifier });
}

export async function listCapabilities(appIdentifier: string): Promise<Capability[]> {
  return invoke<Capability[]>("list_capabilities", { appIdentifier });
}

export async function retrieveTools(query: string, topK: number): Promise<ToolSchema[]> {
  return invoke<ToolSchema[]>("retrieve_tools", { query, topK });
}

export async function invokeCapability(
  toolId: string,
  params: Record<string, unknown>,
): Promise<ResolvedPlan> {
  return invoke<ResolvedPlan>("invoke_capability", { toolId, params });
}
