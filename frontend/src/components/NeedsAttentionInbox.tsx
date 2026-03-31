import {
  Alert,
  Badge,
  Box,
  Button,
  Card,
  CardContent,
  Stack,
  Typography
} from "@mui/material";
import CheckCircleOutlineRoundedIcon from "@mui/icons-material/CheckCircleOutlineRounded";
import WarningAmberRoundedIcon from "@mui/icons-material/WarningAmberRounded";
import type { Notification, Task } from "../types";

type AttentionItem = {
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

function buildItems(
  tasks: Task[],
  notifications: Notification[],
  securityLogs: Array<{ event_type: string; severity: string; message: string }>,
  settingsLoaded: boolean,
  hasLlmConfigured: boolean
): AttentionItem[] {
  const items: AttentionItem[] = [];

  // Setup nudge: no LLM model pool configured
  if (settingsLoaded && !hasLlmConfigured) {
    items.push({
      id: "__setup_llm",
      kind: "setup",
      title: "Set up your AI model",
      detail: "No LLM model is configured yet. Go to Settings > LLM Config to get started."
    });
  }

  // Tasks awaiting approval
  for (const t of tasks) {
    if (isTestArtifactTask(t)) continue;
    const s = String(t?.status || "").toLowerCase();
    if (s.includes("awaitingapproval")) {
      items.push({
        id: t.id,
        kind: "approval",
        title: t.description || "Task needs approval",
      });
    }
  }

  // Failed tasks
  for (const t of tasks) {
    if (isTestArtifactTask(t)) continue;
    const s = String(t?.status || "").toLowerCase();
    if (s.includes("failed")) {
      items.push({
        id: t.id,
        kind: "failed",
        title: t.description || "Task failed",
      });
    }
  }

  // Critical security alerts
  for (const log of securityLogs) {
    const sev = (log.severity || "").toLowerCase();
    if (sev === "high" || sev === "critical") {
      items.push({
        id: `sec_${log.event_type}_${log.message?.slice(0, 20)}`,
        kind: "security",
        title: log.message || `Security: ${log.event_type}`,
        detail: `Severity: ${log.severity}`,
      });
    }
  }

  // Critical unread notifications
  for (const n of notifications) {
    if (!n.read && (n.level === "error" || n.level === "critical")) {
      items.push({
        id: `notif_${n.id}`,
        kind: "security",
        title: n.title || "Alert",
        detail: n.body?.slice(0, 80),
        targetView: notificationTargetView(n),
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
  const items = buildItems(tasks, notifications, securityLogs, settingsLoaded, hasLlmConfigured);
  const count = items.length;

  return (
    <Card className="attention-card" sx={{ alignSelf: "stretch", height: "100%" }}>
      <CardContent sx={{ p: 1.55, height: "100%" }}>
        <Stack direction="row" alignItems="center" spacing={1} mb={0.45}>
          <WarningAmberRoundedIcon
            sx={{ color: count > 0 ? "rgba(255, 167, 38, 0.9)" : "rgba(155, 180, 214, 0.4)", fontSize: 20 }}
          />
          <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>
            Decision Queue
          </Typography>
          {count > 0 ? (
            <Badge badgeContent={count} color="warning" />
          ) : null}
        </Stack>
        <Typography variant="body2" color="text.secondary" sx={{ mb: count > 0 ? 1.25 : 0.9 }}>
          Blocking approvals, failed runs, security alerts, and setup gaps that require an operator.
        </Typography>

        {count === 0 ? (
          <Box className="empty-state" sx={{ py: 3 }}>
            <CheckCircleOutlineRoundedIcon
              sx={{ fontSize: 36, color: "rgba(20, 241, 149, 0.6)" }}
            />
            <Typography variant="body2" color="text.secondary" sx={{ fontWeight: 600 }}>
              Operator queue is clear.
            </Typography>
          </Box>
        ) : (
          <Stack spacing={0.85}>
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
                          letterSpacing: "0.08em",
                          textTransform: "uppercase",
                        }}
                      >
                        {meta.label}
                      </Box>
                      <Typography variant="body2" fontWeight={700} title={item.title}>
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
                        variant="text"
                        size="small"
                        onClick={() => onNavigate(item.targetView || "settings")}
                        sx={{ textTransform: "none", minWidth: 56 }}
                      >
                        View
                      </Button>
                    )}
                  </Stack>
                </Stack>
              </Box>
            )})}

            {count >= 5 ? (
              <Button
                size="small"
                onClick={() => onNavigate("tasks")}
                sx={{ textTransform: "none", alignSelf: "flex-start", mt: 0.5 }}
              >
                Open full task queue
              </Button>
            ) : null}
          </Stack>
        )}
      </CardContent>
    </Card>
  );
}
