import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  Grid as Grid2,
  IconButton,
  Menu,
  MenuItem,
  Stack,
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
import MoreVertIcon from "@mui/icons-material/MoreVert";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import type { BackgroundSessionDetail, BackgroundSessionSummary } from "../types";
import { WorkspacePageHeader, WorkspacePageShell } from "./WorkspacePage";

const REFRESH_MS = 8000;

type NoticeState = { kind: "success" | "error"; text: string } | null;
type DetailTab = "overview" | "work" | "trace";
type JsonRecord = Record<string, unknown>;
type SelectableTask = { id: string; description: string; action: string; status: string };
type SelectableWatcher = {
  id: string;
  description: string;
  poll_action: string;
  status: string;
  history_only: boolean;
};
type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

type SessionFormState = {
  title: string;
  objective: string;
  summary: string;
  current_focus: string;
  waiting_on: string;
  next_expected_action: string;
  working_memory: string;
  preferred_delivery_channel: string;
  status: string;
  task_ids: string[];
  watcher_ids: string[];
};

function emptySessionForm(): SessionFormState {
  return {
    title: "",
    objective: "",
    summary: "",
    current_focus: "",
    waiting_on: "",
    next_expected_action: "",
    working_memory: "",
    preferred_delivery_channel: "",
    status: "active",
    task_ids: [],
    watcher_ids: [],
  };
}

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : {};
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

function toBool(value: unknown): boolean {
  return value === true;
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return "Request failed.";
}

function chipColor(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = status.toLowerCase();
  if (["active", "completed", "triggered"].some((token) => normalized.includes(token))) return "success";
  if (["waiting", "paused", "needs_input", "needs-input"].some((token) => normalized.includes(token))) {
    return "warning";
  }
  if (["failed", "cancelled", "canceled", "stopped"].some((token) => normalized.includes(token))) {
    return "error";
  }
  if (["running", "in_progress"].some((token) => normalized.includes(token))) return "info";
  return "default";
}

function statusLabel(status: string): string {
  const normalized = status.toLowerCase();
  if (normalized === "needs_input") return "Needs input";
  return normalized
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatTimestamp(value?: string | null): string {
  return formatUiDateTime(value, { fallback: "-" });
}

function sessionFormFromDetail(detail: BackgroundSessionDetail): SessionFormState {
  return {
    title: detail.session.title || "",
    objective: detail.session.objective || "",
    summary: detail.session.summary || "",
    current_focus: detail.session.current_focus || "",
    waiting_on: detail.session.waiting_on || "",
    next_expected_action: detail.session.next_expected_action || "",
    working_memory: detail.session_detail.working_memory || "",
    preferred_delivery_channel: detail.session.preferred_delivery_channel || "",
    status: detail.session.status || "active",
    task_ids: detail.session.linked_task_ids || [],
    watcher_ids: detail.session.linked_watcher_ids || [],
  };
}

function sessionCount(session: BackgroundSessionSummary): number {
  return (session.counts?.tasks_total || 0) + (session.counts?.watchers_total || 0);
}

function RowOpsMenu({ actions, ariaLabel = "Row actions" }: { actions: RowMenuAction[]; ariaLabel?: string }) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton size="small" aria-label={ariaLabel} onClick={(e) => setAnchorEl(e.currentTarget)}>
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu anchorEl={anchorEl} open={open} onClose={closeMenu}>
        {actions.map((action, idx) => (
          <MenuItem
            key={`${action.label}-${idx}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={() => {
              closeMenu();
              if (action.disabled) return;
              void action.onClick();
            }}
            sx={
              action.tone === "error"
                ? { color: "error.main" }
                : action.tone === "warning"
                  ? { color: "warning.main" }
                  : undefined
            }
          >
            {action.label}
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

export function BackgroundSessionsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailTab, setDetailTab] = useState<DetailTab>("overview");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [form, setForm] = useState<SessionFormState>(emptySessionForm);
  const [formError, setFormError] = useState<string | null>(null);
  const [notice, setNotice] = useState<NoticeState>(null);

  const sessionsQ = useQuery({
    queryKey: ["background-sessions"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const detailQ = useQuery({
    queryKey: ["background-session-detail", selectedId],
    queryFn: () => api.getBackgroundSession(selectedId || ""),
    enabled: !!selectedId,
    refetchInterval: autoRefresh && selectedId ? REFRESH_MS : false,
  });

  const tasksQ = useQuery({
    queryKey: ["background-session-selectable-tasks"],
    queryFn: () => api.rawGet("/tasks?limit=100"),
    refetchInterval: false,
    staleTime: 30_000,
  });

  const watchersQ = useQuery({
    queryKey: ["background-session-selectable-watchers"],
    queryFn: () => api.rawGet("/watchers"),
    refetchInterval: false,
    staleTime: 30_000,
  });

  const sessions = sessionsQ.data?.sessions || [];

  useEffect(() => {
    if (!sessions.length && selectedId !== null) {
      setSelectedId(null);
    }
  }, [selectedId, sessions]);

  const availableTasks = useMemo<SelectableTask[]>(
    () =>
      pickRecords(tasksQ.data, "tasks").map((item) => ({
        id: str(item.id),
        description: str(item.description, "Task"),
        action: str(item.action, ""),
        status: str(item.status, ""),
      })),
    [tasksQ.data],
  );

  const availableWatchers = useMemo<SelectableWatcher[]>(
    () =>
      pickRecords(watchersQ.data, "watchers")
        .map((item) => ({
          id: str(item.id),
          description: str(item.description, "Watcher"),
          poll_action: str(item.poll_action, ""),
          status: str(item.status, ""),
          history_only: toBool(item.history_only),
        }))
        .filter((item) => !item.history_only),
    [watchersQ.data],
  );

  const actionMutation = useMutation({
    mutationFn: async ({
      kind,
      sessionId,
    }: {
      kind: "pause" | "resume" | "cancel" | "delete";
      sessionId: string;
    }) => {
      if (kind === "pause") return api.pauseBackgroundSession(sessionId);
      if (kind === "resume") return api.resumeBackgroundSession(sessionId);
      if (kind === "cancel") return api.cancelBackgroundSession(sessionId);
      return api.deleteBackgroundSession(sessionId);
    },
    onSuccess: async (_result, variables) => {
      if (variables.kind === "delete" && selectedId === variables.sessionId) {
        setSelectedId(null);
      }
      setNotice({
        kind: "success",
        text:
          variables.kind === "delete"
            ? "Background session deleted."
            : variables.kind === "cancel"
              ? "Background session stopped."
              : variables.kind === "pause"
                ? "Background session paused."
                : "Background session resumed.",
      });
      await queryClient.invalidateQueries({ queryKey: ["background-sessions"] });
      await queryClient.invalidateQueries({ queryKey: ["background-session-detail"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) }),
  });

  const saveMutation = useMutation({
    mutationFn: async (nextForm: SessionFormState) => {
      const payload = {
        title: nextForm.title.trim() || undefined,
        objective: nextForm.objective.trim(),
        summary: nextForm.summary,
        current_focus: nextForm.current_focus,
        waiting_on: nextForm.waiting_on,
        next_expected_action: nextForm.next_expected_action,
        working_memory: nextForm.working_memory,
        preferred_delivery_channel: nextForm.preferred_delivery_channel,
        status: nextForm.status,
      };

      if (!payload.objective) throw new Error("Objective is required.");

      if (editingSessionId) {
        await api.updateBackgroundSession(editingSessionId, payload);
        const existingTaskIds = detailQ.data?.session.linked_task_ids || [];
        const existingWatcherIds = detailQ.data?.session.linked_watcher_ids || [];
        const attachTaskIds = nextForm.task_ids.filter((id) => !existingTaskIds.includes(id));
        const detachTaskIds = existingTaskIds.filter((id) => !nextForm.task_ids.includes(id));
        const attachWatcherIds = nextForm.watcher_ids.filter((id) => !existingWatcherIds.includes(id));
        const detachWatcherIds = existingWatcherIds.filter((id) => !nextForm.watcher_ids.includes(id));

        if (attachTaskIds.length || attachWatcherIds.length) {
          await api.attachBackgroundSessionWork(editingSessionId, {
            task_ids: attachTaskIds,
            watcher_ids: attachWatcherIds,
          });
        }
        if (detachTaskIds.length || detachWatcherIds.length) {
          await api.detachBackgroundSessionWork(editingSessionId, {
            task_ids: detachTaskIds,
            watcher_ids: detachWatcherIds,
          });
        }
        return editingSessionId;
      }

      const created = await api.createBackgroundSession({
        ...payload,
        task_ids: nextForm.task_ids,
        watcher_ids: nextForm.watcher_ids,
      });
      return created.id;
    },
    onSuccess: async (sessionId) => {
      setDialogOpen(false);
      setEditingSessionId(null);
      setForm(emptySessionForm());
      setFormError(null);
      setSelectedId(sessionId);
      setNotice({
        kind: "success",
        text: editingSessionId ? "Background session updated." : "Background session created.",
      });
      await queryClient.invalidateQueries({ queryKey: ["background-sessions"] });
      await queryClient.invalidateQueries({ queryKey: ["background-session-detail"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    },
    onError: (error) => setFormError(errMessage(error)),
  });

  const activeCount = sessions.filter((session) => session.status === "active").length;
  const waitingCount = sessions.filter((session) =>
    ["waiting", "needs_input"].includes(session.status),
  ).length;
  const pausedCount = sessions.filter((session) => session.status === "paused").length;
  const closedCount = sessions.filter((session) =>
    ["completed", "failed", "cancelled"].includes(session.status),
  ).length;

  const openCreateDialog = () => {
    setEditingSessionId(null);
    setForm(emptySessionForm());
    setFormError(null);
    setDialogOpen(true);
  };

  const openEditDialog = (session: BackgroundSessionSummary) => {
    if (!detailQ.data || detailQ.data.session.id !== session.id) return;
    setEditingSessionId(detailQ.data.session.id);
    setForm(sessionFormFromDetail(detailQ.data));
    setFormError(null);
    setDialogOpen(true);
  };

  const selectedSession =
    detailQ.data?.session || sessions.find((session) => session.id === selectedId) || null;

  return (
    <WorkspacePageShell spacing={1.5}>
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {sessionsQ.error ? <Alert severity="error">{errMessage(sessionsQ.error)}</Alert> : null}
      <WorkspacePageHeader
        eyebrow="Operations"
        title="Sessions"
        description="Durable containers for ongoing work across tasks, watchers, and recent runtime history."
        actions={
          <Button variant="contained" onClick={openCreateDialog}>
            New Session
          </Button>
        }
      />
      {/* Compact stat strip */}
      <Box className="list-shell stat-strip">
        {[
          { label: "Active", value: activeCount },
          { label: "Waiting", value: waitingCount },
          { label: "Paused", value: pausedCount },
          { label: "Closed", value: closedCount },
        ].map((item) => (
          <div key={item.label} className="stat-strip-item">
            <span className="stat-strip-label">{item.label}</span>
            <span className="stat-strip-value">{item.value}</span>
          </div>
        ))}
      </Box>
      {/* Sessions table — same pattern as Tasks page */}
      <Box className="list-shell">
        <Typography variant="h6" sx={{
          mb: 1
        }}>
          Session List
        </Typography>
        {sessionsQ.isLoading ? (
          <Box sx={{ py: 6, textAlign: "center" }}>
            <CircularProgress size={28} />
          </Box>
        ) : sessions.length === 0 ? (
          <Box sx={{ py: 5, textAlign: "center" }}>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
                mb: 2
              }}>
              No background sessions yet. Create a session when work should persist beyond one chat turn.
            </Typography>
            <Button variant="outlined" onClick={openCreateDialog}>
              Create your first session
            </Button>
          </Box>
        ) : (
          <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
            <Table size="small" sx={{ minWidth: 860 }}>
              <TableHead>
                <TableRow>
                  <TableCell>Title</TableCell>
                  <TableCell>Objective</TableCell>
                  <TableCell>Status</TableCell>
                  <TableCell>Tasks</TableCell>
                  <TableCell>Watchers</TableCell>
                  <TableCell>Updated</TableCell>
                  <TableCell align="right">Ops</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {sessions.map((session) => {
                  const isPaused = session.status === "paused";
                  const isTerminal = ["completed", "failed", "cancelled"].includes(session.status);
                  const rowActions: RowMenuAction[] = [
                    {
                      label: "View",
                      onClick: () => {
                        setSelectedId(session.id);
                        setDetailTab("overview");
                      },
                    },
                    {
                      label: "Edit",
                      onClick: () => {
                        setSelectedId(session.id);
                        // Need detail loaded first — open edit after a tick
                        setTimeout(() => openEditDialog(session), 200);
                      },
                    },
                    {
                      label: isPaused ? "Resume" : "Pause",
                      disabled: isTerminal,
                      onClick: () =>
                        actionMutation.mutate({
                          kind: isPaused ? "resume" : "pause",
                          sessionId: session.id,
                        }),
                    },
                    {
                      label: "Stop",
                      tone: "warning",
                      disabled: isTerminal,
                      onClick: () =>
                        actionMutation.mutate({ kind: "cancel", sessionId: session.id }),
                    },
                    {
                      label: "Delete",
                      tone: "error",
                      divider: true,
                      onClick: () => {
                        const confirmed = window.confirm(
                          "Delete this background session? Linked tasks and watchers will be detached but not deleted.",
                        );
                        if (!confirmed) return;
                        actionMutation.mutate({ kind: "delete", sessionId: session.id });
                      },
                    },
                  ];

                  return (
                    <TableRow key={session.id}>
                      <TableCell sx={{ maxWidth: 280 }}>
                        <Typography variant="body2" noWrap title={session.title}>
                          {session.title}
                        </Typography>
                      </TableCell>
                      <TableCell sx={{ maxWidth: 420 }}>
                        <Typography variant="body2" noWrap title={session.live_summary || session.objective}>
                          {session.live_summary || session.objective}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Chip size="small" label={statusLabel(session.status)} color={chipColor(session.status)} />
                      </TableCell>
                      <TableCell>{session.counts.tasks_total}</TableCell>
                      <TableCell>{session.counts.watchers_total}</TableCell>
                      <TableCell sx={{ whiteSpace: "nowrap" }} title={formatTimestamp(session.updated_at)}>
                        {formatTimestamp(session.updated_at)}
                      </TableCell>
                      <TableCell align="right">
                        <RowOpsMenu actions={rowActions} ariaLabel="Session options" />
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </Box>
      {/* Session detail dialog — opened via "View" in ops menu */}
      <Dialog open={selectedId != null} onClose={() => setSelectedId(null)} maxWidth="md" fullWidth>
        <DialogTitle sx={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={selectedSession?.title || "Session"}>{selectedSession?.title || "Session"}</DialogTitle>
        <DialogContent>
          {!selectedId || detailQ.isLoading ? (
            <Box sx={{ py: 4, textAlign: "center" }}>
              <CircularProgress size={28} />
            </Box>
          ) : detailQ.error || !selectedSession ? (
            <Alert severity="error">{errMessage(detailQ.error)}</Alert>
          ) : (
            <Stack spacing={1}>
              <Stack
                direction="row"
                spacing={1}
                sx={{
                  flexWrap: "wrap",
                  alignItems: "center"
                }}>
                <Chip
                  size="small"
                  label={statusLabel(selectedSession.status)}
                  color={chipColor(selectedSession.status)}
                />
                <Chip size="small" variant="outlined" label={`${selectedSession.counts.tasks_total} tasks`} />
                <Chip size="small" variant="outlined" label={`${selectedSession.counts.watchers_total} watchers`} />
                <Chip size="small" variant="outlined" label={`${sessionCount(selectedSession)} linked`} />
              </Stack>

              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Updated: {formatTimestamp(selectedSession.updated_at)}
              </Typography>

              <Tabs value={detailTab} onChange={(_event, value: DetailTab) => setDetailTab(value)}>
                <Tab value="overview" label="Overview" />
                <Tab value="work" label="Work" />
                <Tab value="trace" label="Trace" />
              </Tabs>
              <Divider />

              {detailTab === "overview" ? (
                <Stack spacing={1.5}>
                  <Box className="metadata-box">
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>Objective</Typography>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {selectedSession.objective}
                    </Typography>
                  </Box>
                  {selectedSession.summary ? (
                    <Box className="metadata-box">
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>Summary</Typography>
                      <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                        {selectedSession.summary}
                      </Typography>
                    </Box>
                  ) : null}

                  <Grid2 container spacing={1.25}>
                    {[
                      { label: "Current Focus", value: selectedSession.current_focus || "Not set yet." },
                      { label: "Waiting On", value: selectedSession.waiting_on || "Nothing blocking right now." },
                      {
                        label: "Next Expected Action",
                        value: selectedSession.next_expected_action || "No next step recorded yet.",
                      },
                    ].map((item) => (
                      <Grid2 key={item.label} size={{ xs: 12, md: 4 }}>
                        <Box className="metadata-box" sx={{ height: "100%" }}>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>{item.label}</Typography>
                          <Typography variant="body2">{item.value}</Typography>
                        </Box>
                      </Grid2>
                    ))}
                  </Grid2>

                  {detailQ.data?.session_detail.working_memory ? (
                    <Box className="metadata-box">
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>Working memory</Typography>
                      <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                        {detailQ.data.session_detail.working_memory}
                      </Typography>
                    </Box>
                  ) : null}

                  {selectedSession.last_error ? <Alert severity="error">{selectedSession.last_error}</Alert> : null}

                  {(detailQ.data?.missing_links.task_ids.length || detailQ.data?.missing_links.watcher_ids.length) ? (
                    <Alert severity="warning">
                      Some previously linked work is no longer live. Missing tasks:{" "}
                      {detailQ.data?.missing_links.task_ids.length || 0}, missing watchers:{" "}
                      {detailQ.data?.missing_links.watcher_ids.length || 0}.
                    </Alert>
                  ) : null}
                </Stack>
              ) : null}

              {detailTab === "work" ? (
                <Grid2 container spacing={1.5}>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Box className="metadata-box" sx={{ height: "100%" }}>
                      <Typography variant="subtitle2" sx={{ mb: 1 }}>Linked tasks</Typography>
                      {detailQ.data?.linked_tasks.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.linked_tasks.map((task) => (
                            <Box key={task.id} className="micro-surface-list-item">
                              <Stack
                                direction="row"
                                spacing={1}
                                sx={{
                                  justifyContent: "space-between",
                                  alignItems: "center"
                                }}>
                                <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={task.description}>
                                  {task.description}
                                </Typography>
                                <Chip size="small" label={statusLabel(task.status)} color={chipColor(task.status)} />
                              </Stack>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                {task.action || "task"} | {formatTimestamp(task.created_at)}
                              </Typography>
                            </Box>
                          ))}
                        </Stack>
                      ) : (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No tasks linked to this session.
                        </Typography>
                      )}
                    </Box>
                  </Grid2>

                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Box className="metadata-box" sx={{ height: "100%" }}>
                      <Typography variant="subtitle2" sx={{ mb: 1 }}>Linked watchers</Typography>
                      {detailQ.data?.linked_watchers.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.linked_watchers.map((watcher) => (
                            <Box key={watcher.id} className="micro-surface-list-item">
                              <Stack
                                direction="row"
                                spacing={1}
                                sx={{
                                  justifyContent: "space-between",
                                  alignItems: "center"
                                }}>
                                <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={watcher.description}>
                                  {watcher.description}
                                </Typography>
                                <Chip size="small" label={statusLabel(watcher.status)} color={chipColor(watcher.status)} />
                              </Stack>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                {watcher.poll_action || "watcher"} | {formatTimestamp(watcher.created_at)}
                              </Typography>
                            </Box>
                          ))}
                        </Stack>
                      ) : (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No watchers linked to this session.
                        </Typography>
                      )}
                    </Box>
                  </Grid2>
                </Grid2>
              ) : null}

              {detailTab === "trace" ? (
                <Grid2 container spacing={1.5}>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Box className="metadata-box" sx={{ height: "100%" }}>
                      <Typography variant="subtitle2" sx={{ mb: 1 }}>Recent runs</Typography>
                      {detailQ.data?.recent_runs.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.recent_runs.map((run) => (
                            <Box key={run.id} className="micro-surface-list-item">
                              <Stack
                                direction="row"
                                spacing={1}
                                sx={{
                                  justifyContent: "space-between",
                                  alignItems: "center"
                                }}>
                                <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={run.title}>
                                  {run.title}
                                </Typography>
                                <Chip size="small" label={statusLabel(run.status)} color={chipColor(run.status)} />
                              </Stack>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                  display: "block",
                                  mt: 0.5
                                }}>
                                {run.summary}
                              </Typography>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                {formatTimestamp(run.started_at)}
                              </Typography>
                            </Box>
                          ))}
                        </Stack>
                      ) : (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No recorded runs for this session yet.
                        </Typography>
                      )}
                    </Box>
                  </Grid2>

                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Box className="metadata-box" sx={{ height: "100%" }}>
                      <Typography variant="subtitle2" sx={{ mb: 1 }}>Session timeline</Typography>
                      {detailQ.data?.session_detail.events.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.session_detail.events
                            .slice()
                            .reverse()
                            .map((event) => (
                              <Box key={event.id} className="micro-surface-list-item">
                                <Typography variant="body2" sx={{ fontWeight: 600 }}>
                                  {event.summary}
                                </Typography>
                                {event.detail ? (
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      mt: 0.35
                                    }}>
                                    {event.detail}
                                  </Typography>
                                ) : null}
                                <Typography variant="caption" sx={{
                                  color: "text.secondary"
                                }}>
                                  {formatTimestamp(event.at)}
                                </Typography>
                              </Box>
                            ))}
                        </Stack>
                      ) : (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No session events yet.
                        </Typography>
                      )}
                    </Box>
                  </Grid2>
                </Grid2>
              ) : null}
            </Stack>
          )}
        </DialogContent>
      </Dialog>
      {/* Create/Edit form dialog */}
      <Dialog open={dialogOpen} onClose={() => setDialogOpen(false)} maxWidth="md" fullWidth>
        <DialogTitle>{editingSessionId ? "Edit Background Session" : "Create Background Session"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <TextField
              fullWidth
              size="small"
              label="Title"
              value={form.title}
              onChange={(event) => setForm((prev) => ({ ...prev, title: event.target.value }))}
            />
            <TextField
              fullWidth
              multiline
              minRows={2}
              label="Objective"
              value={form.objective}
              onChange={(event) => setForm((prev) => ({ ...prev, objective: event.target.value }))}
            />
            <Grid2 container spacing={1.25}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={2}
                  label="Summary"
                  value={form.summary}
                  onChange={(event) => setForm((prev) => ({ ...prev, summary: event.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={2}
                  label="Current Focus"
                  value={form.current_focus}
                  onChange={(event) => setForm((prev) => ({ ...prev, current_focus: event.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={2}
                  label="Waiting On"
                  value={form.waiting_on}
                  onChange={(event) => setForm((prev) => ({ ...prev, waiting_on: event.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={2}
                  label="Next Expected Action"
                  value={form.next_expected_action}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, next_expected_action: event.target.value }))
                  }
                />
              </Grid2>
            </Grid2>
            <TextField
              fullWidth
              multiline
              minRows={3}
              label="Working Memory"
              value={form.working_memory}
              onChange={(event) => setForm((prev) => ({ ...prev, working_memory: event.target.value }))}
            />
            <Grid2 container spacing={1.25}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Preferred Delivery Channel"
                  value={form.preferred_delivery_channel}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, preferred_delivery_channel: event.target.value }))
                  }
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  select
                  label="Status"
                  value={form.status}
                  disabled={!editingSessionId}
                  onChange={(event) => setForm((prev) => ({ ...prev, status: event.target.value }))}
                >
                  {["draft", "active", "waiting", "needs_input", "paused", "completed", "failed", "cancelled"].map(
                    (status) => (
                      <MenuItem key={status} value={status}>
                        {statusLabel(status)}
                      </MenuItem>
                    ),
                  )}
                </TextField>
              </Grid2>
            </Grid2>

            <Divider />

            <Grid2 container spacing={1.5}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <Box className="metadata-box">
                  <Typography variant="subtitle2" sx={{ mb: 1 }}>
                    Link Tasks
                  </Typography>
                  <Stack spacing={0.4} sx={{ maxHeight: 220, overflowY: "auto" }}>
                    {availableTasks.map((task) => (
                      <FormControlLabel
                        key={task.id}
                        control={
                          <Checkbox
                            checked={form.task_ids.includes(task.id)}
                            onChange={(event) =>
                              setForm((prev) => ({
                                ...prev,
                                task_ids: event.target.checked
                                  ? [...prev.task_ids, task.id]
                                  : prev.task_ids.filter((id) => id !== task.id),
                              }))
                            }
                          />
                        }
                        label={
                          <Box sx={{ minWidth: 0 }}>
                            <Typography variant="body2" noWrap title={task.description}>
                              {task.description}
                            </Typography>
                            <Typography variant="caption" sx={{
                              color: "text.secondary"
                            }}>
                              {task.action || "task"} | {statusLabel(task.status)}
                            </Typography>
                          </Box>
                        }
                      />
                    ))}
                    {!availableTasks.length ? <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>No live tasks available.</Typography> : null}
                  </Stack>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <Box className="metadata-box">
                  <Typography variant="subtitle2" sx={{ mb: 1 }}>
                    Link Watchers
                  </Typography>
                  <Stack spacing={0.4} sx={{ maxHeight: 220, overflowY: "auto" }}>
                    {availableWatchers.map((watcher) => (
                      <FormControlLabel
                        key={watcher.id}
                        control={
                          <Checkbox
                            checked={form.watcher_ids.includes(watcher.id)}
                            onChange={(event) =>
                              setForm((prev) => ({
                                ...prev,
                                watcher_ids: event.target.checked
                                  ? [...prev.watcher_ids, watcher.id]
                                  : prev.watcher_ids.filter((id) => id !== watcher.id),
                              }))
                            }
                          />
                        }
                        label={
                          <Box sx={{ minWidth: 0 }}>
                            <Typography variant="body2" noWrap title={watcher.description}>
                              {watcher.description}
                            </Typography>
                            <Typography variant="caption" sx={{
                              color: "text.secondary"
                            }}>
                              {watcher.poll_action || "watcher"} | {statusLabel(watcher.status)}
                            </Typography>
                          </Box>
                        }
                      />
                    ))}
                    {!availableWatchers.length ? <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>No live watchers available.</Typography> : null}
                  </Stack>
                </Box>
              </Grid2>
            </Grid2>

            {formError ? <Alert severity="error">{formError}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            disabled={saveMutation.isPending}
            onClick={() => {
              setFormError(null);
              saveMutation.mutate(form);
            }}
          >
            {saveMutation.isPending ? "Saving..." : editingSessionId ? "Save Session" : "Create Session"}
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
