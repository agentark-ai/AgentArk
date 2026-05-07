import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  IconButton,
  Stack,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  Tooltip,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import MemoryPage from "./MemoryPage";
import {
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";

const REFRESH_MS = 8000;

function arkmemoryHistoryEventVisible(event: JsonRecord): boolean {
  const type = str(event.event_type, "").trim();
  return [
    "memory_created",
    "memory_updated",
    "memory_status_changed",
    "queue_memory_merged",
    "ledger_event_rolled_back",
    "queue_item_rejected",
  ].includes(type);
}

function arkmemoryHistoryTypeLabel(event: JsonRecord): string {
  const type = str(event.event_type, "").trim();
  const next = asRecord(event.new_snapshot);
  const old = asRecord(event.old_snapshot);
  const nextStatus = str(next.status, "").trim().toLowerCase();
  const oldStatus = str(old.status, "").trim().toLowerCase();
  switch (type) {
    case "memory_created":
      return "Added";
    case "memory_updated":
      return "Updated";
    case "memory_status_changed":
      if (nextStatus === "deprecated" && oldStatus !== "deprecated") {
        return "Archived";
      }
      return "Status";
    case "queue_memory_merged":
      return "Consolidated";
    case "ledger_event_rolled_back":
      return "Rollback";
    case "queue_item_rejected":
      return "Rejected";
    default:
      return "Change";
  }
}

function arkmemoryHistoryMemoryTitle(event: JsonRecord): string {
  const next = asRecord(event.new_snapshot);
  const old = asRecord(event.old_snapshot);
  return (
    str(next.title, "").trim() ||
    str(old.title, "").trim() ||
    str(event.memory_id, "").trim() ||
    str(event.related_memory_id, "").trim() ||
    "Memory"
  );
}

function arkmemoryHistoryTitle(event: JsonRecord): string {
  const type = str(event.event_type, "").trim();
  const memoryTitle = arkmemoryHistoryMemoryTitle(event);
  const next = asRecord(event.new_snapshot);
  const old = asRecord(event.old_snapshot);
  const nextStatus = str(next.status, "").trim().toLowerCase();
  const oldStatus = str(old.status, "").trim().toLowerCase();
  switch (type) {
    case "memory_created":
      return `${memoryTitle} added to memory`;
    case "memory_updated":
      return `${memoryTitle} updated`;
    case "memory_status_changed":
      if (nextStatus === "deprecated" && oldStatus !== "deprecated") {
        return `${memoryTitle} moved out of active memory`;
      }
      if (oldStatus && nextStatus && oldStatus !== nextStatus) {
        return `${memoryTitle} status changed`;
      }
      return `${memoryTitle} changed`;
    case "queue_memory_merged":
      return "Memory consolidated";
    case "ledger_event_rolled_back":
      return `${memoryTitle} restored`;
    case "queue_item_rejected":
      return "Pending memory change rejected";
    default:
      return str(event.summary, memoryTitle);
  }
}

function arkmemoryHistoryDetail(event: JsonRecord): string {
  const summary = str(event.summary, "").trim();
  if (summary) return summary;
  return arkmemoryHistoryTitle(event);
}

function arkmemoryHistoryCanRestore(event: JsonRecord): boolean {
  return (
    toBool(event.reversible) && str(event.reverted_at, "").trim().length === 0
  );
}

function replayGateLabel(status: string): string {
  const normalized = status.trim().replace(/_/g, " ");
  if (!normalized) return "Not checked";
  return normalized[0].toUpperCase() + normalized.slice(1);
}

function tokenLabel(value: unknown, fallback = "Unknown"): string {
  const normalized = str(value, "").trim().replace(/[_-]+/g, " ");
  if (!normalized) return fallback;
  return normalized
    .split(/\s+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function healthSeverityColor(
  severity: unknown,
): "default" | "info" | "warning" | "error" | "success" {
  const normalized = str(severity, "").trim().toLowerCase();
  if (normalized === "warning" || normalized === "review") return "warning";
  if (normalized === "error" || normalized === "failed") return "error";
  if (normalized === "success" || normalized === "ok") return "success";
  if (normalized === "info") return "info";
  return "default";
}

function healthFindingTitle(finding: JsonRecord, fallbackIndex: number): string {
  return (
    str(finding.title, "").trim() ||
    tokenLabel(finding.kind, `Finding ${fallbackIndex + 1}`)
  );
}

function healthFindingDetail(finding: JsonRecord): string {
  return (
    str(finding.last_error_detail, "").trim() ||
    str(finding.detail, "").trim() ||
    "No additional detail was recorded."
  );
}

function healthReviewPattern(finding: JsonRecord): JsonRecord {
  return asRecord(finding.review_pattern);
}

function healthReviewPatternLabel(pattern: JsonRecord): string {
  const count = num(pattern.similar_review_count);
  const suggested = str(pattern.suggested_outcome, "").trim();
  if (count <= 0) return "";
  const suffix = suggested ? `, most often: ${tokenLabel(suggested)}` : "";
  return `${count} similar reviewed pattern${count === 1 ? "" : "s"}${suffix}`;
}

function healthSourceContext(finding: JsonRecord): JsonRecord {
  return asRecord(finding.source_context);
}

function healthSourceTitle(source: JsonRecord): string {
  const title = str(source.conversation_title, "").trim();
  const channel = str(source.conversation_channel, "").trim();
  if (title && channel) return `${title} (${tokenLabel(channel)})`;
  if (title) return title;
  if (channel) return tokenLabel(channel);
  return "Source message";
}

type ArkMemoryPageProps = {
  autoRefresh: boolean;
  onNavigateToView?: (view: string, replace?: boolean) => void;
};

export default function ArkMemoryPage({
  autoRefresh,
  onNavigateToView,
}: ArkMemoryPageProps) {
  const queryClient = useQueryClient();
  const [memoryTab, setMemoryTab] = useState<"current" | "queue" | "history">(
    "current",
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [healthDetailsOpen, setHealthDetailsOpen] = useState(false);
  const [captureDetailsOpen, setCaptureDetailsOpen] = useState(false);
  const invalidateArkMemory = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["arkmemory-summary"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-queue"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-ledger"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-health"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-stats"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-facts"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-user-data"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-knowledge"] }),
    ]);
  };

  const summaryQ = useQuery({
    queryKey: ["arkmemory-summary"],
    queryFn: () => api.rawGet("/arkmemory/summary"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const queueQ = useQuery({
    queryKey: ["arkmemory-queue"],
    queryFn: () => api.rawGet("/arkmemory/queue?limit=50"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const ledgerQ = useQuery({
    queryKey: ["arkmemory-ledger"],
    queryFn: () => api.rawGet("/arkmemory/ledger?limit=80"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const healthQ = useQuery({
    queryKey: ["arkmemory-health"],
    queryFn: () => api.rawGet("/arkmemory/health?limit=50"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const approveQueueMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/arkmemory/queue/${encodeURIComponent(id)}/approve`),
    onSuccess: async () => {
      setNotice("Memory queue item applied.");
      await invalidateArkMemory();
    },
  });
  const rejectQueueMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/arkmemory/queue/${encodeURIComponent(id)}/reject`),
    onSuccess: async () => {
      setNotice("Memory queue item rejected.");
      await invalidateArkMemory();
    },
  });
  const rollbackMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/arkmemory/ledger/${encodeURIComponent(id)}/rollback`),
    onSuccess: async () => {
      setNotice("Memory restored from history.");
      await invalidateArkMemory();
    },
  });
  const applyHealthMutation = useMutation({
    mutationFn: ({
      id,
      outcome,
    }: {
      id: string;
      outcome: "acknowledged" | "expected_sensitive_skip" | "false_positive_safe_memory";
    }) =>
      api.rawPost(`/arkmemory/health/${encodeURIComponent(id)}/apply`, {
        outcome,
      }),
    onSuccess: async () => {
      setNotice("Memory health finding marked reviewed.");
      await invalidateArkMemory();
    },
  });

  const summary = asRecord(summaryQ.data);
  const currentMemory = asRecord(summary.current_memory);
  const capturePipeline = asRecord(summary.capture_pipeline);
  const pendingCaptureEvents = pickRecords(capturePipeline, "pending_events");
  const queueItems = pickRecords(queueQ.data, "items");
  const ledgerEvents = pickRecords(ledgerQ.data, "events");
  const healthFindings = pickRecords(healthQ.data, "findings");
  const historyEvents = useMemo(
    () => ledgerEvents.filter(arkmemoryHistoryEventVisible),
    [ledgerEvents],
  );
  const historyRestoreByMemoryId = useMemo(() => {
    const map = new Map<string, string>();
    historyEvents.forEach((event) => {
      const eventId = str(event.id, "").trim();
      const memoryId = str(event.memory_id, "").trim();
      const next = asRecord(event.new_snapshot);
      const old = asRecord(event.old_snapshot);
      const nextStatus = str(next.status, "").trim().toLowerCase();
      const oldStatus = str(old.status, "").trim().toLowerCase();
      if (
        eventId &&
        memoryId &&
        arkmemoryHistoryCanRestore(event) &&
        str(event.event_type, "").trim() === "memory_status_changed" &&
        nextStatus === "deprecated" &&
        oldStatus !== nextStatus
      ) {
        map.set(memoryId, eventId);
      }
    });
    return map;
  }, [historyEvents]);
  const showQueueTab = queueItems.length > 0;
  const memoryTotal =
    num(currentMemory.facts) +
    num(currentMemory.assistant_preferences) +
    num(currentMemory.work_preferences) +
    num(currentMemory.project_domain_memory) +
    num(currentMemory.ephemeral_context) +
    num(currentMemory.other_memory) +
    num(currentMemory.preferences) +
    num(currentMemory.user_data) +
    num(currentMemory.knowledge);
  const pendingConsolidation =
    pendingCaptureEvents.length || num(capturePipeline.pending);
  const failedCaptureCount = num(capturePipeline.failed);
  useEffect(() => {
    if (!showQueueTab && memoryTab === "queue") {
      setMemoryTab("current");
    }
  }, [memoryTab, showQueueTab]);
  useEffect(() => {
    if (failedCaptureCount > 0) {
      setHealthDetailsOpen(true);
    }
  }, [failedCaptureCount]);
  const busy =
    approveQueueMutation.isPending ||
    rejectQueueMutation.isPending ||
    rollbackMutation.isPending ||
    applyHealthMutation.isPending;
  const firstError =
    summaryQ.error ||
    queueQ.error ||
    ledgerQ.error ||
    healthQ.error ||
    approveQueueMutation.error ||
    rejectQueueMutation.error ||
    rollbackMutation.error ||
    applyHealthMutation.error;
  const memoryTabValue =
    memoryTab === "current" ? 0 : memoryTab === "queue" ? 1 : showQueueTab ? 2 : 1;

  const statItems: Array<{
    label: string;
    value: number;
    helper: string;
    onClick?: () => void;
  }> = [
    { label: "Current Memory", value: memoryTotal, helper: "Stored items" },
    ...(showQueueTab
      ? [
          {
            label: "Pending Review",
            value: queueItems.length,
            helper: "Memory changes",
          },
        ]
      : []),
    ...(pendingConsolidation > 0
      ? [
          {
            label: "Queued",
            value: pendingConsolidation,
            helper: "Consolidating",
            onClick: () => setCaptureDetailsOpen(true),
          },
        ]
      : []),
    { label: "History", value: historyEvents.length, helper: "Changes and rollbacks" },
  ];
  const emptyState = (copy: string) => (
    <Typography variant="body2" sx={{ color: "text.secondary" }}>
      {copy}
    </Typography>
  );

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Ark Core / ArkMemory"
        title="ArkMemory"
        description={
          <>
            ArkMemory is what the agent remembers about you and your work.
            <br />
            ArkMemory separates profile facts, assistant preferences, work preferences, domain memory, user data, and useful knowledge gathered from chats and background signals.
          </>
        }
      />
      {notice ? (
        <Alert severity="success" onClose={() => setNotice(null)}>
          {notice}
        </Alert>
      ) : null}
      {firstError ? (
        <Alert severity="error">{errMessage(firstError)}</Alert>
      ) : null}
      <Stack
        direction="row"
        spacing={0.45}
        sx={{ alignItems: "center", color: "text.secondary" }}
      >
        <Tooltip
          title="ArkMemory consolidates background signals outside the active chat, so newly saved memories take a little time to show up here."
          arrow
          placement="top-start"
        >
          <IconButton
            size="small"
            aria-label="ArkMemory consolidation timing details"
            sx={{ p: 0.2, color: "text.secondary" }}
          >
            <InfoOutlinedIcon sx={{ fontSize: 16 }} />
          </IconButton>
        </Tooltip>
        <Typography variant="body2" sx={{ color: "text.secondary" }}>
          New memories may take a moment to appear.
        </Typography>
      </Stack>
      {pendingConsolidation > 0 ? (
        <Alert
          severity="info"
          action={
            <Button
              color="inherit"
              size="small"
              onClick={() => setCaptureDetailsOpen(true)}
            >
              Details
            </Button>
          }
        >
          {pendingConsolidation === 1
            ? "1 memory signal is queued for ArkMemory consolidation."
            : `${pendingConsolidation} memory signals are queued for ArkMemory consolidation.`}
        </Alert>
      ) : null}
      {failedCaptureCount > 0 ? (
        <Alert
          severity="warning"
          action={
            <Button
              color="inherit"
              size="small"
              onClick={() => setHealthDetailsOpen(true)}
            >
              Review
            </Button>
          }
        >
          {failedCaptureCount === 1
            ? "1 memory capture needs attention."
            : `${failedCaptureCount} memory captures need attention.`}
        </Alert>
      ) : null}
      {healthFindings.length > 0 || healthDetailsOpen ? (
        <Accordion
          disableGutters
          expanded={healthDetailsOpen}
          onChange={(_event, expanded) => setHealthDetailsOpen(expanded)}
          className="list-shell"
          sx={{
            background: "transparent",
            "&:before": { display: "none" },
          }}
        >
          <AccordionSummary expandIcon={<ExpandMoreIcon />}>
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{ alignItems: "center", flexWrap: "wrap", width: "100%" }}
            >
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                Memory Health
              </Typography>
              <Chip
                size="small"
                variant="outlined"
                color={healthFindings.length > 0 ? "warning" : "default"}
                label={`${healthFindings.length} finding${
                  healthFindings.length === 1 ? "" : "s"
                }`}
              />
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <Stack spacing={1}>
              {healthQ.isLoading ? (
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Loading memory health details...
                </Typography>
              ) : null}
              {healthQ.error ? (
                <Alert severity="warning">{errMessage(healthQ.error)}</Alert>
              ) : null}
              {!healthQ.isLoading && !healthQ.error && healthFindings.length === 0 ? (
                <Alert severity="info">
                  No active memory health findings are currently reported.
                </Alert>
              ) : null}
              {healthFindings.map((finding, index) => {
                const id = str(finding.id, `health-${index}`).trim();
                const captureEventId = str(finding.capture_event_id, "").trim();
                const status = str(finding.status, "").trim();
                const captureKind = str(finding.capture_kind, "").trim();
                const lastErrorCode = str(finding.last_error_code, "").trim();
                const reviewPattern = healthReviewPattern(finding);
                const reviewPatternLabel = healthReviewPatternLabel(reviewPattern);
                const sourceContext = healthSourceContext(finding);
                const sourceTime = humanTs(str(sourceContext.source_message_at, ""));
                const sourcePreview = str(
                  sourceContext.source_message_preview,
                  "",
                ).trim();
                const sourceMessageId = str(
                  sourceContext.source_message_id,
                  "",
                ).trim();
                const sourceChars = num(sourceContext.source_message_chars, -1);
                const isSensitiveSkip = status === "rejected_sensitive_input";
                const created = humanTs(str(finding.created_at, ""));
                return (
                  <Box
                    key={id}
                    className="metadata-box"
                    sx={{ borderColor: "var(--ui-rgba-255-193-7-180)" }}
                  >
                    <Stack spacing={0.85}>
                      <Stack
                        direction="row"
                        spacing={1}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          justifyContent: "space-between",
                          flexWrap: "wrap",
                        }}
                      >
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{ alignItems: "center", flexWrap: "wrap", minWidth: 0 }}
                        >
                          <Chip
                            size="small"
                            variant="outlined"
                            color={healthSeverityColor(finding.severity)}
                            label={tokenLabel(finding.severity, "Review")}
                          />
                          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                            {healthFindingTitle(finding, index)}
                          </Typography>
                        </Stack>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{ flexWrap: "wrap", justifyContent: "flex-end" }}
                        >
                          {isSensitiveSkip ? (
                            <Button
                              size="small"
                              variant="outlined"
                              color="warning"
                              disabled={busy || !id}
                              onClick={() =>
                                applyHealthMutation.mutate({
                                  id,
                                  outcome: "expected_sensitive_skip",
                                })
                              }
                            >
                              Correct skip
                            </Button>
                          ) : null}
                          {isSensitiveSkip ? (
                            <Button
                              size="small"
                              variant="outlined"
                              color="warning"
                              disabled={busy || !id}
                              onClick={() =>
                                applyHealthMutation.mutate({
                                  id,
                                  outcome: "false_positive_safe_memory",
                                })
                              }
                            >
                              False positive
                            </Button>
                          ) : (
                            <Button
                              size="small"
                              variant="outlined"
                              color="warning"
                              disabled={busy || !id}
                              onClick={() =>
                                applyHealthMutation.mutate({
                                  id,
                                  outcome: "acknowledged",
                                })
                              }
                            >
                              Mark reviewed
                            </Button>
                          )}
                        </Stack>
                      </Stack>
                      <Typography variant="body2" sx={{ color: "text.secondary" }}>
                        {healthFindingDetail(finding)}
                      </Typography>
                      <Box className="metadata-box">
                        <Stack spacing={0.45}>
                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                            Source: {healthSourceTitle(sourceContext)}
                            {sourceTime.label !== "-" ? ` - ${sourceTime.label}` : ""}
                            {sourceChars >= 0 ? ` - ${sourceChars} chars` : ""}
                            {sourceMessageId ? ` - msg ${sourceMessageId.slice(0, 8)}` : ""}
                          </Typography>
                          {sourcePreview ? (
                            <Typography
                              variant="body2"
                              sx={{
                                color: "text.secondary",
                                overflowWrap: "anywhere",
                              }}
                            >
                              {sourcePreview}
                            </Typography>
                          ) : null}
                        </Stack>
                      </Box>
                      {reviewPatternLabel ? (
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          {reviewPatternLabel}
                        </Typography>
                      ) : null}
                      <Stack
                        direction="row"
                        spacing={1}
                        useFlexGap
                        sx={{ flexWrap: "wrap", color: "text.secondary" }}
                      >
                        {status ? (
                          <Typography variant="caption">
                            Status: {tokenLabel(status)}
                          </Typography>
                        ) : null}
                        {captureKind ? (
                          <Typography variant="caption">
                            Type: {tokenLabel(captureKind)}
                          </Typography>
                        ) : null}
                        {lastErrorCode ? (
                          <Typography variant="caption">
                            Error: {tokenLabel(lastErrorCode)}
                          </Typography>
                        ) : null}
                        {captureEventId ? (
                          <Typography variant="caption">
                            Capture: {captureEventId.slice(0, 18)}
                          </Typography>
                        ) : null}
                        <Typography variant="caption" title={created.tip}>
                          Updated: {created.label}
                        </Typography>
                      </Stack>
                    </Stack>
                  </Box>
                );
              })}
            </Stack>
          </AccordionDetails>
        </Accordion>
      ) : null}

      <Box className="list-shell stat-strip">
        {statItems.map((item) =>
          item.onClick ? (
            <button
              key={item.label}
              type="button"
              className="stat-strip-item stat-strip-button"
              onClick={item.onClick}
            >
              <span className="stat-strip-label">{item.label}</span>
              <span className="stat-strip-value">{item.value}</span>
              <span className="stat-strip-helper">{item.helper}</span>
            </button>
          ) : (
            <div key={item.label} className="stat-strip-item">
              <span className="stat-strip-label">{item.label}</span>
              <span className="stat-strip-value">{item.value}</span>
              <span className="stat-strip-helper">{item.helper}</span>
            </div>
          ),
        )}
      </Box>

      <Dialog
        open={captureDetailsOpen}
        onClose={() => setCaptureDetailsOpen(false)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>ArkMemory Consolidation</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {summaryQ.isLoading ? (
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                Loading queued memory signals...
              </Typography>
            ) : null}
            {!summaryQ.isLoading && pendingCaptureEvents.length === 0 ? (
              <Alert severity="info">
                No memory signals are currently queued or processing.
              </Alert>
            ) : null}
            {pendingCaptureEvents.map((event, index) => {
              const id = str(event.id, `pending-${index}`);
              const sourceContext = asRecord(event.source_context);
              const sourceTime = humanTs(str(sourceContext.source_message_at, ""));
              const sourcePreview = str(
                sourceContext.source_message_preview,
                "",
              ).trim();
              const statuses = Array.isArray(event.statuses)
                ? event.statuses
                    .map((status) => tokenLabel(status))
                    .filter((status) => status.trim().length > 0)
                : [];
              const created = humanTs(str(event.created_at, ""));
              const updated = humanTs(str(event.updated_at, ""));
              const backendEvents = pickRecords(event, "events");
              return (
                <Box key={id} className="metadata-box">
                  <Stack spacing={0.85}>
                    <Stack
                      direction="row"
                      spacing={0.75}
                      useFlexGap
                      sx={{ alignItems: "center", flexWrap: "wrap" }}
                    >
                      <Chip
                        size="small"
                        variant="outlined"
                        color="info"
                        label={statuses[0] || tokenLabel(event.status, "Queued")}
                      />
                      <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                        {healthSourceTitle(sourceContext)}
                      </Typography>
                    </Stack>
                    {sourcePreview ? (
                      <Typography
                        variant="body2"
                        sx={{ color: "text.secondary", overflowWrap: "anywhere" }}
                      >
                        {sourcePreview}
                      </Typography>
                    ) : null}
                    <Stack
                      direction="row"
                      spacing={1}
                      useFlexGap
                      sx={{ flexWrap: "wrap", color: "text.secondary" }}
                    >
                      <Typography variant="caption" title={created.tip}>
                        Queued: {created.label}
                      </Typography>
                      <Typography variant="caption" title={updated.tip}>
                        Updated: {updated.label}
                      </Typography>
                      {sourceTime.label !== "-" ? (
                        <Typography variant="caption" title={sourceTime.tip}>
                          Source: {sourceTime.label}
                        </Typography>
                      ) : null}
                      <Typography variant="caption">
                        Backend events: {num(event.event_count, backendEvents.length)}
                      </Typography>
                    </Stack>
                    {backendEvents.length > 0 ? (
                      <>
                        <Divider />
                        <Stack spacing={0.45}>
                          {backendEvents.map((backendEvent, eventIndex) => {
                            const backendId = str(
                              backendEvent.id,
                              `event-${eventIndex}`,
                            );
                            const backendUpdated = humanTs(
                              str(backendEvent.updated_at, ""),
                            );
                            return (
                              <Typography
                                key={backendId}
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                  fontFamily:
                                    "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, monospace",
                                  overflowWrap: "anywhere",
                                }}
                              >
                                {backendId} -{" "}
                                {tokenLabel(backendEvent.status, "Queued")} -{" "}
                                {backendUpdated.label}
                              </Typography>
                            );
                          })}
                        </Stack>
                      </>
                    ) : null}
                  </Stack>
                </Box>
              );
            })}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCaptureDetailsOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>

      <Tabs
        value={memoryTabValue}
        onChange={(_event, next) => {
          if (next === 0) {
            setMemoryTab("current");
            return;
          }
          if (showQueueTab && next === 1) {
            setMemoryTab("queue");
            return;
          }
          setMemoryTab("history");
        }}
        variant="scrollable"
        allowScrollButtonsMobile
        sx={{
          minHeight: 0,
          "& .MuiTab-root": { minHeight: 0, py: 0.5, fontSize: "0.8rem" },
        }}
      >
        <Tab label="Current Memory" />
        {showQueueTab ? <Tab label={`Pending Review (${queueItems.length})`} /> : null}
        <Tab label={`History (${historyEvents.length})`} />
      </Tabs>

      {memoryTab === "current" ? (
        <MemoryPage
          autoRefresh={autoRefresh}
          showHeader={false}
          showScopeControls={false}
          onNavigateToView={onNavigateToView}
        />
      ) : null}

      {memoryTab === "queue" && showQueueTab ? (
        <Box className="list-shell">
          <Stack spacing={1.25}>
            <Typography variant="h6">Pending Review</Typography>
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Candidate</TableCell>
                    <TableCell>Type</TableCell>
                    <TableCell>Confidence</TableCell>
                    <TableCell>Updated</TableCell>
                    <TableCell align="right">Review</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {queueItems.map((item, idx) => {
                    const id = str(item.id, String(idx));
                    const updated = humanTs(str(item.updated_at, "-"));
                    const replayGate = asRecord(item.replay_gate);
                    const replayGateAllows = toBool(replayGate.allow_approval);
                    const replayGateStatus = str(replayGate.status, "").trim();
                    const replayGateReason =
                      str(replayGate.reason, "").trim() ||
                      "Replay gate has not checked this item yet.";
                    return (
                      <TableRow key={id}>
                        <TableCell sx={{ maxWidth: 560 }}>
                          <Stack spacing={0.35}>
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{ alignItems: "center", flexWrap: "wrap" }}
                            >
                              <Chip
                                size="small"
                                variant="outlined"
                                color={replayGateAllows ? "success" : "warning"}
                                label={`Replay: ${replayGateLabel(replayGateStatus)}`}
                              />
                            </Stack>
                            <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
                              {str(item.title, id)}
                            </Typography>
                            <Typography
                              variant="body2"
                              sx={{
                                color: "text.secondary",
                                display: "-webkit-box",
                                WebkitBoxOrient: "vertical",
                                WebkitLineClamp: 2,
                                overflow: "hidden",
                              }}
                            >
                              {str(item.summary, "No summary recorded.")}
                            </Typography>
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary" }}
                            >
                              Replay gate: {replayGateReason}
                            </Typography>
                          </Stack>
                        </TableCell>
                        <TableCell>{str(item.candidate_type)}</TableCell>
                        <TableCell
                          title="How sure ArkMemory is about this candidate, based on repeated signals."
                        >
                          {`${(num(item.confidence, 0) * 100).toFixed(0)}%`}
                        </TableCell>
                        <TableCell title={updated.tip}>{updated.label}</TableCell>
                        <TableCell align="right">
                          <Stack
                            direction="row"
                            spacing={0.75}
                            sx={{ justifyContent: "flex-end" }}
                          >
                            <Button
                              size="small"
                              variant="contained"
                              disabled={busy || !replayGateAllows}
                              title={replayGateAllows ? "Apply" : replayGateReason}
                              onClick={() => approveQueueMutation.mutate(id)}
                            >
                              Apply
                            </Button>
                            <Button
                              size="small"
                              color="warning"
                              disabled={busy}
                              onClick={() => rejectQueueMutation.mutate(id)}
                            >
                              Reject
                            </Button>
                          </Stack>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          </Stack>
        </Box>
      ) : null}

      {memoryTab === "history" ? (
        <Box className="list-shell">
          <Stack spacing={1.25}>
            <Stack spacing={0.35}>
              <Typography variant="h6">History</Typography>
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                Changes, consolidations, and rollbacks. Expand an item for
                technical detail.
              </Typography>
            </Stack>
            {historyEvents.length === 0 ? (
              emptyState(
                "No memory changes yet. Once ArkMemory adds, updates, or retires a saved fact, preference, or note, the change shows up here so you can review or undo it.",
              )
            ) : (
              <Stack spacing={1}>
                {historyEvents.map((event, idx) => {
                  const id = str(event.id, `history-${idx}`);
                  const created = humanTs(str(event.created_at, "-"));
                  const type = str(event.event_type, "").trim();
                  const relatedMemoryId = str(event.related_memory_id, "").trim();
                  const revertedAt = str(event.reverted_at, "").trim();
                  const directRestoreId = arkmemoryHistoryCanRestore(event) ? id : "";
                  const linkedRestoreId =
                    type === "queue_memory_merged" && relatedMemoryId
                      ? (historyRestoreByMemoryId.get(relatedMemoryId) ?? "")
                      : "";
                  const restoreTargetId = directRestoreId || linkedRestoreId;
                  const restoreLabel =
                    type === "queue_memory_merged" && linkedRestoreId
                      ? "Restore merged memory"
                      : "Restore previous version";
                  return (
                    <Accordion
                      key={id}
                      disableGutters
                      sx={{
                        background: "transparent",
                        border: "1px solid var(--ui-rgba-148-163-184-160)",
                        borderRadius: 1,
                        overflow: "hidden",
                        "&:before": { display: "none" },
                      }}
                    >
                      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                        <Stack spacing={0.75} sx={{ width: "100%", minWidth: 0 }}>
                          <Stack
                            direction="row"
                            spacing={0.75}
                            useFlexGap
                            sx={{ alignItems: "center", flexWrap: "wrap", pr: 1 }}
                          >
                            <Chip
                              size="small"
                              variant="outlined"
                              label={arkmemoryHistoryTypeLabel(event)}
                            />
                            {revertedAt ? (
                              <Chip
                                size="small"
                                variant="outlined"
                                label="Restored"
                              />
                            ) : restoreTargetId ? (
                              <Chip
                                size="small"
                                variant="outlined"
                                label="Restorable"
                              />
                            ) : null}
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary" }}
                            >
                              {created.label}
                            </Typography>
                          </Stack>
                          <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
                            {arkmemoryHistoryTitle(event)}
                          </Typography>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                              display: "-webkit-box",
                              WebkitBoxOrient: "vertical",
                              WebkitLineClamp: 2,
                              overflow: "hidden",
                            }}
                          >
                            {arkmemoryHistoryDetail(event)}
                          </Typography>
                        </Stack>
                      </AccordionSummary>
                      <AccordionDetails>
                        <Stack spacing={1}>
                          <Box className="metadata-box">
                            <Stack spacing={0.6}>
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                Event: {type || "-"}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                Memory: {str(event.memory_id, "-")}
                              </Typography>
                              {relatedMemoryId ? (
                                <Typography
                                  variant="caption"
                                  sx={{ color: "text.secondary" }}
                                >
                                  Related memory: {relatedMemoryId}
                                </Typography>
                              ) : null}
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                Actor: {str(event.actor, "-")}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                Recorded: {created.tip}
                              </Typography>
                            </Stack>
                          </Box>
                          {restoreTargetId ? (
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={1}
                              sx={{
                                justifyContent: "space-between",
                                alignItems: { xs: "stretch", sm: "center" },
                              }}
                            >
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                {type === "queue_memory_merged" &&
                                linkedRestoreId
                                  ? "Restores the archived source memory behind this consolidation."
                                  : "Restores the previous memory snapshot recorded for this change."}
                              </Typography>
                              <Button
                                size="small"
                                variant="outlined"
                                color="warning"
                                disabled={busy}
                                onClick={() =>
                                  rollbackMutation.mutate(restoreTargetId)
                                }
                              >
                                {restoreLabel}
                              </Button>
                            </Stack>
                          ) : null}
                        </Stack>
                      </AccordionDetails>
                    </Accordion>
                  );
                })}
              </Stack>
            )}
          </Stack>
        </Box>
      ) : null}
    </WorkspacePageShell>
  );
}
