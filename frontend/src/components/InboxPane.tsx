import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Stack,
  Typography,
} from "@mui/material";
import { useMemo } from "react";
import { formatUiDateTime } from "../lib/dateFormat";
import type { Notification, Task } from "../types";

const ACTIVE_TASK_STALE_MS = 24 * 60 * 60 * 1000;

type Props = {
  tasks: Task[];
  notifications: Notification[];
  onNavigateToView: (view: string, replace?: boolean) => void;
};

function taskStatusKey(task: Task): string {
  return String(task?.status || "").toLowerCase();
}

function formatStatus(task: Task): string {
  const value = taskStatusKey(task);
  if (value.includes("awaitingapproval")) return "Awaiting approval";
  if (value.includes("paused")) return "Paused";
  if (value.includes("inprogress")) return "Running";
  if (value.includes("failed")) return "Failed";
  if (value.includes("completed")) return "Completed";
  return value || "Pending";
}

function formatWhen(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "-" });
}

function isFreshInProgressTask(task: Task): boolean {
  const status = taskStatusKey(task);
  if (!status.includes("inprogress")) return false;
  const createdAt = Date.parse(String(task?.created_at || ""));
  if (Number.isNaN(createdAt)) return true;
  return Date.now() - createdAt <= ACTIVE_TASK_STALE_MS;
}

function notificationTargetView(notification: Notification): string {
  const text = `${notification.title || ""} ${notification.body || ""} ${notification.source || ""}`.toLowerCase();
  if (text.includes("arkpulse")) return "arkpulse";
  if (text.includes("watcher")) return "status";
  if (text.includes("trace")) return "trace";
  if (text.includes("task")) return "tasks";
  return "settings";
}

export function InboxPane({ tasks, notifications, onNavigateToView }: Props) {
  const waitingTasks = useMemo(
    () =>
      tasks.filter((task) => {
        const status = taskStatusKey(task);
        return status.includes("awaitingapproval") || status.includes("paused");
      }),
    [tasks]
  );
  const runningTasks = useMemo(
    () => tasks.filter((task) => isFreshInProgressTask(task)).slice(0, 6),
    [tasks]
  );
  const failedTasks = useMemo(
    () => tasks.filter((task) => taskStatusKey(task).includes("failed")).slice(0, 6),
    [tasks]
  );
  const unreadNotifications = useMemo(
    () => notifications.filter((notification) => !notification.read).slice(0, 8),
    [notifications]
  );

  return (
    <Box className="inbox-shell">
      <Box className="inbox-hero">
        <Typography variant="overline" className="workspace-shell-kicker">
          Inbox
        </Typography>
        <Typography variant="h4" sx={{ fontWeight: 700, letterSpacing: 0, mb: 0.45 }}>
          Human input, blocked work, and alerts.
        </Typography>
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
            maxWidth: 860
          }}>
          Keep approvals, failures, pauses, and unread alerts together. The agent can keep running in chat while
          the inbox stays focused on what needs your attention.
        </Typography>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{
            flexWrap: "wrap",
            mt: 1.2
          }}>
          <Chip size="small" color="warning" label={`${waitingTasks.length} waiting`} />
          <Chip size="small" color="info" label={`${runningTasks.length} running`} />
          <Chip size="small" color={failedTasks.length > 0 ? "error" : "default"} label={`${failedTasks.length} failed`} />
          <Chip size="small" color={unreadNotifications.length > 0 ? "warning" : "default"} label={`${unreadNotifications.length} unread alerts`} />
        </Stack>
      </Box>
      <Box className="inbox-grid">
        <Card className="workspace-side-card">
          <CardContent sx={{ p: 1.5 }}>
            <Stack spacing={1}>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Waiting for you
              </Typography>
              {waitingTasks.length === 0 ? (
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  No paused or approval-gated tasks right now.
                </Typography>
              ) : (
                waitingTasks.slice(0, 6).map((task) => (
                  <Box key={task.id} className="action-row">
                    <Stack spacing={0.5}>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Chip size="small" variant="outlined" color="warning" label={formatStatus(task)} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={task.description}>
                          {task.description || "Task"}
                        </Typography>
                      </Stack>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Created {formatWhen(task.created_at)}
                      </Typography>
                    </Stack>
                  </Box>
                ))
              )}
              <Button variant="outlined" size="small" onClick={() => onNavigateToView("tasks")} sx={{ alignSelf: "flex-start", textTransform: "none" }}>
                Open tasks
              </Button>
            </Stack>
          </CardContent>
        </Card>

        <Card className="workspace-side-card">
          <CardContent sx={{ p: 1.5 }}>
            <Stack spacing={1}>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Runs to watch
              </Typography>
              {runningTasks.length === 0 && failedTasks.length === 0 ? (
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  Nothing urgent in the run queue.
                </Typography>
              ) : (
                [...runningTasks, ...failedTasks].slice(0, 8).map((task) => (
                  <Box key={task.id} className="action-row">
                    <Stack spacing={0.5}>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Chip
                          size="small"
                          variant="outlined"
                          color={taskStatusKey(task).includes("failed") ? "error" : "info"}
                          label={formatStatus(task)}
                        />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={task.description}>
                          {task.description || "Task"}
                        </Typography>
                      </Stack>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Created {formatWhen(task.created_at)}
                      </Typography>
                    </Stack>
                  </Box>
                ))
              )}
              <Stack direction="row" spacing={1}>
                <Button variant="outlined" size="small" onClick={() => onNavigateToView("chat")} sx={{ textTransform: "none" }}>
                  Open chat
                </Button>
                <Button variant="text" size="small" onClick={() => onNavigateToView("trace")} sx={{ textTransform: "none" }}>
                  Trace
                </Button>
              </Stack>
            </Stack>
          </CardContent>
        </Card>

        <Card className="workspace-side-card">
          <CardContent sx={{ p: 1.5 }}>
            <Stack spacing={1}>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Unread alerts
              </Typography>
              {unreadNotifications.length === 0 ? (
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  Inbox is clear.
                </Typography>
              ) : (
                unreadNotifications.map((notification) => (
                  <Box key={notification.id} className="action-row">
                    <Stack spacing={0.5}>
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap"
                        }}>
                        <Chip
                          size="small"
                          color={
                            (notification.level || "").toLowerCase() === "critical" ||
                            (notification.level || "").toLowerCase() === "error"
                              ? "error"
                              : "warning"
                          }
                          label={notification.level || "Alert"}
                        />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={notification.title || notification.body}>
                          {notification.title || "Notification"}
                        </Typography>
                      </Stack>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "-webkit-box",
                          WebkitLineClamp: 2,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden"
                        }}>
                        {notification.body}
                      </Typography>
                      <Stack direction="row" spacing={1} sx={{
                        alignItems: "center"
                      }}>
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          {formatWhen(notification.created_at)}
                        </Typography>
                        <Button
                          size="small"
                          variant="text"
                          onClick={() => onNavigateToView(notificationTargetView(notification))}
                          sx={{ textTransform: "none", minWidth: 0, p: 0 }}
                        >
                          Open
                        </Button>
                      </Stack>
                    </Stack>
                  </Box>
                ))
              )}
            </Stack>
          </CardContent>
        </Card>
      </Box>
    </Box>
  );
}
