import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import {
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import { isStandaloneBackgroundWorkTask } from "../../lib/backgroundSessions";
import type { BackgroundSessionSummary, Task } from "../../types";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  type JsonRecord,
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
} from "./pageHelpers";

const REFRESH_MS = 8000;

type WorkBadge = "Monitor" | "Reminder" | "Recurring" | "Follow-up";

type DeliveryChannelOption = {
  id: string;
  label: string;
};

type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

function formatTimestampForHumans(value: string): {
  label: string;
  tooltip: string;
} {
  const meta = formatUiDateTimeMeta(value, { fallback: value || "-" });
  return { label: meta.label, tooltip: meta.tip };
}

function humanTs(raw: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(raw, { fallback: "-" });
}

function formatDurationFromSeconds(value: unknown): string {
  const total = num(value, -1);
  if (total < 0) return "-";
  const seconds = Math.floor(total);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) {
    return remainingSeconds > 0
      ? `${minutes}m ${remainingSeconds}s`
      : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours < 24) {
    return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return remainingHours > 0 ? `${days}d ${remainingHours}h` : `${days}d`;
}

function dotColor(raw: unknown): string {
  const value = str(raw, "").toLowerCase();
  if (value.includes("active") || value.includes("pending")) {
    return "var(--ui-rgba-57-208-255-850)";
  }
  if (value.includes("completed") || value.includes("triggered")) {
    return "var(--ui-rgba-74-210-157-850)";
  }
  if (value.includes("failed") || value.includes("cancelled")) {
    return "var(--ui-rgba-255-100-100-850)";
  }
  if (value.includes("paused") || value.includes("approval")) {
    return "var(--ui-rgba-255-191-130-850)";
  }
  return "var(--ui-rgba-180-200-220-500)";
}

function statusLabel(raw: unknown): string {
  const value = str(raw, "").trim();
  if (!value) return "-";
  return value.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
}

function statusColor(
  raw: unknown,
): "success" | "warning" | "error" | "default" | "info" {
  const value = str(raw, "").toLowerCase();
  if (value.includes("active") || value.includes("pending")) return "success";
  if (value.includes("paused") || value.includes("approval")) return "warning";
  if (value.includes("triggered") || value.includes("running")) return "info";
  if (
    value.includes("failed") ||
    value.includes("timed") ||
    value.includes("cancelled")
  ) {
    return "error";
  }
  return "default";
}

function terminalStatus(raw: unknown): boolean {
  const value = str(raw, "").toLowerCase();
  return (
    value.includes("completed") ||
    value.includes("failed") ||
    value.includes("cancelled") ||
    value.includes("timed")
  );
}

function watcherConditionSummary(raw: unknown): string {
  const condition = asRecord(raw);
  const description = str(condition.description, "").trim();
  if (description) return description;
  const kind = str(condition.type, "").trim();
  if (kind) return kind.replace(/_/g, " ");
  const entries = Object.entries(condition);
  if (entries.length === 0) return "-";
  return entries[0][0].replace(/_/g, " ");
}

function notificationChannelLabel(raw: unknown): string {
  const value = str(raw, "").trim().toLowerCase();
  if (!value || value === "in_app" || value === "web") return "In-app";
  if (value === "preferred") return "Preferred";
  return value
    .replace(/_/g, " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function taskStatusValue(task: Task | null | undefined): string {
  return str(task?.status, "").trim().toLowerCase();
}

function taskStatusLabel(task: Task | null | undefined): string {
  const value = taskStatusValue(task);
  if (!value) return "Pending";
  if (value.includes("awaitingapproval") || value.includes("awaiting_approval")) {
    return "Awaiting approval";
  }
  if (value.includes("inprogress") || value.includes("in_progress")) return "Running";
  if (value.includes("expired")) return "Needs approval";
  return value.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
}

function taskStatusColor(
  task: Task | null | undefined,
): "success" | "warning" | "error" | "default" | "info" {
  const value = taskStatusValue(task);
  if (value.includes("pending") || value.includes("progress")) return "info";
  if (value.includes("paused") || value.includes("approval")) return "warning";
  if (value.includes("completed")) return "success";
  if (value.includes("failed") || value.includes("cancelled")) return "error";
  return "default";
}

function taskTerminal(task: Task | null | undefined): boolean {
  const value = taskStatusValue(task);
  return (
    value.includes("completed") ||
    value.includes("failed") ||
    value.includes("cancelled")
  );
}

function taskAutomationSessionId(task: Task | null | undefined): string {
  return str(asRecord(asRecord(task?.arguments)._automation).background_session_id, "").trim();
}

function watcherAutomationSessionId(watcher: JsonRecord | null | undefined): string {
  return str(
    asRecord(asRecord(watcher?.poll_arguments)._automation).background_session_id,
    "",
  ).trim();
}

function workBadgeForTask(task: Task | null | undefined): Exclude<WorkBadge, "Monitor"> {
  const cron = str(task?.cron, "").trim();
  const scheduledFor = str(task?.scheduled_for, "").trim();
  const action = str(task?.action, "").trim().toLowerCase();
  if (cron) return "Recurring";
  if (scheduledFor || action === "notify_user" || action === "goal_reminder") {
    return "Reminder";
  }
  return "Follow-up";
}

function workBadgeForSession(
  session: BackgroundSessionSummary,
  linkedTasks: Task[],
): WorkBadge {
  if (
    (session.counts?.watchers_total || 0) > 0 ||
    (session.linked_watcher_ids || []).length > 0
  ) {
    return "Monitor";
  }
  if (linkedTasks.some((task) => str(task.cron, "").trim())) return "Recurring";
  if (
    linkedTasks.some((task) => {
      const action = str(task.action, "").trim().toLowerCase();
      return str(task.scheduled_for, "").trim() || action === "notify_user";
    })
  ) {
    return "Reminder";
  }
  return "Follow-up";
}

function taskMetaLine(task: Task): string {
  const parts = [str(task.action, "").trim() || "task"];
  if (str(task.cron, "").trim()) parts.push(`cron ${str(task.cron, "")}`);
  if (str(task.scheduled_for, "").trim()) {
    parts.push(`scheduled ${formatTimestampForHumans(str(task.scheduled_for, "")).label}`);
  }
  return parts.join(" - ");
}

function watcherMetaLine(watcher: JsonRecord): string {
  const interval = formatDurationFromSeconds(num(watcher.interval_secs, 0));
  const lastPoll = str(watcher.last_poll_at, "").trim()
    ? formatTimestampForHumans(str(watcher.last_poll_at, "")).label
    : "never";
  const lastStatus = watcherLatestStatusLine(watcher);
  const notification = watcherNotificationLine(watcher);
  return `${str(watcher.poll_action, "poller")} - every ${interval} - last poll ${lastPoll} - last status ${lastStatus} - notify ${notificationChannelLabel(
    watcher.notify_channel,
  )}${notification ? ` - ${notification}` : ""}`;
}

function watcherLatestStatusLine(watcher: JsonRecord): string {
  const outcome = str(watcher.last_poll_outcome, "").trim();
  if (outcome) return statusLabel(outcome);
  if (str(watcher.last_error, "").trim() || str(watcher.status_error, "").trim()) {
    return "Error";
  }
  if (!str(watcher.last_poll_at, "").trim()) return "Not run yet";
  return "Completed";
}

function watcherNotificationLine(watcher: JsonRecord): string {
  const attempts = pickRecords(watcher.notification_attempts);
  const latest = attempts[attempts.length - 1];
  if (!latest) return "";
  const channel = notificationChannelLabel(latest.channel);
  const status = toBool(latest.success) ? "sent" : "failed";
  const when = str(latest.attempted_at, "").trim()
    ? formatTimestampForHumans(str(latest.attempted_at, "")).label
    : "unknown time";
  return `${channel} ${status} ${when}`;
}

function payloadText(raw: unknown): string {
  if (raw == null) return "";
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (!trimmed) return "";
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch {
      return trimmed;
    }
  }
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return String(raw);
  }
}

function RowOpsMenu({
  actions,
  ariaLabel = "Row actions",
}: {
  actions: RowMenuAction[];
  ariaLabel?: string;
}) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton
        size="small"
        aria-label={ariaLabel}
        onClick={(event) => {
          event.stopPropagation();
          setAnchorEl(event.currentTarget);
        }}
      >
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu
        anchorEl={anchorEl}
        open={open}
        onClose={closeMenu}
        onClick={(event) => event.stopPropagation()}
      >
        {actions.map((action, index) => (
          <MenuItem
            key={`${action.label}-${index}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={(event) => {
              event.stopPropagation();
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

function BackgroundWorkChildRow({
  badge,
  title,
  status,
  rowStatusColor = "default",
  meta,
  onClick,
}: {
  badge: WorkBadge;
  title: string;
  status: string;
  rowStatusColor?: "success" | "warning" | "error" | "default" | "info";
  meta: string;
  onClick?: () => void;
}) {
  const content = (
    <Stack
      direction="row"
      spacing={1}
      sx={{ alignItems: "center", minWidth: 0, width: "100%" }}
    >
      <Chip size="small" variant="outlined" label={badge} sx={{ flexShrink: 0 }} />
      <Typography
        variant="body2"
        noWrap
        sx={{ fontWeight: 600, minWidth: 0, flex: 1 }}
        title={title}
      >
        {title}
      </Typography>
      <Chip
        size="small"
        color={rowStatusColor}
        variant="outlined"
        label={status}
        sx={{ flexShrink: 0 }}
      />
      <Typography
        variant="caption"
        noWrap
        sx={{ color: "text.secondary", minWidth: 0, flex: 1 }}
        title={meta}
      >
        {meta}
      </Typography>
    </Stack>
  );
  const sx = {
    width: "100%",
    textAlign: "left",
    justifyContent: "flex-start",
    px: 1,
    py: 0.75,
    border: "1px solid var(--surface-border)",
    borderRadius: 1,
    background: "var(--ui-rgba-255-255-255-020)",
  };
  if (!onClick) return <Box sx={sx}>{content}</Box>;
  return (
    <ButtonBase
      onClick={onClick}
      sx={{
        ...sx,
        "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
      }}
    >
      {content}
    </ButtonBase>
  );
}

type WatchersPageProps = {
  autoRefresh: boolean;
};

export default function WatchersPage({ autoRefresh }: WatchersPageProps) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedWatcherId, setSelectedWatcherId] = useState<string | null>(null);
  const [expandedSessionIds, setExpandedSessionIds] = useState<Set<string>>(
    () => new Set(),
  );

  const toggleSession = (sessionId: string) => {
    setExpandedSessionIds((previous) => {
      const next = new Set(previous);
      if (next.has(sessionId)) next.delete(sessionId);
      else next.add(sessionId);
      return next;
    });
  };

  const invalidateBackgroundWork = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] }),
      queryClient.invalidateQueries({ queryKey: ["background-sessions-watcher-links"] }),
      queryClient.invalidateQueries({ queryKey: ["background-sessions"] }),
      queryClient.invalidateQueries({ queryKey: ["background-work-tasks"] }),
      queryClient.invalidateQueries({ queryKey: ["tasks"] }),
      queryClient.invalidateQueries({ queryKey: ["tasks-manager"] }),
      queryClient.invalidateQueries({ queryKey: ["background-session-detail"] }),
    ]);
  };

  const watchersQ = useQuery({
    queryKey: ["watchers-page-watchers"],
    queryFn: () => api.rawGet("/watchers"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const tasksQ = useQuery({
    queryKey: ["background-work-tasks"],
    queryFn: api.getTasks,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const sessionsQ = useQuery({
    queryKey: ["background-sessions-watcher-links"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    staleTime: 10_000,
  });
  const channelsQ = useQuery({
    queryKey: ["available-messaging-channels"],
    queryFn: () => api.rawGet("/channels/available"),
    refetchInterval: false,
    staleTime: 30_000,
  });

  const watcherMutation = useMutation({
    mutationFn: async ({
      kind,
      id,
    }: {
      kind: "pause" | "resume" | "cancel" | "delete" | "run";
      id: string;
    }) => {
      if (kind === "delete") return api.rawDelete(`/watchers/${encodeURIComponent(id)}`);
      if (kind === "run") return api.rawPost(`/watchers/${encodeURIComponent(id)}/run-now`, {});
      return api.rawPost(`/watchers/${encodeURIComponent(id)}/${kind}`, {});
    },
    onSuccess: invalidateBackgroundWork,
    onError: (err) => setError(errMessage(err)),
  });

  const taskMutation = useMutation({
    mutationFn: async ({
      kind,
      id,
    }: {
      kind: "pause" | "resume" | "cancel" | "delete";
      id: string;
    }) => {
      if (kind === "delete") return api.rawDelete(`/tasks/${encodeURIComponent(id)}`);
      return api.rawPost(`/tasks/${encodeURIComponent(id)}/${kind}`, {});
    },
    onSuccess: invalidateBackgroundWork,
    onError: (err) => setError(errMessage(err)),
  });

  const sessionMutation = useMutation({
    mutationFn: async ({
      kind,
      id,
    }: {
      kind: "pause" | "resume" | "cancel" | "delete";
      id: string;
    }) => {
      if (kind === "pause") return api.pauseBackgroundSession(id);
      if (kind === "resume") return api.resumeBackgroundSession(id);
      if (kind === "cancel") return api.cancelBackgroundSession(id);
      return api.deleteBackgroundSession(id);
    },
    onSuccess: invalidateBackgroundWork,
    onError: (err) => setError(errMessage(err)),
  });

  const sessionDeliveryMutation = useMutation({
    mutationFn: async ({ id, channel }: { id: string; channel: string }) =>
      api.updateBackgroundSession(id, { preferred_delivery_channel: channel }),
    onSuccess: invalidateBackgroundWork,
    onError: (err) => setError(errMessage(err)),
  });

  const watchers = useMemo(() => pickRecords(watchersQ.data, "watchers"), [watchersQ.data]);
  const tasks = useMemo(() => tasksQ.data || [], [tasksQ.data]);
  const sessions = useMemo(
    () => sessionsQ.data?.sessions || [],
    [sessionsQ.data],
  );

  const deliveryOptions = useMemo<DeliveryChannelOption[]>(() => {
    const configured = pickRecords(channelsQ.data, "channels")
      .filter((channel) => toBool(channel.configured))
      .map((channel) => ({
        id: str(channel.id, "").trim(),
        label: str(channel.display_name, str(channel.id, "")).trim(),
      }))
      .filter((channel) => channel.id);
    const base = [
      { id: "preferred", label: "Preferred" },
      { id: "in_app", label: "In-app" },
    ];
    const seen = new Set<string>();
    return [...base, ...configured].filter((channel) => {
      if (seen.has(channel.id)) return false;
      seen.add(channel.id);
      return true;
    });
  }, [channelsQ.data]);

  const tasksById = useMemo(() => {
    const map = new Map<string, Task>();
    for (const task of tasks) {
      if (task.id) map.set(task.id, task);
    }
    return map;
  }, [tasks]);

  const watchersById = useMemo(() => {
    const map = new Map<string, JsonRecord>();
    for (const watcher of watchers) {
      const id = str(watcher.id, "").trim();
      if (id) map.set(id, watcher);
    }
    return map;
  }, [watchers]);

  const sessionIds = useMemo(
    () => new Set(sessions.map((session) => session.id).filter(Boolean)),
    [sessions],
  );

  const linkedTaskIds = useMemo(() => {
    const ids = new Set<string>();
    for (const session of sessions) {
      for (const id of session.linked_task_ids || []) ids.add(id);
      for (const task of tasks) {
        if (taskAutomationSessionId(task) === session.id) ids.add(task.id);
      }
    }
    return ids;
  }, [sessions, tasks]);

  const linkedWatcherIds = useMemo(() => {
    const ids = new Set<string>();
    for (const session of sessions) {
      for (const id of session.linked_watcher_ids || []) ids.add(id);
      for (const watcher of watchers) {
        const id = str(watcher.id, "").trim();
        if (id && watcherAutomationSessionId(watcher) === session.id) ids.add(id);
      }
    }
    return ids;
  }, [sessions, watchers]);

  const sessionChildren = useMemo(() => {
    const map = new Map<
      string,
      {
        tasks: Task[];
        watchers: JsonRecord[];
        missingTaskIds: string[];
        missingWatcherIds: string[];
      }
    >();
    for (const session of sessions) {
      const childTasks = new Map<string, Task>();
      const childWatchers = new Map<string, JsonRecord>();
      const missingTaskIds: string[] = [];
      const missingWatcherIds: string[] = [];

      for (const id of session.linked_task_ids || []) {
        const task = tasksById.get(id);
        if (task) childTasks.set(id, task);
        else missingTaskIds.push(id);
      }
      for (const id of session.linked_watcher_ids || []) {
        const watcher = watchersById.get(id);
        if (watcher) childWatchers.set(id, watcher);
        else missingWatcherIds.push(id);
      }
      for (const task of tasks) {
        if (taskAutomationSessionId(task) === session.id) childTasks.set(task.id, task);
      }
      for (const watcher of watchers) {
        const id = str(watcher.id, "").trim();
        if (id && watcherAutomationSessionId(watcher) === session.id) {
          childWatchers.set(id, watcher);
        }
      }
      const liveTasks = Array.from(childTasks.values());
      const liveWatchers = Array.from(childWatchers.values());
      map.set(session.id, {
        tasks: liveTasks,
        watchers: liveWatchers,
        missingTaskIds: liveTasks.length > 0 ? [] : missingTaskIds,
        missingWatcherIds: liveWatchers.length > 0 ? [] : missingWatcherIds,
      });
    }
    return map;
  }, [sessions, tasks, tasksById, watchers, watchersById]);

  const orphanWatchers = useMemo(
    () =>
      watchers.filter((watcher) => {
        const id = str(watcher.id, "").trim();
        const sessionId = watcherAutomationSessionId(watcher);
        return !linkedWatcherIds.has(id) && (!sessionId || !sessionIds.has(sessionId));
      }),
    [linkedWatcherIds, sessionIds, watchers],
  );

  const orphanTasks = useMemo(
    () =>
      tasks.filter((task) => {
        const sessionId = taskAutomationSessionId(task);
        return (
          isStandaloneBackgroundWorkTask(task) &&
          !linkedTaskIds.has(task.id) &&
          (!sessionId || !sessionIds.has(sessionId))
        );
      }),
    [linkedTaskIds, sessionIds, tasks],
  );

  const selectedWatcher = useMemo(
    () =>
      watchers.find((watcher) => str(watcher.id, "") === selectedWatcherId) ?? null,
    [selectedWatcherId, watchers],
  );

  const workBadgeCounts = useMemo(() => {
    let monitors = orphanWatchers.length;
    let reminders = 0;
    let recurring = 0;
    let followUps = 0;
    for (const task of orphanTasks) {
      const badge = workBadgeForTask(task);
      if (badge === "Reminder") reminders += 1;
      else if (badge === "Recurring") recurring += 1;
      else followUps += 1;
    }
    for (const session of sessions) {
      const badge = workBadgeForSession(
        session,
        sessionChildren.get(session.id)?.tasks || [],
      );
      if (badge === "Monitor") monitors += 1;
      else if (badge === "Reminder") reminders += 1;
      else if (badge === "Recurring") recurring += 1;
      else followUps += 1;
    }
    return { monitors, reminders, recurring, followUps };
  }, [orphanTasks, orphanWatchers.length, sessionChildren, sessions]);

  const watcherActions = (watcher: JsonRecord): RowMenuAction[] => {
    const id = str(watcher.id, "").trim();
    const status = str(watcher.status, "").toLowerCase();
    const isActive = status.includes("active");
    const isPaused = status.includes("paused");
    const isHistoryOnly = toBool(watcher.history_only);
    const actions: RowMenuAction[] = [
      {
        label: "Inspect",
        onClick: () => {
          setError(null);
          setSelectedWatcherId(id);
        },
      },
    ];
    if (!isHistoryOnly && isActive) {
      actions.push(
        {
          label: "Run now",
          disabled: watcherMutation.isPending,
          onClick: () => watcherMutation.mutate({ kind: "run", id }),
        },
        {
          label: "Pause",
          disabled: watcherMutation.isPending,
          onClick: () => watcherMutation.mutate({ kind: "pause", id }),
        },
      );
    }
    if (!isHistoryOnly && isPaused) {
      actions.push({
        label: "Resume",
        disabled: watcherMutation.isPending,
        onClick: () => watcherMutation.mutate({ kind: "resume", id }),
      });
    }
    if (!isHistoryOnly && (isActive || isPaused)) {
      actions.push({
        label: "Stop",
        tone: "warning",
        disabled: watcherMutation.isPending,
        onClick: () => watcherMutation.mutate({ kind: "cancel", id }),
      });
    }
    actions.push({
      label: "Delete",
      tone: "error",
      divider: true,
      disabled: watcherMutation.isPending,
      onClick: () => {
        if (!window.confirm("Delete this monitor? This cannot be undone.")) return;
        watcherMutation.mutate({ kind: "delete", id });
      },
    });
    return actions;
  };

  const taskActions = (task: Task): RowMenuAction[] => {
    const status = taskStatusValue(task);
    const isPaused = status.includes("paused");
    const isTerminal = taskTerminal(task);
    const canPause =
      status.includes("pending") ||
      status.includes("awaitingapproval") ||
      status.includes("awaiting_approval");
    const actions: RowMenuAction[] = [];
    if (!isTerminal && isPaused) {
      actions.push({
        label: "Resume",
        disabled: taskMutation.isPending,
        onClick: () => taskMutation.mutate({ kind: "resume", id: task.id }),
      });
    } else if (!isTerminal && canPause) {
      actions.push({
        label: "Pause",
        disabled: taskMutation.isPending,
        onClick: () => taskMutation.mutate({ kind: "pause", id: task.id }),
      });
    }
    if (!isTerminal) {
      actions.push({
        label: "Stop",
        tone: "warning",
        disabled: taskMutation.isPending,
        onClick: () => taskMutation.mutate({ kind: "cancel", id: task.id }),
      });
    }
    actions.push({
      label: "Delete",
      tone: "error",
      divider: actions.length > 0,
      disabled: taskMutation.isPending,
      onClick: () => {
        if (!window.confirm("Delete this task? This cannot be undone.")) return;
        taskMutation.mutate({ kind: "delete", id: task.id });
      },
    });
    return actions;
  };

  const sessionActions = (session: BackgroundSessionSummary): RowMenuAction[] => {
    const isPaused = session.status.toLowerCase() === "paused";
    const isTerminal = terminalStatus(session.status);
    const deliveryActions = deliveryOptions.map<RowMenuAction>((channel, index) => ({
      label: `Change notifications to ${channel.label}`,
      divider: index === 0,
      disabled: sessionDeliveryMutation.isPending,
      onClick: () =>
        sessionDeliveryMutation.mutate({ id: session.id, channel: channel.id }),
    }));
    const actions: RowMenuAction[] = [];
    if (!isTerminal) {
      actions.push(
        {
          label: isPaused ? "Resume" : "Pause",
          disabled: sessionMutation.isPending,
          onClick: () =>
            sessionMutation.mutate({
              kind: isPaused ? "resume" : "pause",
              id: session.id,
            }),
        },
        {
          label: "Stop",
          tone: "warning",
          disabled: sessionMutation.isPending,
          onClick: () => sessionMutation.mutate({ kind: "cancel", id: session.id }),
        },
      );
    }
    actions.push(...deliveryActions);
    actions.push({
      label: "Delete",
      tone: "error",
      divider: true,
      disabled: sessionMutation.isPending,
      onClick: () => {
        const confirmed = window.confirm(
          "Delete this background work? Linked tasks and monitors will be removed.",
        );
        if (!confirmed) return;
        sessionMutation.mutate({ kind: "delete", id: session.id });
      },
    });
    return actions;
  };

  const renderSession = (session: BackgroundSessionSummary) => {
    const children = sessionChildren.get(session.id) || {
      tasks: [],
      watchers: [],
      missingTaskIds: [],
      missingWatcherIds: [],
    };
    const badge = workBadgeForSession(session, children.tasks);
    const childCount =
      children.tasks.length +
      children.watchers.length +
      children.missingTaskIds.length +
      children.missingWatcherIds.length;
    const expanded = expandedSessionIds.has(session.id);
    const watcherNotificationChannels = Array.from(
      new Set(
        children.watchers
          .map((watcher) => notificationChannelLabel(watcher.notify_channel))
          .filter((label) => label && label !== "-"),
      ),
    );
    const notificationLabel = watcherNotificationChannels.length
      ? watcherNotificationChannels.join(", ")
      : notificationChannelLabel(session.preferred_delivery_channel || "preferred");
    return (
      <Box
        key={session.id}
        sx={{ py: 1.15, borderBottom: "1px solid", borderColor: "divider" }}
      >
        <Stack direction="row" spacing={1} sx={{ alignItems: "flex-start" }}>
          <Box
            sx={{
              width: 7,
              height: 7,
              borderRadius: "50%",
              flexShrink: 0,
              mt: 0.85,
              background: dotColor(session.status),
            }}
          />
          <Box sx={{ minWidth: 0, flex: 1 }}>
            <Stack
              direction="row"
              spacing={0.75}
              onClick={() => toggleSession(session.id)}
              sx={{
                alignItems: "center",
                minWidth: 0,
                flexWrap: "wrap",
                cursor: "pointer",
                borderRadius: 1,
                px: 0.5,
                py: 0.35,
                mx: -0.5,
                "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
              }}
            >
              <Typography
                variant="body2"
                noWrap
                sx={{ fontWeight: 700, minWidth: 160, flex: 1 }}
                title={session.title}
              >
                {session.title}
              </Typography>
              <Chip size="small" variant="outlined" label={badge} />
              <Chip
                size="small"
                color={statusColor(session.status)}
                variant="outlined"
                label={statusLabel(session.status)}
              />
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                {formatTimestampForHumans(session.last_activity_at || session.updated_at).label}
              </Typography>
            </Stack>
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", display: "block", mt: 0.25 }}
              title={session.live_summary || session.objective}
            >
              {session.live_summary || session.objective}
            </Typography>
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", display: "block", mt: 0.2 }}
            >
              {`${children.watchers.length} monitor${children.watchers.length === 1 ? "" : "s"} - ${children.tasks.length} task${children.tasks.length === 1 ? "" : "s"} - notifications ${notificationLabel}`}
            </Typography>
            {expanded ? (
              <Stack spacing={0.65} sx={{ mt: 0.75 }}>
                {children.watchers.map((watcher) => (
                  <BackgroundWorkChildRow
                    key={`watcher-${str(watcher.id)}`}
                    badge="Monitor"
                    title={str(watcher.description, "Monitor")}
                    status={statusLabel(watcher.status)}
                    rowStatusColor={statusColor(watcher.status)}
                    meta={watcherMetaLine(watcher)}
                    onClick={() => setSelectedWatcherId(str(watcher.id, ""))}
                  />
                ))}
                {children.tasks.map((task) => (
                  <BackgroundWorkChildRow
                    key={`task-${task.id}`}
                    badge={workBadgeForTask(task)}
                    title={task.description || "Task"}
                    status={taskStatusLabel(task)}
                    rowStatusColor={taskStatusColor(task)}
                    meta={taskMetaLine(task)}
                  />
                ))}
                {children.missingWatcherIds.map((id) => (
                  <BackgroundWorkChildRow
                    key={`missing-watcher-${id}`}
                    badge="Monitor"
                    title={`Missing monitor ${id.slice(0, 8)}`}
                    status="Missing"
                    rowStatusColor="warning"
                    meta="No live or historical monitor record is available for this linked id"
                  />
                ))}
                {children.missingTaskIds.map((id) => (
                  <BackgroundWorkChildRow
                    key={`missing-task-${id}`}
                    badge="Follow-up"
                    title={`Missing task ${id.slice(0, 8)}`}
                    status="Missing"
                    rowStatusColor="warning"
                    meta="Linked record was not found in the task list"
                  />
                ))}
                {!childCount ? (
                  <Typography variant="caption" sx={{ color: "text.secondary" }}>
                    No linked task or monitor records are currently attached.
                  </Typography>
                ) : null}
              </Stack>
            ) : childCount ? (
              <Typography
                variant="caption"
                sx={{ color: "text.secondary", display: "block", mt: 0.35 }}
              >
                Click row to show {childCount} linked item{childCount === 1 ? "" : "s"}.
              </Typography>
            ) : null}
          </Box>
          <Box sx={{ flexShrink: 0 }}>
            <RowOpsMenu actions={sessionActions(session)} ariaLabel="Background work actions" />
          </Box>
        </Stack>
      </Box>
    );
  };

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Operations"
        title="Background Work"
        description="Durable monitors, reminders, recurring checks, and follow-ups. Parent rows manage the whole background session; runtime pollers and tasks stay inside details."
      />
      <Box className="list-shell stat-strip">
        {[
          { label: "Sessions", value: sessions.length },
          { label: "Monitors", value: workBadgeCounts.monitors },
          { label: "Reminders", value: workBadgeCounts.reminders + workBadgeCounts.recurring },
          { label: "Other", value: orphanWatchers.length + orphanTasks.length },
        ].map((s) => (
          <div key={s.label} className="stat-strip-item">
            <span className="stat-strip-label">{s.label}</span>
            <span className="stat-strip-value">{s.value}</span>
          </div>
        ))}
      </Box>

      {sessionsQ.isLoading || watchersQ.isLoading || tasksQ.isLoading ? (
        <Box className="list-shell" sx={{ py: 5, textAlign: "center" }}>
          <Typography variant="body2" sx={{ color: "text.secondary" }}>
            Loading background work...
          </Typography>
        </Box>
      ) : sessions.length === 0 && orphanWatchers.length === 0 && orphanTasks.length === 0 ? (
        <Box className="list-shell" sx={{ py: 8, textAlign: "center" }}>
          <Typography variant="h6" sx={{ color: "text.secondary" }}>
            No background work
          </Typography>
          <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.5 }}>
            Ask AgentArk to remind you later, monitor a condition, or keep a follow-up alive.
          </Typography>
        </Box>
      ) : (
        <>
          <Box className="list-shell" sx={{ minHeight: 0 }}>
            <Stack
              direction="row"
              sx={{ justifyContent: "space-between", alignItems: "center", mb: 1 }}
            >
              <Typography variant="h6">Background Work</Typography>
              <Button size="small" onClick={() => void invalidateBackgroundWork()}>
                Refresh
              </Button>
            </Stack>
            {sessions.length ? (
              sessions.map(renderSession)
            ) : (
              <Typography variant="body2" sx={{ color: "text.secondary", py: 2 }}>
                No parent background sessions yet.
              </Typography>
            )}
          </Box>

          {orphanWatchers.length || orphanTasks.length ? (
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Typography variant="h6" sx={{ mb: 1 }}>
                Other Background Work
              </Typography>
              <Stack spacing={0.75}>
                {orphanWatchers.map((watcher) => {
                  const id = str(watcher.id, "");
                  return (
                    <Stack
                      key={`orphan-watcher-${id}`}
                      direction="row"
                      spacing={0.75}
                      sx={{ alignItems: "center", minWidth: 0 }}
                    >
                      <BackgroundWorkChildRow
                        badge="Monitor"
                        title={str(watcher.description, "Monitor")}
                        status={statusLabel(watcher.status)}
                        rowStatusColor={statusColor(watcher.status)}
                        meta={watcherMetaLine(watcher)}
                        onClick={() => setSelectedWatcherId(id)}
                      />
                      <RowOpsMenu actions={watcherActions(watcher)} ariaLabel="Monitor actions" />
                    </Stack>
                  );
                })}
                {orphanTasks.map((task) => (
                  <Stack
                    key={`orphan-task-${task.id}`}
                    direction="row"
                    spacing={0.75}
                    sx={{ alignItems: "center", minWidth: 0 }}
                  >
                    <BackgroundWorkChildRow
                      badge={workBadgeForTask(task)}
                      title={task.description || "Task"}
                      status={taskStatusLabel(task)}
                      rowStatusColor={taskStatusColor(task)}
                      meta={taskMetaLine(task)}
                    />
                    <RowOpsMenu actions={taskActions(task)} ariaLabel="Task actions" />
                  </Stack>
                ))}
              </Stack>
            </Box>
          ) : null}
        </>
      )}

      <Dialog
        open={selectedWatcher != null}
        onClose={() => setSelectedWatcherId(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-bg-elevated)",
              boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)",
              maxHeight: "86vh",
            },
          },
        }}
      >
        <DialogTitle
          sx={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            borderBottom: "1px solid",
            borderColor: "divider",
            pb: 1.5,
          }}
        >
          <Typography
            variant="h6"
            noWrap
            sx={{ fontWeight: 600, lineHeight: 1.4, flex: 1, minWidth: 0 }}
            title={str(selectedWatcher?.description, "Monitor")}
          >
            {str(selectedWatcher?.description, "Monitor")}
          </Typography>
          <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", ml: 1 }}>
            <Chip size="small" variant="outlined" label="Monitor" />
            <Chip
              size="small"
              label={statusLabel(selectedWatcher?.status)}
              color={statusColor(selectedWatcher?.status)}
            />
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              {str(selectedWatcher?.id, "-").slice(0, 12)}
            </Typography>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack spacing={0.75}>
              {[
                { label: "Action", value: str(selectedWatcher?.poll_action, "-") },
                {
                  label: "Interval",
                  value: toBool(selectedWatcher?.history_only)
                    ? "-"
                    : formatDurationFromSeconds(num(selectedWatcher?.interval_secs, 0)),
                },
                {
                  label: "Timeout",
                  value: toBool(selectedWatcher?.history_only)
                    ? "-"
                    : formatDurationFromSeconds(num(selectedWatcher?.timeout_secs, 0)),
                },
                {
                  label: "Notify",
                  value: notificationChannelLabel(selectedWatcher?.notify_channel),
                },
                { label: "Polls", value: String(num(selectedWatcher?.poll_count, 0)) },
                { label: "Last status", value: watcherLatestStatusLine(selectedWatcher || {}) },
                ...(watcherNotificationLine(selectedWatcher || {})
                  ? [
                      {
                        label: "Latest notify",
                        value: watcherNotificationLine(selectedWatcher || {}),
                      },
                    ]
                  : []),
                {
                  label: "Created",
                  value: humanTs(str(selectedWatcher?.created_at, "-")).label,
                  tip: humanTs(str(selectedWatcher?.created_at, "-")).tip,
                },
                ...(str(selectedWatcher?.last_poll_at, "").trim()
                  ? [
                      {
                        label: "Last poll",
                        value: humanTs(str(selectedWatcher?.last_poll_at, "")).label,
                        tip: humanTs(str(selectedWatcher?.last_poll_at, "")).tip,
                      },
                    ]
                  : []),
              ].map((row) => (
                <Stack
                  key={row.label}
                  direction="row"
                  spacing={1.5}
                  sx={{ alignItems: "baseline" }}
                >
                  <Typography
                    variant="caption"
                    sx={{ color: "text.secondary", minWidth: 74, flexShrink: 0 }}
                  >
                    {row.label}
                  </Typography>
                  <Typography variant="body2" title={(row as { tip?: string }).tip || ""}>
                    {row.value}
                  </Typography>
                </Stack>
              ))}
            </Stack>

            <Box>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                Condition
              </Typography>
              <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                {watcherConditionSummary(selectedWatcher?.condition)}
              </Typography>
            </Box>

            {str(selectedWatcher?.on_trigger, "").trim() ? (
              <Box>
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  On trigger
                </Typography>
                <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                  {str(selectedWatcher?.on_trigger, "-")}
                </Typography>
              </Box>
            ) : null}

            {str(selectedWatcher?.last_error, "").trim() ||
            str(selectedWatcher?.status_error, "").trim() ? (
              <Alert severity="error" variant="outlined" sx={{ py: 0.5 }}>
                <Typography
                  variant="body2"
                  sx={{
                    fontFamily: "monospace",
                    fontSize: "0.8rem",
                    wordBreak: "break-word",
                  }}
                >
                  {str(selectedWatcher?.last_error, "").trim() ||
                    str(selectedWatcher?.status_error, "").trim()}
                </Typography>
              </Alert>
            ) : null}

            {payloadText(selectedWatcher?.last_result) ? (
              <Box>
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  Latest poll result
                </Typography>
                <Typography
                  component="pre"
                  variant="body2"
                  sx={{
                    mt: 0.5,
                    mb: 0,
                    p: 1,
                    maxHeight: 220,
                    overflow: "auto",
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                    fontSize: "0.75rem",
                    background: "var(--ui-rgba-0-0-0-300)",
                    borderRadius: 1,
                  }}
                >
                  {payloadText(selectedWatcher?.last_result)}
                </Typography>
              </Box>
            ) : null}

            {payloadText(selectedWatcher?.trigger_result) ? (
              <Box>
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  Trigger payload
                </Typography>
                <Typography
                  component="pre"
                  variant="body2"
                  sx={{
                    mt: 0.5,
                    mb: 0,
                    p: 1,
                    maxHeight: 220,
                    overflow: "auto",
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                    fontSize: "0.75rem",
                    background: "var(--ui-rgba-0-0-0-300)",
                    borderRadius: 1,
                  }}
                >
                  {payloadText(selectedWatcher?.trigger_result)}
                </Typography>
              </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSelectedWatcherId(null)}>Close</Button>
        </DialogActions>
      </Dialog>

      {watchersQ.error || sessionsQ.error || tasksQ.error || error ? (
        <Alert severity="error">
          {error ||
            errMessage(watchersQ.error) ||
            errMessage(sessionsQ.error) ||
            errMessage(tasksQ.error)}
        </Alert>
      ) : null}
    </WorkspacePageShell>
  );
}
