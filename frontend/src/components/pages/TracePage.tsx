import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import CloseIcon from "@mui/icons-material/Close";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
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
import Grid2 from "@mui/material/Grid";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { api } from "../../api/client";
import { formatUiTime } from "../../lib/dateFormat";
import { formatChannelSource } from "../channelLabels";
import { LiveEventConsole } from "../LiveEventConsole";
import { MetricBarCard } from "../analytics/MetricBarCard";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  asRecord,
  errMessage,
  isRecord,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";
import type { TraceOperationalEvent, TraceSummary } from "../../types";

const REFRESH_MS = 8000;
type TraceSection = "history" | "agentark" | "sync" | "exports" | "security";

const TRACE_SECTIONS = new Set<TraceSection>([
  "history",
  "agentark",
  "sync",
  "exports",
  "security",
]);

function traceSectionFromLocationSearch(): TraceSection {
  if (typeof window === "undefined") return "history";
  const raw = new URLSearchParams(window.location.search).get("section") || "";
  const normalized = raw.trim().toLowerCase().replace(/[\s_]+/g, "-");
  const compact = normalized.replace(/-/g, "");
  if (TRACE_SECTIONS.has(normalized as TraceSection)) {
    return normalized as TraceSection;
  }
  if (TRACE_SECTIONS.has(compact as TraceSection)) {
    return compact as TraceSection;
  }
  return "history";
}

function setTraceSectionLocationSearch(section: TraceSection) {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  if (section === "history") {
    url.searchParams.delete("section");
  } else {
    url.searchParams.set("section", section);
  }
  const nextUrl = `${url.pathname}${url.search}${url.hash}`;
  const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (nextUrl !== currentUrl) {
    window.history.replaceState(null, "", nextUrl);
  }
}

const TRACE_EVENT_TYPE_PALETTE = [
  "#ff9b9b",
  "#ffbf82",
  "#78f2b0",
  "#d8ad78",
  "#89d7ab",
  "#c8d8c9",
  "#f2c14e",
];

function WorkspaceLazyPanel({
  children,
  message = "Loading panel...",
}: {
  children: ReactNode;
  message?: string;
}) {
  return (
    <Box className="list-shell" sx={{ minHeight: 180, p: 1.5 }}>
      <Typography variant="body2" sx={{ color: "text.secondary" }}>
        {message}
      </Typography>
      {children}
    </Box>
  );
}

function formatTraceDuration(durationMs: unknown): string {
  const ms = num(durationMs, -1);
  if (ms < 0) return "pending";
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60)
    return `${totalSeconds >= 10 ? totalSeconds.toFixed(0) : totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  return `${minutes}m ${seconds}s`;
}

function formatTraceStepTime(raw: string): string {
  if (!raw) return "";
  const match = raw.match(/^(.+?)(\s*\(\d+ms\))?$/);
  if (!match) return raw;
  const isoPart = match[1].trim();
  const durationPart = match[2]?.trim() || "";
  const date = new Date(isoPart);
  if (Number.isNaN(date.getTime())) return raw;
  const time = formatUiTime(date, { fallback: raw, includeSeconds: true });
  return durationPart ? `${time} ${durationPart}` : time;
}

function buildEvolutionFocusCaseLabel(row: JsonRecord): string {
  const surface = str(row.surface, "case").trim();
  const delta = num(row.score_delta, Number.NaN);
  const preview = str(row.prompt_preview, "").trim();
  const invalidBefore = toBool(row.invalid_json_before);
  const invalidAfter = toBool(row.invalid_json_after);
  const parts = [surface];
  if (Number.isFinite(delta)) {
    parts.push(`${delta >= 0 ? "+" : ""}${(delta * 100).toFixed(0)} pts`);
  }
  if (invalidBefore !== invalidAfter) {
    parts.push(invalidAfter ? "JSON regressed" : "JSON stabilized");
  }
  if (preview) parts.push(preview);
  return parts.join(" | ");
}

function traceStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  switch (status.trim().toLowerCase()) {
    case "completed":
      return "success";
    case "warning":
      return "warning";
    case "failed":
      return "error";
    default:
      return "default";
  }
}

function traceStepColor(
  stepType: string,
): "default" | "success" | "warning" | "error" {
  switch (stepType.trim().toLowerCase()) {
    case "success":
      return "success";
    case "warning":
      return "warning";
    case "error":
      return "error";
    default:
      return "default";
  }
}

function formatTraceData(value: unknown): string {
  if (typeof value !== "string") return str(value, "");
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

type TraceEvidenceItem = {
  title: string;
  detail: string;
  type: string;
};

type TraceReceiptLine = {
  label: string;
  value: string;
};

type TraceReceiptItem = {
  label: string;
  detail: string;
  status?: string;
};

type TraceRunReceipt = {
  summary: string;
  rows: TraceReceiptLine[];
  actions: TraceReceiptItem[];
  outputs: TraceReceiptItem[];
  evidence: TraceReceiptItem[];
  failures: TraceReceiptItem[];
};

type TraceStepConsoleView = {
  detail: string;
  dataText: string;
};

function pickTraceStepArtifacts(step: JsonRecord): JsonRecord[] {
  return pickRecords(step, "artifacts");
}

function traceArtifactLabel(artifact: JsonRecord): string {
  const explicit = str(artifact.label, "").trim();
  if (explicit) return explicit;
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

function traceArtifactKindLabel(artifact: JsonRecord): string {
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

function traceArtifactFormat(artifact: JsonRecord): string {
  return str(artifact.format, "").trim().toUpperCase();
}

function traceArtifactBody(artifact: JsonRecord): string {
  const raw = artifact.data;
  if (typeof raw === "string") return formatTraceData(raw);
  if (raw == null) return "";
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return str(raw, "");
  }
}

function traceArtifactSummary(artifact: JsonRecord): string {
  const explicit = str(artifact.summary, "").trim();
  if (explicit) return explicit;
  const body = collapseInlineWhitespace(traceArtifactBody(artifact));
  return body ? truncateTraceEvidence(body, 180) : "";
}

function traceArtifactChipLabel(artifact: JsonRecord): string {
  const label = traceArtifactLabel(artifact);
  const summary = traceArtifactSummary(artifact);
  if (summary && summary.length <= 56) {
    return `${label}: ${summary}`;
  }
  return label;
}

function summarizeTraceArtifactsInline(artifacts: JsonRecord[]): string {
  return uniqueNonEmptyStrings(
    artifacts.map(
      (artifact) =>
        traceArtifactSummary(artifact) || traceArtifactLabel(artifact),
    ),
  )
    .slice(0, 2)
    .map((value) => truncateTraceEvidence(value, 120))
    .join(" | ");
}

function buildTraceArtifactBlocks(artifacts: JsonRecord[]): string {
  return artifacts
    .map((artifact) => {
      const label = traceArtifactLabel(artifact);
      const format = traceArtifactFormat(artifact);
      const summary = traceArtifactSummary(artifact);
      const body = traceArtifactBody(artifact);
      const lines = [
        format ? `${label} (${format})` : label,
        summary ? `Summary: ${summary}` : "",
        body,
      ].filter(Boolean);
      return lines.join("\n");
    })
    .filter(Boolean)
    .join("\n\n");
}

function firstReceiptString(record: JsonRecord, keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
    if (typeof value === "number" || typeof value === "boolean") {
      return String(value);
    }
  }
  return "";
}

function traceReceiptStatus(record: JsonRecord, fallback = ""): string {
  return firstReceiptString(record, [
    "status",
    "state",
    "outcome",
    "result",
    "type",
    "step_type",
  ]) || fallback;
}

function traceReceiptStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (
    normalized === "completed" ||
    normalized === "success" ||
    normalized === "succeeded" ||
    normalized === "ok"
  ) {
    return "success";
  }
  if (
    normalized === "failed" ||
    normalized === "failure" ||
    normalized === "error" ||
    normalized === "cancelled" ||
    normalized === "canceled"
  ) {
    return "error";
  }
  if (
    normalized === "warning" ||
    normalized === "blocked" ||
    normalized === "timeout" ||
    normalized === "timed_out"
  ) {
    return "warning";
  }
  return "default";
}

function traceReceiptIsFailure(record: JsonRecord): boolean {
  if (record.success === false || record.ok === false) return true;
  return traceReceiptStatusColor(traceReceiptStatus(record)) === "error";
}

function traceReceiptItemDetail(record: JsonRecord): string {
  const direct = firstReceiptString(record, [
    "failure_class",
    "last_error",
    "error",
    "error_text",
    "result_summary",
    "summary",
    "detail",
    "message",
    "outcome",
  ]);
  if (direct) return truncateTraceEvidence(direct, 180);
  const latency = firstReceiptString(record, ["latency_ms", "duration_ms"]);
  return latency ? `${latency} ms` : "";
}

function traceReceiptActionLabel(record: JsonRecord, fallback: string): string {
  return (
    firstReceiptString(record, [
      "tool_name",
      "action_name",
      "event_type",
      "kind",
      "title",
      "name",
    ]) || fallback
  );
}

function buildTraceRunReceipt(
  trace: JsonRecord,
  steps: JsonRecord[],
): TraceRunReceipt {
  const executionRun = asRecord(trace.execution_run);
  const toolAttempts = pickRecords(trace, "tool_attempts");
  const operationalLogs = pickRecords(trace, "operational_logs");
  const checkpoints = pickRecords(trace, "checkpoints");
  const artifacts = steps.flatMap(pickTraceStepArtifacts);
  const status = traceReceiptStatus(trace, str(trace.status, "running"));
  const duration = formatTraceDuration(trace.duration_ms);
  const resultSummary =
    firstReceiptString(executionRun, ["result_summary", "summary"]) ||
    firstReceiptString(trace, ["result_summary", "response"]);
  const summary =
    resultSummary ||
    (status.trim().toLowerCase() === "completed"
      ? `Completed in ${duration}`
      : `Current status: ${status || "running"}`);

  const rows = [
    { label: "Outcome", value: status || "-" },
    { label: "Duration", value: duration },
    {
      label: "Steps",
      value: String(num(trace.step_count, steps.length)),
    },
    { label: "Source", value: formatChannelSource(str(trace.channel, "chat"), "Chat") },
    str(trace.model) ? { label: "Model", value: str(trace.model) } : null,
    num(trace.total_tokens, 0) > 0
      ? { label: "Tokens", value: String(num(trace.total_tokens, 0)) }
      : null,
  ].filter((row): row is TraceReceiptLine => Boolean(row));

  const actions: TraceReceiptItem[] = [
    ...toolAttempts.map((attempt, index) => ({
      label: traceReceiptActionLabel(attempt, `Tool attempt ${index + 1}`),
      status: traceReceiptStatus(attempt, "recorded"),
      detail: traceReceiptItemDetail(attempt),
    })),
    ...operationalLogs.slice(0, Math.max(0, 8 - toolAttempts.length)).map(
      (event, index) => ({
        label: traceReceiptActionLabel(event, `Operational event ${index + 1}`),
        status:
          typeof event.success === "boolean"
            ? event.success
              ? "success"
              : "failed"
            : traceReceiptStatus(event, "recorded"),
        detail: traceReceiptItemDetail(event),
      }),
    ),
  ].filter((item) => item.label || item.detail);

  const fallbackActions =
    actions.length > 0
      ? actions
      : steps.slice(0, 6).map((step, index) => ({
          label: firstReceiptString(step, ["title", "source"]) || `Step ${index + 1}`,
          status: traceReceiptStatus(step, "recorded"),
          detail: traceReceiptItemDetail(step),
        }));

  const outputs = uniqueNonEmptyStrings(
    artifacts.map((artifact) => traceArtifactChipLabel(artifact)),
  )
    .slice(0, 6)
    .map((label) => ({ label, detail: "Artifact" }));
  if (outputs.length === 0 && str(trace.response).trim()) {
    outputs.push({
      label: "Final output",
      detail: truncateTraceEvidence(str(trace.response), 180),
    });
  }

  const evidence = uniqueNonEmptyStrings([
    ...artifacts.map((artifact) => traceArtifactSummary(artifact)),
    ...steps.map((step) => firstReceiptString(step, ["ref_id", "source"])),
    ...checkpoints.map((checkpoint) =>
      firstReceiptString(checkpoint, ["label", "kind", "id"]),
    ),
  ])
    .slice(0, 6)
    .map((value) => ({
      label: truncateTraceEvidence(value, 120),
      detail: "Evidence",
    }));

  const failures = [
    ...toolAttempts.filter(traceReceiptIsFailure),
    ...operationalLogs.filter(traceReceiptIsFailure),
    ...steps.filter(traceReceiptIsFailure),
  ]
    .slice(0, 4)
    .map((item, index) => ({
      label: traceReceiptActionLabel(item, `Failure ${index + 1}`),
      status: traceReceiptStatus(item, "failed"),
      detail: traceReceiptItemDetail(item) || "No failure detail recorded.",
    }));

  return {
    summary: truncateTraceEvidence(summary, 320),
    rows,
    actions: fallbackActions.slice(0, 8),
    outputs,
    evidence,
    failures,
  };
}

function truncateTraceEvidence(value: string, max = 240): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, Math.max(0, max - 3)).trimEnd()}...`;
}

function summarizeTraceOutcome(trace: JsonRecord): string {
  const status = str(trace.status, "").trim().toLowerCase();
  if (status === "completed") {
    return `Outcome: completed successfully in ${formatTraceDuration(trace.duration_ms)}`;
  }
  if (status === "failed" || status === "error" || status === "warning") {
    return `Outcome: failed after ${formatTraceDuration(trace.duration_ms)}`;
  }
  if (!status || status === "running") {
    return "Outcome: still running";
  }
  return `Outcome: ${status}`;
}

function isExecutionProofStep(step: JsonRecord): boolean {
  const combined =
    `${str(step.title, "")}\n${str(step.detail, "")}\n${formatTraceData(step.data)}`.toLowerCase();
  return /execution record saved|execution proof generated|verification id:|proof id:/.test(
    combined,
  );
}

function buildTraceEvidenceItems(steps: JsonRecord[]): TraceEvidenceItem[] {
  return steps
    .map((step) => {
      const title = str(step.title, "").trim();
      const detail = str(step.detail, "").trim();
      const dataText = formatTraceData(step.data);
      const type = str(step.type, str(step.step_type, "step")).trim() || "step";
      const combined = `${title}\n${detail}\n${dataText}`.toLowerCase();
      if (!title && !detail && !dataText) return null;
      if (
        /execution record saved|execution proof generated|verification id:|proof id:/.test(
          combined,
        )
      )
        return null;
      if (
        /memory available|context packing|selected the best available model|using a direct execution strategy|prepared the next response/.test(
          combined,
        ) &&
        !dataText
      ) {
        return null;
      }
      const summary = detail || dataText || title;
      return {
        title: title || "Step",
        detail: truncateTraceEvidence(summary),
        type,
      };
    })
    .filter((item): item is TraceEvidenceItem => Boolean(item))
    .slice(-4);
}

function extractTraceArtifacts(
  trace: JsonRecord,
  steps: JsonRecord[],
): string[] {
  const artifactLabels = uniqueNonEmptyStrings(
    steps.flatMap((step) =>
      pickTraceStepArtifacts(step).map((artifact) =>
        traceArtifactChipLabel(artifact),
      ),
    ),
  );
  if (artifactLabels.length > 0) {
    return artifactLabels.slice(0, 6);
  }
  const sources = [
    str(trace.response, ""),
    ...steps.flatMap((step) => [
      str(step.detail, ""),
      formatTraceData(step.data),
    ]),
  ];
  const found = new Set<string>();
  for (const source of sources) {
    const text = source.trim();
    if (!text) continue;
    for (const match of text.match(/https?:\/\/[^\s"'<>]+/g) || []) {
      found.add(match);
    }
    for (const match of text.match(/[A-Za-z]:\\[^\s"'<>]+/g) || []) {
      found.add(match);
    }
  }
  return Array.from(found).slice(0, 6);
}

function buildExecutionProofConsoleEvidence(
  trace: JsonRecord,
  steps: JsonRecord[],
): string {
  const lines: string[] = [];
  const action = truncateTraceEvidence(str(trace.message, ""), 180);
  const outcome = summarizeTraceOutcome(trace);
  const finalResult = truncateTraceEvidence(str(trace.response, ""), 220);
  const artifacts = extractTraceArtifacts(trace, steps).slice(0, 3);
  const keyEvidence = buildTraceEvidenceItems(steps).slice(-3);

  if (action) lines.push(`Action attempted: ${action}`);
  lines.push(outcome);
  if (finalResult) lines.push(`Final result: ${finalResult}`);
  if (artifacts.length > 0) {
    lines.push(`Artifacts or outputs: ${artifacts.join(" | ")}`);
  }
  for (const item of keyEvidence) {
    lines.push(`${item.title}: ${item.detail}`);
  }
  lines.push(
    "Open Trace Detail for the verification record and the full evidence.",
  );

  return lines.join("\n");
}

function buildTraceStepConsoleView(
  trace: JsonRecord,
  steps: JsonRecord[],
  step: JsonRecord,
): TraceStepConsoleView {
  const detail = str(step.detail, "").trim();
  const rawDataText = formatTraceData(step.data);
  const artifactText = buildTraceArtifactBlocks(pickTraceStepArtifacts(step));
  const dataText =
    artifactText && rawDataText.trim() === artifactText.trim()
      ? artifactText
      : [rawDataText, artifactText].filter(Boolean).join("\n\n");

  if (!isExecutionProofStep(step)) {
    return { detail, dataText };
  }

  return {
    detail:
      "Verifiable execution record saved. The evidence for this run is summarized below.",
    dataText: buildExecutionProofConsoleEvidence(trace, steps),
  };
}

function parseTraceDataRecord(value: unknown): JsonRecord {
  if (isRecord(value)) return value;
  if (typeof value !== "string") return {};
  const trimmed = value.trim();
  if (!trimmed) return {};
  try {
    return asRecord(JSON.parse(trimmed));
  } catch {
    return {};
  }
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => str(item, "").trim()).filter(Boolean);
}

function promotionGateSummary(data: JsonRecord): string {
  const report = asRecord(data.promotion_gate_report);
  return (
    str(report.summary, "").trim() ||
    str(data.promotion_gate_summary, "").trim() ||
    str(data.promotion_gate, "").trim()
  );
}

function percentageLabel(value: unknown, digits = 1): string {
  const parsed = num(value, Number.NaN);
  if (!Number.isFinite(parsed)) return "";
  return `${(parsed * 100).toFixed(digits)}%`;
}

type EvolutionReviewCard = {
  key: string;
  title: string;
  status: string;
  detail: string;
  chips: string[];
  rationale?: string;
  example?: string;
  evidence?: string;
};

type EvolutionPatternCard = EvolutionReviewCard & {
  runs: JsonRecord[];
  latestSeen?: string;
  toolSummary?: string;
  completedCount: number;
  failedCount: number;
  acceptedCount: number;
};

function collapseInlineWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function truncateUiText(value: string, maxChars = 120): string {
  const normalized = collapseInlineWhitespace(value);
  if (normalized.length <= maxChars) return normalized;
  return `${normalized.slice(0, Math.max(0, maxChars - 1)).trimEnd()}…`;
}

function titleCaseLabel(value: string): string {
  return value
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(" ");
}

function learningEvidenceStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (
    normalized === "completed" ||
    normalized === "success" ||
    normalized === "succeeded"
  )
    return "success";
  if (normalized === "failed" || normalized === "error") return "error";
  if (normalized === "accepted") return "info";
  if (normalized === "repeating successfully") return "success";
  if (normalized === "mixed results") return "warning";
  if (normalized === "needs review") return "error";
  if (normalized === "user preference captured" || normalized === "seen once")
    return "info";
  return "default";
}

function learningEvidenceTimestampMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function latestLearningEvidenceTimestamp(runs: JsonRecord[]): number {
  return runs.reduce(
    (latest, run) =>
      Math.max(latest, learningEvidenceTimestampMs(str(run.created_at, ""))),
    0,
  );
}

function learningEvidenceToolLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "schedule_task") return "Scheduled task";
  if (normalized === "calendar_create") return "Calendar event";
  return titleCaseLabel(normalized);
}

function summarizeLearningEvidenceTools(values: string[]): string {
  if (values.length === 0) return "";
  const counts = new Map<string, number>();
  values.forEach((value) => {
    const key = value.trim().toLowerCase();
    if (!key) return;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  });
  return Array.from(counts.entries())
    .sort(
      (left, right) => right[1] - left[1] || left[0].localeCompare(right[0]),
    )
    .slice(0, 3)
    .map(
      ([name, count]) =>
        `${learningEvidenceToolLabel(name)}${count > 1 ? ` x${count}` : ""}`,
    )
    .join(", ");
}

function uniqueNonEmptyStrings(values: Array<unknown>): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  values.forEach((value) => {
    const normalized = collapseInlineWhitespace(str(value, "").trim());
    if (!normalized || seen.has(normalized)) return;
    seen.add(normalized);
    out.push(normalized);
  });
  return out;
}

function summarizeEvolutionPatternRun(run: JsonRecord): string {
  const decision = asRecord(run.decision_summary);
  const executionStatus = asRecord(run.execution_status);
  const summary = collapseInlineWhitespace(
    str(
      run.failure_reason,
      str(
        run.outcome_summary,
        str(
          decision.summary,
          str(decision.completion_status, str(executionStatus.status, "")),
        ),
      ),
    ),
  );
  return summary || "No summary recorded.";
}

function evolutionPatternStatusExplanation(card: EvolutionPatternCard): string {
  const normalized = card.status.trim().toLowerCase();
  if (normalized === "user preference captured") {
    return "A direct user correction was recorded here. That kind of signal is strong and can steer similar requests in the future.";
  }
  if (normalized === "repeating successfully") {
    return "Similar requests have been finishing through the same path. AgentArk is checking whether that path is stable enough to reuse by default.";
  }
  if (normalized === "mixed results") {
    return "Similar requests have both worked and failed. AgentArk is comparing the difference before changing behavior.";
  }
  if (normalized === "needs review") {
    return "Similar requests are running into issues often enough that AgentArk may need a guardrail or a different route.";
  }
  if (normalized === "seen once") {
    return "This has only been seen once so far. One example is useful context, but not enough to change behavior yet.";
  }
  return "AgentArk is collecting a few more examples before it decides whether this pattern should change how future requests are handled.";
}

function normalizeLearningEvidenceState(run: JsonRecord): string {
  const decision = asRecord(run.decision_summary);
  return collapseInlineWhitespace(
    str(
      run.correction_state,
      str(decision.completion_status, str(run.success_state, "observed")),
    ),
  ).toLowerCase();
}

function inferLearningEvidenceTitle(
  runs: JsonRecord[],
  requestPreview: string,
  taskType: string,
): string {
  const combinedRequest = requestPreview.toLowerCase();
  const allTools = new Set<string>();
  runs.forEach((run) => {
    stringList(run.tool_names).forEach((toolName) => {
      const normalized = toolName.trim().toLowerCase();
      if (normalized) allTools.add(normalized);
    });
  });

  if (
    /dont use|don't use|do not use/.test(combinedRequest) &&
    /(calendar|calender)/.test(combinedRequest)
  ) {
    return "Operator correction: avoid Calendar";
  }
  if (
    /meeting/.test(combinedRequest) &&
    /(notify|remind|arrive)/.test(combinedRequest)
  ) {
    return "Meeting-date reminder requests";
  }
  if (
    /(notify|remind)/.test(combinedRequest) &&
    /(date|today|tomorrow|january|february|march|april|may|june|july|august|september|october|november|december|\d{4})/.test(
      combinedRequest,
    )
  ) {
    return "Date-based reminder requests";
  }
  if (
    allTools.has("calendar_create") ||
    /(calendar|event)/.test(combinedRequest)
  ) {
    return "Calendar event creation requests";
  }
  if (allTools.has("schedule_task")) {
    return "Scheduled follow-up requests";
  }
  if (taskType) return `${titleCaseLabel(taskType)} requests`;
  if (requestPreview) return truncateUiText(requestPreview, 80);
  return "Observed request pattern";
}

function buildEvolutionReviewCards(steps: JsonRecord[]): EvolutionReviewCard[] {
  const cards: EvolutionReviewCard[] = [];
  steps.forEach((step, idx) => {
    const data = parseTraceDataRecord(step.data);
    const traceKind = str(data.trace_kind, "").trim().toLowerCase();
    if (!traceKind.startsWith("self_evolve.")) return;

    const status = str(step.type, str(step.step_type, "info")).trim() || "info";
    const title = str(step.title, "Evolve").trim();
    const detail = str(step.detail, "").trim();
    const chips: string[] = [];
    const evidence: string[] = [];
    let rationale = "";

    if (traceKind === "self_evolve.request") {
      const mode = str(data.mode, "policy");
      const request = str(data.request, "").trim();
      chips.push(`Mode ${mode}`);
      if (toBool(data.apply_promotion)) chips.push("Promotion enabled");
      const canaryRollout = num(data.canary_rollout_percent, -1);
      if (canaryRollout > 0) chips.push(`Canary ${canaryRollout}%`);
      rationale = request;
    } else if (traceKind === "self_evolve.policy.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineAccuracy = percentageLabel(data.baseline_accuracy, 0);
      const candidateAccuracy = percentageLabel(
        data.best_candidate_accuracy,
        0,
      );
      const gain = num(data.accuracy_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const changedFields = stringList(data.changed_fields);
      const notes = stringList(data.notes);
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineAccuracy || candidateAccuracy) {
        chips.push(`${baselineAccuracy || "?"} -> ${candidateAccuracy || "?"}`);
      }
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      if (changedFields.length)
        evidence.push(`Changed fields: ${changedFields.join(", ")}`);
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.policy.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      chips.push(`Promotion ${promotionMode}`);
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Replay samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Replay gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (traceKind === "self_evolve.prompt.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineScore = percentageLabel(data.baseline_score, 0);
      const candidateScore = percentageLabel(data.best_candidate_score, 0);
      const gain = num(data.score_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      const notes = stringList(data.notes);
      const diffSummary = asRecord(data.diff_summary);
      const routerChanged = stringList(diffSummary.router_changed_fields);
      const primaryResponseChanged = stringList(
        diffSummary.primary_response_changed_fields,
      );
      const synthesisChanged = stringList(
        diffSummary.delegation_synthesis_changed_fields,
      );
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineScore || candidateScore) {
        chips.push(`${baselineScore || "?"} -> ${candidateScore || "?"}`);
      }
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (routerChanged.length)
        evidence.push(`Router changes: ${routerChanged.join(", ")}`);
      if (primaryResponseChanged.length)
        evidence.push(
          `Primary response changes: ${primaryResponseChanged.join(", ")}`,
        );
      if (synthesisChanged.length)
        evidence.push(`Synthesis changes: ${synthesisChanged.join(", ")}`);
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      const invalidRateBefore = num(
        data.baseline_router_invalid_json_rate,
        Number.NaN,
      );
      const invalidRateAfter = num(
        data.candidate_router_invalid_json_rate,
        Number.NaN,
      );
      if (
        Number.isFinite(invalidRateBefore) &&
        Number.isFinite(invalidRateAfter)
      ) {
        evidence.push(
          `Router invalid JSON: ${(invalidRateBefore * 100).toFixed(1)}% -> ${(invalidRateAfter * 100).toFixed(1)}%`,
        );
      }
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.prompt.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      chips.push(`Promotion ${promotionMode}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Experience samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Experience gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (traceKind === "self_evolve.specialist_prompt.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineScore = percentageLabel(data.baseline_score, 0);
      const candidateScore = percentageLabel(data.best_candidate_score, 0);
      const gain = num(data.score_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      const notes = stringList(data.notes);
      const diffSummary = asRecord(data.diff_summary);
      const changedItems = stringList(diffSummary.changed_surfaces).concat(
        stringList(diffSummary.changed_roles),
      );
      const changePreview = stringList(diffSummary.change_preview);
      const focusCases = pickRecords(data, "focus_cases");
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineScore || candidateScore)
        chips.push(`${baselineScore || "?"} -> ${candidateScore || "?"}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (changedItems.length)
        evidence.push(`Changed: ${changedItems.join(", ")}`);
      if (changePreview.length)
        evidence.push(`Preview: ${changePreview.join(" | ")}`);
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      if (focusCases.length) {
        evidence.push(
          `Focus cases: ${focusCases.slice(0, 3).map(buildEvolutionFocusCaseLabel).join(" | ")}`,
        );
      }
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.specialist_prompt.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      chips.push(`Promotion ${promotionMode}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Experience samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Experience gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (
      traceKind === "self_evolve.manual_action.result" ||
      traceKind === "self_evolve.manual_action.request"
    ) {
      const action = str(data.action, "").trim().replace(/_/g, " ");
      const canaryState = asRecord(data.canary_state);
      chips.push(action || "Manual action");
      if (Object.keys(canaryState).length > 0) {
        chips.push(
          toBool(canaryState.enabled) ? "Canary enabled" : "Canary disabled",
        );
      }
      rationale = str(data.message, detail).trim();
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) evidence.push(`Rollout: ${rollout}%`);
    }

    cards.push({
      key: `${traceKind}-${idx}`,
      title,
      status,
      detail,
      chips,
      rationale: rationale || undefined,
      evidence: evidence.join("\n") || undefined,
    });
  });
  return cards;
}

function evolutionTraceIdHint(payload: unknown): string {
  const traceId = str(asRecord(payload).trace_id, "").trim();
  return traceId ? ` Trace ${traceId.slice(0, 8)} recorded.` : "";
}

function syncRunStatusColor(
  status: string,
): "success" | "warning" | "error" | "default" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "completed") return "success";
  if (normalized === "failed") return "error";
  if (normalized === "blocked") return "warning";
  return "default";
}

function syncRunTriggerLabel(trigger: string): string {
  const normalized = trigger.trim().toLowerCase();
  if (normalized === "manual") return "Manual";
  if (normalized === "background") return "Background";
  return normalized ? normalized.replace(/_/g, " ") : "Unknown";
}

type TraceRange = "1h" | "6h" | "24h" | "7d" | "14d" | "30d";
const TRACE_RANGE_PRESETS: {
  value: TraceRange;
  label: string;
  hours: number;
}[] = [
  { value: "1h", label: "1 hour", hours: 1 },
  { value: "6h", label: "6 hours", hours: 6 },
  { value: "24h", label: "24 hours", hours: 24 },
  { value: "7d", label: "7 days", hours: 168 },
  { value: "14d", label: "14 days", hours: 336 },
  { value: "30d", label: "30 days", hours: 720 },
];

function traceRangeHours(range: TraceRange): number {
  return TRACE_RANGE_PRESETS.find((p) => p.value === range)?.hours || 168;
}

function traceRangeSinceISO(range: TraceRange): string {
  const ms = traceRangeHours(range) * 3600 * 1000;
  return new Date(Date.now() - ms).toISOString();
}

function traceSecurityEventTypeLabel(eventType: string): string {
  const normalized = (eventType || "").trim().toLowerCase();
  if (!normalized) return "Unknown";
  return normalized
    .replace(/_/g, " ")
    .replace(/\b\w/g, (m) => m.toUpperCase());
}

type TracePageProps = {
  autoRefresh: boolean;
};

export default function TracePage({ autoRefresh }: TracePageProps) {
  const [traceSection, setTraceSection] = useState<TraceSection>(() =>
    traceSectionFromLocationSearch(),
  );
  const [traceRange, setTraceRange] = useState<TraceRange>("7d");
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);
  const [selectedOperationalEvent, setSelectedOperationalEvent] =
    useState<TraceOperationalEvent | null>(null);
  const [selectedSyncRunId, setSelectedSyncRunId] = useState<string | null>(
    null,
  );
  const [syncRunPage, setSyncRunPage] = useState(0);
  const [showTraceConsole, setShowTraceConsole] = useState(false);
  const [expandedSteps, setExpandedSteps] = useState<Set<number>>(new Set());
  const [historyPage, setHistoryPage] = useState(0);
  const [activityPage, setActivityPage] = useState(0);
  const historyPageSize = 20;
  const activityPageSize = 20;
  const syncRunPageSize = 12;

  useEffect(() => {
    const syncSectionFromLocation = () => {
      setTraceSection(traceSectionFromLocationSearch());
    };
    window.addEventListener("popstate", syncSectionFromLocation);
    window.addEventListener("agentark:navigation", syncSectionFromLocation);
    syncSectionFromLocation();
    return () => {
      window.removeEventListener("popstate", syncSectionFromLocation);
      window.removeEventListener("agentark:navigation", syncSectionFromLocation);
    };
  }, []);

  const traceSince = traceRangeSinceISO(traceRange);
  const traceQ = useQuery({
    queryKey: ["trace-manager", traceRange, activityPage],
    queryFn: () =>
      api.rawGet(
        `/trace?limit=200&since=${encodeURIComponent(traceSince)}&activity_limit=${activityPageSize}&activity_offset=${activityPage * activityPageSize}`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const traceDetailQ = useQuery({
    queryKey: ["trace-detail", selectedTraceId],
    queryFn: () =>
      api.rawGet(`/trace/${encodeURIComponent(selectedTraceId || "")}`),
    enabled: !!selectedTraceId,
  });
  const syncRunsQ = useQuery({
    queryKey: ["integration-sync-runs", syncRunPage],
    queryFn: () =>
      api.rawGet(
        `/integrations/sync/runs?limit=${syncRunPageSize}&offset=${syncRunPage * syncRunPageSize}`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const exportLogsQ = useQuery({
    queryKey: ["settings-observability-logs"],
    queryFn: () => api.rawGet("/settings/observability/logs"),
    refetchInterval: autoRefresh ? 30000 : false,
  });
  const exportLogs = pickRecords(asRecord(exportLogsQ.data), "logs");

  const securityLogsQ = useQuery({
    queryKey: ["trace-security-logs"],
    queryFn: () => api.rawGet("/security/logs?limit=200"),
    refetchInterval: autoRefresh && traceSection === "security" ? REFRESH_MS : false,
  });
  const securityLogs = pickRecords(securityLogsQ.data, "logs");
  const [selectedSecurityLog, setSelectedSecurityLog] = useState<JsonRecord | null>(null);

  const traceData = asRecord(traceQ.data);
  const history = pickRecords(traceData, "history") as TraceSummary[];
  const recentEvents = pickRecords(
    traceData,
    "recent_events",
  ) as TraceOperationalEvent[];
  const recentEventsTotal = num(traceData.recent_events_total, recentEvents.length);
  const selectedOperationalEventTime = humanTs(
    str(selectedOperationalEvent?.created_at, ""),
  );
  const selectedOperationalEventDetails = selectedOperationalEvent
    ? formatTraceData(selectedOperationalEvent.details)
    : "";
  const selectedTrace = asRecord(traceDetailQ.data);
  const steps = pickRecords(traceDetailQ.data, "steps");
  const historyTotal = num(traceData.history_total, history.length);
  const selectedTraceStatus = str(
    selectedTrace.status,
    selectedTraceId ? "running" : "-",
  );
  const selectedTraceProofId = str(selectedTrace.proof_id, "");
  const selectedTraceChannel = str(selectedTrace.channel, "chat");
  const selectedTraceSource = formatChannelSource(selectedTraceChannel, "Chat");
  const selectedTraceResponse = str(selectedTrace.response, "").trim();
  const selectedTraceStarted = humanTs(str(selectedTrace.started_at, ""));
  const selectedTraceCompleted = humanTs(str(selectedTrace.completed_at, ""));
  const traceEvidence = buildTraceEvidenceItems(steps);
  const traceArtifacts = extractTraceArtifacts(selectedTrace, steps);
  const evolutionReviewCards = useMemo(
    () => buildEvolutionReviewCards(steps),
    [steps],
  );
  const syncRunData = asRecord(syncRunsQ.data);
  const syncRuns = pickRecords(syncRunData, "items");
  const syncRunTotal = num(syncRunData.total, syncRuns.length);
  const syncRunStats = asRecord(syncRunData.stats);
  const syncRunBuckets = pickRecords(syncRunStats, "buckets");
  const syncRunPages = Math.max(1, Math.ceil(syncRunTotal / syncRunPageSize));
  const selectedSyncRun = useMemo(
    () =>
      syncRuns.find((item) => str(item.id, "") === selectedSyncRunId) || null,
    [syncRuns, selectedSyncRunId],
  );
  const selectedSyncRunStatus = str(selectedSyncRun?.status, "");
  const selectedSyncRunStarted = humanTs(str(selectedSyncRun?.started_at, ""));
  const selectedSyncRunCompleted = humanTs(
    str(selectedSyncRun?.completed_at, ""),
  );
  const syncRunTrendRows: Array<{ label: string; value: string }> =
    syncRunBuckets.map((bucket) => ({
      label: str(bucket.label, "-"),
      value: `${num(bucket.runs, 0)} runs`,
    }));
  const syncRunTrendValues = syncRunBuckets.map((bucket) =>
    num(bucket.runs, 0),
  );
  const syncAttentionRows: Array<{ label: string; value: string }> =
    syncRunBuckets.map((bucket) => ({
      label: str(bucket.label, "-"),
      value: `${num(bucket.failures, 0) + num(bucket.blocked, 0)} attention`,
    }));
  const syncAttentionValues = syncRunBuckets.map(
    (bucket) => num(bucket.failures, 0) + num(bucket.blocked, 0),
  );
  const traceOutcomeSummary =
    selectedTraceStatus === "completed"
      ? `Completed successfully in ${formatTraceDuration(selectedTrace.duration_ms)}`
      : selectedTraceStatus === "failed"
        ? `Failed after ${formatTraceDuration(selectedTrace.duration_ms)}`
        : `Status: ${selectedTraceStatus}`;
  const traceRunReceipt = buildTraceRunReceipt(selectedTrace, steps);
  const renderDiagnosticsSectionHeader = ({
    eyebrow,
    title,
    description,
    meta,
    compact = false,
  }: {
    eyebrow: string;
    title: string;
    description: string;
    meta: string;
    compact?: boolean;
  }) => (
    <Stack
      className={`diagnostics-section-head${compact ? " diagnostics-section-head-compact" : ""}`}
      direction={{ xs: "column", lg: "row" }}
      spacing={1.5}
      sx={{
        alignItems: { xs: "flex-start", lg: "center" },
        justifyContent: "space-between",
      }}
    >
      <Box className="diagnostics-section-copy">
        <Typography className="diagnostics-section-eyebrow">
          {eyebrow}
        </Typography>
        <Typography
          variant="h5"
          className={`diagnostics-section-title${compact ? " diagnostics-section-title-compact" : ""}`}
        >
          {title}
        </Typography>
        <Typography variant="body2" className="diagnostics-section-description">
          {description}
        </Typography>
      </Box>
      <Box className="diagnostics-section-meta">{meta}</Box>
    </Stack>
  );

  return (
    <WorkspacePageShell spacing={1.25} className="trace-page-shell">
      <WorkspacePageHeader
        eyebrow="Operations"
        title="Trace"
        description="Recent runs, integration activity, and export logs in one place."
        actions={
          <TextField
            select
            className="workspace-page-select"
            size="small"
            value={traceRange}
            onChange={(event) => {
              setTraceRange(event.target.value as TraceRange);
              setHistoryPage(0);
              setActivityPage(0);
            }}
            sx={{ minWidth: 132, flexShrink: 0 }}
          >
            {TRACE_RANGE_PRESETS.map((preset) => (
              <MenuItem key={preset.value} value={preset.value}>
                {preset.label}
              </MenuItem>
            ))}
          </TextField>
        }
      />
      <Box
        className="list-shell workspace-page-subnav-shell"
        data-tour-target="trace-tabs"
      >
        <Stack
          direction="row"
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <Tabs
            value={traceSection}
            onChange={(_, value) => {
              const nextSection = TRACE_SECTIONS.has(value as TraceSection)
                ? (value as TraceSection)
                : "history";
              setTraceSection(nextSection);
              setTraceSectionLocationSearch(nextSection);
              if (nextSection !== "agentark") setActivityPage(0);
            }}
            variant="scrollable"
            allowScrollButtonsMobile
            className="workspace-page-subnav-tabs"
            sx={{ flex: 1 }}
          >
            <Tab value="history" label={`Runs (${historyTotal})`} />
            <Tab value="agentark" label="Runtime" />
            <Tab value="sync" label={`Sync (${syncRunTotal})`} />
            <Tab value="exports" label={`Exports (${exportLogs.length})`} />
            <Tab value="security" label={`Security (${securityLogs.length})`} />
          </Tabs>
        </Stack>
      </Box>
      {traceSection === "history" ? (
        <Box className="list-shell diagnostics-section-shell trace-section-shell">
          {(() => {
            const completed = history.filter(
              (h) => str(h.status, "").toLowerCase() === "completed",
            );
            const failed = history.filter(
              (h) => str(h.status, "").toLowerCase() === "failed",
            );
            const avgSteps =
              history.length > 0
                ? Math.round(
                    history.reduce((sum, h) => sum + num(h.step_count, 0), 0) /
                      history.length,
                  )
                : 0;
            return (
              <Stack spacing={1.25}>
                {/* Compact stats strip */}
                <Stack
                  direction="row"
                  spacing={0}
                  useFlexGap
                  className="trace-stats-bar"
                  sx={{
                    alignItems: "center",
                    flexWrap: "wrap",
                  }}
                >
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Total
                    </Typography>
                    <Typography variant="body2" className="trace-stat-value">
                      {history.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Completed
                    </Typography>
                    <Typography
                      variant="body2"
                      className="trace-stat-value trace-stat-value--success"
                    >
                      {completed.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Failed
                    </Typography>
                    <Typography
                      variant="body2"
                      className="trace-stat-value trace-stat-value--error"
                    >
                      {failed.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Avg steps
                    </Typography>
                    <Typography variant="body2" className="trace-stat-value">
                      {avgSteps}
                    </Typography>
                  </Box>
                  <Box sx={{ flex: 1 }} />
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      pr: 0.5,
                    }}
                  >
                    {history.length} of {historyTotal} runs
                  </Typography>
                </Stack>
                {/* Execution console */}
                {history.length > 0 ? (
                  <Accordion
                    expanded={showTraceConsole}
                    onChange={(_, expanded) => setShowTraceConsole(expanded)}
                    className="trace-console-accordion"
                    disableGutters
                  >
                    <AccordionSummary
                      expandIcon={
                        <ExpandMoreIcon
                          sx={{ color: "var(--ui-rgba-148-190-225-600)" }}
                        />
                      }
                      className="trace-console-accordion-header"
                    >
                      <Typography
                        variant="body2"
                        sx={{ color: "text.secondary" }}
                      >
                        Execution Console
                      </Typography>
                    </AccordionSummary>
                    <AccordionDetails className="trace-console-accordion-body">
                      <WorkspaceLazyPanel message="Loading console...">
                        <LiveEventConsole
                          history={history}
                          events={recentEvents}
                          compact
                        />
                      </WorkspaceLazyPanel>
                    </AccordionDetails>
                  </Accordion>
                ) : null}
                {/* Paginated history table */}
                {history.length === 0 ? (
                  <Alert severity="info">
                    No trace history yet. New runs will appear here
                    automatically.
                  </Alert>
                ) : (
                  (() => {
                    const totalPages = Math.max(
                      1,
                      Math.ceil(history.length / historyPageSize),
                    );
                    const pageSlice = history.slice(
                      historyPage * historyPageSize,
                      (historyPage + 1) * historyPageSize,
                    );
                    return (
                      <>
                        <TableContainer className="table-shell diagnostics-table-shell trace-table-full">
                          <Table size="small" sx={{ tableLayout: "fixed" }}>
                            <TableHead>
                              <TableRow>
                                <TableCell width="13%">Started</TableCell>
                                <TableCell width="9%">Source</TableCell>
                                <TableCell width="44%">Message</TableCell>
                                <TableCell width="12%">Status</TableCell>
                                <TableCell width="10%">Steps</TableCell>
                                <TableCell width="12%">Duration</TableCell>
                              </TableRow>
                            </TableHead>
                            <TableBody>
                              {pageSlice.map((item, idx) => {
                                const id = str(
                                  item.id,
                                  `trace-${historyPage * historyPageSize + idx}`,
                                );
                                const status = str(item.status, "running");
                                const source = formatChannelSource(item.channel);
                                return (
                                  <TableRow
                                    key={id}
                                    hover
                                    onClick={() => setSelectedTraceId(id)}
                                    sx={{ cursor: "pointer" }}
                                  >
                                    <TableCell>
                                      <Typography
                                        variant="body2"
                                        noWrap
                                        title={
                                          humanTs(str(item.started_at)).tip
                                        }
                                      >
                                        {humanTs(str(item.started_at)).label}
                                      </Typography>
                                    </TableCell>
                                    <TableCell>
                                      <Typography
                                        variant="body2"
                                        noWrap
                                        title={str(item.channel)}
                                      >
                                        {source}
                                      </Typography>
                                    </TableCell>
                                    <TableCell>
                                      <Typography
                                        variant="body2"
                                        className="diagnostics-cell-clamp diagnostics-cell-clamp--2"
                                        title={str(item.message_preview)}
                                        sx={{ fontWeight: 600 }}
                                      >
                                        {str(item.message_preview)}
                                      </Typography>
                                    </TableCell>
                                    <TableCell>
                                      <Box
                                        component="span"
                                        sx={{
                                          display: "inline-flex",
                                          alignItems: "center",
                                          gap: 0.75,
                                        }}
                                      >
                                        <Box
                                          component="span"
                                          sx={{
                                            width: 7,
                                            height: 7,
                                            borderRadius: "50%",
                                            flexShrink: 0,
                                            bgcolor:
                                              status === "completed"
                                                ? "var(--ui-rgba-74-210-157-850)"
                                                : status === "failed"
                                                  ? "var(--ui-rgba-255-100-100-850)"
                                                  : "var(--ui-rgba-180-200-220-500)",
                                          }}
                                        />
                                        <Typography
                                          variant="body2"
                                          noWrap
                                          sx={{ color: "text.secondary" }}
                                        >
                                          {status}
                                        </Typography>
                                      </Box>
                                    </TableCell>
                                    <TableCell>
                                      <Typography variant="body2" noWrap>
                                        {num(item.step_count, 0)}
                                      </Typography>
                                    </TableCell>
                                    <TableCell>
                                      <Typography variant="body2" noWrap>
                                        {formatTraceDuration(item.duration_ms)}
                                      </Typography>
                                    </TableCell>
                                  </TableRow>
                                );
                              })}
                            </TableBody>
                          </Table>
                        </TableContainer>
                        {totalPages > 1 ? (
                          <Stack
                            direction="row"
                            spacing={1}
                            sx={{
                              justifyContent: "space-between",
                              alignItems: "center",
                              pt: 0.5,
                            }}
                          >
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              Page {historyPage + 1} of {totalPages} (
                              {history.length} runs)
                            </Typography>
                            <Stack direction="row" spacing={0.5}>
                              <Button
                                size="small"
                                disabled={historyPage === 0}
                                onClick={() => setHistoryPage((p) => p - 1)}
                              >
                                Prev
                              </Button>
                              <Button
                                size="small"
                                disabled={historyPage >= totalPages - 1}
                                onClick={() => setHistoryPage((p) => p + 1)}
                              >
                                Next
                              </Button>
                            </Stack>
                          </Stack>
                        ) : null}
                      </>
                    );
                  })()
                )}
              </Stack>
            );
          })()}
        </Box>
      ) : null}
      {traceSection === "agentark" ? (
        <Box className="list-shell diagnostics-section-shell trace-section-shell">
          <Stack spacing={1.25}>
            <Stack
              direction="row"
              spacing={0}
              useFlexGap
              className="trace-stats-bar"
              sx={{ alignItems: "center", flexWrap: "wrap" }}
            >
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Retention
                </Typography>
                <Typography variant="body2" className="trace-stat-value">
                  14d
                </Typography>
              </Box>
              <Box className="trace-stat-divider" />
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Attention
                </Typography>
                <Typography
                  variant="body2"
                  className="trace-stat-value trace-stat-value--error"
                >
                  {recentEvents.filter((event) => !toBool(event.success)).length}
                </Typography>
              </Box>
              <Box className="trace-stat-divider" />
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Page
                </Typography>
                <Typography variant="body2" className="trace-stat-value">
                  {activityPage + 1}
                </Typography>
              </Box>
              <Box sx={{ flex: 1 }} />
              <Typography variant="caption" sx={{ color: "text.secondary", pr: 0.5 }}>
                Latest first
              </Typography>
            </Stack>

            {traceQ.error ? (
              <Alert severity="warning">{errMessage(traceQ.error)}</Alert>
            ) : recentEvents.length === 0 ? (
              <Alert severity="info">
                No runtime activity recorded for this range yet.
              </Alert>
            ) : (
              <>
                <TableContainer className="table-shell diagnostics-table-shell trace-table-full">
                  <Table size="small" sx={{ tableLayout: "fixed" }}>
                    <TableHead>
                      <TableRow>
                        <TableCell width="13%">Time</TableCell>
                        <TableCell width="10%">Source</TableCell>
                        <TableCell width="14%">Event</TableCell>
                        <TableCell width="12%">Module</TableCell>
                        <TableCell width="39%">Outcome</TableCell>
                        <TableCell width="12%">Duration</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {recentEvents.map((event, idx) => {
                        const eventId = str(event.id, `agentark-event-${idx}`);
                        const time = humanTs(str(event.created_at, ""));
                        const source = formatChannelSource(str(event.source, "Runtime"));
                        const moduleLabel = formatChannelSource(str(event.channel, "agentark"));
                        const eventLabel = titleCaseLabel(str(event.event_type, "activity"));
                        const outcome = str(event.outcome, "Activity recorded");
                        const toolName = str(event.tool_name, "").trim();
                        return (
                          <TableRow
                            key={eventId}
                            hover
                            onClick={() => setSelectedOperationalEvent(event)}
                            sx={{ cursor: "pointer" }}
                          >
                            <TableCell>
                              <Typography variant="body2" noWrap title={time.tip}>
                                {time.label}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap title={source}>
                                {source}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center" }}>
                                <Box
                                  component="span"
                                  sx={{
                                    width: 7,
                                    height: 7,
                                    borderRadius: "50%",
                                    flexShrink: 0,
                                    bgcolor: toBool(event.success)
                                      ? "var(--ui-rgba-74-210-157-850)"
                                      : "var(--ui-rgba-255-100-100-850)",
                                  }}
                                />
                                <Typography variant="body2" noWrap title={eventLabel}>
                                  {eventLabel}
                                </Typography>
                              </Stack>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap title={toolName || moduleLabel}>
                                {toolName ? titleCaseLabel(toolName) : moduleLabel}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography
                                variant="body2"
                                className="diagnostics-cell-clamp diagnostics-cell-clamp--2"
                                title={outcome}
                                sx={{ color: toBool(event.success) ? "text.secondary" : "error.main" }}
                              >
                                {outcome}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap>
                                {event.latency_ms == null ? "-" : formatTraceDuration(event.latency_ms)}
                              </Typography>
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
                <Stack
                  direction="row"
                  spacing={1}
                  useFlexGap
                  className="trace-table-footer"
                  sx={{ justifyContent: "space-between", alignItems: "center", flexWrap: "wrap" }}
                >
                  <Typography variant="caption" sx={{ color: "text.secondary" }}>
                    Page {activityPage + 1}
                    {(activityPage + 1) * activityPageSize < recentEventsTotal
                      ? " | more activity available"
                      : ""}
                  </Typography>
                  <Stack direction="row" spacing={1}>
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={activityPage === 0}
                      onClick={() => setActivityPage((prev) => Math.max(0, prev - 1))}
                    >
                      Prev
                    </Button>
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={(activityPage + 1) * activityPageSize >= recentEventsTotal}
                      onClick={() => setActivityPage((prev) => prev + 1)}
                    >
                      Next
                    </Button>
                  </Stack>
                </Stack>
              </>
            )}
          </Stack>
        </Box>
      ) : null}
      {traceQ.error || traceDetailQ.error ? (
        <Alert severity="error">
          {errMessage(traceQ.error || traceDetailQ.error)}
        </Alert>
      ) : null}
      {traceSection === "sync" ? (
        <Box className="list-shell diagnostics-section-shell trace-section-shell">
          <Stack spacing={1.25}>
            {/* Compact stats strip */}
            <Stack
              direction="row"
              spacing={0}
              useFlexGap
              className="trace-stats-bar"
              sx={{
                alignItems: "center",
                flexWrap: "wrap",
              }}
            >
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Completed
                </Typography>
                <Typography
                  variant="body2"
                  className="trace-stat-value trace-stat-value--success"
                >
                  {num(syncRunStats.completed_runs, 0)}
                </Typography>
              </Box>
              <Box className="trace-stat-divider" />
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Failed
                </Typography>
                <Typography
                  variant="body2"
                  className="trace-stat-value trace-stat-value--error"
                >
                  {num(syncRunStats.failed_runs, 0)}
                </Typography>
              </Box>
              <Box className="trace-stat-divider" />
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Blocked
                </Typography>
                <Typography
                  variant="body2"
                  className="trace-stat-value trace-stat-value--warn"
                >
                  {num(syncRunStats.blocked_runs, 0)}
                </Typography>
              </Box>
              <Box className="trace-stat-divider" />
              <Box className="trace-stat-pill">
                <Typography variant="caption" className="trace-stat-label">
                  Avg duration
                </Typography>
                <Typography variant="body2" className="trace-stat-value">
                  {formatTraceDuration(syncRunStats.avg_duration_ms)}
                </Typography>
              </Box>
              <Box sx={{ flex: 1 }} />
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                  pr: 0.5,
                }}
              >
                {syncRuns.length} of {syncRunTotal} runs
              </Typography>
            </Stack>

            {syncRunsQ.error ? (
              <Alert severity="warning">{errMessage(syncRunsQ.error)}</Alert>
            ) : syncRuns.length === 0 ? (
              <Alert severity="info">No sync runs recorded yet.</Alert>
            ) : (
              <>
                <TableContainer className="table-shell diagnostics-table-shell trace-table-full">
                  <Table size="small" sx={{ tableLayout: "fixed" }}>
                    <TableHead>
                      <TableRow>
                        <TableCell width="13%">Started</TableCell>
                        <TableCell width="14%">Integration</TableCell>
                        <TableCell width="9%">Trigger</TableCell>
                        <TableCell width="10%">Status</TableCell>
                        <TableCell width="30%">Summary</TableCell>
                        <TableCell width="12%">Items</TableCell>
                        <TableCell width="12%">Duration</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {syncRuns.map((item, idx) => {
                        const id = str(item.id, `sync-run-${idx}`);
                        const status = str(item.status, "completed");
                        return (
                          <TableRow
                            key={id}
                            hover
                            onClick={() => setSelectedSyncRunId(id)}
                            sx={{ cursor: "pointer" }}
                          >
                            <TableCell>
                              <Typography
                                variant="body2"
                                noWrap
                                title={humanTs(str(item.started_at)).tip}
                              >
                                {humanTs(str(item.started_at)).label}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography
                                variant="body2"
                                noWrap
                                title={str(item.integration_name)}
                              >
                                {str(item.integration_name)}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap>
                                {syncRunTriggerLabel(str(item.trigger))}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Chip
                                size="small"
                                color={syncRunStatusColor(status)}
                                label={status}
                              />
                            </TableCell>
                            <TableCell>
                              <Typography
                                variant="body2"
                                className="diagnostics-cell-clamp diagnostics-cell-clamp--2"
                                title={str(item.summary)}
                              >
                                {str(item.summary)}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap>
                                {num(item.new_item_count, 0)} new /{" "}
                                {num(item.important_item_count, 0)} imp
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography variant="body2" noWrap>
                                {formatTraceDuration(item.duration_ms)}
                              </Typography>
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
                <Stack
                  direction="row"
                  useFlexGap
                  className="trace-table-footer"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center",
                    flexWrap: "wrap",
                  }}
                >
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Page {Math.min(syncRunPage + 1, syncRunPages)} of{" "}
                    {syncRunPages}
                  </Typography>
                  <Stack direction="row" spacing={1}>
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={syncRunPage === 0}
                      onClick={() =>
                        setSyncRunPage((prev) => Math.max(0, prev - 1))
                      }
                    >
                      Prev
                    </Button>
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={syncRunPage >= syncRunPages - 1}
                      onClick={() =>
                        setSyncRunPage((prev) =>
                          Math.min(syncRunPages - 1, prev + 1),
                        )
                      }
                    >
                      Next
                    </Button>
                  </Stack>
                </Stack>
              </>
            )}
          </Stack>
        </Box>
      ) : null}
      <Dialog
        open={selectedOperationalEvent != null}
        onClose={() => setSelectedOperationalEvent(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            className:
              "diagnostics-dialog-shell diagnostics-dialog-shell--trace",
          },
        }}
      >
        <DialogTitle
          className="diagnostics-dialog-title"
          sx={{
            display: "flex",
            alignItems: "flex-start",
            justifyContent: "space-between",
            gap: 2,
          }}
        >
          <Box>
            <Typography variant="h6">Runtime Event</Typography>
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              <span title={selectedOperationalEventTime.tip}>
                {selectedOperationalEventTime.label}
              </span>
              {selectedOperationalEvent
                ? ` | ${formatChannelSource(str(selectedOperationalEvent.channel, "agentark"))}`
                : ""}
            </Typography>
          </Box>
          <IconButton
            size="small"
            className="diagnostics-dialog-close"
            onClick={() => setSelectedOperationalEvent(null)}
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers className="diagnostics-dialog-content">
          {!selectedOperationalEvent ? (
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Event details are not available.
            </Typography>
          ) : (
            <Stack spacing={1.5}>
              <Stack
                direction="row"
                spacing={1}
                useFlexGap
                className="trace-detail-status-bar"
                sx={{ alignItems: "center", flexWrap: "wrap" }}
              >
                <Chip
                  size="small"
                  color={toBool(selectedOperationalEvent.success) ? "success" : "error"}
                  label={toBool(selectedOperationalEvent.success) ? "allowed" : "attention"}
                />
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  {titleCaseLabel(str(selectedOperationalEvent.event_type, "activity"))}
                </Typography>
                <Typography variant="caption" sx={{ color: "text.secondary", mx: -0.25 }}>
                  |
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  {selectedOperationalEvent.latency_ms == null
                    ? "-"
                    : formatTraceDuration(selectedOperationalEvent.latency_ms)}
                </Typography>
                <Box sx={{ flex: 1 }} />
                <Typography variant="caption" className="diagnostics-keyline">
                  {str(selectedOperationalEvent.id).slice(0, 18)}
                </Typography>
              </Stack>

              <Box className="diagnostics-content-card">
                <Typography variant="caption" className="diagnostics-card-label">
                  Outcome
                </Typography>
                <Typography variant="body2" className="diagnostics-card-copy">
                  {str(selectedOperationalEvent.outcome, "Activity recorded")}
                </Typography>
              </Box>

              <Grid2 container spacing={1}>
                <Grid2 size={{ xs: 12, sm: 6 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography variant="caption" className="diagnostics-stat-label">
                      Source
                    </Typography>
                    <Typography variant="body2">
                      {formatChannelSource(str(selectedOperationalEvent.source, "Runtime"))}
                    </Typography>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 12, sm: 6 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography variant="caption" className="diagnostics-stat-label">
                      Module
                    </Typography>
                    <Typography variant="body2">
                      {selectedOperationalEvent.tool_name
                        ? titleCaseLabel(str(selectedOperationalEvent.tool_name))
                        : formatChannelSource(str(selectedOperationalEvent.channel, "agentark"))}
                    </Typography>
                  </Box>
                </Grid2>
                {selectedOperationalEvent.trace_id ? (
                  <Grid2 size={{ xs: 12 }}>
                    <Box className="metadata-box diagnostics-stat-card">
                      <Typography variant="caption" className="diagnostics-stat-label">
                        Trace
                      </Typography>
                      <Typography variant="body2">
                        {str(selectedOperationalEvent.trace_id)}
                      </Typography>
                    </Box>
                  </Grid2>
                ) : null}
              </Grid2>

              {selectedOperationalEventDetails ? (
                <Box className="diagnostics-content-card">
                  <Typography variant="caption" className="diagnostics-card-label">
                    Details
                  </Typography>
                  <Typography
                    component="pre"
                    variant="body2"
                    className="diagnostics-card-copy diagnostics-card-copy--scroll"
                    sx={{ whiteSpace: "pre-wrap", m: 0 }}
                  >
                    {selectedOperationalEventDetails}
                  </Typography>
                </Box>
              ) : null}
            </Stack>
          )}
        </DialogContent>
        <DialogActions className="diagnostics-dialog-actions">
          {selectedOperationalEvent?.trace_id ? (
            <Button
              onClick={() => {
                const traceId = str(selectedOperationalEvent.trace_id, "");
                setSelectedOperationalEvent(null);
                setSelectedTraceId(traceId);
              }}
            >
              Open Trace
            </Button>
          ) : null}
          <Button onClick={() => setSelectedOperationalEvent(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={selectedTraceId != null}
        onClose={() => setSelectedTraceId(null)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: {
            className:
              "diagnostics-dialog-shell diagnostics-dialog-shell--trace",
          },
        }}
      >
        <DialogTitle
          className="diagnostics-dialog-title"
          sx={{
            display: "flex",
            alignItems: "flex-start",
            justifyContent: "space-between",
            gap: 2,
          }}
        >
          <Box>
            <Typography variant="h6">Trace Detail</Typography>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              <span title={selectedTraceStarted.tip}>
                {selectedTraceStarted.label}
              </span>{" "}
              | {selectedTraceSource}
            </Typography>
          </Box>
          <IconButton
            size="small"
            className="diagnostics-dialog-close"
            onClick={() => setSelectedTraceId(null)}
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers className="diagnostics-dialog-content">
          {traceDetailQ.isLoading ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              Loading trace...
            </Typography>
          ) : (
            <Stack spacing={1.5}>
              {/* Compact status bar — replaces the old intro blob */}
              <Stack
                direction="row"
                spacing={1}
                useFlexGap
                className="trace-detail-status-bar"
                sx={{
                  alignItems: "center",
                  flexWrap: "wrap",
                }}
              >
                <Chip
                  size="small"
                  color={traceStatusColor(selectedTraceStatus)}
                  label={selectedTraceStatus}
                />
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  {num(selectedTrace.step_count, steps.length)} steps
                </Typography>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                    mx: -0.25,
                  }}
                >
                  |
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  {formatTraceDuration(selectedTrace.duration_ms)}
                </Typography>
                {selectedTrace.total_tokens ? (
                  <>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        mx: -0.25,
                      }}
                    >
                      |
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      {num(selectedTrace.total_tokens, 0)} tokens
                    </Typography>
                  </>
                ) : null}
                {str(selectedTrace.model) ? (
                  <>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        mx: -0.25,
                      }}
                    >
                      |
                    </Typography>
                    <Typography variant="body2" className="diagnostics-keyline">
                      {str(selectedTrace.model)}
                    </Typography>
                  </>
                ) : null}
                <Box sx={{ flex: 1 }} />
                {selectedTraceProofId ? (
                  <Typography variant="caption" className="diagnostics-keyline">
                    {selectedTraceProofId.slice(0, 12)}
                  </Typography>
                ) : null}
              </Stack>

              <Box className="diagnostics-content-card diagnostics-content-card--receipt">
                <Stack spacing={1.1}>
                  <Stack
                    direction={{ xs: "column", md: "row" }}
                    spacing={1}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: { xs: "flex-start", md: "center" },
                    }}
                  >
                    <Box>
                      <Typography variant="subtitle2">Run receipt</Typography>
                      <Typography
                        variant="body2"
                        sx={{ color: "text.secondary", mt: 0.25 }}
                      >
                        {traceRunReceipt.summary}
                      </Typography>
                    </Box>
                    <Stack direction="row" spacing={0.5} useFlexGap sx={{ flexWrap: "wrap" }}>
                      {traceRunReceipt.rows.map((row) => (
                        <Chip
                          key={`${row.label}-${row.value}`}
                          size="small"
                          variant="outlined"
                          label={`${row.label}: ${row.value}`}
                        />
                      ))}
                    </Stack>
                  </Stack>

                  <Grid2 container spacing={1}>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <Box className="diagnostics-subcard">
                        <Typography variant="caption" className="diagnostics-card-label">
                          Actions
                        </Typography>
                        <Stack spacing={0.75} sx={{ mt: 0.75 }}>
                          {traceRunReceipt.actions.length === 0 ? (
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              No action records.
                            </Typography>
                          ) : (
                            traceRunReceipt.actions.map((item, index) => (
                              <Stack
                                key={`${item.label}-${index}`}
                                direction="row"
                                spacing={0.75}
                                sx={{ alignItems: "flex-start" }}
                              >
                                {item.status ? (
                                  <Chip
                                    size="small"
                                    color={traceReceiptStatusColor(item.status)}
                                    variant="outlined"
                                    label={item.status}
                                  />
                                ) : null}
                                <Box sx={{ minWidth: 0 }}>
                                  <Typography variant="body2" sx={{ fontWeight: 650 }}>
                                    {item.label}
                                  </Typography>
                                  {item.detail ? (
                                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                      {item.detail}
                                    </Typography>
                                  ) : null}
                                </Box>
                              </Stack>
                            ))
                          )}
                        </Stack>
                      </Box>
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <Box className="diagnostics-subcard">
                        <Typography variant="caption" className="diagnostics-card-label">
                          Evidence and output
                        </Typography>
                        <Stack spacing={0.75} sx={{ mt: 0.75 }}>
                          {[...traceRunReceipt.outputs, ...traceRunReceipt.evidence]
                            .slice(0, 8)
                            .map((item, index) => (
                              <Box key={`${item.label}-${index}`}>
                                <Typography variant="body2" sx={{ fontWeight: 650 }}>
                                  {item.label}
                                </Typography>
                                {item.detail ? (
                                  <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                    {item.detail}
                                  </Typography>
                                ) : null}
                              </Box>
                            ))}
                          {traceRunReceipt.outputs.length === 0 &&
                          traceRunReceipt.evidence.length === 0 ? (
                            <Typography variant="body2" sx={{ color: "text.secondary" }}>
                              No evidence records.
                            </Typography>
                          ) : null}
                        </Stack>
                      </Box>
                    </Grid2>
                    {traceRunReceipt.failures.length > 0 ? (
                      <Grid2 size={{ xs: 12 }}>
                        <Box className="diagnostics-subcard">
                          <Typography variant="caption" className="diagnostics-card-label">
                            Needs attention
                          </Typography>
                          <Stack spacing={0.75} sx={{ mt: 0.75 }}>
                            {traceRunReceipt.failures.map((item, index) => (
                              <Stack
                                key={`${item.label}-${index}`}
                                direction="row"
                                spacing={0.75}
                                sx={{ alignItems: "flex-start" }}
                              >
                                <Chip
                                  size="small"
                                  color={traceReceiptStatusColor(item.status || "failed")}
                                  variant="outlined"
                                  label={item.status || "failed"}
                                />
                                <Box>
                                  <Typography variant="body2" sx={{ fontWeight: 650 }}>
                                    {item.label}
                                  </Typography>
                                  <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                    {item.detail}
                                  </Typography>
                                </Box>
                              </Stack>
                            ))}
                          </Stack>
                        </Box>
                      </Grid2>
                    ) : null}
                  </Grid2>
                </Stack>
              </Box>

              {/* Input / Output side by side on wide screens */}
              <Stack direction={{ xs: "column", md: "row" }} spacing={1.25}>
                <Box
                  className="diagnostics-content-card diagnostics-content-card--input"
                  sx={{ flex: 1 }}
                >
                  <Typography
                    variant="caption"
                    className="diagnostics-card-label"
                  >
                    Input
                  </Typography>
                  <Typography variant="body2" className="diagnostics-card-copy">
                    {str(selectedTrace.message)}
                  </Typography>
                </Box>
                {selectedTraceResponse ? (
                  <Box
                    className="diagnostics-content-card diagnostics-content-card--output"
                    sx={{ flex: 1 }}
                  >
                    <Typography
                      variant="caption"
                      className="diagnostics-card-label"
                    >
                      Output
                    </Typography>
                    <Typography
                      variant="body2"
                      className="diagnostics-card-copy diagnostics-card-copy--scroll"
                    >
                      {selectedTraceResponse}
                    </Typography>
                  </Box>
                ) : null}
              </Stack>

              {evolutionReviewCards.length > 0 ? (
                <Box className="diagnostics-content-card diagnostics-content-card--review">
                  <Typography variant="subtitle2" sx={{ mb: 1 }}>
                    Evolve Review
                  </Typography>
                  <Stack spacing={0.75}>
                    {evolutionReviewCards.map((card) => (
                      <Box key={card.key} className="diagnostics-subcard">
                        <Stack
                          direction="row"
                          spacing={1}
                          useFlexGap
                          sx={{
                            alignItems: "center",
                            flexWrap: "wrap",
                            mb: 0.5,
                          }}
                        >
                          <Typography
                            variant="body2"
                            sx={{
                              fontWeight: 700,
                            }}
                          >
                            {card.title}
                          </Typography>
                          <Chip
                            size="small"
                            color={traceStepColor(card.status)}
                            label={card.status}
                          />
                          {card.chips.map((chip) => (
                            <Chip
                              key={`${card.key}-${chip}`}
                              size="small"
                              variant="outlined"
                              label={chip}
                            />
                          ))}
                        </Stack>
                        {card.detail ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                              whiteSpace: "pre-wrap",
                            }}
                          >
                            {card.detail}
                          </Typography>
                        ) : null}
                        {card.rationale ? (
                          <Typography
                            variant="caption"
                            sx={{
                              display: "block",
                              mt: 0.5,
                              whiteSpace: "pre-wrap",
                            }}
                          >
                            Why: {card.rationale}
                          </Typography>
                        ) : null}
                        {card.evidence ? (
                          <Box
                            component="pre"
                            className="diagnostics-code-block"
                          >
                            {card.evidence}
                          </Box>
                        ) : null}
                      </Box>
                    ))}
                  </Stack>
                </Box>
              ) : null}

              {traceArtifacts.length > 0 ? (
                <Stack
                  direction="row"
                  spacing={0.5}
                  useFlexGap
                  className="diagnostics-artifact-row"
                  sx={{
                    flexWrap: "wrap",
                    alignItems: "center",
                  }}
                >
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Artifacts:
                  </Typography>
                  {traceArtifacts.map((a) => (
                    <Chip key={a} size="small" variant="outlined" label={a} />
                  ))}
                </Stack>
              ) : null}

              {/* Execution timeline — collapsible steps */}
              <Box className="diagnostics-content-card diagnostics-content-card--steps">
                <Stack
                  direction="row"
                  sx={{
                    alignItems: "center",
                    justifyContent: "space-between",
                    mb: 0.75,
                  }}
                >
                  <Typography variant="subtitle2">Execution Steps</Typography>
                  <Stack direction="row" spacing={0.75}>
                    <Chip
                      size="small"
                      variant="outlined"
                      label="Expand all"
                      onClick={() =>
                        setExpandedSteps(new Set(steps.map((_, i) => i)))
                      }
                      sx={{ cursor: "pointer", fontSize: "10.5px" }}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label="Collapse all"
                      onClick={() => setExpandedSteps(new Set())}
                      sx={{ cursor: "pointer", fontSize: "10.5px" }}
                    />
                  </Stack>
                </Stack>
                <Box className="diagnostics-steps-shell diagnostics-steps-timeline">
                  {steps.length === 0 ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      No steps recorded.
                    </Typography>
                  ) : (
                    <Stack spacing={0}>
                      {steps.map((step, idx) => {
                        const consoleView = buildTraceStepConsoleView(
                          selectedTrace,
                          steps,
                          step,
                        );
                        const stepArtifacts = pickTraceStepArtifacts(step);
                        const stepTime = formatTraceStepTime(str(step.time));
                        const isExpanded = expandedSteps.has(idx);
                        const collapsedPreview =
                          consoleView.detail ||
                          summarizeTraceArtifactsInline(stepArtifacts);
                        const hasContent = !!(
                          consoleView.detail ||
                          consoleView.dataText ||
                          stepArtifacts.length > 0
                        );
                        const showConsoleData =
                          !!consoleView.dataText &&
                          (stepArtifacts.length === 0 ||
                            isExecutionProofStep(step));
                        const stepSignal =
                          `${str(step.status, "")} ${str(step.type, str(step.step_type, ""))}`.toLowerCase();
                        const stepColor =
                          stepSignal.includes("error") ||
                          stepSignal.includes("fail")
                            ? "var(--ui-rgba-255-100-100-850)"
                            : stepSignal.includes("success") ||
                                stepSignal.includes("complete")
                              ? "var(--ui-rgba-74-210-157-850)"
                              : stepSignal.includes("warning") ||
                                  stepSignal.includes("blocked")
                                ? "var(--ui-rgba-255-211-106-850)"
                                : stepSignal.includes("think") ||
                                    stepSignal.includes("reason")
                                  ? "var(--ui-rgba-255-211-106-850)"
                                  : "var(--ui-rgba-120-160-210-500)";
                        return (
                          <Box
                            key={`${str(step.time, "step")}-${idx}`}
                            className={`diagnostics-step-item diagnostics-step-item--timeline${isExpanded ? " diagnostics-step-item--expanded" : ""}`}
                            onClick={() => {
                              if (!hasContent) return;
                              setExpandedSteps((prev) => {
                                const next = new Set(prev);
                                if (next.has(idx)) next.delete(idx);
                                else next.add(idx);
                                return next;
                              });
                            }}
                            sx={{ cursor: hasContent ? "pointer" : "default" }}
                          >
                            {/* Timeline dot */}
                            <Box
                              component="span"
                              className="diagnostics-step-dot"
                              sx={{ bgcolor: stepColor }}
                            />
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              className="diagnostics-step-head"
                              sx={{
                                alignItems: "baseline",
                              }}
                            >
                              <Typography
                                variant="caption"
                                className="diagnostics-step-time"
                              >
                                {stepTime}
                              </Typography>
                              <Typography
                                variant="body2"
                                className="diagnostics-step-title"
                                sx={{
                                  fontWeight: 600,
                                  flex: 1,
                                }}
                              >
                                {str(step.title)}
                              </Typography>
                              {hasContent ? (
                                <ChevronRightRoundedIcon
                                  fontSize="small"
                                  className={`diagnostics-step-chevron${isExpanded ? " diagnostics-step-chevron--open" : ""}`}
                                />
                              ) : null}
                            </Stack>
                            {isExpanded ? (
                              <Box className="diagnostics-step-body">
                                {consoleView.detail ? (
                                  <Typography
                                    variant="caption"
                                    className="diagnostics-step-detail"
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {consoleView.detail}
                                  </Typography>
                                ) : null}
                                {stepArtifacts.length > 0 ? (
                                  <Stack
                                    spacing={0.75}
                                    className="trace-step-artifacts"
                                  >
                                    {stepArtifacts.map(
                                      (artifact, artifactIdx) => {
                                        const artifactLabel =
                                          traceArtifactLabel(artifact);
                                        const artifactKind =
                                          traceArtifactKindLabel(artifact);
                                        const artifactFormat =
                                          traceArtifactFormat(artifact);
                                        const artifactSummaryText =
                                          traceArtifactSummary(artifact);
                                        const artifactBody =
                                          traceArtifactBody(artifact);
                                        return (
                                          <Box
                                            key={`${artifactLabel}-${artifactIdx}`}
                                            className="trace-artifact-card"
                                          >
                                            <Stack
                                              direction="row"
                                              spacing={0.75}
                                              useFlexGap
                                              className="trace-artifact-head"
                                              sx={{
                                                alignItems: "flex-start",
                                                justifyContent: "space-between",
                                              }}
                                            >
                                              <Box
                                                sx={{ minWidth: 0, flex: 1 }}
                                              >
                                                <Typography
                                                  variant="caption"
                                                  className="trace-artifact-kind"
                                                >
                                                  {artifactKind}
                                                </Typography>
                                                <Typography
                                                  variant="body2"
                                                  className="trace-artifact-label"
                                                >
                                                  {artifactLabel}
                                                </Typography>
                                              </Box>
                                              {artifactFormat ? (
                                                <Chip
                                                  size="small"
                                                  variant="outlined"
                                                  label={artifactFormat}
                                                />
                                              ) : null}
                                            </Stack>
                                            {artifactSummaryText ? (
                                              <Typography
                                                variant="caption"
                                                className="trace-artifact-summary"
                                              >
                                                {artifactSummaryText}
                                              </Typography>
                                            ) : null}
                                            {artifactBody ? (
                                              <Box
                                                component="pre"
                                                className="diagnostics-code-block trace-artifact-pre"
                                              >
                                                {artifactBody}
                                              </Box>
                                            ) : null}
                                          </Box>
                                        );
                                      },
                                    )}
                                  </Stack>
                                ) : null}
                                {showConsoleData ? (
                                  <Box
                                    component="pre"
                                    className="diagnostics-code-block diagnostics-step-code"
                                  >
                                    {consoleView.dataText}
                                  </Box>
                                ) : null}
                              </Box>
                            ) : collapsedPreview ? (
                              <Typography
                                variant="caption"
                                className="diagnostics-step-preview"
                                noWrap
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {collapsedPreview}
                              </Typography>
                            ) : null}
                          </Box>
                        );
                      })}
                    </Stack>
                  )}
                </Box>
              </Box>

              {/* Compact timing footer */}
              <Typography
                variant="caption"
                className="diagnostics-footer-note"
                sx={{
                  color: "text.secondary",
                }}
              >
                Started:{" "}
                <span title={selectedTraceStarted.tip}>
                  {selectedTraceStarted.label}
                </span>
                {selectedTrace.completed_at ? (
                  <>
                    {" | Completed: "}
                    <span title={selectedTraceCompleted.tip}>
                      {selectedTraceCompleted.label}
                    </span>
                  </>
                ) : (
                  ""
                )}
              </Typography>
            </Stack>
          )}
        </DialogContent>
        <DialogActions className="diagnostics-dialog-actions">
          <Button onClick={() => setSelectedTraceId(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={selectedSyncRunId != null}
        onClose={() => setSelectedSyncRunId(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            className:
              "diagnostics-dialog-shell diagnostics-dialog-shell--sync",
          },
        }}
      >
        <DialogTitle
          className="diagnostics-dialog-title"
          sx={{
            display: "flex",
            alignItems: "flex-start",
            justifyContent: "space-between",
            gap: 2,
          }}
        >
          <Box>
            <Typography variant="h6">Integration Sync Run</Typography>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              <span title={selectedSyncRunStarted.tip}>
                {selectedSyncRunStarted.label}
              </span>
              {selectedSyncRun
                ? ` | ${str(selectedSyncRun.integration_name, "-")} | ${syncRunTriggerLabel(str(selectedSyncRun.trigger, ""))}`
                : ""}
            </Typography>
          </Box>
          <IconButton
            size="small"
            className="diagnostics-dialog-close"
            onClick={() => setSelectedSyncRunId(null)}
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers className="diagnostics-dialog-content">
          {!selectedSyncRun ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              Run details are not available on this page.
            </Typography>
          ) : (
            <Stack spacing={1.75}>
              <Box className="diagnostics-dialog-intro">
                <Typography className="diagnostics-dialog-eyebrow">
                  Integration execution
                </Typography>
                <Stack
                  direction={{ xs: "column", lg: "row" }}
                  spacing={1.5}
                  sx={{
                    alignItems: { xs: "flex-start", lg: "center" },
                    justifyContent: "space-between",
                  }}
                >
                  <Box className="diagnostics-section-copy">
                    <Typography
                      variant="h5"
                      className="diagnostics-section-title"
                    >
                      What This Sync Run Shows
                    </Typography>
                    <Typography
                      variant="body2"
                      className="diagnostics-section-description"
                    >
                      This view shows what one sync execution fetched, what
                      changed, whether the integration was connected, and which
                      sample items were captured in that run.
                    </Typography>
                  </Box>
                  <Box className="diagnostics-section-meta">
                    {formatTraceDuration(selectedSyncRun.duration_ms)} |{" "}
                    {str(selectedSyncRun.sync_kind, "activity")}
                  </Box>
                </Stack>
                <Stack
                  direction="row"
                  spacing={1}
                  useFlexGap
                  sx={{
                    alignItems: "center",
                    flexWrap: "wrap",
                    mt: 1.25,
                  }}
                >
                  <Chip
                    size="small"
                    color={syncRunStatusColor(selectedSyncRunStatus)}
                    label={selectedSyncRunStatus || "unknown"}
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={syncRunTriggerLabel(
                      str(selectedSyncRun.trigger, ""),
                    )}
                  />
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {str(selectedSyncRun.integration_name, "-")}
                  </Typography>
                </Stack>
              </Box>

              <Alert
                severity={
                  selectedSyncRunStatus === "failed"
                    ? "error"
                    : selectedSyncRunStatus === "blocked"
                      ? "warning"
                      : "info"
                }
              >
                {str(selectedSyncRun.summary, "No summary available.")}
              </Alert>

              <Grid2 container spacing={1}>
                <Grid2 size={{ xs: 6, sm: 3 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography
                      variant="caption"
                      className="diagnostics-stat-label"
                    >
                      Fetched
                    </Typography>
                    <Typography variant="h6" className="diagnostics-stat-value">
                      {num(selectedSyncRun.fetched_item_count, 0)}
                    </Typography>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 6, sm: 3 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography
                      variant="caption"
                      className="diagnostics-stat-label"
                    >
                      New
                    </Typography>
                    <Typography variant="h6" className="diagnostics-stat-value">
                      {num(selectedSyncRun.new_item_count, 0)}
                    </Typography>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 6, sm: 3 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography
                      variant="caption"
                      className="diagnostics-stat-label"
                    >
                      Recorded
                    </Typography>
                    <Typography variant="h6" className="diagnostics-stat-value">
                      {num(selectedSyncRun.recorded_item_count, 0)}
                    </Typography>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 6, sm: 3 }}>
                  <Box className="metadata-box diagnostics-stat-card">
                    <Typography
                      variant="caption"
                      className="diagnostics-stat-label"
                    >
                      Important
                    </Typography>
                    <Typography variant="h6" className="diagnostics-stat-value">
                      {num(selectedSyncRun.important_item_count, 0)}
                    </Typography>
                  </Box>
                </Grid2>
              </Grid2>

              <Grid2 container spacing={1}>
                <Grid2 size={{ xs: 12, sm: 6 }}>
                  <Box
                    className="diagnostics-content-card"
                    sx={{ minHeight: 110 }}
                  >
                    <Typography
                      variant="caption"
                      className="diagnostics-card-label"
                    >
                      Runtime state
                    </Typography>
                    <Typography
                      variant="body2"
                      className="diagnostics-card-copy"
                    >
                      {toBool(selectedSyncRun.connected)
                        ? "Connected"
                        : "Not connected"}
                      {" | "}
                      {toBool(selectedSyncRun.integration_enabled)
                        ? "Integration enabled"
                        : "Integration disabled"}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                        mt: 0.5,
                      }}
                    >
                      Completed:{" "}
                      <span title={selectedSyncRunCompleted.tip}>
                        {selectedSyncRunCompleted.label}
                      </span>
                    </Typography>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 12, sm: 6 }}>
                  <Box
                    className="diagnostics-content-card"
                    sx={{ minHeight: 110 }}
                  >
                    <Typography
                      variant="caption"
                      className="diagnostics-card-label"
                    >
                      Last detected item
                    </Typography>
                    <Typography
                      variant="body2"
                      className="diagnostics-card-copy"
                    >
                      {str(selectedSyncRun.last_item_at)
                        ? humanTs(str(selectedSyncRun.last_item_at)).label
                        : "None"}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                        mt: 0.5,
                      }}
                    >
                      {toBool(selectedSyncRun.baseline_mode)
                        ? "This run seeded baseline history."
                        : "Normal incremental sync run."}
                    </Typography>
                  </Box>
                </Grid2>
              </Grid2>

              {str(selectedSyncRun.error, "").trim() ? (
                <Alert severity="error">{str(selectedSyncRun.error)}</Alert>
              ) : null}

              {Array.isArray(selectedSyncRun.sample_titles) &&
              selectedSyncRun.sample_titles.length > 0 ? (
                <Box className="diagnostics-content-card">
                  <Typography variant="subtitle2">Captured items</Typography>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Top items seen in this run.
                  </Typography>
                  <Stack spacing={0.5} sx={{ mt: 1 }}>
                    {selectedSyncRun.sample_titles.map((title, index) => (
                      <Typography
                        key={`${selectedSyncRunId}-${index}`}
                        variant="body2"
                      >
                        {index + 1}. {str(title)}
                      </Typography>
                    ))}
                  </Stack>
                </Box>
              ) : null}
            </Stack>
          )}
        </DialogContent>
        <DialogActions className="diagnostics-dialog-actions">
          <Button onClick={() => setSelectedSyncRunId(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      {/* Observability Export Delivery Logs */}
      {traceSection === "exports" ? (
        <Box className="list-shell diagnostics-section-shell trace-section-shell">
          {(() => {
            const successLogs = exportLogs.filter(
              (e) => str(e.level, "").toLowerCase() === "success",
            );
            const errorLogs = exportLogs.filter(
              (e) => str(e.level, "").toLowerCase() === "error",
            );
            const uniqueEvents = new Set(
              exportLogs.map((e) => str(e.event, "")),
            ).size;
            return (
              <Stack spacing={1.25}>
                {/* Compact stats strip */}
                <Stack
                  direction="row"
                  spacing={0}
                  useFlexGap
                  className="trace-stats-bar"
                  sx={{
                    alignItems: "center",
                    flexWrap: "wrap",
                  }}
                >
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Total
                    </Typography>
                    <Typography variant="body2" className="trace-stat-value">
                      {exportLogs.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Success
                    </Typography>
                    <Typography
                      variant="body2"
                      className="trace-stat-value trace-stat-value--success"
                    >
                      {successLogs.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Errors
                    </Typography>
                    <Typography
                      variant="body2"
                      className="trace-stat-value trace-stat-value--error"
                    >
                      {errorLogs.length}
                    </Typography>
                  </Box>
                  <Box className="trace-stat-divider" />
                  <Box className="trace-stat-pill">
                    <Typography variant="caption" className="trace-stat-label">
                      Events
                    </Typography>
                    <Typography variant="body2" className="trace-stat-value">
                      {uniqueEvents}
                    </Typography>
                  </Box>
                  <Box sx={{ flex: 1 }} />
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      pr: 0.5,
                    }}
                  >
                    {exportLogs.length} deliveries
                  </Typography>
                </Stack>
                {exportLogs.length === 0 ? (
                  <Alert severity="info">
                    No export deliveries recorded yet.
                  </Alert>
                ) : (
                  <TableContainer className="table-shell diagnostics-table-shell trace-table-full">
                    <Table size="small" sx={{ tableLayout: "fixed" }}>
                      <TableHead>
                        <TableRow>
                          <TableCell width="15%">Time</TableCell>
                          <TableCell width="10%">Status</TableCell>
                          <TableCell width="15%">Event</TableCell>
                          <TableCell width="48%">Message</TableCell>
                          <TableCell width="12%">Trace</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {exportLogs.slice(0, 20).map((entry, idx) => {
                          const level = str(entry.level, "").toLowerCase();
                          const ts = humanTs(str(entry.timestamp, ""));
                          const traceId = str(entry.trace_id, "").trim();
                          return (
                            <TableRow
                              key={`exp-${str(entry.id, "log")}-${idx}`}
                              hover
                              onClick={() => {
                                if (traceId) {
                                  setTraceSection("history");
                                  setSelectedTraceId(traceId);
                                }
                              }}
                              sx={{ cursor: traceId ? "pointer" : "default" }}
                            >
                              <TableCell>
                                <Typography
                                  variant="body2"
                                  noWrap
                                  title={ts.tip}
                                >
                                  {ts.label}
                                </Typography>
                              </TableCell>
                              <TableCell>
                                <Box
                                  component="span"
                                  sx={{
                                    display: "inline-flex",
                                    alignItems: "center",
                                    gap: 0.75,
                                  }}
                                >
                                  <Box
                                    component="span"
                                    sx={{
                                      width: 7,
                                      height: 7,
                                      borderRadius: "50%",
                                      flexShrink: 0,
                                      bgcolor:
                                        level === "error"
                                          ? "var(--ui-rgba-255-100-100-850)"
                                          : level === "success"
                                            ? "var(--ui-rgba-74-210-157-850)"
                                            : "var(--ui-rgba-180-200-220-500)",
                                    }}
                                  />
                                  <Typography
                                    variant="body2"
                                    noWrap
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {level || "info"}
                                  </Typography>
                                </Box>
                              </TableCell>
                              <TableCell>
                                <Typography variant="body2" noWrap>
                                  {str(entry.event, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell>
                                <Typography
                                  variant="body2"
                                  className="diagnostics-cell-clamp diagnostics-cell-clamp--2"
                                  color={
                                    level === "error"
                                      ? "error"
                                      : "text.secondary"
                                  }
                                  title={str(entry.message, "-")}
                                >
                                  {str(entry.message, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell
                                sx={{
                                  fontFamily: "monospace",
                                  fontSize: "0.76rem",
                                }}
                              >
                                {traceId ? traceId.slice(0, 8) : "-"}
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                )}
              </Stack>
            );
          })()}
        </Box>
      ) : null}
      {traceSection === "security" ? (
        <Box className="list-shell diagnostics-section-shell">
          <Stack spacing={1.25}>
            {securityLogsQ.isLoading ? (
              <Typography variant="body2" sx={{ color: "text.secondary", py: 3, textAlign: "center" }}>Loading security logs...</Typography>
            ) : securityLogsQ.error ? (
              <Alert severity="error">{errMessage(securityLogsQ.error)}</Alert>
            ) : securityLogs.length === 0 ? (
              <Typography variant="body2" sx={{ color: "text.secondary", py: 3, textAlign: "center" }}>No security events recorded yet.</Typography>
            ) : (
              <>
                {/* Stats bar */}
                {(() => {
                  const bySeverity: Record<string, number> = {};
                  const byType: Record<string, number> = {};
                  for (const log of securityLogs) {
                    const s = str(log.severity, "low");
                    const t = str(log.event_type, "other");
                    bySeverity[s] = (bySeverity[s] || 0) + 1;
                    byType[t] = (byType[t] || 0) + 1;
                  }
                  const typeLabels = Object.keys(byType).map((t) => traceSecurityEventTypeLabel(t));
                  const typeValues = Object.values(byType);
                  const typeRows = typeLabels.map((label, i) => ({ label, value: String(typeValues[i]) }));
                  const sevLabels = ["high", "medium", "low"].filter((s) => (bySeverity[s] || 0) > 0);
                  const sevValues = sevLabels.map((s) => bySeverity[s] || 0);
                  const sevRows = sevLabels.map((label, i) => ({ label: label.charAt(0).toUpperCase() + label.slice(1), value: String(sevValues[i]) }));
                  return (
                    <>
                      <Stack direction="row" spacing={0} useFlexGap className="trace-stats-bar" sx={{ alignItems: "center", flexWrap: "wrap" }}>
                        <Box className="trace-stat-pill">
                          <Typography variant="caption" className="trace-stat-label">Total</Typography>
                          <Typography variant="body2" className="trace-stat-value">{securityLogs.length}</Typography>
                        </Box>
                        <Box className="trace-stat-divider" />
                        <Box className="trace-stat-pill">
                          <Typography variant="caption" className="trace-stat-label">High</Typography>
                          <Typography variant="body2" className="trace-stat-value trace-stat-value--error">{bySeverity["high"] || 0}</Typography>
                        </Box>
                        <Box className="trace-stat-divider" />
                        <Box className="trace-stat-pill">
                          <Typography variant="caption" className="trace-stat-label">Medium</Typography>
                          <Typography variant="body2" className="trace-stat-value trace-stat-value--warning">{bySeverity["medium"] || 0}</Typography>
                        </Box>
                        <Box className="trace-stat-divider" />
                        <Box className="trace-stat-pill">
                          <Typography variant="caption" className="trace-stat-label">Low</Typography>
                          <Typography variant="body2" className="trace-stat-value">{bySeverity["low"] || 0}</Typography>
                        </Box>
                        {Object.keys(byType).map((t) => (
                          <Box key={t} sx={{ display: "contents" }}>
                            <Box className="trace-stat-divider" />
                            <Box className="trace-stat-pill">
                              <Typography variant="caption" className="trace-stat-label">{traceSecurityEventTypeLabel(t)}</Typography>
                              <Typography variant="body2" className="trace-stat-value">{byType[t]}</Typography>
                            </Box>
                          </Box>
                        ))}
                      </Stack>
                      <Stack direction={{ xs: "column", md: "row" }} spacing={1.25}>
                        {sevLabels.length > 1 ? (
                          <MetricBarCard
                            className="diagnostics-chart-card"
                            title="By Severity"
                            value={`${bySeverity["high"] || 0} high`}
                            values={sevValues}
                            rows={sevRows}
                            palette={["#ff9b9b", "#ffbf82", "#89d7ab"]}
                            chartHeight={72}
                          />
                        ) : null}
                        {typeLabels.length > 1 ? (
                          <MetricBarCard
                            className="diagnostics-chart-card"
                            title="By Event Type"
                            value={`${typeLabels.length} types`}
                            values={typeValues}
                            rows={typeRows}
                            palette={TRACE_EVENT_TYPE_PALETTE}
                            chartHeight={72}
                          />
                        ) : null}
                      </Stack>
                    </>
                  );
                })()}

                {/* Log rows - clickable */}
                <Stack spacing={0} sx={{ borderTop: "1px solid", borderColor: "divider" }}>
                  {securityLogs.map((row, idx) => {
                    const eventType = str(row.event_type, "");
                    const severity = str(row.severity, "-");
                    const message = str(row.message, "-");
                    const source = str(row.source, "").trim() || "system";
                    const count = Math.max(1, num(row.count, 1));
                    const createdAt = humanTs(str(row.created_at, "-"));
                    const dotColor = severity === "high" ? "var(--ui-rgba-255-100-100-850)" : severity === "medium" ? "var(--ui-rgba-255-191-130-850)" : "var(--ui-rgba-74-210-157-850)";
                    return (
                      <ButtonBase
                        key={`${str(row.created_at, "")}:${eventType}:${idx}`}
                        onClick={() => setSelectedSecurityLog(row)}
                        sx={{ width: "100%", textAlign: "left", justifyContent: "flex-start", px: 0, py: 0.85, borderBottom: "1px solid", borderColor: "divider", transition: "background 0.15s ease", "&:hover": { background: "var(--ui-rgba-57-208-255-040)" }, display: "block" }}
                      >
                        <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", justifyContent: "space-between" }}>
                          <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", minWidth: 0, flex: 1 }}>
                            <Box component="span" sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, bgcolor: dotColor }} />
                            <Typography variant="body2" sx={{ fontWeight: 600 }}>{traceSecurityEventTypeLabel(eventType)}</Typography>
                            <Typography variant="caption" sx={{ color: "text.secondary" }}>{severity}{count > 1 ? ` \u00b7 ${count}x` : ""}</Typography>
                          </Stack>
                          <Typography variant="caption" sx={{ color: "text.secondary", whiteSpace: "nowrap", flexShrink: 0 }} title={createdAt.tip}>{createdAt.label}</Typography>
                        </Stack>
                        <Typography variant="caption" noWrap sx={{ color: "text.secondary", pl: "15px", display: "block" }}>{message}</Typography>
                      </ButtonBase>
                    );
                  })}
                </Stack>
              </>
            )}
          </Stack>
        </Box>
      ) : null}
      {/* Security log detail dialog */}
      <Dialog
        open={selectedSecurityLog != null}
        onClose={() => setSelectedSecurityLog(null)}
        maxWidth="sm"
        fullWidth
        slotProps={{ paper: { sx: { borderRadius: "8px", border: "1px solid var(--surface-border)", background: "var(--surface-bg-elevated)", boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)" } } }}
      >
        {selectedSecurityLog ? (() => {
          const eventType = str(selectedSecurityLog.event_type, "");
          const severity = str(selectedSecurityLog.severity, "-");
          const message = str(selectedSecurityLog.message, "-");
          const source = str(selectedSecurityLog.source, "").trim() || "system";
          const count = Math.max(1, num(selectedSecurityLog.count, 1));
          const createdAt = humanTs(str(selectedSecurityLog.created_at, "-"));
          const dotColor = severity === "high" ? "var(--ui-rgba-255-100-100-850)" : severity === "medium" ? "var(--ui-rgba-255-191-130-850)" : "var(--ui-rgba-74-210-157-850)";
          return (
            <>
              <DialogTitle sx={{ pb: 0.5, display: "flex", alignItems: "center", gap: 1.5, borderBottom: "1px solid", borderColor: "divider" }}>
                <Box component="span" sx={{ width: 10, height: 10, borderRadius: "50%", flexShrink: 0, bgcolor: dotColor }} />
                <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>{traceSecurityEventTypeLabel(eventType)}</Typography>
                <Chip size="small" label={severity} color={severity === "high" ? "error" : severity === "medium" ? "warning" : "success"} />
              </DialogTitle>
              <DialogContent>
                <Stack spacing={1.5} sx={{ pt: 1.5 }}>
                  <Box className="micro-surface" sx={{ p: 1.5 }}>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{message}</Typography>
                  </Box>
                  <Stack spacing={0.5}>
                    <Stack direction="row" sx={{ justifyContent: "space-between" }}>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>Event type</Typography>
                      <Typography variant="caption">{eventType}</Typography>
                    </Stack>
                    <Stack direction="row" sx={{ justifyContent: "space-between" }}>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>Severity</Typography>
                      <Typography variant="caption">{severity}</Typography>
                    </Stack>
                    <Stack direction="row" sx={{ justifyContent: "space-between" }}>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>Source</Typography>
                      <Typography variant="caption" sx={{ wordBreak: "break-all" }}>{source}</Typography>
                    </Stack>
                    <Stack direction="row" sx={{ justifyContent: "space-between" }}>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>Count</Typography>
                      <Typography variant="caption">{count}</Typography>
                    </Stack>
                    <Stack direction="row" sx={{ justifyContent: "space-between" }}>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>Time</Typography>
                      <Typography variant="caption" title={createdAt.tip}>{createdAt.label}</Typography>
                    </Stack>
                  </Stack>
                </Stack>
              </DialogContent>
              <DialogActions sx={{ borderTop: "1px solid", borderColor: "divider", px: 2.5, py: 1.5 }}>
                <Button variant="outlined" color="secondary" onClick={() => setSelectedSecurityLog(null)}>Close</Button>
              </DialogActions>
            </>
          );
        })() : null}
      </Dialog>
    </WorkspacePageShell>
  );
}
