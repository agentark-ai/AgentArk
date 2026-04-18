import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Divider,
  Stack,
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

function proposalDotColor(status: string): string {
  const tone = proposalTone(status);
  if (tone === "success") return "rgba(74,210,157,0.85)";
  if (tone === "warning" || tone === "info") return "rgba(57,208,255,0.85)";
  if (tone === "error") return "rgba(255,100,100,0.85)";
  return "rgba(180,200,220,0.5)";
}

function observationDotColor(priority: number): string {
  if (priority <= 1) return "rgba(255,100,100,0.85)";
  if (priority === 2) return "rgba(57,208,255,0.85)";
  return "rgba(74,210,157,0.85)";
}

function proposalActionLabel(proposal: SentinelProposal): string {
  return proposal.proposal_kind === "chat_suggestion_accept" ? "Launch" : "Run";
}

function modeLabel(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "Off";
  if (mode === "auto") return "Auto";
  return "Suggest first";
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
  const configuredMode = str(settingsQ.data?.autonomy_mode, "assist").toLowerCase();
  const currentAutonomyMode: "off" | "assist" | "auto" =
    configuredMode === "off" || configuredMode === "auto" ? configuredMode : "assist";
  const autonomyDisabled = Boolean(settingsQ.data?.agent_paused) || currentAutonomyMode === "off";
  const lastScanLabel = scan?.last_completed_at ? humanTs(scan.last_completed_at).label : "Waiting for the first check";
  const currentModeLabel = settingsQ.data?.agent_paused ? "Paused" : modeLabel(currentAutonomyMode);
  const sentinelHeroHeadline =
    settingsQ.data?.agent_paused
      ? "ArkSentinel is paused."
      : currentAutonomyMode === "off"
        ? "ArkSentinel is turned off."
        : openProposals.length > 0
          ? `${openProposals.length} follow-up${openProposals.length === 1 ? "" : "s"} waiting for you.`
          : recentObservations.length > 0
            ? "No action needed right now."
            : "No follow-ups right now.";
  const sentinelHeroDetail =
    settingsQ.data?.agent_paused
      ? "Turn autonomy back on to resume background checks, suggestions, and learning."
      : currentAutonomyMode === "off"
        ? "ArkSentinel is not scanning for follow-ups while this mode is off."
        : openProposals.length > 0
          ? "Review the suggested next steps below or leave them for later."
          : currentAutonomyMode === "auto"
            ? "ArkSentinel is scanning in the background and can handle lightweight routine work automatically."
            : "ArkSentinel is scanning in the background and will ask before it acts.";
  const heroTone =
    settingsQ.data?.agent_paused || currentAutonomyMode === "off"
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
            border: "rgba(255, 255, 255, 0.12)",
            background: "linear-gradient(135deg, rgba(24, 24, 28, 0.96), rgba(15, 17, 21, 0.96))"
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

  function openAdvancedSettings() {
    const nextUrl = "/ui/settings?settings_tab=advanced";
    const currentUrl = `${window.location.pathname}${window.location.search}`;
    if (currentUrl === nextUrl) return;
    window.history.pushState(null, "", nextUrl);
    window.dispatchEvent(new PopStateEvent("popstate"));
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
          eyebrow="Ark Autonomy"
          title="ArkSentinel"
          description="Unfinished work, repeated routines, pending follow-ups, and suggested next actions."
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
                color={autonomyDisabled ? "warning" : currentAutonomyMode === "auto" ? "success" : "info"}
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
                            px: 0,
                            py: 1.15,
                            borderBottom: "1px solid",
                            borderColor: "divider",
                            transition: "background 0.15s ease",
                            "&:hover": { background: "rgba(57, 208, 255, 0.04)" },
                            ...(selected && { background: "rgba(57, 208, 255, 0.06)" }),
                          }}
                        >
                          <Stack sx={{ width: "100%", minWidth: 0 }}>
                            <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center" }}>
                              <Stack direction="row" spacing={1} sx={{ alignItems: "center", minWidth: 0 }}>
                                <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: proposalDotColor(proposal.status) }} />
                                <Typography variant="body2" sx={{ fontWeight: 600 }}>{proposal.title}</Typography>
                              </Stack>
                              <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0, ml: 1 }}>
                                {humanTs(proposal.updated_at).label}
                              </Typography>
                            </Stack>
                            <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                              {compactText(proposal.detail, 150)}
                            </Typography>
                          </Stack>
                        </ButtonBase>
                      );
                    })}
                    {selectedProposal ? (
                      <Box className="metadata-box">
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
                            px: 0,
                            py: 1.15,
                            borderBottom: "1px solid",
                            borderColor: "divider",
                            transition: "background 0.15s ease",
                            "&:hover": { background: "rgba(57, 208, 255, 0.04)" },
                            ...(selected && { background: "rgba(57, 208, 255, 0.06)" }),
                          }}
                        >
                          <Stack sx={{ width: "100%", minWidth: 0 }}>
                            <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center" }}>
                              <Stack direction="row" spacing={1} sx={{ alignItems: "center", minWidth: 0 }}>
                                <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: observationDotColor(observation.priority) }} />
                                <Typography variant="body2" sx={{ fontWeight: 600 }}>{observation.title}</Typography>
                              </Stack>
                              <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0, ml: 1 }}>
                                {humanTs(observation.updated_at).label}
                              </Typography>
                            </Stack>
                            <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                              {compactText(observation.detail, 150)}
                            </Typography>
                          </Stack>
                        </ButtonBase>
                      );
                    })}
                    {selectedObservation ? (
                      <Box className="metadata-box">
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
