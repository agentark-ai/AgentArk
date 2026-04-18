import {
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
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Tab,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { isBackgroundSessionVisibleInUi } from "../lib/backgroundSessions";
import { formatUiDateTime } from "../lib/dateFormat";
import type {
  BackgroundSessionDetail,
  BackgroundSessionSummary,
  BrowserSessionSummary,
} from "../types";
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

function dotColor(status: string): string {
  const normalized = status.toLowerCase();
  if (normalized === "active") return "rgba(57,208,255,0.85)";
  if (normalized === "completed") return "rgba(74,210,157,0.85)";
  if (normalized === "failed") return "rgba(255,100,100,0.85)";
  if (["paused", "waiting", "needs_input"].includes(normalized)) return "rgba(255,191,130,0.85)";
  if (["draft", "cancelled"].includes(normalized)) return "rgba(180,200,220,0.5)";
  return "rgba(180,200,220,0.5)";
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

function browserSessionHandoffUrl(sessionId: string): string {
  return `/ui/browser-handoff/${encodeURIComponent(sessionId)}`;
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

  const browserSessionsQ = useQuery({
    queryKey: ["browser-sessions"],
    queryFn: api.getBrowserSessions,
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

  const sessions = useMemo(
    () => (sessionsQ.data?.sessions || []).filter((session) => isBackgroundSessionVisibleInUi(session)),
    [sessionsQ.data],
  );
  const browserSessions = useMemo<BrowserSessionSummary[]>(
    () => browserSessionsQ.data?.sessions || [],
    [browserSessionsQ.data],
  );

  useEffect(() => {
    if (selectedId !== null && !sessions.some((session) => session.id === selectedId)) {
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

  const browserActionMutation = useMutation({
    mutationFn: async ({
      kind,
      sessionId,
    }: {
      kind: "stop" | "delete";
      sessionId: string;
    }) => {
      if (kind === "stop") return api.stopBrowserSession(sessionId);
      return api.deleteBrowserSession(sessionId);
    },
    onSuccess: async (_result, variables) => {
      setNotice({
        kind: "success",
        text:
          variables.kind === "delete"
            ? "Browser session deleted."
            : "Browser session stopped.",
      });
      await queryClient.invalidateQueries({ queryKey: ["browser-sessions"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-browser-sessions"] });
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
        description="Durable work created through chat and kept here for inspection, pause/resume control, and linked task or watcher management."
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
                maxWidth: 520,
                mx: "auto"
              }}>
              No background sessions yet. Start one from chat when work should stay alive across follow-ups, linked tasks, or watchers. This page is for reviewing and managing those sessions after they exist.
            </Typography>
          </Box>
        ) : (
          <Box>
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
                <Box key={session.id} sx={{ display: "flex", alignItems: "flex-start" }}>
                  <ButtonBase
                    onClick={() => {
                      setSelectedId(session.id);
                      setDetailTab("overview");
                    }}
                    sx={{
                      width: "100%",
                      textAlign: "left",
                      justifyContent: "flex-start",
                      px: 0,
                      py: 1.15,
                      borderBottom: "1px solid",
                      borderColor: "divider",
                      transition: "background 0.15s ease",
                      "&:hover": { background: "rgba(57, 208, 255, 0.04)" },
                      display: "block",
                    }}
                  >
                    {/* First line: dot + title ... status text */}
                    <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                      <Box
                        component="span"
                        sx={{
                          width: 7,
                          height: 7,
                          borderRadius: "50%",
                          flexShrink: 0,
                          bgcolor: dotColor(session.status),
                        }}
                      />
                      <Typography
                        variant="body2"
                        sx={{ fontWeight: 600, flex: 1, minWidth: 0 }}
                        noWrap
                        title={session.title}
                      >
                        {session.title}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", flexShrink: 0 }}
                      >
                        {statusLabel(session.status)}
                      </Typography>
                    </Box>
                    {/* Second line: objective summary */}
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                      noWrap
                      title={session.live_summary || session.objective}
                    >
                      {session.live_summary || session.objective}
                    </Typography>
                    {/* Third line: metadata */}
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                    >
                      Created {formatTimestamp(session.created_at)} &middot; Updated {formatTimestamp(session.updated_at)} &middot; Last activity {formatTimestamp(session.last_activity_at)}
                    </Typography>
                  </ButtonBase>
                  <Box sx={{ flexShrink: 0, pt: 1.15 }}>
                    <RowOpsMenu actions={rowActions} ariaLabel="Session options" />
                  </Box>
                </Box>
              );
            })}
          </Box>
        )}
      </Box>
      {/* Session detail dialog — opened via "View" in ops menu */}
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
              queryClient.invalidateQueries({ queryKey: ["browser-sessions"] })
            }
          >
            Refresh
          </Button>
        </Stack>
        {browserSessionsQ.isLoading ? (
          <Box sx={{ py: 6, textAlign: "center" }}>
            <CircularProgress size={28} />
          </Box>
        ) : browserSessionsQ.error ? (
          <Alert severity="error">{errMessage(browserSessionsQ.error)}</Alert>
        ) : browserSessions.length === 0 ? (
          <Box sx={{ py: 5, textAlign: "center" }}>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
                maxWidth: 520,
                mx: "auto",
              }}
            >
              No active browser sessions.
            </Typography>
          </Box>
        ) : (
          <Box>
            {browserSessions.map((session) => {
              const isTerminal = ["completed", "failed", "interrupted"].includes(session.status);
              const detailLine =
                session.summary ||
                session.question ||
                session.reason ||
                session.page_title ||
                session.page_url ||
                "Live browser session";
              const rowActions: RowMenuAction[] = [
                {
                  label: "Open live handoff",
                  onClick: () => {
                    window.open(browserSessionHandoffUrl(session.id), "_blank", "noopener,noreferrer");
                  },
                },
                {
                  label: "Stop",
                  tone: "warning",
                  disabled: isTerminal,
                  onClick: () =>
                    browserActionMutation.mutate({
                      kind: "stop",
                      sessionId: session.id,
                    }),
                },
                {
                  label: "Delete",
                  tone: "error",
                  divider: true,
                  onClick: () => {
                    const confirmed = window.confirm(
                      "Delete this browser session? This closes the live browser and removes the saved session record.",
                    );
                    if (!confirmed) return;
                    browserActionMutation.mutate({
                      kind: "delete",
                      sessionId: session.id,
                    });
                  },
                },
              ];

              return (
                <Box key={session.id} sx={{ display: "flex", alignItems: "flex-start" }}>
                  <Box
                    sx={{
                      width: "100%",
                      px: 0,
                      py: 1.15,
                      borderBottom: "1px solid",
                      borderColor: "divider",
                      display: "block",
                    }}
                  >
                    <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                      <Box
                        component="span"
                        sx={{
                          width: 7,
                          height: 7,
                          borderRadius: "50%",
                          flexShrink: 0,
                          bgcolor: dotColor(session.status),
                        }}
                      />
                      <Typography
                        variant="body2"
                        sx={{ fontWeight: 600, flex: 1, minWidth: 0 }}
                        noWrap
                        title={session.task_description}
                      >
                        {session.task_description}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", flexShrink: 0 }}
                      >
                        {statusLabel(session.status)}
                      </Typography>
                    </Box>
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                      noWrap
                      title={detailLine}
                    >
                      {detailLine}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                    >
                      Created {formatTimestamp(session.created_at)} &middot; Updated {formatTimestamp(session.updated_at)}
                    </Typography>
                  </Box>
                  <Box sx={{ flexShrink: 0, pt: 1.15 }}>
                    <RowOpsMenu actions={rowActions} ariaLabel="Browser session options" />
                  </Box>
                </Box>
              );
            })}
          </Box>
        )}
      </Box>
      <Dialog
        open={selectedId != null}
        onClose={() => setSelectedId(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-bg-elevated)",
              boxShadow: "0 28px 96px rgba(0,0,0,0.5)",
            },
          },
        }}
      >
        <DialogTitle
          sx={{
            pb: 0.5,
            display: "flex",
            alignItems: "center",
            gap: 1.5,
            borderBottom: "1px solid",
            borderColor: "divider",
          }}
        >
          <Typography
            variant="h6"
            sx={{
              fontWeight: 700,
              flex: 1,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={selectedSession?.title || "Session"}
          >
            {selectedSession?.title || "Session"}
          </Typography>
          {selectedSession ? (
            <Chip
              size="small"
              label={statusLabel(selectedSession.status)}
              color={chipColor(selectedSession.status)}
            />
          ) : null}
        </DialogTitle>
        <DialogContent sx={{ pt: 2 }}>
          {!selectedId || detailQ.isLoading ? (
            <Box sx={{ py: 4, textAlign: "center" }}>
              <CircularProgress size={28} />
            </Box>
          ) : detailQ.error || !selectedSession ? (
            <Alert severity="error">{errMessage(detailQ.error)}</Alert>
          ) : (
            <Stack spacing={1.25}>
              <Box
                sx={{
                  borderRadius: "8px",
                  border: "1px solid var(--surface-border)",
                  background: "var(--micro-surface-bg)",
                  p: 1.45,
                  boxShadow: "inset 0 1px 0 rgba(255,255,255,0.04)",
                }}
              >
                <Stack spacing={1.15}>
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
                      label={statusLabel(selectedSession.status)}
                      color={chipColor(selectedSession.status)}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`${selectedSession.counts.tasks_total} tasks`}
                      sx={{
                        borderColor: "rgba(255,255,255,0.14)",
                        background: "rgba(255,255,255,0.03)",
                      }}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`${selectedSession.counts.watchers_total} watchers`}
                      sx={{
                        borderColor: "rgba(255,255,255,0.14)",
                        background: "rgba(255,255,255,0.03)",
                      }}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`${sessionCount(selectedSession)} linked`}
                      sx={{
                        borderColor: "rgba(255,255,255,0.14)",
                        background: "rgba(255,255,255,0.03)",
                      }}
                    />
                  </Stack>

                  <Grid2 container spacing={1}>
                    <Grid2 size={{ xs: 12, sm: 7 }}>
                      <Box
                        sx={{
                          height: "100%",
                          borderRadius: "8px",
                          border: "1px solid rgba(255,255,255,0.08)",
                          background: "rgba(255,255,255,0.03)",
                          p: 1.15,
                        }}
                      >
                        <Typography variant="caption" sx={{ color: "rgba(188, 198, 212, 0.68)" }}>
                          Objective
                        </Typography>
                        <Typography variant="body1" sx={{ mt: 0.45, fontWeight: 600, lineHeight: 1.45 }}>
                          {selectedSession.objective}
                        </Typography>
                      </Box>
                    </Grid2>
                    <Grid2 size={{ xs: 12, sm: 5 }}>
                      <Box
                        sx={{
                          height: "100%",
                          borderRadius: "8px",
                          border: "1px solid rgba(255,255,255,0.08)",
                          background: "rgba(255,255,255,0.03)",
                          p: 1.15,
                        }}
                      >
                        <Typography variant="caption" sx={{ color: "rgba(188, 198, 212, 0.68)" }}>
                          Updated
                        </Typography>
                        <Typography variant="body2" sx={{ mt: 0.45, color: "rgba(231, 236, 243, 0.78)" }}>
                          {formatTimestamp(selectedSession.updated_at)}
                        </Typography>
                      </Box>
                    </Grid2>
                  </Grid2>
                </Stack>
              </Box>

              <Tabs
                value={detailTab}
                onChange={(_event, value: DetailTab) => setDetailTab(value)}
                sx={{
                  minHeight: 40,
                  "& .MuiTab-root": {
                    minHeight: 40,
                    textTransform: "none",
                    fontWeight: 600,
                  },
                }}
              >
                <Tab value="overview" label="Overview" />
                <Tab value="work" label="Work" />
                <Tab value="trace" label="Trace" />
              </Tabs>
              <Divider />

              {detailTab === "overview" ? (
                <Stack spacing={1.5}>
                  {selectedSession.summary ? (
                    <Box
                      sx={{
                        borderRadius: "8px",
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.025)",
                        p: 1.25,
                      }}
                    >
                      <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>Summary</Typography>
                      <Typography variant="body2" sx={{ mt: 0.7, whiteSpace: "pre-wrap", color: "rgba(231, 236, 243, 0.76)" }}>
                        {selectedSession.summary}
                      </Typography>
                    </Box>
                  ) : null}

                  <Grid2 container spacing={1.25}>
                    {[
                      { label: "Current Focus", value: selectedSession.current_focus || "Not set yet." },
                      { label: "Waiting On", value: selectedSession.waiting_on || "Nothing blocking right now." },
                      { label: "Next Expected Action", value: selectedSession.next_expected_action || "No next step recorded yet." },
                    ].map((item) => (
                      <Grid2 key={item.label} size={{ xs: 12, md: 4 }}>
                        <Box
                          sx={{
                            height: "100%",
                            borderRadius: "8px",
                            border: "1px solid rgba(255,255,255,0.08)",
                            background: "rgba(255,255,255,0.025)",
                            p: 1.15,
                          }}
                        >
                          <Typography variant="caption" sx={{ color: "rgba(188, 198, 212, 0.68)" }}>
                            {item.label}
                          </Typography>
                          <Typography variant="body2" sx={{ mt: 0.5, lineHeight: 1.5 }}>
                            {item.value}
                          </Typography>
                        </Box>
                      </Grid2>
                    ))}
                  </Grid2>

                  {detailQ.data?.session_detail.working_memory ? (
                    <Box
                      sx={{
                        borderRadius: "8px",
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.025)",
                        p: 1.25,
                      }}
                    >
                      <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>Working memory</Typography>
                      <Typography variant="body2" sx={{ mt: 0.75, whiteSpace: "pre-wrap", color: "rgba(231, 236, 243, 0.76)" }}>
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
                    <Box
                      sx={{
                        height: "100%",
                        borderRadius: "8px",
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.025)",
                        p: 1.25,
                      }}
                    >
                      <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", mb: 1 }}>
                        <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>Linked tasks</Typography>
                        <Chip size="small" variant="outlined" label={detailQ.data?.linked_tasks.length || 0} />
                      </Stack>
                      {detailQ.data?.linked_tasks.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.linked_tasks.map((task) => (
                            <Box
                              key={task.id}
                              className="micro-surface-list-item"
                              sx={{
                                borderRadius: "8px",
                                border: "1px solid rgba(255,255,255,0.07)",
                                background: "rgba(255,255,255,0.02)",
                                borderLeft: "3px solid rgba(255,255,255,0.12)",
                              }}
                            >
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
                    <Box
                      sx={{
                        height: "100%",
                        borderRadius: "8px",
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.025)",
                        p: 1.25,
                      }}
                    >
                      <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", mb: 1 }}>
                        <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>Linked watchers</Typography>
                        <Chip size="small" variant="outlined" label={detailQ.data?.linked_watchers.length || 0} />
                      </Stack>
                      {detailQ.data?.linked_watchers.length ? (
                        <Stack spacing={0.9}>
                          {detailQ.data.linked_watchers.map((watcher) => (
                            <Box
                              key={watcher.id}
                              className="micro-surface-list-item"
                              sx={{
                                borderRadius: "8px",
                                border: "1px solid rgba(255,255,255,0.07)",
                                background: "rgba(255,255,255,0.02)",
                                borderLeft: "3px solid rgba(52, 211, 153, 0.7)",
                              }}
                            >
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
          <Button variant="outlined" color="secondary" onClick={() => setDialogOpen(false)}>Cancel</Button>
          <Button
            variant="outlined"
            color="secondary"
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
