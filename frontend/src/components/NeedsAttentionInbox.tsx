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
import { TASK_RETRY_CONTROLS_ENABLED } from "../lib/featureFlags";
import type { Notification, Task } from "../types";

export type AttentionItem = {
  id: string;
  kind: "approval" | "input" | "failed" | "security" | "setup";
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
        accent: "var(--ui-rgba-255-194-87-960)",
        defaultDetail: "An operator decision is blocking execution.",
      };
    case "input":
      return {
        label: "Input",
        accent: "var(--ui-rgba-97-208-255-960)",
        defaultDetail: "Execution is paused until missing input is provided.",
      };
    case "failed":
      return {
        label: "Failure",
        accent: "var(--ui-rgba-255-123-123-950)",
        defaultDetail: "A run degraded and may need intervention.",
      };
    case "setup":
      return {
        label: "Setup",
        accent: "var(--ui-rgba-97-208-255-960)",
        defaultDetail: "Core capability setup is incomplete.",
      };
    default:
      return {
        label: "Alert",
        accent: "var(--ui-rgba-255-151-115-940)",
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

type SecurityLog = {
  event_type: string;
  severity: string;
  message: string;
  source?: string;
  created_at?: string;
};

type Props = {
  tasks: Task[];
  notifications: Notification[];
  securityLogs: SecurityLog[];
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

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function isInternalWebChatTask(task: Task): boolean {
  const args = asRecord(task.arguments);
  return (
    String(task.action || "").trim().toLowerCase() === "chat_request" &&
    String(args._origin || "").trim().toLowerCase() === "chat" &&
    String(args.channel || "").trim().toLowerCase() === "web"
  );
}

function taskNeedsInput(task: Task): boolean {
  const raw = task.result;
  const text = typeof raw === "string" ? raw.trim().toLowerCase() : "";
  if (text.startsWith("__input_needed__:")) return true;
  const payload = asRecord(raw);
  const kind = String(payload.kind || "").trim().toLowerCase();
  return kind === "input_needed" || kind === "input-needed" || kind === "workflow_inputs";
}

function securityLogIsOperatorActionable(log: SecurityLog): boolean {
  const severity = String(log.severity || "").toLowerCase();
  if (severity !== "high" && severity !== "critical") return false;
  const eventType = String(log.event_type || "").toLowerCase();
  const message = String(log.message || "").toLowerCase();
  if (eventType === "capability_correlation") return false;
  if (message.includes("runtime capability correlation")) return false;
  return true;
}

function notificationIsOperatorActionable(notification: Notification): boolean {
  if (notification.read) return false;
  const level = String(notification.level || "").toLowerCase();
  if (level !== "error" && level !== "critical") return false;
  const hay = [
    notification.title,
    notification.body,
    notification.source,
    notification.metadata?.source,
  ]
    .map((value) => String(value || "").toLowerCase())
    .join(" ");
  if (hay.includes("runtime capability correlation")) return false;
  if (hay.includes("semantic router")) return false;
  if (hay.includes("run stream not found")) return false;
  return true;
}

export function buildAttentionItems(
  tasks: Task[],
  notifications: Notification[],
  securityLogs: SecurityLog[],
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
    if (isInternalWebChatTask(task)) continue;
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
    if (isInternalWebChatTask(task)) continue;
    const status = String(task?.status || "").toLowerCase();
    if (taskNeedsInput(task) || status.includes("paused")) {
      items.push({
        id: task.id,
        kind: "input",
        title: task.description || "Task needs input",
      });
    }
  }

  for (const task of tasks) {
    if (isTestArtifactTask(task)) continue;
    if (isInternalWebChatTask(task)) continue;
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
    if (securityLogIsOperatorActionable(log)) {
      items.push({
        id: `sec_${log.event_type}_${log.message?.slice(0, 20)}`,
        kind: "security",
        title: log.message || `Security: ${log.event_type}`,
        detail: `Severity: ${log.severity}`,
      });
    }
  }

  for (const notification of notifications) {
    if (notificationIsOperatorActionable(notification)) {
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
    if (isInternalWebChatTask(task)) return false;
    const status = String(task?.status || "").toLowerCase();
    return status.includes("awaitingapproval") || status.includes("paused") || taskNeedsInput(task);
  }).length;
  const failedCount = tasks.filter(
    (task) => !isInternalWebChatTask(task) && String(task?.status || "").toLowerCase().includes("failed")
  ).length;
  const unreadAlerts = notifications.filter(notificationIsOperatorActionable).length;

  return (
    <Card className="attention-card mission-panel mission-panel--adaptive" data-tour-target="overview-attention">
      <CardContent sx={{ p: 1.3, display: "flex", flexDirection: "column" }}>
        <Stack spacing={1.15} className="mission-panel-content">
          <Box>
            <Stack
              direction="row"
              spacing={1}
              sx={{
                alignItems: "center",
                mb: 0.45
              }}>
              <WarningAmberRoundedIcon
                sx={{ color: count > 0 ? "var(--ui-rgba-255-167-38-900)" : "var(--ui-rgba-155-180-214-400)", fontSize: 20 }}
              />
              <Typography variant="body1" sx={{ flex: 1, fontWeight: 700 }}>
                Needs Attention
              </Typography>
              {count > 0 ? <Badge badgeContent={count} color="warning" /> : null}
            </Stack>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              One queue for approvals, pauses, failures, urgent alerts, and setup gaps that require an operator.
            </Typography>
          </Box>

          <Stack direction="row" spacing={0.75} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Chip size="small" color={waitingCount > 0 ? "warning" : "default"} label={`${waitingCount} waiting`} />
            <Chip size="small" color={failedCount > 0 ? "error" : "default"} label={`${failedCount} failed`} />
            <Chip size="small" color={unreadAlerts > 0 ? "warning" : "default"} label={`${unreadAlerts} unread alerts`} />
          </Stack>

          {count === 0 ? (
            <Box className="empty-state mission-empty-copy" sx={{ py: 3 }}>
              <CheckCircleOutlineRoundedIcon sx={{ fontSize: 36, color: "var(--ui-rgba-20-241-149-600)" }} />
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                  fontWeight: 600
                }}>
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
                      background: "var(--ui-rgba-255-255-255-020)",
                    }}
                  >
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{
                        justifyContent: "space-between",
                        alignItems: { xs: "flex-start", sm: "flex-start" }
                      }}>
                      <Box sx={{ flex: 1, minWidth: 0 }}>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                            alignItems: "center",
                            mb: 0.35
                          }}>
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
                          <Typography variant="body2" title={item.title} className="mission-title-clamp" sx={{
                            fontWeight: 700
                          }}>
                            {item.title}
                          </Typography>
                        </Stack>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block",
                            lineHeight: 1.45
                          }}>
                          {item.detail || meta.defaultDetail}
                        </Typography>
                      </Box>

                      <Stack
                        direction="row"
                        spacing={0.6}
                        sx={{
                          flexShrink: 0,
                          pt: { sm: 0.2 }
                        }}>
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
                        ) : item.kind === "failed" && TASK_RETRY_CONTROLS_ENABLED ? (
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

          <Stack direction="row" spacing={0.75} useFlexGap className="mission-panel-footer" sx={{
            flexWrap: "wrap"
          }}>
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
