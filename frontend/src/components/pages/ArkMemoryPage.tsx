import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  Stack,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import MemoryPage from "./MemoryPage";
import { withProjectScope } from "./projectScope";
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

type ArkMemoryPageProps = {
  autoRefresh: boolean;
  projects: JsonRecord[];
  activeProjectId: string;
  onNavigateToView?: (view: string, replace?: boolean) => void;
};

export default function ArkMemoryPage({
  autoRefresh,
  projects,
  activeProjectId,
  onNavigateToView,
}: ArkMemoryPageProps) {
  const queryClient = useQueryClient();
  const [memoryTab, setMemoryTab] = useState<"current" | "queue" | "history">(
    "current",
  );
  const [notice, setNotice] = useState<string | null>(null);
  const invalidateArkMemory = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["arkmemory-summary"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-queue"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-ledger"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-stats"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-facts"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-user-data"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-knowledge"] }),
    ]);
  };

  const summaryQ = useQuery({
    queryKey: ["arkmemory-summary", activeProjectId],
    queryFn: () =>
      api.rawGet(withProjectScope("/arkmemory/summary", activeProjectId)),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const queueQ = useQuery({
    queryKey: ["arkmemory-queue", activeProjectId],
    queryFn: () =>
      api.rawGet(withProjectScope("/arkmemory/queue?limit=50", activeProjectId)),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const ledgerQ = useQuery({
    queryKey: ["arkmemory-ledger", activeProjectId],
    queryFn: () =>
      api.rawGet(withProjectScope("/arkmemory/ledger?limit=80", activeProjectId)),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const approveQueueMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(
        withProjectScope(
          `/arkmemory/queue/${encodeURIComponent(id)}/approve`,
          activeProjectId,
        ),
      ),
    onSuccess: async () => {
      setNotice("Memory queue item applied.");
      await invalidateArkMemory();
    },
  });
  const rejectQueueMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(
        withProjectScope(
          `/arkmemory/queue/${encodeURIComponent(id)}/reject`,
          activeProjectId,
        ),
      ),
    onSuccess: async () => {
      setNotice("Memory queue item rejected.");
      await invalidateArkMemory();
    },
  });
  const rollbackMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(
        withProjectScope(
          `/arkmemory/ledger/${encodeURIComponent(id)}/rollback`,
          activeProjectId,
        ),
      ),
    onSuccess: async () => {
      setNotice("Memory restored from history.");
      await invalidateArkMemory();
    },
  });

  const summary = asRecord(summaryQ.data);
  const currentMemory = asRecord(summary.current_memory);
  const queueItems = pickRecords(queueQ.data, "items");
  const ledgerEvents = pickRecords(ledgerQ.data, "events");
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
    num(currentMemory.preferences) +
    num(currentMemory.user_data) +
    num(currentMemory.knowledge);
  useEffect(() => {
    if (!showQueueTab && memoryTab === "queue") {
      setMemoryTab("current");
    }
  }, [memoryTab, showQueueTab]);
  const busy =
    approveQueueMutation.isPending ||
    rejectQueueMutation.isPending ||
    rollbackMutation.isPending;
  const firstError =
    summaryQ.error ||
    queueQ.error ||
    ledgerQ.error ||
    approveQueueMutation.error ||
    rejectQueueMutation.error ||
    rollbackMutation.error;
  const memoryTabValue =
    memoryTab === "current" ? 0 : memoryTab === "queue" ? 1 : showQueueTab ? 2 : 1;

  const statItems = [
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
        eyebrow="ArkCore"
        title="ArkMemory"
        descriptionNoWrap
        description={
          <>
            Current facts, preferences, user data, and knowledge.
            <span className="workspace-page-note">
              New memories may take a little time to appear while ArkMemory
              consolidates background signals outside the active chat.
            </span>
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

      <Box className="list-shell stat-strip">
        {statItems.map((item) => (
          <div key={item.label} className="stat-strip-item">
            <span className="stat-strip-label">{item.label}</span>
            <span className="stat-strip-value">{item.value}</span>
            <span className="stat-strip-helper">{item.helper}</span>
          </div>
        ))}
      </Box>

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
          projects={projects}
          activeProjectId={activeProjectId}
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
                Changes, consolidations, and rollbacks. Open Advanced on an item
                when you need technical detail or a restore action.
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
                              {restoreTargetId
                                ? type === "queue_memory_merged" &&
                                  linkedRestoreId
                                  ? "Restores the archived source memory behind this consolidation."
                                  : "Restores the previous memory snapshot recorded for this change."
                                : "No restore action is available for this history item."}
                            </Typography>
                            <Button
                              size="small"
                              variant="outlined"
                              color="warning"
                              disabled={busy || !restoreTargetId}
                              onClick={() =>
                                restoreTargetId
                                  ? rollbackMutation.mutate(restoreTargetId)
                                  : undefined
                              }
                            >
                              {restoreLabel}
                            </Button>
                          </Stack>
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
