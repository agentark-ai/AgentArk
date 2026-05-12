import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import {
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
const HEALTH_FINDINGS_PAGE_SIZE = 2;

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

// Pull the actual memory body (the thing the user remembers as "what was
// saved") from either snapshot, trying common field names. Falls back to
// the event-level summary. Returns empty string when no meaningful
// content exists — used both for filtering useless rows and for the row
// preview line. No structural decisions depend on phrasing; field names
// are canonical.
function arkmemoryHistoryPreview(event: JsonRecord): string {
  const next = asRecord(event.new_snapshot);
  const old = asRecord(event.old_snapshot);
  const candidates = [
    str(next.body, ""),
    str(next.text, ""),
    str(next.content, ""),
    str(next.value, ""),
    str(next.note, ""),
    str(next.summary, ""),
    str(old.body, ""),
    str(old.text, ""),
    str(old.content, ""),
    str(old.value, ""),
    str(old.note, ""),
    str(old.summary, ""),
    str(event.summary, ""),
  ];
  for (const candidate of candidates) {
    const trimmed = candidate.trim();
    if (trimmed) return trimmed;
  }
  return "";
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
  const [historyPage, setHistoryPage] = useState(0);
  const [historyDialogEvent, setHistoryDialogEvent] = useState<JsonRecord | null>(null);
  const HISTORY_PAGE_SIZE = 10;
  const [memoryTab, setMemoryTab] = useState<"current" | "queue" | "history">(
    "current",
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [healthDetailsOpen, setHealthDetailsOpen] = useState(false);
  const [healthPage, setHealthPage] = useState(0);
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
  const healthPageCount = Math.max(
    1,
    Math.ceil(healthFindings.length / HEALTH_FINDINGS_PAGE_SIZE),
  );
  const healthDialogPage = Math.min(healthPage, healthPageCount - 1);
  const healthStart = healthDialogPage * HEALTH_FINDINGS_PAGE_SIZE;
  const healthEnd = Math.min(
    healthStart + HEALTH_FINDINGS_PAGE_SIZE,
    healthFindings.length,
  );
  const visibleHealthFindings = healthFindings.slice(healthStart, healthEnd);
  const healthRangeLabel =
    healthFindings.length > 0
      ? `${healthStart + 1}-${healthEnd} of ${healthFindings.length}`
      : "0 of 0";
  const openHealthDetails = () => {
    setHealthPage(0);
    setHealthDetailsOpen(true);
  };
  useEffect(() => {
    if (!showQueueTab && memoryTab === "queue") {
      setMemoryTab("current");
    }
  }, [memoryTab, showQueueTab]);
  useEffect(() => {
    if (healthPage > healthPageCount - 1) {
      setHealthPage(Math.max(0, healthPageCount - 1));
    }
  }, [healthPage, healthPageCount]);
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
        eyebrow="ARK CORE"
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
              onClick={openHealthDetails}
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
      {healthFindings.length > 0 ? (
        <Box className="list-shell">
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            useFlexGap
            sx={{
              alignItems: { xs: "stretch", sm: "center" },
              justifyContent: "space-between",
            }}
          >
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{ alignItems: "center", flexWrap: "wrap" }}
            >
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                Memory Health
              </Typography>
              <Chip
                size="small"
                variant="outlined"
                color="warning"
                label={`${healthFindings.length} finding${
                  healthFindings.length === 1 ? "" : "s"
                }`}
              />
            </Stack>
            <Button
              size="small"
              variant="outlined"
              color="warning"
              onClick={openHealthDetails}
            >
              Review findings
            </Button>
          </Stack>
        </Box>
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
        open={healthDetailsOpen}
        onClose={() => setHealthDetailsOpen(false)}
        maxWidth="lg"
        fullWidth
      >
        <DialogTitle>Memory Health</DialogTitle>
        <DialogContent dividers sx={{ maxHeight: "72vh" }}>
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
            {visibleHealthFindings.map((finding, index) => {
              const absoluteIndex = healthStart + index;
              const id = str(finding.id, `health-${absoluteIndex}`).trim();
              const captureEventId = str(finding.capture_event_id, "").trim();
              const status = str(finding.status, "").trim();
              const findingKind = str(finding.kind, "").trim();
              const captureKind = str(finding.capture_kind, "").trim();
              const lastErrorCode = str(finding.last_error_code, "").trim();
              const review = asRecord(finding.review);
              const reviewOutcome = str(
                finding.review_outcome,
                str(review.outcome, ""),
              ).trim();
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
              const isAutoReviewed = findingKind === "auto_reviewed_capture";
              const canCorrectSensitiveSkip =
                status === "rejected_sensitive_input" ||
                toBool(finding.can_correct_sensitive_skip) ||
                reviewOutcome === "expected_sensitive_skip" ||
                reviewOutcome === "false_positive_safe_memory";
              const created = humanTs(str(finding.created_at, ""));
              const operation = asRecord(finding.operation);
              const operationId = str(operation.id, str(finding.operation_id, "")).trim();
              const operationType = str(
                operation.operation_type,
                str(finding.operation_type, ""),
              ).trim();
              const operationStatus = str(operation.status, status).trim();
              const operationKey = str(operation.key, "").trim();
              const operationValue = str(operation.value, "").trim();
              const operationKind = str(operation.memory_kind, "").trim();
              const operationScope = str(operation.scope, "").trim();
              const operationDurability = str(operation.durability, "").trim();
              const operationRationale = str(operation.rationale, "").trim();
              const operationConfidence = num(operation.confidence, -1);
              const evidenceRefs = Array.isArray(operation.evidence_refs)
                ? operation.evidence_refs
                    .map((value) => str(value, "").trim())
                    .filter(Boolean)
                : [];
              const hasOperationDetail =
                operationId ||
                operationType ||
                operationKey ||
                operationValue ||
                operationKind ||
                operationScope ||
                operationDurability ||
                operationRationale ||
                evidenceRefs.length > 0;
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
                        {isAutoReviewed ? (
                          <Chip
                            size="small"
                            variant="outlined"
                            color="info"
                            label="Auto-reviewed"
                          />
                        ) : null}
                        {reviewOutcome ? (
                          <Chip
                            size="small"
                            variant="outlined"
                            color="default"
                            label={tokenLabel(reviewOutcome)}
                          />
                        ) : null}
                        <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                          {healthFindingTitle(finding, absoluteIndex)}
                        </Typography>
                      </Stack>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{ flexWrap: "wrap", justifyContent: "flex-end" }}
                      >
                        {canCorrectSensitiveSkip ? (
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
                            {isAutoReviewed ? "Confirm skip" : "Correct skip"}
                          </Button>
                        ) : null}
                        {canCorrectSensitiveSkip ? (
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
                    {hasOperationDetail ? (
                      <Box className="metadata-box">
                        <Stack spacing={0.45}>
                          <Typography
                            variant="caption"
                            sx={{ color: "text.secondary", fontWeight: 700 }}
                          >
                            Operation
                          </Typography>
                          {operationKey || operationValue ? (
                            <Typography
                              variant="body2"
                              sx={{ color: "text.secondary", overflowWrap: "anywhere" }}
                            >
                              {operationKey ? `${operationKey}: ` : ""}
                              {operationValue || "No value recorded."}
                            </Typography>
                          ) : null}
                          <Stack
                            direction="row"
                            spacing={1}
                            useFlexGap
                            sx={{ flexWrap: "wrap", color: "text.secondary" }}
                          >
                            {operationType ? (
                              <Typography variant="caption">
                                Type: {tokenLabel(operationType)}
                              </Typography>
                            ) : null}
                            {operationStatus ? (
                              <Typography variant="caption">
                                Status: {tokenLabel(operationStatus)}
                              </Typography>
                            ) : null}
                            {operationKind ? (
                              <Typography variant="caption">
                                Kind: {tokenLabel(operationKind)}
                              </Typography>
                            ) : null}
                            {operationDurability ? (
                              <Typography variant="caption">
                                Durability: {tokenLabel(operationDurability)}
                              </Typography>
                            ) : null}
                            {operationScope ? (
                              <Typography variant="caption">
                                Scope: {tokenLabel(operationScope)}
                              </Typography>
                            ) : null}
                            {operationConfidence >= 0 ? (
                              <Typography variant="caption">
                                Confidence: {operationConfidence.toFixed(2)}
                              </Typography>
                            ) : null}
                          </Stack>
                          {operationRationale ? (
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary", overflowWrap: "anywhere" }}
                            >
                              Reason: {operationRationale}
                            </Typography>
                          ) : null}
                          {operationId || evidenceRefs.length > 0 ? (
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary", overflowWrap: "anywhere" }}
                            >
                              {operationId ? `Operation: ${operationId}` : ""}
                              {operationId && evidenceRefs.length > 0 ? " - " : ""}
                              {evidenceRefs.length > 0
                                ? `Evidence: ${evidenceRefs.join(", ")}`
                                : ""}
                            </Typography>
                          ) : null}
                        </Stack>
                      </Box>
                    ) : null}
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
                          Finding status: {tokenLabel(status)}
                        </Typography>
                      ) : null}
                      {captureKind ? (
                        <Typography variant="caption">
                          Capture type: {tokenLabel(captureKind)}
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
        </DialogContent>
        <DialogActions>
          <Typography
            variant="caption"
            sx={{ color: "text.secondary", mr: "auto", pl: 1 }}
          >
            {healthRangeLabel}
          </Typography>
          <Button
            size="small"
            disabled={healthDialogPage <= 0}
            onClick={() => setHealthPage((page) => Math.max(0, page - 1))}
          >
            Previous
          </Button>
          <Button
            size="small"
            disabled={healthDialogPage >= healthPageCount - 1}
            onClick={() =>
              setHealthPage((page) => Math.min(healthPageCount - 1, page + 1))
            }
          >
            Next
          </Button>
          <Button size="small" onClick={() => setHealthDetailsOpen(false)}>
            Close
          </Button>
        </DialogActions>
      </Dialog>

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
            {(() => {
              // Drop rows we have no actual content to show — "Learned user
              // memory added to memory" with no body is noise to a novice.
              // The detail dialog stays available; we just hide entries
              // that have nothing to reveal inside it either.
              const usefulHistory = historyEvents.filter(
                (event) => arkmemoryHistoryPreview(event).trim().length > 0,
              );
              if (usefulHistory.length === 0) {
                return emptyState(
                  "No memory changes to show yet. Once ArkMemory adds, updates, or retires a saved fact, preference, or note, the change shows up here so you can review or undo it.",
                );
              }
              const pageCount = Math.max(
                1,
                Math.ceil(usefulHistory.length / HISTORY_PAGE_SIZE),
              );
              const page = Math.min(historyPage, pageCount - 1);
              const start = page * HISTORY_PAGE_SIZE;
              const slice = usefulHistory.slice(start, start + HISTORY_PAGE_SIZE);
              return (
                <>
                  <Stack spacing={0.4}>
                    {slice.map((event, idx) => {
                      const id = str(event.id, `history-${start + idx}`);
                      const created = humanTs(str(event.created_at, "-"));
                      const type = str(event.event_type, "").trim();
                      const relatedMemoryId = str(
                        event.related_memory_id,
                        "",
                      ).trim();
                      const revertedAt = str(event.reverted_at, "").trim();
                      const directRestoreId = arkmemoryHistoryCanRestore(event)
                        ? id
                        : "";
                      const linkedRestoreId =
                        type === "queue_memory_merged" && relatedMemoryId
                          ? (historyRestoreByMemoryId.get(relatedMemoryId) ?? "")
                          : "";
                      const restoreTargetId = directRestoreId || linkedRestoreId;
                      const preview = arkmemoryHistoryPreview(event);
                      return (
                        <Box
                          key={id}
                          role="button"
                          tabIndex={0}
                          onClick={() => setHistoryDialogEvent(event)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              setHistoryDialogEvent(event);
                            }
                          }}
                          sx={{
                            display: "flex",
                            alignItems: "center",
                            gap: 1,
                            px: 1.25,
                            py: 0.9,
                            borderRadius: 1,
                            border: "1px solid transparent",
                            cursor: "pointer",
                            transition:
                              "background 0.16s ease, border-color 0.16s ease",
                            "&:hover": {
                              background: "var(--ui-rgba-255-255-255-035)",
                              borderColor: "var(--ui-rgba-255-255-255-080)",
                            },
                            "&:focus-visible": {
                              outline: "2px solid #78f2b0",
                              outlineOffset: "-2px",
                            },
                          }}
                        >
                          <Chip
                            size="small"
                            variant="outlined"
                            label={arkmemoryHistoryTypeLabel(event)}
                            sx={{ flex: "0 0 auto" }}
                          />
                          {revertedAt ? (
                            <Chip
                              size="small"
                              variant="outlined"
                              label="Restored"
                              sx={{ flex: "0 0 auto" }}
                            />
                          ) : restoreTargetId ? (
                            <Chip
                              size="small"
                              variant="outlined"
                              label="Restorable"
                              sx={{ flex: "0 0 auto" }}
                            />
                          ) : null}
                          <Typography
                            sx={{
                              flex: 1,
                              minWidth: 0,
                              fontSize: "0.84rem",
                              color: "var(--text-primary)",
                              whiteSpace: "nowrap",
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                            }}
                          >
                            {preview}
                          </Typography>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                              flex: "0 0 auto",
                              fontFamily: "var(--font-mono)",
                              fontSize: "0.7rem",
                            }}
                            title={created.tip}
                          >
                            {created.label}
                          </Typography>
                        </Box>
                      );
                    })}
                  </Stack>
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "center",
                      mt: 1,
                      px: 0.25,
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary" }}
                    >
                      {usefulHistory.length} change
                      {usefulHistory.length === 1 ? "" : "s"} · Page {page + 1} /{" "}
                      {pageCount}
                    </Typography>
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        variant="outlined"
                        disabled={page <= 0}
                        onClick={() =>
                          setHistoryPage((value) => Math.max(0, value - 1))
                        }
                      >
                        Previous
                      </Button>
                      <Button
                        size="small"
                        variant="outlined"
                        disabled={page >= pageCount - 1}
                        onClick={() =>
                          setHistoryPage((value) =>
                            Math.min(pageCount - 1, value + 1),
                          )
                        }
                      >
                        Next
                      </Button>
                    </Stack>
                  </Stack>
                </>
              );
            })()}
          </Stack>
        </Box>
      ) : null}

      {/* History detail dialog. Opens when a row is clicked; shows the
          full saved content, all event metadata, and the restore action
          if the change is reversible. */}
      <Dialog
        open={historyDialogEvent !== null}
        onClose={() => setHistoryDialogEvent(null)}
        maxWidth="sm"
        fullWidth
      >
        {historyDialogEvent ? (
          (() => {
            const event = historyDialogEvent;
            const eventId = str(event.id, "");
            const created = humanTs(str(event.created_at, "-"));
            const type = str(event.event_type, "").trim();
            const relatedMemoryId = str(event.related_memory_id, "").trim();
            const revertedAt = str(event.reverted_at, "").trim();
            const directRestoreId = arkmemoryHistoryCanRestore(event)
              ? eventId
              : "";
            const linkedRestoreId =
              type === "queue_memory_merged" && relatedMemoryId
                ? (historyRestoreByMemoryId.get(relatedMemoryId) ?? "")
                : "";
            const restoreTargetId = directRestoreId || linkedRestoreId;
            const restoreLabel =
              type === "queue_memory_merged" && linkedRestoreId
                ? "Restore merged memory"
                : "Restore previous version";
            const restoreHelp =
              type === "queue_memory_merged" && linkedRestoreId
                ? "Restores the archived source memory behind this consolidation."
                : "Restores the previous memory snapshot recorded for this change.";
            const preview = arkmemoryHistoryPreview(event);
            return (
              <>
                <DialogTitle
                  sx={{
                    display: "flex",
                    alignItems: "center",
                    gap: 1,
                    flexWrap: "wrap",
                  }}
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
                    variant="subtitle1"
                    sx={{ flex: 1, fontWeight: 600 }}
                  >
                    {arkmemoryHistoryTitle(event)}
                  </Typography>
                </DialogTitle>
                <DialogContent dividers>
                  <Stack spacing={1.5}>
                    {preview ? (
                      <Box
                        sx={{
                          p: 1.25,
                          borderRadius: 1,
                          border: "1px solid var(--surface-border)",
                          background: "var(--surface-bg-elevated-stronger, rgba(255,255,255,0.02))",
                        }}
                      >
                        <Typography
                          sx={{
                            fontSize: "0.88rem",
                            lineHeight: 1.6,
                            color: "var(--text-primary)",
                            whiteSpace: "pre-wrap",
                            wordBreak: "break-word",
                          }}
                        >
                          {preview}
                        </Typography>
                      </Box>
                    ) : null}
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
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary" }}
                      >
                        {restoreHelp}
                      </Typography>
                    ) : null}
                  </Stack>
                </DialogContent>
                <DialogActions>
                  {restoreTargetId ? (
                    <Button
                      size="small"
                      variant="outlined"
                      color="warning"
                      disabled={busy}
                      onClick={() => {
                        rollbackMutation.mutate(restoreTargetId);
                        setHistoryDialogEvent(null);
                      }}
                    >
                      {restoreLabel}
                    </Button>
                  ) : null}
                  <Button onClick={() => setHistoryDialogEvent(null)}>
                    Close
                  </Button>
                </DialogActions>
              </>
            );
          })()
        ) : null}
      </Dialog>
    </WorkspacePageShell>
  );
}
