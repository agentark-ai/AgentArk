import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import OpenInFullRoundedIcon from "@mui/icons-material/OpenInFullRounded";
import ShieldOutlinedIcon from "@mui/icons-material/ShieldOutlined";
import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ApprovalLogEntry, Task } from "../types";

type ApprovalTask = Task & {
  arguments?: Record<string, unknown>;
};

type ApprovalDecisionTarget = {
  kind: "task" | "direct_chat";
  id: string;
};

type ApprovalStep = {
  actionName: string;
  detail: string;
  argumentsPreview?: unknown;
};

type ApprovalCard = {
  key: string;
  id: string;
  target: ApprovalDecisionTarget;
  title: string;
  summary: string;
  reason: string;
  actionName: string;
  sourceLabel: string;
  ruleName: string;
  riskLevel: string;
  riskScore: string;
  scopeLabel: string;
  requestedAt: string;
  expiresAt: string;
  status: "pending" | "expired";
  canApprove: boolean;
  canReject: boolean;
  canDismiss: boolean;
  steps: ApprovalStep[];
  preview: unknown;
};

type Props = {
  tasks: Task[];
  approvalLogs?: ApprovalLogEntry[];
  busyTargetKey?: string | null;
  errorMessage?: string | null;
  onDecide: (
    target: ApprovalDecisionTarget,
    decision: "approve" | "reject",
    comment?: string,
  ) => void;
  onDismiss: (target: ApprovalDecisionTarget, comment?: string) => void;
  onOpenTasks: () => void;
  onOpenChat?: () => void;
  hiddenTargetKeys?: readonly string[];
};

const UNAVAILABLE_APPROVAL_DESCRIPTION = "Older task details unavailable";

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asRecords(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.map(asRecord).filter((item) => Object.keys(item).length > 0) : [];
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function finiteNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function safeJsonRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "string") return asRecord(value);
  try {
    return asRecord(JSON.parse(value));
  } catch {
    return {};
  }
}

function normalizeCompactStatus(status: unknown): string {
  return str(status, "")
    .toLowerCase()
    .replace(/[^a-z]/g, "");
}

function normalizeTaskApprovalStatus(status: unknown): "pending" | "expired" | "other" {
  const compact = normalizeCompactStatus(status);
  if (compact.includes("awaitingapproval")) return "pending";
  if (compact.includes("expiredneedsreapproval")) return "expired";
  return "other";
}

function normalizeApprovalLogStatus(status: unknown): "pending" | "expired" | "other" {
  const compact = normalizeCompactStatus(status);
  if (compact === "pending") return "pending";
  if (compact === "expired") return "expired";
  return "other";
}

function valueLooksSensitive(key: string | null): boolean {
  if (!key) return false;
  const compact = key.toLowerCase().replace(/[^a-z0-9]/g, "");
  return [
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "accesskey",
    "privatekey",
    "authorization",
    "cookie",
    "credential",
  ].some((needle) => compact.includes(needle));
}

function truncateText(text: string, max = 420): string {
  if (text.length <= max) return text;
  return `${text.slice(0, max - 1)}...`;
}

function redactedPreviewValue(key: string | null, value: unknown, depth = 0): unknown {
  if (valueLooksSensitive(key)) return ".redacted";
  if (value == null) return value;
  if (typeof value === "string") return truncateText(value);
  if (typeof value === "number" || typeof value === "boolean") return value;
  if (depth >= 4) {
    if (Array.isArray(value)) return `[${value.length} item${value.length === 1 ? "" : "s"}]`;
    const record = asRecord(value);
    if (Object.keys(record).length > 0) {
      const count = Object.keys(record).length;
      return `{${count} field${count === 1 ? "" : "s"}}`;
    }
    return value;
  }
  if (Array.isArray(value)) {
    const preview = value
      .slice(0, 8)
      .map((item) => redactedPreviewValue(null, item, depth + 1));
    if (value.length > preview.length) preview.push(`+${value.length - preview.length} more`);
    return preview;
  }
  const record = asRecord(value);
  const output: Record<string, unknown> = {};
  const keys = Object.keys(record).sort();
  for (const entryKey of keys.slice(0, 18)) {
    output[entryKey] = redactedPreviewValue(entryKey, record[entryKey], depth + 1);
  }
  if (keys.length > 18) output._omitted = keys.length - 18;
  return output;
}

function withoutApprovalEnvelope(args: Record<string, unknown>): Record<string, unknown> {
  const output: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) {
    if (key === "_approval") continue;
    output[key] = value;
  }
  return output;
}

function approvalSteps(value: unknown): ApprovalStep[] {
  const steps: ApprovalStep[] = [];
  for (const item of asRecords(value)) {
    const actionName = str(
      item.action_name,
      str(item.actionName, str(item.title, "")),
    ).trim();
    if (!actionName) continue;
    const argumentsPreview =
      item.arguments_preview ??
      item.argumentsPreview ??
      item.arguments ??
      item.args;
    steps.push({
      actionName,
      detail: str(item.summary, str(item.reason, str(item.description, ""))).trim(),
      ...(argumentsPreview === undefined ? {} : { argumentsPreview }),
    });
  }
  return steps;
}

function formattedRiskScore(value: unknown): string {
  const numeric = finiteNumber(value);
  if (numeric == null) return str(value, "").trim();
  return numeric > 0 && numeric < 10 ? numeric.toFixed(1) : String(Math.round(numeric));
}

function approvalMetadataTitle(approval: Record<string, unknown>, fallback: string): string {
  return (
    str(approval.title, "").trim() ||
    str(approval.summary, "").trim() ||
    fallback ||
    "Approval required"
  );
}

function buildTaskApprovalCard(task: ApprovalTask): ApprovalCard | null {
  const status = normalizeTaskApprovalStatus(task.status);
  if (status === "other") return null;
  const args = asRecord(task.arguments);
  const approval = asRecord(args._approval);
  const description = str(task.description, "").trim();
  const hasDisplayDetails =
    description !== UNAVAILABLE_APPROVAL_DESCRIPTION ||
    Boolean(str(approval.title, "").trim()) ||
    Boolean(str(approval.summary, "").trim()) ||
    Boolean(str(approval.reason, "").trim()) ||
    Boolean(str(approval.risk_level, "").trim()) ||
    Boolean(str(approval.source, "").trim());
  if (!hasDisplayDetails) return null;

  const riskScore = formattedRiskScore(approval.risk_score);
  const actionName = str(
    approval.action_name,
    str(approval.actionName, str(task.action, "")),
  ).trim();
  return {
    key: `task:${task.id}`,
    id: str(task.id, ""),
    target: { kind: "task", id: str(task.id, "") },
    title: approvalMetadataTitle(approval, description || "Approval required"),
    summary: str(approval.summary, "").trim(),
    reason: str(approval.reason, "").trim(),
    actionName: actionName || "task",
    sourceLabel: str(approval.source, "").trim(),
    ruleName: str(approval.rule_name, str(approval.ruleName, "")).trim(),
    riskLevel: str(approval.risk_level, str(approval.riskLevel, "")).trim(),
    riskScore,
    scopeLabel: str(approval.scope, str(approval.channel, "")).trim(),
    requestedAt: str(task.created_at, "").trim(),
    expiresAt: str(approval.expires_at, str(approval.expiresAt, "")).trim(),
    status,
    canApprove: true,
    canReject: true,
    canDismiss: true,
    steps: approvalSteps(approval.steps),
    preview: redactedPreviewValue(null, withoutApprovalEnvelope(args)),
  };
}

function buildApprovalLogCard(entry: ApprovalLogEntry): ApprovalCard | null {
  const status = normalizeApprovalLogStatus(entry.status);
  if (status === "other") return null;
  const payload = safeJsonRecord(entry.arguments);
  const calls = asRecords(payload.calls);
  const steps = calls.map((call) => ({
    actionName: str(call.action_name, str(call.actionName, "action")).trim() || "action",
    detail: "",
    argumentsPreview: redactedPreviewValue(null, call.arguments ?? call.args ?? {}),
  }));
  const firstStep = steps[0] ?? null;
  const expiresAt = str(payload.expires_at, str(payload.expiresAt, "")).trim();
  const expiredByTime =
    status === "pending" &&
    Boolean(expiresAt) &&
    Number.isFinite(Date.parse(expiresAt)) &&
    Date.parse(expiresAt) <= Date.now();
  const effectiveStatus = expiredByTime ? "expired" : status;
  const requestChannel = str(payload.request_channel, str(payload.channel, "")).trim();
  const conversationId = str(payload.conversation_id, str(payload.conversationId, "")).trim();
  const actionName =
    calls.length > 1
      ? "action_chain"
      : firstStep?.actionName || str(entry.action_name, "action").trim();
  const supportsDirectDecision = calls.length > 0 && Boolean(expiresAt);
  if (!supportsDirectDecision) return null;
  const canApprove = effectiveStatus === "pending";
  const canReject = effectiveStatus === "pending" || effectiveStatus === "expired";
  const canDismiss = true;

  return {
    key: `direct_chat:${entry.id}`,
    id: entry.id,
    target: { kind: "direct_chat", id: entry.id },
    title:
      calls.length > 1
        ? `${calls.length} actions need approval`
        : `Run ${actionName || "action"}`,
    summary: str(payload.summary, "").trim(),
    reason: str(payload.reason, "").trim(),
    actionName: actionName || "action",
    sourceLabel: requestChannel,
    ruleName: str(entry.rule_name, "").trim(),
    riskLevel: str(payload.risk_level, str(payload.riskLevel, "")).trim(),
    riskScore: formattedRiskScore(payload.risk_score ?? payload.riskScore),
    scopeLabel: [requestChannel, conversationId ? "conversation" : ""]
      .filter(Boolean)
      .join(" . "),
    requestedAt: str(payload.requested_at, str(entry.requested_at, "")).trim(),
    expiresAt,
    status: effectiveStatus,
    canApprove,
    canReject,
    canDismiss,
    steps,
    preview:
      calls.length === 1
        ? steps[0]?.argumentsPreview
        : redactedPreviewValue(null, {
            step_count: calls.length,
            actions: steps.map((step) => step.actionName),
          }),
  };
}

function sortNewestFirst(left: ApprovalCard, right: ApprovalCard): number {
  const priority = (card: ApprovalCard) => {
    if (card.canApprove) return 0;
    if (card.canReject) return 1;
    return 2;
  };
  const priorityDelta = priority(left) - priority(right);
  if (priorityDelta !== 0) return priorityDelta;
  const leftTs = Date.parse(left.requestedAt || "");
  const rightTs = Date.parse(right.requestedAt || "");
  return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
}

function targetKey(target: ApprovalDecisionTarget): string {
  return `${target.kind}:${target.id}`;
}

function formatDateTime(raw: string): string {
  if (!raw) return "Not provided";
  const date = new Date(raw);
  if (!Number.isFinite(date.getTime())) return raw;
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatTtl(raw: string, status: ApprovalCard["status"]): string {
  if (status === "expired") return "Expired";
  if (!raw) return "No expiry";
  const expiresAt = Date.parse(raw);
  if (!Number.isFinite(expiresAt)) return raw;
  const remainingMs = expiresAt - Date.now();
  if (remainingMs <= 0) return "Expired";
  const minutes = Math.floor(remainingMs / 60_000);
  if (minutes < 1) return "Under 1 min";
  if (minutes < 60) return `${minutes} min remaining`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes ? `${hours}h ${restMinutes}m remaining` : `${hours}h remaining`;
}

function previewText(value: unknown): string {
  if (value == null) return "Not provided";
  if (typeof value === "string") return value || "Not provided";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function shortId(value: string): string {
  if (value.length <= 12) return value;
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
}

function statusTone(card: ApprovalCard): {
  label: string;
  color: string;
  border: string;
  background: string;
} {
  if (card.status === "expired") {
    if (card.canApprove) {
      return {
        label: "REAPPROVAL",
        color: "var(--ui-rgba-255-194-87-960)",
        border: "var(--ui-rgba-255-194-87-680)",
        background: "var(--ui-rgba-255-194-87-080)",
      };
    }
    return {
      label: "EXPIRED",
      color: "var(--ui-rgba-255-151-115-960)",
      border: "var(--ui-rgba-255-151-115-560)",
      background: "var(--ui-rgba-255-151-115-090)",
    };
  }
  return {
    label: "AWAITING",
    color: "var(--ui-rgba-255-194-87-960)",
    border: "var(--ui-rgba-255-194-87-760)",
    background: "var(--ui-rgba-255-194-87-080)",
  };
}

function DetailRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <Box
      sx={{
        display: "grid",
        gridTemplateColumns: { xs: "1fr", sm: "132px minmax(0, 1fr)" },
        gap: { xs: 0.25, sm: 1 },
        py: 0.9,
        borderTop: "1px solid var(--ui-rgba-166-120-255-180)",
      }}
    >
      <Typography
        variant="caption"
        sx={{
          color: "var(--ui-rgba-180-154-218-850)",
          letterSpacing: 0,
        }}
      >
        {label}
      </Typography>
      <Typography
        variant="caption"
        title={value}
        sx={{
          color: "var(--ui-rgba-248-246-255-960)",
          fontFamily: mono ? "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)" : undefined,
          fontWeight: mono ? 700 : 600,
          textAlign: { xs: "left", sm: "right" },
          overflowWrap: "anywhere",
        }}
      >
        {value || "Not provided"}
      </Typography>
    </Box>
  );
}

export function ApprovalPromptOverlay({
  tasks,
  approvalLogs = [],
  busyTargetKey,
  errorMessage,
  onDecide,
  onDismiss,
  onOpenTasks,
  onOpenChat,
  hiddenTargetKeys = [],
}: Props) {
  const [expanded, setExpanded] = useState(true);
  const [activeIndex, setActiveIndex] = useState(0);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [comment, setComment] = useState("");
  const previousSignatureRef = useRef("");
  const hiddenTargets = useMemo(() => new Set(hiddenTargetKeys), [hiddenTargetKeys]);

  const approvals = useMemo(
    () =>
      [
        ...tasks
          .map((task) => buildTaskApprovalCard(task as ApprovalTask))
          .filter((task): task is ApprovalCard => Boolean(task)),
        ...approvalLogs
          .map(buildApprovalLogCard)
          .filter((entry): entry is ApprovalCard => Boolean(entry)),
      ]
        .filter((approval) => !hiddenTargets.has(targetKey(approval.target)))
        .sort(sortNewestFirst),
    [approvalLogs, hiddenTargets, tasks],
  );

  const signature = approvals.map((approval) => approval.key).join("|");

  useEffect(() => {
    if (!signature) {
      previousSignatureRef.current = "";
      return;
    }
    if (previousSignatureRef.current !== signature) {
      previousSignatureRef.current = signature;
      setExpanded(true);
      setActiveIndex(0);
      setPreviewOpen(false);
    }
  }, [signature]);

  useEffect(() => {
    setActiveIndex((current) => Math.min(current, Math.max(approvals.length - 1, 0)));
  }, [approvals.length]);

  const activeApproval = approvals[activeIndex] ?? approvals[0] ?? null;

  useEffect(() => {
    setComment("");
    setPreviewOpen(false);
  }, [activeApproval?.key]);

  if (!activeApproval) return null;

  const busy = busyTargetKey === targetKey(activeApproval.target);
  const tone = statusTone(activeApproval);
  const pendingCount = approvals.filter((approval) => approval.status === "pending").length;
  const expiredCount = approvals.filter((approval) => approval.status === "expired").length;
  const canUseNote = activeApproval.canApprove || activeApproval.canReject;
  const stepSummary =
    activeApproval.steps.length > 1
      ? activeApproval.steps.map((step) => step.actionName).join(" -> ")
      : activeApproval.steps[0]?.actionName || activeApproval.actionName;
  const preview = previewText(activeApproval.preview);

  if (!expanded) {
    return (
      <Box
        role="button"
        tabIndex={0}
        onClick={() => setExpanded(true)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") setExpanded(true);
        }}
        sx={{
          position: "fixed",
          right: { xs: 12, md: 22 },
          bottom: { xs: 12, md: 22 },
          zIndex: 1450,
          width: { xs: "calc(100vw - 24px)", sm: 360 },
          maxWidth: "calc(100vw - 24px)",
          borderRadius: "8px",
          border: `1px solid ${tone.border}`,
          background:
            "linear-gradient(160deg, var(--ui-rgba-11-8-20-980), var(--ui-rgba-21-12-41-960))",
          boxShadow: "0 20px 56px var(--ui-rgba-0-0-0-520)",
          p: 1.1,
          cursor: "pointer",
        }}
      >
        <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
          <Box
            sx={{
              width: 34,
              height: 34,
              borderRadius: "8px",
              display: "grid",
              placeItems: "center",
              color: tone.color,
              background: tone.background,
              flexShrink: 0,
            }}
          >
            <NotificationsActiveRoundedIcon fontSize="small" />
          </Box>
          <Box sx={{ minWidth: 0, flex: 1 }}>
            <Typography variant="body2" sx={{ color: "var(--ui-rgba-248-246-255-980)", fontWeight: 800 }}>
              {pendingCount > 0 ? "Approval waiting" : "Approval needs attention"}
            </Typography>
            <Typography variant="caption" sx={{ color: "var(--ui-rgba-190-178-220-820)" }}>
              {pendingCount} pending{expiredCount ? ` . ${expiredCount} expired` : ""} . Click to review
            </Typography>
          </Box>
          <OpenInFullRoundedIcon sx={{ color: "var(--ui-rgba-190-178-220-760)", fontSize: 18 }} />
        </Stack>
      </Box>
    );
  }

  return (
    <Box
      role="dialog"
      aria-modal="false"
      aria-label="Approval required"
      sx={{
        position: "fixed",
        right: { xs: 10, md: 24 },
        bottom: { xs: 10, md: 24 },
        width: { xs: "calc(100vw - 20px)", sm: 560, lg: 610 },
        maxWidth: "calc(100vw - 20px)",
        maxHeight: { xs: "calc(100vh - 20px)", md: "calc(100vh - 48px)" },
        overflowY: "auto",
        zIndex: 1450,
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-166-120-255-360)",
        background:
          "radial-gradient(circle at 12% 0%, var(--ui-rgba-126-87-255-170), transparent 32%), linear-gradient(155deg, var(--ui-rgba-8-5-16-985), var(--ui-rgba-17-11-31-970))",
        boxShadow:
          "0 28px 80px var(--ui-rgba-0-0-0-620), inset 0 1px 0 var(--ui-rgba-255-255-255-060)",
        backdropFilter: "blur(22px)",
        WebkitBackdropFilter: "blur(22px)",
        p: { xs: 1.2, sm: 1.6 },
        "&::before, &::after": {
          content: '""',
          position: "absolute",
          pointerEvents: "none",
          width: 34,
          height: 34,
          borderColor: "var(--ui-rgba-166-120-255-820)",
        },
        "&::before": {
          top: -1,
          left: -1,
          borderTop: "2px solid",
          borderLeft: "2px solid",
        },
        "&::after": {
          right: -1,
          bottom: -1,
          borderRight: "2px solid",
          borderBottom: "2px solid",
        },
      }}
    >
      <Stack spacing={1.4}>
        <Stack direction="row" spacing={1.2} sx={{ alignItems: "flex-start" }}>
          <Box
            sx={{
              width: 38,
              height: 38,
              borderRadius: "8px",
              display: "grid",
              placeItems: "center",
              color: "var(--ui-rgba-183-152-255-980)",
              background: "var(--ui-rgba-166-120-255-130)",
              border: "1px solid var(--ui-rgba-166-120-255-260)",
              flexShrink: 0,
            }}
          >
            <ShieldOutlinedIcon fontSize="small" />
          </Box>
          <Box sx={{ minWidth: 0, flex: 1 }}>
            <Stack
              direction="row"
              spacing={0.75}
              useFlexGap
              sx={{ alignItems: "center", flexWrap: "wrap", mb: 0.3 }}
            >
              <Typography
                variant="subtitle1"
                sx={{ color: "var(--ui-rgba-248-246-255-990)", fontWeight: 850, lineHeight: 1.2 }}
              >
                Pending action
              </Typography>
              <Typography
                variant="subtitle1"
                sx={{
                  color: "var(--ui-rgba-190-178-220-820)",
                  fontWeight: 700,
                  lineHeight: 1.2,
                }}
              >
                .
              </Typography>
              <Typography
                variant="subtitle1"
                title={activeApproval.actionName}
                sx={{
                  color: "var(--ui-rgba-248-246-255-990)",
                  fontFamily: "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)",
                  fontWeight: 850,
                  lineHeight: 1.2,
                  overflowWrap: "anywhere",
                }}
              >
                {activeApproval.actionName}
              </Typography>
            </Stack>
            <Typography variant="caption" sx={{ color: "var(--ui-rgba-190-178-220-800)" }}>
              {approvals.length > 1
                ? `${activeIndex + 1} / ${approvals.length} approvals`
                : "Human-in-the-loop approval"}
            </Typography>
          </Box>
          <Chip
            size="small"
            label={tone.label}
            sx={{
          minWidth: 112,
              height: 30,
              borderRadius: "4px",
              color: tone.color,
              border: `1px solid ${tone.border}`,
              background: tone.background,
              fontFamily: "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)",
              fontWeight: 900,
              letterSpacing: 0,
            }}
          />
          <IconButton
            size="small"
            onClick={() => setExpanded(false)}
            sx={{ color: "var(--ui-rgba-190-178-220-780)", mt: -0.4 }}
            aria-label="Collapse approval popup"
          >
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </Stack>

        <Box
          sx={{
            borderRadius: "8px",
            border: "1px solid var(--ui-rgba-166-120-255-240)",
            background: "var(--ui-rgba-12-8-24-760)",
            px: { xs: 1.15, sm: 1.45 },
            py: 0.65,
          }}
        >
          <DetailRow label="title" value={activeApproval.title} />
          <DetailRow label="action" value={activeApproval.actionName} mono />
          <DetailRow label="scope" value={activeApproval.scopeLabel || activeApproval.sourceLabel || "local"} mono />
          <DetailRow label="requested" value={formatDateTime(activeApproval.requestedAt)} mono />
          <DetailRow label="ttl" value={formatTtl(activeApproval.expiresAt, activeApproval.status)} mono />
          <DetailRow label="chain" value={stepSummary} mono />
          <DetailRow label="approval id" value={shortId(activeApproval.id)} mono />
        </Box>

        <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
          {activeApproval.riskLevel ? (
            <Chip
              size="small"
              label={`Risk ${activeApproval.riskLevel}`}
              sx={{
                color: "var(--ui-rgba-255-194-87-930)",
                border: "1px solid var(--ui-rgba-255-194-87-260)",
                background: "var(--ui-rgba-255-194-87-070)",
              }}
            />
          ) : null}
          {activeApproval.riskScore ? (
            <Chip
              size="small"
              label={`Score ${activeApproval.riskScore}`}
              sx={{
                color: "var(--ui-rgba-132-224-255-920)",
                border: "1px solid var(--ui-rgba-132-224-255-220)",
                background: "var(--ui-rgba-132-224-255-060)",
              }}
            />
          ) : null}
          {activeApproval.ruleName ? (
            <Chip
              size="small"
              label={activeApproval.ruleName}
              sx={{
                color: "var(--ui-rgba-190-178-220-860)",
                border: "1px solid var(--ui-rgba-190-178-220-180)",
                background: "var(--ui-rgba-255-255-255-030)",
              }}
            />
          ) : null}
        </Stack>

        {activeApproval.summary || activeApproval.reason ? (
          <Box
            sx={{
              borderRadius: "8px",
              border: "1px solid var(--ui-rgba-255-255-255-070)",
              background: "var(--ui-rgba-255-255-255-025)",
              px: 1.2,
              py: 1,
            }}
          >
            {activeApproval.summary ? (
              <Typography variant="body2" sx={{ color: "var(--ui-rgba-236-232-255-930)", lineHeight: 1.45 }}>
                {activeApproval.summary}
              </Typography>
            ) : null}
            {activeApproval.reason ? (
              <Typography
                variant="caption"
                sx={{
                  display: "block",
                  mt: activeApproval.summary ? 0.8 : 0,
                  color: "var(--ui-rgba-190-178-220-840)",
                  lineHeight: 1.45,
                }}
              >
                {activeApproval.reason}
              </Typography>
            ) : null}
          </Box>
        ) : null}

        {previewOpen ? (
          <Box
            sx={{
              borderRadius: "8px",
              border: "1px solid var(--ui-rgba-132-224-255-220)",
              background: "var(--ui-rgba-3-8-18-780)",
              overflow: "hidden",
            }}
          >
            {activeApproval.steps.length > 0 ? (
              <Stack spacing={0.6} sx={{ p: 1.1, borderBottom: "1px solid var(--ui-rgba-132-224-255-120)" }}>
                <Typography variant="caption" sx={{ color: "var(--ui-rgba-132-224-255-900)", fontWeight: 800 }}>
                  Chain preview
                </Typography>
                {activeApproval.steps.map((step, index) => (
                  <Box key={`${activeApproval.key}-step-${index}`}>
                    <Typography variant="caption" sx={{ color: "var(--ui-rgba-248-246-255-940)", fontWeight: 750 }}>
                      {index + 1}. {step.actionName}
                    </Typography>
                    {step.detail ? (
                      <Typography variant="caption" sx={{ display: "block", color: "var(--ui-rgba-190-178-220-760)" }}>
                        {step.detail}
                      </Typography>
                    ) : null}
                  </Box>
                ))}
              </Stack>
            ) : null}
            <Box
              component="pre"
              sx={{
                m: 0,
                p: 1.15,
                maxHeight: 220,
                overflow: "auto",
                color: "var(--ui-rgba-222-236-255-930)",
                fontFamily: "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)",
                fontSize: 12,
                lineHeight: 1.45,
                whiteSpace: "pre-wrap",
                overflowWrap: "anywhere",
              }}
            >
              {preview}
            </Box>
          </Box>
        ) : null}

        {errorMessage ? (
          <Alert severity="error" sx={{ py: 0.25 }}>
            {errorMessage}
          </Alert>
        ) : null}

        <TextField
          size="small"
          label="Operator note"
          value={comment}
          onChange={(event) => setComment(event.target.value)}
          placeholder="Optional note"
          multiline
          minRows={2}
          disabled={busy || !canUseNote}
          sx={{
            "& .MuiInputBase-root": {
              color: "var(--ui-rgba-248-246-255-960)",
              background: "var(--ui-rgba-255-255-255-030)",
              borderRadius: "8px",
            },
            "& .MuiInputLabel-root": { color: "var(--ui-rgba-190-178-220-780)" },
            "& .MuiOutlinedInput-notchedOutline": { borderColor: "var(--ui-rgba-166-120-255-220)" },
          }}
        />

        {!activeApproval.canApprove && activeApproval.canReject ? (
          <Alert severity="warning" sx={{ py: 0.35 }}>
            This approval expired before it could be accepted. Dismiss it here, then ask the agent to prepare the action again.
          </Alert>
        ) : !activeApproval.canApprove && !activeApproval.canReject ? (
          <Alert severity="warning" sx={{ py: 0.35 }}>
            This approval can no longer be accepted. Ask the agent to prepare the action again.
          </Alert>
        ) : null}

        <Stack direction={{ xs: "column", sm: "row" }} spacing={0.9}>
          <Button
            variant="outlined"
            color="inherit"
            onClick={() => onDecide(activeApproval.target, "reject", comment)}
            disabled={busy || !activeApproval.canReject}
            sx={{
              flex: 1,
              minHeight: 44,
              borderRadius: "6px",
              textTransform: "none",
              color: "var(--ui-rgba-190-178-220-900)",
              borderColor: "var(--ui-rgba-166-120-255-360)",
            }}
          >
            {activeApproval.status === "expired" && !activeApproval.canApprove ? "Dismiss" : "Deny"}
          </Button>
          <Button
            variant="outlined"
            onClick={() => setPreviewOpen((value) => !value)}
            sx={{
              flex: 1,
              minHeight: 44,
              borderRadius: "6px",
              textTransform: "none",
              color: "var(--ui-rgba-222-236-255-940)",
              borderColor: "var(--ui-rgba-132-224-255-380)",
            }}
          >
            {previewOpen ? "Hide Preview" : "Preview"}
          </Button>
          <Button
            variant="contained"
            onClick={() => onDecide(activeApproval.target, "approve", comment)}
            disabled={busy || !activeApproval.canApprove}
            sx={{
              flex: 1,
              minHeight: 44,
              borderRadius: "6px",
              textTransform: "none",
              fontWeight: 900,
              color: "var(--ui-rgba-255-255-255-980)",
              background:
                "linear-gradient(135deg, var(--ui-rgba-126-58-242-980), var(--ui-rgba-166-120-255-900))",
              boxShadow: "0 12px 28px var(--ui-rgba-126-58-242-300)",
              "&.Mui-disabled": {
                color: "var(--ui-rgba-190-178-220-540)",
                border: "1px solid var(--ui-rgba-190-178-220-160)",
                background: "var(--ui-rgba-255-255-255-045)",
                boxShadow: "none",
              },
            }}
          >
            {busy ? "Working..." : "Approve ->"}
          </Button>
        </Stack>

        <Stack
          direction="row"
          spacing={0.8}
          useFlexGap
          sx={{ justifyContent: "space-between", alignItems: "center", flexWrap: "wrap" }}
        >
          <Stack direction="row" spacing={0.7} useFlexGap sx={{ flexWrap: "wrap" }}>
            {approvals.length > 1 ? (
              <>
                <Button
                  size="small"
                  onClick={() => setActiveIndex((index) => Math.max(index - 1, 0))}
                  disabled={activeIndex === 0}
                  sx={{ textTransform: "none", color: "var(--ui-rgba-190-178-220-820)" }}
                >
                  Previous
                </Button>
                <Button
                  size="small"
                  onClick={() => setActiveIndex((index) => Math.min(index + 1, approvals.length - 1))}
                  disabled={activeIndex >= approvals.length - 1}
                  sx={{ textTransform: "none", color: "var(--ui-rgba-190-178-220-820)" }}
                >
                  Next
                </Button>
              </>
            ) : null}
          </Stack>
          <Stack direction="row" spacing={0.7} useFlexGap sx={{ flexWrap: "wrap" }}>
            <Button
              size="small"
              onClick={() => onDismiss(activeApproval.target, comment)}
              disabled={busy || !activeApproval.canDismiss}
              sx={{ textTransform: "none", color: "var(--ui-rgba-255-151-115-880)" }}
            >
              Dismiss
            </Button>
            <Button
              size="small"
              onClick={activeApproval.target.kind === "direct_chat" ? onOpenChat || onOpenTasks : onOpenTasks}
              sx={{ textTransform: "none", color: "var(--ui-rgba-190-178-220-850)" }}
            >
              Open {activeApproval.target.kind === "direct_chat" ? "Chat" : "Tasks"}
            </Button>
            <Button
              size="small"
              onClick={() => setExpanded(false)}
              sx={{ textTransform: "none", color: "var(--ui-rgba-190-178-220-760)" }}
            >
              Later
            </Button>
          </Stack>
        </Stack>
      </Stack>
    </Box>
  );
}
