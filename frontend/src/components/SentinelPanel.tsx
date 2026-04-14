import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Divider,
  Stack,
  Switch,
  Tab,
  Tabs,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiRelativeDateTimeMeta } from "../lib/dateFormat";
import { SuggestionRunDialog, type SuggestionRunState } from "./SuggestionRunDialog";
import { WorkspacePageHeader, WorkspacePageShell } from "./WorkspacePage";
import type {
  SentinelBackgroundLearning,
  SentinelFeedResponse,
  SentinelObservation,
  SentinelProposal,
} from "../types";

const REFRESH_MS = 8000;
const SENTINEL_SECTION_PAGE_SIZE = 4;

type JsonRecord = Record<string, unknown>;

type SentinelFormState = {
  enabled: boolean;
  watch_in_app: boolean;
  watch_connected_services: boolean;
  infer_new_automations: boolean;
  confidence_threshold: string;
  max_proposals_per_scan: string;
  autonomy_mode: "off" | "assist" | "auto";
};

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  const root = asRecord(value);
  const items = root[key];
  return Array.isArray(items)
    ? items.filter((item): item is JsonRecord => !!item && typeof item === "object" && !Array.isArray(item))
    : [];
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function num(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
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

function buildTraceConsoleView(step: JsonRecord): { detail: string; dataText: string } {
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

function proposalTone(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = status.toLowerCase();
  if (normalized === "completed") return "success";
  if (normalized === "queued_for_approval" || normalized === "snoozed") return "warning";
  if (normalized === "failed") return "error";
  if (normalized === "running") return "info";
  return "default";
}

function proposalActionLabel(proposal: SentinelProposal): string {
  return proposal.proposal_kind === "chat_suggestion_accept" ? "Launch" : "Run";
}

function modeLabel(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "Off";
  if (mode === "auto") return "Auto";
  return "Suggest first";
}

function modeChoiceDescription(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "No background suggestions or automatic help.";
  if (mode === "auto") return "Handles lightweight routine help when it is confident enough.";
  return "Prepares ideas and waits for you to approve them.";
}

function modeChoiceEyebrow(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "Quiet";
  if (mode === "auto") return "Hands-on";
  return "Balanced";
}

function modeChoiceSupport(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "ArkSentinel stays available, but it will not prepare or run follow-up help.";
  if (mode === "auto") return "Low-risk routine help can run on its own when the confidence bar is met.";
  return "ArkSentinel drafts the next step first and waits for your approval before anything runs.";
}

function modeChoiceAccent(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "rgba(148,163,184,0.78)";
  if (mode === "auto") return "rgba(56,189,248,0.92)";
  return "rgba(168,130,255,0.92)";
}

function proposalStatusLabel(status: string): string {
  const normalized = status.trim().toLowerCase();
  if (normalized === "queued_for_approval") return "Waiting for approval";
  if (normalized === "running") return "Running";
  if (normalized === "completed") return "Completed";
  if (normalized === "snoozed") return "Later";
  if (normalized === "failed") return "Needs attention";
  if (!normalized) return "Open";
  return humanizeBackgroundKey(normalized);
}

function sourceKindLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return "AgentArk";
  if (normalized === "in_app_activity") return "Inside AgentArk";
  if (normalized === "connected_service" || normalized === "service_event") return "Connected apps";
  if (normalized === "chat") return "Chat";
  if (normalized === "observation") return "Recent activity";
  return humanizeBackgroundKey(normalized);
}

function observationKindLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return "Observation";
  if (normalized === "pattern" || normalized === "pattern_match") return "Repeated pattern";
  if (normalized === "opportunity") return "Opportunity";
  if (normalized === "risk") return "Heads-up";
  return humanizeBackgroundKey(normalized);
}

function priorityLabel(value: number): string {
  if (value <= 1) return "High priority";
  if (value === 2) return "Medium priority";
  return "Low priority";
}

type SentinelBooleanSettingKey =
  | "enabled"
  | "watch_in_app"
  | "watch_connected_services"
  | "infer_new_automations";

const SENTINEL_SIGNAL_OPTIONS: Array<{
  key: SentinelBooleanSettingKey;
  label: string;
  description: string;
}> = [
  {
    key: "enabled",
    label: "Keep ArkSentinel available",
    description: "Lets it stay ready in the background."
  },
  {
    key: "watch_in_app",
    label: "Pay attention inside AgentArk",
    description: "Uses your in-app activity to spot useful follow-ups."
  },
  {
    key: "watch_connected_services",
    label: "Pay attention to connected apps",
    description: "Uses signals from Gmail, Calendar, Slack, and other services once you connect them."
  },
  {
    key: "infer_new_automations",
    label: "Look for routines worth automating",
    description: "Spots repeated work that could become a reminder, watcher, or reusable flow."
  }
];

function blankForm(): SentinelFormState {
  return {
    enabled: true,
    watch_in_app: true,
    watch_connected_services: true,
    infer_new_automations: true,
    confidence_threshold: "0.72",
    max_proposals_per_scan: "6",
    autonomy_mode: "assist",
  };
}

function backgroundTone(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = status.trim().toLowerCase();
  if (["completed", "updated", "changed", "ok", "success"].includes(normalized)) return "success";
  if (["running", "queued", "working"].includes(normalized)) return "info";
  if (["paused", "disabled", "waiting"].includes(normalized)) return "warning";
  if (["failed", "error"].includes(normalized)) return "error";
  return "default";
}

function humanizeBackgroundKey(key: string): string {
  return key
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function compactText(value: string, maxChars = 120): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const chars = Array.from(trimmed);
  if (chars.length <= maxChars) return trimmed;
  return `${chars.slice(0, Math.max(0, maxChars - 3)).join("").trimEnd()}...`;
}

export function SentinelPanel({
  autoRefresh,
  navigateToView,
}: {
  autoRefresh: boolean;
  navigateToView: (view: string, replace?: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [sentinelTab, setSentinelTab] = useState<"overview" | "settings">("overview");
  const [form, setForm] = useState<SentinelFormState>(blankForm);
  const [hydrated, setHydrated] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [run, setRun] = useState<SuggestionRunState | null>(null);
  const [runOpen, setRunOpen] = useState(false);
  const [runMinimized, setRunMinimized] = useState(false);
  const [selectedProposalId, setSelectedProposalId] = useState("");
  const [selectedObservationId, setSelectedObservationId] = useState("");
  const [proposalPage, setProposalPage] = useState(0);
  const [observationPage, setObservationPage] = useState(0);

  const settingsQ = useQuery({
    queryKey: ["sentinel-settings"],
    queryFn: api.getSentinelSettings,
  });

  const feedQ = useQuery({
    queryKey: ["sentinel-feed"],
    queryFn: api.getSentinelFeed,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const runTraceId = run?.traceId || "";
  const runTraceQ = useQuery({
    queryKey: ["sentinel-run-trace", runTraceId],
    queryFn: () => api.rawGet(`/trace/${encodeURIComponent(runTraceId)}`),
    enabled: !!runTraceId && runOpen,
    refetchInterval: runOpen && !!runTraceId && run?.status === "running" ? REFRESH_MS : false,
  });

  const saveMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.updateSentinelSettings(payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["sentinel-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
    },
  });

  const approveMutation = useMutation({
    mutationFn: (id: string) => api.approveSentinelProposal(id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });

  const dismissMutation = useMutation({
    mutationFn: (id: string) => api.dismissSentinelProposal(id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
    },
  });

  const snoozeMutation = useMutation({
    mutationFn: (id: string) => api.snoozeSentinelProposal(id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
    },
  });

  useEffect(() => {
    if (hydrated || !settingsQ.data) return;
    const settings = settingsQ.data.settings;
    const rawMode = str(settingsQ.data.autonomy_mode, "assist").toLowerCase();
    setForm({
      enabled: Boolean(settings.enabled ?? true),
      watch_in_app: Boolean(settings.watch_in_app ?? true),
      watch_connected_services: Boolean(settings.watch_connected_services ?? true),
      infer_new_automations: Boolean(settings.infer_new_automations ?? true),
      confidence_threshold: String(num(settings.confidence_threshold, 0.72)),
      max_proposals_per_scan: String(num(settings.max_proposals_per_scan, 6)),
      autonomy_mode: rawMode === "off" || rawMode === "auto" ? rawMode : "assist",
    });
    setHydrated(true);
  }, [hydrated, settingsQ.data]);

  useEffect(() => {
    if (!run?.traceId) return;
    const trace = asRecord(runTraceQ.data);
    const steps = pickRecords(runTraceQ.data, "steps");
    if (runTraceQ.isLoading || runTraceQ.error || !Object.keys(trace).length) return;
    const status = str(trace.status, run.status).toLowerCase();
    const lastStep = steps[steps.length - 1] || {};
    const nextStatus: "running" | "completed" | "error" =
      status === "completed" ? "completed" : status === "failed" || status === "error" || status === "warning" ? "error" : "running";
    const nextSummary = str(trace.response, "").trim() || str(lastStep.detail, "").trim() || run.summary;
    const nextStartedAt = str(trace.started_at, run.startedAt || "");
    const nextCompletedAt = str(trace.completed_at, run.completedAt || "");
    if (
      nextStatus !== run.status ||
      nextSummary !== run.summary ||
      nextStartedAt !== (run.startedAt || "") ||
      nextCompletedAt !== (run.completedAt || "")
    ) {
      setRun((current) =>
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
  }, [run, runTraceQ.data, runTraceQ.error, runTraceQ.isLoading]);

  const feed = feedQ.data as SentinelFeedResponse | undefined;
  const openProposals = useMemo(
    () =>
      (feed?.proposals || []).filter((proposal) =>
        ["open", "running", "queued_for_approval", "snoozed"].includes(String(proposal.status || "").toLowerCase())
      ),
    [feed?.proposals]
  );
  const recentObservations = useMemo(() => feed?.observations || [], [feed?.observations]);
  const proposalPageCount = Math.max(1, Math.ceil(openProposals.length / SENTINEL_SECTION_PAGE_SIZE));
  const observationPageCount = Math.max(1, Math.ceil(recentObservations.length / SENTINEL_SECTION_PAGE_SIZE));
  const pagedOpenProposals = useMemo(
    () =>
      openProposals.slice(
        proposalPage * SENTINEL_SECTION_PAGE_SIZE,
        proposalPage * SENTINEL_SECTION_PAGE_SIZE + SENTINEL_SECTION_PAGE_SIZE
      ),
    [openProposals, proposalPage]
  );
  const pagedRecentObservations = useMemo(
    () =>
      recentObservations.slice(
        observationPage * SENTINEL_SECTION_PAGE_SIZE,
        observationPage * SENTINEL_SECTION_PAGE_SIZE + SENTINEL_SECTION_PAGE_SIZE
      ),
    [recentObservations, observationPage]
  );

  useEffect(() => {
    setProposalPage((current) => Math.min(current, Math.max(0, proposalPageCount - 1)));
  }, [proposalPageCount]);

  useEffect(() => {
    setObservationPage((current) => Math.min(current, Math.max(0, observationPageCount - 1)));
  }, [observationPageCount]);

  useEffect(() => {
    if (pagedOpenProposals.length === 0) {
      setSelectedProposalId("");
      return;
    }
    if (!pagedOpenProposals.some((proposal) => proposal.id === selectedProposalId)) {
      setSelectedProposalId(pagedOpenProposals[0]?.id || "");
    }
  }, [pagedOpenProposals, selectedProposalId]);

  useEffect(() => {
    if (pagedRecentObservations.length === 0) {
      setSelectedObservationId("");
      return;
    }
    if (!pagedRecentObservations.some((observation) => observation.id === selectedObservationId)) {
      setSelectedObservationId(pagedRecentObservations[0]?.id || "");
    }
  }, [pagedRecentObservations, selectedObservationId]);

  const selectedProposal = useMemo(
    () => pagedOpenProposals.find((proposal) => proposal.id === selectedProposalId) || pagedOpenProposals[0] || null,
    [pagedOpenProposals, selectedProposalId]
  );
  const selectedObservation = useMemo(
    () =>
      pagedRecentObservations.find((observation) => observation.id === selectedObservationId) ||
      pagedRecentObservations[0] ||
      null,
    [pagedRecentObservations, selectedObservationId]
  );

  const backgroundLearning: SentinelBackgroundLearning | null = feed?.background_learning || null;
  const scan = feed?.scan;
  const stats = feed?.stats;
  const autonomyDisabled = Boolean(settingsQ.data?.agent_paused) || str(settingsQ.data?.autonomy_mode, "assist").toLowerCase() === "off";
  const lastScanLabel = scan?.last_completed_at ? humanTs(scan.last_completed_at).label : "Waiting for the first check";
  const currentModeLabel = modeLabel(form.autonomy_mode);
  const sentinelHeroHeadline =
    settingsQ.data?.agent_paused
      ? "ArkSentinel is paused."
      : form.autonomy_mode === "off"
        ? "ArkSentinel is turned off."
        : openProposals.length > 0
          ? `${openProposals.length} follow-up${openProposals.length === 1 ? "" : "s"} waiting for you.`
          : recentObservations.length > 0
            ? "No action needed right now."
            : "No follow-ups right now.";
  const sentinelHeroDetail =
    settingsQ.data?.agent_paused
      ? "Turn autonomy back on to resume background checks, suggestions, and learning."
      : form.autonomy_mode === "off"
        ? "ArkSentinel is not scanning for follow-ups while this mode is off."
        : openProposals.length > 0
          ? "Review the suggested next steps below or leave them for later."
          : form.autonomy_mode === "auto"
            ? "ArkSentinel is scanning in the background and can handle lightweight routine work automatically."
            : "ArkSentinel is scanning in the background and will ask before it acts.";
  const heroTone =
    settingsQ.data?.agent_paused || form.autonomy_mode === "off"
      ? {
          border: "rgba(148, 163, 184, 0.24)",
          background: "linear-gradient(135deg, rgba(25, 29, 35, 0.96), rgba(15, 17, 21, 0.96))"
        }
      : openProposals.length > 0
        ? {
            border: "rgba(251, 191, 36, 0.24)",
            background: "linear-gradient(135deg, rgba(33, 27, 15, 0.96), rgba(15, 17, 21, 0.96))"
          }
        : {
            border: "rgba(59, 130, 246, 0.24)",
            background: "linear-gradient(135deg, rgba(13, 24, 38, 0.96), rgba(15, 17, 21, 0.96))"
          };
  const heroStats = [
    {
      label: "Waiting for you",
      value: String(openProposals.length),
      helper: openProposals.length === 0 ? "Nothing needs approval" : "Suggested next step" + (openProposals.length === 1 ? "" : "s")
    },
    {
      label: "Last check",
      value: lastScanLabel,
      helper: str(scan?.last_status, "idle") || "idle"
    },
    {
      label: "Connected apps",
      value: String(stats?.connected_services ?? 0),
      helper: `${stats?.connected_services ?? 0} connected service${(stats?.connected_services ?? 0) === 1 ? "" : "s"}`
    }
  ];
  const currentStatusSummary = str(scan?.last_error, "").trim()
    ? `The last background pass hit an issue: ${str(scan?.last_error, "").trim()}`
    : scan?.last_completed_at
      ? `ArkSentinel last checked ${humanTs(scan.last_completed_at).label}.`
      : "ArkSentinel has not completed its first background check yet.";
  const backgroundLearningSummary =
    str(backgroundLearning?.summary, "").trim() ||
    (autonomyDisabled
      ? "Learning is paused until background help is turned back on."
      : "ArkSentinel reviews recent activity to remember what worked, spot repeated patterns, and improve future suggestions.");
  const configuredThreshold = settingsQ.data ? num(settingsQ.data.settings.confidence_threshold, 0.72) : 0.72;
  const configuredMax = settingsQ.data ? num(settingsQ.data.settings.max_proposals_per_scan, 6) : 6;
  const configuredMode = str(settingsQ.data?.autonomy_mode, "assist").toLowerCase();
  const dirty =
    hydrated &&
    (form.enabled !== Boolean(settingsQ.data?.settings.enabled ?? true) ||
      form.watch_in_app !== Boolean(settingsQ.data?.settings.watch_in_app ?? true) ||
      form.watch_connected_services !== Boolean(settingsQ.data?.settings.watch_connected_services ?? true) ||
      form.infer_new_automations !== Boolean(settingsQ.data?.settings.infer_new_automations ?? true) ||
      Number(form.confidence_threshold) !== configuredThreshold ||
      Number(form.max_proposals_per_scan) !== configuredMax ||
      form.autonomy_mode !== (configuredMode === "off" || configuredMode === "auto" ? configuredMode : "assist"));

  async function saveSettings() {
    setError(null);
    setSuccess(null);
    const threshold = Number(form.confidence_threshold);
    const maxProposals = Number(form.max_proposals_per_scan);
    try {
      await saveMutation.mutateAsync({
        enabled: form.enabled,
        watch_in_app: form.watch_in_app,
        watch_connected_services: form.watch_connected_services,
        infer_new_automations: form.infer_new_automations,
        confidence_threshold: Number.isFinite(threshold) ? Math.min(1, Math.max(0.1, threshold)) : 0.72,
        max_proposals_per_scan: Number.isFinite(maxProposals) ? Math.min(20, Math.max(1, Math.round(maxProposals))) : 6,
        autonomy_mode: form.autonomy_mode,
      });
      setSuccess("ArkSentinel settings saved.");
    } catch (saveError) {
      setError(errMessage(saveError));
    }
  }

  async function runProposal(proposal: SentinelProposal) {
    setError(null);
    setSuccess(null);
    setRun({
      title: proposal.title,
      status: "running",
      summary: "Launching ArkSentinel proposal...",
      startedAt: new Date().toISOString(),
      suggestionId: proposal.id,
    });
    setRunOpen(true);
    setRunMinimized(false);
    try {
      const response = await approveMutation.mutateAsync(proposal.id);
      const traceId = str(response.trace_id, str(asRecord(response.proposal).trace_id, "")).trim();
      const proposalRecord = asRecord(response.proposal);
      const runStatus = str(proposalRecord.run_status, "").toLowerCase();
      setRun({
        title: proposal.title,
        status:
          runStatus === "failed"
            ? "error"
            : traceId && runStatus !== "queued_for_approval"
              ? "running"
              : "completed",
        summary: str(response.message, "ArkSentinel proposal accepted."),
        traceId: traceId || undefined,
        startedAt: new Date().toISOString(),
        completedAt: runStatus === "queued_for_approval" || !traceId ? new Date().toISOString() : undefined,
        suggestionId: proposal.id,
      });
      setSuccess("ArkSentinel proposal accepted.");
    } catch (runError) {
      const message = errMessage(runError);
      setRun((current) =>
        current
          ? {
              ...current,
              status: "error",
              summary: message,
              completedAt: new Date().toISOString(),
            }
          : current
      );
      setError(message);
    }
  }

  async function dismissProposal(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await dismissMutation.mutateAsync(id);
      setSuccess("ArkSentinel proposal dismissed.");
    } catch (dismissError) {
      setError(errMessage(dismissError));
    }
  }

  async function snoozeProposal(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await snoozeMutation.mutateAsync(id);
      setSuccess("ArkSentinel proposal snoozed for 6 hours.");
    } catch (snoozeError) {
      setError(errMessage(snoozeError));
    }
  }

  return (
    <>
      <WorkspacePageShell spacing={1.5}>
        <WorkspacePageHeader
          eyebrow="Ark Core"
          title="ArkSentinel"
          description="ArkSentinel reviews activity in your workspace and connected apps, spots follow-ups or routine work, and either suggests the next step or handles it based on your settings."
          actions={
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
                alignItems: "flex-start"
              }}>
              <Chip
                color={form.autonomy_mode === "auto" ? "success" : form.autonomy_mode === "assist" ? "info" : "default"}
                label={currentModeLabel}
              />
              <Chip label={openProposals.length > 0 ? `${openProposals.length} waiting` : "Nothing waiting"} />
              <Chip label={`Checked ${lastScanLabel}`} />
            </Stack>
          }
        />

        {error ? <Alert severity="error">{error}</Alert> : null}
        {success ? <Alert severity="success">{success}</Alert> : null}
        {settingsQ.error || feedQ.error ? (
          <Alert severity="error">{errMessage(settingsQ.error || feedQ.error)}</Alert>
        ) : null}

        <Box className="list-shell stat-strip">
          {heroStats.map((item) => (
            <div key={item.label} className="stat-strip-item">
              <span className="stat-strip-label">{item.label}</span>
              <span className="stat-strip-value">{item.value}</span>
              <span className="stat-strip-helper">{item.helper}</span>
            </div>
          ))}
        </Box>

        <Box className="list-shell" sx={{ p: 0.75 }}>
          <Tabs
            value={sentinelTab}
            onChange={(_, next) => setSentinelTab(next as "overview" | "settings")}
            className="workspace-page-subnav-tabs"
          >
            <Tab value="overview" label="Overview" />
            <Tab value="settings" label="Settings" />
          </Tabs>
        </Box>

        {sentinelTab === "overview" ? (
        <Stack spacing={1.5}>
          <Box className="list-shell">
            <Stack spacing={1}>
              <Typography variant="h6">{sentinelHeroHeadline}</Typography>
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                {sentinelHeroDetail}
              </Typography>

              {str(scan?.last_error, "").trim() ? (
                <Alert severity="warning">{str(scan?.last_error, "")}</Alert>
              ) : null}
            </Stack>
          </Box>

          <Box className="list-shell">
            <Stack spacing={1}>
              <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center" }}>
                <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                  <Typography variant="h6">Needs your attention</Typography>
                  <Chip size="small" variant="outlined" label={`${openProposals.length} total`} />
                </Stack>
                <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                  {proposalPageCount > 1 ? (
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Page {proposalPage + 1} of {proposalPageCount}
                    </Typography>
                  ) : null}
                  {feedQ.isLoading ? <CircularProgress size={18} /> : null}
                </Stack>
              </Stack>
                {openProposals.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    No suggestions waiting.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {pagedOpenProposals.map((proposal) => {
                      const selected = selectedProposal?.id === proposal.id;
                      return (
                        <ButtonBase
                          key={proposal.id}
                          onClick={() => setSelectedProposalId(proposal.id)}
                          sx={{
                            width: "100%",
                            textAlign: "left",
                            borderRadius: "8px",
                            border: selected ? "1px solid rgba(168,130,255,0.45)" : "1px solid rgba(255,255,255,0.08)",
                            background: selected ? "rgba(168,130,255,0.08)" : "rgba(255,255,255,0.02)",
                            justifyContent: "flex-start",
                            alignItems: "stretch",
                            px: 1.1,
                            py: 0.95,
                          }}
                        >
                          <Stack spacing={0.75} sx={{ width: "100%", minWidth: 0 }}>
                            <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", alignItems: { sm: "center" } }}>
                              <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap", alignItems: "center", minWidth: 0 }}>
                                <Typography variant="subtitle2">{proposal.title}</Typography>
                                <Chip size="small" color={proposalTone(proposal.status)} label={proposalStatusLabel(proposal.status)} />
                                <Chip size="small" variant="outlined" label={proposal.source_label || sourceKindLabel(proposal.source_kind)} />
                              </Stack>
                              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                Updated {humanTs(proposal.updated_at).label}
                              </Typography>
                            </Stack>
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              {compactText(proposal.detail, 150)}
                            </Typography>
                          </Stack>
                        </ButtonBase>
                      );
                    })}
                    {selectedProposal ? (
                      <Box className="action-row">
                        <Stack spacing={0.9}>
                          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between" }}>
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                                alignItems: "center"
                              }}>
                              <Typography variant="subtitle2">{selectedProposal.title}</Typography>
                              <Chip size="small" color={proposalTone(selectedProposal.status)} label={proposalStatusLabel(selectedProposal.status)} />
                              <Chip size="small" variant="outlined" label={selectedProposal.source_label || sourceKindLabel(selectedProposal.source_kind)} />
                            </Stack>
                            <Stack direction="row" spacing={1}>
                              <Button
                                size="small"
                                variant="contained"
                                onClick={() => void runProposal(selectedProposal)}
                                disabled={approveMutation.isPending}
                              >
                                {proposalActionLabel(selectedProposal)}
                              </Button>
                              <Button size="small" variant="outlined" onClick={() => void snoozeProposal(selectedProposal.id)}>
                                Snooze
                              </Button>
                              <Button size="small" onClick={() => void dismissProposal(selectedProposal.id)}>
                                Dismiss
                              </Button>
                            </Stack>
                          </Stack>
                          <Typography variant="body2">{selectedProposal.detail}</Typography>
                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                            Why now: {selectedProposal.rationale}
                          </Typography>
                          <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                            {selectedProposal.snoozed_until ? (
                              <Chip size="small" variant="outlined" label={`Later until ${humanTs(selectedProposal.snoozed_until).label}`} />
                            ) : null}
                            <Chip size="small" variant="outlined" label={`Updated ${humanTs(selectedProposal.updated_at).label}`} />
                          </Stack>
                        </Stack>
                      </Box>
                    ) : null}
                    {proposalPageCount > 1 ? (
                      <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", pt: 0.25 }}>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          Showing {pagedOpenProposals.length} of {openProposals.length}
                        </Typography>
                        <Stack direction="row" spacing={1}>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={proposalPage === 0}
                            onClick={() => setProposalPage((current) => Math.max(0, current - 1))}
                          >
                            Prev
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={proposalPage >= proposalPageCount - 1}
                            onClick={() =>
                              setProposalPage((current) => Math.min(proposalPageCount - 1, current + 1))
                            }
                          >
                            Next
                          </Button>
                        </Stack>
                      </Stack>
                    ) : null}
                  </Stack>
                )}
              </Stack>
            </Box>

            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center" }}>
                  <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                    <Typography variant="h6">Recent signals</Typography>
                    <Chip size="small" variant="outlined" label={`${recentObservations.length} total`} />
                  </Stack>
                  {observationPageCount > 1 ? (
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Page {observationPage + 1} of {observationPageCount}
                    </Typography>
                  ) : null}
                </Stack>
                {recentObservations.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    No recent signals saved.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {pagedRecentObservations.map((observation: SentinelObservation) => {
                      const selected = selectedObservation?.id === observation.id;
                      return (
                        <ButtonBase
                          key={observation.id}
                          onClick={() => setSelectedObservationId(observation.id)}
                          sx={{
                            width: "100%",
                            textAlign: "left",
                            borderRadius: "8px",
                            border: selected ? "1px solid rgba(59,130,246,0.4)" : "1px solid rgba(255,255,255,0.08)",
                            background: selected ? "rgba(59,130,246,0.08)" : "rgba(255,255,255,0.02)",
                            justifyContent: "flex-start",
                            alignItems: "stretch",
                            px: 1.1,
                            py: 0.95,
                          }}
                        >
                          <Stack spacing={0.75} sx={{ width: "100%", minWidth: 0 }}>
                            <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", alignItems: { sm: "center" } }}>
                              <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap", alignItems: "center", minWidth: 0 }}>
                                <Typography variant="subtitle2">{observation.title}</Typography>
                                <Chip size="small" variant="outlined" label={observationKindLabel(observation.kind)} />
                                <Chip size="small" variant="outlined" label={priorityLabel(observation.priority)} />
                              </Stack>
                              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                Updated {humanTs(observation.updated_at).label}
                              </Typography>
                            </Stack>
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              {compactText(observation.detail, 150)}
                            </Typography>
                          </Stack>
                        </ButtonBase>
                      );
                    })}
                    {selectedObservation ? (
                      <Box className="action-row">
                        <Stack spacing={0.75}>
                          <Stack
                            direction="row"
                            spacing={1}
                            useFlexGap
                            sx={{
                              flexWrap: "wrap",
                              alignItems: "center"
                            }}>
                            <Typography variant="subtitle2">{selectedObservation.title}</Typography>
                            <Chip size="small" variant="outlined" label={observationKindLabel(selectedObservation.kind)} />
                            <Chip size="small" variant="outlined" label={priorityLabel(selectedObservation.priority)} />
                          </Stack>
                          <Typography variant="body2">{selectedObservation.detail}</Typography>
                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                            {selectedObservation.source_label || sourceKindLabel(selectedObservation.source_kind)} | Updated {humanTs(selectedObservation.updated_at).label}
                          </Typography>
                        </Stack>
                      </Box>
                    ) : null}
                    {observationPageCount > 1 ? (
                      <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", pt: 0.25 }}>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          Showing {pagedRecentObservations.length} of {recentObservations.length}
                        </Typography>
                        <Stack direction="row" spacing={1}>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={observationPage === 0}
                            onClick={() => setObservationPage((current) => Math.max(0, current - 1))}
                          >
                            Prev
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={observationPage >= observationPageCount - 1}
                            onClick={() =>
                              setObservationPage((current) => Math.min(observationPageCount - 1, current + 1))
                            }
                          >
                            Next
                          </Button>
                        </Stack>
                      </Stack>
                    ) : null}
                  </Stack>
                )}
              </Stack>
            </Box>
        </Stack>
        ) : null}

        {sentinelTab === "settings" ? (
        <Stack spacing={1.5}>
          <Box className="list-shell">
            <Stack spacing={1.25}>
              <Stack spacing={0.35}>
                <Typography variant="h6">Choose how hands-on ArkSentinel should be</Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Pick the level of help you want. You can change this any time.
                </Typography>
              </Stack>
              <Box
                role="radiogroup"
                aria-label="ArkSentinel autonomy mode"
                sx={{
                  display: "grid",
                  gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
                  gap: 1.25
                }}
              >
                {(["off", "assist", "auto"] as const).map((mode) => {
                  const selected = form.autonomy_mode === mode;
                  const accent = modeChoiceAccent(mode);
                  return (
                    <ButtonBase
                      key={mode}
                      role="radio"
                      aria-checked={selected}
                      onClick={() => setForm((current) => ({ ...current, autonomy_mode: mode }))}
                      sx={{
                        width: "100%",
                        textAlign: "left",
                        borderRadius: "8px",
                        minHeight: 152,
                        border: selected ? `1px solid ${accent}` : "1px solid rgba(255,255,255,0.08)",
                        background: selected
                          ? `linear-gradient(180deg, ${accent.replace(/0\.\d+\)$/, "0.16)")}, rgba(255,255,255,0.03))`
                          : "rgba(255,255,255,0.02)",
                        boxShadow: selected
                          ? `0 0 0 1px rgba(255,255,255,0.03) inset, 0 18px 40px ${accent.replace(/0\.\d+\)$/, "0.12)")}`
                          : "0 0 0 1px rgba(255,255,255,0.02) inset",
                        px: 1.35,
                        py: 1.25,
                        alignItems: "stretch",
                        justifyContent: "flex-start",
                        transition: "border-color 180ms ease, background 180ms ease, box-shadow 180ms ease, transform 180ms ease",
                        "&:hover": {
                          borderColor: accent,
                          background: `linear-gradient(180deg, ${accent.replace(/0\.\d+\)$/, "0.10)")}, rgba(255,255,255,0.03))`,
                          transform: "translateY(-1px)"
                        },
                        "&:focus-visible": {
                          outline: "2px solid rgba(196,223,255,0.85)",
                          outlineOffset: 2
                        }
                      }}
                    >
                      <Stack spacing={1.1} sx={{ width: "100%", minWidth: 0 }}>
                        <Stack
                          direction="row"
                          spacing={1}
                          useFlexGap
                          sx={{ alignItems: "flex-start", justifyContent: "space-between" }}
                        >
                          <Stack spacing={0.6} sx={{ minWidth: 0, pr: 1 }}>
                            <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                              <Box
                                sx={{
                                  display: "inline-flex",
                                  alignItems: "center",
                                  justifyContent: "center",
                                  minHeight: 22,
                                  px: 0.9,
                                  borderRadius: "999px",
                                  background: selected ? accent.replace(/0\.\d+\)$/, "0.18)") : "rgba(255,255,255,0.05)",
                                  color: selected ? "rgba(245,247,250,0.96)" : "rgba(188,198,212,0.8)",
                                  fontSize: "0.68rem",
                                  fontWeight: 700,
                                  letterSpacing: 0,
                                  textTransform: "uppercase"
                                }}
                              >
                                {modeChoiceEyebrow(mode)}
                              </Box>
                              {mode === "assist" ? (
                                <Chip
                                  size="small"
                                  label="Recommended"
                                  sx={{
                                    height: 22,
                                    borderRadius: "999px",
                                    background: "rgba(255,255,255,0.06)",
                                    color: "rgba(220,228,239,0.88)",
                                    border: "1px solid rgba(255,255,255,0.08)"
                                  }}
                                />
                              ) : null}
                            </Stack>
                            <Typography variant="subtitle1" sx={{ fontWeight: 700, lineHeight: 1.2 }}>
                              {modeLabel(mode)}
                            </Typography>
                          </Stack>
                          <Box
                            sx={{
                              width: 22,
                              height: 22,
                              flexShrink: 0,
                              borderRadius: "50%",
                              border: selected ? `1px solid ${accent}` : "1px solid rgba(255,255,255,0.18)",
                              background: selected ? accent.replace(/0\.\d+\)$/, "0.18)") : "rgba(255,255,255,0.02)",
                              display: "flex",
                              alignItems: "center",
                              justifyContent: "center",
                              mt: 0.1
                            }}
                          >
                            <Box
                              sx={{
                                width: 10,
                                height: 10,
                                borderRadius: "50%",
                                background: selected ? accent : "transparent",
                                boxShadow: selected ? `0 0 10px ${accent.replace(/0\.\d+\)$/, "0.45)")}` : "none"
                              }}
                            />
                          </Box>
                        </Stack>
                        <Typography variant="body2" sx={{ color: selected ? "rgba(236,240,246,0.92)" : "text.secondary", lineHeight: 1.55 }}>
                          {modeChoiceDescription(mode)}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: selected ? "rgba(210,220,232,0.82)" : "rgba(188,198,212,0.7)",
                            lineHeight: 1.45
                          }}
                        >
                          {modeChoiceSupport(mode)}
                        </Typography>
                      </Stack>
                    </ButtonBase>
                  );
                })}
              </Box>
            </Stack>
          </Box>

          <Box className="list-shell">
            <Stack spacing={1.25}>
              <Stack spacing={0.35}>
                <Typography variant="h6">Watch for these signals</Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  These switches decide where ArkSentinel learns from and what kinds of follow-up it can suggest.
                </Typography>
              </Stack>
              <Stack spacing={0}>
                {SENTINEL_SIGNAL_OPTIONS.map((item, index) => (
                  <Box
                    key={item.key}
                    sx={{
                      display: "grid",
                      gridTemplateColumns: "minmax(0, 1fr) auto",
                      gap: 1,
                      alignItems: "center",
                      py: 1,
                      borderTop: index === 0 ? "none" : "1px solid rgba(255,255,255,0.06)"
                    }}
                  >
                    <Stack spacing={0.25} sx={{ minWidth: 0 }}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>{item.label}</Typography>
                      <Typography variant="caption" sx={{ color: "text.secondary", lineHeight: 1.45 }}>{item.description}</Typography>
                    </Stack>
                    <Switch
                      checked={Boolean(form[item.key])}
                      onChange={(event) => setForm((current) => ({ ...current, [item.key]: event.target.checked }))}
                    />
                  </Box>
                ))}
              </Stack>
              <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                <Chip size="small" variant="outlined" label={`Daily help limit ${settingsQ.data?.daily_run_limit ?? 40}`} />
                <Chip size="small" variant="outlined" label={`${stats?.connected_services ?? 0} connected app${(stats?.connected_services ?? 0) === 1 ? "" : "s"}`} />
                {settingsQ.data?.quiet_hours_start || settingsQ.data?.quiet_hours_end ? (
                  <Chip size="small" variant="outlined" label={`Quiet hours ${settingsQ.data?.quiet_hours_start || "--:--"} - ${settingsQ.data?.quiet_hours_end || "--:--"}`} />
                ) : null}
              </Stack>
              <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                <Button variant="contained" disabled={!dirty || saveMutation.isPending} onClick={saveSettings}>
                  {saveMutation.isPending ? "Saving..." : "Save"}
                </Button>
                <Button
                  variant="outlined"
                  disabled={!dirty}
                  onClick={() =>
                    settingsQ.data &&
                    setForm({
                      enabled: Boolean(settingsQ.data.settings.enabled ?? true),
                      watch_in_app: Boolean(settingsQ.data.settings.watch_in_app ?? true),
                      watch_connected_services: Boolean(settingsQ.data.settings.watch_connected_services ?? true),
                      infer_new_automations: Boolean(settingsQ.data.settings.infer_new_automations ?? true),
                      confidence_threshold: String(num(settingsQ.data.settings.confidence_threshold, 0.72)),
                      max_proposals_per_scan: String(num(settingsQ.data.settings.max_proposals_per_scan, 6)),
                      autonomy_mode: str(settingsQ.data.autonomy_mode, "assist").toLowerCase() === "off" ? "off" : str(settingsQ.data.autonomy_mode, "assist").toLowerCase() === "auto" ? "auto" : "assist",
                    })
                  }
                >
                  Reset
                </Button>
              </Stack>
            </Stack>
          </Box>
        </Stack>
        ) : null}
      </WorkspacePageShell>
      <SuggestionRunDialog
        run={run}
        open={runOpen}
        minimized={runMinimized}
        trace={asRecord(runTraceQ.data)}
        traceSteps={pickRecords(runTraceQ.data, "steps")}
        traceLoading={runTraceQ.isLoading}
        traceError={runTraceQ.error}
        detailError={null}
        acceptedOutcomes={[]}
        onClose={() => {
          setRunOpen(false);
          setRunMinimized(false);
          setRun(null);
        }}
        onMinimize={() => setRunMinimized(true)}
        onRestore={() => setRunMinimized(false)}
        onOpenWorkspacePanel={(view) => navigateToView(view)}
        getConsoleView={buildTraceConsoleView}
        getTraceStepColor={traceStepColor}
        humanTs={humanTs}
        errMessage={errMessage}
      />
    </>
  );
}
