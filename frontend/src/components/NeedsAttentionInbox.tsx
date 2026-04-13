import {
  Alert,
  Badge,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Stack,
  Typography,
} from "@mui/material";
import CheckCircleOutlineRoundedIcon from "@mui/icons-material/CheckCircleOutlineRounded";
import WarningAmberRoundedIcon from "@mui/icons-material/WarningAmberRounded";
import type { Notification, Task } from "../types";

export type AttentionItem = {
  id: string;
  kind: "approval" | "failed" | "security" | "setup";
  title: string;
  detail?: string;
  targetView?: string;
};

function attentionMeta(kind: AttentionItem["kind"]): {
  label: string;
  accent: string;
  defaultDetail: string;
} {
  switch (kind) {
    case "approval":
      return {
        label: "Approval",
        accent: "rgba(255, 194, 87, 0.96)",
        defaultDetail: "An operator decision is blocking execution.",
      };
    case "failed":
      return {
        label: "Failure",
        accent: "rgba(255, 123, 123, 0.95)",
        defaultDetail: "A run degraded and may need intervention.",
      };
    case "setup":
      return {
        label: "Setup",
        accent: "rgba(97, 208, 255, 0.96)",
        defaultDetail: "Core capability setup is incomplete.",
      };
    default:
      return {
        label: "Alert",
        accent: "rgba(255, 151, 115, 0.94)",
        defaultDetail: "A system alert needs operator review.",
      };
  }
}

function notificationTargetView(notification: Notification): string {
  const title = String(notification.title || "").toLowerCase();
  const body = String(notification.body || "").toLowerCase();
  const source = String(notification.source || notification.metadata?.source || "").toLowerCase();
  const hay = `${title} ${body} ${source}`;
  if (hay.includes("arkpulse")) return "arkpulse";
  return "settings";
}

type Props = {
  tasks: Task[];
  notifications: Notification[];
  securityLogs: Array<{ event_type: string; severity: string; message: string }>;
  settingsLoaded: boolean;
  hasLlmConfigured: boolean;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  onRetry: (id: string) => void;
  onNavigate: (view: string) => void;
  approving: boolean;
  rejecting: boolean;
  retrying: boolean;
};

function isTestArtifactTask(task: Task): boolean {
  const desc = String(task?.description || "")
    .toLowerCase()
    .replace(/\u2014/g, "-")
    .trim();
  if (!desc.includes("safe to delete")) return false;
  return desc.includes("e2e test task") || desc.includes("integration test task");
}

export function buildAttentionItems(
  tasks: Task[],
  notifications: Notification[],
  securityLogs: Array<{ event_type: string; severity: string; message: string }>,
  settingsLoaded: boolean,
  hasLlmConfigured: boolean
): AttentionItem[] {
  const items: AttentionItem[] = [];

  if (settingsLoaded && !hasLlmConfigured) {
    items.push({
      id: "__setup_llm",
      kind: "setup",
      title: "Set up your AI model",
      detail: "No LLM model is configured yet. Go to Settings > Models to get started.",
    });
  }

  for (const task of tasks) {
    if (isTestArtifactTask(task)) continue;
    const status = String(task?.status || "").toLowerCase();
    if (status.includes("awaitingapproval")) {
      items.push({
        id: task.id,
        kind: "approval",
        title: task.description || "Task needs approval",
      });
    }
  }

  for (const task of tasks) {
    if (isTestArtifactTask(task)) continue;
    const status = String(task?.status || "").toLowerCase();
    if (status.includes("failed")) {
      items.push({
        id: task.id,
        kind: "failed",
        title: task.description || "Task failed",
      });
    }
  }

  for (const log of securityLogs) {
    const severity = (log.severity || "").toLowerCase();
    if (severity === "high" || severity === "critical") {
      items.push({
        id: `sec_${log.event_type}_${log.message?.slice(0, 20)}`,
        kind: "security",
        title: log.message || `Security: ${log.event_type}`,
        detail: `Severity: ${log.severity}`,
      });
    }
  }

  for (const notification of notifications) {
    if (!notification.read && (notification.level === "error" || notification.level === "critical")) {
      items.push({
        id: `notif_${notification.id}`,
        kind: "security",
        title: notification.title || "Alert",
        detail: notification.body?.slice(0, 80),
        targetView: notificationTargetView(notification),
      });
    }
  }

  return items.slice(0, 6);
}

export function NeedsAttentionInbox({
  tasks,
  notifications,
  securityLogs,
  settingsLoaded,
  hasLlmConfigured,
  onApprove,
  onReject,
  onRetry,
  onNavigate,
  approving,
  rejecting,
  retrying,
}: Props) {
  const items = buildAttentionItems(tasks, notifications, securityLogs, settingsLoaded, hasLlmConfigured);
  const count = items.length;
  const waitingCount = tasks.filter((task) => {
    const status = String(task?.status || "").toLowerCase();
    return status.includes("awaitingapproval") || status.includes("paused");
  }).length;
  const failedCount = tasks.filter((task) => String(task?.status || "").toLowerCase().includes("failed")).length;
  const unreadAlerts = notifications.filter((notification) => !notification.read).length;

  return (
    <Card className="attention-card mission-panel mission-panel--adaptive" data-tour-target="overview-attention">
      <CardContent sx={{ p: 1.3, display: "flex", flexDirection: "column" }}>
        <Stack spacing={1.15} className="mission-panel-content">
          <Box>
            <Stack direction="row" alignItems="center" spacing={1} mb={0.45}>
              <WarningAmberRoundedIcon
                sx={{ color: count > 0 ? "rgba(255, 167, 38, 0.9)" : "rgba(155, 180, 214, 0.4)", fontSize: 20 }}
              />
              <Typography variant="body1" sx={{ flex: 1, fontWeight: 700 }}>
                Needs Attention
              </Typography>
              {count > 0 ? <Badge badgeContent={count} color="warning" /> : null}
            </Stack>
            <Typography variant="body2" color="text.secondary">
              One queue for approvals, pauses, failures, urgent alerts, and setup gaps that require an operator.
            </Typography>
          </Box>

          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
            <Chip size="small" color={waitingCount > 0 ? "warning" : "default"} label={`${waitingCount} waiting`} />
            <Chip size="small" color={failedCount > 0 ? "error" : "default"} label={`${failedCount} failed`} />
            <Chip size="small" color={unreadAlerts > 0 ? "warning" : "default"} label={`${unreadAlerts} unread alerts`} />
          </Stack>

          {count === 0 ? (
            <Box className="empty-state mission-empty-copy" sx={{ py: 3 }}>
              <CheckCircleOutlineRoundedIcon sx={{ fontSize: 36, color: "rgba(20, 241, 149, 0.6)" }} />
              <Typography variant="body2" color="text.secondary" sx={{ fontWeight: 600 }}>
                Operator queue is clear.
              </Typography>
            </Box>
          ) : (
            <Stack spacing={0.85} className="mission-panel-section">
              {items.map((item) => {
                const meta = attentionMeta(item.kind);
                return (
                  <Box
                    key={item.id}
                    className="action-row"
                    sx={{
                      p: "10px 12px",
                      borderLeft: `3px solid ${meta.accent}`,
                      background: "linear-gradient(180deg, rgba(8, 18, 34, 0.76), rgba(6, 14, 28, 0.72))",
                    }}
                  >
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      justifyContent="space-between"
                      alignItems={{ xs: "flex-start", sm: "flex-start" }}
                    >
                      <Box sx={{ flex: 1, minWidth: 0 }}>
                        <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" alignItems="center" sx={{ mb: 0.35 }}>
                          <Box
                            sx={{
                              px: 0.8,
                              py: 0.2,
                              borderRadius: 999,
                              border: `1px solid ${meta.accent}`,
                              color: meta.accent,
                              fontSize: "0.68rem",
                              fontWeight: 700,
                              letterSpacing: 0,
                              textTransform: "uppercase",
                            }}
                          >
                            {meta.label}
                          </Box>
                          <Typography variant="body2" fontWeight={700} title={item.title} className="mission-title-clamp">
                            {item.title}
                          </Typography>
                        </Stack>
                        <Typography variant="caption" color="text.secondary" sx={{ display: "block", lineHeight: 1.45 }}>
                          {item.detail || meta.defaultDetail}
                        </Typography>
                      </Box>

                      <Stack direction="row" spacing={0.6} flexShrink={0} sx={{ pt: { sm: 0.2 } }}>
                        {item.kind === "approval" ? (
                          <>
                            <Button
                              variant="contained"
                              size="small"
                              color="success"
                              disabled={approving}
                              onClick={() => onApprove(item.id)}
                              sx={{ minWidth: 78, textTransform: "none" }}
                            >
                              Approve
                            </Button>
                            <Button
                              variant="outlined"
                              size="small"
                              color="warning"
                              disabled={rejecting}
                              onClick={() => onReject(item.id)}
                              sx={{ minWidth: 68, textTransform: "none" }}
                            >
                              Reject
                            </Button>
                          </>
                        ) : item.kind === "failed" ? (
                          <Button
                            variant="outlined"
                            size="small"
                            disabled={retrying}
                            onClick={() => onRetry(item.id)}
                            sx={{ textTransform: "none", minWidth: 68 }}
                          >
                            Retry
                          </Button>
                        ) : item.kind === "setup" ? (
                          <Button
                            variant="contained"
                            size="small"
                            onClick={() => onNavigate("settings")}
                            sx={{ textTransform: "none", minWidth: 68 }}
                          >
                            Set Up
                          </Button>
                        ) : (
                          <Button
                            variant="outlined"
                            size="small"
                            onClick={() => onNavigate(item.targetView || "settings")}
                            sx={{ textTransform: "none", minWidth: 62 }}
                          >
                            View
                          </Button>
                        )}
                      </Stack>
                    </Stack>
                  </Box>
                );
              })}
            </Stack>
          )}

          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" className="mission-panel-footer">
            <Button variant="contained" size="small" onClick={() => onNavigate("tasks")} sx={{ textTransform: "none" }}>
              Open task queue
            </Button>
            <Button size="small" variant="outlined" onClick={() => onNavigate("trace")} sx={{ textTransform: "none" }}>
              Open trace
            </Button>
          </Stack>
        </Stack>
      </CardContent>
    </Card>
  );
}
