import { formatUiDateTimeMeta } from "../../lib/dateFormat";
import type { ArkPulseRemediationSpec } from "../../types";
import { asRecord, num, str, toBool } from "./pageHelpers";

export const AUTO_APPROVE_ACTION_OPTIONS = [
  "web_search",
  "research",
  "generate_image",
  "generate_video",
  "browse",
  "file_read",
  "http_get",
  "schedule_task",
  "list_tasks",
  "clipboard_read",
  "clipboard_write",
  "gmail_scan",
  "gmail_reply",
] as const;

export const MODEL_FALLBACKS_BY_PROVIDER: Record<string, string[]> = {
  openai: ["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini", "o3"],
  anthropic: [
    "claude-opus-4-20250514",
    "claude-sonnet-4-20250514",
    "claude-3-7-sonnet-latest",
    "claude-3-5-haiku-latest",
  ],
  openrouter: [
    "openai/gpt-5",
    "anthropic/claude-sonnet-4",
    "google/gemini-2.5-pro",
  ],
  "openai-compatible": [],
  ollama: [],
};

export const SEARCH_API_PROVIDER_OPTIONS = [
  {
    id: "serper",
    label: "Serper",
    keyField: "search_serper_key",
    configuredField: "search_serper_configured",
    editingField: "search_serper_editing",
    clearField: "search_serper_clear",
  },
  {
    id: "brave_api",
    label: "Brave API",
    keyField: "search_brave_key",
    configuredField: "search_brave_configured",
    editingField: "search_brave_editing",
    clearField: "search_brave_clear",
  },
  {
    id: "exa",
    label: "Exa",
    keyField: "search_exa_key",
    configuredField: "search_exa_configured",
    editingField: "search_exa_editing",
    clearField: "search_exa_clear",
  },
  {
    id: "tavily",
    label: "Tavily",
    keyField: "search_tavily_key",
    configuredField: "search_tavily_configured",
    editingField: "search_tavily_editing",
    clearField: "search_tavily_clear",
  },
  {
    id: "perplexity",
    label: "Perplexity",
    keyField: "search_perplexity_key",
    configuredField: "search_perplexity_configured",
    editingField: "search_perplexity_editing",
    clearField: "search_perplexity_clear",
  },
  {
    id: "firecrawl",
    label: "Firecrawl",
    keyField: "search_firecrawl_key",
    configuredField: "search_firecrawl_configured",
    editingField: "search_firecrawl_editing",
    clearField: "search_firecrawl_clear",
  },
] as const;

export const SEARCH_PROVIDER_OPTIONS = [
  ...SEARCH_API_PROVIDER_OPTIONS,
  { id: "searxng", label: "SearXNG" },
] as const;

export function tunnelCheckAlertSeverity(
  status: unknown,
): "success" | "info" | "warning" | "error" {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass" || normalized === "healthy" || normalized === "ok") {
    return "success";
  }
  if (normalized === "fail" || normalized === "error" || normalized === "down") {
    return "error";
  }
  if (
    normalized === "warn" ||
    normalized === "warning" ||
    normalized === "degraded"
  ) {
    return "warning";
  }
  return "info";
}

export function tunnelCheckChipColor(
  status: unknown,
): "success" | "info" | "warning" | "error" | "default" {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass" || normalized === "healthy" || normalized === "ok") {
    return "success";
  }
  if (normalized === "fail" || normalized === "error" || normalized === "down") {
    return "error";
  }
  if (
    normalized === "warn" ||
    normalized === "warning" ||
    normalized === "degraded"
  ) {
    return "warning";
  }
  if (normalized === "info") return "info";
  return "default";
}

export function tunnelCheckLabel(status: unknown): string {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass") return "Ready";
  if (normalized === "fail") return "Needs action";
  if (normalized === "warn") return "Check";
  if (!normalized) return "Info";
  return normalized.charAt(0).toUpperCase() + normalized.slice(1);
}

export function isUserActionableDoctorFinding(value: unknown): boolean {
  const row = asRecord(value);
  if (!Object.prototype.hasOwnProperty.call(row, "user_actionable")) return true;
  return toBool(row.user_actionable);
}

export function parseArkPulseRemediationSpec(
  value: unknown,
): ArkPulseRemediationSpec | null {
  const row = asRecord(value);
  const kind = str(row.kind, "").trim().toLowerCase();
  if (!kind) return null;
  if (kind === "tunnel_start_verify") return { kind: "tunnel_start_verify" };
  if (kind === "tunnel_restart_verify") return { kind: "tunnel_restart_verify" };
  if (kind === "app_restart") {
    const appId = str(row.app_id, "").trim();
    if (!appId) return null;
    return { kind: "app_restart", app_id: appId };
  }
  if (kind === "readonly_investigation") {
    const topic = str(row.topic, "").trim().toLowerCase();
    if (topic === "memory_capture_health") {
      return { kind: "readonly_investigation", topic: "memory_capture_health" };
    }
    return null;
  }
  if (kind === "shell_command") {
    const command = str(row.command, "").trim();
    if (!command) return null;
    return { kind: "shell_command", command };
  }
  return null;
}

export function describeArkPulseRemediation(
  remediation: ArkPulseRemediationSpec | null,
): string {
  if (!remediation) return "-";
  if (remediation.kind === "tunnel_start_verify") {
    return "Start tunnel and verify /tunnel/status returns active + URL";
  }
  if (remediation.kind === "tunnel_restart_verify") {
    return "Restart tunnel and verify public reachability";
  }
  if (remediation.kind === "app_restart") {
    return `Restart app ${remediation.app_id} and re-check health`;
  }
  if (remediation.kind === "readonly_investigation") {
    return "Review failed memory captures and model health";
  }
  return remediation.command.trim() || "-";
}

function isArkPulseReadonlyInvestigation(
  remediation: ArkPulseRemediationSpec | null,
): boolean {
  return remediation?.kind === "readonly_investigation";
}

export function arkPulseRunActionLabel(
  remediation: ArkPulseRemediationSpec | null,
): string {
  return isArkPulseReadonlyInvestigation(remediation)
    ? "Run diagnostic"
    : "Run fix now";
}

export function arkPulseRemediationFootnote(
  remediation: ArkPulseRemediationSpec | null,
  canRunFix: boolean,
): string {
  if (!remediation) {
    return canRunFix
      ? "Legacy event: ArkPulse is using the saved fix command fallback for this run."
      : "This next step is advisory only. Copy it and run it manually.";
  }
  if (remediation.kind === "readonly_investigation") {
    return "Runs a read-only diagnostic from ArkPulse and returns a summary here.";
  }
  return "Runs directly from ArkPulse using the finding's typed remediation.";
}

function classifyLegacyRunnableArkPulseFixCommand(
  value: string,
): ArkPulseRemediationSpec | null {
  const normalized = (value || "").trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower === "-" || lower === "n/a" || lower === "none") return null;
  if (lower.includes("start tunnel") && lower.includes("/tunnel/status")) {
    return { kind: "tunnel_start_verify" };
  }
  if (lower.includes("restart") && lower.includes("tunnel")) {
    return { kind: "tunnel_restart_verify" };
  }
  const appRestartMatch = normalized.match(
    /^POST\s+\/api\/apps\/([A-Za-z0-9_-]+)\/restart$/i,
  );
  if (appRestartMatch?.[1]) {
    return { kind: "app_restart", app_id: appRestartMatch[1] };
  }
  if (
    lower.includes("\n") ||
    lower.includes("\r") ||
    lower.includes("||") ||
    lower.includes(";") ||
    lower.includes("`") ||
    lower.includes("$(")
  ) {
    return null;
  }
  const segments = normalized
    .split("&&")
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0);
  if (segments.length < 2) return null;
  const cdSegment = segments[0].toLowerCase();
  if (!cdSegment.startsWith("cd /app/data/apps/")) return null;
  const supported = segments.slice(1).every((segment) => {
    const seg = segment.toLowerCase();
    return (
      seg.startsWith("pip-compile requirements.txt") ||
      seg.startsWith("rg -n ") ||
      seg === "cargo generate-lockfile" ||
      seg.startsWith("npm pkg delete ") ||
      seg.startsWith("mv .env ")
    );
  });
  if (!supported) return null;
  return { kind: "shell_command", command: normalized };
}

export function getRunnableArkPulseRemediation(
  value: unknown,
): ArkPulseRemediationSpec | null {
  const row = asRecord(value);
  return (
    parseArkPulseRemediationSpec(row.remediation) ??
    classifyLegacyRunnableArkPulseFixCommand(str(row.fix_command, "").trim())
  );
}

export function getArkPulseFixText(value: unknown): string {
  const row = asRecord(value);
  const remediation = parseArkPulseRemediationSpec(row.remediation);
  if (remediation) return describeArkPulseRemediation(remediation);
  const fix = str(row.fix_command, "").trim();
  if (fix) return fix;
  return "-";
}

export function formatDurationClock(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m ${secs}s`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

export function looksLikeIsoTimestamp(value: string): boolean {
  const normalized = (value || "").trim();
  if (!/^\d{4}-\d{2}-\d{2}T/.test(normalized)) return false;
  const parsed = new Date(normalized);
  return !Number.isNaN(parsed.getTime());
}

export function formatTimestampForHumans(value: string): {
  label: string;
  tooltip: string;
} {
  const meta = formatUiDateTimeMeta(value, { fallback: value || "-" });
  return { label: meta.label, tooltip: meta.tip };
}

export function formatDurationFromSeconds(value: unknown): string {
  const total = num(value, -1);
  if (total < 0) return "-";
  const seconds = Math.floor(total);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remSeconds = seconds % 60;
  if (minutes < 60) {
    return remSeconds > 0 ? `${minutes}m ${remSeconds}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  if (hours < 24) {
    return remMinutes > 0 ? `${hours}h ${remMinutes}m` : `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `${days}d ${remHours}h` : `${days}d`;
}

export function charsLabel(value: unknown): string {
  const amount = num(value, -1);
  if (amount < 0) return "-";
  return `${Math.round(amount).toLocaleString()} chars`;
}

export function promptProposalStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved") return "success";
  if (normalized === "rejected") return "error";
  return "warning";
}

export function promptProposalRiskColor(
  risk: string,
): "default" | "success" | "warning" | "error" {
  const normalized = risk.trim().toLowerCase();
  if (normalized === "high") return "error";
  if (normalized === "medium") return "warning";
  if (normalized === "low") return "success";
  return "default";
}

export function promptCanarySafetyStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "auto_reverted" || normalized === "disabled_by_user") {
    return "success";
  }
  if (normalized === "kept_active") return "default";
  return "warning";
}

export function formatTraceDuration(durationMs: unknown): string {
  const milliseconds = num(durationMs, -1);
  if (milliseconds < 0) return "pending";
  if (milliseconds < 1000) return `${milliseconds}ms`;
  const totalSeconds = milliseconds / 1000;
  if (totalSeconds < 60) {
    return `${totalSeconds >= 10 ? totalSeconds.toFixed(0) : totalSeconds.toFixed(1)}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  return `${minutes}m ${seconds}s`;
}

export function collapseInlineWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

export function truncateUiText(value: string, maxChars = 120): string {
  const normalized = collapseInlineWhitespace(value);
  if (normalized.length <= maxChars) return normalized;
  return `${normalized.slice(0, Math.max(0, maxChars - 1)).trimEnd()}...`;
}

export function titleCaseLabel(value: string): string {
  return value
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(" ");
}
