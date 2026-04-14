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
import CloseIcon from "@mui/icons-material/Close";
import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { isBackgroundSessionVisibleInUi } from "../lib/backgroundSessions";
import { formatUiDateTime, formatUiRelativeDateTimeMeta } from "../lib/dateFormat";
import { useUiStore } from "../store/uiStore";
import { AgentStatusBar } from "./AgentStatusBar";
import { WelcomeHero } from "./WelcomeHero";
import { NeedsAttentionInbox } from "./NeedsAttentionInbox";
import { buildAttentionItems } from "./NeedsAttentionInbox";
import { TodaysHighlights } from "./TodaysHighlights";
import { SmartSuggestions } from "./SmartSuggestions";
import { ActivityFeed } from "./ActivityFeed";
import { SuggestionRunDialog, type SuggestionRunState } from "./SuggestionRunDialog";
import type {
  AutonomyActionExecutionResponse,
  BackgroundSessionSummary,
  BriefingResponse,
  RecommendedAction,
  Task,
  TraceSummary,
  Notification,
} from "../types";

const REFRESH_MS = 8000;
const ACTIVE_TASK_STALE_MS = 24 * 60 * 60 * 1000;
type JsonRecord = Record<string, unknown>;
type PausePhase = "idle" | "stopping" | "stopped" | "resuming" | "resumed";
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
  project_id?: string | null;
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

function automationKindLabel(kind: string): string {
  const normalized = (kind || "").toLowerCase();
  if (normalized === "task") return "Task";
  if (normalized === "watcher") return "Watcher";
  if (normalized === "app") return "App";
  if (normalized === "integration") return "Integration";
  return kind || "Automation";
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

export function OverviewPane({ navigateToView, serverStatus, serverError, serverLoading }: Props) {
  const queryClient = useQueryClient();
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const interval = autoRefresh ? REFRESH_MS : false;
  const [pauseDialogOpen, setPauseDialogOpen] = useState(false);
  const [pausePhase, setPausePhase] = useState<PausePhase>("idle");
  const [pauseError, setPauseError] = useState<string | null>(null);
  const [pauseTarget, setPauseTarget] = useState<"pause" | "resume" | null>(null);
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
  const notifications = Array.isArray(notificationsQ.data) ? notificationsQ.data : [];
  const securityLogs = (securityQ.data as { logs?: Array<{ event_type: string; severity: string; message: string }> })?.logs || [];
  const suggestionTrace = asRecord(suggestionTraceQ.data);
  const suggestionTraceSteps = pickRecords(suggestionTraceQ.data, "steps");
  const automationObjects = useMemo(() => pickAutomationObjects(automationQ.data), [automationQ.data]);
  const automationPreview = automationObjects.slice(0, 8);
  const automationRuns = useMemo(() => pickAutomationRuns(automationRunsQ.data), [automationRunsQ.data]);
  const automationRunsPreview = automationRuns.slice(0, 6);
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
        if (kind === "integration") acc.integrations += 1;
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

  const agentPaused = Boolean(autonomySettings.agent_paused ?? false);
  const pauseScopeItems = [
    "Scheduled tasks",
    "Watchers",
    "ArkPulse runs",
    "Autopilot/background analysis",
    "Proactive outbound notifications",
  ];

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
  const pauseMutation = useMutation({
    mutationFn: (nextPaused: boolean) =>
      api.rawPost("/autonomy/settings", {
        agent_paused: nextPaused,
        pause_mode: "autonomous_only",
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings-dashboard"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });

  async function handleTogglePause() {
    if (pauseMutation.isPending) return;
    const nextPaused = !agentPaused;
    setPauseDialogOpen(true);
    setPauseError(null);
    setPauseTarget(nextPaused ? "pause" : "resume");
    setPausePhase(nextPaused ? "stopping" : "resuming");
    try {
      await pauseMutation.mutateAsync(nextPaused);
      setPausePhase(nextPaused ? "stopped" : "resumed");
    } catch (error) {
      setPauseError(errMessage(error));
      setPausePhase("idle");
    }
  }

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
  const automationSurfaceTotal =
    automationCounts.tasks +
    automationCounts.watchers +
    automationCounts.apps +
    automationCounts.integrations;
  const attentionItems = useMemo(
    () => buildAttentionItems(tasks, notifications, securityLogs, !settingsQ.isLoading, hasLlmConfigured),
    [tasks, notifications, securityLogs, settingsQ.isLoading, hasLlmConfigured]
  );
  const showAttentionPanel = attentionItems.length > 0;
  const showActiveSessionsPanel = activeBackgroundSessions.length > 0;
  const showActivityFeed = traces.length > 0;
  const automationHeadline =
    automationSurfaceTotal > 0
      ? `${automationSurfaceTotal} automation surfaces are currently in play.`
      : "No active automation surfaces are live right now.";

  return (
    <Box
      data-tour-target="overview-dashboard"
      className="overview-shell"
    >
      {hasErrors ? (
        <Alert severity="error">
          {dataSourceErrorSummary}
        </Alert>
      ) : null}
      <Box className="overview-command-grid">
        <Box className="overview-main-column">
          <Box data-tour-target="welcome-hero">
            <WelcomeHero
              onGoChat={() => navigateToView("chat")}
              onRunBriefing={() => runBriefingMutation.mutate()}
              onViewTasks={() => navigateToView("tasks")}
              onTogglePause={() => {
                void handleTogglePause();
              }}
              agentPaused={agentPaused}
              briefingLoading={runBriefingMutation.isPending}
              pauseLoading={pauseMutation.isPending}
              prompts={heroPrompts}
              currentTaskDesc={currentTask}
            />
          </Box>

          {showAttentionPanel ? (
            <NeedsAttentionInbox
              tasks={tasks}
              notifications={notifications}
              securityLogs={securityLogs}
              settingsLoaded={!settingsQ.isLoading}
              hasLlmConfigured={hasLlmConfigured}
              onApprove={(id) => approveMutation.mutate(id)}
              onReject={(id) => rejectMutation.mutate(id)}
              onRetry={(id) => retryMutation.mutate(id)}
              onNavigate={navigateToView}
              approving={approveMutation.isPending}
              rejecting={rejectMutation.isPending}
              retrying={retryMutation.isPending}
            />
          ) : (
            <Box className="overview-inline-note">
              <Stack
                direction={{ xs: "column", sm: "row" }}
                spacing={1}
                sx={{
                  alignItems: { xs: "flex-start", sm: "center" },
                  justifyContent: "space-between"
                }}>
                <Box>
                  <Typography variant="overline" className="overview-inline-note__kicker">
                    Operator Queue
                  </Typography>
                  <Typography variant="body2" sx={{ color: "text.primary", fontWeight: 600 }}>
                    No approvals, failures, or urgent interventions are waiting.
                  </Typography>
                </Box>
                <Button variant="outlined" size="small" onClick={() => navigateToView("tasks")}>
                  Review Tasks
                </Button>
                </Stack>
              </Box>
          )}

          {showActiveSessionsPanel ? (
            <Box className="overview-inline-note overview-inline-note--sessions">
              <Stack spacing={1}>
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={1}
                  sx={{
                    alignItems: { xs: "flex-start", sm: "center" },
                    justifyContent: "space-between"
                  }}>
                  <Box>
                  <Typography variant="overline" className="overview-inline-note__kicker">
                    Active Sessions
                  </Typography>
                  <Typography variant="body2" sx={{ color: "text.primary", fontWeight: 600 }}>
                    {`${activeBackgroundSessions.length} background session${activeBackgroundSessions.length === 1 ? "" : "s"} currently have ongoing work or supervision state.`}
                  </Typography>
                </Box>
                  <Button variant="outlined" size="small" onClick={() => navigateToView("sessions")}>
                    Open Sessions
                  </Button>
                </Stack>
                <Stack spacing={0.75}>
                  {activeBackgroundSessions.slice(0, 3).map((session) => (
                    <Stack
                      key={session.id}
                      direction="row"
                      spacing={0.75}
                      sx={{
                        alignItems: "center",
                        px: 0.9,
                        py: 0.75,
                        borderRadius: 2,
                        background: "rgba(255, 255, 255, 0.03)",
                        border: "1px solid rgba(255, 255, 255, 0.08)"
                      }}>
                      <Chip size="small" label={session.status.replace(/_/g, " ")} color={automationStatusColor(session.status)} />
                      <Box sx={{ minWidth: 0, flex: 1 }}>
                        <Typography variant="body2" noWrap sx={{ fontWeight: 600 }} title={session.title}>
                          {session.title}
                        </Typography>
                        <Typography variant="caption" noWrap title={session.live_summary} sx={{
                          color: "text.secondary"
                        }}>
                          {session.live_summary}
                        </Typography>
                      </Box>
                    </Stack>
                  ))}
                </Stack>
              </Stack>
            </Box>
          ) : null}

          <Box className={`overview-bento-grid ${showActivityFeed ? "overview-bento-grid--three" : "overview-bento-grid--two"}`}>
            <Box className="overview-panel-slot">
              <Box className="overview-action-card mission-panel mission-panel--adaptive">
                <Stack spacing={1.15} className="mission-panel-content">
                  <Stack spacing={1.15} className="mission-panel-section">
                    <Box>
                      <Typography
                        variant="overline"
                        sx={{
                          color: "rgba(183, 188, 196, 0.68)",
                          letterSpacing: 0,
                          display: "block",
                          mb: 0.35
                        }}
                      >
                        Automation Posture
                      </Typography>
                      <Typography variant="h6" sx={{ fontWeight: 700, mb: 0.45 }}>
                        Live surfaces and system drift.
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        {automationHeadline}
                      </Typography>
                    </Box>

                    <Box
                      sx={{
                        display: "grid",
                        gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
                        gap: 0.85,
                      }}
                    >
                      {[
                        { label: "Tasks", value: automationCounts.tasks },
                        { label: "Watchers", value: automationCounts.watchers },
                        { label: "Apps", value: automationCounts.apps },
                        { label: "Integrations", value: automationCounts.integrations },
                      ].map((item) => (
                        <Box
                          key={item.label}
                          className="mission-metric-card"
                          sx={{
                            px: 1,
                            py: 0.9,
                          }}
                        >
                          <Typography variant="caption" className="mission-metric-card__label">
                            {item.label}
                          </Typography>
                          <Typography variant="subtitle1" className="mission-metric-card__value" sx={{ mt: 0.2 }}>
                            {item.value}
                          </Typography>
                        </Box>
                      ))}
                    </Box>

                    {recentFailedAutomationRun ? (
                      <Alert severity="warning" sx={{ py: 0.3 }}>
                        Degraded run: {recentFailedAutomationRun.title || recentFailedAutomationRun.summary}
                      </Alert>
                    ) : automationPreview.length > 0 ? (
                      <Stack spacing={0.5}>
                        {automationPreview.slice(0, 3).map((item) => (
                          <Stack
                            key={`${item.kind}-${item.id}`}
                            direction="row"
                            spacing={0.75}
                            sx={{
                              alignItems: "center",
                              px: 0.9,
                              py: 0.7,
                              borderRadius: 2,
                              background: "rgba(255, 255, 255, 0.03)",
                              border: "1px solid rgba(255, 255, 255, 0.08)"
                            }}>
                            <Chip size="small" label={automationKindLabel(item.kind)} />
                            <Typography variant="body2" noWrap sx={{ minWidth: 0, flex: 1 }} title={item.title}>
                              {item.title}
                            </Typography>
                          </Stack>
                        ))}
                      </Stack>
                    ) : (
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Runtime inventory is quiet. The system is standing by for a new surface or trigger.
                      </Typography>
                    )}
                  </Stack>

                  <Stack direction={{ xs: "column", sm: "row" }} spacing={0.85} className="mission-panel-footer">
                    <Button variant="contained" size="small" onClick={() => setInventoryOpen(true)}>
                      Automation Inventory
                    </Button>
                    <Button variant="outlined" size="small" onClick={() => setActivityOpen(true)}>
                      Recent Activity
                    </Button>
                    <Button variant="outlined" size="small" onClick={() => navigateToView("trace")}>
                      Open Trace
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            </Box>
            <Box className="overview-panel-slot">
              <TodaysHighlights tasks={tasks} traces={traces} />
            </Box>
            {showActivityFeed ? (
              <Box className="overview-panel-slot">
                <ActivityFeed
                  traces={traces}
                  onViewAll={() => {
                    setActivityOpen(true);
                  }}
                />
              </Box>
            ) : null}
          </Box>
        </Box>

        <Box className="overview-side-column">
          <AgentStatusBar
            serverStatus={serverStatus}
            serverError={serverError}
            serverLoading={serverLoading}
            currentTaskDesc={currentTask}
            agentPaused={agentPaused}
            hasLlmConfigured={hasLlmConfigured}
            automationCounts={automationCounts}
            recentFailureTitle={recentFailedAutomationRun?.title || recentFailedAutomationRun?.summary || null}
          />
          <SmartSuggestions
            briefing={briefingQ.data}
            onExecuteAction={handleExecuteSuggestedAction}
            executing={executeActionMutation.isPending}
          />
        </Box>
      </Box>
      {/* Automation Inventory Dialog */}
      <Dialog
        open={inventoryOpen}
        onClose={() => setInventoryOpen(false)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              background: "rgba(22, 22, 26, 0.98)",
              border: "1px solid rgba(255, 255, 255, 0.08)",
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
                      <Chip size="small" label={item.status || "-"} color={automationStatusColor(item.status || "")} />
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
              background: "rgba(22, 22, 26, 0.98)",
              border: "1px solid rgba(255, 255, 255, 0.08)",
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
              background: "rgba(22, 22, 26, 0.98)",
              border: "1px solid rgba(255, 255, 255, 0.08)",
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
                {latestDailyBriefNotification ? <Chip size="small" color="info" label="Notification logged" /> : null}
                {latestDailyBriefTrace ? (
                  <Chip size="small" color={automationStatusColor(latestDailyBriefTrace.status)} label={`Trace: ${latestDailyBriefTrace.status || "recorded"}`} />
                ) : null}
                {latestDailyBriefAutomationRun ? (
                  <Chip
                    size="small"
                    color={automationStatusColor(latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "")}
                    label={`Automation: ${latestDailyBriefAutomationRun.current_status || latestDailyBriefAutomationRun.status || "recorded"}`}
                  />
                ) : null}
              </Stack>

              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                  What AgentArk did
                </Typography>
                <Stack spacing={0.45}>
                  <Typography variant="body2">- Built a fresh daily brief from current tasks, recent activity, and connected data.</Typography>
                  <Typography variant="body2">- Logged a `Daily Command Brief` notification inside AgentArk.</Typography>
                  <Typography variant="body2">- Attempted delivery to your preferred briefing channel if one is configured.</Typography>
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
                      border: "1px solid rgba(255, 255, 255, 0.08)",
                      background: "rgba(24, 24, 28, 0.9)",
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
      <Dialog
        open={pauseDialogOpen}
        onClose={() => {
          if (pauseMutation.isPending) return;
          setPauseDialogOpen(false);
          setPauseError(null);
          setPausePhase("idle");
          setPauseTarget(null);
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {pausePhase === "stopping" && "Pausing Agent"}
          {pausePhase === "stopped" && "Agent Paused"}
          {pausePhase === "resuming" && "Resuming Agent"}
          {pausePhase === "resumed" && "Agent Resumed"}
          {pausePhase === "idle" && (pauseTarget === "resume" ? "Resume Agent" : "Pause Agent")}
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {pausePhase === "stopping" || pausePhase === "resuming" ? (
              <Stack direction="row" spacing={1} sx={{
                alignItems: "center"
              }}>
                <CircularProgress size={18} />
                <Typography variant="body2">
                  {pausePhase === "stopping"
                    ? "stopping: disabling autonomous background activity..."
                    : "resuming: re-enabling autonomous background activity..."}
                </Typography>
              </Stack>
            ) : null}

            {pausePhase === "stopped" || pausePhase === "resumed" ? (
              <Chip
                size="small"
                color="success"
                label={pausePhase === "stopped" ? "stopped" : "resumed"}
                sx={{ width: "fit-content" }}
              />
            ) : null}

            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              {pauseTarget === "pause"
                ? "When paused, these systems are suspended:"
                : "On resume, these systems are active again:"}
            </Typography>

            <Stack spacing={0.5}>
              {pauseScopeItems.map((item) => (
                <Typography key={item} variant="body2">
                  - {item}
                </Typography>
              ))}
            </Stack>

            {pauseError ? <Alert severity="error">{pauseError}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setPauseDialogOpen(false);
              setPauseError(null);
              setPausePhase("idle");
              setPauseTarget(null);
            }}
            disabled={pauseMutation.isPending}
          >
            Close
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
