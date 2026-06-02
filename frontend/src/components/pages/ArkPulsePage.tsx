import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  ButtonBase,
  Checkbox,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import ErrorOutlineRoundedIcon from "@mui/icons-material/ErrorOutlineRounded";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import { humanizeStatusLabel } from "../../lib/displayLabels";
import type {
  PulseCleanupCandidate,
  PulseCleanupPreviewResponse,
  PulseCleanupRequest,
  PulseRemediationSpec,
  PulseRunFixRequest,
} from "../../types";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { asRecord, errMessage, num, pickRecords, str, toBool, type JsonRecord } from "./pageHelpers";
import {
  arkPulseRemediationFootnote,
  arkPulseManualFollowupText,
  collapseInlineWhitespace,
  describeArkPulseRemediation,
  formatDurationFromSeconds,
  formatTimestampForHumans,
  formatTraceDuration,
  getArkPulseFixText,
  getRunnableArkPulseRemediation,
  isUserActionableDoctorFinding,
  looksLikeIsoTimestamp,
  parseArkPulseRemediationSpec,
  titleCaseLabel,
  truncateUiText,
} from "./settingsPageHelpers";
import {
  fetchArkPulseLog,
  SETTINGS_BACKGROUND_STALE_TIME_MS,
  SETTINGS_CACHE_GC_TIME_MS,
  SETTINGS_QUERY_KEYS,
} from "./settingsData";
import { humanTs } from "./workspaceUiBits";

const REFRESH_MS = 8000;

type PulsePageProps = {
  autoRefresh: boolean;
};

type PulseInlineResult = {
  severity: "success" | "info" | "warning" | "error";
  message: string;
  output?: string;
  timestamp: string;
};

type PulseFinding = {
  row: JsonRecord;
  findingIndex: number;
};

function formatBytesForUi(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value >= 10 || unitIndex === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unitIndex]}`;
}

function severityChipColor(
  severity: string,
): "error" | "warning" | "info" | "success" | "default" {
  const normalized = severity.trim().toLowerCase();
  if (["critical", "high", "error"].includes(normalized)) return "error";
  if (["medium", "warn", "warning"].includes(normalized)) return "warning";
  if (normalized === "low") return "info";
  if (["ok", "info"].includes(normalized)) return "success";
  return "default";
}

function scanStatusColor(
  status: string,
): "error" | "warning" | "info" | "success" | "default" {
  const normalized = status.trim().toLowerCase();
  if (["error", "critical", "high", "failed"].includes(normalized)) return "error";
  if (["warning", "warn", "medium"].includes(normalized)) return "warning";
  if (["ok", "success", "completed"].includes(normalized)) return "success";
  if (["running", "info"].includes(normalized)) return "info";
  return "default";
}

function statusLabel(status: string): string {
  const normalized = status.trim().toLowerCase();
  if (!normalized) return "Unknown";
  if (normalized === "ok") return "OK";
  return humanizeStatusLabel(normalized, "Unknown");
}

function eventTimestamp(event: JsonRecord): string {
  return str(event.timestamp, "").trim();
}

function eventKey(event: JsonRecord, fallback: string): string {
  return str(event.id, "").trim() || eventTimestamp(event) || fallback;
}

function eventTitle(event: JsonRecord, details: JsonRecord): string {
  const summary = collapseInlineWhitespace(str(event.summary, ""));
  if (summary) return summary;
  const message = collapseInlineWhitespace(str(event.message, ""));
  if (message) return message;
  const status = str(event.status, "").trim().toLowerCase();
  const score = num(details.doctor_score, -1);
  if (status === "ok" || score >= 90) return "All systems healthy";
  return "Issues detected";
}

function actionableFindings(details: JsonRecord): PulseFinding[] {
  return pickRecords(details, "doctor_findings")
    .map((row, findingIndex) => ({ row, findingIndex }))
    .filter((finding) => isUserActionableDoctorFinding(finding.row));
}

function manualHandlingText(finding: JsonRecord): string {
  void finding;
  return arkPulseManualFollowupText();
}

function remediationModeLabel(remediation: PulseRemediationSpec | null): string {
  if (!remediation) return "Manual";
  if (remediation.kind === "readonly_investigation") return "Diagnostic";
  if (remediation.kind === "app_restart") return "Auto restart";
  if (remediation.kind === "managed_app_operation") return "Auto app fix";
  if (remediation.kind === "tunnel_start_verify" || remediation.kind === "tunnel_restart_verify") {
    return "Auto tunnel";
  }
  return "Manual command";
}

function remediationActionLabel(remediation: PulseRemediationSpec | null): string {
  if (!remediation) return "Run remediation";
  if (remediation.kind === "app_restart") return "Restart app";
  if (remediation.kind === "tunnel_start_verify") return "Start tunnel";
  if (remediation.kind === "tunnel_restart_verify") return "Restart tunnel";
  if (remediation.kind === "readonly_investigation") return "Run diagnostic";
  if (remediation.kind === "managed_app_operation") return "Run app fix";
  return "Run action";
}

function findingEvidence(finding: JsonRecord): string {
  const evidence = collapseInlineWhitespace(str(finding.evidence, ""));
  return evidence ? truncateUiText(evidence, 220) : "";
}

async function copyClipboardText(value: string): Promise<void> {
  const text = value.trim();
  if (!text) throw new Error("Nothing to copy.");
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(textarea);
  if (!ok) throw new Error("Copy failed.");
}

export default function PulsePage({ autoRefresh }: PulsePageProps) {
  const queryClient = useQueryClient();
  const [selectedPulseEvent, setSelectedPulseEvent] = useState<JsonRecord | null>(null);
  const [activeFixId, setActiveFixId] = useState<string | null>(null);
  const [inlineResults, setInlineResults] = useState<Record<string, PulseInlineResult>>({});
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [pulsePollState, setPulsePollState] = useState<{
    baselineEventId: string;
    deadlineAt: number;
  } | null>(null);
  const [cleanupDialogOpen, setCleanupDialogOpen] = useState(false);
  const [cleanupPreview, setCleanupPreview] = useState<PulseCleanupPreviewResponse | null>(null);
  const [selectedCleanupIds, setSelectedCleanupIds] = useState<Record<string, boolean>>({});
  const [cleanupConfirmed, setCleanupConfirmed] = useState(false);
  const [cleanupJob, setCleanupJob] = useState<JsonRecord | null>(null);

  const pulseQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.arkPulseLog,
    queryFn: fetchArkPulseLog,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: pulsePollState ? 2000 : autoRefresh ? REFRESH_MS : false,
  });

  const pulseEvents = useMemo(
    () =>
      pickRecords(pulseQ.data, "events").sort((left, right) => {
        const leftTs = Date.parse(str(left.timestamp, ""));
        const rightTs = Date.parse(str(right.timestamp, ""));
        return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
      }),
    [pulseQ.data],
  );
  const pulseMeta = useMemo(() => asRecord(pulseQ.data), [pulseQ.data]);
  const pulseRunning = toBool(pulseMeta.running);
  const pulseHistoryUnavailable = toBool(pulseMeta.history_unavailable);
  const pulseHistoryUnavailableReason = str(pulseMeta.history_unavailable_reason, "").trim();
  const latestPulseEvent = asRecord(pulseEvents[0]);
  const latestPulseDetails = asRecord(latestPulseEvent.details);
  const latestPulseEventKey = eventKey(latestPulseEvent, "latest");
  const latestFindingCount = actionableFindings(latestPulseDetails).length;
  const latestPulseScore = num(latestPulseDetails.doctor_score, -1);
  const latestPulseStatus = str(latestPulseEvent.status, "").toLowerCase();

  useEffect(() => {
    if (!pulsePollState) return;
    if (Date.now() >= pulsePollState.deadlineAt) {
      setPulsePollState(null);
      return;
    }
    if (!pulseRunning && latestPulseEventKey && latestPulseEventKey !== pulsePollState.baselineEventId) {
      setPulsePollState(null);
    }
  }, [latestPulseEventKey, pulsePollState, pulseRunning]);

  const latestPulseHeadline = pulseRunning
    ? "Pulse is currently running."
    : pulseEvents.length === 0
      ? pulseHistoryUnavailable
        ? "Earlier Pulse history is unavailable."
        : "No health checks yet."
      : latestFindingCount > 0
        ? `${latestFindingCount} issue${latestFindingCount === 1 ? "" : "s"} need attention.`
        : latestPulseStatus === "ok" || latestPulseScore >= 90
          ? "System health looks good."
          : "Health check completed.";
  const latestPulseSubtitle = pulseRunning
    ? "Please wait for this run to finish before starting another."
    : pulseEvents.length === 0
      ? pulseHistoryUnavailable
        ? pulseHistoryUnavailableReason ||
          "A previous Pulse payload exists, but this runtime could not load it. New runs will appear normally."
        : "Click Run now to generate your first diagnostics report."
      : latestFindingCount > 0
        ? "Open the latest report and start with the first priority item."
        : "No urgent action needed right now.";

  const selectedPulseDetails = asRecord(selectedPulseEvent?.details);
  const selectedPulseFindings = useMemo(
    () => actionableFindings(selectedPulseDetails),
    [selectedPulseDetails],
  );
  const selectedPulseScore = num(selectedPulseDetails.doctor_score, -1);
  const selectedPulseStatus = str(selectedPulseEvent?.status, "-");
  const selectedPulseStatusOk = selectedPulseStatus.toLowerCase() === "ok";
  const selectedTimestampRaw = eventTimestamp(asRecord(selectedPulseEvent));
  const selectedCaptured = looksLikeIsoTimestamp(selectedTimestampRaw)
    ? formatTimestampForHumans(selectedTimestampRaw)
    : { label: selectedTimestampRaw || "-", tooltip: selectedTimestampRaw || "-" };
  const selectedScanLog = pickRecords(selectedPulseDetails, "scan_log");
  const selectedScanDurationMs = num(selectedPulseDetails.scan_duration_ms, -1);
  const selectedNotificationOutcome = str(selectedPulseDetails.notification_outcome, "").trim();
  const selectedPulseGuidance =
    selectedPulseFindings.length === 0 && (selectedPulseStatusOk || selectedPulseScore >= 90)
      ? {
          severity: "success" as const,
          title: "System health looks good.",
          detail: "No active issues were detected in this run.",
        }
      : selectedPulseFindings.length > 0
        ? {
            severity: "warning" as const,
            title: `${selectedPulseFindings.length} issue${selectedPulseFindings.length === 1 ? "" : "s"} need attention.`,
            detail: "Run only verified Pulse actions; findings without a runnable remediation are manual follow-up.",
          }
        : {
            severity: "info" as const,
            title: "No direct findings were returned.",
            detail: "Review the snapshot for context and run another check after changes.",
          };
  const selectedPulseHeroIcon =
    selectedPulseGuidance.severity === "success" ? (
      <CheckCircleRoundedIcon sx={{ fontSize: 22 }} />
    ) : selectedPulseGuidance.severity === "warning" ? (
      <ErrorOutlineRoundedIcon sx={{ fontSize: 22 }} />
    ) : (
      <InfoOutlinedIcon sx={{ fontSize: 22 }} />
    );
  const selectedPulsePrimaryStats = [
    {
      label: "Health score",
      value: selectedPulseScore >= 0 ? String(selectedPulseScore) : "-",
      helper:
        selectedPulseScore >= 90
          ? "Healthy run"
          : selectedPulseFindings.length > 0
            ? "Needs follow-up"
            : "Score unavailable",
    },
    {
      label: "Findings",
      value: String(selectedPulseFindings.length),
      helper:
        selectedPulseFindings.length === 0
          ? "Nothing urgent"
          : `${selectedPulseFindings.length} item${selectedPulseFindings.length === 1 ? "" : "s"} to review`,
    },
    {
      label: "Watchers",
      value: String(num(selectedPulseDetails.active_watchers, 0)),
      helper: "Active background monitors",
    },
  ];
  const cleanupCandidates = cleanupPreview?.candidates ?? [];
  const selectedCleanupCandidates = cleanupCandidates.filter((candidate) => selectedCleanupIds[candidate.id]);
  const selectedCleanupSize = selectedCleanupCandidates.reduce(
    (sum, candidate) => sum + num(candidate.size_bytes, 0),
    0,
  );

  const triggerPulseMutation = useMutation({
    mutationFn: () => api.rawPost("/arkpulse/trigger", {}),
  });

  const runPulseFixMutation = useMutation({
    mutationFn: async (payload: {
      fixCommand: string;
      remediation?: PulseRemediationSpec | null;
      issueTitle: string;
      target: string;
      eventTimestamp: string;
      findingIndex: number;
    }) => {
      const body: PulseRunFixRequest = {
        issue_title: payload.issueTitle,
        target: payload.target,
        event_timestamp: payload.eventTimestamp,
        finding_index: payload.findingIndex,
      };
      if (!payload.eventTimestamp || !Number.isFinite(payload.findingIndex)) {
        const fixCommand = payload.fixCommand.trim();
        if (fixCommand) body.fix_command = fixCommand;
        if (payload.remediation) body.remediation = payload.remediation;
      }
      const out = asRecord(await api.rawPost("/arkpulse/fix", body));
      const status = str(out.status, "").toLowerCase();
      if (status === "error") {
        throw new Error(str(out.error, "").trim() || str(out.message, "").trim() || "Pulse fix failed.");
      }
      return out;
    },
  });

  const cleanupPreviewMutation = useMutation({
    mutationFn: async () => asRecord(await api.rawPost("/arkpulse/cleanup-preview", {})),
  });

  const cleanupMutation = useMutation({
    mutationFn: async (body: PulseCleanupRequest) => asRecord(await api.rawPost("/arkpulse/cleanup", body)),
  });

  async function runArkPulseCheck() {
    setError(null);
    setSuccess(null);
    try {
      const out = asRecord(await triggerPulseMutation.mutateAsync());
      const status = str(out.status, "").toLowerCase();
      setSuccess(
        status === "already_running"
          ? str(out.message, "Pulse is already running.")
          : str(out.message, "Pulse check started."),
      );
      setPulsePollState({
        baselineEventId: latestPulseEventKey,
        deadlineAt: Date.now() + 2 * 60 * 1000,
      });
      await queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEYS.arkPulseLog });
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function openCleanupReview() {
    setError(null);
    setSuccess(null);
    setCleanupDialogOpen(true);
    setCleanupPreview(null);
    setCleanupConfirmed(false);
    setCleanupJob(null);
    try {
      const out = (await cleanupPreviewMutation.mutateAsync()) as unknown as PulseCleanupPreviewResponse;
      const candidates: PulseCleanupCandidate[] = Array.isArray(out.candidates) ? out.candidates : [];
      setCleanupPreview({ ...out, candidates });
      const defaults: Record<string, boolean> = {};
      for (const candidate of candidates) {
        defaults[candidate.id] = Boolean(candidate.selected_by_default);
      }
      setSelectedCleanupIds(defaults);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function submitCleanupArchive() {
    setError(null);
    setSuccess(null);
    const candidateIds = selectedCleanupCandidates.map((candidate) => candidate.id);
    if (!cleanupConfirmed || candidateIds.length === 0) return;
    try {
      const out = await cleanupMutation.mutateAsync({
        candidate_ids: candidateIds,
        confirm_archive: true,
      });
      setCleanupJob(out);
      setSuccess(str(out.message, "Pulse cleanup is running on its background worker."));
      await queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEYS.arkPulseLog });
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function runFindingFix(finding: PulseFinding, fixId: string) {
    const row = finding.row;
    const typedRemediation = parseArkPulseRemediationSpec(row.remediation);
    const title = str(row.title, "Issue");
    const target = str(row.target, "-");
    const rawFixCommand = str(row.fix_command, "").trim();
    setError(null);
    setSuccess(null);
    setActiveFixId(fixId);
    setInlineResults((prev) => {
      if (!prev[fixId]) return prev;
      const next = { ...prev };
      delete next[fixId];
      return next;
    });
    try {
      const result = await runPulseFixMutation.mutateAsync({
        fixCommand: rawFixCommand,
        remediation: typedRemediation,
        issueTitle: title,
        target,
        eventTimestamp: selectedTimestampRaw,
        findingIndex: finding.findingIndex,
      });
      const message = str(result.message, "Pulse fix completed.").trim() || "Pulse fix completed.";
      const output = str(result.output, "").trim();
      setSuccess(output ? `${message}\n\n${output}` : message);
      setInlineResults((prev) => ({
        ...prev,
        [fixId]: {
          severity: "success",
          message,
          output,
          timestamp: new Date().toISOString(),
        },
      }));
      await queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEYS.arkPulseLog });
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-workspace-tunnel"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-workspace-apps"] });
    } catch (e) {
      const message = errMessage(e);
      setError(message);
      setInlineResults((prev) => ({
        ...prev,
        [fixId]: {
          severity: "error",
          message,
          timestamp: new Date().toISOString(),
        },
      }));
    } finally {
      setActiveFixId((current) => (current === fixId ? null : current));
    }
  }

  return (
    <WorkspacePageShell spacing={1.5} className="arkpulse-page-shell">
      <WorkspacePageHeader
        eyebrow="ARK CORE"
        title="Pulse"
        description="Pulse runs checks and offers a one-click fix when it can safely resolve an issue."
        actions={
          <Stack direction="row" spacing={0.75}>
            <Button
              size="small"
              variant="outlined"
              onClick={() => void openCleanupReview()}
              disabled={cleanupPreviewMutation.isPending || cleanupMutation.isPending}
            >
              {cleanupPreviewMutation.isPending ? "Loading..." : "Cleanup"}
            </Button>
            <Button
              size="small"
              variant="contained"
              startIcon={
                triggerPulseMutation.isPending || pulseRunning ? (
                  <CircularProgress size={14} color="inherit" />
                ) : (
                  <AutorenewRoundedIcon fontSize="small" />
                )
              }
              onClick={() => void runArkPulseCheck()}
              disabled={triggerPulseMutation.isPending || pulseRunning}
            >
              {triggerPulseMutation.isPending || pulseRunning ? "Running..." : "Run now"}
            </Button>
          </Stack>
        }
      />

      {success ? (
        <Alert severity="success" onClose={() => setSuccess(null)} sx={{ whiteSpace: "pre-wrap" }}>
          {success}
        </Alert>
      ) : null}
      {error ? (
        <Alert severity="error" onClose={() => setError(null)}>
          {error}
        </Alert>
      ) : null}
      {pulseQ.error ? <Alert severity="error">{errMessage(pulseQ.error)}</Alert> : null}

      <Box className="list-shell" sx={{ minHeight: 260, display: "flex", flexDirection: "column" }}>
        {!pulseQ.error ? (
          <Alert
            severity={
              pulseRunning
                ? "info"
                : pulseHistoryUnavailable || latestFindingCount > 0
                  ? "warning"
                  : "success"
            }
            sx={{ mb: 1 }}
          >
            <Typography variant="subtitle2">{latestPulseHeadline}</Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              {latestPulseSubtitle}
            </Typography>
          </Alert>
        ) : null}

        {pulseEvents.length === 0 ? (
          <Stack spacing={1} sx={{ flex: 1 }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              {pulseHistoryUnavailable
                ? "Stored Pulse history could not be loaded in this runtime."
                : "No Pulse events yet."}
            </Typography>
            <Box sx={{ flex: 1 }} />
          </Stack>
        ) : (
          <Stack spacing={0} sx={{ flex: 1, minHeight: 0, borderTop: "1px solid", borderColor: "divider" }}>
            {pulseEvents.slice(0, 40).map((event, index) => {
              const details = asRecord(event.details);
              const findings = actionableFindings(details);
              const score = num(details.doctor_score, -1);
              const status = str(event.status, "-");
              const ok = status.toLowerCase() === "ok";
              const overdue = num(event.overdue_tasks, 0);
              const failed = num(event.failed_tasks, 0);
              const metaParts: string[] = [];
              if (score >= 0) metaParts.push(`Score ${score}`);
              if (findings.length > 0) metaParts.push(`${findings.length} finding${findings.length === 1 ? "" : "s"}`);
              if (overdue > 0) metaParts.push(`${overdue} overdue`);
              if (failed > 0) metaParts.push(`${failed} failed`);
              if (metaParts.length === 0) metaParts.push("No issues");
              return (
                <ButtonBase
                  key={eventKey(event, String(index))}
                  onClick={() => setSelectedPulseEvent(event)}
                  sx={{
                    width: "100%",
                    textAlign: "left",
                    justifyContent: "flex-start",
                    px: 0,
                    py: 0.85,
                    borderBottom: "1px solid",
                    borderColor: "divider",
                    transition: "background 0.15s ease",
                    "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
                    display: "block",
                  }}
                >
                  <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", justifyContent: "space-between" }}>
                    <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", minWidth: 0, flex: 1 }}>
                      <Box
                        component="span"
                        sx={{
                          width: 7,
                          height: 7,
                          borderRadius: "50%",
                          flexShrink: 0,
                          bgcolor: ok ? "var(--ui-rgba-74-210-157-850)" : "var(--ui-rgba-255-180-60-850)",
                        }}
                      />
                      <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
                        {eventTitle(event, details)}
                      </Typography>
                    </Stack>
                    <Typography variant="caption" sx={{ color: "text.secondary", whiteSpace: "nowrap", flexShrink: 0 }} title={humanTs(eventTimestamp(event)).tip}>
                      {humanTs(eventTimestamp(event)).label}
                    </Typography>
                  </Stack>
                  <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px", display: "block" }}>
                    {metaParts.join(" / ")}
                  </Typography>
                </ButtonBase>
              );
            })}
          </Stack>
        )}
      </Box>

      <Dialog
        open={selectedPulseEvent != null}
        onClose={() => setSelectedPulseEvent(null)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-bg-elevated)",
              boxShadow: "0 30px 96px var(--ui-rgba-0-0-0-500)",
            },
          },
        }}
      >
        <DialogTitle sx={{ pb: 1.2, borderBottom: "1px solid var(--ui-rgba-255-255-255-060)" }}>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1.25}
            sx={{ justifyContent: "space-between", alignItems: { xs: "flex-start", sm: "center" } }}
          >
            <Box>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Pulse Run
              </Typography>
              <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.35, maxWidth: 720 }}>
                {str(selectedPulseEvent?.summary, "Health check details, findings, and scan ledger.")}
              </Typography>
            </Box>
            <Chip
              size="small"
              label={selectedPulseStatus}
              color={selectedPulseStatusOk ? "success" : "warning"}
              variant="outlined"
            />
          </Stack>
        </DialogTitle>
        <DialogContent sx={{ pt: 2 }}>
          <Stack spacing={1.25}>
            <Box
              sx={{
                borderRadius: "8px",
                border: "1px solid var(--ui-rgba-255-255-255-080)",
                background: "var(--ui-rgba-255-255-255-020)",
                p: { xs: 1.5, sm: 1.75 },
                boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)",
              }}
            >
              <Stack spacing={1.5}>
                <Stack direction={{ xs: "column", sm: "row" }} spacing={1} useFlexGap sx={{ alignItems: { xs: "flex-start", sm: "center" }, flexWrap: "wrap" }}>
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`Captured: ${selectedCaptured.label}`}
                    title={selectedCaptured.tooltip}
                    sx={{ borderColor: "var(--ui-rgba-255-255-255-140)", background: "var(--ui-rgba-255-255-255-030)" }}
                  />
                  <Chip size="small" label={`Status: ${selectedPulseStatus}`} color={selectedPulseStatusOk ? "success" : "warning"} variant="outlined" />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`${selectedPulseFindings.length} priority item${selectedPulseFindings.length === 1 ? "" : "s"}`}
                    sx={{ borderColor: "var(--ui-rgba-255-255-255-140)", background: "var(--ui-rgba-255-255-255-030)" }}
                  />
                  {selectedScanDurationMs >= 0 ? (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`Scan: ${formatTraceDuration(selectedScanDurationMs)}`}
                      sx={{ borderColor: "var(--ui-rgba-255-255-255-140)", background: "var(--ui-rgba-255-255-255-030)" }}
                    />
                  ) : null}
                </Stack>

                <Grid2 container spacing={1.25} sx={{ alignItems: "stretch" }}>
                  <Grid2 size={{ xs: 12, lg: 7 }}>
                    <Stack direction="row" spacing={1.25} sx={{ alignItems: "flex-start" }}>
                      <Box
                        sx={{
                          width: 42,
                          height: 42,
                          borderRadius: "8px",
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          color: "var(--ui-rgba-243-246-250-920)",
                          background: "var(--ui-rgba-255-255-255-050)",
                          border: "1px solid var(--ui-rgba-255-255-255-080)",
                          flex: "0 0 auto",
                        }}
                      >
                        {selectedPulseHeroIcon}
                      </Box>
                      <Stack spacing={0.65} sx={{ minWidth: 0 }}>
                        <Typography variant="h6" sx={{ fontWeight: 700, lineHeight: 1.15 }}>
                          {selectedPulseGuidance.title}
                        </Typography>
                        <Typography variant="body2" sx={{ color: "text.secondary", maxWidth: 560, lineHeight: 1.55 }}>
                          {selectedPulseGuidance.detail}
                        </Typography>
                      </Stack>
                    </Stack>
                  </Grid2>
                  <Grid2 size={{ xs: 12, lg: 5 }}>
                    <Box
                      sx={{
                        display: "grid",
                        gridTemplateColumns: { xs: "1fr", sm: "repeat(3, minmax(0, 1fr))" },
                        gap: 1,
                        height: "100%",
                      }}
                    >
                      {selectedPulsePrimaryStats.map((item) => (
                        <Box
                          key={item.label}
                          sx={{
                            minWidth: 0,
                            p: 1.2,
                            borderRadius: "8px",
                            border: "1px solid var(--ui-rgba-255-255-255-080)",
                            background: "var(--ui-rgba-255-255-255-030)",
                          }}
                        >
                          <Typography variant="caption" sx={{ display: "block", color: "var(--ui-rgba-188-198-212-700)" }}>
                            {item.label}
                          </Typography>
                          <Typography variant="h5" sx={{ mt: 0.35, fontWeight: 700, fontVariantNumeric: "tabular-nums" }}>
                            {item.value}
                          </Typography>
                          <Typography variant="caption" sx={{ display: "block", mt: 0.4, color: "text.secondary" }}>
                            {item.helper}
                          </Typography>
                        </Box>
                      ))}
                    </Box>
                  </Grid2>
                </Grid2>
              </Stack>
            </Box>

            <Stack spacing={0.3} sx={{ pt: 0.35 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                Priority actions
              </Typography>
              <Typography variant="body2" sx={{ color: "var(--ui-rgba-188-198-212-720)" }}>
                {selectedPulseFindings.length === 0
                  ? "This run did not return any actionable issues."
                  : "Work from top to bottom. Runnable remediation appears only when AgentArk has a real action for the finding."}
              </Typography>
            </Stack>

            {selectedPulseFindings.length === 0 ? (
              <Box
                sx={{
                  borderRadius: "8px",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "var(--ui-rgba-255-255-255-020)",
                  px: 1.4,
                  py: 1.25,
                }}
              >
                <Stack direction="row" spacing={1.1} sx={{ alignItems: "flex-start" }}>
                  <CheckCircleRoundedIcon sx={{ fontSize: 20, color: "success.main", mt: 0.2 }} />
                  <Stack spacing={0.35}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      Nothing urgent in this run
                    </Typography>
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      The system snapshot below is still useful for context, but there is no remediation queued from this report.
                    </Typography>
                  </Stack>
                </Stack>
              </Box>
            ) : (
              <Grid2 container spacing={1.25}>
                {selectedPulseFindings.slice(0, 20).map((finding, displayIndex) => {
                  const row = finding.row;
                  const severity = str(row.severity, "");
                  const title = str(row.title, "Issue");
                  const target = str(row.target, "-");
                  const cause = str(row.root_cause, "-");
                  const typedRemediation = parseArkPulseRemediationSpec(row.remediation);
                  const runnableRemediation = typedRemediation ?? getRunnableArkPulseRemediation(row);
                  const displayRemediation = typedRemediation ?? runnableRemediation;
                  const rawFixCommand = str(row.fix_command, "").trim();
                  const fix = displayRemediation ? describeArkPulseRemediation(displayRemediation) : getArkPulseFixText(row);
                  const canCopyFix = fix.trim().length > 0 && fix.trim() !== "-";
                  const canRunFix = runnableRemediation != null;
                  const fixId = `${selectedTimestampRaw}:${finding.findingIndex}:${title}:${target}`;
                  const fixBusy = activeFixId === fixId && runPulseFixMutation.isPending;
                  const inlineResult = inlineResults[fixId];
                  const evidence = findingEvidence(row);
                  const useMonospaceFix = displayRemediation?.kind === "shell_command" || (!displayRemediation && rawFixCommand.length > 0);
                  return (
                    <Grid2 key={fixId} size={{ xs: 12, xl: 6 }}>
                      <Box
                        sx={{
                          height: "100%",
                          borderRadius: "8px",
                          border: "1px solid var(--ui-rgba-255-255-255-080)",
                          background: "var(--ui-rgba-255-255-255-020)",
                          p: 1.35,
                        }}
                      >
                        <Stack spacing={0.75}>
                          <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "flex-start", flexWrap: "wrap" }}>
                            <Box
                              sx={{
                                minWidth: 28,
                                height: 28,
                                borderRadius: "8px",
                                display: "flex",
                                alignItems: "center",
                                justifyContent: "center",
                                background: "var(--ui-rgba-255-255-255-060)",
                                color: "var(--ui-rgba-243-246-250-920)",
                                fontSize: "0.8rem",
                                fontWeight: 700,
                              }}
                            >
                              {displayIndex + 1}
                            </Box>
                            <Stack spacing={0.15} sx={{ minWidth: 0, flex: 1 }}>
                              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                                {title}
                              </Typography>
                              <Typography variant="caption" sx={{ color: "var(--ui-rgba-188-198-212-700)" }}>
                                Target: {target}
                              </Typography>
                            </Stack>
                            <Chip size="small" label={severity || "-"} color={severityChipColor(severity)} />
                            <Chip
                              size="small"
                              label={remediationModeLabel(displayRemediation)}
                              color={canRunFix ? "success" : "default"}
                              variant="outlined"
                            />
                          </Stack>

                          <Typography variant="body2" sx={{ color: "var(--ui-rgba-231-236-243-720)", lineHeight: 1.55 }}>
                            {cause === "-" ? "The run flagged this issue but did not include a detailed cause." : cause}
                          </Typography>

                          <Box
                            sx={{
                              border: "1px solid var(--ui-rgba-255-255-255-060)",
                              borderRadius: "8px",
                              p: 1.05,
                              background: "var(--ui-rgba-255-255-255-030)",
                            }}
                          >
                            <Typography variant="caption" sx={{ color: "var(--ui-rgba-188-198-212-700)" }}>
                              Recommended next step
                            </Typography>
                            <Typography
                              variant="body2"
                              sx={{
                                mt: 0.6,
                                ...(useMonospaceFix
                                  ? { fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" }
                                  : {}),
                                whiteSpace: "pre-wrap",
                                overflowWrap: "anywhere",
                                color: "var(--ui-rgba-245-247-250-920)",
                              }}
                            >
                              {fix}
                            </Typography>
                          </Box>

                          {!canRunFix ? (
                            <Alert severity="warning" icon={<ErrorOutlineRoundedIcon />}>
                              <Typography variant="body2">{manualHandlingText(row)}</Typography>
                            </Alert>
                          ) : (
                            <Typography variant="caption" sx={{ color: "var(--ui-rgba-188-198-212-660)", lineHeight: 1.45 }}>
                              {arkPulseRemediationFootnote(typedRemediation, canRunFix)}
                            </Typography>
                          )}

                          {evidence ? (
                            <Typography variant="caption" sx={{ color: "var(--ui-rgba-188-198-212-660)", lineHeight: 1.45 }}>
                              Evidence: {evidence}
                            </Typography>
                          ) : null}

                          {canRunFix ? (
                            <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                              <Tooltip title={canCopyFix ? "Copy the recommended step" : "No remediation text to copy"} arrow>
                                <span>
                                  <Button
                                    size="small"
                                    variant="outlined"
                                    startIcon={<ContentCopyRoundedIcon fontSize="small" />}
                                    disabled={!canCopyFix}
                                    onClick={async () => {
                                      setError(null);
                                      setSuccess(null);
                                      try {
                                        await copyClipboardText(fix);
                                        setSuccess("Remediation copied.");
                                      } catch (e) {
                                        setError(errMessage(e));
                                      }
                                    }}
                                  >
                                    Copy next step
                                  </Button>
                                </span>
                              </Tooltip>
                              <Button
                                size="small"
                                variant="contained"
                                startIcon={fixBusy ? <CircularProgress size={14} color="inherit" /> : <AutorenewRoundedIcon fontSize="small" />}
                                disabled={runPulseFixMutation.isPending || !selectedTimestampRaw}
                                onClick={() => void runFindingFix(finding, fixId)}
                              >
                                {fixBusy ? "Running..." : remediationActionLabel(displayRemediation)}
                              </Button>
                            </Stack>
                          ) : null}

                          {inlineResult ? (
                            <Alert severity={inlineResult.severity} className="arkpulse-inline-result">
                              <Typography variant="body2" className="arkpulse-inline-result-message">
                                {inlineResult.message}
                              </Typography>
                              {inlineResult.output ? (
                                <Box component="pre" className="arkpulse-inline-result-output">
                                  {inlineResult.output}
                                </Box>
                              ) : null}
                            </Alert>
                          ) : null}
                        </Stack>
                      </Box>
                    </Grid2>
                  );
                })}
              </Grid2>
            )}

            <Stack spacing={0.3} sx={{ pt: 0.25 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                Run ledger
              </Typography>
              <Typography variant="body2" sx={{ color: "var(--ui-rgba-188-198-212-720)" }}>
                This shows exactly what Pulse scanned, how long each phase took, and what happened with notifications. Sections stay collapsed until you open them.
              </Typography>
            </Stack>
            <Box sx={{ borderRadius: "8px", border: "1px solid var(--ui-rgba-255-255-255-080)", overflow: "hidden" }}>
              {selectedScanLog.length === 0 ? (
                <Typography variant="body2" sx={{ color: "text.secondary", p: 1.4 }}>
                  No scan ledger was attached to this run.
                </Typography>
              ) : (
                <Stack divider={<Divider flexItem />} spacing={0}>
                  {selectedScanLog.map((row, index) => {
                    const status = str(row.status, "info");
                    const metrics = pickRecords(row, "metrics");
                    return (
                      <Accordion key={`${str(row.id, "scan")}-${index}`} disableGutters elevation={0} square sx={{ background: "transparent", color: "inherit", "&:before": { display: "none" } }}>
                        <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                          <Stack direction={{ xs: "column", sm: "row" }} spacing={0.75} useFlexGap sx={{ width: "100%", alignItems: { xs: "flex-start", sm: "center" } }}>
                            <Chip size="small" color={scanStatusColor(status)} label={statusLabel(status)} />
                            <Typography variant="body2" sx={{ fontWeight: 700, flex: 1 }}>
                              {str(row.title, str(row.section, `Scan phase ${index + 1}`))}
                            </Typography>
                            <Typography variant="caption" sx={{ color: "text.secondary", whiteSpace: "nowrap" }}>
                              {formatTraceDuration(row.duration_ms)}
                            </Typography>
                          </Stack>
                        </AccordionSummary>
                        <AccordionDetails>
                          <Stack spacing={0.75}>
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              {str(row.detail, str(row.summary, "No detail was recorded."))}
                            </Typography>
                            {metrics.length > 0 ? (
                              <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
                                {metrics.slice(0, 8).map((metric, metricIndex) => (
                                  <Chip
                                    key={`${str(metric.label, "metric")}-${metricIndex}`}
                                    size="small"
                                    variant="outlined"
                                    label={`${titleCaseLabel(str(metric.label, "Metric").replace(/:$/, ""))}: ${str(metric.value, "-")}`}
                                  />
                                ))}
                              </Stack>
                            ) : null}
                          </Stack>
                        </AccordionDetails>
                      </Accordion>
                    );
                  })}
                </Stack>
              )}
              {selectedNotificationOutcome ? (
                <Alert severity="info" sx={{ m: 1.2 }}>
                  Notification outcome: {selectedNotificationOutcome}
                </Alert>
              ) : null}
            </Box>
          </Stack>
        </DialogContent>
      </Dialog>

      <Dialog open={cleanupDialogOpen} onClose={() => setCleanupDialogOpen(false)} maxWidth="md" fullWidth>
        <DialogTitle>Managed Artifact Cleanup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.2}>
            {cleanupPreviewMutation.isPending ? (
              <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
                <CircularProgress size={16} />
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Loading cleanup preview on the Pulse worker.
                </Typography>
              </Stack>
            ) : cleanupPreview ? (
              <>
                <Alert severity={cleanupCandidates.length > 0 ? "info" : "success"}>
                  {cleanupCandidates.length > 0
                    ? `${cleanupCandidates.length} managed artifact${cleanupCandidates.length === 1 ? "" : "s"} can be archived. Selected size: ${formatBytesForUi(selectedCleanupSize)}.`
                    : "No managed cleanup candidates were found."}
                </Alert>
                <Stack spacing={0.75}>
                  {cleanupCandidates.map((candidate: PulseCleanupCandidate) => {
                    const checked = Boolean(selectedCleanupIds[candidate.id]);
                    return (
                      <Box
                        key={candidate.id}
                        sx={{
                          border: "1px solid",
                          borderColor: checked ? "primary.main" : "divider",
                          borderRadius: "8px",
                          p: 1,
                        }}
                      >
                        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ alignItems: { sm: "flex-start" } }}>
                          <Checkbox
                            checked={checked}
                            onChange={(event) =>
                              setSelectedCleanupIds((prev) => ({ ...prev, [candidate.id]: event.target.checked }))
                            }
                            slotProps={{ input: { "aria-label": `Select ${candidate.path_label}` } }}
                          />
                          <Stack spacing={0.45} sx={{ minWidth: 0, flex: 1 }}>
                            <Stack direction="row" spacing={0.6} useFlexGap sx={{ flexWrap: "wrap", alignItems: "center" }}>
                              <Chip size="small" label={candidate.category_label || titleCaseLabel(candidate.category)} />
                              <Chip size="small" variant="outlined" color={severityChipColor(candidate.risk)} label={candidate.risk} />
                              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                {formatBytesForUi(num(candidate.size_bytes, 0))} - {num(candidate.age_days, 0).toFixed(1)}d old
                              </Typography>
                            </Stack>
                            <Typography variant="body2" sx={{ fontWeight: 700, wordBreak: "break-word" }}>
                              {candidate.path_label}
                            </Typography>
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              {candidate.reason}
                            </Typography>
                          </Stack>
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
                {cleanupCandidates.length > 0 ? (
                  <FormControlLabel
                    control={
                      <Checkbox
                        checked={cleanupConfirmed}
                        onChange={(event) => setCleanupConfirmed(event.target.checked)}
                      />
                    }
                    label={`Archive selected live artifacts to ${cleanupPreview.archive_root}; archives auto-delete after ${cleanupPreview.archive_retention_days} days.`}
                  />
                ) : null}
                {cleanupJob ? (
                  <Alert severity="success">
                    Cleanup job {str(cleanupJob.job_id, "-")} is {humanizeStatusLabel(str(cleanupJob.status, "accepted"))}.
                  </Alert>
                ) : null}
              </>
            ) : (
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                Cleanup preview is not loaded.
              </Typography>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCleanupDialogOpen(false)}>Close</Button>
          <Button
            variant="contained"
            onClick={() => void submitCleanupArchive()}
            disabled={
              cleanupMutation.isPending ||
              !cleanupConfirmed ||
              selectedCleanupCandidates.length === 0 ||
              !cleanupPreview
            }
          >
            {cleanupMutation.isPending ? "Queueing..." : "Archive selected"}
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
