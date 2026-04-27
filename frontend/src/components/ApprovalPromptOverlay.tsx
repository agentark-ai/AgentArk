import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import type { Task } from "../types";

type ApprovalTask = Task & {
  arguments?: Record<string, unknown>;
};

type ApprovalCard = {
  id: string;
  title: string;
  summary: string;
  reason: string;
  riskLevel: string;
  riskScore: string;
  source: string;
  createdAt: string;
};

type Props = {
  tasks: Task[];
  busyTaskId?: string | null;
  errorMessage?: string | null;
  onApprove: (id: string, comment?: string) => void;
  onReject: (id: string, comment?: string) => void;
  onOpenTasks: () => void;
};

const UNAVAILABLE_APPROVAL_DESCRIPTION = "Older task details unavailable";

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function normalizeTaskStatus(status: unknown): string {
  const compact = str(status, "")
    .toLowerCase()
    .replace(/[^a-z]/g, "");
  if (compact.includes("awaitingapproval")) return "awaiting_approval";
  return compact;
}

function buildApprovalCard(task: ApprovalTask): ApprovalCard | null {
  if (normalizeTaskStatus(task.status) !== "awaiting_approval") return null;
  const argumentsRecord = asRecord(task.arguments);
  const approval = asRecord(argumentsRecord._approval);
  const description = str(task.description, "").trim();
  const riskScoreRaw = approval.risk_score;
  const riskScore =
    typeof riskScoreRaw === "number" && Number.isFinite(riskScoreRaw)
      ? String(Math.round(riskScoreRaw))
      : str(riskScoreRaw, "").trim();
  const hasDisplayDetails =
    Boolean(str(approval.title, "").trim()) ||
    Boolean(str(approval.summary, "").trim()) ||
    Boolean(str(approval.reason, "").trim()) ||
    Boolean(str(approval.risk_level, "").trim()) ||
    Boolean(riskScore) ||
    Boolean(str(approval.source, "").trim());
  if (description === UNAVAILABLE_APPROVAL_DESCRIPTION && !hasDisplayDetails) {
    return null;
  }
  return {
    id: str(task.id, ""),
    title: str(approval.title, str(task.description, "Approval needed")).trim() || "Approval needed",
    summary: str(approval.summary, "").trim(),
    reason: str(approval.reason, "").trim(),
    riskLevel: str(approval.risk_level, "").trim(),
    riskScore,
    source: str(approval.source, "").trim(),
    createdAt: str(task.created_at, "").trim()
  };
}

function sortNewestFirst(left: ApprovalCard, right: ApprovalCard): number {
  const leftTs = Date.parse(left.createdAt || "");
  const rightTs = Date.parse(right.createdAt || "");
  return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
}

export function ApprovalPromptOverlay({
  tasks,
  busyTaskId,
  errorMessage,
  onApprove,
  onReject,
  onOpenTasks
}: Props) {
  const [dismissedIds, setDismissedIds] = useState<string[]>([]);
  const [comment, setComment] = useState("");

  const approvals = useMemo(
    () =>
      tasks
        .map((task) => buildApprovalCard(task as ApprovalTask))
        .filter((task): task is ApprovalCard => Boolean(task))
        .sort(sortNewestFirst),
    [tasks]
  );

  useEffect(() => {
    setDismissedIds((current) => current.filter((id) => approvals.some((task) => task.id === id)));
  }, [approvals]);

  const activeApproval = approvals.find((task) => !dismissedIds.includes(task.id)) ?? null;

  useEffect(() => {
    setComment("");
  }, [activeApproval?.id]);

  if (!activeApproval) return null;

  const remainingCount = approvals.filter((task) => !dismissedIds.includes(task.id)).length;
  const busy = busyTaskId === activeApproval.id;

  return (
    <Box
      sx={{
        position: "fixed",
        right: { xs: 12, md: 22 },
        bottom: { xs: 12, md: 22 },
        width: { xs: "calc(100vw - 24px)", sm: 430 },
        maxWidth: "calc(100vw - 24px)",
        zIndex: 1450,
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-255-179-71-240)",
        background: "linear-gradient(165deg, var(--ui-rgba-24-15-6-960), var(--ui-rgba-12-18-33-950))",
        boxShadow: "0 24px 56px var(--ui-rgba-0-0-0-460), 0 0 0 1px var(--ui-rgba-255-179-71-080)",
        backdropFilter: "blur(20px)",
        WebkitBackdropFilter: "blur(20px)",
        overflow: "hidden"
      }}
    >
      <Stack spacing={1.25} sx={{ p: 1.5 }}>
        <Stack
          direction="row"
          spacing={1}
          sx={{
            alignItems: "flex-start",
            justifyContent: "space-between"
          }}>
          <Stack
            direction="row"
            spacing={1.1}
            sx={{
              alignItems: "center",
              minWidth: 0
            }}>
            <Box
              sx={{
                width: 34,
                height: 34,
                borderRadius: "8px",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                background: "var(--ui-rgba-255-179-71-140)",
                color: "var(--ui-rgba-255-210-140-950)",
                flexShrink: 0
              }}
            >
              <NotificationsActiveRoundedIcon fontSize="small" />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="subtitle2" sx={{ fontWeight: 700, color: "var(--ui-rgba-255-241-224-960)" }}>
                Approval Needed
              </Typography>
              <Typography variant="caption" sx={{ color: "var(--ui-rgba-255-219-174-720)" }}>
                {remainingCount === 1
                  ? "A task is waiting for your decision."
                  : `${remainingCount} tasks are waiting for your decision.`}
              </Typography>
            </Box>
          </Stack>
          <IconButton
            size="small"
            onClick={() => setDismissedIds((current) => [...current, activeApproval.id])}
            sx={{ color: "var(--ui-rgba-255-219-174-600)" }}
            aria-label="Dismiss approval popup"
          >
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </Stack>

        <Stack direction="row" spacing={0.75} useFlexGap sx={{
          flexWrap: "wrap"
        }}>
          {activeApproval.riskLevel ? (
            <Chip
              size="small"
              label={`Risk: ${activeApproval.riskLevel}`}
              sx={{
                background: "var(--ui-rgba-255-179-71-120)",
                color: "var(--ui-rgba-255-224-184-920)",
                border: "1px solid var(--ui-rgba-255-179-71-180)"
              }}
            />
          ) : null}
          {activeApproval.riskScore ? (
            <Chip
              size="small"
              label={`Score ${activeApproval.riskScore}`}
              sx={{
                background: "var(--ui-rgba-120-174-255-080)",
                color: "var(--ui-rgba-207-226-255-900)",
                border: "1px solid var(--ui-rgba-120-174-255-120)"
              }}
            />
          ) : null}
          {activeApproval.source ? (
            <Chip
              size="small"
              variant="outlined"
              label={activeApproval.source}
              sx={{
                color: "var(--ui-rgba-197-214-238-820)",
                borderColor: "var(--ui-rgba-116-153-202-180)"
              }}
            />
          ) : null}
        </Stack>

        <Box
          sx={{
            borderRadius: "8px",
            p: 1.25,
            background: "var(--ui-rgba-255-255-255-025)",
            border: "1px solid var(--ui-rgba-255-255-255-050)"
          }}
        >
          <Typography variant="body2" sx={{ fontWeight: 700, color: "var(--ui-rgba-241-248-255-950)" }}>
            {activeApproval.title}
          </Typography>
          {activeApproval.summary ? (
            <Typography variant="body2" sx={{ mt: 0.75, color: "var(--ui-rgba-210-223-242-820)" }}>
              {activeApproval.summary}
            </Typography>
          ) : null}
          {activeApproval.reason ? (
            <Typography variant="caption" sx={{ mt: 1, display: "block", color: "var(--ui-rgba-255-215-167-720)" }}>
              Why it asked: {activeApproval.reason}
            </Typography>
          ) : null}
        </Box>

        {errorMessage ? (
          <Alert severity="error" sx={{ py: 0.25 }}>
            {errorMessage}
          </Alert>
        ) : null}

        <TextField
          size="small"
          label="Comment"
          value={comment}
          onChange={(event) => setComment(event.target.value)}
          placeholder="Optional note for the agent"
          multiline
          minRows={2}
          disabled={busy}
          sx={{
            "& .MuiInputBase-root": {
              color: "var(--ui-rgba-241-248-255-950)",
              background: "var(--ui-rgba-255-255-255-030)"
            },
            "& .MuiInputLabel-root": { color: "var(--ui-rgba-255-219-174-720)" },
            "& .MuiOutlinedInput-notchedOutline": { borderColor: "var(--ui-rgba-255-255-255-120)" }
          }}
        />

        <Stack direction={{ xs: "column", sm: "row" }} spacing={0.9}>
          <Button
            variant="contained"
            color="success"
            onClick={() => onApprove(activeApproval.id, comment)}
            disabled={busy}
            sx={{ textTransform: "none", flex: 1 }}
          >
            {busy ? "Working..." : "Approve"}
          </Button>
          <Button
            variant="outlined"
            color="warning"
            onClick={() => onReject(activeApproval.id, comment)}
            disabled={busy}
            sx={{ textTransform: "none", flex: 1 }}
          >
            Reject
          </Button>
        </Stack>

        <Stack
          direction="row"
          spacing={1}
          sx={{
            justifyContent: "space-between",
            alignItems: "center"
          }}>
          <Button
            size="small"
            onClick={onOpenTasks}
            sx={{ textTransform: "none", color: "var(--ui-rgba-197-214-238-850)" }}
          >
            Open Tasks
          </Button>
          <Button
            size="small"
            onClick={() => setDismissedIds((current) => [...current, activeApproval.id])}
            sx={{ textTransform: "none", color: "var(--ui-rgba-255-219-174-700)" }}
          >
            Later
          </Button>
        </Stack>
      </Stack>
    </Box>
  );
}
