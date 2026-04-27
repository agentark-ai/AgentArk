import { isRecord, str, type JsonRecord } from "./pageHelpers";

export const REFRESH_MS = 8000;
export const EVOLUTION_DEV_QUERY_LIMIT = 250;
export const EVOLUTION_DEV_REFRESH_MS = 30000;
export const IMPORT_SECURITY_FORCE_RISK_THRESHOLD = 8;
export const DEVELOPER_MODE_EVENT = "agentark:developer-mode-change";
export const OLLAMA_DEFAULT_BASE_URL = "http://localhost:11434";
export const OPENROUTER_DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
export const SHOW_EXPERIMENTAL_AUTONOMY_TOOLS = false;

const DEVELOPER_MODE_STORAGE_KEY = "agentark.developer_mode";

export function getDeveloperModeEnabled(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(DEVELOPER_MODE_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

export function setDeveloperModeEnabled(next: boolean): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(DEVELOPER_MODE_STORAGE_KEY, next ? "1" : "0");
  } catch {
    // Ignore storage write errors and still emit an event for the current session.
  }
  window.dispatchEvent(
    new CustomEvent(DEVELOPER_MODE_EVENT, { detail: { enabled: next } }),
  );
}

export function asRecords(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord);
}

export function boolText(value: unknown): string {
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "string") return value;
  if (typeof value === "number") return value === 0 ? "false" : "true";
  return "false";
}

export function humanizeStatusLabel(value: string): string {
  const normalized = value.trim();
  if (!normalized) return "-";
  return normalized.replace(/_/g, " ");
}

export function dedupeStrings(values: string[]): string[] {
  return Array.from(new Set(values.map((item) => item.trim()).filter(Boolean)));
}

export type HookTriggerValue =
  | "pre_message"
  | "post_message"
  | "pre_action"
  | "post_action"
  | "on_consolidate"
  | "on_error";

export function sanitizeHookName(value: string): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9-_\s]/g, "")
    .replace(/[_\s]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

export function inferHookTriggerFromInstruction(
  text: string,
  defaultTrigger: HookTriggerValue = "post_action",
): HookTriggerValue {
  const normalized = (text || "").toLowerCase();
  if (!normalized) return defaultTrigger;
  if (
    normalized.includes("on error") ||
    normalized.includes("error") ||
    normalized.includes("fail")
  ) {
    return "on_error";
  }
  if (
    normalized.includes("before action") ||
    normalized.includes("pre action")
  ) {
    return "pre_action";
  }
  if (
    normalized.includes("after action") ||
    normalized.includes("post action") ||
    normalized.includes("on success") ||
    normalized.includes("when done")
  ) {
    return "post_action";
  }
  if (
    normalized.includes("before message") ||
    normalized.includes("pre message")
  ) {
    return "pre_message";
  }
  if (
    normalized.includes("after message") ||
    normalized.includes("post message") ||
    normalized.includes("after reply")
  ) {
    return "post_message";
  }
  if (
    normalized.includes("consolidate") ||
    normalized.includes("memory")
  ) {
    return "on_consolidate";
  }
  return defaultTrigger;
}

export function extractFirstUrl(text: string): string {
  const match = (text || "").match(/https?:\/\/[^\s]+/i);
  return match ? match[0] : "";
}

function extractCronExpression(text: string): string {
  const tokens = (text || "").trim().split(/\s+/).filter(Boolean);
  const isCronToken = (token: string) => /^[0-9A-Za-z*/,\-]+$/.test(token);
  for (let index = 0; index < tokens.length; index += 1) {
    for (const width of [6, 5]) {
      if (index + width > tokens.length) continue;
      const slice = tokens.slice(index, index + width);
      if (slice.every(isCronToken)) return slice.join(" ");
    }
  }
  return "";
}

export function inferTaskCronFromInstruction(text: string): string {
  const normalized = (text || "").trim().toLowerCase();
  if (!normalized) return "";
  const explicitCron = extractCronExpression(text);
  if (explicitCron) return explicitCron;
  if (normalized.includes("every 5") && normalized.includes("min"))
    return "*/5 * * * *";
  if (normalized.includes("every 10") && normalized.includes("min"))
    return "*/10 * * * *";
  if (normalized.includes("every 15") && normalized.includes("min"))
    return "*/15 * * * *";
  if (normalized.includes("every 30") && normalized.includes("min"))
    return "*/30 * * * *";
  if (normalized.includes("hourly") || normalized.includes("every hour"))
    return "0 * * * *";
  if (normalized.includes("weekday")) return "0 9 * * 1-5";
  if (normalized.includes("weekly")) return "0 9 * * 1";
  if (normalized.includes("monthly")) return "0 9 1 * *";
  if (normalized.includes("daily") || normalized.includes("every day"))
    return "0 9 * * *";
  return "";
}

export function isHookAttachedToAction(
  hookName: string,
  actionName: string,
): boolean {
  const normalizedHookName = sanitizeHookName(hookName);
  const normalizedActionName = sanitizeHookName(actionName);
  if (!normalizedHookName || !normalizedActionName) return false;
  return normalizedHookName.startsWith(`action-${normalizedActionName}-`);
}

export function isHookRecordAttachedToAction(
  hook: JsonRecord,
  actionName: string,
): boolean {
  const explicit = sanitizeHookName(str(hook.action_name, ""));
  const normalizedActionName = sanitizeHookName(actionName);
  if (explicit && normalizedActionName && explicit === normalizedActionName) {
    return true;
  }
  return isHookAttachedToAction(str(hook.name, ""), actionName);
}
