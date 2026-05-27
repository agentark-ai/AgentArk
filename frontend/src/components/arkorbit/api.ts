import { api, apiUrl } from "../../api/client";
import type {
  Orbit,
  OrbitChatHistoryMessage,
  OrbitChatMessageStatus,
  OrbitChatTranscriptPage,
  OrbitChatTranscript,
  OrbitFileEntry,
  OrbitId,
  OrbitPatch,
  OrbitsResponse,
} from "./types";

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function asOrbitChatMessageStatus(value: unknown): OrbitChatMessageStatus | undefined {
  if (
    value === "running" ||
    value === "completed" ||
    value === "failed" ||
    value === "stopped"
  ) {
    return value;
  }
  return undefined;
}

function asOrbit(value: unknown): Orbit | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.id !== "string" || typeof raw.name !== "string") return null;
  return {
    id: raw.id,
    name: raw.name,
    is_default: typeof raw.is_default === "boolean" ? raw.is_default : undefined,
    icon: asString(raw.icon),
    color: asString(raw.color),
    agent_instructions: asString(raw.agent_instructions),
    created_at: asString(raw.created_at),
    updated_at: asString(raw.updated_at),
  };
}

function extractOrbits(payload: unknown): Orbit[] {
  if (Array.isArray(payload)) {
    return payload.map(asOrbit).filter((orbit): orbit is Orbit => orbit !== null);
  }
  if (payload && typeof payload === "object") {
    const list = (payload as { orbits?: unknown }).orbits;
    if (Array.isArray(list)) {
      return list.map(asOrbit).filter((orbit): orbit is Orbit => orbit !== null);
    }
  }
  return [];
}

function extractOrbit(payload: unknown): Orbit | null {
  if (payload && typeof payload === "object") {
    const wrapped = (payload as { orbit?: unknown }).orbit;
    return asOrbit(wrapped ?? payload);
  }
  return null;
}

function asChatHistoryMessage(value: unknown): OrbitChatHistoryMessage | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (
    typeof raw.id !== "string" ||
    typeof raw.role !== "string" ||
    typeof raw.content !== "string"
  ) {
    return null;
  }
  return {
    id: raw.id,
    role: raw.role,
    content: raw.content,
    created_at: asString(raw.created_at),
    status: asOrbitChatMessageStatus(raw.status),
    activity: asString(raw.activity),
    model: asString(raw.model),
    input_tokens: asNumber(raw.input_tokens),
    output_tokens: asNumber(raw.output_tokens),
    total_tokens: asNumber(raw.total_tokens),
    cost_usd: asNumber(raw.cost_usd),
    estimated: typeof raw.estimated === "boolean" ? raw.estimated : undefined,
    duration_ms: asNumber(raw.duration_ms),
    time_to_first_token_ms: asNumber(raw.time_to_first_token_ms),
  };
}

function extractChatHistory(payload: unknown): OrbitChatHistoryMessage[] {
  const list =
    payload && typeof payload === "object"
      ? (payload as { messages?: unknown }).messages
      : payload;
  if (!Array.isArray(list)) return [];
  return list
    .map(asChatHistoryMessage)
    .filter((message): message is OrbitChatHistoryMessage => message !== null);
}

function asTranscript(value: unknown): OrbitChatTranscript | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.id !== "string" || typeof raw.title !== "string") return null;
  return {
    id: raw.id,
    title: raw.title,
    created_at: asString(raw.created_at),
    updated_at: asString(raw.updated_at),
    message_count:
      typeof raw.message_count === "number" && Number.isFinite(raw.message_count)
        ? raw.message_count
        : 0,
    current: typeof raw.current === "boolean" ? raw.current : undefined,
  };
}

function asOrbitFile(value: unknown): OrbitFileEntry | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.path !== "string") return null;
  return {
    path: raw.path,
    bytes:
      typeof raw.bytes === "number" && Number.isFinite(raw.bytes)
        ? raw.bytes
        : 0,
  };
}

function isUserArtifactFile(path: string): boolean {
  const normalized = path.trim().replace(/\\/g, "/");
  if (
    !normalized ||
    normalized.startsWith(".tmp/") ||
    normalized === "index.html" ||
    normalized === "orbit.json" ||
    normalized === "messages.jsonl" ||
    normalized === "data/chat-session.txt" ||
    normalized.startsWith("data/chat-history/")
  ) {
    return false;
  }
  return (
    normalized.startsWith("mod/") ||
    normalized.startsWith("assets/") ||
    normalized.startsWith("data/")
  );
}

function extractOrbitFiles(payload: unknown): OrbitFileEntry[] {
  const list =
    payload && typeof payload === "object"
      ? (payload as { files?: unknown }).files
      : payload;
  if (!Array.isArray(list)) return [];
  return list
    .map(asOrbitFile)
    .filter(
      (file): file is OrbitFileEntry =>
        file !== null && isUserArtifactFile(file.path),
    );
}

function extractTranscripts(payload: unknown): OrbitChatTranscript[] {
  const list =
    payload && typeof payload === "object"
      ? (payload as { transcripts?: unknown }).transcripts
      : payload;
  if (!Array.isArray(list)) return [];
  return list
    .map(asTranscript)
    .filter((transcript): transcript is OrbitChatTranscript => transcript !== null);
}

function extractTranscriptPage(
  payload: unknown,
  fallbackLimit: number,
  fallbackOffset: number,
): OrbitChatTranscriptPage {
  const transcripts = extractTranscripts(payload);
  if (!payload || typeof payload !== "object") {
    return {
      transcripts,
      total: transcripts.length,
      limit: fallbackLimit,
      offset: fallbackOffset,
      has_more: false,
    };
  }
  const raw = payload as Record<string, unknown>;
  const total = asNumber(raw.total) ?? transcripts.length;
  const limit = asNumber(raw.limit) ?? fallbackLimit;
  const offset = asNumber(raw.offset) ?? fallbackOffset;
  return {
    transcripts,
    total,
    limit,
    offset,
    has_more: typeof raw.has_more === "boolean" ? raw.has_more : undefined,
  };
}

function encodePath(path: string): string {
  return path
    .split("/")
    .filter((part) => part.length > 0)
    .map((part) => encodeURIComponent(part))
    .join("/");
}

export type CreateOrbitPayload = {
  name: string;
  icon?: string;
  color?: string;
  agent_instructions?: string;
};

export const arkorbitApi = {
  async listOrbits(): Promise<OrbitsResponse> {
    const raw = await api.rawGet("/api/arkorbit/orbits");
    return { orbits: extractOrbits(raw) };
  },
  async getOrbit(orbitId: OrbitId): Promise<Orbit | null> {
    const raw = await api.rawGet(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}`,
    );
    return extractOrbit(raw);
  },
  async createOrbit(body: CreateOrbitPayload): Promise<Orbit | null> {
    const raw = await api.rawPost("/api/arkorbit/orbits", body);
    return extractOrbit(raw);
  },
  async updateOrbit(orbitId: OrbitId, patch: OrbitPatch): Promise<Orbit | null> {
    const raw = await api.rawPut(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}`,
      patch,
    );
    return extractOrbit(raw);
  },
  async deleteOrbit(orbitId: OrbitId): Promise<void> {
    await api.rawDelete(`/api/arkorbit/orbits/${encodeURIComponent(orbitId)}`);
  },
  async listMessages(orbitId: OrbitId): Promise<OrbitChatHistoryMessage[]> {
    const raw = await api.rawGet(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/messages`,
    );
    return extractChatHistory(raw);
  },
  async listFiles(orbitId: OrbitId): Promise<OrbitFileEntry[]> {
    const raw = await api.rawGet(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/files`,
    );
    return extractOrbitFiles(raw);
  },
  async listTranscripts(
    orbitId: OrbitId,
    options: { limit?: number; offset?: number } = {},
  ): Promise<OrbitChatTranscriptPage> {
    const params = new URLSearchParams();
    if (typeof options.limit === "number") params.set("limit", String(options.limit));
    if (typeof options.offset === "number") params.set("offset", String(options.offset));
    const query = params.toString();
    const raw = await api.rawGet(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/chat/transcripts${
        query ? `?${query}` : ""
      }`,
    );
    return extractTranscriptPage(raw, options.limit ?? 25, options.offset ?? 0);
  },
  async getTranscriptMessages(
    orbitId: OrbitId,
    transcriptId: string,
  ): Promise<OrbitChatHistoryMessage[]> {
    const raw = await api.rawGet(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/chat/transcripts/${encodeURIComponent(transcriptId)}`,
    );
    return extractChatHistory(raw);
  },
  async resetChat(orbitId: OrbitId): Promise<void> {
    await api.rawPost(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/chat/reset`,
      {},
    );
  },
  async deleteWidget(orbitId: OrbitId, widgetId: string): Promise<void> {
    await api.rawDelete(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/widgets/${encodeURIComponent(widgetId)}`,
    );
  },
  async updateWidgetLayout(
    orbitId: OrbitId,
    widgetId: string,
    layout: { left?: number; top?: number },
  ): Promise<void> {
    await api.rawPut(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/widgets/${encodeURIComponent(widgetId)}`,
      layout,
    );
  },
  orbitIndexUrl(orbitId: OrbitId): string {
    return apiUrl(`/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/index`);
  },
  orbitEventsUrl(orbitId: OrbitId): string {
    return apiUrl(`/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/events`);
  },
  orbitChatUrl(orbitId: OrbitId): string {
    return apiUrl(`/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/chat`);
  },
  orbitPublicFetchUrl(orbitId: OrbitId, url: string): string {
    return apiUrl(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/fetch?url=${encodeURIComponent(url)}`,
    );
  },
  orbitPublicFetchUrlPrefix(orbitId: OrbitId): string {
    return apiUrl(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/fetch?url=`,
    );
  },
  orbitFileUrl(orbitId: OrbitId, path: string): string {
    return apiUrl(
      `/api/arkorbit/orbits/${encodeURIComponent(orbitId)}/files/${encodePath(path)}`,
    );
  },
  moduleUrl(orbitId: OrbitId, path: string): string {
    return apiUrl(
      `/api/arkorbit/mod/${encodeURIComponent(orbitId)}/${encodePath(path)}`,
    );
  },
};

export type ArkorbitApi = typeof arkorbitApi;
