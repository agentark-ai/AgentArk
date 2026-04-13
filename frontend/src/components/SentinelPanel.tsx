import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  FormControlLabel,
  Stack,
  Switch,
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
  SentinelBackgroundLearningJob,
  SentinelFeedResponse,
  SentinelObservation,
  SentinelProposal,
  TraceSummary,
} from "../types";

const REFRESH_MS = 8000;

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

function modeSummary(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "Sentinel is off and will not prepare or run background work.";
  if (mode === "auto") return "Sentinel can quietly handle safe background work for you.";
  return "Sentinel suggests useful actions first and waits for your approval.";
}

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

const BACKGROUND_LEARNING_JOB_ORDER = [
  { key: "reflection_pass", label: "Reflection pass" },
  { key: "experience_consolidation", label: "Experience consolidation" },
  { key: "pattern_induction", label: "Pattern induction" },
  { key: "candidate_generation", label: "Candidate generation" },
] as const;

type BackgroundLearningJobKey = (typeof BACKGROUND_LEARNING_JOB_ORDER)[number]["key"];

function backgroundTone(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = status.trim().toLowerCase();
  if (["completed", "updated", "changed", "ok", "success"].includes(normalized)) return "success";
  if (["running", "queued", "working"].includes(normalized)) return "info";
  if (["paused", "disabled", "waiting"].includes(normalized)) return "warning";
  if (["failed", "error"].includes(normalized)) return "error";
  return "default";
}

function formatBackgroundSummary(job: SentinelBackgroundLearningJob | undefined): string {
  const summary = str(job?.summary, "").trim();
  if (summary) return summary;
  const status = str(job?.status, "").trim().toLowerCase();
  if (status === "disabled") return "Disabled until autonomy is re-enabled.";
  if (status === "paused") return "Paused with the rest of autonomy.";
  if (status === "running") return "Running background learning pass...";
  if (status === "completed") return "Completed with no additional detail.";
  if (status === "error") return "Last pass failed.";
  return "Waiting for the next background pass.";
}

function humanizeBackgroundKey(key: string): string {
  return key
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function backgroundJobStats(job: SentinelBackgroundLearningJob | undefined): Array<[string, unknown]> {
  const stats = job?.stats;
  if (!stats || typeof stats !== "object" || Array.isArray(stats)) return [];
  return Object.entries(stats)
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .slice(0, 4);
}

function backgroundJobLabel(key: BackgroundLearningJobKey): string {
  const match = BACKGROUND_LEARNING_JOB_ORDER.find((item) => item.key === key);
  return match ? match.label : humanizeBackgroundKey(key);
}

export function SentinelPanel({
  autoRefresh,
  navigateToView,
}: {
  autoRefresh: boolean;
  navigateToView: (view: string, replace?: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<SentinelFormState>(blankForm);
  const [hydrated, setHydrated] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [run, setRun] = useState<SuggestionRunState | null>(null);
  const [runOpen, setRunOpen] = useState(false);
  const [runMinimized, setRunMinimized] = useState(false);

  const settingsQ = useQuery({
    queryKey: ["sentinel-settings"],
    queryFn: api.getSentinelSettings,
  });

  const feedQ = useQuery({
    queryKey: ["sentinel-feed"],
    queryFn: api.getSentinelFeed,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const traceQ = useQuery({
    queryKey: ["trace"],
    queryFn: api.getTrace,
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
  const traces = useMemo<TraceSummary[]>(() => traceQ.data?.history || [], [traceQ.data]);
  const recentSentinelRuns = useMemo(
    () =>
      traces.filter((trace) => {
        const channel = String(trace.channel || "").toLowerCase();
        return channel === "sentinel" || channel === "autonomy";
      }),
    [traces]
  );
  const openProposals = useMemo(
    () =>
      (feed?.proposals || []).filter((proposal) =>
        ["open", "running", "queued_for_approval", "snoozed"].includes(String(proposal.status || "").toLowerCase())
      ),
    [feed?.proposals]
  );
  const recentObservations = useMemo(() => (feed?.observations || []).slice(0, 12), [feed?.observations]);
  const backgroundLearning: SentinelBackgroundLearning | null = feed?.background_learning || null;
  const backgroundLearningJobs = useMemo(
    () =>
      BACKGROUND_LEARNING_JOB_ORDER.map((entry) => {
        const job = backgroundLearning?.jobs?.[entry.key];
        return {
          key: entry.key,
          label: entry.label,
          job,
        };
      }),
    [backgroundLearning]
  );
  const scan = feed?.scan;
  const stats = feed?.stats;
  const autonomyDisabled = Boolean(settingsQ.data?.agent_paused) || str(settingsQ.data?.autonomy_mode, "assist").toLowerCase() === "off";

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
      setSuccess("Sentinel settings saved.");
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
      summary: "Launching Sentinel proposal...",
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
        summary: str(response.message, "Sentinel proposal accepted."),
        traceId: traceId || undefined,
        startedAt: new Date().toISOString(),
        completedAt: runStatus === "queued_for_approval" || !traceId ? new Date().toISOString() : undefined,
        suggestionId: proposal.id,
      });
      setSuccess("Sentinel proposal accepted.");
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
      setSuccess("Sentinel proposal dismissed.");
    } catch (dismissError) {
      setError(errMessage(dismissError));
    }
  }

  async function snoozeProposal(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await snoozeMutation.mutateAsync(id);
      setSuccess("Sentinel proposal snoozed for 6 hours.");
    } catch (snoozeError) {
      setError(errMessage(snoozeError));
    }
  }

  return (
    <>
      <WorkspacePageShell spacing={1.5}>
        <WorkspacePageHeader
          eyebrow="Ambient engine"
          title="Sentinel"
          description="Sentinel watches activity in the background, suggests helpful next steps, and can handle lightweight routine help automatically when you choose Auto."
          actions={
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
                alignItems: "flex-start"
              }}>
              <Chip label={`${stats?.open_proposals ?? 0} open proposals`} />
              <Chip label={`${stats?.connected_services ?? 0} connected services`} />
              <Chip label={`${stats?.recent_runs ?? 0} recent runs`} />
              <Button variant="outlined" onClick={() => navigateToView("trace")}>
                Open Trace
              </Button>
            </Stack>
          }
        />

        {error ? <Alert severity="error">{error}</Alert> : null}
        {success ? <Alert severity="success">{success}</Alert> : null}
        {settingsQ.error || feedQ.error || traceQ.error ? (
          <Alert severity="error">{errMessage(settingsQ.error || feedQ.error || traceQ.error)}</Alert>
        ) : null}

        <Stack direction={{ xs: "column", xl: "row" }} spacing={1.5} sx={{
          alignItems: "stretch"
        }}>
          <Stack spacing={1.5} sx={{ flex: 1.1 }}>
            <Box className="list-shell">
              <Stack spacing={1.25}>
                <Stack
                  direction="row"
                  spacing={1}
                  useFlexGap
                  sx={{
                    flexWrap: "wrap",
                    alignItems: "center"
                  }}>
                  <Typography variant="h6">How Sentinel should help</Typography>
                  <Chip
                    size="small"
                    color={form.autonomy_mode === "auto" ? "success" : form.autonomy_mode === "assist" ? "warning" : "default"}
                    label={form.autonomy_mode === "auto" ? "Auto" : form.autonomy_mode === "assist" ? "Assist" : "Off"}
                  />
                  {settingsQ.data?.agent_paused ? <Chip size="small" color="warning" label="Agent paused" /> : null}
                </Stack>
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  {modeSummary(form.autonomy_mode)}
                </Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  {(["off", "assist", "auto"] as const).map((mode) => (
                    <Button
                      key={mode}
                      variant={form.autonomy_mode === mode ? "contained" : "outlined"}
                      onClick={() => setForm((current) => ({ ...current, autonomy_mode: mode }))}
                    >
                      {mode === "off" ? "Off" : mode === "assist" ? "Suggest first" : "Auto-run"}
                    </Button>
                  ))}
                </Stack>
                <Divider />
                <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
                  <Stack spacing={0.75} sx={{ flex: 1 }}>
                    <FormControlLabel
                      control={<Switch checked={form.enabled} onChange={(event) => setForm((current) => ({ ...current, enabled: event.target.checked }))} />}
                      label="Keep Sentinel available"
                    />
                    <FormControlLabel
                      control={<Switch checked={form.watch_in_app} onChange={(event) => setForm((current) => ({ ...current, watch_in_app: event.target.checked }))} />}
                      label="Watch activity inside AgentArk"
                    />
                    <FormControlLabel
                      control={<Switch checked={form.watch_connected_services} onChange={(event) => setForm((current) => ({ ...current, watch_connected_services: event.target.checked }))} />}
                      label="Watch connected services"
                    />
                    <FormControlLabel
                      control={<Switch checked={form.infer_new_automations} onChange={(event) => setForm((current) => ({ ...current, infer_new_automations: event.target.checked }))} />}
                      label="Suggest recurring automations"
                    />
                  </Stack>
                  <Stack spacing={1} sx={{ flex: 1 }}>
                    <Alert severity="info">
                      Advanced scoring and proposal limits use the built-in defaults. This page keeps the main setup simple.
                    </Alert>
                    <Stack direction="row" spacing={1} useFlexGap sx={{
                      flexWrap: "wrap"
                    }}>
                      <Chip size="small" variant="outlined" label={`Daily run limit ${settingsQ.data?.daily_run_limit ?? 40}`} />
                      <Chip size="small" variant="outlined" label={`${stats?.connected_services ?? 0} connected services`} />
                      {settingsQ.data?.quiet_hours_start || settingsQ.data?.quiet_hours_end ? (
                        <Chip
                          size="small"
                          variant="outlined"
                          label={`Quiet hours ${settingsQ.data?.quiet_hours_start || "--:--"} - ${settingsQ.data?.quiet_hours_end || "--:--"}`}
                        />
                      ) : null}
                    </Stack>
                  </Stack>
                </Stack>
                <Stack direction="row" spacing={1}>
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

            <Box className="list-shell">
              <Stack spacing={1}>
                <Typography variant="h6">Current status</Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  <Chip size="small" label={`${scan?.open_proposals ?? 0} open`} />
                  <Chip size="small" label={`${scan?.last_auto_executed ?? 0} auto-ran`} />
                  <Chip size="small" label={str(scan?.last_status, "idle")} />
                </Stack>
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  Last background pass: {scan?.last_completed_at ? humanTs(scan.last_completed_at).label : "waiting for the first run"}
                </Typography>
                {str(scan?.last_error, "").trim() ? (
                  <Alert severity="warning">{str(scan?.last_error, "")}</Alert>
                ) : null}
              </Stack>
            </Box>

            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Typography variant="h6">Background learning</Typography>
                  <Stack
                    direction="row"
                    spacing={0.75}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                      justifyContent: "flex-end"
                    }}>
                    <Chip
                      size="small"
                      color={backgroundTone(str(backgroundLearning?.status, autonomyDisabled ? "disabled" : "idle"))}
                      label={str(backgroundLearning?.status, autonomyDisabled ? "disabled" : "idle")}
                    />
                    {typeof backgroundLearning?.changed === "boolean" ? (
                      <Chip
                        size="small"
                        variant="outlined"
                        label={backgroundLearning.changed ? "Changed" : "No change"}
                      />
                    ) : null}
                  </Stack>
                </Stack>
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  {str(backgroundLearning?.summary, "").trim()
                    ? str(backgroundLearning?.summary, "")
                    : autonomyDisabled
                      ? "Background learning is paused until autonomy is re-enabled."
                      : "Sentinel quietly reviews recent activity and improves memory and reuse in the background."}
                </Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      backgroundLearning?.last_completed_at
                        ? `Last run ${humanTs(backgroundLearning.last_completed_at).label}`
                        : "Last run -"
                    }
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      backgroundLearning?.last_started_at
                        ? `Started ${humanTs(backgroundLearning.last_started_at).label}`
                        : "Started -"
                    }
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={autonomyDisabled ? "Autonomy disabled" : "Auto background learning enabled"}
                  />
                </Stack>
                <Divider />
                <Stack spacing={1} sx={{ display: "none" }}>
                  {backgroundLearningJobs.map(({ key, label, job }) => {
                    const status = str(job?.status, autonomyDisabled ? "disabled" : "idle");
                    const statsEntries = backgroundJobStats(job);
                    return (
                      <Box key={key} className="action-row">
                        <Stack spacing={0.85}>
                          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{
                            justifyContent: "space-between"
                          }}>
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                                alignItems: "center"
                              }}>
                              <Typography variant="subtitle2">{label}</Typography>
                              <Chip size="small" color={backgroundTone(status)} label={status} />
                              {typeof job?.changed === "boolean" ? (
                                <Chip size="small" variant="outlined" label={job.changed ? "Changed" : "No change"} />
                              ) : null}
                              {typeof job?.runs === "number" ? (
                                <Chip size="small" variant="outlined" label={`Runs ${job.runs}`} />
                              ) : null}
                            </Stack>
                          </Stack>
                          <Typography variant="body2">{formatBackgroundSummary(job)}</Typography>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {job?.last_started_at ? `Started ${humanTs(job.last_started_at).label}` : "Started -"}
                            {" | "}
                            {job?.last_completed_at ? `Completed ${humanTs(job.last_completed_at).label}` : "Completed -"}
                            {job?.last_error ? ` | Error: ${job.last_error}` : ""}
                          </Typography>
                          {statsEntries.length > 0 ? (
                            <Stack direction="row" spacing={0.75} useFlexGap sx={{
                              flexWrap: "wrap"
                            }}>
                              {statsEntries.map(([statKey, statValue]) => (
                                <Chip
                                  key={`${key}-${statKey}`}
                                  size="small"
                                  variant="outlined"
                                  label={`${humanizeBackgroundKey(statKey)} ${String(statValue)}`}
                                />
                              ))}
                            </Stack>
                          ) : null}
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Reflection, memory cleanup, and pattern learning continue automatically in the background. Detailed internals are hidden here to keep this page simple.
                </Typography>
              </Stack>
            </Box>
          </Stack>

          <Stack spacing={1.5} sx={{ flex: 1.4 }}>
            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Typography variant="h6">Open proposals</Typography>
                  {feedQ.isLoading ? <CircularProgress size={18} /> : null}
                </Stack>
                {openProposals.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    Nothing needs your attention right now.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {openProposals.map((proposal) => (
                      <Box key={proposal.id} className="action-row">
                        <Stack spacing={0.9}>
                          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{
                            justifyContent: "space-between"
                          }}>
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                                alignItems: "center"
                              }}>
                              <Typography variant="subtitle2">{proposal.title}</Typography>
                              <Chip size="small" color={proposalTone(proposal.status)} label={proposal.status} />
                              <Chip size="small" variant="outlined" label={proposal.source_kind} />
                            </Stack>
                            <Stack direction="row" spacing={1}>
                              <Button
                                size="small"
                                variant="contained"
                                onClick={() => void runProposal(proposal)}
                                disabled={approveMutation.isPending}
                              >
                                {proposalActionLabel(proposal)}
                              </Button>
                              <Button size="small" variant="outlined" onClick={() => void snoozeProposal(proposal.id)}>
                                Snooze
                              </Button>
                              <Button size="small" onClick={() => void dismissProposal(proposal.id)}>
                                Dismiss
                              </Button>
                            </Stack>
                          </Stack>
                          <Typography variant="body2">{proposal.detail}</Typography>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {proposal.rationale}
                          </Typography>
                          <Stack direction="row" spacing={1} useFlexGap sx={{
                            flexWrap: "wrap"
                          }}>
                            {proposal.source_label ? <Chip size="small" variant="outlined" label={proposal.source_label} /> : null}
                            {proposal.trace_id ? <Chip size="small" variant="outlined" label={`Trace ${proposal.trace_id}`} /> : null}
                            {proposal.snoozed_until ? (
                              <Chip size="small" variant="outlined" label={`Snoozed until ${humanTs(proposal.snoozed_until).label}`} />
                            ) : null}
                          </Stack>
                        </Stack>
                      </Box>
                    ))}
                  </Stack>
                )}
              </Stack>
            </Box>

            <Box className="list-shell">
              <Stack spacing={1}>
                <Typography variant="h6">Recent observations</Typography>
                {recentObservations.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    No recent background signals were worth surfacing.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {recentObservations.map((observation: SentinelObservation) => (
                      <Box key={observation.id} className="action-row">
                        <Stack spacing={0.75}>
                          <Stack
                            direction="row"
                            spacing={1}
                            useFlexGap
                            sx={{
                              flexWrap: "wrap",
                              alignItems: "center"
                            }}>
                            <Typography variant="subtitle2">{observation.title}</Typography>
                            <Chip size="small" variant="outlined" label={observation.kind} />
                            <Chip size="small" variant="outlined" label={observation.source_kind} />
                            <Chip size="small" variant="outlined" label={`P${observation.priority}`} />
                          </Stack>
                          <Typography variant="body2">{observation.detail}</Typography>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {observation.source_label || "Internal signal"} | {humanTs(observation.updated_at).label}
                          </Typography>
                        </Stack>
                      </Box>
                    ))}
                  </Stack>
                )}
              </Stack>
            </Box>

            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Typography variant="h6">Recent runs</Typography>
                  <Button size="small" onClick={() => navigateToView("trace")}>
                    Open Trace
                  </Button>
                </Stack>
                {recentSentinelRuns.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    No Sentinel-related traces recorded yet.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {recentSentinelRuns.slice(0, 8).map((trace) => (
                      <Box key={trace.id} className="action-row">
                        <Stack spacing={0.6}>
                          <Stack
                            direction="row"
                            spacing={1}
                            useFlexGap
                            sx={{
                              flexWrap: "wrap",
                              alignItems: "center"
                            }}>
                            <Typography variant="subtitle2">{trace.message_preview}</Typography>
                            <Chip size="small" color={proposalTone(trace.status)} label={trace.status} />
                            <Chip size="small" variant="outlined" label={trace.channel} />
                          </Stack>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {humanTs(trace.started_at).label} | {trace.step_count} steps
                          </Typography>
                        </Stack>
                      </Box>
                    ))}
                  </Stack>
                )}
              </Stack>
            </Box>
          </Stack>
        </Stack>
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
