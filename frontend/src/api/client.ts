import type {
  SkillImportRequest,
  SkillImportResponse,
  SkillSecretsResponse,
  SkillSecretsUpdateRequest,
  SkillTestResponse,
  BriefingResponse,
  IntegrationItem,
  LlmAnalyticsResponse,
  Notification,
  PredictiveNudgesResponse,
  StatusResponse,
  Task,
  RecommendedSkill,
  TraceResponse
} from "../types";

let sessionRefreshInFlight: Promise<void> | null = null;

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
  sessionRefreshInFlight = (async () => {
    try {
      await fetch("/ui/v2", {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        headers: { Accept: "text/html" }
      });
    } catch {
      // best effort only
    } finally {
      sessionRefreshInFlight = null;
    }
  })();
  return sessionRefreshInFlight;
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const doFetch = () =>
    fetch(path, {
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        ...(init?.headers || {})
      },
      ...init
    });
  let res = await doFetch();
  if (!res.ok) {
    let text = await res.text();
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = await res.text();
        throw new Error(text || `Request failed (${res.status})`);
      }
      return (await res.json()) as T;
    }
    throw new Error(text || `Request failed (${res.status})`);
  }
  return (await res.json()) as T;
}

export async function requestForm<T>(path: string, formData: FormData, init?: RequestInit): Promise<T> {
  const headers = { ...(init?.headers || {}) };
  const doFetch = () =>
    fetch(path, {
      credentials: "include",
      ...init,
      headers,
      body: formData
    });
  let res = await doFetch();
  if (!res.ok) {
    let text = await res.text();
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = await res.text();
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
};

type ChatStreamHandlers = {
  onEvent?: (event: string, payload: unknown) => void;
  onToken?: (token: string) => void;
  onThinking?: (step: Record<string, unknown>) => void;
  onToolStart?: (name: string, payload?: Record<string, unknown>) => void;
  onToolProgress?: (name: string, content: string, payload?: Record<string, unknown>) => void;
  onToolResult?: (name: string, content: string, payload?: Record<string, unknown>) => void;
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

async function streamChat(payload: ChatStreamPayload, handlers: ChatStreamHandlers = {}): Promise<void> {
  const doFetch = () =>
    fetch("/chat/stream", {
      method: "POST",
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        Accept: "text/event-stream"
      },
      body: JSON.stringify(payload)
    });
  let res = await doFetch();

  if (!res.ok) {
    let text = await res.text();
    if (isMissingAuthError(res.status, text)) {
      await refreshUiSessionCookie();
      res = await doFetch();
      if (!res.ok) {
        text = await res.text();
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
  getBriefing: () => request<BriefingResponse>("/autonomy/briefing"),
  getNudges: () => request<PredictiveNudgesResponse>("/autonomy/nudges"),
  feedbackNudge: (
    id: string,
    payload: { action: "dismiss" | "snooze" | "interested" | "reset"; note?: string; snooze_minutes?: number }
  ) =>
    request<{ status: string }>(`/autonomy/nudges/${encodeURIComponent(id)}/feedback`, {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  planNudges: (payload: { max_items?: number; dry_run?: boolean }) =>
    request<{
      status: string;
      dry_run: boolean;
      planned: Array<Record<string, unknown>>;
      skipped: Array<Record<string, unknown>>;
    }>("/autonomy/nudges/plan", {
      method: "POST",
      body: JSON.stringify(payload)
    }),
  getIntegrations: () => request<{ integrations: IntegrationItem[] }>("/integrations"),
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
    request<{ status: string; message?: string; queued?: boolean }>("/autonomy/skills/execute", {
      method: "POST",
      body: JSON.stringify({ action, dry_run: false })
    }),
  chat: (payload: { message: string; channel?: string; conversation_id?: string | null }) =>
    request<{ response: string; proof_id?: string; conversation_id?: string; conversation_title?: string }>(
      "/chat",
      {
        method: "POST",
        body: JSON.stringify(payload)
      }
    ),
  chatStream: (payload: ChatStreamPayload, handlers?: ChatStreamHandlers) => streamChat(payload, handlers),
  approveTask: (id: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/approve`, {
      method: "POST",
      body: JSON.stringify({})
    }),
  rejectTask: (id: string) =>
    request<{ status: string }>(`/tasks/${encodeURIComponent(id)}/reject`, {
      method: "POST",
      body: JSON.stringify({})
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
  deleteSkill: (name: string) =>
    request<{ status: string }>(`/skills/${encodeURIComponent(name)}`, { method: "DELETE" })
};
