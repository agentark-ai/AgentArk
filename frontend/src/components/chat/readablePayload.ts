export type ReadablePayloadTone =
  | "running"
  | "success"
  | "warning"
  | "error"
  | "idle";

export type ReadablePayloadField = {
  label: string;
  value: string;
};

export type ReadablePayloadSummary = {
  title: string;
  detail: string;
  status?: string;
  tone: ReadablePayloadTone;
  fields: ReadablePayloadField[];
  isDelegation: boolean;
  isStructured: boolean;
};

type JsonRecord = Record<string, unknown>;

const INTERNAL_FIELD_KEYS = new Set([
  "__streamKey",
  "__omitted_keys",
  "agent_id",
  "chat_visible",
  "conversation_id",
  "conversationId",
  "cid",
  "delegation_id",
  "id",
  "name",
  "plan_id",
  "plan_revision",
  "plan_step_id",
  "plan_step_title",
  "run_id",
  "runId",
  "sequence",
  "task_id",
  "taskId",
  "time",
  "timestamp",
  "trace_id",
  "traceId",
  "ts",
  // Presentational duplicates of header/subtitle content. The trace step
  // header already shows title, detail, status, icon — surfacing them again
  // as cards is JSON-dump noise.
  "title",
  "detail",
  "details",
  "detail_full",
  "detailFull",
  "step_type",
  "stepType",
  "step",
  "step_key",
  "stepKey",
  "icon",
  "event_type",
  "eventType",
  // `data` is typically an object whose value summarizes to its own key
  // list (see summarizeStructuredValue), producing meta-meta noise like
  // "Duration MS, Metric" instead of useful information.
  "data",
]);

const SECRET_FIELD_PATTERN =
  /(?:^|[_-])(?:access_password|password|passcode|secret|token|api_key|apikey|private_key|client_secret|refresh_token)(?:$|[_-])/i;

const FIELD_PRIORITY = [
  "agent_name",
  "agent_role",
  "status",
  "reason",
  "model_name",
  "elapsed_ms",
  "is_specialist",
  "task",
  "summary",
  "content",
  "tool_name",
  "kind",
  "title",
  "file",
  "path",
  "url",
  "query",
  "error",
];

export function isReadableRecord(value: unknown): value is JsonRecord {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function readableString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

export function readableNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

export function parseReadableJson(value: unknown): unknown | null {
  if (isReadableRecord(value) || Array.isArray(value)) return value;
  const text = readableString(value).trim();
  if (!text) return null;
  const first = text[0];
  if (first !== "{" && first !== "[") return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function compactText(value: string, maxLen = 260): string {
  const text = (value || "").replace(/\s+/g, " ").trim();
  if (text.length <= maxLen) return text;
  return `${text.slice(0, Math.max(0, maxLen - 3)).trimEnd()}...`;
}

function omittedPlaceholder(value: string): boolean {
  return /^\[omitted\s+\d+\s+chars?\]$/i.test(value.trim());
}

export function humanizePayloadLabel(value: string, fallback = "Value"): string {
  const normalized = (value || "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized) return fallback;
  const direct: Record<string, string> = {
    agent: "Agent",
    "agent name": "Agent",
    "agent role": "Role",
    "duration ms": "Duration",
    "duration seconds": "Duration",
    "elapsed ms": "Elapsed",
    "is specialist": "Agent type",
    "latency ms": "Latency",
    "model name": "Model",
    "size bytes": "Size",
    "tool name": "Tool",
  };
  const lower = normalized.toLowerCase();
  if (direct[lower]) return direct[lower];
  return normalized
    .split(" ")
    .map((part) =>
      part.length <= 3 && part === part.toLowerCase()
        ? part.toUpperCase()
        : `${part.charAt(0).toUpperCase()}${part.slice(1)}`,
    )
    .join(" ");
}

export function humanizePayloadStatus(value: unknown, fallback = "Updated"): string {
  const raw = readableString(value, String(value ?? "")).trim();
  if (!raw) return fallback;
  return raw
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

export function formatReadableDurationMs(value: unknown): string {
  const ms = readableNumber(value, 0);
  if (!ms || ms < 0) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(seconds >= 10 ? 0 : 1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.round(seconds % 60);
  return remaining > 0 ? `${minutes}m ${remaining}s` : `${minutes}m`;
}

function fieldValueToText(key: string, value: unknown): string {
  if (SECRET_FIELD_PATTERN.test(key)) return "[redacted]";
  if (value == null) return "";
  if (typeof value === "string") {
    const parsed = parseReadableJson(value);
    if (parsed != null) return summarizeStructuredValue(parsed);
    const text = compactText(value);
    return omittedPlaceholder(text) ? "" : text;
  }
  if (typeof value === "number") {
    if (/elapsed|duration|latency/i.test(key)) {
      return formatReadableDurationMs(value);
    }
    return Number.isInteger(value) ? value.toLocaleString() : String(value);
  }
  if (typeof value === "boolean") {
    if (key === "is_specialist") return value ? "Specialist" : "Generalist";
    return value ? "Yes" : "No";
  }
  return summarizeStructuredValue(value);
}

function isInternalField(key: string): boolean {
  if (!key) return true;
  if (key.startsWith("__")) return true;
  if (INTERNAL_FIELD_KEYS.has(key)) return true;
  return false;
}

export function readableFieldsFromRecord(
  record: JsonRecord,
  limit = 8,
): ReadablePayloadField[] {
  const entries = Object.entries(record).filter(([key]) => !isInternalField(key));
  const ordered = [
    ...FIELD_PRIORITY
      .filter((key) => Object.prototype.hasOwnProperty.call(record, key))
      .map((key) => [key, record[key]] as [string, unknown]),
    ...entries.filter(([key]) => !FIELD_PRIORITY.includes(key)),
  ];
  const fields: ReadablePayloadField[] = [];
  const seen = new Set<string>();

  for (const [key, value] of ordered) {
    if (fields.length >= limit) break;
    if (seen.has(key) || isInternalField(key)) continue;
    seen.add(key);
    if (key === "content" && omittedPlaceholder(readableString(value))) continue;
    const text = fieldValueToText(key, value);
    if (!text) continue;
    fields.push({
      label: humanizePayloadLabel(key),
      value: compactText(text, 320),
    });
  }
  return fields;
}

function summarizeStructuredValue(value: unknown): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return "No items";
    return `${value.length} item${value.length === 1 ? "" : "s"}`;
  }
  const record = isReadableRecord(value) ? value : {};
  const keys = Object.keys(record).filter((key) => !isInternalField(key));
  if (keys.length === 0) return "";

  const direct = [
    readableString(record.summary),
    readableString(record.message),
    readableString(record.detail),
    readableString(record.title),
    readableString(record.content),
    readableString(record.error),
  ]
    .map((item) => compactText(item))
    .find((item) => item && !omittedPlaceholder(item));
  if (direct) return direct;

  return `${keys.slice(0, 4).map((key) => humanizePayloadLabel(key)).join(", ")}${
    keys.length > 4 ? ` +${keys.length - 4} more` : ""
  }`;
}

export function delegationRecordFromValue(value: unknown): JsonRecord | null {
  const parsed = parseReadableJson(value);
  const record = isReadableRecord(parsed) ? parsed : isReadableRecord(value) ? value : null;
  if (!record) return null;
  const kind = readableString(record.kind).trim().toLowerCase();
  if (kind.startsWith("delegation_")) return record;
  for (const key of ["data", "payload", "arguments", "args", "result"]) {
    const nested = record[key];
    const nestedRecord = isReadableRecord(nested)
      ? nested
      : isReadableRecord(parseReadableJson(nested))
        ? (parseReadableJson(nested) as JsonRecord)
        : null;
    if (
      nestedRecord &&
      readableString(nestedRecord.kind).trim().toLowerCase().startsWith("delegation_")
    ) {
      return nestedRecord;
    }
  }
  return null;
}

function delegationTone(kind: string, status: string, reason: string): ReadablePayloadTone {
  const combined = `${kind} ${status} ${reason}`.toLowerCase();
  if (/failed|timed_out|timeout|panicked|error|interrupted|cancel/.test(combined))
    return "error";
  if (/completed|complete|success|finished/.test(combined)) return "success";
  if (/assignment|assigned|synthesis|started|progress|running/.test(combined))
    return "running";
  return "idle";
}

function delegationSubject(record: JsonRecord): string {
  const agentName = readableString(record.agent_name).trim();
  const agentRole = readableString(record.agent_role).trim();
  if (agentName && agentRole) return `${agentName} / ${agentRole}`;
  return agentName || agentRole || "Agent swarm";
}

function delegationDetail(record: JsonRecord, kind: string): string {
  const status = readableString(record.status).trim();
  const reason = readableString(record.reason).trim();
  const summary = readableString(record.summary).trim();
  const content = readableString(record.content).trim();
  const task = readableString(record.task).trim();
  const elapsed = formatReadableDurationMs(record.elapsed_ms);
  const usableContent = content && !omittedPlaceholder(content) ? content : "";
  if (kind === "delegation_agent_failed") {
    if (/timeout|timed_out/i.test(`${status} ${reason}`)) {
      return "Timed out before returning a result.";
    }
    return reason || summary || "Delegated work failed.";
  }
  if (kind === "delegation_agent_completed") {
    return usableContent || summary || (elapsed ? `Completed in ${elapsed}.` : "Delegated work completed.");
  }
  if (kind === "delegation_assignment") {
    return task || summary || "Assignment prepared.";
  }
  if (kind === "delegation_agent_started") {
    return task || summary || "Delegated work started.";
  }
  if (kind === "delegation_agent_progress") {
    return usableContent || summary || task || "Delegated work is still running.";
  }
  if (kind === "delegation_started") {
    const count = readableNumber(record.agent_count, 0);
    return count > 0
      ? `Launching ${count} delegated agent${count === 1 ? "" : "s"}.`
      : "Launching delegated agents.";
  }
  if (kind === "delegation_synthesis_started") {
    return summary || "Combining delegated results into one answer.";
  }
  if (kind === "delegation_completed") {
    return summary || "Delegated work completed.";
  }
  return summary || usableContent || task || "Delegation update received.";
}

function delegationTitle(record: JsonRecord, kind: string): string {
  const subject = delegationSubject(record);
  switch (kind) {
    case "delegation_started":
      return "Launching agent swarm";
    case "delegation_assignment":
      return `Assigned ${subject}`;
    case "delegation_agent_started":
    case "delegation_agent_progress":
      return `${subject} is working`;
    case "delegation_agent_completed":
      return `${subject} completed`;
    case "delegation_agent_failed":
      return `${subject} needs attention`;
    case "delegation_synthesis_started":
      return "Synthesizing agent results";
    case "delegation_completed":
      return "Agent swarm completed";
    default:
      return humanizePayloadStatus(kind, "Delegation update");
  }
}

function delegationSummary(record: JsonRecord): ReadablePayloadSummary {
  const kind = readableString(record.kind).trim().toLowerCase();
  const status = readableString(record.status).trim();
  const reason = readableString(record.reason).trim();
  const fields = readableFieldsFromRecord(record, 8).filter(
    (field) => !["Kind", "Content"].includes(field.label),
  );
  return {
    title: delegationTitle(record, kind),
    detail: delegationDetail(record, kind),
    status: humanizePayloadStatus(status || reason || kind, "Updated"),
    tone: delegationTone(kind, status, reason),
    fields,
    isDelegation: true,
    isStructured: true,
  };
}

function genericStructuredSummary(value: unknown): ReadablePayloadSummary | null {
  if (Array.isArray(value)) {
    return {
      title: "Structured results",
      detail:
        value.length === 0
          ? "No items were returned."
          : `Collected ${value.length} item${value.length === 1 ? "" : "s"}.`,
      status: value.length === 0 ? "Empty" : "Ready",
      tone: "success",
      fields: value.slice(0, 6).map((item, index) => ({
        label: `Item ${index + 1}`,
        value: compactText(summarizeStructuredValue(item), 260),
      })).filter((field) => field.value),
      isDelegation: false,
      isStructured: true,
    };
  }

  const record = isReadableRecord(value) ? value : {};
  const keys = Object.keys(record);
  if (keys.length === 0) return null;
  const status = readableString(record.status).trim();
  const error = readableString(record.error).trim();
  const title =
    readableString(record.title).trim() ||
    humanizePayloadStatus(
      readableString(record.tool_name, readableString(record.name, readableString(record.kind))),
      "Structured update",
    );
  const detail =
    summarizeStructuredValue(record) ||
    (error ? humanizePayloadStatus(error) : "Received structured details.");
  return {
    title,
    detail,
    status: status ? humanizePayloadStatus(status) : undefined,
    tone: error || /fail|error|blocked/i.test(status) ? "error" : "success",
    fields: readableFieldsFromRecord(record, 10),
    isDelegation: false,
    isStructured: true,
  };
}

export function readablePayloadFromValue(value: unknown): ReadablePayloadSummary | null {
  const delegation = delegationRecordFromValue(value);
  if (delegation) return delegationSummary(delegation);
  const parsed = parseReadableJson(value);
  if (parsed == null) return null;
  return genericStructuredSummary(parsed);
}

export function readablePayloadCopyText(summary: ReadablePayloadSummary): string {
  return [
    summary.title,
    summary.detail,
    ...summary.fields.map((field) => `${field.label}: ${field.value}`),
  ]
    .filter(Boolean)
    .join("\n");
}
