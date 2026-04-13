import type {
  BackgroundSessionDetail,
  BackgroundSessionsResponse,
  AutonomyActionExecutionResponse,
  SkillImportRequest,
  SkillImportResponse,
  SkillSecretsResponse,
  SkillSecretsUpdateRequest,
  SkillTestResponse,
  BriefingResponse,
  ExtensionPackConnectionView,
  ExtensionPackEventsResponse,
  ExtensionPackSearchResponse,
  ExtensionPackView,
  IntegrationItem,
  IntegrationSyncFeedItem,
  IntegrationSyncStatus,
  BrowserProfilesResponse,
  GatewayChannelsResponse,
  GatewayOpsOverview,
  GatewayRoutingResponse,
  GatewayRoutingSimulation,
  GoogleWorkspaceOAuthClientSettings,
  LlmAnalyticsResponse,
  ModelFailoverResponse,
  NodeCommandsResponse,
  NodesResponse,
  Notification,
  StatusResponse,
  Task,
  RecommendedSkill,
  SentinelFeedResponse,
  SentinelSettingsResponse,
  TraceResponse
} from "../types";

let sessionRefreshInFlight: Promise<void> | null = null;
let promptedUiApiKey: string | null = null;

declare global {
  interface Window {
    __AGENTARK_BOOTSTRAP_TOKEN__?: string;
  }
}

const DEV_API_ORIGIN = String(import.meta.env.VITE_AGENTARK_API_ORIGIN || "")
  .trim()
  .replace(/\/+$/, "");

export function apiUrl(path: string): string {
  if (!DEV_API_ORIGIN) return path;
  if (/^https?:\/\//i.test(path)) return path;
  if (path.startsWith("/")) return `${DEV_API_ORIGIN}${path}`;
  return `${DEV_API_ORIGIN}/${path}`;
}

function extractErrorMessage(text: string): string {
  const trimmed = (text || "").trim();
  if (!trimmed) return "";
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    const message =
      (typeof parsed.error === "string" && parsed.error) ||
      (typeof parsed.message === "string" && parsed.message) ||
      (typeof parsed.detail === "string" && parsed.detail) ||
      "";
    return message || trimmed;
  } catch {
    return trimmed;
  }
}

function isLocalBrowserHost(): boolean {
  if (typeof window === "undefined") return false;
  const host = (window.location.hostname || "").trim().toLowerCase();
  return host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]";
}

function buildHeaders(initHeaders?: HeadersInit, options?: { json?: boolean }): Headers {
  const headers = new Headers(initHeaders || undefined);
  if (options?.json !== false && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  if (promptedUiApiKey && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${promptedUiApiKey}`);
  }
  return headers;
}

function extractBootstrapTokenFromHash(): string | null {
  const rawHash = window.location.hash || "";
  const cleaned = rawHash.startsWith("#") ? rawHash.slice(1) : rawHash;
  if (!cleaned) return null;
  const params = new URLSearchParams(cleaned);
  const token = (params.get("bootstrap") || "").trim();
  return token || null;
}

function clearBootstrapTokenFromLocation(): void {
  if (!window.location.hash) return;
  const rawHash = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  const params = new URLSearchParams(rawHash);
  if (!params.has("bootstrap")) return;
  params.delete("bootstrap");
  const nextHash = params.toString();
  const nextUrl = `${window.location.pathname}${window.location.search}${nextHash ? `#${nextHash}` : ""}`;
  window.history.replaceState(null, "", nextUrl);
}

type LocalBootstrapAttempt = {
  ok: boolean;
  error?: string;
};

async function redeemLocalBootstrapToken(token: string): Promise<LocalBootstrapAttempt> {
  if (!token.trim()) return { ok: false };
  try {
    const response = await fetch(apiUrl("/session/bootstrap/local"), {
      method: "POST",
      credentials: "include",
      cache: "no-store",
      headers: buildHeaders({ Accept: "application/json" }),
      body: JSON.stringify({ token })
    });
    if (response.ok) return { ok: true };
    return { ok: false, error: extractErrorMessage(await response.text()) || undefined };
  } catch {
    return { ok: false };
  }
}

async function requestLocalBootstrapToken(): Promise<{ token: string | null; error?: string }> {
  try {
    const response = await fetch(apiUrl("/session/bootstrap/local"), {
      method: "GET",
      credentials: "include",
      cache: "no-store",
      headers: buildHeaders({ Accept: "application/json" }, { json: false })
    });
    if (!response.ok) {
      return {
        token: null,
        error: extractErrorMessage(await response.text()) || undefined
      };
    }
    const payload = (await response.json()) as Record<string, unknown>;
    const token = typeof payload.token === "string" ? payload.token.trim() : "";
    return { token: token || null };
  } catch {
    return { token: null };
  }
}

async function trySilentBootstrap(): Promise<LocalBootstrapAttempt> {
  const tokenFromHash = extractBootstrapTokenFromHash();
  const tokenFromWindow = (window.__AGENTARK_BOOTSTRAP_TOKEN__ || "").trim() || null;
  const requested = tokenFromHash || tokenFromWindow ? { token: tokenFromHash || tokenFromWindow } : await requestLocalBootstrapToken();
  const token = requested.token;
  if (!token) return { ok: false, error: requested.error };

  const redeemed = await redeemLocalBootstrapToken(token);
  if (redeemed.ok) {
    clearBootstrapTokenFromLocation();
  }
  try {
    delete window.__AGENTARK_BOOTSTRAP_TOKEN__;
  } catch {
    window.__AGENTARK_BOOTSTRAP_TOKEN__ = undefined;
  }
  return redeemed;
}

function isMissingAuthError(status: number, text: string): boolean {
  if (status !== 401 && status !== 403) return false;
  const lower = (text || "").toLowerCase();
  return (
    lower.includes("missing authorization") ||
    lower.includes("bearer <api_key>") ||
    lower.includes("invalid api key") ||
    lower.includes("api authentication")
  );
}

async function refreshUiSessionCookie(): Promise<void> {
  if (sessionRefreshInFlight) return sessionRefreshInFlight;

  const bootstrapWithApiKey = async (apiKey: string): Promise<boolean> => {
    try {
      const response = await fetch(apiUrl("/session/bootstrap"), {
        method: "POST",
        credentials: "include",
        cache: "no-store",
        headers: buildHeaders(
          {
            Accept: "application/json",
            Authorization: `Bearer ${apiKey}`
          },
          { json: false }
        )
      });
      return response.ok;
    } catch {
      return false;
    }
  };

  const probeProtectedSession = async (): Promise<boolean> => {
    try {
      const response = await fetch(apiUrl("/autonomy/settings"), {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        headers: buildHeaders({ Accept: "application/json" }, { json: false })
      });
      if (response.ok) return true;
      const text = extractErrorMessage(await response.text());
      return !isMissingAuthError(response.status, text);
    } catch {
      return false;
    }
  };

  sessionRefreshInFlight = (async () => {
    const firstSilentBootstrap = await trySilentBootstrap();
    if (firstSilentBootstrap.ok) return;

    try {
      await fetch(apiUrl("/ui/v2"), {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        headers: { Accept: "text/html" }
      });
    } catch {
      // best effort only
    }

    if (await probeProtectedSession()) return;

    const secondSilentBootstrap = await trySilentBootstrap();
    if (secondSilentBootstrap.ok) {
      if (await probeProtectedSession()) return;
    }

    if (promptedUiApiKey) {
      if (await bootstrapWithApiKey(promptedUiApiKey)) {
        if (await probeProtectedSession()) return;
      }
      promptedUiApiKey = null;
    }

    if (isLocalBrowserHost()) {
      throw new Error(
        firstSilentBootstrap.error ||
          secondSilentBootstrap.error ||
          "Could not authorize this local browser session automatically. Restart AgentArk and refresh the page."
      );
    }

    throw new Error(
      "This browser session is not authorized. Open AgentArk locally, or use the public-link password sign-in page."
    );
  })().finally(() => {
    sessionRefreshInFlight = null;
  });

  return sessionRefreshInFlight;
}

export async function initializeUiSession(): Promise<void> {
  await trySilentBootstrap();
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const doFetch = () =>
    fetch(apiUrl(path), {
      credentials: "include",
      ...init
      ,
      headers: buildHeaders(init?.headers)
    });
  let res = await doFetch();
  if (!res.ok) {
    let text = extractErrorMessage(await res.text());
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = extractErrorMessage(await res.text());
        throw new Error(text || `Request failed (${res.status})`);
      }
      return (await res.json()) as T;
    }
    throw new Error(text || `Request failed (${res.status})`);
  }
  return (await res.json()) as T;
}

export async function requestForm<T>(path: string, formData: FormData, init?: RequestInit): Promise<T> {
  const doFetch = () =>
    fetch(apiUrl(path), {
      credentials: "include",
      ...init,
      headers: buildHeaders(init?.headers, { json: false }),
      body: formData
    });
  let res = await doFetch();
  if (!res.ok) {
    let text = extractErrorMessage(await res.text());
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = extractErrorMessage(await res.text());
        throw new Error(text || `Request failed (${res.status})`);
      }
      return (await res.json()) as T;
    }
    throw new Error(text || `Request failed (${res.status})`);
  }
  return (await res.json()) as T;
}

type ChatStreamPayload = {
  message: string;
  channel?: string;
  conversation_id?: string | null;
  project_id?: string | null;
  deep_research?: boolean;
  plan_confirmation_mode?: string;
  execution_mode?: string;
  attachments_present?: boolean;
};

type ChatStreamHandlers = {
  signal?: AbortSignal;
  onEvent?: (event: string, payload: unknown) => void;
  onToken?: (token: string) => void;
  onThinking?: (step: Record<string, unknown>) => void;
  onToolStart?: (name: string, payload?: Record<string, unknown>) => void;
  onToolProgress?: (name: string, content: string, payload?: Record<string, unknown>) => void;
  onToolResult?: (name: string, content: string, payload?: Record<string, unknown>) => void;
  onTaskStarted?: (payload: Record<string, unknown>) => void;
  onTaskStatus?: (payload: Record<string, unknown>) => void;
  onContent?: (payload: Record<string, unknown>) => void;
  onError?: (message: string, payload?: unknown) => void;
  onDone?: () => void;
};

function parseMaybeJson(raw: string): unknown {
  const trimmed = raw.trim();
  if (!trimmed) return {};
  try {
    return JSON.parse(trimmed);
  } catch {
    return raw;
  }
}

function asObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function asText(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function extractStreamErrorMessage(payloadValue: unknown): string {
  if (typeof payloadValue === "string") return payloadValue;
  const obj = asObject(payloadValue);
  const direct =
    asText(obj.error) ||
    asText(obj.message) ||
    asText(obj.detail) ||
    "";
  if (direct) return direct;
  try {
    return JSON.stringify(payloadValue);
  } catch {
    return "";
  }
}

async function streamSseJson(
  path: string,
  payload: unknown,
  handlers: ChatStreamHandlers = {}
): Promise<void> {
  const doFetch = () =>
    fetch(apiUrl(path), {
      method: "POST",
      credentials: "include",
      signal: handlers.signal,
      headers: buildHeaders({
        Accept: "text/event-stream"
      }),
      body: JSON.stringify(payload)
    });
  let res = await doFetch();

  if (!res.ok) {
    let text = extractErrorMessage(await res.text());
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = extractErrorMessage(await res.text());
        throw new Error(text || `Request failed (${res.status})`);
      }
    } else {
      throw new Error(text || `Request failed (${res.status})`);
    }
  }

  if (!res.body) throw new Error("Streaming is not available in this browser session.");

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let doneReceived = false;

  const processBlock = (block: string) => {
    const lines = block.split("\n");
    let eventName = "message";
    const dataLines: string[] = [];

    for (const line of lines) {
      if (!line || line.startsWith(":")) continue;
      const splitIdx = line.indexOf(":");
      if (splitIdx < 0) continue;
      const field = line.slice(0, splitIdx).trim();
      const value = line.slice(splitIdx + 1).trimStart();
      if (field === "event") eventName = value;
      if (field === "data") dataLines.push(value);
    }

    const payloadValue = parseMaybeJson(dataLines.join("\n"));
    handlers.onEvent?.(eventName, payloadValue);

    if (eventName === "token") {
      const content = asText(asObject(payloadValue).content);
      if (content) handlers.onToken?.(content);
      return;
    }
    if (eventName === "thinking") {
      handlers.onThinking?.(asObject(payloadValue));
      return;
    }
    if (eventName === "tool_start") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      if (name) handlers.onToolStart?.(name, obj);
      return;
    }
    if (eventName === "tool_result") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      const content = asText(obj.content);
      handlers.onToolResult?.(name, content, obj);
      return;
    }
    if (eventName === "tool_progress") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      const content = asText(obj.content);
      handlers.onToolProgress?.(name, content, obj);
      return;
    }
    if (eventName === "task_started") {
      handlers.onTaskStarted?.(asObject(payloadValue));
      return;
    }
    if (eventName === "task_status") {
      handlers.onTaskStatus?.(asObject(payloadValue));
      return;
    }
    if (eventName === "content") {
      handlers.onContent?.(asObject(payloadValue));
      return;
    }
    if (eventName === "error") {
      const message = extractStreamErrorMessage(payloadValue) || "Stream failed.";
      handlers.onError?.(message, payloadValue);
      return;
    }
    if (eventName === "done") {
      doneReceived = true;
      handlers.onDone?.();
    }
  };

  try {
    while (!doneReceived) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      buffer = buffer.replace(/\r\n/g, "\n");
      let splitAt = buffer.indexOf("\n\n");
      while (splitAt >= 0) {
        const rawEvent = buffer.slice(0, splitAt);
        buffer = buffer.slice(splitAt + 2);
        if (rawEvent.trim()) processBlock(rawEvent);
        if (doneReceived) break;
        splitAt = buffer.indexOf("\n\n");
      }
    }
  } finally {
    try {
      await reader.cancel();
    } catch {
      // ignore cleanup errors
    }
  }
}

async function streamChat(payload: ChatStreamPayload, handlers: ChatStreamHandlers = {}): Promise<void> {
  return streamSseJson("/chat/stream", payload, handlers);
}

async function streamRun(runId: string, sinceSeq = 0, handlers: ChatStreamHandlers = {}): Promise<void> {
  const query = sinceSeq > 0 ? `?since_seq=${encodeURIComponent(String(sinceSeq))}` : "";
  const path = `/runs/${encodeURIComponent(runId)}/stream${query}`;
  const doFetch = () =>
    fetch(apiUrl(path), {
      method: "GET",
      credentials: "include",
      signal: handlers.signal,
      headers: buildHeaders({
        Accept: "text/event-stream"
      }, { json: false })
    });
  let res = await doFetch();

  if (!res.ok) {
    let text = extractErrorMessage(await res.text());
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = extractErrorMessage(await res.text());
        throw new Error(text || `Request failed (${res.status})`);
      }
    } else {
      throw new Error(text || `Request failed (${res.status})`);
    }
  }

  if (!res.body) throw new Error("Streaming is not available in this browser session.");

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let doneReceived = false;

  const processBlock = (block: string) => {
    const lines = block.split("\n");
    let eventName = "message";
    const dataLines: string[] = [];

    for (const line of lines) {
      if (!line || line.startsWith(":")) continue;
      const splitIdx = line.indexOf(":");
      if (splitIdx < 0) continue;
      const field = line.slice(0, splitIdx).trim();
      const value = line.slice(splitIdx + 1).trimStart();
      if (field === "event") eventName = value;
      if (field === "data") dataLines.push(value);
    }

    const payloadValue = parseMaybeJson(dataLines.join("\n"));
    handlers.onEvent?.(eventName, payloadValue);

    if (eventName === "token") {
      const content = asText(asObject(payloadValue).content);
      if (content) handlers.onToken?.(content);
      return;
    }
    if (eventName === "thinking") {
      handlers.onThinking?.(asObject(payloadValue));
      return;
    }
    if (eventName === "tool_start") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      if (name) handlers.onToolStart?.(name, obj);
      return;
    }
    if (eventName === "tool_result") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      const content = asText(obj.content);
      handlers.onToolResult?.(name, content, obj);
      return;
    }
    if (eventName === "tool_progress") {
      const obj = asObject(payloadValue);
      const name = asText(obj.name);
      const content = asText(obj.content);
      handlers.onToolProgress?.(name, content, obj);
      return;
    }
    if (eventName === "content") {
      handlers.onContent?.(asObject(payloadValue));
      return;
    }
    if (eventName === "error") {
      const message = extractStreamErrorMessage(payloadValue) || "Stream failed.";
      handlers.onError?.(message, payloadValue);
      return;
    }
    if (eventName === "done") {
      doneReceived = true;
      handlers.onDone?.();
    }
  };

  try {
    while (!doneReceived) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      buffer = buffer.replace(/\r\n/g, "\n");
      let splitAt = buffer.indexOf("\n\n");
      while (splitAt >= 0) {
        const rawEvent = buffer.slice(0, splitAt);
        buffer = buffer.slice(splitAt + 2);
        if (rawEvent.trim()) processBlock(rawEvent);
        if (doneReceived) break;
        splitAt = buffer.indexOf("\n\n");
      }
    }
  } finally {
    try {
      await reader.cancel();
    } catch {
      // ignore cleanup errors
    }
  }
}

export const api = {
  rawGet: (path: string) => request<unknown>(path),
  rawPost: (path: string, payload?: unknown) =>
    request<unknown>(path, {
      method: "POST",
      body: JSON.stringify(payload ?? {})
    }),
  rawPut: (path: string, payload?: unknown) =>
    request<unknown>(path, {
      method: "PUT",
      body: JSON.stringify(payload ?? {})
    }),
  rawPatch: (path: string, payload?: unknown) =>
    request<unknown>(path, {
      method: "PATCH",
      body: JSON.stringify(payload ?? {})
    }),
  rawDelete: (path: string) =>
    request<unknown>(path, {
      method: "DELETE"
    }),
  rawPostForm: (path: string, formData: FormData) =>
    requestForm<unknown>(path, formData, {
      method: "POST"
    }),
  getStatus: () => request<StatusResponse>("/status"),
  getTasks: async () => {
    const raw = await request<unknown>("/tasks");
    if (Array.isArray(raw)) return raw as Task[];
    if (raw && typeof raw === "object" && Array.isArray((raw as { tasks?: unknown }).tasks)) {
      return (raw as { tasks: Task[] }).tasks;
    }
    return [];
  },
  getBackgroundSessions: () => request<BackgroundSessionsResponse>("/background-sessions"),
  getBackgroundSession: (id: string) =>
    request<BackgroundSessionDetail>(`/background-sessions/${encodeURIComponent(id)}`),
  createBackgroundSession: (payload: Record<string, unknown>) =>
    request<{ status: string; id: string }>("/background-sessions", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  updateBackgroundSession: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  attachBackgroundSessionWork: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}/attach`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  detachBackgroundSessionWork: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}/detach`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  pauseBackgroundSession: (id: string) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}/pause`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  resumeBackgroundSession: (id: string) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}/resume`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  cancelBackgroundSession: (id: string) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}/cancel`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  deleteBackgroundSession: (id: string) =>
    request<{ status: string }>(`/background-sessions/${encodeURIComponent(id)}`, {
      method: "DELETE"
    }),
  getNotifications: async () => {
    const raw = await request<unknown>("/notifications");
    if (Array.isArray(raw)) return raw as Notification[];
    if (
      raw &&
      typeof raw === "object" &&
      Array.isArray((raw as { notifications?: unknown }).notifications)
    ) {
      return (raw as { notifications: Notification[] }).notifications;
    }
    return [];
  },
  getTrace: () => request<TraceResponse>("/trace"),
  getSentinelSettings: () => request<SentinelSettingsResponse>("/autonomy/sentinel/settings"),
  updateSentinelSettings: (payload: Record<string, unknown>) =>
    request<SentinelSettingsResponse>("/autonomy/sentinel/settings", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  getSentinelFeed: () => request<SentinelFeedResponse>("/autonomy/sentinel/feed"),
  approveSentinelProposal: (id: string) =>
    request<{ status: string; message?: string; trace_id?: string; proposal?: Record<string, unknown> }>(
      `/autonomy/sentinel/proposals/${encodeURIComponent(id)}/approve`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  dismissSentinelProposal: (id: string) =>
    request<{ status: string }>(`/autonomy/sentinel/proposals/${encodeURIComponent(id)}/dismiss`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  snoozeSentinelProposal: (id: string) =>
    request<{ status: string; snoozed_until?: string }>(
      `/autonomy/sentinel/proposals/${encodeURIComponent(id)}/snooze`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  getBriefing: () => request<BriefingResponse>("/autonomy/briefing"),
  getIntegrations: () => request<{ integrations: IntegrationItem[] }>("/integrations"),
  getExtensionPacks: (params?: { query?: string; kind?: string }) => {
    const query = new URLSearchParams();
    if (params?.query) query.set("query", params.query);
    if (params?.kind) query.set("kind", params.kind);
    const suffix = query.toString();
    return request<ExtensionPackSearchResponse>(
      `/extension-packs${suffix ? `?${suffix}` : ""}`
    );
  },
  getExtensionPack: (id: string) =>
    request<{ pack: ExtensionPackView; connections: ExtensionPackConnectionView[] }>(
      `/extension-packs/${encodeURIComponent(id)}`
    ),
  getExtensionPackEvents: (id: string, limit = 25) =>
    request<ExtensionPackEventsResponse>(
      `/extension-packs/${encodeURIComponent(id)}/events?limit=${encodeURIComponent(String(limit))}`
    ),
  installExtensionPack: (payload: Record<string, unknown>) =>
    request<{ status: string; pack: ExtensionPackView }>("/extension-packs/install", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  uploadExtensionPack: (formData: FormData) =>
    requestForm<{ status: string; pack: ExtensionPackView }>("/extension-packs/upload", formData, {
      method: "POST"
    }),
  scaffoldExtensionPack: (payload: Record<string, unknown>) =>
    request<{ status: string; pack: ExtensionPackView }>("/extension-packs/scaffold", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  upsertExtensionPackConnection: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; connection: ExtensionPackConnectionView }>(
      `/extension-packs/${encodeURIComponent(id)}/connections`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  getExtensionPackConnectUrl: (id: string, redirectUri?: string) => {
    const query = new URLSearchParams();
    if (redirectUri) query.set("redirect_uri", redirectUri);
    const suffix = query.toString();
    return request<{ url: string; auth_url: string; redirect_uri: string }>(
      `/extension-packs/${encodeURIComponent(id)}/connect-url${suffix ? `?${suffix}` : ""}`
    );
  },
  testExtensionPackConnection: (id: string, connectionId: string) =>
    request<{ status: string; result: Record<string, unknown> }>(
      `/extension-packs/${encodeURIComponent(id)}/connections/${encodeURIComponent(connectionId)}/test`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  setExtensionPackEnabled: (id: string, enabled: boolean) =>
    request<{ status: string; pack: ExtensionPackView }>(
      `/extension-packs/${encodeURIComponent(id)}/enabled`,
      {
        method: "POST",
        body: JSON.stringify({ enabled })
      }
    ),
  deleteExtensionPack: (id: string, params?: { remove_connections?: boolean }) => {
    const query = new URLSearchParams();
    if (typeof params?.remove_connections === "boolean") {
      query.set("remove_connections", String(params.remove_connections));
    }
    const suffix = query.toString();
    return request<{ status: string }>(
      `/extension-packs/${encodeURIComponent(id)}${suffix ? `?${suffix}` : ""}`,
      {
        method: "DELETE"
      }
    );
  },
  getIntegrationSyncStatus: () => request<{ statuses: IntegrationSyncStatus[] }>("/integrations/sync/status"),
  getIntegrationSyncFeed: (params?: { integration_id?: string; limit?: number }) => {
    const query = new URLSearchParams();
    if (params?.integration_id) query.set("integration_id", params.integration_id);
    if (typeof params?.limit === "number") query.set("limit", String(params.limit));
    const suffix = query.toString();
    return request<{ items: IntegrationSyncFeedItem[] }>(
      `/integrations/sync/feed${suffix ? `?${suffix}` : ""}`
    );
  },
  updateIntegrationSync: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; sync: IntegrationSyncStatus }>(`/integrations/${encodeURIComponent(id)}/sync`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  runIntegrationSyncNow: (id: string) =>
    request<{ status: string; sync: IntegrationSyncStatus }>(`/integrations/${encodeURIComponent(id)}/sync-now`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  getChannels: () => request<GatewayChannelsResponse>("/gateway/channels"),
  getGatewayOpsOverview: () => request<GatewayOpsOverview>("/gateway/ops"),
  createChannelAccount: (payload: Record<string, unknown>) =>
    request<{ status: string; account?: Record<string, unknown>; message?: string }>("/gateway/channels/accounts", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  updateChannelAccount: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; account?: Record<string, unknown>; message?: string }>(
      `/gateway/channels/accounts/${encodeURIComponent(id)}`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  deleteChannelAccount: (id: string) =>
    request<{ status: string; message?: string }>(`/gateway/channels/accounts/${encodeURIComponent(id)}`, {
      method: "DELETE"
    }),
  getRouting: () => request<GatewayRoutingResponse>("/gateway/routing"),
  createRoutingRule: (payload: Record<string, unknown>) =>
    request<{ status: string; rule?: Record<string, unknown>; message?: string }>("/gateway/routing/rules", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  createBroadcastGroup: (payload: Record<string, unknown>) =>
    request<{ status: string; group?: Record<string, unknown>; message?: string }>("/gateway/routing/groups", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  updateRoutingRule: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; rule?: Record<string, unknown>; message?: string }>(
      `/gateway/routing/rules/${encodeURIComponent(id)}`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  deleteRoutingRule: (id: string) =>
    request<{ status: string; message?: string }>(`/gateway/routing/rules/${encodeURIComponent(id)}`, {
      method: "DELETE"
    }),
  simulateRouting: (payload: Record<string, unknown>) =>
    request<{ status: string; simulation: GatewayRoutingSimulation }>("/gateway/routing/simulate", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  getNodes: () => request<NodesResponse>("/nodes"),
  createNode: (payload: Record<string, unknown>) =>
    request<{ status: string; node?: Record<string, unknown>; message?: string }>("/nodes", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  updateNode: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; node?: Record<string, unknown>; message?: string }>(`/nodes/${encodeURIComponent(id)}`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  deleteNode: (id: string) =>
    request<{ status: string; node?: Record<string, unknown>; message?: string }>(`/nodes/${encodeURIComponent(id)}`, {
      method: "DELETE"
    }),
  refreshNodeHeartbeat: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; heartbeat?: Record<string, unknown>; message?: string }>(
      `/nodes/${encodeURIComponent(id)}/heartbeat`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  getNodeCommands: (id: string) => request<NodeCommandsResponse>(`/nodes/${encodeURIComponent(id)}/commands`),
  logNodeCommand: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; command?: Record<string, unknown>; message?: string }>(
      `/nodes/${encodeURIComponent(id)}/commands`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  getBrowserProfiles: () => request<BrowserProfilesResponse>("/browser/profiles"),
  createBrowserProfile: (payload: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>("/browser/profiles", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  updateBrowserProfile: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/browser/profiles/${encodeURIComponent(id)}`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  deleteBrowserProfile: (id: string) =>
    request<{ status: string; message?: string }>(`/browser/profiles/${encodeURIComponent(id)}`, {
      method: "DELETE"
    }),
  lockBrowserProfile: (id: string, payload?: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/browser/profiles/${encodeURIComponent(id)}/lock`,
      {
        method: "POST",
        body: JSON.stringify(payload ?? {})
      }
    ),
  unlockBrowserProfile: (id: string, payload?: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/browser/profiles/${encodeURIComponent(id)}/unlock`,
      {
        method: "POST",
        body: JSON.stringify(payload ?? {})
      }
    ),
  recordBrowserSession: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/browser/profiles/${encodeURIComponent(id)}/sessions`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  getModelFailover: () => request<ModelFailoverResponse>("/models/failover"),
  upsertAuthProfile: (payload: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      "/models/failover/profiles",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  setDefaultAuthProfile: (id: string) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/models/failover/profiles/${encodeURIComponent(id)}/default`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  disableAuthProfile: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; profile?: Record<string, unknown>; message?: string }>(
      `/models/failover/profiles/${encodeURIComponent(id)}/disable`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  clearAuthProfileCooldown: (id: string) =>
    request<{ status: string; result?: Record<string, unknown>; message?: string }>(
      `/models/failover/profiles/${encodeURIComponent(id)}/clear-cooldown`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  rotateAuthProfile: (id: string) =>
    request<{ status: string; result?: Record<string, unknown>; message?: string }>(
      `/models/failover/profiles/${encodeURIComponent(id)}/rotate`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  upsertProviderHealth: (payload: Record<string, unknown>) =>
    request<{ status: string; provider?: Record<string, unknown>; message?: string }>(
      "/models/failover/providers",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  disableProviderHealth: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; provider?: Record<string, unknown>; message?: string }>(
      `/models/failover/providers/${encodeURIComponent(id)}/disable`,
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  clearProviderCooldown: (id: string) =>
    request<{ status: string; result?: Record<string, unknown>; message?: string }>(
      `/models/failover/providers/${encodeURIComponent(id)}/clear-cooldown`,
      {
        method: "POST",
        body: JSON.stringify({})
      }
    ),
  upsertFallbackChain: (payload: Record<string, unknown>) =>
    request<{ status: string; chain?: Record<string, unknown>; message?: string }>(
      "/models/failover/chains",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  selectModelFailoverCandidate: (payload: Record<string, unknown>) =>
    request<{ status: string; result?: Record<string, unknown>; message?: string }>(
      "/models/failover/select",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  importSkill: (payload: SkillImportRequest) =>
    request<SkillImportResponse>("/skills/import", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  getSkillSecrets: (name: string) =>
    request<SkillSecretsResponse>(`/skills/${encodeURIComponent(name)}/secrets`),
  setSkillSecrets: (name: string, payload: SkillSecretsUpdateRequest) =>
    request<SkillSecretsResponse>(`/skills/${encodeURIComponent(name)}/secrets`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  testSkill: (name: string, argumentsPayload?: unknown) =>
    request<SkillTestResponse>(`/skills/${encodeURIComponent(name)}/test`, {
      method: "POST",
      body: JSON.stringify({ arguments: argumentsPayload ?? {} })
    }),
  setSkillEnabled: (name: string, enabled: boolean) =>
    request<{ status: string; name: string; enabled: boolean }>(`/skills/${encodeURIComponent(name)}/enabled`, {
      method: "POST",
      body: JSON.stringify({ enabled })
    }),
  configureIntegration: (id: string, payload: Record<string, unknown>) =>
    request<{ status: string; message?: string }>(`/integrations/${encodeURIComponent(id)}/configure`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  disconnectIntegration: (id: string) =>
    request<{ status: string; message?: string }>(`/integrations/${encodeURIComponent(id)}/disconnect`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  enableIntegration: (id: string) =>
    request<{ status: string; message?: string }>(`/integrations/${encodeURIComponent(id)}/enable`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  disableIntegration: (id: string) =>
    request<{ status: string; message?: string }>(`/integrations/${encodeURIComponent(id)}/disable`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  testIntegration: (id: string) =>
    request<{ status: string; connected?: boolean; detail?: string }>(`/integrations/${encodeURIComponent(id)}/test`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  getLlmAnalytics: (params?: { range?: string; bucket?: "hour" | "day" | "week" | string; from?: string; to?: string }) => {
    const range = encodeURIComponent(params?.range || "24h");
    const bucket = encodeURIComponent(params?.bucket || "hour");
    const from = params?.from ? `&from=${encodeURIComponent(params.from)}` : "";
    const to = params?.to ? `&to=${encodeURIComponent(params.to)}` : "";
    return request<LlmAnalyticsResponse>(`/analytics/llm?range=${range}&bucket=${bucket}${from}${to}`);
  },
  executeRecommendedSkill: (action: RecommendedSkill) =>
      request<AutonomyActionExecutionResponse>("/autonomy/skills/execute", {
        method: "POST",
        body: JSON.stringify({ action, dry_run: false })
      }),
  chat: (payload: { message: string; channel?: string; conversation_id?: string | null; project_id?: string | null; deep_research?: boolean; plan_confirmation_mode?: string; execution_mode?: string }) =>
    request<{ response: string; proof_id?: string; conversation_id?: string; conversation_title?: string }>(
      "/chat",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  chatStream: (payload: ChatStreamPayload, handlers?: ChatStreamHandlers) => streamChat(payload, handlers),
  resumeChatTaskStream: (
    id: string,
    payloadOrHandlers?: { plan_override?: Record<string, unknown> } | ChatStreamHandlers,
    maybeHandlers?: ChatStreamHandlers
  ) => {
    const payload =
      payloadOrHandlers &&
      typeof payloadOrHandlers === "object" &&
      ("signal" in payloadOrHandlers ||
        "onEvent" in payloadOrHandlers ||
        "onToken" in payloadOrHandlers ||
        "onThinking" in payloadOrHandlers ||
        "onToolStart" in payloadOrHandlers ||
        "onToolProgress" in payloadOrHandlers ||
        "onToolResult" in payloadOrHandlers ||
        "onTaskStarted" in payloadOrHandlers ||
        "onTaskStatus" in payloadOrHandlers ||
        "onContent" in payloadOrHandlers ||
        "onError" in payloadOrHandlers ||
        "onDone" in payloadOrHandlers)
        ? undefined
        : (payloadOrHandlers as { plan_override?: Record<string, unknown> } | undefined);
    const handlers = payload ? maybeHandlers : (payloadOrHandlers as ChatStreamHandlers | undefined);
    return streamSseJson(`/tasks/${encodeURIComponent(id)}/resume-chat/stream`, payload ?? {}, handlers);
  },
  cancelTask: (id: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/cancel`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  runStream: (runId: string, sinceSeq?: number, handlers?: ChatStreamHandlers) =>
    streamRun(runId, sinceSeq, handlers),
  approveTask: (id: string, comment?: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/approve`, {
      method: "POST",
      body: JSON.stringify({ comment: comment?.trim() || undefined })
    }),
  rejectTask: (id: string, comment?: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/reject`, {
      method: "POST",
      body: JSON.stringify({ comment: comment?.trim() || undefined })
    }),
  retryTask: (id: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/retry`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  getSecurityLogs: (limit = 5) =>
    request<{ logs: Array<{ event_type: string; severity: string; message: string; source?: string; created_at?: string }> }>(
      `/security/logs?limit=${limit}`
    ),
  getSettings: () => request<Record<string, unknown>>("/settings"),
  getGoogleWorkspaceOAuthClientSettings: () =>
    request<GoogleWorkspaceOAuthClientSettings>("/settings/google-workspace/oauth-client"),
  updateGoogleWorkspaceOAuthClientSettings: (payload: Record<string, unknown>) =>
    request<GoogleWorkspaceOAuthClientSettings>("/settings/google-workspace/oauth-client", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  deleteSkill: (name: string) =>
    request<{ status: string }>(`/skills/${encodeURIComponent(name)}`, { method: "DELETE" })
};
