export type RunPayloadView = {
  kind: "json" | "text";
  badgeLabel: string;
  headerLabel: string;
  preview: string;
  body: string;
  lineCount: number;
  items: RunPayloadItem[];
};

export type RunPayloadItem = {
  label: string;
  value: string;
};

type JsonRecord = Record<string, unknown>;

const PAYLOAD_STRING_MAX_CHARS = 1600;
const PAYLOAD_ARRAY_MAX_ITEMS = 80;
const PAYLOAD_OBJECT_MAX_KEYS = 80;
const PAYLOAD_DEPTH_MAX = 5;
const NESTED_SUMMARY_MAX_CHARS = 480;
const NESTED_SUMMARY_MAX_ITEMS = 12;
const NESTED_SUMMARY_MAX_KEYS = 12;
const NESTED_SUMMARY_DEPTH_MAX = 2;

const FIELD_PRIORITY = [
  "status",
  "state",
  "action",
  "op",
  "operation",
  "channel",
  "task",
  "at",
  "scheduled_for",
  "time",
  "path",
  "file",
  "url",
  "query",
  "command",
  "result",
  "output",
  "message",
  "summary",
  "error",
  "elapsed_ms",
  "duration_ms",
  "kind",
  "tool_name",
  "name",
  "content",
];

const INTERNAL_KEYS = new Set([
  "__omitted_keys",
  "__streamKey",
  "_automation",
  "agent_id",
  "background_session_id",
  "call_id",
  "chat_visible",
  "conversation_id",
  "conversationId",
  "delegation_id",
  "id",
  "message_id",
  "parent_step_id",
  "plan_id",
  "plan_revision",
  "plan_step_id",
  "run_id",
  "runId",
  "sequence",
  "task_id",
  "taskId",
  "trace_id",
  "traceId",
  "ts",
  "timestamp",
]);

const FORCE_SHOW_KEYS = new Set([
  "action",
  "args",
  "arguments",
  "content",
  "error",
  "input",
  "output",
  "payload",
  "result",
  "status",
  "task",
]);

const FLATTEN_KEYS = new Set([
  "args",
  "arguments",
  "content",
  "input",
  "params",
  "payload",
]);

const SECRET_KEY_PATTERN =
  /(?:^|[_-])(?:access_password|password|passcode|secret|token|api_key|apikey|private_key|client_secret|refresh_token)(?:$|[_-])/i;

function isRecord(value: unknown): value is JsonRecord {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function parseJson(value: unknown): unknown | null {
  if (isRecord(value) || Array.isArray(value)) return value;
  if (typeof value !== "string") return null;
  const text = value.trim();
  if (!text || (text[0] !== "{" && text[0] !== "[")) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function compactText(value: string, maxLen = PAYLOAD_STRING_MAX_CHARS): string {
  const text = (value || "").replace(/\s+/g, " ").trim();
  if (text.length <= maxLen) return text;
  return `${text.slice(0, Math.max(0, maxLen - 3)).trimEnd()}...`;
}

function omittedPlaceholder(value: string): boolean {
  return /^\[omitted\s+[\d,]+\s+chars?(?:\s*\/\s*[\d,]+\s+lines?)?\]$/i.test(
    value.trim(),
  );
}

export function humanizeRunPayloadLabel(value: string, fallback = "Value"): string {
  const normalized = (value || "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized) return fallback;
  const direct: Record<string, string> = {
    at: "At",
    "api key": "API Key",
    "api token": "API Token",
    "duration ms": "Duration",
    "elapsed ms": "Elapsed",
    op: "Operation",
    url: "URL",
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

function humanizeStatus(value: unknown, fallback = "Updated"): string {
  const raw = typeof value === "string" ? value : String(value ?? "");
  const text = raw.trim();
  if (!text) return fallback;
  return text
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

function formatDurationMs(value: unknown): string {
  const ms =
    typeof value === "number"
      ? value
      : typeof value === "string" && value.trim()
        ? Number(value)
        : 0;
  if (!Number.isFinite(ms) || ms <= 0) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(seconds >= 10 ? 0 : 1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.round(seconds % 60);
  return remaining > 0 ? `${minutes}m ${remaining}s` : `${minutes}m`;
}

function isInternalKey(key: string): boolean {
  return !key || key.startsWith("__") || INTERNAL_KEYS.has(key);
}

function orderedEntries(record: JsonRecord): Array<[string, unknown]> {
  const entries = Object.entries(record).filter(([key]) => !isInternalKey(key));
  const priority = FIELD_PRIORITY
    .filter((key) => Object.prototype.hasOwnProperty.call(record, key))
    .map((key) => [key, record[key]] as [string, unknown]);
  const rest = entries.filter(([key]) => !FIELD_PRIORITY.includes(key));
  return [...priority, ...rest];
}

function summarizeStructuredValue(value: unknown, depth = 0): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return "No items";
    return `${value.length} item${value.length === 1 ? "" : "s"}`;
  }
  const record = asRecord(value);
  const keys = Object.keys(record).filter((key) => !isInternalKey(key));
  if (keys.length === 0) return "";

  const direct = [
    record.summary,
    record.message,
    record.detail,
    record.title,
    record.content,
    record.error,
  ]
    .filter((item): item is string => typeof item === "string")
    .map((item) => compactText(item, NESTED_SUMMARY_MAX_CHARS))
    .find((item) => item && !omittedPlaceholder(item));
  if (direct) return direct;

  if (depth >= NESTED_SUMMARY_DEPTH_MAX) {
    return `${keys.slice(0, NESTED_SUMMARY_MAX_KEYS).map((key) => humanizeRunPayloadLabel(key)).join(", ")}${
      keys.length > NESTED_SUMMARY_MAX_KEYS
        ? ` +${keys.length - NESTED_SUMMARY_MAX_KEYS} more`
        : ""
    }`;
  }

  const scalarPairs = orderedEntries(record)
    .slice(0, NESTED_SUMMARY_MAX_ITEMS)
    .map(([key, item]) => {
      const text = fieldValueToText(key, item, depth + 1);
      return text ? `${humanizeRunPayloadLabel(key)}: ${text}` : "";
    })
    .filter(Boolean);
  if (scalarPairs.length > 0 && scalarPairs.length <= 2) {
    return compactText(scalarPairs.join(", "), NESTED_SUMMARY_MAX_CHARS);
  }
  return `${keys.slice(0, NESTED_SUMMARY_MAX_KEYS).map((key) => humanizeRunPayloadLabel(key)).join(", ")}${
    keys.length > NESTED_SUMMARY_MAX_KEYS
      ? ` +${keys.length - NESTED_SUMMARY_MAX_KEYS} more`
      : ""
  }`;
}

function fieldValueToText(key: string, value: unknown, depth = 0): string {
  if (SECRET_KEY_PATTERN.test(key)) return "[redacted]";
  if (value == null) return "";
  if (typeof value === "string") {
    const parsed = parseJson(value);
    if (parsed != null) return summarizeStructuredValue(parsed, depth);
    const text = compactText(value);
    return omittedPlaceholder(text) ? "" : text;
  }
  if (typeof value === "number") {
    if (/elapsed|duration|latency/i.test(key)) return formatDurationMs(value);
    return Number.isInteger(value) ? value.toLocaleString() : String(value);
  }
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (depth >= PAYLOAD_DEPTH_MAX) return summarizeStructuredValue(value, depth);
  if (Array.isArray(value)) {
    return value.length === 0
      ? "No items"
      : `${Math.min(value.length, PAYLOAD_ARRAY_MAX_ITEMS)} item${value.length === 1 ? "" : "s"}`;
  }
  return summarizeStructuredValue(value, depth);
}

function buildPayloadItems(value: unknown, limit = 12): RunPayloadItem[] {
  const out: RunPayloadItem[] = [];
  const addItems = (source: unknown, depth = 0): void => {
    if (out.length >= limit || depth > PAYLOAD_DEPTH_MAX) return;
    const record = asRecord(source);
    for (const [key, raw] of orderedEntries(record).slice(0, PAYLOAD_OBJECT_MAX_KEYS)) {
      if (out.length >= limit) break;
      if (isInternalKey(key)) continue;
      const nested = asRecord(raw);
      if (
        depth < 2 &&
        FLATTEN_KEYS.has(key.trim().toLowerCase()) &&
        Object.keys(nested).length > 0
      ) {
        addItems(nested, depth + 1);
        continue;
      }
      const valueText = fieldValueToText(key, raw, depth);
      if (!valueText) continue;
      out.push({
        label: humanizeRunPayloadLabel(key),
        value: compactText(valueText, 320),
      });
    }
  };

  if (Array.isArray(value)) {
    value.slice(0, 6).forEach((entry, index) => {
      const text = fieldValueToText(`item_${index + 1}`, entry);
      if (!text) return;
      out.push({
        label: `Item ${index + 1}`,
        value: compactText(text, 320),
      });
    });
  } else {
    addItems(value);
  }
  return out;
}

function valueToPreview(value: unknown): string {
  const items = buildPayloadItems(value, 4);
  if (items.length > 0) {
    return items
      .map((item) => `${item.label}: ${item.value}`)
      .join(" | ");
  }
  const summary = summarizeStructuredValue(value);
  return summary || "Structured activity captured.";
}

function shouldTreatAsTextPayload(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  if (parseJson(trimmed) != null) return true;
  if (/^<!doctype html/i.test(trimmed) || /^<html\b/i.test(trimmed)) return true;
  if (/^(from\s+\w+\s+import|import\s+[\w.{},* ]+|def\s+\w+\(|class\s+\w+|async\s+def\s+\w+\()/m.test(trimmed)) {
    return true;
  }
  if (/^(const|let|var|function|export|import)\s/m.test(trimmed)) return true;
  if (trimmed.length >= 80 && /[\r\n]/.test(trimmed)) return true;
  return trimmed.length >= 220;
}

function shouldShowStructuredPayload(value: unknown, body: string): boolean {
  if (Array.isArray(value)) return true;
  const record = asRecord(value);
  const keys = Object.keys(record);
  const visibleKeys = keys.filter((key) => !isInternalKey(key));
  if (visibleKeys.length === 0) return false;
  const hasNested = visibleKeys.some((key) => {
    const raw = record[key];
    return Array.isArray(raw) || isRecord(raw) || parseJson(raw) != null;
  });
  return (
    hasNested ||
    visibleKeys.length > 4 ||
    body.length > 220 ||
    keys.some((key) => FORCE_SHOW_KEYS.has(key))
  );
}

export function buildRunPayloadView(value: unknown): RunPayloadView | null {
  if (value == null) return null;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const parsed = parseJson(trimmed);
    if (parsed != null) return buildRunPayloadView(parsed);
    if (!shouldTreatAsTextPayload(trimmed)) return null;
    return {
      kind: "text",
      badgeLabel: "Output",
      headerLabel: "Text output",
      preview: compactText(trimmed, 180),
      body: trimmed,
      lineCount: trimmed.split(/\r?\n/).length,
      items: [],
    };
  }

  if (!Array.isArray(value) && !isRecord(value)) return null;
  const body = JSON.stringify(value, null, 2);
  if (!body || !shouldShowStructuredPayload(value, body)) return null;
  const items = buildPayloadItems(value);
  return {
    kind: "json",
    badgeLabel: "Details",
    headerLabel: items.length > 0 ? "Readable summary" : "Structured activity",
    preview: valueToPreview(value),
    body,
    lineCount: body.split(/\r?\n/).length,
    items,
  };
}

export function buildRunPayloadViewFromSources(
  ...values: unknown[]
): RunPayloadView | null {
  for (const value of values) {
    const view = buildRunPayloadView(value);
    if (view) return view;
  }
  return null;
}
