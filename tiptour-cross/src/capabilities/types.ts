// Mirror of src-tauri/src/capabilities/types.rs. Field names are camelCase
// because the Rust side uses #[serde(rename_all = "camelCase")] so the
// JSON shape that crosses the Tauri bridge matches TS idioms.

export type SafetyClassification = "safe" | "destructive" | "unknown";

export type Action =
  | { kind: "click"; elementName: string; elementPath: string[] }
  | { kind: "shortcut"; chordRaw: string }
  | { kind: "type"; targetElementName: string; textParameterName: string }
  | { kind: "scroll"; direction: string; magnitude: number }
  | { kind: "navigate"; menuPath: string[] };

export interface StateNode {
  stateId: string;
  foregroundWindowTitle: string | null;
  elementCount: number;
  discoveredAtUnixMs: number;
}

export interface Edge {
  fromStateId: string;
  toStateId: string;
  action: Action;
  safety: SafetyClassification;
  isOneWay: boolean;
}

export interface ParameterSchema {
  // JSON-shaped param definition keyed by parameter name. Each value is a
  // small object with "type" and "description" fields so it can be handed
  // straight to Gemini's function-calling schema.
  properties: Record<string, unknown>;
  required: string[];
}

export interface Capability {
  capabilityId: string;
  canonicalName: string;
  description: string;
  parameterSchema: ParameterSchema;
  safety: SafetyClassification;
  replayActions: Action[];
  keywords: string[];
}

export interface CapabilityGraph {
  appIdentifier: string;
  appVersion: string | null;
  nodes: StateNode[];
  edges: Edge[];
  capturedAtUnixMs: number;
}

export interface ToolSchema {
  toolId: string;
  name: string;
  description: string;
  parameterSchema: ParameterSchema;
  safety: SafetyClassification;
}

export interface ResolvedPlan {
  toolId: string;
  actions: Action[];
  safety: SafetyClassification;
}

export interface ExploreSummary {
  appIdentifier: string;
  nodesDiscovered: number;
  edgesDiscovered: number;
  capabilitiesExtracted: number;
  stoppedReason: string;
  elapsedMs: number;
}
