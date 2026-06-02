// Shared types for the split chat layout (chips + Computer pane).
// Structurally compatible with ActivityTimelineCard from ChatPage.tsx so
// existing card arrays satisfy these interfaces without conversion.

export interface ChatPayloadView {
  kind: "json" | "text";
  badgeLabel: string;
  headerLabel: string;
  preview: string;
  body: string;
  lineCount: number;
}

export type SurfaceStatus = "pending" | "running" | "done" | "error" | "waiting";

export type SurfaceFallback = "generic-artifact" | "text" | "json" | "activity" | "trace";

export interface SurfacePayload {
  role: string;
  contentType: string;
  text?: string;
  json?: unknown;
  uri?: string;
  path?: string;
  preview?: string;
  metadata?: Record<string, unknown>;
}

export interface SurfaceArtifact {
  id: string;
  role: string;
  contentType: string;
  label?: string;
  text?: string;
  json?: unknown;
  uri?: string;
  path?: string;
  preview?: string;
  metadata?: Record<string, unknown>;
}

export interface SurfaceDescriptor {
  protocolVersion: 1;
  renderer: {
    id: string;
    version: number;
    fallback: SurfaceFallback;
  };
  call: {
    runId?: string;
    callId: string;
    sequence?: number;
    parentStepId?: string;
  };
  tool?: {
    id: string;
    displayName?: string;
  };
  status: SurfaceStatus;
  title?: string;
  capabilities?: string[];
  input?: SurfacePayload[];
  output?: SurfacePayload[];
  artifacts?: SurfaceArtifact[];
  timing?: {
    startedAt?: string;
    completedAt?: string;
    updatedAt?: string;
  };
  error?: {
    code?: string;
    message: string;
    detail?: unknown;
  };
}

export interface ChatStepCard {
  id: string;
  index: number;
  stepType: string;
  rawTitle: string;
  tone: string;
  kind: string;
  label: string;
  detail: string;
  detailFull: string;
  summary: string;
  rawDetailFull: string;
  traceJson?: string;
  payloadView: ChatPayloadView | null;
  isHeartbeat: boolean;
  time: string;
  surface?: SurfaceDescriptor | null;
}

export interface ComputerPaneFile {
  path: string;
  displayPath?: string;
  content: string;
}

export type ComputerViewKind = string;

export type ComputerPaneTab = "computer" | "files" | "activity";

export type ChipStatus = "running" | "done" | "issue" | "idle";
