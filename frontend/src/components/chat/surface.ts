import type {
  ChatStepCard,
  SurfaceArtifact,
  SurfaceDescriptor,
  SurfaceFallback,
  SurfacePayload,
  SurfaceStatus,
} from "./types";

export const AGENTARK_RENDERERS = {
  TERMINAL: "agentark.terminal.transcript.v1",
  BROWSER: "agentark.browser.reader.v1",
  SEARCH: "agentark.search.results.v1",
  FILE: "agentark.file.editor.v1",
  IMAGE: "agentark.artifact.image.v1",
  DEPLOY: "agentark.app.deploy.v1",
  GENERIC: "agentark.artifact.generic.v1",
  WORKING: "agentark.working.v1",
} as const;

const FALLBACK_RENDERER_BY_MODE: Record<SurfaceFallback, string> = {
  "generic-artifact": AGENTARK_RENDERERS.GENERIC,
  text: AGENTARK_RENDERERS.GENERIC,
  json: AGENTARK_RENDERERS.GENERIC,
  activity: AGENTARK_RENDERERS.WORKING,
  trace: AGENTARK_RENDERERS.GENERIC,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function parseRecord(raw: unknown): Record<string, unknown> | null {
  if (isRecord(raw)) return raw;
  const text = str(raw).trim();
  if (!text || text[0] !== "{") return null;
  try {
    const parsed = JSON.parse(text) as unknown;
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function payloadRecord(card: ChatStepCard): Record<string, unknown> | null {
  return (
    parseRecord(card.payloadView?.body) ||
    parseRecord(card.rawDetailFull) ||
    parseRecord(card.detailFull)
  );
}

function normalizeStatus(value: unknown): SurfaceStatus {
  const status = str(value).trim().toLowerCase();
  if (
    status === "pending" ||
    status === "running" ||
    status === "done" ||
    status === "error" ||
    status === "waiting"
  ) {
    return status;
  }
  return "pending";
}

function normalizeFallback(value: unknown): SurfaceFallback {
  const fallback = str(value).trim();
  if (
    fallback === "generic-artifact" ||
    fallback === "text" ||
    fallback === "json" ||
    fallback === "activity" ||
    fallback === "trace"
  ) {
    return fallback;
  }
  return "generic-artifact";
}

function normalizePayloadList(value: unknown): SurfacePayload[] {
  const raw = Array.isArray(value) ? value : [];
  return raw
    .map((item): SurfacePayload | null => {
      if (!isRecord(item)) return null;
      const role = str(item.role).trim() || "payload";
      const contentType = str(item.contentType).trim() || "application/octet-stream";
      return {
        role,
        contentType,
        text: str(item.text) || undefined,
        json: item.json,
        uri: str(item.uri) || undefined,
        path: str(item.path) || undefined,
        preview: str(item.preview) || undefined,
        metadata: isRecord(item.metadata) ? item.metadata : undefined,
      } satisfies SurfacePayload;
    })
    .filter((item): item is SurfacePayload => Boolean(item));
}

function normalizeArtifactList(value: unknown): SurfaceArtifact[] {
  const raw = Array.isArray(value) ? value : [];
  return raw
    .map((item, index): SurfaceArtifact | null => {
      if (!isRecord(item)) return null;
      const role = str(item.role).trim() || "artifact";
      const contentType = str(item.contentType).trim() || "application/octet-stream";
      return {
        id: str(item.id).trim() || `artifact-${index}`,
        role,
        contentType,
        label: str(item.label) || undefined,
        text: str(item.text) || undefined,
        json: item.json,
        uri: str(item.uri) || undefined,
        path: str(item.path) || undefined,
        preview: str(item.preview) || undefined,
        metadata: isRecord(item.metadata) ? item.metadata : undefined,
      } satisfies SurfaceArtifact;
    })
    .filter((item): item is SurfaceArtifact => Boolean(item));
}

export function surfaceFromValue(
  value: unknown,
  fallbackCallId = "surface",
): SurfaceDescriptor | null {
  const record = parseRecord(value);
  const surfaceRecord = isRecord(record?.surface) ? record.surface : record;
  if (!isRecord(surfaceRecord)) return null;

  const renderer = isRecord(surfaceRecord.renderer) ? surfaceRecord.renderer : {};
  const rendererId = str(renderer.id).trim();
  if (!rendererId) return null;

  const call = isRecord(surfaceRecord.call) ? surfaceRecord.call : {};
  const tool = isRecord(surfaceRecord.tool) ? surfaceRecord.tool : null;
  const timing = isRecord(surfaceRecord.timing) ? surfaceRecord.timing : null;
  const error = isRecord(surfaceRecord.error) ? surfaceRecord.error : null;
  const callId =
    str(call.callId).trim() ||
    str(record?.__streamKey).trim() ||
    str(record?.stream_key).trim() ||
    fallbackCallId;

  return {
    protocolVersion: 1,
    renderer: {
      id: rendererId,
      version: num(renderer.version) ?? 1,
      fallback: normalizeFallback(renderer.fallback),
    },
    call: {
      runId: str(call.runId) || str(record?.run_id) || undefined,
      callId,
      sequence: num(call.sequence) ?? num(record?.seq),
      parentStepId: str(call.parentStepId) || undefined,
    },
    tool: tool
      ? {
          id: str(tool.id).trim(),
          displayName: str(tool.displayName) || undefined,
        }
      : undefined,
    status: normalizeStatus(surfaceRecord.status),
    title: str(surfaceRecord.title) || undefined,
    capabilities: Array.isArray(surfaceRecord.capabilities)
      ? surfaceRecord.capabilities.map(str).filter(Boolean)
      : undefined,
    input: normalizePayloadList(surfaceRecord.input),
    output: normalizePayloadList(surfaceRecord.output),
    artifacts: normalizeArtifactList(surfaceRecord.artifacts),
    timing: timing
      ? {
          startedAt: str(timing.startedAt) || undefined,
          completedAt: str(timing.completedAt) || undefined,
          updatedAt: str(timing.updatedAt) || undefined,
        }
      : undefined,
    error:
      error && str(error.message)
        ? {
            code: str(error.code) || undefined,
            message: str(error.message),
            detail: error.detail,
          }
        : undefined,
  };
}

export function surfaceFromCard(card: ChatStepCard | null | undefined): SurfaceDescriptor | null {
  if (!card) return null;
  return (
    card.surface ||
    surfaceFromValue(payloadRecord(card), card.id) ||
    surfaceFromValue(card.payloadView?.body, card.id) ||
    surfaceFromValue(card.rawDetailFull, card.id) ||
    surfaceFromValue(card.detailFull, card.id)
  );
}

export function rendererIdForCard(card: ChatStepCard | null | undefined): string {
  const surface = surfaceFromCard(card);
  if (!surface) return AGENTARK_RENDERERS.GENERIC;
  return surface.renderer.id || FALLBACK_RENDERER_BY_MODE[surface.renderer.fallback];
}

export function isRegisteredWorkspaceSurface(card: ChatStepCard): boolean {
  const payload = payloadRecord(card);
  if (str(payload?.kind).trim().toLowerCase() === "turn_completed") {
    return false;
  }
  const surface = surfaceFromCard(card);
  if (!surface) return false;
  if ((surface.tool?.id || "").trim().toLowerCase() === "agent_turn_loop") {
    return false;
  }
  return surface.renderer.fallback !== "activity" && surface.renderer.fallback !== "trace";
}

export function surfaceStatus(card: ChatStepCard, live = false): SurfaceStatus {
  const surface = surfaceFromCard(card);
  if (surface?.status) return surface.status;
  return live ? "running" : "pending";
}

export function surfaceGroupKey(card: ChatStepCard): string {
  const surface = surfaceFromCard(card);
  if (!surface) return card.id;
  return [
    surface.call.runId || "",
    surface.call.callId,
    String(surface.call.sequence ?? ""),
    surface.renderer.id,
  ]
    .filter(Boolean)
    .join(":");
}

export function surfaceDisplayTitle(card: ChatStepCard): string {
  const surface = surfaceFromCard(card);
  if ((surface?.tool?.id || "").trim().toLowerCase() === "agent_turn_loop") {
    return surface?.title || card.label || "Working";
  }
  return (
    surface?.title ||
    surface?.tool?.displayName ||
    card.label ||
    card.rawTitle ||
    "Working"
  );
}

export function surfacePayloads(card: ChatStepCard): Array<SurfacePayload | SurfaceArtifact> {
  const surface = surfaceFromCard(card);
  return [...(surface?.input || []), ...(surface?.output || []), ...(surface?.artifacts || [])];
}

export function firstSurfaceText(card: ChatStepCard, roles: string[] = []): string {
  const roleSet = new Set(roles);
  for (const item of surfacePayloads(card)) {
    if (roleSet.size > 0 && !roleSet.has(item.role)) continue;
    if (item.text) return item.text;
    if (item.preview) return item.preview;
    if (item.json != null) {
      try {
        return JSON.stringify(item.json, null, 2);
      } catch {
        return "";
      }
    }
  }
  return "";
}

export function firstSurfacePath(card: ChatStepCard): string {
  for (const item of surfacePayloads(card)) {
    if (item.path) return item.path;
    const metadata = item.metadata;
    if (metadata) {
      const path = str(metadata.path) || str(metadata.file);
      if (path) return path;
    }
  }
  return "";
}

export function firstSurfaceUri(card: ChatStepCard): string {
  for (const item of surfacePayloads(card)) {
    if (item.uri) return item.uri;
    const metadata = item.metadata;
    if (metadata) {
      const uri = str(metadata.url) || str(metadata.uri) || str(metadata.href);
      if (uri) return uri;
    }
  }
  return "";
}

export function firstSurfaceCommand(card: ChatStepCard): string {
  for (const item of surfacePayloads(card)) {
    const metadata = item.metadata;
    if (metadata) {
      const command = str(metadata.command) || str(metadata.cmd);
      if (command) return command;
    }
    if (item.role === "command" && item.text) return item.text;
    if (isRecord(item.json)) {
      const command = str(item.json.command) || str(item.json.cmd);
      if (command) return command;
    }
  }
  return "";
}

export function rendererSlug(rendererId: string): string {
  return rendererId.replace(/[^a-z0-9]+/gi, "-").replace(/^-+|-+$/g, "").toLowerCase();
}
