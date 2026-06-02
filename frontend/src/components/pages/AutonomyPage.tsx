import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import AgentLogo from "../../assets/logo.svg";
import { humanizeMachineLabel, humanizeStatusLabel } from "../../lib/displayLabels";
import { MetricBarCard } from "../analytics/MetricBarCard";
import { LiveEventConsole } from "../LiveEventConsole";
import {
  SuggestionRunDialog,
  type SuggestionRunState,
} from "../SuggestionRunDialog";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs, KeyValuePanel, RowOpsMenu } from "./workspaceUiBits";
import {
  DEVELOPER_MODE_EVENT,
  getDeveloperModeEnabled,
  REFRESH_MS,
  SHOW_EXPERIMENTAL_AUTONOMY_TOOLS,
} from "./workspaceCore";
import {
  buildTraceStepConsoleView,
  traceStepColor,
} from "./traceEvolutionHelpers";

type ReadinessDialogState = {
  title: string;
  readiness: JsonRecord;
};

function readinessRecord(value: unknown): JsonRecord | null {
  const record = asRecord(value);
  return Object.keys(record).length > 0 ? record : null;
}

function valueStringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => String(item ?? "").trim())
    .filter((item) => item.length > 0);
}

function readinessChipColor(stage: string) {
  if (stage === "auto_ready") return "success" as const;
  if (stage === "review_ready") return "info" as const;
  return "warning" as const;
}

function readinessShortLabel(readiness: JsonRecord | null) {
  if (!readiness) return "Evidence unavailable";
  const label = str(readiness.label, "Still learning");
  const score = num(readiness.score, NaN);
  const scoreText = Number.isFinite(score) ? ` ${Math.round(score)}%` : "";
  return `${label}${scoreText}`;
}

export default function AutonomyPage({
  autoRefresh,
}: {
  autoRefresh: boolean;
}) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [autonomyMode, setAutonomyMode] = useState<"off" | "assist" | "auto">(
    "assist",
  );
  const [alwaysAskHighRisk, setAlwaysAskHighRisk] = useState(true);
  const [onlyApprovedSkills, setOnlyApprovedSkills] = useState(true);
  const [quietHoursStart, setQuietHoursStart] = useState("");
  const [quietHoursEnd, setQuietHoursEnd] = useState("");
  const [dailyRunLimit, setDailyRunLimit] = useState("40");
  const [settingsHydrated, setSettingsHydrated] = useState(false);

  const [incidentResult, setIncidentResult] = useState<JsonRecord | null>(null);
  const [rollingBackEventId, setRollingBackEventId] = useState<string | null>(
    null,
  );

  const [triageLabelsCsv, setTriageLabelsCsv] = useState(
    "Act now, Delegate, Ignore",
  );
  const [triageMessagesJson, setTriageMessagesJson] = useState("");
  const [triageResult, setTriageResult] = useState<JsonRecord | null>(null);

  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [sessionResponse, setSessionResponse] = useState("");
  const [browserRespondResult, setBrowserRespondResult] =
    useState<JsonRecord | null>(null);
  const [suggestionRun, setSuggestionRun] = useState<SuggestionRunState | null>(
    null,
  );
  const [suggestionRunOpen, setSuggestionRunOpen] = useState(false);
  const [suggestionRunMinimized, setSuggestionRunMinimized] = useState(false);
  const [activeSuggestionActionId, setActiveSuggestionActionId] = useState<
    string | null
  >(null);
  const [readinessDialog, setReadinessDialog] =
    useState<ReadinessDialogState | null>(null);

  const settingsQ = useQuery({
    queryKey: ["autonomy-settings"],
    queryFn: () => api.rawGet("/autonomy/settings"),
  });
  const briefingQ = useQuery({
    queryKey: ["autonomy-briefing"],
    queryFn: () => api.rawGet("/autonomy/briefing"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const notificationsQ = useQuery({
    queryKey: ["autonomy-unread-notifications"],
    queryFn: () => api.rawGet("/notifications?unread=true&limit=120"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const evolutionQ = useQuery({
    queryKey: ["autonomy-evolution-summary"],
    queryFn: () => api.rawGet("/settings/evolution"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const incidentsQ = useQuery({
    queryKey: ["autonomy-incidents-live"],
    queryFn: () => api.rawGet("/autonomy/incidents/live"),
    enabled: showAdvanced,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const timelineQ = useQuery({
    queryKey: ["autonomy-timeline"],
    queryFn: () => api.rawGet("/autonomy/timeline?limit=120"),
    enabled: showAdvanced && SHOW_EXPERIMENTAL_AUTONOMY_TOOLS,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const browserSessionsQ = useQuery({
    queryKey: ["autonomy-browser-sessions"],
    queryFn: () => api.rawGet("/browser/sessions"),
    enabled: showAdvanced,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const browserStatusQ = useQuery({
    queryKey: ["autonomy-browser-session-status", selectedSessionId],
    queryFn: () =>
      api.rawGet(
        `/browser/sessions/${encodeURIComponent(selectedSessionId)}/status`,
      ),
    enabled: showAdvanced && !!selectedSessionId,
    refetchInterval: autoRefresh && !!selectedSessionId ? REFRESH_MS : false,
  });
  const suggestionTraceId = suggestionRun?.traceId || "";
  const suggestionDetailId = suggestionRun?.suggestionId || "";
  const suggestionTraceQ = useQuery({
    queryKey: ["autonomy-suggestion-trace", suggestionTraceId],
    queryFn: () =>
      api.rawGet(`/trace/${encodeURIComponent(suggestionTraceId)}`),
    enabled: !!suggestionTraceId && suggestionRunOpen,
    refetchInterval:
      suggestionRunOpen &&
      !!suggestionTraceId &&
      suggestionRun?.status === "running"
        ? REFRESH_MS
        : false,
  });
  const suggestionDetailQ = useQuery({
    queryKey: ["autonomy-suggestion-detail", suggestionDetailId],
    queryFn: () =>
      api.rawGet(
        `/autonomy/suggestions/${encodeURIComponent(suggestionDetailId)}`,
      ),
    enabled: !!suggestionDetailId && suggestionRunOpen,
    refetchInterval:
      suggestionRunOpen &&
      !!suggestionDetailId &&
      suggestionRun?.status === "running"
        ? REFRESH_MS
        : false,
  });

  const saveAutonomySettingsMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/autonomy/settings", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
    },
  });
  const executeIncidentMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/autonomy/incidents/${encodeURIComponent(id)}/execute`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-incidents-live"],
      });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-timeline"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    },
  });
  const rollbackMutation = useMutation({
    mutationFn: (payload: { event_id: string; operation?: string }) =>
      api.rawPost("/autonomy/timeline/rollback", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-timeline"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
    },
  });
  const triageMutation = useMutation({
    mutationFn: (payload: { labels?: string[]; messages: unknown[] }) =>
      api.rawPost("/autonomy/inbox/triage", payload),
  });
  const browserRespondMutation = useMutation({
    mutationFn: (payload: { id: string; response: string }) =>
      api.rawPost(
        `/browser/sessions/${encodeURIComponent(payload.id)}/respond`,
        { response: payload.response },
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-browser-sessions"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-browser-session-status", selectedSessionId],
      });
    },
  });
  const acceptSuggestionMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/autonomy/suggestions/${encodeURIComponent(id)}/accept`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    },
  });
  const dismissSuggestionMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(
        `/autonomy/suggestions/${encodeURIComponent(id)}/dismiss`,
        {},
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
    },
  });

  const incidents = pickRecords(incidentsQ.data, "incidents");
  const timelineEvents = pickRecords(timelineQ.data, "events");
  const triageRows = pickRecords(triageResult, "triage");
  const browserSessions = pickRecords(browserSessionsQ.data, "sessions");
  const browserStatus = asRecord(browserStatusQ.data);
  const evolution = asRecord(evolutionQ.data);
  const evolutionCanary = asRecord(evolution.canary);
  const evolutionLearningQueue = asRecord(evolution.learning_queue);
  const suggestionTrace = asRecord(suggestionTraceQ.data);
  const suggestionTraceSteps = pickRecords(suggestionTraceQ.data, "steps");
  const suggestionDetail = asRecord(
    asRecord(suggestionDetailQ.data).suggestion,
  );
  const suggestionAcceptedOutcomes = pickRecords(
    suggestionDetail,
    "accepted_outcomes",
  );

  function severityChipColor(
    sev: string,
  ): "error" | "warning" | "info" | "success" | "default" {
    const s = (sev || "").toLowerCase();
    if (s === "critical" || s === "high" || s === "error") return "error";
    if (s === "medium" || s === "warn" || s === "warning") return "warning";
    if (s === "low") return "info";
    if (s === "ok" || s === "info") return "success";
    return "default";
  }

  function parseCsv(value: string): string[] {
    return value
      .split(",")
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
  }

  function parseTriageMessages(value: string): unknown[] {
    const trimmed = value.trim();
    if (!trimmed) return [];
    const parsed: unknown = JSON.parse(trimmed);
    if (!Array.isArray(parsed)) {
      throw new Error("Messages JSON must be an array.");
    }
    return parsed;
  }

  function effectiveRollbackOperation(
    operation: string,
    status: string,
  ): string {
    if (operation !== "toggle_notification_read") return operation;
    return status.toLowerCase() === "read" ? "mark_unread" : "mark_read";
  }

  function rollbackLabel(operation: string): string {
    const op = (operation || "").toLowerCase();
    if (op === "cancel_task") return "Cancel task";
    if (op === "cancel_watcher") return "Cancel watcher";
    if (op === "mark_unread") return "Mark unread";
    if (op === "mark_read") return "Mark read";
    if (op === "toggle_notification_read") return "Toggle read";
    return "Rollback";
  }

  const settingsRecord = asRecord(asRecord(settingsQ.data).settings);
  const briefingRecord = asRecord(briefingQ.data);
  const queueSummary = asRecord(asRecord(briefingRecord.trust_summary).queue);
  const topRisks = pickRecords(briefingRecord, "top_risks");
  const recommendedActions = pickRecords(briefingRecord, "recommended_actions");
  const suggestedAutomations = pickRecords(
    briefingRecord,
    "suggested_automations",
  );
  const suggestionScan = asRecord(briefingRecord.suggestion_scan);
  const attentionRisks = topRisks.filter((risk) => {
    const hay =
      `${str(risk.type, "")} ${str(risk.title, "")} ${str(risk.detail, "")}`.toLowerCase();
    return !(
      hay.includes("arkpulse") ||
      hay.includes("auth-related security events") ||
      hay.includes("security events were logged")
    );
  });
  const unreadNotifications = pickRecords(notificationsQ.data, "notifications");
  const awaitingApprovals = num(queueSummary.awaiting_approval, 0);
  const missingInputs = unreadNotifications.filter((row) => {
    const source = str(row.source, "").toLowerCase();
    const title = str(row.title, "").toLowerCase();
    const body = str(row.body, "").toLowerCase();
    return (
      source === "workflow_inputs" ||
      title.includes("missing input") ||
      body.includes("missing input") ||
      title.includes("required input") ||
      body.includes("required input")
    );
  }).length;
  const suggestionScanStatus = str(suggestionScan.last_status, "scheduled");
  const suggestionScanLabel =
    suggestionScanStatus === "completed"
      ? "Ready"
      : suggestionScanStatus === "disabled"
        ? "Disabled"
        : suggestionScanStatus === "deferred_busy"
          ? "Deferred"
          : suggestionScanStatus === "running"
            ? "Scanning"
            : suggestionScanStatus === "no_user_chat"
              ? "Waiting for chat"
              : suggestionScanStatus === "error"
                ? "Needs attention"
                : "Scheduled";
  const modeIndicator =
    autonomyMode === "auto"
      ? "Auto"
      : autonomyMode === "assist"
        ? "Assist"
        : "Off";
  const controlsTabIndex = 0;
  const suggestionsTabIndex = 1;
  const selfEvolveTabIndex = 2;
  const opsTabIndex = 3;
  const waitingStatusLine =
    autonomyMode === "off"
      ? "Mode: Off | Background autonomy is paused. Existing tasks, watchers, and history stay stored, scheduled reminders still fire, and new proactive runs stay paused until you turn autonomy back on."
      : awaitingApprovals === 0 && missingInputs === 0
        ? `Mode: ${modeIndicator} | You're all set. Nothing is waiting on you.`
        : `Mode: ${modeIndicator} | Waiting on you: ${awaitingApprovals} approval${awaitingApprovals === 1 ? "" : "s"}, ${missingInputs} required input${missingInputs === 1 ? "" : "s"}`;
  const modePlainHint =
    autonomyMode === "off"
      ? "Autonomy is paused. Sentinel, Pulse, background learning, and chat suggestion scans are paused until you turn it back on. Scheduled reminders still fire."
      : autonomyMode === "assist"
        ? "Agent prepares work and asks before sensitive actions."
        : "Agent runs allowed work automatically and only asks when required.";
  const selfEvolveEnabled = toBool(evolution.self_evolve_enabled);
  const selfEvolveCanaryEnabled = toBool(evolutionCanary.enabled);
  const selfEvolveDraftCount = num(evolutionLearningQueue.draft_candidates, 0);
  const selfEvolveBacklogCount = num(
    evolutionLearningQueue.pending_consolidation,
    0,
  );
  const needsReviewCount =
    awaitingApprovals + missingInputs + attentionRisks.length;
  const primaryStatusSeverity: "success" | "warning" | "info" =
    autonomyMode === "off"
      ? "warning"
      : needsReviewCount > 0
        ? "warning"
        : "success";
  const primaryStatusTitle =
    autonomyMode === "off"
      ? "AgentArk autonomy is paused"
      : needsReviewCount > 0
        ? `${needsReviewCount} item${needsReviewCount === 1 ? "" : "s"} need your review`
        : "Everything looks good";
  const primaryStatusDetail =
    autonomyMode === "off"
      ? "Background help is paused. Existing work stays saved, scheduled reminders still fire, and new proactive runs will not start until autonomy is re-enabled in Settings > Advanced."
      : needsReviewCount > 0
        ? `Approvals: ${awaitingApprovals}, missing input: ${missingInputs}, flagged issues: ${attentionRisks.length}.`
        : "Nothing is blocked, no approvals are waiting, and AgentArk can keep helping in the background.";
  const selfEvolveStatusLabel = evolutionQ.isLoading
    ? "Checking"
    : evolutionQ.error
      ? "Unavailable"
      : !selfEvolveEnabled
        ? "Off"
        : selfEvolveCanaryEnabled
          ? "Testing improvements"
          : selfEvolveDraftCount > 0 || selfEvolveBacklogCount > 0
            ? "Learning from recent work"
            : "Ready";
  const selfEvolveStatusTone: "default" | "success" | "warning" =
    evolutionQ.error
      ? "warning"
      : !selfEvolveEnabled
        ? "default"
        : selfEvolveCanaryEnabled || selfEvolveDraftCount > 0
          ? "warning"
          : "success";
  const selfEvolveDetail = evolutionQ.isLoading
    ? "Checking whether AgentArk is learning from completed work."
    : evolutionQ.error
      ? "Self-evolve details are unavailable right now."
      : !selfEvolveEnabled
        ? "Self-evolve is off. Background learning and evolution canaries stay paused until you turn it back on in Settings > Advanced."
        : selfEvolveCanaryEnabled
          ? `Canary rollout is testing ${str(evolutionCanary.candidate_version, "a candidate policy")} against ${str(evolutionCanary.baseline_version, "the baseline")}.`
          : selfEvolveDraftCount > 0 || selfEvolveBacklogCount > 0
            ? `Drafts waiting: ${selfEvolveDraftCount}. Learning backlog: ${selfEvolveBacklogCount}.`
            : "Self-evolve is on and there are no draft changes waiting for review.";
  const backgroundActivityLabel =
    autonomyMode === "off"
      ? "Paused"
      : suggestionScanStatus === "running"
        ? "Busy"
        : suggestionScanStatus === "error"
          ? "Needs review"
          : "Active";
  const backgroundActivityTone: "default" | "success" | "warning" =
    autonomyMode === "off"
      ? "default"
      : suggestionScanStatus === "error" ||
          suggestionScanStatus === "deferred_busy"
        ? "warning"
        : "success";
  const backgroundActivityDetail =
    autonomyMode === "off"
      ? "Health checks, background monitoring, and suggestion scans are paused. Scheduled reminders still fire."
      : `Health checks are running quietly. Suggestion scan is ${suggestionScanLabel.toLowerCase()} and ${suggestedAutomations.length} automation suggestion${suggestedAutomations.length === 1 ? "" : "s"} ${suggestedAutomations.length === 1 ? "is" : "are"} waiting.`;
  const suggestionLastRunLabel = str(suggestionScan.last_completed_at, "")
    ? humanTs(str(suggestionScan.last_completed_at, "")).label
    : "Not yet";
  const suggestionNextRunLabel = str(suggestionScan.next_due_at, "")
    ? humanTs(str(suggestionScan.next_due_at, "")).label
    : "Scheduling";
  const attentionPreview = attentionRisks.slice(0, 3);
  const recommendedActionPreview = recommendedActions.slice(0, 3);
  const suggestionPreview = suggestedAutomations.slice(0, 3);
  const needsYouItems: Array<{
    key: string;
    title: string;
    detail: string;
    actionLabel: string;
    onClick: () => void;
  }> = [
    ...(awaitingApprovals > 0
      ? [
          {
            key: "approvals",
            title: `${awaitingApprovals} approval${awaitingApprovals === 1 ? "" : "s"} waiting`,
            detail:
              "AgentArk is waiting for your approval before it can continue some work.",
            actionLabel: "Review",
            onClick: () => openWorkspacePanel("tasks"),
          },
        ]
      : []),
    ...(missingInputs > 0
      ? [
          {
            key: "missing-inputs",
            title: `${missingInputs} input request${missingInputs === 1 ? "" : "s"} waiting`,
            detail:
              "Some runs need a missing answer or file before they can continue.",
            actionLabel: "Open",
            onClick: () => openWorkspacePanel("tasks"),
          },
        ]
      : []),
    ...attentionPreview.map((risk, idx) => ({
      key: `risk-${idx}`,
      title: str(risk.title, "Risk"),
      detail: str(risk.detail, "Review this item to keep AgentArk healthy."),
      actionLabel: "Open",
      onClick: () => openSettingsTab(recommendedTabForRisk(risk)),
    })),
  ];
  const needsYouSummary =
    needsYouItems.length === 0
      ? "Nothing is waiting on you right now."
      : `${needsYouItems.length} item${needsYouItems.length === 1 ? "" : "s"} may need your attention.`;
  const suggestedAutomationSummary =
    suggestionPreview.length === 0 && recommendedActionPreview.length === 0
      ? "No new automation ideas are waiting from recent chat."
      : `${suggestedAutomations.length + recommendedActionPreview.length} suggestion${suggestedAutomations.length + recommendedActionPreview.length === 1 ? "" : "s"} are ready for review in Advanced details.`;
  const configuredModeRaw = str(
    settingsRecord.autonomy_mode,
    "assist",
  ).toLowerCase();
  const configuredMode: "off" | "assist" | "auto" =
    configuredModeRaw === "off" ||
    configuredModeRaw === "auto" ||
    configuredModeRaw === "assist"
      ? configuredModeRaw
      : "assist";
  const configuredAutonomyDisabled =
    Boolean(settingsRecord.agent_paused ?? false) || configuredMode === "off";
  const configuredEffectiveMode: "off" | "assist" | "auto" =
    configuredAutonomyDisabled ? "off" : configuredMode;
  const configuredAlwaysAskHighRisk = Boolean(
    settingsRecord.always_ask_high_risk ?? true,
  );
  const configuredOnlyApprovedSkills = Boolean(
    settingsRecord.only_approved_skills ?? true,
  );
  const configuredQuietHoursStart = str(
    settingsRecord.quiet_hours_start,
    "",
  ).trim();
  const configuredQuietHoursEnd = str(
    settingsRecord.quiet_hours_end,
    "",
  ).trim();
  const configuredDailyRunLimit =
    typeof settingsRecord.daily_run_limit === "number" &&
    Number.isFinite(settingsRecord.daily_run_limit)
      ? Math.round(settingsRecord.daily_run_limit)
      : null;
  const normalizedQuietHoursStart = quietHoursStart.trim();
  const normalizedQuietHoursEnd = quietHoursEnd.trim();
  const normalizedLimitText = dailyRunLimit.trim();
  let parsedLimitForUi: number | null = null;
  let dailyRunLimitInvalid = false;
  if (normalizedLimitText.length > 0) {
    const n = Number(normalizedLimitText);
    if (!Number.isFinite(n) || n < 1) {
      dailyRunLimitInvalid = true;
    } else {
      parsedLimitForUi = Math.round(n);
    }
  }
  const guardrailsDirty =
    settingsHydrated &&
    (autonomyMode !== configuredEffectiveMode ||
      alwaysAskHighRisk !== configuredAlwaysAskHighRisk ||
      onlyApprovedSkills !== configuredOnlyApprovedSkills ||
      normalizedQuietHoursStart !== configuredQuietHoursStart ||
      normalizedQuietHoursEnd !== configuredQuietHoursEnd ||
      parsedLimitForUi !== configuredDailyRunLimit);

  function openAdvancedTab(nextTab = 0) {
    setTab(nextTab);
    setShowAdvanced(true);
  }

  function openSettingsTab(tabName: string) {
    const nextPath = "/ui/settings";
    const nextSearch = `?settings_tab=${encodeURIComponent(tabName)}`;
    const nextUrl = `${nextPath}${nextSearch}`;
    const current = `${window.location.pathname}${window.location.search}`;
    if (current !== nextUrl) {
      window.history.pushState(null, "", nextUrl);
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }

  function recommendedTabForRisk(risk: JsonRecord): string {
    const bag =
      `${str(risk.type, "")} ${str(risk.title, "")} ${str(risk.detail, "")}`.toLowerCase();
    if (bag.includes("auth") || bag.includes("security")) return "security";
    return "system";
  }

  function suggestionKindColor(
    kind: string,
  ): "default" | "success" | "warning" | "info" | "error" {
    const normalized = kind.toLowerCase();
    if (normalized === "watcher") return "info";
    if (normalized === "app") return "success";
    if (normalized === "workflow") return "warning";
    if (normalized === "task") return "default";
    return "default";
  }

  function openWorkspacePanel(view: string) {
    const normalized = (view || "").trim().toLowerCase();
    const path =
      normalized === "app" || normalized === "apps"
        ? "/ui/apps"
        : normalized === "task" || normalized === "tasks"
          ? "/ui/tasks"
          : normalized === "watcher" ||
              normalized === "watchers" ||
              normalized === "status"
            ? "/ui/background-work"
            : normalized === "session" || normalized === "sessions"
              ? "/ui/sessions"
            : normalized === "trace"
              ? "/ui/trace"
              : normalized === "document" ||
                      normalized === "documents" ||
                      normalized === "file" ||
                      normalized === "files"
                    ? "/ui/documents"
                    : normalized === "skill" || normalized === "skills"
                      ? "/ui/skills"
                      : normalized === "goal" || normalized === "goals"
                        ? "/ui/goals"
                        : "";
    if (!path) return;
    if (window.location.pathname !== path) {
      window.history.pushState(null, "", path);
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }

  async function runSuggestionAccept(suggestion: JsonRecord) {
    const suggestionId = str(suggestion.id, "");
    if (!suggestionId) return;
    const title = str(suggestion.title, "Suggested automation");
    setError(null);
    setSuccess(null);
    setActiveSuggestionActionId(suggestionId);
    setSuggestionRun({
      title,
      status: "running",
      summary: "Launching real execution run...",
      startedAt: new Date().toISOString(),
      suggestionId,
    });
    setSuggestionRunOpen(true);
    setSuggestionRunMinimized(false);

    try {
      const response = asRecord(
        await acceptSuggestionMutation.mutateAsync(suggestionId),
      );
      const run = asRecord(response.run);
      setSuggestionRun({
        title: str(run.title, title),
        status: "running",
        summary: str(run.summary, "Suggestion run started."),
        traceId: str(response.trace_id, str(run.trace_id, "")),
        startedAt: str(run.started_at, ""),
        suggestionId,
      });
      setSuccess("Suggestion run started.");
    } catch (e) {
      const message = errMessage(e);
      setSuggestionRun((current) => ({
        title: current?.title || title,
        status: "error",
        summary: message,
        traceId: current?.traceId,
        startedAt: current?.startedAt,
        completedAt: new Date().toISOString(),
        suggestionId,
      }));
      setSuccess(null);
      setError(message);
    } finally {
      setActiveSuggestionActionId(null);
    }
  }

  async function runSuggestionDismiss(suggestion: JsonRecord) {
    const suggestionId = str(suggestion.id, "");
    if (!suggestionId) return;
    setError(null);
    setSuccess(null);
    setActiveSuggestionActionId(suggestionId);
    try {
      await dismissSuggestionMutation.mutateAsync(suggestionId);
      setSuccess("Suggestion dismissed.");
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setActiveSuggestionActionId(null);
    }
  }

  useEffect(() => {
    if (!suggestionRun?.traceId) return;
    if (
      suggestionTraceQ.isLoading ||
      suggestionTraceQ.error ||
      !Object.keys(suggestionTrace).length
    )
      return;
    const traceStatusRaw = str(
      suggestionTrace.status,
      suggestionRun.status,
    ).toLowerCase();
    const lastSuggestionStep = asRecord(
      suggestionTraceSteps[suggestionTraceSteps.length - 1],
    );
    const lastSuggestionConsoleView = buildTraceStepConsoleView(
      suggestionTrace,
      suggestionTraceSteps,
      lastSuggestionStep,
    );
    const nextStatus: "running" | "completed" | "error" =
      traceStatusRaw === "failed" ||
      traceStatusRaw === "error" ||
      traceStatusRaw === "warning"
        ? "error"
        : traceStatusRaw === "completed"
          ? "completed"
          : "running";
    const nextSummary =
      str(suggestionTrace.response, "").trim() ||
      lastSuggestionConsoleView.detail ||
      suggestionRun.summary;
    const nextCompletedAt = str(
      suggestionTrace.completed_at,
      suggestionRun.completedAt || "",
    );
    const nextStartedAt = str(
      suggestionTrace.started_at,
      suggestionRun.startedAt || "",
    );
    if (
      suggestionRun.status !== nextStatus ||
      suggestionRun.summary !== nextSummary ||
      suggestionRun.completedAt !== nextCompletedAt ||
      suggestionRun.startedAt !== nextStartedAt
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
          : current,
      );
    }
  }, [
    suggestionRun,
    suggestionTrace,
    suggestionTraceQ.isLoading,
    suggestionTraceQ.error,
    suggestionTraceSteps,
  ]);

  useEffect(() => {
    if (!suggestionRun?.suggestionId) return;
    if (
      suggestionDetailQ.isLoading ||
      suggestionDetailQ.error ||
      !Object.keys(suggestionDetail).length
    )
      return;
    const runStatusRaw = str(
      suggestionDetail.run_status,
      suggestionRun.status,
    ).toLowerCase();
    const nextStatus: "running" | "completed" | "error" =
      runStatusRaw === "failed" || runStatusRaw === "error"
        ? "error"
        : runStatusRaw === "completed"
          ? "completed"
          : "running";
    const outcomeTitles = suggestionAcceptedOutcomes
      .map((row) => str(row.title, "").trim())
      .filter(Boolean);
    const outcomeSummary = outcomeTitles.length
      ? `Saved ${suggestionAcceptedOutcomes.length} outcome${suggestionAcceptedOutcomes.length === 1 ? "" : "s"}: ${outcomeTitles.slice(0, 3).join(", ")}${outcomeTitles.length > 3 ? ` (+${outcomeTitles.length - 3} more)` : ""}`
      : "";
    const currentSummary = str(suggestionRun.summary, "").trim();
    const genericSummary =
      !currentSummary ||
      currentSummary === "Launching real execution run..." ||
      currentSummary === "Suggestion run started." ||
      currentSummary.startsWith("Launched a real ");
    const nextSummary =
      str(suggestionDetail.last_run_error, "").trim() ||
      (nextStatus === "completed" && outcomeSummary && genericSummary
        ? outcomeSummary
        : suggestionRun.summary);
    const nextCompletedAt = str(
      suggestionDetail.last_run_completed_at,
      suggestionRun.completedAt || "",
    );
    const nextStartedAt = str(
      suggestionDetail.last_run_started_at,
      suggestionRun.startedAt || "",
    );
    if (
      suggestionRun.status !== nextStatus ||
      suggestionRun.summary !== nextSummary ||
      suggestionRun.completedAt !== nextCompletedAt ||
      suggestionRun.startedAt !== nextStartedAt
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
          : current,
      );
    }
  }, [
    suggestionRun,
    suggestionDetail,
    suggestionDetailQ.isLoading,
    suggestionDetailQ.error,
    suggestionAcceptedOutcomes,
  ]);

  useEffect(() => {
    if (settingsHydrated) return;
    if (!Object.keys(settingsRecord).length) return;
    const rawMode = str(settingsRecord.autonomy_mode, "assist").toLowerCase();
    const disabled =
      Boolean(settingsRecord.agent_paused ?? false) || rawMode === "off";
    if (disabled) {
      setAutonomyMode("off");
    } else if (rawMode === "auto" || rawMode === "assist") {
      setAutonomyMode(rawMode);
    } else {
      setAutonomyMode("assist");
    }
    setAlwaysAskHighRisk(Boolean(settingsRecord.always_ask_high_risk ?? true));
    setOnlyApprovedSkills(Boolean(settingsRecord.only_approved_skills ?? true));
    setQuietHoursStart(str(settingsRecord.quiet_hours_start, ""));
    setQuietHoursEnd(str(settingsRecord.quiet_hours_end, ""));
    const configuredLimit = settingsRecord.daily_run_limit;
    if (
      typeof configuredLimit === "number" &&
      Number.isFinite(configuredLimit)
    ) {
      setDailyRunLimit(String(configuredLimit));
    } else {
      setDailyRunLimit("");
    }
    setSettingsHydrated(true);
  }, [settingsHydrated, settingsRecord]);

  useEffect(() => {
    if (!showAdvanced) return;
    const maxAllowedTab = 3;
    if (tab > maxAllowedTab) {
      setTab(0);
    }
  }, [showAdvanced, tab]);

  async function saveBeginnerAutonomySettings(
    modeOverride?: "off" | "assist" | "auto",
  ) {
    setError(null);
    setSuccess(null);
    const selectedMode = modeOverride ?? autonomyMode;
    const normalizedLimit = dailyRunLimit.trim();
    let parsedLimit: number | null = null;
    if (normalizedLimit.length > 0) {
      const n = Number(normalizedLimit);
      if (!Number.isFinite(n) || n < 1) {
        setError("Daily run limit must be a positive number.");
        return;
      }
      parsedLimit = Math.round(n);
    }
    try {
      await saveAutonomySettingsMutation.mutateAsync({
        autonomy_mode: selectedMode,
        agent_paused: selectedMode === "off",
        always_ask_high_risk: alwaysAskHighRisk,
        only_approved_skills: onlyApprovedSkills,
        quiet_hours_start: quietHoursStart.trim() || null,
        quiet_hours_end: quietHoursEnd.trim() || null,
        daily_run_limit: parsedLimit,
      });
      setSuccess("Autonomy settings saved.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Ark Core"
        title="Mission Control"
        description="A calm overview of what AgentArk is doing, what needs you, and whether background help is healthy."
        actions={
          <Button
            size="small"
            variant="text"
            onClick={() => openAdvancedTab(controlsTabIndex)}
          >
            Advanced details
          </Button>
        }
      />
      <Box
        className="list-shell workspace-page-hero-shell"
        sx={{
          overflow: "hidden",
          background:
            "linear-gradient(135deg, var(--ui-rgba-8-26-48-940), var(--ui-rgba-12-18-28-960) 52%, var(--ui-rgba-15-15-18-960)), radial-gradient(circle at top left, var(--ui-rgba-69-206-255-180), transparent 46%)",
        }}
      >
        <Stack spacing={2}>
          <Stack
            direction={{ xs: "column", md: "row" }}
            spacing={2}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", md: "center" },
            }}
          >
            <Stack
              direction="row"
              spacing={1.5}
              sx={{
                alignItems: "center",
              }}
            >
              <Box
                className="shell-brand-mark"
                sx={{
                  width: 64,
                  height: 64,
                  borderRadius: "8px",
                  background: "var(--ui-rgba-8-18-32-450)",
                  boxShadow: "none",
                  "&::before": { inset: 5, borderRadius: "8px" },
                }}
              >
                <Box
                  component="img"
                  src={AgentLogo}
                  alt="AgentArk"
                  sx={{
                    width: 54,
                    height: 54,
                    position: "relative",
                    zIndex: 1,
                  }}
                />
              </Box>
              <Stack spacing={0.35}>
                <Typography
                  variant="caption"
                  sx={{
                    letterSpacing: 0,
                    textTransform: "uppercase",
                    color: "var(--ui-rgba-186-205-228-780)",
                  }}
                >
                  AgentArk
                </Typography>
                <Typography
                  variant="h4"
                  sx={{ fontWeight: 700, letterSpacing: 0 }}
                >
                  Mission Control
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                    maxWidth: 620,
                  }}
                >
                  Simple at a glance, detailed when you need it. AgentArk keeps
                  watch in the background and brings only the important
                  decisions back to you.
                </Typography>
              </Stack>
            </Stack>
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
              }}
            >
              <Chip
                size="small"
                color={
                  autonomyMode === "off"
                    ? "warning"
                    : autonomyMode === "auto"
                      ? "success"
                      : "info"
                }
                label={
                  autonomyMode === "off" ? "Paused" : `${modeIndicator} mode`
                }
              />
              <Chip
                size="small"
                color={selfEvolveStatusTone}
                label={`Self-evolve: ${selfEvolveStatusLabel}`}
              />
            </Stack>
          </Stack>
          <Alert severity={primaryStatusSeverity} sx={{ py: 1 }}>
            <Stack spacing={0.45}>
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                {primaryStatusTitle}
              </Typography>
              <Typography
                variant="body2"
                sx={{
                  color: "inherit",
                }}
              >
                {primaryStatusDetail}
              </Typography>
            </Stack>
          </Alert>
        </Stack>
      </Box>
      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 140, height: "100%" }}>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Overall status
            </Typography>
            <Typography variant="h6" sx={{ mt: 0.75, mb: 0.6 }}>
              {primaryStatusTitle}
            </Typography>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              {primaryStatusDetail}
            </Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 140, height: "100%" }}>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Waiting on you
            </Typography>
            <Typography variant="h4" sx={{ mt: 0.8, mb: 0.3 }}>
              {needsYouItems.length}
            </Typography>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              {needsYouSummary}
            </Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 140, height: "100%" }}>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Background activity
            </Typography>
            <Stack
              direction="row"
              spacing={1}
              sx={{
                alignItems: "center",
                mt: 0.8,
                mb: 0.8,
              }}
            >
              <Typography variant="h6">{backgroundActivityLabel}</Typography>
              <Chip
                size="small"
                color={backgroundActivityTone}
                label={suggestionScanLabel}
              />
            </Stack>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              {backgroundActivityDetail}
            </Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 140, height: "100%" }}>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Self-evolve
            </Typography>
            <Stack
              direction="row"
              spacing={1}
              sx={{
                alignItems: "center",
                mt: 0.8,
                mb: 0.8,
              }}
            >
              <Typography variant="h6">{selfEvolveStatusLabel}</Typography>
              <Chip
                size="small"
                color={selfEvolveStatusTone}
                label={`${selfEvolveDraftCount} draft${selfEvolveDraftCount === 1 ? "" : "s"}`}
              />
            </Stack>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              {selfEvolveDetail}
            </Typography>
          </Box>
        </Grid2>
      </Grid2>
      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell" sx={{ height: "100%" }}>
            <Stack spacing={1.1}>
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "flex-start", md: "center" },
                }}
              >
                <Box>
                  <Typography variant="h6">Needs you</Typography>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Only the items that need your approval, input, or review
                    appear here.
                  </Typography>
                </Box>
                {needsYouItems.length > 0 ? (
                  <Chip
                    size="small"
                    color="warning"
                    label={`${needsYouItems.length} waiting`}
                  />
                ) : (
                  <Chip size="small" color="success" label="Nothing waiting" />
                )}
              </Stack>
              {needsYouItems.length === 0 ? (
                <Alert severity="success" sx={{ py: 0.9 }}>
                  You do not need to do anything right now. AgentArk can keep
                  working in the background.
                </Alert>
              ) : (
                <Stack spacing={0.9}>
                  {needsYouItems.map((item) => (
                    <Stack
                      key={item.key}
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      className="action-row"
                      sx={{
                        alignItems: { xs: "flex-start", sm: "center" },
                        justifyContent: "space-between",
                      }}
                    >
                      <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                        <Typography variant="body2" sx={{ fontWeight: 700 }}>
                          {item.title}
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {item.detail}
                        </Typography>
                      </Stack>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={item.onClick}
                      >
                        {item.actionLabel}
                      </Button>
                    </Stack>
                  ))}
                </Stack>
              )}
            </Stack>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <Box className="list-shell" sx={{ height: "100%" }}>
            <Stack spacing={1.1}>
              <Typography variant="h6">Working in background</Typography>
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                Passive reassurance for novice users: what is active, what is
                paused, and what AgentArk is preparing next.
              </Typography>
              <Box className="action-row">
                <Stack spacing={0.45}>
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                      alignItems: "center",
                    }}
                  >
                    <Typography variant="body2" sx={{ fontWeight: 700 }}>
                      Safety state
                    </Typography>
                    <Chip
                      size="small"
                      color={alwaysAskHighRisk ? "success" : "warning"}
                      label={
                        alwaysAskHighRisk
                          ? "Confirmation on risky actions"
                          : "Risky actions can auto-run"
                      }
                    />
                  </Stack>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {modePlainHint}
                  </Typography>
                </Stack>
              </Box>
              <Box className="action-row">
                <Stack spacing={0.45}>
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                      alignItems: "center",
                    }}
                  >
                    <Typography variant="body2" sx={{ fontWeight: 700 }}>
                      Suggestion scan
                    </Typography>
                    <Chip
                      size="small"
                      color={backgroundActivityTone}
                      label={suggestionScanLabel}
                    />
                  </Stack>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Last pass: {suggestionLastRunLabel}. Next pass:{" "}
                    {suggestionNextRunLabel}. Tracked chats:{" "}
                    {num(suggestionScan.tracked_chats, 0)}.
                  </Typography>
                </Stack>
              </Box>
              <Box className="action-row">
                <Stack spacing={0.45}>
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                      alignItems: "center",
                    }}
                  >
                    <Typography variant="body2" sx={{ fontWeight: 700 }}>
                      Self-evolve progress
                    </Typography>
                    <Chip
                      size="small"
                      color={selfEvolveStatusTone}
                      label={selfEvolveStatusLabel}
                    />
                  </Stack>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {selfEvolveDetail}
                  </Typography>
                </Stack>
              </Box>
            </Stack>
          </Box>
        </Grid2>
      </Grid2>
      <Box className="list-shell">
        <Stack spacing={1.1}>
          <Stack
            direction={{ xs: "column", md: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", md: "center" },
            }}
          >
            <Box>
              <Typography variant="h6">Suggested next steps</Typography>
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                {suggestedAutomationSummary}
              </Typography>
            </Box>
            <Button
              size="small"
              variant="outlined"
              onClick={() => openAdvancedTab(suggestionsTabIndex)}
            >
              Review in advanced
            </Button>
          </Stack>
          {suggestionPreview.length === 0 &&
          recommendedActionPreview.length === 0 ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              No undeployed chat wishes are waiting right now.
            </Typography>
          ) : (
            <Stack spacing={0.9}>
              {recommendedActionPreview.map((action, idx) => {
                const readiness = readinessRecord(action.readiness);
                const trust = asRecord(action.trust);
                return (
                  <Box
                    key={str(action.id, `recommended-action-${idx}`)}
                    className="action-row"
                  >
                    <Stack spacing={0.45}>
                      <Stack
                        direction="row"
                        spacing={1}
                        useFlexGap
                        sx={{
                          flexWrap: "wrap",
                          alignItems: "center",
                        }}
                      >
                        <Chip
                          size="small"
                          color="primary"
                          label={humanizeMachineLabel(str(action.action_kind, "action"))}
                        />
                        <Typography variant="body2" sx={{ fontWeight: 700 }}>
                          {str(action.title, "Suggested action")}
                        </Typography>
                        <Chip
                          size="small"
                          variant="outlined"
                          label={`Risk ${num(trust.score, 0)}`}
                        />
                        {readiness ? (
                          <Chip
                            size="small"
                            clickable
                            color={readinessChipColor(str(readiness.stage, ""))}
                            label={readinessShortLabel(readiness)}
                            onClick={() =>
                              setReadinessDialog({
                                title: str(action.title, "Action readiness"),
                                readiness,
                              })
                            }
                          />
                        ) : null}
                      </Stack>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        {str(action.description, str(action.summary, ""))}
                      </Typography>
                      {readiness ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          Readiness: {str(readiness.plain_summary, "")}
                        </Typography>
                      ) : null}
                    </Stack>
                  </Box>
                );
              })}
              {suggestionPreview.map((suggestion, idx) => (
                <Box
                  key={str(suggestion.id, `suggestion-preview-${idx}`)}
                  className="action-row"
                >
                  <Stack spacing={0.45}>
                    <Stack
                      direction="row"
                      spacing={1}
                      useFlexGap
                      sx={{
                        flexWrap: "wrap",
                        alignItems: "center",
                      }}
                    >
                        <Chip
                          size="small"
                          color={suggestionKindColor(
                            str(suggestion.kind, "automation"),
                          )}
                          label={humanizeMachineLabel(str(suggestion.kind, "automation"))}
                        />
                      <Typography variant="body2" sx={{ fontWeight: 700 }}>
                        {str(suggestion.title, "Suggested automation")}
                      </Typography>
                    </Stack>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      {str(suggestion.detail, "")}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Why AgentArk suggested it: {str(suggestion.rationale, "")}
                    </Typography>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}
        </Stack>
      </Box>
      {false ? (
        <Box className="list-shell">
          <Typography
            variant="subtitle2"
            sx={{
              mb: 0.75,
            }}
          >
            Needs Your Attention
          </Typography>
          <Stack spacing={0.75}>
            {attentionRisks.slice(0, 4).map((risk, idx) => (
              <Stack
                key={`risk-${idx}`}
                direction={{ xs: "column", sm: "row" }}
                spacing={1}
                className="action-row"
                sx={{
                  alignItems: { xs: "flex-start", sm: "center" },
                  justifyContent: "space-between",
                }}
              >
                <Stack spacing={0.25} sx={{ minWidth: 0 }}>
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {str(risk.title, "Risk")}
                  </Typography>
                  <Typography
                    variant="caption"
                    noWrap
                    title={str(risk.detail, "")}
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {str(risk.detail, "")}
                  </Typography>
                </Stack>
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => openSettingsTab(recommendedTabForRisk(risk))}
                >
                  Open
                </Button>
              </Stack>
            ))}
          </Stack>
        </Box>
      ) : null}
      {false ? (
        <Box className="list-shell">
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              mb: 1,
            }}
          >
            <Typography variant="h6">Live Incidents</Typography>
            <Button
              size="small"
              onClick={() =>
                queryClient.invalidateQueries({
                  queryKey: ["autonomy-incidents-live"],
                })
              }
            >
              Refresh
            </Button>
          </Stack>
          {incidentsQ.error ? (
            <Alert severity="error">{errMessage(incidentsQ.error)}</Alert>
          ) : incidents.length === 0 ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              No incidents right now.
            </Typography>
          ) : (
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Severity</TableCell>
                    <TableCell>Title</TableCell>
                    <TableCell>Detail</TableCell>
                    <TableCell>ID</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {incidents.map((incident, idx) => {
                    const id = str(incident.id, `incident-${idx}`);
                    return (
                      <TableRow key={id}>
                        <TableCell>
                          <Chip
                            size="small"
                            label={str(incident.severity, "-")}
                            color={severityChipColor(
                              str(incident.severity, ""),
                            )}
                          />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 260 }}>
                          <Typography
                            variant="body2"
                            noWrap
                            title={str(incident.title, "-")}
                          >
                            {str(incident.title, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 420 }}>
                          <Typography
                            variant="body2"
                            noWrap
                            title={str(incident.detail, "-")}
                          >
                            {str(incident.detail, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 180 }}>
                          <Typography
                            variant="caption"
                            noWrap
                            title={id}
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {id}
                          </Typography>
                        </TableCell>
                        <TableCell align="right">
                          <RowOpsMenu
                            actions={[
                              {
                                label: "Run Playbook",
                                disabled: executeIncidentMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  setSuccess(null);
                                  setIncidentResult(null);
                                  try {
                                    const out = asRecord(
                                      await executeIncidentMutation.mutateAsync(
                                        id,
                                      ),
                                    );
                                    setIncidentResult(out);
                                    setSuccess("Incident playbook executed.");
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                            ]}
                            ariaLabel="Incident options"
                          />
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          )}
          {incidentResult ? (
            <Box sx={{ mt: 1 }}>
              <KeyValuePanel
                title="Last playbook result"
                data={incidentResult ?? {}}
              />
            </Box>
          ) : null}
        </Box>
      ) : null}
      {false ? (
        <Box className="list-shell">
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              mb: 1,
            }}
          >
            <Typography variant="h6">Timeline & Rollback</Typography>
            <Button
              size="small"
              onClick={() =>
                queryClient.invalidateQueries({
                  queryKey: ["autonomy-timeline"],
                })
              }
            >
              Refresh
            </Button>
          </Stack>
          {timelineQ.error ? (
            <Alert severity="error">{errMessage(timelineQ.error)}</Alert>
          ) : timelineEvents.length === 0 ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              No timeline events yet.
            </Typography>
          ) : (
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Time</TableCell>
                    <TableCell>Source</TableCell>
                    <TableCell>Title</TableCell>
                    <TableCell>Status</TableCell>
                    <TableCell>Detail</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {timelineEvents.map((event, idx) => {
                    const eventId = str(event.id, `event-${idx}`);
                    const status = str(event.status, "");
                    const rollback = asRecord(event.rollback);
                    const operation = str(rollback.operation, "");
                    const effectiveOp = effectiveRollbackOperation(
                      operation,
                      status,
                    );
                    const canRollback = !!operation && operation !== "none";
                    return (
                      <TableRow key={eventId}>
                        <TableCell
                          sx={{ whiteSpace: "nowrap" }}
                          title={humanTs(str(event.timestamp, "-")).tip}
                        >
                          {humanTs(str(event.timestamp, "-")).label}
                        </TableCell>
                        <TableCell>{str(event.source, "-")}</TableCell>
                        <TableCell sx={{ maxWidth: 280 }}>
                          <Typography
                            variant="body2"
                            noWrap
                            title={str(event.title, "-")}
                          >
                            {str(event.title, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell>{humanizeStatusLabel(status, "-")}</TableCell>
                        <TableCell sx={{ maxWidth: 360 }}>
                          <Typography
                            variant="caption"
                            noWrap
                            title={str(event.detail, "-")}
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {str(event.detail, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell align="right">
                          {canRollback ? (
                            <RowOpsMenu
                              actions={[
                                {
                                  label:
                                    rollingBackEventId === eventId
                                      ? "Applying..."
                                      : rollbackLabel(effectiveOp || operation),
                                  disabled:
                                    rollbackMutation.isPending ||
                                    rollingBackEventId === eventId,
                                  onClick: async () => {
                                    setError(null);
                                    setSuccess(null);
                                    setRollingBackEventId(eventId);
                                    try {
                                      await rollbackMutation.mutateAsync({
                                        event_id: eventId,
                                        operation: effectiveOp || undefined,
                                      });
                                      setSuccess(
                                        `Rollback applied: ${rollbackLabel(effectiveOp || operation)}.`,
                                      );
                                    } catch (e) {
                                      setError(errMessage(e));
                                    } finally {
                                      setRollingBackEventId(null);
                                    }
                                  },
                                },
                              ]}
                              ariaLabel="Timeline event options"
                            />
                          ) : (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              n/a
                            </Typography>
                          )}
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </Box>
      ) : null}
      {false ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Inbox Triage
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Labels"
                  value={triageLabelsCsv}
                  onChange={(e) => setTriageLabelsCsv(e.target.value)}
                  helperText="Comma-separated labels. Default: Act now, Delegate, Ignore"
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={5}
                  label="Messages JSON (optional)"
                  value={triageMessagesJson}
                  onChange={(e) => setTriageMessagesJson(e.target.value)}
                  placeholder='[{"id":"m1","from":"boss@company.com","subject":"Budget","snippet":"Need approval today"}]'
                  helperText="Leave empty to triage recent notifications automatically."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Button
                  variant="contained"
                  disabled={triageMutation.isPending}
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    setTriageResult(null);
                    try {
                      const out = asRecord(
                        await triageMutation.mutateAsync({
                          labels: parseCsv(triageLabelsCsv),
                          messages: parseTriageMessages(triageMessagesJson),
                        }),
                      );
                      setTriageResult(out);
                      setSuccess("Inbox triage complete.");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {triageMutation.isPending ? "Running..." : "Run Triage"}
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Triage Results
            </Typography>
            {triageRows.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                Run triage to see classification and draft replies.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Message</TableCell>
                      <TableCell>Label</TableCell>
                      <TableCell>Reason</TableCell>
                      <TableCell>Draft Reply</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {triageRows.map((row, idx) => (
                      <TableRow key={str(row.message_id, `triage-${idx}`)}>
                        <TableCell sx={{ maxWidth: 180 }}>
                          <Typography
                            variant="caption"
                            noWrap
                            title={str(row.message_id, "-")}
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {str(row.message_id, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell>
                          <Chip
                            size="small"
                            label={str(row.label, "-")}
                            variant="outlined"
                          />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 320 }}>
                          <Typography
                            variant="body2"
                            noWrap
                            title={str(row.reason, "-")}
                          >
                            {str(row.reason, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 480 }}>
                          <Typography
                            variant="body2"
                            noWrap
                            title={str(row.draft_reply, "-")}
                          >
                            {str(row.draft_reply, "-")}
                          </Typography>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}
      {false ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Stack
              direction="row"
              sx={{
                justifyContent: "space-between",
                alignItems: "center",
                mb: 1,
              }}
            >
              <Typography variant="h6">Browser Sessions</Typography>
              <Button
                size="small"
                onClick={() =>
                  queryClient.invalidateQueries({
                    queryKey: ["autonomy-browser-sessions"],
                  })
                }
              >
                Refresh
              </Button>
            </Stack>
            {browserSessionsQ.error ? (
              <Alert severity="error">
                {errMessage(browserSessionsQ.error)}
              </Alert>
            ) : browserSessions.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No active browser sessions.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>ID</TableCell>
                      <TableCell>Task</TableCell>
                      <TableCell>Status</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {browserSessions.map((session, idx) => {
                      const id = str(session.id, `session-${idx}`);
                      return (
                        <TableRow key={id}>
                          <TableCell sx={{ maxWidth: 180 }}>
                            <Typography
                              variant="caption"
                              noWrap
                              title={id}
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {id}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 360 }}>
                            <Typography
                              variant="body2"
                              noWrap
                              title={str(session.task, "-")}
                            >
                              {str(session.task, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 260 }}>
                            <Typography
                              variant="body2"
                              noWrap
                              title={str(session.status, "-")}
                            >
                              {humanizeStatusLabel(str(session.status, ""), "-")}
                            </Typography>
                          </TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Select",
                                  onClick: () => {
                                    setSelectedSessionId(id);
                                    setSessionResponse("");
                                  },
                                },
                                {
                                  label: "Status",
                                  onClick: async () => {
                                    if (selectedSessionId !== id) {
                                      setSelectedSessionId(id);
                                      return;
                                    }
                                    await browserStatusQ.refetch();
                                  },
                                },
                              ]}
                              ariaLabel="Browser session options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>

          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Respond to Session
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Selected session ID"
                  value={selectedSessionId}
                  onChange={(e) => setSelectedSessionId(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Button
                  fullWidth
                  variant="outlined"
                  disabled={
                    !selectedSessionId.trim() || browserStatusQ.isFetching
                  }
                  onClick={() => browserStatusQ.refetch()}
                >
                  {browserStatusQ.isFetching ? "Checking..." : "Check Status"}
                </Button>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Current status:{" "}
                  {str(
                    browserStatus.status,
                    str(
                      browserStatus.error,
                      selectedSessionId ? "unknown" : "select a session",
                    ),
                  )}
                </Typography>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={3}
                  label="Response"
                  value={sessionResponse}
                  onChange={(e) => setSessionResponse(e.target.value)}
                  placeholder="Example: Continue with the first result and summarize key points."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Button
                  variant="contained"
                  disabled={
                    !selectedSessionId.trim() ||
                    !sessionResponse.trim() ||
                    browserRespondMutation.isPending
                  }
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    setBrowserRespondResult(null);
                    try {
                      const out = asRecord(
                        await browserRespondMutation.mutateAsync({
                          id: selectedSessionId.trim(),
                          response: sessionResponse.trim(),
                        }),
                      );
                      setBrowserRespondResult(out);
                      setSuccess("Response sent to browser session.");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {browserRespondMutation.isPending
                    ? "Sending..."
                    : "Send Response"}
                </Button>
              </Grid2>
            </Grid2>
            {browserRespondResult ? (
              <Box sx={{ mt: 1 }}>
                <KeyValuePanel
                  title="Last response result"
                  data={browserRespondResult ?? {}}
                />
              </Box>
            ) : null}
          </Box>
        </Stack>
      ) : null}
      <Dialog
        open={showAdvanced}
        onClose={() => {
          setShowAdvanced(false);
          setTab(controlsTabIndex);
        }}
        maxWidth="xl"
        fullWidth
      >
        <DialogTitle>
          <Stack spacing={0.5}>
            <Typography variant="h6">Mission Control Advanced</Typography>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              Detailed controls, operator tools, and self-evolve internals live
              here so the main page can stay simple.
            </Typography>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={2}>
            <Alert severity={primaryStatusSeverity} sx={{ py: 0.75 }}>
              {waitingStatusLine}
            </Alert>
            <Tabs
              value={tab}
              onChange={(_, value) => setTab(Number(value) || 0)}
              variant="scrollable"
              scrollButtons="auto"
              allowScrollButtonsMobile
            >
              <Tab label="Controls" value={controlsTabIndex} />
              <Tab label="Suggestions" value={suggestionsTabIndex} />
              <Tab label="Self-evolve" value={selfEvolveTabIndex} />
              <Tab label="Ops" value={opsTabIndex} />
            </Tabs>

            {tab === controlsTabIndex ? (
              <Box className="list-shell">
                <Stack spacing={1.25}>
                  <Typography variant="h6">Automation guardrails</Typography>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Pause and resume autonomy from Settings &gt; Advanced. This
                    section keeps the safety rules for background work in one
                    place.
                  </Typography>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {modePlainHint}
                  </Typography>
                  <Alert
                    severity={autonomyMode === "off" ? "warning" : "info"}
                    sx={{ py: 0.75 }}
                  >
                    {autonomyMode === "off"
                      ? "Autonomy is paused from Settings > Advanced. Scheduled reminders still fire while proactive systems stay paused."
                      : `Autonomy is running in ${modeIndicator} mode. Use Settings > Advanced if you need to pause or resume background autonomy.`}
                  </Alert>
                  <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                    <Chip
                      size="small"
                      color={
                        autonomyMode === "off"
                          ? "warning"
                          : autonomyMode === "auto"
                            ? "success"
                            : "info"
                      }
                      label={
                        autonomyMode === "off"
                          ? "Paused"
                          : `${modeIndicator} mode`
                      }
                    />
                    <Button
                      size="small"
                      variant="outlined"
                      onClick={() => openSettingsTab("advanced")}
                      sx={{ width: { xs: "100%", sm: "fit-content" } }}
                    >
                      Open Settings &gt; Advanced
                    </Button>
                  </Stack>
                  <Grid2 container spacing={1}>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <FormControlLabel
                        control={
                          <Switch
                            checked={alwaysAskHighRisk}
                            onChange={(e) =>
                              setAlwaysAskHighRisk(e.target.checked)
                            }
                          />
                        }
                        label="Ask before risky actions"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <FormControlLabel
                        control={
                          <Switch
                            checked={onlyApprovedSkills}
                            onChange={(e) =>
                              setOnlyApprovedSkills(e.target.checked)
                            }
                          />
                        }
                        label="Use only approved skills"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        type="time"
                        label="Quiet hours start (local)"
                        value={quietHoursStart}
                        onChange={(e) => setQuietHoursStart(e.target.value)}
                        helperText="Avoid starting new runs after this time."
                        slotProps={{
                          inputLabel: { shrink: true },
                        }}
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        type="time"
                        label="Quiet hours end (local)"
                        value={quietHoursEnd}
                        onChange={(e) => setQuietHoursEnd(e.target.value)}
                        helperText="Resume normal runs after this time."
                        slotProps={{
                          inputLabel: { shrink: true },
                        }}
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        type="number"
                        label="Daily run limit"
                        value={dailyRunLimit}
                        onChange={(e) => setDailyRunLimit(e.target.value)}
                        error={dailyRunLimitInvalid}
                        helperText={
                          dailyRunLimitInvalid
                            ? "Enter a positive number (1 or more), or leave blank."
                            : "Leave blank for no cap."
                        }
                        slotProps={{
                          htmlInput: { min: 1, max: 1000 },
                        }}
                      />
                    </Grid2>
                  </Grid2>
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                    }}
                  >
                    <Button
                      variant="contained"
                      onClick={() => saveBeginnerAutonomySettings()}
                      disabled={
                        saveAutonomySettingsMutation.isPending ||
                        settingsQ.isFetching ||
                        !guardrailsDirty ||
                        dailyRunLimitInvalid
                      }
                    >
                      {saveAutonomySettingsMutation.isPending
                        ? "Saving..."
                        : "Save changes"}
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            ) : null}
            {tab === suggestionsTabIndex ? (
              <Box className="list-shell">
                <Stack spacing={1.25}>
                  <Stack
                    direction={{ xs: "column", md: "row" }}
                    spacing={1}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: { xs: "flex-start", md: "center" },
                    }}
                  >
                    <Box>
                      <Typography variant="h6">
                        Suggested automations
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Chat is scanned every 12 hours only when the server is
                        quiet. Busy periods defer automatically.
                      </Typography>
                    </Box>
                    <Chip
                      size="small"
                      color={
                        suggestionScanStatus === "error"
                          ? "error"
                          : suggestionScanStatus === "deferred_busy"
                            ? "warning"
                            : suggestionScanStatus === "completed"
                              ? "success"
                              : "default"
                      }
                      label={`Scan: ${suggestionScanLabel}`}
                    />
                  </Stack>
                  <Alert
                    severity={
                      suggestionScanStatus === "error"
                        ? "error"
                        : suggestionScanStatus === "deferred_busy"
                          ? "warning"
                          : "info"
                    }
                    sx={{ py: 0.75 }}
                  >
                    Last run: {suggestionLastRunLabel} | Next run:{" "}
                    {suggestionNextRunLabel} | Batch cap:{" "}
                    {num(suggestionScan.last_examined_chats, 0) > 0
                      ? `${num(suggestionScan.last_examined_chats, 0)} chat(s) last pass`
                      : "12 chats per pass"}{" "}
                    | Tracked chats: {num(suggestionScan.tracked_chats, 0)}
                  </Alert>
                  {suggestedAutomations.length === 0 ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      No undeployed chat wishes are waiting right now.
                    </Typography>
                  ) : (
                    <Stack spacing={1}>
                      {suggestedAutomations.map((suggestion, idx) => {
                        const suggestionId = str(
                          suggestion.id,
                          `suggestion-${idx}`,
                        );
                        const kind = str(suggestion.kind, "automation");
                        const busy = activeSuggestionActionId === suggestionId;
                        return (
                          <Box key={suggestionId} className="action-row">
                            <Stack spacing={1}>
                              <Stack
                                direction={{ xs: "column", md: "row" }}
                                spacing={1}
                                sx={{
                                  justifyContent: "space-between",
                                  alignItems: {
                                    xs: "flex-start",
                                    md: "center",
                                  },
                                }}
                              >
                                <Stack
                                  direction="row"
                                  spacing={1}
                                  useFlexGap
                                  sx={{
                                    flexWrap: "wrap",
                                    alignItems: "center",
                                  }}
                                >
                                  <Chip
                                    size="small"
                                    color={suggestionKindColor(kind)}
                                    label={humanizeMachineLabel(str(suggestion.kind, "automation"))}
                                  />
                                  <Typography
                                    variant="body2"
                                    sx={{ fontWeight: 600 }}
                                  >
                                    {str(
                                      suggestion.title,
                                      "Suggested automation",
                                    )}
                                  </Typography>
                                </Stack>
                                <Stack direction="row" spacing={1}>
                                  <Button
                                    size="small"
                                    variant="contained"
                                    disabled={busy}
                                    onClick={() =>
                                      void runSuggestionAccept(suggestion)
                                    }
                                  >
                                    {busy && acceptSuggestionMutation.isPending
                                      ? "Starting..."
                                      : "Accept"}
                                  </Button>
                                  <Button
                                    size="small"
                                    variant="outlined"
                                    color="inherit"
                                    disabled={busy}
                                    onClick={() =>
                                      void runSuggestionDismiss(suggestion)
                                    }
                                  >
                                    {busy && dismissSuggestionMutation.isPending
                                      ? "Dismissing..."
                                      : "Dismiss"}
                                  </Button>
                                </Stack>
                              </Stack>
                              <Typography
                                variant="body2"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {str(suggestion.detail, "")}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Why this was suggested:{" "}
                                {str(suggestion.rationale, "")}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Source chat:{" "}
                                {str(suggestion.source_snippet, "")}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Accept launches a real execution run, opens a
                                live trace window, and shows step-by-step logs
                                while the agent builds the app, watcher, or
                                workflow.
                              </Typography>
                            </Stack>
                          </Box>
                        );
                      })}
                    </Stack>
                  )}
                </Stack>
              </Box>
            ) : null}
            {tab === selfEvolveTabIndex ? (
              <Stack spacing={2}>
                <Alert
                  severity={
                    selfEvolveStatusTone === "warning" ? "warning" : "info"
                  }
                  sx={{ py: 0.8 }}
                >
                  Self-evolve is where AgentArk learns from completed work,
                  tests better routing in the background, and prepares changes
                  for promotion without cluttering the main page. Runtime
                  switches live in Settings &gt; Advanced.
                </Alert>
                <Grid2 container spacing={2}>
                  <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 120, height: "100%" }}
                    >
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Self-evolve
                      </Typography>
                      <Stack
                        direction="row"
                        spacing={1}
                        sx={{
                          alignItems: "center",
                          mt: 0.8,
                        }}
                      >
                        <Typography variant="h6">
                          {selfEvolveStatusLabel}
                        </Typography>
                        <Chip
                          size="small"
                          color={selfEvolveStatusTone}
                          label={selfEvolveEnabled ? "Enabled" : "Disabled"}
                        />
                      </Stack>
                    </Box>
                  </Grid2>
                  <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 120, height: "100%" }}
                    >
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Learning queue
                      </Typography>
                      <Typography variant="h4" sx={{ mt: 0.8 }}>
                        {selfEvolveBacklogCount}
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Items waiting for consolidation from recent work.
                      </Typography>
                    </Box>
                  </Grid2>
                  <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 120, height: "100%" }}
                    >
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Draft candidates
                      </Typography>
                      <Typography variant="h4" sx={{ mt: 0.8 }}>
                        {selfEvolveDraftCount}
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Candidate improvements waiting for evaluation or
                        approval.
                      </Typography>
                    </Box>
                  </Grid2>
                  <Grid2 size={{ xs: 12, sm: 6, xl: 3 }}>
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 120, height: "100%" }}
                    >
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Canary rollout
                      </Typography>
                      <Typography variant="h4" sx={{ mt: 0.8 }}>
                        {num(evolutionCanary.rollout_percent, 0)}%
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Candidate traffic share when testing is enabled.
                      </Typography>
                    </Box>
                  </Grid2>
                </Grid2>
                <Box className="list-shell">
                  {evolutionQ.isLoading ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Loading self-evolve details...
                    </Typography>
                  ) : evolutionQ.error ? (
                    <Alert severity="error">
                      {errMessage(evolutionQ.error)}
                    </Alert>
                  ) : (
                    <Stack spacing={1}>
                      <Stack
                        direction="row"
                        spacing={1}
                        useFlexGap
                        sx={{
                          flexWrap: "wrap",
                        }}
                      >
                        <Chip
                          size="small"
                          color={selfEvolveEnabled ? "success" : "default"}
                          label={`Self-evolve ${selfEvolveEnabled ? "On" : "Off"}`}
                        />
                        <Chip
                          size="small"
                          color={
                            selfEvolveCanaryEnabled ? "warning" : "default"
                          }
                          label={`Canary ${selfEvolveCanaryEnabled ? "On" : "Off"}`}
                        />
                      </Stack>
                      <Typography variant="body2">
                        Baseline policy:{" "}
                        {str(
                          evolutionCanary.baseline_version,
                          "routing-policy-default-v1",
                        )}
                      </Typography>
                      <Typography variant="body2">
                        Candidate policy:{" "}
                        {str(evolutionCanary.candidate_version, "-")}
                      </Typography>
                      <Typography variant="body2">
                        Last promotion:{" "}
                        {str(
                          evolution.last_promotion_result,
                          "No evolution runs yet",
                        )}
                      </Typography>
                      <Typography variant="body2">
                        Promotion mode: {humanizeMachineLabel(str(evolution.promotion_mode, "none"))}
                      </Typography>
                      <Typography variant="body2">
                        Replay gate: {humanizeStatusLabel(str(evolution.replay_gate_result, ""), "-")}
                      </Typography>
                      <Typography variant="body2">
                        Learning queue: provisional{" "}
                        {num(evolutionLearningQueue.provisional_runs, 0)} |
                        backlog{" "}
                        {num(evolutionLearningQueue.pending_consolidation, 0)} |
                        drafts {num(evolutionLearningQueue.draft_candidates, 0)}{" "}
                        | active patterns{" "}
                        {num(evolutionLearningQueue.active_patterns, 0)}
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Configure Evolve switches in Settings &gt; Advanced.
                      </Typography>
                    </Stack>
                  )}
                </Box>
              </Stack>
            ) : null}
            {tab === opsTabIndex ? (
              <Stack spacing={2}>
                <Box className="list-shell">
                  <Stack
                    direction="row"
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "center",
                      mb: 1,
                    }}
                  >
                    <Typography variant="h6">Live incidents</Typography>
                    <Button
                      size="small"
                      onClick={() =>
                        queryClient.invalidateQueries({
                          queryKey: ["autonomy-incidents-live"],
                        })
                      }
                    >
                      Refresh
                    </Button>
                  </Stack>
                  {incidentsQ.error ? (
                    <Alert severity="error">
                      {errMessage(incidentsQ.error)}
                    </Alert>
                  ) : incidents.length === 0 ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      No incidents right now.
                    </Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>Severity</TableCell>
                            <TableCell>Title</TableCell>
                            <TableCell>Detail</TableCell>
                            <TableCell>ID</TableCell>
                            <TableCell align="right">Ops</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {incidents.map((incident, idx) => {
                            const id = str(incident.id, `incident-${idx}`);
                            return (
                              <TableRow key={id}>
                                <TableCell>
                                  <Chip
                                    size="small"
                                    label={str(incident.severity, "-")}
                                    color={severityChipColor(
                                      str(incident.severity, ""),
                                    )}
                                  />
                                </TableCell>
                                <TableCell sx={{ maxWidth: 260 }}>
                                  <Typography
                                    variant="body2"
                                    noWrap
                                    title={str(incident.title, "-")}
                                  >
                                    {str(incident.title, "-")}
                                  </Typography>
                                </TableCell>
                                <TableCell sx={{ maxWidth: 420 }}>
                                  <Typography
                                    variant="body2"
                                    noWrap
                                    title={str(incident.detail, "-")}
                                  >
                                    {str(incident.detail, "-")}
                                  </Typography>
                                </TableCell>
                                <TableCell sx={{ maxWidth: 180 }}>
                                  <Typography
                                    variant="caption"
                                    noWrap
                                    title={id}
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {id}
                                  </Typography>
                                </TableCell>
                                <TableCell align="right">
                                  <RowOpsMenu
                                    actions={[
                                      {
                                        label: "Run Playbook",
                                        disabled:
                                          executeIncidentMutation.isPending,
                                        onClick: async () => {
                                          setError(null);
                                          setSuccess(null);
                                          setIncidentResult(null);
                                          try {
                                            const out = asRecord(
                                              await executeIncidentMutation.mutateAsync(
                                                id,
                                              ),
                                            );
                                            setIncidentResult(out);
                                            setSuccess(
                                              "Incident playbook executed.",
                                            );
                                          } catch (e) {
                                            setError(errMessage(e));
                                          }
                                        },
                                      },
                                    ]}
                                    ariaLabel="Incident options"
                                  />
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  )}
                  {incidentResult ? (
                    <Box sx={{ mt: 1 }}>
                      <KeyValuePanel
                        title="Last playbook result"
                        data={incidentResult}
                      />
                    </Box>
                  ) : null}
                </Box>

                <Box className="list-shell">
                  <Stack
                    direction="row"
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "center",
                      mb: 1,
                    }}
                  >
                    <Typography variant="h6">Browser sessions</Typography>
                    <Button
                      size="small"
                      onClick={() =>
                        queryClient.invalidateQueries({
                          queryKey: ["autonomy-browser-sessions"],
                        })
                      }
                    >
                      Refresh
                    </Button>
                  </Stack>
                  {browserSessionsQ.error ? (
                    <Alert severity="error">
                      {errMessage(browserSessionsQ.error)}
                    </Alert>
                  ) : browserSessions.length === 0 ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      No active browser sessions.
                    </Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>ID</TableCell>
                            <TableCell>Task</TableCell>
                            <TableCell>Status</TableCell>
                            <TableCell align="right">Ops</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {browserSessions.map((session, idx) => {
                            const id = str(session.id, `session-${idx}`);
                            return (
                              <TableRow key={id}>
                                <TableCell sx={{ maxWidth: 180 }}>
                                  <Typography
                                    variant="caption"
                                    noWrap
                                    title={id}
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {id}
                                  </Typography>
                                </TableCell>
                                <TableCell sx={{ maxWidth: 360 }}>
                                  <Typography
                                    variant="body2"
                                    noWrap
                                    title={str(session.task, "-")}
                                  >
                                    {str(session.task, "-")}
                                  </Typography>
                                </TableCell>
                                <TableCell sx={{ maxWidth: 260 }}>
                                  <Typography
                                    variant="body2"
                                    noWrap
                                    title={str(session.status, "-")}
                                  >
                                    {humanizeStatusLabel(str(session.status, ""), "-")}
                                  </Typography>
                                </TableCell>
                                <TableCell align="right">
                                  <RowOpsMenu
                                    actions={[
                                      {
                                        label: "Select",
                                        onClick: () => {
                                          setSelectedSessionId(id);
                                          setSessionResponse("");
                                        },
                                      },
                                      {
                                        label: "Status",
                                        onClick: async () => {
                                          if (selectedSessionId !== id) {
                                            setSelectedSessionId(id);
                                            return;
                                          }
                                          await browserStatusQ.refetch();
                                        },
                                      },
                                    ]}
                                    ariaLabel="Browser session options"
                                  />
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  )}
                </Box>

                <Box className="list-shell">
                  <Typography
                    variant="h6"
                    sx={{
                      mb: 1,
                    }}
                  >
                    Respond to session
                  </Typography>
                  <Grid2 container spacing={1}>
                    <Grid2 size={{ xs: 12, md: 8 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Selected session ID"
                        value={selectedSessionId}
                        onChange={(e) => setSelectedSessionId(e.target.value)}
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <Button
                        fullWidth
                        variant="outlined"
                        disabled={
                          !selectedSessionId.trim() || browserStatusQ.isFetching
                        }
                        onClick={() => browserStatusQ.refetch()}
                      >
                        {browserStatusQ.isFetching
                          ? "Checking..."
                          : "Check Status"}
                      </Button>
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Current status:{" "}
                        {str(
                          browserStatus.status,
                          str(
                            browserStatus.error,
                            selectedSessionId ? "unknown" : "select a session",
                          ),
                        )}
                      </Typography>
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        multiline
                        minRows={3}
                        label="Response"
                        value={sessionResponse}
                        onChange={(e) => setSessionResponse(e.target.value)}
                        placeholder="Example: Continue with the first result and summarize key points."
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <Button
                        variant="contained"
                        disabled={
                          !selectedSessionId.trim() ||
                          !sessionResponse.trim() ||
                          browserRespondMutation.isPending
                        }
                        onClick={async () => {
                          setError(null);
                          setSuccess(null);
                          setBrowserRespondResult(null);
                          try {
                            const out = asRecord(
                              await browserRespondMutation.mutateAsync({
                                id: selectedSessionId.trim(),
                                response: sessionResponse.trim(),
                              }),
                            );
                            setBrowserRespondResult(out);
                            setSuccess("Response sent to browser session.");
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                      >
                        {browserRespondMutation.isPending
                          ? "Sending..."
                          : "Send Response"}
                      </Button>
                    </Grid2>
                  </Grid2>
                  {browserRespondResult ? (
                    <Box sx={{ mt: 1 }}>
                      <KeyValuePanel
                        title="Last response result"
                        data={browserRespondResult}
                      />
                    </Box>
                  ) : null}
                </Box>
              </Stack>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setShowAdvanced(false);
              setTab(controlsTabIndex);
            }}
          >
            Close
          </Button>
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
        detailError={suggestionDetailQ.error}
        acceptedOutcomes={suggestionAcceptedOutcomes}
        onClose={() => setSuggestionRunOpen(false)}
        onMinimize={() => setSuggestionRunMinimized(true)}
        onRestore={() => setSuggestionRunMinimized(false)}
        onOpenWorkspacePanel={openWorkspacePanel}
        getConsoleView={(stepRecord) =>
          buildTraceStepConsoleView(
            suggestionTrace,
            suggestionTraceSteps,
            stepRecord,
          )
        }
        getTraceStepColor={traceStepColor}
        humanTs={humanTs}
        errMessage={errMessage}
      />
      <Dialog
        open={!!readinessDialog}
        onClose={() => setReadinessDialog(null)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>{readinessDialog?.title || "Readiness details"}</DialogTitle>
        <DialogContent dividers>
          {readinessDialog ? (
            <Stack spacing={1.25}>
              <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
                <Chip
                  size="small"
                  color={readinessChipColor(str(readinessDialog.readiness.stage, ""))}
                  label={readinessShortLabel(readinessDialog.readiness)}
                />
                <Chip
                  size="small"
                  variant="outlined"
                  label={
                    toBool(readinessDialog.readiness.allows_auto)
                      ? "Auto-run allowed"
                      : toBool(readinessDialog.readiness.allows_review)
                        ? "Review allowed"
                        : "Watching only"
                  }
                />
              </Stack>
              <Typography variant="body2">
                {str(
                  readinessDialog.readiness.plain_summary,
                  "AgentArk is still collecting enough evidence.",
                )}
              </Typography>
              {valueStringList(readinessDialog.readiness.blockers).length > 0 ? (
                <Alert severity="warning" sx={{ borderRadius: 1 }}>
                  <Stack spacing={0.5}>
                    {valueStringList(readinessDialog.readiness.blockers).map(
                      (line, idx) => (
                        <Typography key={`autonomy-readiness-blocker-${idx}`} variant="body2">
                          {line}
                        </Typography>
                      ),
                    )}
                  </Stack>
                </Alert>
              ) : null}
              <Accordion disableGutters>
                <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                  <Typography variant="body2">Power-user signals</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <Box
                    component="pre"
                    sx={{
                      m: 0,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      fontSize: 12,
                      color: "text.secondary",
                    }}
                  >
                    {JSON.stringify(readinessDialog.readiness.signals ?? {}, null, 2)}
                  </Box>
                </AccordionDetails>
              </Accordion>
            </Stack>
          ) : null}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setReadinessDialog(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      {settingsQ.error ||
      briefingQ.error ||
      notificationsQ.error ||
      error ||
      (showAdvanced && (timelineQ.error || browserStatusQ.error)) ? (
        <Alert severity="error">
          {error ||
            errMessage(
              settingsQ.error ||
                briefingQ.error ||
                notificationsQ.error ||
                (showAdvanced ? timelineQ.error || browserStatusQ.error : null),
            )}
        </Alert>
      ) : null}
      {success ? <Alert severity="success">{success}</Alert> : null}
    </WorkspacePageShell>
  );
}
