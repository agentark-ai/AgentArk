import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
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
  SentinelFeedResponse,
  SentinelProposal,
} from "../types";

const REFRESH_MS = 8000;
const SENTINEL_SECTION_PAGE_SIZE = 12;
const CHAT_PENDING_LAUNCH_STORAGE_KEY = "agentark.chat.pendingLaunch";

type JsonRecord = Record<string, unknown>;
type SentinelClarificationChoice = { label: string; submitText: string };
type SentinelProposalGroup = {
  key: string;
  proposal: SentinelProposal;
  proposals: SentinelProposal[];
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

function proposalDotColor(status: string): string {
  const tone = proposalTone(status);
  if (tone === "success") return "var(--ui-rgba-74-210-157-850)";
  if (tone === "warning" || tone === "info") return "var(--ui-rgba-57-208-255-850)";
  if (tone === "error") return "var(--ui-rgba-255-100-100-850)";
  return "var(--ui-rgba-180-200-220-500)";
}

function proposalActionLabel(proposal: SentinelProposal): string {
  const actionKind = str(proposal.action?.action_kind, "").trim();
  if (actionKind === "chat_prompt") return "Open in Chat";
  return "Review";
}

function modeLabel(mode: "off" | "assist" | "auto"): string {
  if (mode === "off") return "Off";
  if (mode === "auto") return "Auto";
  return "Review first";
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
  if (normalized === "integration" || normalized === "connected_service" || normalized === "service_event") return "Connected source";
  if (normalized === "chat") return "Chat";
  if (normalized === "observation") return "System signal";
  return humanizeBackgroundKey(normalized);
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

function proposalMetadata(proposal: SentinelProposal): JsonRecord {
  return asRecord(proposal.metadata);
}

function proposalClarificationChoices(proposal: SentinelProposal): SentinelClarificationChoice[] {
  const rawChoices = proposalMetadata(proposal).choices;
  if (!Array.isArray(rawChoices)) return [];
  return rawChoices
    .filter((choice): choice is JsonRecord => !!choice && typeof choice === "object" && !Array.isArray(choice))
    .map((choice) => {
      const label = str(choice.label, "").trim();
      const submitText = str(choice.submit_text, str(choice.submitText, "")).trim();
      if (!label || !submitText) return null;
      return { label, submitText };
    })
    .filter((choice): choice is SentinelClarificationChoice => choice !== null);
}

function proposalConversationId(proposal: SentinelProposal): string {
  return str(proposalMetadata(proposal).conversation_id, "").trim();
}

function proposalChatSuggestionId(proposal: SentinelProposal): string {
  return str(proposal.chat_suggestion_id, str(proposalMetadata(proposal).suggestion_id, "")).trim();
}

function proposalHasRunnableAction(proposal: SentinelProposal): boolean {
  return !!proposal.action && !!str(proposal.action.action_kind, "").trim();
}

function proposalLooksLikeRouterNoise(proposal: SentinelProposal): boolean {
  const metadata = proposalMetadata(proposal);
  const combined = [
    proposal.title,
    proposal.detail,
    proposal.rationale,
    proposal.last_run_summary || "",
    str(metadata.status, ""),
    str(metadata.current_stage, ""),
  ]
    .join("\n")
    .toLowerCase();
  return (
    combined.includes("semantic router") ||
    combined.includes("could not route this request") ||
    combined.includes("router model call failed") ||
    combined.includes("unified semantic router failed")
  );
}

function proposalIsUserActionable(proposal: SentinelProposal): boolean {
  const status = str(proposal.status, "").toLowerCase();
  if (status !== "open" && status !== "queued_for_approval") return false;
  if (
    proposal.proposal_kind === "chat_suggestion_accept" ||
    proposal.source_kind === "chat_suggestion" ||
    proposalChatSuggestionId(proposal)
  ) {
    return false;
  }
  if (proposal.source_kind === "execution_run" && proposalLooksLikeRouterNoise(proposal)) return false;
  const metadata = proposalMetadata(proposal);
  if (proposal.source_kind === "execution_run" && metadata.background_signal !== true) return false;
  const choices = proposalClarificationChoices(proposal);
  if (choices.length > 0) return true;
  if (!proposalHasRunnableAction(proposal)) return false;
  if (proposal.source_kind !== "execution_run") return true;

  if (typeof metadata.user_actionable === "boolean") return metadata.user_actionable;

  const runStatus = str(proposal.run_status, str(metadata.status, "")).toLowerCase();
  if (runStatus === "needs_input") return choices.length > 0;
  return runStatus === "needs_stronger_model";
}

function proposalIntentKey(proposal: SentinelProposal): string {
  const metadata = proposalMetadata(proposal);
  const sourceIdentity =
    str(metadata.run_id, "") ||
    str(proposal.source_id, "") ||
    str(proposal.trace_id, "") ||
    proposalConversationId(proposal) ||
    proposal.fingerprint ||
    proposal.id;
  const choices = proposalClarificationChoices(proposal);
  if (choices.length > 0) {
    return [
      "clarification",
      proposal.proposal_kind,
      proposal.source_kind,
      sourceIdentity.toLowerCase(),
      str(proposal.run_status, proposal.status).toLowerCase(),
      str(proposal.action?.action_kind, "").toLowerCase(),
      String(choices.length),
    ].join(":");
  }
  return [
    "proposal",
    proposal.proposal_kind,
    proposal.source_kind,
    sourceIdentity.toLowerCase(),
    str(proposal.run_status, proposal.status).toLowerCase(),
    proposal.fingerprint,
  ].join(":");
}

function groupSentinelProposals(proposals: SentinelProposal[]): SentinelProposalGroup[] {
  const groups = new Map<string, SentinelProposalGroup>();
  for (const proposal of proposals) {
    const key = proposalIntentKey(proposal);
    const existing = groups.get(key);
    if (existing) {
      existing.proposals.push(proposal);
      continue;
    }
    groups.set(key, { key, proposal, proposals: [proposal] });
  }
  return Array.from(groups.values());
}

function storeChatPendingLaunch(snapshot: {
  createdAt: number;
  launchMode: "message";
  message: string;
  conversationId?: string;
  source?: string;
  acceptedSuggestionId?: string;
  sentinelProposalId?: string;
}): void {
  if (typeof window === "undefined") return;
  window.sessionStorage.setItem(CHAT_PENDING_LAUNCH_STORAGE_KEY, JSON.stringify(snapshot));
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
  const [proposalPage, setProposalPage] = useState(0);

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
  const runSuggestionDetailId = run?.suggestionId || "";
  const runTraceQ = useQuery({
    queryKey: ["sentinel-run-trace", runTraceId],
    queryFn: () => api.rawGet(`/trace/${encodeURIComponent(runTraceId)}`),
    enabled: !!runTraceId && runOpen,
    refetchInterval: runOpen && !!runTraceId && run?.status === "running" ? REFRESH_MS : false,
  });
  const runSuggestionDetailQ = useQuery({
    queryKey: ["sentinel-suggestion-detail", runSuggestionDetailId],
    queryFn: () => api.rawGet(`/autonomy/suggestions/${encodeURIComponent(runSuggestionDetailId)}`),
    enabled: !!runSuggestionDetailId && runOpen,
    refetchInterval: runOpen && !!runSuggestionDetailId && run?.status === "running" ? REFRESH_MS : false,
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

  const runAcceptedOutcomes = useMemo(
    () => pickRecords(asRecord(runSuggestionDetailQ.data).suggestion, "accepted_outcomes"),
    [runSuggestionDetailQ.data]
  );

  useEffect(() => {
    if (!run || runAcceptedOutcomes.length === 0) return;
    const titles = runAcceptedOutcomes
      .map((outcome) => str(outcome.title, "").trim())
      .filter(Boolean);
    const summary = `Saved ${runAcceptedOutcomes.length} outcome${runAcceptedOutcomes.length === 1 ? "" : "s"}${titles.length ? `: ${titles.slice(0, 3).join(", ")}` : "."}`;
    if (run.status === "completed" && run.summary === summary) return;
    setRun((current) =>
      current
        ? {
            ...current,
            status: "completed",
            summary,
            completedAt: current.completedAt || new Date().toISOString(),
          }
        : current
    );
  }, [run, runAcceptedOutcomes]);

  const feed = feedQ.data as SentinelFeedResponse | undefined;
  const openProposals = useMemo(
    () =>
      (feed?.proposals || []).filter((proposal) => proposalIsUserActionable(proposal)),
    [feed?.proposals]
  );
  const openProposalGroups = useMemo(() => groupSentinelProposals(openProposals), [openProposals]);
  const proposalPageCount = Math.max(1, Math.ceil(openProposalGroups.length / SENTINEL_SECTION_PAGE_SIZE));
  const pagedOpenProposalGroups = useMemo(
    () =>
      openProposalGroups.slice(
        proposalPage * SENTINEL_SECTION_PAGE_SIZE,
        proposalPage * SENTINEL_SECTION_PAGE_SIZE + SENTINEL_SECTION_PAGE_SIZE
      ),
    [openProposalGroups, proposalPage]
  );
  useEffect(() => {
    setProposalPage((current) => Math.min(current, Math.max(0, proposalPageCount - 1)));
  }, [proposalPageCount]);

  const selectedProposalGroup = useMemo(
    () =>
      openProposalGroups.find((group) => group.proposals.some((proposal) => proposal.id === selectedProposalId)) ||
      null,
    [openProposalGroups, selectedProposalId]
  );
  const selectedProposal = useMemo(
    () => selectedProposalGroup?.proposal || null,
    [selectedProposalGroup]
  );
  const scan = feed?.scan;
  const stats = feed?.stats;
  const configuredMode = str(settingsQ.data?.autonomy_mode, "assist").toLowerCase();
  const currentAutonomyMode: "off" | "assist" | "auto" =
    configuredMode === "off" || configuredMode === "auto" ? configuredMode : "assist";
  const autonomyDisabled = Boolean(settingsQ.data?.agent_paused) || currentAutonomyMode === "off";
  const lastScanLabel = scan?.last_completed_at ? humanTs(scan.last_completed_at).label : "Waiting for the first check";
  const currentModeLabel = settingsQ.data?.agent_paused ? "Paused" : modeLabel(currentAutonomyMode);
  const connectedServicesCount = num(stats?.connected_services, 0);
  const inAppEventCount = num(stats?.in_app_events, 0);
  const sentinelHeroHeadline =
    settingsQ.data?.agent_paused
      ? "Sentinel is paused."
      : currentAutonomyMode === "off"
        ? "Sentinel is turned off."
        : openProposals.length > 0
          ? "Background signals need review."
          : "No background signals need review.";
  const sentinelHeroDetail =
    settingsQ.data?.agent_paused
      ? "Turn autonomy back on to resume connected-source and background-run checks."
      : currentAutonomyMode === "off"
        ? "Sentinel is not scanning connected sources or detached background work while this mode is off."
        : connectedServicesCount === 0 && inAppEventCount === 0
          ? "Connect services or start background work; Sentinel will show only signals that are not already attached to an active chat."
          : connectedServicesCount === 0
            ? `Sentinel found ${inAppEventCount} detached background signal${inAppEventCount === 1 ? "" : "s"}. Connected sources will appear here after setup.`
          : currentAutonomyMode === "auto"
            ? `Sentinel is checking ${connectedServicesCount} connected source${connectedServicesCount === 1 ? "" : "s"} plus detached background work.`
            : `Sentinel is checking ${connectedServicesCount} connected source${connectedServicesCount === 1 ? "" : "s"} and will ask before acting.`;
  const heroStats = [
    {
      label: "Signals",
      value: String(openProposals.length),
      helper: openProposals.length === 0 ? "No action needed" : "Needs review"
    },
    {
      label: "Last check",
      value: lastScanLabel,
      helper: str(scan?.last_status, "idle") || "idle"
    },
    {
      label: "Connected sources",
      value: String(stats?.connected_services ?? 0),
      helper: `${stats?.connected_services ?? 0} source${(stats?.connected_services ?? 0) === 1 ? "" : "s"} active`
    },
    {
      label: "Background runs",
      value: String(stats?.in_app_events ?? 0),
      helper: `${stats?.recent_runs ?? 0} recent run${(stats?.recent_runs ?? 0) === 1 ? "" : "s"} checked`
    }
  ];
  async function runProposal(proposal: SentinelProposal) {
    const linkedSuggestionId = proposalChatSuggestionId(proposal);
    const actionKind = str(proposal.action?.action_kind, "").trim();
    const actionPayload = asRecord(proposal.action?.payload);
    const actionPrompt = str(actionPayload.prompt, "").trim();
    if (actionKind === "chat_prompt" && actionPrompt) {
      setError(null);
      setSuccess(null);
      storeChatPendingLaunch({
        createdAt: Date.now(),
        launchMode: "message",
        message: actionPrompt,
        conversationId: str(actionPayload.conversation_id, proposalConversationId(proposal)).trim() || undefined,
        source: "sentinel",
        sentinelProposalId: proposal.id,
      });
      void dismissMutation.mutateAsync(proposal.id).catch(() => undefined);
      setSelectedProposalId((current) => current === proposal.id ? "" : current);
      setSuccess("Opening this Sentinel signal in Chat.");
      navigateToView("chat");
      return;
    }
    setError(null);
    setSuccess(null);
    setRun({
      title: proposal.title,
      status: "running",
      summary: "Opening Sentinel signal in Chat...",
      startedAt: new Date().toISOString(),
      suggestionId: linkedSuggestionId || undefined,
    });
    setRunOpen(true);
    setRunMinimized(false);
    try {
      const response = await approveMutation.mutateAsync(proposal.id);
      const traceId = str(response.trace_id, str(asRecord(response.proposal).trace_id, "")).trim();
      const proposalRecord = asRecord(response.proposal);
      const runStatus = str(proposalRecord.run_status, "").toLowerCase();
      const responseSuggestionId = str(
        proposalRecord.chat_suggestion_id,
        str(asRecord(proposalRecord.metadata).suggestion_id, linkedSuggestionId)
      ).trim();
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
        suggestionId: responseSuggestionId || undefined,
      });
      setSelectedProposalId((current) => current === proposal.id ? "" : current);
      setSuccess("Sentinel signal opened.");
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
      setSelectedProposalId((current) => current === id ? "" : current);
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
      setSelectedProposalId((current) => current === id ? "" : current);
      setSuccess("Sentinel proposal snoozed for 6 hours.");
    } catch (snoozeError) {
      setError(errMessage(snoozeError));
    }
  }

  function launchClarificationChoice(proposal: SentinelProposal, choice: SentinelClarificationChoice) {
    const conversationId = proposalConversationId(proposal);
    setError(null);
    setSuccess(null);
    storeChatPendingLaunch({
      createdAt: Date.now(),
      launchMode: "message",
      message: conversationId
        ? choice.submitText
        : `Review Sentinel signal: ${proposal.title}\n\n${choice.submitText}`,
      conversationId: conversationId || undefined,
      source: "sentinel",
    });
    void dismissMutation.mutateAsync(proposal.id).catch(() => undefined);
    setSelectedProposalId((current) => current === proposal.id ? "" : current);
    setSuccess(`Opening "${choice.label}" in Chat.`);
    navigateToView("chat");
  }

  return (
    <>
      <WorkspacePageShell spacing={1.5}>
        <WorkspacePageHeader
          eyebrow="ARK CORE"
          title="Sentinel"
          description="Spots follow-ups, routine work, and unattended issues, then suggests or handles the next step when policy allows it."
          actions={
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
                alignItems: "center"
              }}>
              <Chip
                color={autonomyDisabled ? "warning" : currentAutonomyMode === "auto" ? "success" : "info"}
                label={currentModeLabel}
              />
              <Chip label={openProposals.length > 0 ? `${openProposals.length} signals` : "No signals"} />
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
                  <Typography variant="h6">Background signals</Typography>
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      openProposalGroups.length === openProposals.length
                        ? `${openProposals.length} total`
                        : `${openProposalGroups.length} grouped`
                    }
                  />
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
                    No connected-source or detached background signals need review.
                  </Typography>
                ) : (
                  <Stack spacing={1}>
                    {pagedOpenProposalGroups.map((group) => {
                      const proposal = group.proposal;
                      const choices = proposalClarificationChoices(proposal);
                      return (
                        <ButtonBase
                          key={group.key}
                          onClick={() => setSelectedProposalId(proposal.id)}
                          aria-label={`Review ${proposal.title}`}
                          sx={{
                            width: "100%",
                            textAlign: "left",
                            px: 1,
                            py: 1.15,
                            borderBottom: "1px solid",
                            borderColor: "divider",
                            transition: "background 0.15s ease",
                            "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
                          }}
                        >
                          <Stack sx={{ width: "100%", minWidth: 0 }}>
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={0.75}
                              sx={{ justifyContent: "space-between", alignItems: { xs: "flex-start", sm: "center" } }}
                            >
                              <Stack direction="row" spacing={1} sx={{ alignItems: "center", minWidth: 0, maxWidth: "100%" }}>
                                <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: proposalDotColor(proposal.status) }} />
                                <Typography variant="subtitle2" sx={{ fontWeight: 600, minWidth: 0 }}>
                                  {proposal.title}
                                </Typography>
                                {group.proposals.length > 1 ? (
                                  <Chip size="small" variant="outlined" label={`${group.proposals.length} similar`} />
                                ) : null}
                                {choices.length > 0 ? (
                                  <Chip size="small" color="info" variant="outlined" label={`${choices.length} choices`} />
                                ) : null}
                              </Stack>
                              <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>
                                {humanTs(proposal.updated_at).label}
                              </Typography>
                            </Stack>
                            <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px", pr: 1 }}>
                              {compactText(proposal.detail, 150)}
                            </Typography>
                          </Stack>
                        </ButtonBase>
                      );
                    })}
                    {proposalPageCount > 1 ? (
                      <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", pt: 0.25 }}>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          Showing {pagedOpenProposalGroups.length} of {openProposalGroups.length}
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

        </Stack>
      </WorkspacePageShell>
      <Dialog
        open={!!selectedProposal}
        onClose={() => setSelectedProposalId("")}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            className: "diagnostics-dialog-shell",
          },
        }}
      >
        {selectedProposal ? (
          <>
            <DialogTitle className="diagnostics-dialog-title" sx={{ pb: 1 }}>
              <Stack spacing={0.75}>
                <Typography variant="h6">{selectedProposal.title}</Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap", alignItems: "center" }}>
                  <Chip size="small" color={proposalTone(selectedProposal.status)} label={proposalStatusLabel(selectedProposal.status)} />
                  <Chip size="small" variant="outlined" label={selectedProposal.source_label || sourceKindLabel(selectedProposal.source_kind)} />
                  {selectedProposalGroup && selectedProposalGroup.proposals.length > 1 ? (
                    <Chip size="small" variant="outlined" label={`${selectedProposalGroup.proposals.length} grouped`} />
                  ) : null}
                  <Chip size="small" variant="outlined" label={`Updated ${humanTs(selectedProposal.updated_at).label}`} />
                </Stack>
              </Stack>
            </DialogTitle>
            <DialogContent dividers className="diagnostics-dialog-content">
              <Stack spacing={2}>
                {proposalClarificationChoices(selectedProposal).length > 0 ? (
                  <Stack spacing={1}>
                    <Typography variant="subtitle2">Choose how to continue</Typography>
                    <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                      {proposalClarificationChoices(selectedProposal).map((choice, idx) => (
                        <Button
                          key={`${selectedProposal.id}-${choice.submitText}-${idx}`}
                          size="small"
                          variant={idx === 0 ? "contained" : "outlined"}
                          onClick={() => launchClarificationChoice(selectedProposal, choice)}
                        >
                          {choice.label}
                        </Button>
                      ))}
                    </Stack>
                  </Stack>
                ) : null}

                <Stack spacing={0.75}>
                  <Typography variant="subtitle2">Details</Typography>
                  <Typography variant="body1">{selectedProposal.detail}</Typography>
                  {selectedProposal.rationale ? (
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      Reason: {selectedProposal.rationale}
                    </Typography>
                  ) : null}
                  {selectedProposal.last_run_summary ? (
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      Last result: {selectedProposal.last_run_summary}
                    </Typography>
                  ) : null}
                  {selectedProposalGroup && selectedProposalGroup.proposals.length > 1 ? (
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Grouped {selectedProposalGroup.proposals.length} matching signals from the same source context.
                    </Typography>
                  ) : null}
                </Stack>

                <Divider />

                <Stack spacing={1}>
                  <Typography variant="subtitle2">Technical details</Typography>
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", sm: "150px minmax(0, 1fr)" },
                      columnGap: 1.5,
                      rowGap: 0.75,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Proposal ID</Typography>
                    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>{selectedProposal.id}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Kind</Typography>
                    <Typography variant="body2">{selectedProposal.proposal_kind || "-"}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Source</Typography>
                    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
                      {sourceKindLabel(selectedProposal.source_kind)}
                      {selectedProposal.source_id ? ` (${selectedProposal.source_id})` : ""}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Status</Typography>
                    <Typography variant="body2">{selectedProposal.status || "-"}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Run status</Typography>
                    <Typography variant="body2">{selectedProposal.run_status || "-"}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Priority</Typography>
                    <Typography variant="body2">{selectedProposal.priority ?? "-"}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Confidence</Typography>
                    <Typography variant="body2">
                      {typeof selectedProposal.confidence === "number" ? `${(selectedProposal.confidence * 100).toFixed(0)}%` : "-"}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Created</Typography>
                    <Typography variant="body2">{humanTs(selectedProposal.created_at).label}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Updated</Typography>
                    <Typography variant="body2">{humanTs(selectedProposal.updated_at).label}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Trace ID</Typography>
                    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>{selectedProposal.trace_id || "-"}</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>Action</Typography>
                    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
                      {selectedProposal.action?.action_kind || "-"}
                    </Typography>
                    {selectedProposal.snoozed_until ? (
                      <>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>Later until</Typography>
                        <Typography variant="body2">{humanTs(selectedProposal.snoozed_until).label}</Typography>
                      </>
                    ) : null}
                  </Box>
                </Stack>
              </Stack>
            </DialogContent>
            <DialogActions className="diagnostics-dialog-actions" sx={{ px: 3, py: 1.5 }}>
              <Button onClick={() => setSelectedProposalId("")}>Close</Button>
              <Button
                variant="outlined"
                onClick={() => void snoozeProposal(selectedProposal.id)}
                disabled={snoozeMutation.isPending}
              >
                Snooze
              </Button>
              <Button
                variant="text"
                onClick={() => void dismissProposal(selectedProposal.id)}
                disabled={dismissMutation.isPending}
              >
                Dismiss
              </Button>
              {proposalClarificationChoices(selectedProposal).length === 0 ? (
                <Button
                  variant="contained"
                  onClick={() => void runProposal(selectedProposal)}
                  disabled={approveMutation.isPending}
                >
                  {proposalActionLabel(selectedProposal)}
                </Button>
              ) : null}
            </DialogActions>
          </>
        ) : null}
      </Dialog>
      <SuggestionRunDialog
        run={run}
        open={runOpen}
        minimized={runMinimized}
        trace={asRecord(runTraceQ.data)}
        traceSteps={pickRecords(runTraceQ.data, "steps")}
        traceLoading={runTraceQ.isLoading}
        traceError={runTraceQ.error}
        detailError={runSuggestionDetailQ.error}
        acceptedOutcomes={runAcceptedOutcomes}
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
