import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import CloseIcon from "@mui/icons-material/Close";
import LockRoundedIcon from "@mui/icons-material/LockRounded";
import MemoryRoundedIcon from "@mui/icons-material/MemoryRounded";
import WarningAmberRoundedIcon from "@mui/icons-material/WarningAmberRounded";
import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { isBackgroundSessionVisibleInUi } from "../lib/backgroundSessions";
import { formatUiDateTime, formatUiRelativeDateTimeMeta } from "../lib/dateFormat";
import { humanizeMachineLabel, humanizeStatusLabel } from "../lib/displayLabels";
import {
  metricValues,
  readRuntimeMetricHistory,
  RUNTIME_METRIC_HISTORY_EVENT,
  type RuntimeMetricSample,
} from "../lib/runtimeMetricHistory";
import { buildCumulativeSavedTokenSparkValues } from "./overviewArkDistillSpark";
import { useUiStore } from "../store/uiStore";
import { ActivityFeed } from "./ActivityFeed";
import { SuggestionRunDialog, type SuggestionRunState } from "./SuggestionRunDialog";
import { AgentCognitionLoop } from "./missionControl";
import type {
  AutonomyActionExecutionResponse,
  BackgroundSessionSummary,
  BriefingResponse,
  LlmAnalyticsResponse,
  RecommendedAction,
  Task,
  TraceSummary,
  Notification,
} from "../types";

const REFRESH_MS = 8000;
const ACTIVE_TASK_STALE_MS = 24 * 60 * 60 * 1000;
const RUNTIME_TREND_WINDOW_MS = 6 * 60 * 60 * 1000;
type JsonRecord = Record<string, unknown>;
type AutomationObject = {
  id: string;
  kind: string;
  title: string;
  subtitle?: string | null;
  status: string;
  detail?: string | null;
  created_at?: string | null;
  next_run_at?: string | null;
  view: string;
  url?: string | null;
  enabled?: boolean | null;
  connected?: boolean | null;
};

type AutomationRun = {
  id: string;
  automation_id: string;
  kind: string;
  title: string;
  action: string;
  trigger: string;
  status: string;
  current_status?: string | null;
  attempt: number;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  summary: string;
  output_preview?: string | null;
  error?: string | null;
  next_retry_at?: string | null;
  conversation_id?: string | null;
  view: string;
};

type DailyBriefRunDialogState = {
  outcome: "running" | "success" | "error";
  title: string;
  detail: string;
  brief: string;
  triggered_at: string;
  result?: Record<string, unknown>;
};

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  const root = asRecord(value);
  const items = root[key];
  if (!Array.isArray(items)) return [];
  return items.filter((item) => item && typeof item === "object" && !Array.isArray(item)) as JsonRecord[];
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function pickAutomationObjects(raw: unknown): AutomationObject[] {
  const root = asRecord(raw);
  const items = root.objects;
  return Array.isArray(items) ? (items as AutomationObject[]) : [];
}

function pickAutomationRuns(raw: unknown): AutomationRun[] {
  const root = asRecord(raw);
  const items = root.runs;
  return Array.isArray(items) ? (items as AutomationRun[]) : [];
}

function isActiveIntegration(item: AutomationObject): boolean {
  if (String(item.kind || "").toLowerCase() !== "integration") return false;
  const status = String(item.status || "").toLowerCase();
  return (item.connected === true || status === "connected") && item.enabled !== false;
}

function automationKindLabel(kind: string): string {
  const normalized = (kind || "").toLowerCase();
  if (normalized === "task") return "Task";
  if (normalized === "watcher") return "Watcher";
  if (normalized === "app") return "App";
  if (normalized === "integration") return "Integration";
  return humanizeMachineLabel(kind, "Automation");
}

function automationStatusColor(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = (status || "").toLowerCase();
  if (["running", "active", "connected", "completed", "triggered"].some((token) => normalized.includes(token))) {
    return "success";
  }
  if (["pending", "paused", "awaiting", "needs_auth", "not_configured"].some((token) => normalized.includes(token))) {
    return "warning";
  }
  if (["failed", "error", "cancelled", "stopped", "timed_out"].some((token) => normalized.includes(token))) {
    return "error";
  }
  if (normalized.includes("in_progress")) return "info";
  return "default";
}

function formatAutomationTime(value?: string | null): string {
  return formatUiDateTime(value, { fallback: "-" });
}

function targetViewForAutomation(item: AutomationObject): string {
  const explicitView = String(item.view || "").trim();
  if (explicitView) return explicitView;
  const kind = String(item.kind || "").toLowerCase();
  if (kind === "integration") return "settings";
  if (kind === "watcher") return "watchers";
  if (kind === "task") return "tasks";
  if (kind === "app") return "apps";
  if (kind === "session") return "sessions";
  return "trace";
}

function targetViewForAutomationRun(item: AutomationRun): string {
  const explicitView = String(item.view || "").trim();
  if (explicitView) return explicitView;
  const kind = String(item.kind || "").toLowerCase();
  if (kind === "watcher") return "watchers";
  if (kind === "task") return "tasks";
  if (kind === "app") return "apps";
  if (kind === "integration") return "settings";
  if (kind === "session") return "sessions";
  return "trace";
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return "Request failed.";
}

function humanTs(value: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(value, { fallback: "-" });
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function formatCompactPercent(value: unknown): string {
  const next = finiteNumber(value);
  return next == null ? "-" : `${Math.round(next)}%`;
}

function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return Math.round(value).toLocaleString();
}

function sparklinePoints(values: number[], width = 132, height = 30): string {
  const step = Math.max(1, Math.ceil(values.length / 120));
  const compactValues = values.length > 120 ? values.filter((_, index) => index % step === 0).slice(-120) : values;
  const usable = compactValues.length >= 2 ? compactValues : compactValues.length === 1 ? [compactValues[0], compactValues[0]] : [0, 0];
  const min = Math.min(...usable);
  const max = Math.max(...usable);
  const range = Math.max(1, max - min);
  return usable
    .map((value, index) => {
      const x = (index / Math.max(1, usable.length - 1)) * width;
      const y = height - ((value - min) / range) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

// Compact status badge for activity-stream rows: collapses verbose statuses
// into HUD-sized words and picks the warn/crit tint.
function traceStatusBadge(status: string): { label: string; cls: string } {
  const s = String(status || "").toLowerCase();
  if (s.includes("fail") || s.includes("error") || s.includes("timed_out") || s.includes("denied")) {
    return { label: "FAIL", cls: " nw-syn-trow-st--crit" };
  }
  if (
    s.includes("needs") ||
    s.includes("warning") ||
    s.includes("issue") ||
    s.includes("not_configured") ||
    s.includes("pending")
  ) {
    return { label: String(status || "").toUpperCase().slice(0, 10), cls: " nw-syn-trow-st--warn" };
  }
  if (s.includes("running") || s.includes("progress") || s.includes("active") || s.includes("live")) {
    return { label: "LIVE", cls: "" };
  }
  if (s.includes("done") || s.includes("completed") || s.includes("ok") || s.includes("success")) {
    return { label: "OK", cls: "" };
  }
  return { label: String(status || "-").toUpperCase().slice(0, 10), cls: "" };
}

function MetricSparkline({ values }: { values: number[] }) {
  const points = sparklinePoints(values);
  return (
    <svg
      className={`nw-metric-spark${points ? "" : " nw-metric-spark--empty"}`}
      viewBox="0 0 132 30"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      {points ? <polyline points={points} /> : <line x1="0" y1="15" x2="132" y2="15" />}
    </svg>
  );
}

function ArkDistillSpark({ values }: { values: number[] }) {
  if (values.length < 2) {
    return (
      <svg
        className="nw-focus-spark nw-focus-spark--empty"
        viewBox="0 0 200 40"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <line x1="0" y1="20" x2="200" y2="20" />
      </svg>
    );
  }
  const points = sparklinePoints(values, 200, 40);
  return (
    <svg
      className="nw-focus-spark"
      viewBox="0 0 200 40"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <polyline points={points} />
    </svg>
  );
}

function formatCompactUptime(seconds: unknown): string {
  const total = finiteNumber(seconds);
  if (total == null) return "-";
  const safe = Math.max(0, Math.floor(total));
  const days = Math.floor(safe / 86400);
  const hours = Math.floor((safe % 86400) / 3600);
  const minutes = Math.floor((safe % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function pickModelDisplay(settingsData: unknown): string {
  const settings = asRecord(settingsData);
  const slots = Array.isArray(settings.model_pool)
    ? settings.model_pool.map(asRecord)
    : [];
  const primary =
    slots.find((slot) => Boolean(slot.enabled) && str(slot.role, "").toLowerCase() === "primary") ||
    slots.find((slot) => Boolean(slot.enabled)) ||
    slots[0];
  const label = str(primary?.label, "").trim();
  const model = str(primary?.model, "").trim();
  const legacyModel = str(settings.llm_model, "").trim();
  return label || model || legacyModel || "Not configured";
}

function traceStepColor(stepType: string): "default" | "success" | "warning" | "error" | "info" {
  const normalized = stepType.trim().toLowerCase();
  if (["success", "completed", "done"].includes(normalized)) return "success";
  if (["warning", "pending", "queued", "approval"].includes(normalized)) return "warning";
  if (["error", "failed", "failure"].includes(normalized)) return "error";
  if (["action", "tool", "thinking"].includes(normalized)) return "info";
  return "default";
}

function buildSuggestionTraceConsoleView(step: JsonRecord): { detail: string; dataText: string } {
  const detail = str(step.detail, str(step.title, "")).trim();
  const data = step.data;
  if (typeof data === "string") {
    return { detail, dataText: data.trim() };
  }
  if (data && typeof data === "object") {
    return { detail, dataText: JSON.stringify(data, null, 2) };
  }
  return { detail, dataText: "" };
}

function summarizeSuggestedRun(action: RecommendedAction, out: AutonomyActionExecutionResponse): string {
  const result = asRecord(out.result);
  const resultStatus = str(result.status, out.status).trim().toLowerCase();
  if (resultStatus === "queued_for_approval") {
    return `Queued "${action.title}" for approval.`;
  }
  const message = str(out.message, "").trim();
  if (message) return message;
  const response = str(result.response, "").trim();
  if (response) return response;
  const brief = str(result.brief, "").trim();
  if (brief) return "Generated a daily brief and attempted delivery.";
  const taskId = str(result.task_id, "").trim();
  if (taskId) {
    return result.reused_existing === true ? `Reused existing task ${taskId}.` : `Created task ${taskId}.`;
  }
  return `Ran "${action.title}".`;
}

function isDailyBriefSignal(value: unknown): boolean {
  const text = String(value || "").toLowerCase();
  return (
    text.includes("daily brief") ||
    text.includes("daily briefing") ||
    text.includes("daily command brief")
  );
}

function pickDailyBriefNotification(
  notifications: Notification[],
  triggeredAt?: string
): Notification | null {
  const triggeredMs = Date.parse(triggeredAt || "");
  const matching = notifications.filter((item) => {
    const source = String(item.source || "").toLowerCase();
    return (
      source.includes("daily_brief") ||
      isDailyBriefSignal(item.title) ||
      isDailyBriefSignal(item.body)
    );
  });
  if (matching.length === 0) return null;
  if (Number.isFinite(triggeredMs)) {
    const near = matching.find((item) => Date.parse(item.created_at || "") >= triggeredMs - 60_000);
    if (near) return near;
  }
  return matching[0] || null;
}

function pickDailyBriefTrace(traces: TraceSummary[], triggeredAt?: string): TraceSummary | null {
  const triggeredMs = Date.parse(triggeredAt || "");
  const matching = traces.filter((trace) => isDailyBriefSignal(trace.message_preview) || isDailyBriefSignal(trace.channel));
  if (matching.length === 0) return null;
  if (Number.isFinite(triggeredMs)) {
    const near = matching.find((trace) => Date.parse(trace.started_at || "") >= triggeredMs - 60_000);
    if (near) return near;
  }
  return matching[0] || null;
}

function pickDailyBriefAutomationRun(runs: AutomationRun[], triggeredAt?: string): AutomationRun | null {
  const triggeredMs = Date.parse(triggeredAt || "");
  const matching = runs.filter((run) => {
    return (
      isDailyBriefSignal(run.title) ||
      isDailyBriefSignal(run.summary) ||
      isDailyBriefSignal(run.action) ||
      isDailyBriefSignal(run.trigger)
    );
  });
  if (matching.length === 0) return null;
  if (Number.isFinite(triggeredMs)) {
    const near = matching.find((run) => Date.parse(run.started_at || "") >= triggeredMs - 60_000);
    if (near) return near;
  }
  return matching[0] || null;
}

function parseDailyBriefRunResponse(
  out: AutonomyActionExecutionResponse,
  triggeredAt: string
): DailyBriefRunDialogState {
  const result = out.result && typeof out.result === "object" ? out.result : {};
  const brief = String(result.brief || "").trim();
  const executionStatus = String(result.status || out.status || "").trim();
  const title =
    executionStatus === "queued_for_approval"
      ? "Daily Brief queued for approval"
      : "Daily Brief generated";
  const detail =
    executionStatus === "queued_for_approval"
      ? "AgentArk queued the brief-related action for approval. Review it in Tasks."
      : out.queued
        ? "AgentArk accepted the request and queued the run. Evidence will appear below as it lands."
        : "AgentArk generated the brief, logged an in-app notification, and attempted delivery to your preferred channel.";
  return {
    outcome: "success",
    title,
    detail,
    brief,
    triggered_at: triggeredAt,
    result,
  };
}

function isFreshInProgressTask(task: Task): boolean {
  const status = String(task?.status || "").toLowerCase();
  if (!status.includes("inprogress")) return false;
  const createdAt = Date.parse(String(task?.created_at || ""));
  if (Number.isNaN(createdAt)) return true;
  return Date.now() - createdAt <= ACTIVE_TASK_STALE_MS;
}

type Props = {
  navigateToView: (view: string, replace?: boolean) => void;
  serverStatus?: { at: number; rtt_ms: number; status: import("../types").StatusResponse };
  serverError: boolean;
  serverLoading: boolean;
};

export function OverviewPane({ navigateToView, serverStatus, serverError }: Props) {
  const queryClient = useQueryClient();
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const interval = autoRefresh ? REFRESH_MS : false;
  const [inventoryOpen, setInventoryOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [dailyBriefDialogOpen, setDailyBriefDialogOpen] = useState(false);
  const [dailyBriefRun, setDailyBriefRun] = useState<DailyBriefRunDialogState | null>(null);
  const [suggestionRun, setSuggestionRun] = useState<SuggestionRunState | null>(null);
  const [suggestionRunOpen, setSuggestionRunOpen] = useState(false);
  const [suggestionRunMinimized, setSuggestionRunMinimized] = useState(false);

  // --- Data hooks ---
  const tasksQ = useQuery({ queryKey: ["tasks"], queryFn: api.getTasks, refetchInterval: interval });
  const traceQ = useQuery({ queryKey: ["trace"], queryFn: api.getTrace, refetchInterval: interval });
  const briefingQ = useQuery({ queryKey: ["briefing"], queryFn: api.getBriefing, refetchInterval: interval });
  const notificationsQ = useQuery({ queryKey: ["notifications"], queryFn: api.getNotifications, refetchInterval: interval });
  const securityQ = useQuery({
    queryKey: ["security-logs-dashboard"],
    queryFn: () => api.getSecurityLogs(5),
    refetchInterval: autoRefresh ? 30_000 : false,
  });
  const automationQ = useQuery({
    queryKey: ["automation-objects-dashboard"],
    queryFn: () => api.rawGet("/automation/objects"),
    refetchInterval: interval,
  });
  const automationRunsQ = useQuery({
    queryKey: ["automation-runs-dashboard"],
    queryFn: () => api.rawGet("/automation/runs"),
    refetchInterval: interval,
  });
  const sessionsQ = useQuery({
    queryKey: ["background-sessions-dashboard"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: interval,
  });
  const settingsQ = useQuery({
    queryKey: ["settings-dashboard"],
    queryFn: api.getSettings,
    refetchInterval: false,
    staleTime: 60_000,
  });
  const autonomySettingsQ = useQuery({
    queryKey: ["autonomy-settings-dashboard"],
    queryFn: () => api.rawGet("/autonomy/settings"),
    refetchInterval: interval,
    staleTime: 10_000,
  });
  const evolutionQ = useQuery({
    queryKey: ["evolution-status-dashboard"],
    queryFn: () => api.rawGet("/settings/evolution"),
    refetchInterval: autoRefresh ? 60_000 : false,
    staleTime: 45_000,
  });
  const arkDistillAnalyticsQ = useQuery({
    queryKey: ["mission-control-llm-analytics-30d"],
    queryFn: () => api.getLlmAnalytics({ range: "30d", bucket: "day" }),
    refetchInterval: autoRefresh ? 120_000 : false,
    staleTime: 60_000,
  });
  const suggestionTraceId = (suggestionRun?.traceId || "").trim();
  const suggestionTraceQ = useQuery({
    queryKey: ["overview-suggestion-trace", suggestionTraceId],
    queryFn: () => api.rawGet(`/trace/${encodeURIComponent(suggestionTraceId)}`),
    enabled: !!suggestionTraceId && suggestionRunOpen,
    refetchInterval:
      suggestionRunOpen && !!suggestionTraceId && suggestionRun?.status === "running" ? REFRESH_MS : false,
  });

  // --- Derived data ---
  const tasks = Array.isArray(tasksQ.data) ? tasksQ.data : [];
  const traces = traceQ.data?.history || [];
  const traceEvents = traceQ.data?.recent_events || [];
  const runtimeHealth = serverStatus?.status?.runtime_health ?? null;
  const memoryPressureValue = runtimeHealth?.memory_pressure_percent ?? runtimeHealth?.ram_percent;
  const modelDisplay = useMemo(() => pickModelDisplay(settingsQ.data), [settingsQ.data]);
  const [runtimeMetricHistory, setRuntimeMetricHistory] = useState<RuntimeMetricSample[]>(() => readRuntimeMetricHistory());
  const runtimeTrendHistory = useMemo(() => {
    const cutoff = Date.now() - RUNTIME_TREND_WINDOW_MS;
    return runtimeMetricHistory.filter((sample) => sample.t >= cutoff);
  }, [runtimeMetricHistory]);
  const memoryPressureHistory = useMemo(
    () => metricValues(runtimeTrendHistory, "memoryPressure"),
    [runtimeTrendHistory]
  );
  const latencyHistory = useMemo(
    () => metricValues(runtimeTrendHistory, "latencyMs"),
    [runtimeTrendHistory]
  );
  const notifications = Array.isArray(notificationsQ.data) ? notificationsQ.data : [];
  const securityLogs = (securityQ.data as { logs?: Array<{ event_type: string; severity: string; message: string }> })?.logs || [];
  const suggestionTrace = asRecord(suggestionTraceQ.data);
  const suggestionTraceSteps = pickRecords(suggestionTraceQ.data, "steps");
  const automationObjects = useMemo(() => pickAutomationObjects(automationQ.data), [automationQ.data]);
  const automationPreview = automationObjects.slice(0, 8);
  const activeIntegrations = useMemo(
    () => automationObjects.filter(isActiveIntegration),
    [automationObjects]
  );
  const automationRuns = useMemo(() => pickAutomationRuns(automationRunsQ.data), [automationRunsQ.data]);
  const automationRunsPreview = automationRuns.slice(0, 6);
  const evolutionStatus = asRecord(evolutionQ.data);
  const learningQueue = asRecord(evolutionStatus.learning_queue);
  const learningQueueTotal = Math.round(
    (finiteNumber(learningQueue.provisional_runs) ?? 0) +
    (finiteNumber(learningQueue.pending_consolidation) ?? 0) +
    (finiteNumber(learningQueue.pending_reflection) ?? 0) +
    (finiteNumber(learningQueue.draft_candidates) ?? 0) +
    (finiteNumber(learningQueue.active_patterns) ?? 0)
  );
  const backgroundSessions = useMemo<BackgroundSessionSummary[]>(
    () => (sessionsQ.data?.sessions || []).filter((session) => isBackgroundSessionVisibleInUi(session)),
    [sessionsQ.data]
  );
  const activeBackgroundSessions = useMemo(
    () =>
      backgroundSessions.filter((session) =>
        ["active", "waiting", "needs_input", "paused"].includes((session.status || "").toLowerCase())
      ),
    [backgroundSessions]
  );
  const automationCounts = useMemo(() => {
    return automationObjects.reduce(
      (acc, item) => {
        const kind = (item.kind || "").toLowerCase();
        if (kind === "task") acc.tasks += 1;
        if (kind === "watcher") acc.watchers += 1;
        if (kind === "app") acc.apps += 1;
        if (isActiveIntegration(item)) acc.integrations += 1;
        return acc;
      },
      { tasks: 0, watchers: 0, apps: 0, integrations: 0 }
    );
  }, [automationObjects]);

  const currentTask = useMemo(() => {
    const inProgress = tasks.find((task) => isFreshInProgressTask(task));
    return inProgress?.description;
  }, [tasks]);
  const waitingTask = useMemo(() => {
    return tasks.find((task) => {
      const status = String(task?.status || "").toLowerCase();
      return status.includes("awaitingapproval") || status.includes("paused");
    });
  }, [tasks]);
  const recentFailedAutomationRun = useMemo(() => {
    return automationRuns.find((run) => {
      const status = `${run.status || ""} ${run.current_status || ""}`.toLowerCase();
      return status.includes("failed") || status.includes("error");
    });
  }, [automationRuns]);
  const latestDailyBriefNotification = useMemo(
    () => pickDailyBriefNotification(notifications, dailyBriefRun?.triggered_at),
    [notifications, dailyBriefRun?.triggered_at]
  );
  const latestDailyBriefTrace = useMemo(
    () => pickDailyBriefTrace(traces, dailyBriefRun?.triggered_at),
    [traces, dailyBriefRun?.triggered_at]
  );
  const latestDailyBriefAutomationRun = useMemo(
    () => pickDailyBriefAutomationRun(automationRuns, dailyBriefRun?.triggered_at),
    [automationRuns, dailyBriefRun?.triggered_at]
  );
  // Check if LLM is configured from settings
  const hasLlmConfigured = useMemo(() => {
    if (!settingsQ.data) return true; // Assume OK while loading
    const settings = settingsQ.data as Record<string, unknown>;
    const pool = settings.model_pool || settings.llm_pool || settings.models;
    if (Array.isArray(pool)) return pool.length > 0;
    const provider = String(settings.llm_provider ?? settings.provider ?? "").trim();
    const model = String(settings.llm_model ?? settings.model ?? "").trim();
    return provider.length > 0 && model.length > 0;
  }, [settingsQ.data]);

  const heroPrompts = useMemo(() => {
    const prompts: string[] = [];
    const seen = new Set<string>();
    const pushPrompt = (value?: string | null) => {
      const next = (value || "").trim();
      if (!next || seen.has(next)) return;
      seen.add(next);
      prompts.push(next);
    };
    const clean = (value?: string | null, limit = 96) =>
      (value || "")
        .replace(/\s+/g, " ")
        .trim()
        .slice(0, limit);
    const scifiLead = (value: string) => {
      const next = clean(value, 104);
      if (!next) return null;
      return next.endsWith(".") ? next : `${next}.`;
    };
    const briefing = briefingQ.data as BriefingResponse | undefined;
    const recommendedActions = briefing?.recommended_actions || briefing?.recommended_skills || [];
    const topOpportunity = briefing?.top_opportunities?.[0];
    const topRisk = briefing?.top_risks?.[0];

    if (hasLlmConfigured) {
      pushPrompt(
        recommendedActions[0]
          ? scifiLead(
              `Open the next operator lane: ${clean(
                recommendedActions[0].summary ||
                  recommendedActions[0].description ||
                  recommendedActions[0].title,
                92
              )}`
            )
          : null
      );
      pushPrompt(
        topOpportunity
          ? scifiLead(
              `Bring this signal online: ${clean(
                topOpportunity.summary || topOpportunity.detail || topOpportunity.title,
                92
              )}`
            )
          : null
      );
      pushPrompt(
        topRisk
          ? scifiLead(
              `Run a quiet risk sweep on ${clean(
                topRisk.title || topRisk.summary || topRisk.detail || "the active queue",
                72
              )} and surface the safest move`
            )
          : null
      );
      pushPrompt(
        currentTask
          ? scifiLead(
              `Resume the active lane: ${clean(currentTask, 84)} and surface only the next operator decision`
            )
          : null
      );
      pushPrompt(
        waitingTask
          ? scifiLead(
              `Review ${clean(waitingTask.description || "the waiting task", 84)} and return the lowest-risk next step`
            )
          : null
      );
      pushPrompt(
        recentFailedAutomationRun
          ? scifiLead(
              `Sweep the fault trace for ${clean(
                recentFailedAutomationRun.title || recentFailedAutomationRun.summary,
                78
              )} and propose the cleanest recovery path`
            )
          : null
      );
      pushPrompt(
        traces[0]?.message_preview
          ? scifiLead(
              `Read the latest mission signal: ${clean(traces[0].message_preview, 86)} and tell me the next move`
            )
          : null
      );
      pushPrompt("Run a quiet systems sweep and surface the single next move that matters.");
      return prompts.slice(0, 6);
    }

    pushPrompt(
      currentTask
        ? `Continue "${clean(currentTask, 92)}" and only surface blockers that need me.`
        : null
    );
    pushPrompt(
      waitingTask
        ? `Review "${clean(waitingTask.description || "the waiting task", 92)}" and recommend the safest next decision.`
        : null
    );
    pushPrompt(
      recentFailedAutomationRun
        ? `Inspect "${clean(recentFailedAutomationRun.title || recentFailedAutomationRun.summary, 92)}" and tell me what failed and how to fix it.`
        : null
    );
    pushPrompt(
      traces[0]?.message_preview
        ? `Summarize the latest run: "${clean(traces[0].message_preview, 92)}" and tell me the next move.`
        : null
    );
    pushPrompt("Review recent changes and list only the critical risks.");
    pushPrompt("Build a small app to track competitor launches and deploy it.");
    pushPrompt("Import this skill URL and wire up any required secrets.");
    pushPrompt("Inspect active automations and surface anything that needs intervention.");
    return prompts.slice(0, 6);
  }, [briefingQ.data, currentTask, hasLlmConfigured, recentFailedAutomationRun, traces, waitingTask]);

  const autonomySettings = useMemo(() => {
    const root = asRecord(autonomySettingsQ.data);
    return asRecord(root.settings);
  }, [autonomySettingsQ.data]);

  const autonomyMode = str(autonomySettings.autonomy_mode, "assist").toLowerCase();
  const agentPaused = Boolean(autonomySettings.agent_paused ?? false) || autonomyMode === "off";

  useEffect(() => {
    const refreshHistory = (event?: Event) => {
      const detail = (event as CustomEvent<RuntimeMetricSample[]> | undefined)?.detail;
      setRuntimeMetricHistory(Array.isArray(detail) ? detail : readRuntimeMetricHistory());
    };
    refreshHistory();
    window.addEventListener(RUNTIME_METRIC_HISTORY_EVENT, refreshHistory as EventListener);
    return () => window.removeEventListener(RUNTIME_METRIC_HISTORY_EVENT, refreshHistory as EventListener);
  }, []);

  // --- Mutations ---
  const approveMutation = useMutation({
    mutationFn: (id: string) => api.approveTask(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tasks"] }),
  });
  const rejectMutation = useMutation({
    mutationFn: (id: string) => api.rejectTask(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tasks"] }),
  });
  const retryMutation = useMutation({
    mutationFn: api.retryTask,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });
  const executeActionMutation = useMutation({
    mutationFn: api.executeRecommendedAction,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });
  const runBriefingMutation = useMutation({
    mutationFn: () =>
      api.executeRecommendedAction({
        id: "daily_brief_now",
        title: "Generate Daily Brief",
        action_kind: "daily_brief_now",
        payload: {},
      } as RecommendedAction),
    onMutate: () => {
      const triggeredAt = new Date().toISOString();
      setDailyBriefRun({
        outcome: "running",
        title: "Generating Daily Brief",
        detail: "AgentArk is building the brief, logging the run, and checking whether it can deliver it to your preferred channel.",
        brief: "",
        triggered_at: triggeredAt,
      });
      setDailyBriefDialogOpen(true);
    },
    onSuccess: async (out) => {
      const triggeredAt = new Date().toISOString();
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      setDailyBriefRun(parseDailyBriefRunResponse(out, triggeredAt));
      setDailyBriefDialogOpen(true);
    },
    onError: (error) => {
      setDailyBriefRun({
        outcome: "error",
        title: "Daily Brief failed",
        detail: errMessage(error),
        brief: "",
        triggered_at: new Date().toISOString(),
      });
      setDailyBriefDialogOpen(true);
    },
  });
  async function handleExecuteSuggestedAction(action: RecommendedAction) {
    const startedAt = new Date().toISOString();
    setSuggestionRun({
      title: action.title || "Suggested run",
      status: "running",
      summary: `Starting "${action.title || "suggested run"}"...`,
      startedAt,
    });
    setSuggestionRunOpen(true);
    setSuggestionRunMinimized(false);

    try {
      const out = await executeActionMutation.mutateAsync(action);
      const traceId = str(out.trace_id, str(asRecord(out.result).trace_id, "")).trim();
      const summary = summarizeSuggestedRun(action, out);
      const resultStatus = str(asRecord(out.result).status, out.status).trim().toLowerCase();
      const finalStatus: SuggestionRunState["status"] =
        resultStatus === "queued_for_approval" || !traceId ? "completed" : "running";
      setSuggestionRun({
        title: action.title || "Suggested run",
        status: finalStatus,
        summary,
        traceId: traceId || undefined,
        startedAt,
        completedAt: finalStatus === "completed" ? new Date().toISOString() : undefined,
      });
    } catch (error) {
      setSuggestionRun({
        title: action.title || "Suggested run",
        status: "error",
        summary: errMessage(error),
        startedAt,
        completedAt: new Date().toISOString(),
      });
    }
  }

  useEffect(() => {
    if (!suggestionRun?.traceId) return;
    if (suggestionTraceQ.isLoading || suggestionTraceQ.error || !Object.keys(suggestionTrace).length) return;
    const traceStatus = str(suggestionTrace.status, suggestionRun.status).trim().toLowerCase();
    const lastStep = suggestionTraceSteps[suggestionTraceSteps.length - 1] || {};
    const consoleView = buildSuggestionTraceConsoleView(lastStep);
    const nextStatus: SuggestionRunState["status"] =
      traceStatus === "completed"
        ? "completed"
        : traceStatus === "failed" || traceStatus === "error" || traceStatus === "warning"
          ? "error"
          : "running";
    const nextSummary =
      str(suggestionTrace.response, "").trim() || consoleView.detail || suggestionRun.summary;
    const nextStartedAt = str(suggestionTrace.started_at, suggestionRun.startedAt || "");
    const nextCompletedAt = str(suggestionTrace.completed_at, suggestionRun.completedAt || "");
    if (
      nextStatus !== suggestionRun.status ||
      nextSummary !== suggestionRun.summary ||
      nextStartedAt !== (suggestionRun.startedAt || "") ||
      nextCompletedAt !== (suggestionRun.completedAt || "")
    ) {
      setSuggestionRun((current) =>
        current
          ? {
              ...current,
              status: nextStatus,
              summary: nextSummary,
              startedAt: nextStartedAt || current.startedAt,
              completedAt: nextCompletedAt || current.completedAt,
            }
          : current
      );
    }
  }, [suggestionRun, suggestionTrace, suggestionTraceQ.isLoading, suggestionTraceQ.error, suggestionTraceSteps]);

  const hasErrors = !!(
    tasksQ.error ||
    traceQ.error ||
    briefingQ.error ||
    autonomySettingsQ.error ||
    automationQ.error ||
    automationRunsQ.error ||
    sessionsQ.error
  );
  const failingSources = [
    tasksQ.error ? "tasks" : null,
    traceQ.error ? "trace" : null,
    briefingQ.error ? "briefing" : null,
    autonomySettingsQ.error ? "autonomy settings" : null,
    automationQ.error ? "automation objects" : null,
    automationRunsQ.error ? "automation runs" : null,
    sessionsQ.error ? "background sessions" : null,
  ].filter(Boolean) as string[];
  const dataSourceErrorSummary =
    failingSources.length === 0
      ? ""
      : failingSources.length === 1
        ? `${failingSources[0]} failed to load. Retrying automatically.`
        : `${failingSources.join(", ")} failed to load. Retrying automatically.`;
  const showActiveSessionsPanel = activeBackgroundSessions.length > 0;
  const showActivityFeed = traces.length > 0;
  const arkDistillAnalytics = arkDistillAnalyticsQ.data as LlmAnalyticsResponse | undefined;
  const arkDistillTotals = arkDistillAnalytics?.arkdistill?.totals;
  const promptTokens = finiteNumber(arkDistillAnalytics?.totals?.prompt_tokens) ?? 0;
  const cachedPromptTokens = finiteNumber(arkDistillAnalytics?.totals?.cached_prompt_tokens) ?? 0;
  const arkDistillSavedTokens = finiteNumber(arkDistillTotals?.estimated_saved_tokens) ?? 0;
  const combinedInputSavedTokens = cachedPromptTokens + arkDistillSavedTokens;
  const combinedInputBaselineTokens = promptTokens + arkDistillSavedTokens;
  const combinedInputSavingsPercent =
    combinedInputBaselineTokens > 0
      ? (combinedInputSavedTokens / combinedInputBaselineTokens) * 100
      : null;
  const arkDistillHasData = Boolean(
    combinedInputSavedTokens > 0 ||
      (arkDistillTotals &&
        ((finiteNumber(arkDistillTotals.result_count) ?? 0) > 0 ||
          (finiteNumber(arkDistillTotals.saved_chars) ?? 0) > 0))
  );
  const missionPlanSteps = useMemo(() => {
    const recommended = (briefingQ.data?.recommended_actions || briefingQ.data?.recommended_skills || [])[0];
    return [
      {
        label: "Observe system metrics",
        detail: serverStatus ? "Runtime, memory, and queue state loaded" : "Waiting for status pulse",
        state: serverStatus ? "done" : "pending",
      },
      {
        label: "Analyze current signals",
        detail: briefingQ.data ? "Briefing, risks, and opportunities available" : "Briefing is loading",
        state: briefingQ.data ? "done" : "pending",
      },
      {
        label: currentTask ? "Execute active work" : recommended?.title || "Choose the next action",
        detail: currentTask ? "AgentArk has work in progress" : "No active run is pinned",
        state: currentTask ? "active" : recommended ? "ready" : "pending",
      },
      {
        label: "Validate outcome",
        detail: traces.length > 0 ? "Recent trace evidence is available" : "Waiting for a run to land",
        state: traces.length > 0 ? "done" : "pending",
      },
      {
        label: "Reflect and learn",
        detail: showActivityFeed ? "Reflect has recent activity to summarize" : "No recent activity yet",
        state: showActivityFeed ? "ready" : "pending",
      },
    ];
  }, [briefingQ.data, currentTask, serverStatus, showActivityFeed, traces.length]);
  const activeMissionCount = useMemo(() => {
    const running = tasks.filter((task) => {
      const status = String(task?.status || "").toLowerCase();
      return status.includes("progress") || status.includes("running");
    }).length;
    return running + activeBackgroundSessions.length;
  }, [activeBackgroundSessions.length, tasks]);
  const recentTraces = useMemo(() => {
    const all = Array.isArray(traces) ? traces.slice() : [];
    all.sort((a, b) => {
      const aTs = a.started_at || "";
      const bTs = b.started_at || "";
      if (aTs === bTs) return 0;
      return aTs < bTs ? 1 : -1;
    });
    return all.slice(0, 4);
  }, [traces]);
  const topOpportunity = briefingQ.data?.top_opportunities?.[0];
  const topRisk = briefingQ.data?.top_risks?.[0];
  const reflectionNote =
    str(topOpportunity?.summary || topOpportunity?.detail || topOpportunity?.title, "").trim() ||
    str(topRisk?.summary || topRisk?.detail || topRisk?.title, "").trim() ||
    str(traces[0]?.message_preview, "").trim() ||
    "No reflection note has landed yet.";
  const recommendedNext = (briefingQ.data?.recommended_actions || briefingQ.data?.recommended_skills || [])[0];
  const briefingSignal =
    str(recommendedNext?.title || recommendedNext?.summary, "").trim() ||
    str(topRisk?.title || topRisk?.summary, "").trim() ||
    "No new briefing signals this cycle.";
  const nextCheckpoint =
    missionPlanSteps.find((step) => step.state !== "done")?.label ?? "All steps complete";
  const memoryPressurePct =
    typeof memoryPressureValue === "number" && Number.isFinite(memoryPressureValue)
      ? Math.max(0, Math.min(100, memoryPressureValue))
      : null;
  return (
    <Box
      data-tour-target="overview-dashboard"
      className="nw-shell overview-shell"
    >
      {hasErrors ? (
        <Alert severity="error" sx={{ mb: 1.5 }}>
          {dataSourceErrorSummary}
        </Alert>
      ) : null}

      <div className="nw-frame">
        <section className="nw-dashboard nw-dashboard--syn">
          <div className="nw-syn-top" aria-label="Mission status summary">
            <span className="nw-syn-top-kicker">Mission Control</span>
            <span className={`nw-syn-cycle${serverError ? " nw-syn-cycle--warn" : ""}`}>
              <i className={`nw-syn-lamp${serverError ? " nw-syn-lamp--amber" : ""}`} />
              {serverError ? (
                <>RUNTIME · <b>CHECK</b></>
              ) : currentTask ? (
                <>CYCLE · STAGE 04 <b>ACT</b></>
              ) : (
                <>CYCLE · STAGE 01 <b>OBSERVE</b></>
              )}
            </span>
          </div>

          <div className="nw-syn-grid">
            <aside className="nw-syn-col" aria-label="Mission objective and plan">
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Mission</span>
                <span className="nw-syn-rule" />
                <span className={`nw-syn-state${currentTask ? " nw-syn-state--run" : ""}`}>
                  {currentTask ? "RUNNING" : "READY"}
                </span>
              </div>
              <div className="nw-syn-plan">
                {missionPlanSteps.map((step, index) => (
                  <div
                    className={`nw-syn-step nw-syn-step--${step.state}`}
                    key={`${step.label}-${index}`}
                    title={step.detail}
                  >
                    <span className="nw-syn-step-dot" />
                    <span className="nw-syn-step-ix">{String(index + 1).padStart(2, "0")}</span>
                    <span className="nw-syn-step-t">{step.label}</span>
                    <span className="nw-syn-step-state">
                      {step.state === "done"
                        ? "DONE"
                        : step.state === "active"
                          ? "RUNNING"
                          : step.state === "ready"
                            ? "READY"
                            : "QUEUED"}
                    </span>
                  </div>
                ))}
              </div>

              <div className="nw-syn-div" />
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">ArkDistill saved</span>
                <span className="nw-syn-rule" />
              </div>
              {arkDistillHasData && combinedInputSavingsPercent != null ? (
                <>
                  <div className="nw-syn-stat-vrow">
                    <span className="nw-syn-stat-v nw-syn-stat-v--mint nw-syn-stat-v--sm">
                      {formatCompactNumber(combinedInputSavedTokens)}
                    </span>
                    <span className="nw-syn-stat-lab">Tokens saved</span>
                    <span className="nw-syn-stat-v nw-syn-stat-v--mint nw-syn-stat-v--sm nw-syn-stat-v--push">
                      {combinedInputSavingsPercent.toFixed(1)}%
                    </span>
                    <span className="nw-syn-stat-lab">Input saved</span>
                  </div>
                  <div className="nw-syn-meter">
                    <i style={{ width: `${Math.max(2, Math.min(100, combinedInputSavingsPercent))}%` }} />
                  </div>
                  <div className="nw-syn-msub">
                    <span>prompt cache + ArkDistill</span>
                    <span>30 days</span>
                  </div>
                </>
              ) : (
                <p className="nw-syn-copy">ArkDistill savings appear after distilled tool results land.</p>
              )}

              <div className="nw-syn-div" />
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Core readouts</span>
                <span className="nw-syn-rule" />
              </div>
              <div style={{ marginTop: 6 }}>
                <div className="nw-syn-kv">
                  <span className="nw-syn-kv-k">Model</span>
                  <span className="nw-syn-kv-v" title={modelDisplay}>{modelDisplay}</span>
                </div>
                <div className="nw-syn-kv">
                  <span className="nw-syn-kv-k">Uptime</span>
                  <span className="nw-syn-kv-v">{formatCompactUptime(runtimeHealth?.uptime_seconds)}</span>
                </div>
                <div className="nw-syn-kv">
                  <span className="nw-syn-kv-k">Latency</span>
                  <span className="nw-syn-kv-v">
                    {serverStatus?.rtt_ms != null ? `${Math.round(serverStatus.rtt_ms)}ms` : "-"}
                  </span>
                </div>
                <div className="nw-syn-kv">
                  <span className="nw-syn-kv-k">Mem pressure</span>
                  <span className="nw-syn-kv-v nw-syn-kv-v--mint">
                    {formatCompactPercent(memoryPressureValue)}
                  </span>
                </div>
                <div className="nw-syn-kv">
                  <span className="nw-syn-kv-k">Active missions</span>
                  <span className="nw-syn-kv-v">{activeMissionCount}</span>
                </div>
              </div>
              <div className="nw-syn-foot">
                Next checkpoint · <b>{nextCheckpoint}</b>
              </div>
            </aside>

            <div className="nw-syn-center" aria-label="Agent cognition loop">
              <AgentCognitionLoop
                memoryCount={serverStatus?.status?.memory_entries ?? 0}
                skillCount={serverStatus?.status?.skills_loaded ?? serverStatus?.status?.actions_loaded ?? 0}
                appCount={automationCounts.apps}
                integrationCount={automationCounts.integrations}
                traceCount={traces.length}
                selfEvolveEnabled={Boolean(evolutionStatus.self_evolve_enabled)}
                learningQueueCount={learningQueueTotal}
                latencyMs={serverStatus?.rtt_ms ?? null}
                memoryPressureHistory={memoryPressureHistory}
                latencyHistory={latencyHistory}
                running={Boolean(currentTask)}
              />
            </div>

            <aside className="nw-syn-col" aria-label="Mission telemetry">
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Signals</span>
                <span className="nw-syn-rule" />
                <i className="nw-syn-lamp nw-syn-lamp--violet" />
              </div>

              <div className="nw-syn-shead" style={{ marginTop: 16 }}>
                <span className="nw-syn-kicker">ArkSentinel</span>
                <span className="nw-syn-rule" />
              </div>
              <div className="nw-syn-stat-vrow">
                <span className="nw-syn-stat-v">{securityLogs.length}</span>
                <span className="nw-syn-stat-lab">{securityLogs.length === 1 ? "Alert" : "Alerts"}</span>
                <span className={`nw-syn-stat-tail${securityLogs.length > 0 ? " nw-syn-stat-tail--warn" : ""}`}>
                  {securityLogs.length > 0 ? "REVIEW" : "GUARDRAILS ACTIVE"}
                </span>
              </div>
              <p className="nw-syn-copy">
                {securityLogs.length > 0 ? (
                  `Latest severity ${String(securityLogs[0]?.severity || "review").toUpperCase()}.`
                ) : (
                  <>
                    <em>ArkSentinel</em> is watching the perimeter — no policy breaches this cycle.
                  </>
                )}
              </p>

              <div className="nw-syn-div" />
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Memory</span>
                <span className="nw-syn-rule" />
              </div>
              <div className="nw-syn-stat-vrow">
                <span className="nw-syn-stat-v nw-syn-stat-v--mint">
                  {serverStatus?.status?.memory_entries ?? 0}
                </span>
                <span className="nw-syn-stat-lab">Entries</span>
                <span className="nw-syn-stat-tail">
                  {memoryPressurePct != null && memoryPressurePct > 80 ? "PRESSURE" : "HEALTHY"}
                </span>
              </div>
              <div className="nw-syn-meter">
                <i style={{ width: `${Math.max(2, memoryPressurePct ?? 0)}%` }} />
              </div>
              <div className="nw-syn-msub">
                <span>
                  pressure <b>{formatCompactPercent(memoryPressureValue)}</b>
                </span>
                <span>
                  headroom <b>{memoryPressurePct == null ? "-" : `${Math.round(100 - memoryPressurePct)}%`}</b>
                </span>
              </div>

              <div className="nw-syn-div" />
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Runtime</span>
                <span className="nw-syn-rule" />
              </div>
              <div className="nw-syn-stat-vrow">
                <span className={`nw-syn-stat-v nw-syn-stat-v--sm${serverError ? "" : " nw-syn-stat-v--mint"}`}>
                  {serverError ? "CHECK" : "OK"}
                </span>
                <span className="nw-syn-stat-lab">
                  {serverError ? "Attention needed" : "All loops nominal"}
                </span>
              </div>
              <p className="nw-syn-copy">
                {serverError
                  ? "The runtime needs a check — the status pulse failed."
                  : "The runtime is steady — every cognition loop returned nominal this sweep."}
              </p>

              <div className="nw-syn-div" />
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Runtime trends</span>
                <span className="nw-syn-rule" />
              </div>
              <div className="nw-syn-trends" aria-label="Runtime metric trends">
                <div className="nw-syn-trend">
                  <div className="nw-syn-trend-head">
                    <span>Latency</span>
                    <b>{serverStatus?.rtt_ms != null ? `${Math.round(serverStatus.rtt_ms)}ms` : "-"}</b>
                  </div>
                  <MetricSparkline values={latencyHistory} />
                </div>
                <div className="nw-syn-trend">
                  <div className="nw-syn-trend-head">
                    <span>Mem pressure</span>
                    <b>{formatCompactPercent(memoryPressureValue)}</b>
                  </div>
                  <MetricSparkline values={memoryPressureHistory} />
                </div>
              </div>
            </aside>
          </div>

          <div className="nw-syn-bottom">
            <section aria-label="Intel">
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Intel</span>
                <span className="nw-syn-rule" />
                <i className="nw-syn-lamp" />
              </div>
              <div className="nw-syn-intel-grid">
                <div className="nw-syn-note">
                  <div className="nw-syn-note-tag nw-syn-note-tag--mint">Reflection</div>
                  <p>{reflectionNote}</p>
                </div>
                <div className="nw-syn-note">
                  <div className="nw-syn-note-tag nw-syn-note-tag--violet">Signal</div>
                  <p>{briefingSignal}</p>
                </div>
              </div>
            </section>
            <section aria-label="Activity stream">
              <div className="nw-syn-shead">
                <span className="nw-syn-kicker">Activity stream</span>
                <span className="nw-syn-rule" />
                <span className="nw-syn-state">TRC · {traces.length}</span>
              </div>
              <div className="nw-syn-rows">
                {recentTraces.length === 0 ? (
                  <p className="nw-syn-copy">No recent runs landed yet.</p>
                ) : (
                  recentTraces.map((trace) => {
                    const badge = traceStatusBadge(trace.status);
                    return (
                      <div className="nw-syn-trow" key={trace.id}>
                        <span className="nw-syn-trow-tag" title={trace.channel}>
                          {humanizeMachineLabel(trace.channel, "run")}
                        </span>
                        <span className="nw-syn-trow-d" title={trace.message_preview}>
                          {trace.message_preview || "(no preview)"}
                        </span>
                        <span className={`nw-syn-trow-st${badge.cls}`}>{badge.label}</span>
                        <span
                          className="nw-syn-trow-tm"
                          title={formatUiRelativeDateTimeMeta(trace.started_at).tip}
                        >
                          {formatUiRelativeDateTimeMeta(trace.started_at).label}
                        </span>
                      </div>
                    );
                  })
                )}
              </div>
              <button type="button" className="nw-syn-more" onClick={() => setActivityOpen(true)}>
                Activity feed -&gt;
              </button>
            </section>
          </div>
        </section>

        {/* Inline activity feed (preserved when traces exist and not surfaced via runtime card) */}
        {showActivityFeed ? (
          <div style={{ display: "none" }}>
            <ActivityFeed traces={traces} onViewAll={() => setActivityOpen(true)} />
          </div>
        ) : null}
      </div>
      {/* Automation Inventory Dialog */}
      <Dialog
        open={inventoryOpen}
        onClose={() => setInventoryOpen(false)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              background: "var(--ui-rgba-22-22-26-980)",
              border: "1px solid var(--ui-rgba-255-255-255-080)",
              backdropFilter: "blur(20px)",
            },
          }
        }}
      >
        <DialogTitle sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <Box>
            <Typography variant="h6">Automation Inventory</Typography>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Unified runtime view of active tasks, watchers, deployed apps, and integrations.
            </Typography>
          </Box>
          <IconButton size="small" onClick={() => setInventoryOpen(false)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          <Stack
            direction="row"
            spacing={0.75}
            useFlexGap
            sx={{
              flexWrap: "wrap",
              mb: 2
            }}>
            <Chip size="small" label={`${automationCounts.tasks} tasks`} />
            <Chip size="small" label={`${automationCounts.watchers} watchers`} />
            <Chip size="small" label={`${automationCounts.apps} apps`} />
            <Chip size="small" label={`${automationCounts.integrations} integrations`} />
          </Stack>

          {automationQ.error ? (
            <Alert severity="error">{errMessage(automationQ.error)}</Alert>
          ) : automationPreview.length === 0 ? (
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              No active automation objects yet.
            </Typography>
          ) : (
            <Stack spacing={1} sx={{
              mb: 3
            }}>
              {automationPreview.map((item) => (
                <Box key={`${item.kind}-${item.id}`} className="action-row">
                  <Stack
                    direction="row"
                    spacing={1.25}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "flex-start"
                    }}>
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Chip size="small" label={automationKindLabel(item.kind)} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={item.title}>
                          {item.title}
                        </Typography>
                      </Stack>
                      {item.subtitle ? (
                        <Typography variant="caption" noWrap title={item.subtitle} sx={{
                          color: "text.secondary"
                        }}>
                          {item.subtitle}
                        </Typography>
                      ) : null}
                      {item.detail ? (
                        <Typography variant="caption" noWrap title={item.detail} sx={{
                          color: "text.secondary"
                        }}>
                          {item.detail}
                        </Typography>
                      ) : null}
                      {item.next_run_at ? (
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          Next run: {formatAutomationTime(item.next_run_at)}
                        </Typography>
                      ) : null}
                    </Stack>
                    <Stack
                      direction="row"
                      spacing={1}
                      sx={{
                        alignItems: "center",
                        flexShrink: 0
                      }}>
                      <Chip size="small" label={humanizeStatusLabel(item.status, "-")} color={automationStatusColor(item.status || "")} />
                      {item.url ? (
                        <Button
                          size="small"
                          onClick={() => window.open(item.url || "", "_blank", "noopener,noreferrer")}
                        >
                          Open
                        </Button>
                      ) : null}
                      <Button
                        size="small"
                        onClick={() => navigateToView(targetViewForAutomation(item))}
                      >
                        View
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}

          {/* Recent Automation Runs subsection */}
          <Typography variant="h6" sx={{
            mb: 0.5
          }}>Recent Automation Runs</Typography>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              mb: 1.5
            }}>
            Supervisor history for background tasks and watchers, including retries and validation summaries.
          </Typography>

          {automationRunsQ.error ? (
            <Alert severity="error">{errMessage(automationRunsQ.error)}</Alert>
          ) : automationRunsPreview.length === 0 ? (
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              No automation runs recorded yet.
            </Typography>
          ) : (
            <Stack spacing={1}>
              {automationRunsPreview.map((item) => (
                <Box key={item.id} className="action-row">
                  <Stack
                    direction="row"
                    spacing={1.25}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "flex-start"
                    }}>
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Chip size="small" label={automationKindLabel(item.kind)} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={item.title}>
                          {item.title}
                        </Typography>
                        <Chip size="small" label={`Attempt ${item.attempt}`} />
                      </Stack>
                      <Typography variant="caption" noWrap title={item.summary} sx={{
                        color: "text.secondary"
                      }}>
                        {item.summary}
                      </Typography>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Started: {formatAutomationTime(item.started_at)}
                        {item.next_retry_at ? ` | Next retry: ${formatAutomationTime(item.next_retry_at)}` : ""}
                      </Typography>
                    </Stack>
                    <Stack
                      direction="row"
                      spacing={1}
                      sx={{
                        alignItems: "center",
                        flexShrink: 0
                      }}>
                      <Chip
                        size="small"
                        label={item.current_status || item.status || "-"}
                        color={automationStatusColor(item.current_status || item.status || "")}
                      />
                      <Button
                        size="small"
                        onClick={() => navigateToView(targetViewForAutomationRun(item))}
                      >
                        View
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setInventoryOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>
      {/* Recent Activity Dialog */}
      <Dialog
        open={activityOpen}
        onClose={() => setActivityOpen(false)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              background: "var(--ui-rgba-22-22-26-980)",
              border: "1px solid var(--ui-rgba-255-255-255-080)",
              backdropFilter: "blur(20px)",
            },
          }
        }}
      >
        <DialogTitle sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <Typography variant="h6">Recent Activity</Typography>
          <IconButton size="small" onClick={() => setActivityOpen(false)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          <ActivityFeed
            traces={traces}
            onViewAll={() => {
              setActivityOpen(false);
              navigateToView("trace");
            }}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => { setActivityOpen(false); navigateToView("trace"); }}>
            View All Traces
          </Button>
          <Button onClick={() => setActivityOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={dailyBriefDialogOpen}
        onClose={() => setDailyBriefDialogOpen(false)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              background: "var(--ui-rgba-22-22-26-980)",
              border: "1px solid var(--ui-rgba-255-255-255-080)",
              backdropFilter: "blur(20px)",
            },
          }
        }}
      >
        <DialogTitle sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <Box>
            <Typography variant="h6">{dailyBriefRun?.title || "Daily Brief"}</Typography>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              See what AgentArk generated, what it attempted, and the nearest runtime evidence.
            </Typography>
          </Box>
          <IconButton size="small" onClick={() => setDailyBriefDialogOpen(false)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          {dailyBriefRun ? (
            <Stack spacing={1.5}>
              <Alert
                severity={
                  dailyBriefRun.outcome === "success"
                    ? "success"
                    : dailyBriefRun.outcome === "running"
                      ? "info"
                      : "error"
                }
              >
                {dailyBriefRun.detail}
              </Alert>
              {dailyBriefRun.outcome === "running" ? (
                <Stack direction="row" spacing={1} sx={{
                  alignItems: "center"
                }}>
                  <CircularProgress size={18} />
                  <Typography variant="body2">Running daily brief now...</Typography>
                </Stack>
              ) : null}

              <Stack direction="row" spacing={0.75} useFlexGap sx={{
                flexWrap: "wrap"
              }}>
                <Chip
                  size="small"
                  label={
                    dailyBriefRun.outcome === "running"
                      ? "Running"
                      : dailyBriefRun.outcome === "success"
                        ? "Run completed"
                        : "Run failed"
                  }
                />
                {latestDailyBriefNotification ? <Chip size="small" color="info" label="Prior in-app record" /> : null}
                {latestDailyBriefTrace ? (
                  <Chip size="small" color={automationStatusColor(latestDailyBriefTrace.status)} label={`Trace: ${humanizeStatusLabel(latestDailyBriefTrace.status || "recorded")}`} />
                ) : null}
                {latestDailyBriefAutomationRun ? (
                  <Chip
                    size="small"
                    color={automationStatusColor(latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "")}
                    label={`Automation: ${humanizeStatusLabel(latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "recorded")}`}
                  />
                ) : null}
              </Stack>

              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                  What AgentArk did
                </Typography>
                <Stack spacing={0.45}>
                  <Typography variant="body2">- Built a fresh daily brief from current tasks, recent activity, and connected data.</Typography>
                  <Typography variant="body2">- Skipped creating an in-app notification.</Typography>
                  <Typography variant="body2">- Attempted push delivery to your preferred briefing channel if one is configured.</Typography>
                </Stack>
              </Box>

              {dailyBriefRun.brief ? (
                <Box>
                  <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                    Generated brief
                  </Typography>
                  <Box
                    sx={{
                      p: 1.25,
                      borderRadius: 2,
                      border: "1px solid var(--ui-rgba-255-255-255-080)",
                      background: "var(--ui-rgba-24-24-28-900)",
                    }}
                  >
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {dailyBriefRun.brief}
                    </Typography>
                  </Box>
                </Box>
              ) : null}

              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                  Observed evidence
                </Typography>
                <Stack spacing={1}>
                  {latestDailyBriefNotification ? (
                    <Box className="action-row">
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        In-app notification
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block"
                        }}>
                        {formatAutomationTime(latestDailyBriefNotification.created_at)}
                      </Typography>
                      <Typography variant="body2" sx={{ mt: 0.35 }}>
                        {latestDailyBriefNotification.body}
                      </Typography>
                    </Box>
                  ) : null}

                  {latestDailyBriefTrace ? (
                    <Box className="action-row">
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>
                          Latest related trace
                        </Typography>
                        <Chip
                          size="small"
                          label={latestDailyBriefTrace.status || "-"}
                          color={automationStatusColor(latestDailyBriefTrace.status || "")}
                        />
                      </Stack>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block"
                        }}>
                        Started: {formatAutomationTime(latestDailyBriefTrace.started_at)}
                        {typeof latestDailyBriefTrace.duration_ms === "number" ? ` | ${latestDailyBriefTrace.duration_ms}ms` : ""}
                        {typeof latestDailyBriefTrace.step_count === "number" ? ` | ${latestDailyBriefTrace.step_count} step${latestDailyBriefTrace.step_count === 1 ? "" : "s"}` : ""}
                      </Typography>
                      <Typography variant="body2" sx={{ mt: 0.35 }}>
                        {latestDailyBriefTrace.message_preview}
                      </Typography>
                    </Box>
                  ) : null}

                  {latestDailyBriefAutomationRun ? (
                    <Box className="action-row">
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>
                          Related automation run
                        </Typography>
                        <Chip
                          size="small"
                          label={latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "-"}
                          color={automationStatusColor(latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "")}
                        />
                      </Stack>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block"
                        }}>
                        Started: {formatAutomationTime(latestDailyBriefAutomationRun.started_at)}
                      </Typography>
                      <Typography variant="body2" sx={{ mt: 0.35 }}>
                        {latestDailyBriefAutomationRun.summary || latestDailyBriefAutomationRun.title}
                      </Typography>
                    </Box>
                  ) : null}

                  {!latestDailyBriefNotification && !latestDailyBriefTrace && !latestDailyBriefAutomationRun ? (
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      No related runtime evidence has landed yet. If a delivery channel is configured, it may still arrive shortly.
                    </Typography>
                  ) : null}
                </Stack>
              </Box>

              {briefingQ.data ? (
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Current briefing snapshot timestamp: {formatAutomationTime((briefingQ.data as { generated_at?: string }).generated_at)}
                </Typography>
              ) : null}
            </Stack>
          ) : null}
        </DialogContent>
      <DialogActions>
        <Button onClick={() => navigateToView("trace")}>Open Trace</Button>
        <Button onClick={() => navigateToView("tasks")}>Open Tasks</Button>
        <Button onClick={() => setDailyBriefDialogOpen(false)}>Close</Button>
      </DialogActions>
      </Dialog>
      <SuggestionRunDialog
        run={suggestionRun}
        open={suggestionRunOpen}
        minimized={suggestionRunMinimized}
        trace={suggestionTrace}
        traceSteps={suggestionTraceSteps}
        traceLoading={suggestionTraceQ.isLoading}
        traceError={suggestionTraceQ.error}
        detailError={null}
        acceptedOutcomes={[]}
        onClose={() => setSuggestionRunOpen(false)}
        onMinimize={() => setSuggestionRunMinimized(true)}
        onRestore={() => setSuggestionRunMinimized(false)}
        onOpenWorkspacePanel={(view) => navigateToView(view)}
        getConsoleView={(step) => buildSuggestionTraceConsoleView(step)}
        getTraceStepColor={traceStepColor}
        humanTs={humanTs}
        errMessage={errMessage}
      />
    </Box>
  );
}
