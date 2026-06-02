import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Stack,
  Switch,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import EvolveHero from "../arkEvolve/EvolveHero";
import {
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";
import {
  DEVELOPER_MODE_EVENT,
  EVOLUTION_DEV_QUERY_LIMIT,
  EVOLUTION_DEV_REFRESH_MS,
  getDeveloperModeEnabled,
  humanizeStatusLabel,
  REFRESH_MS,
} from "./workspaceCore";
import {
  buildEvolutionEvidenceCards,
  buildEvolutionFocusCaseLabel,
  canonicalSkillIdentifier,
  clampPercent,
  EVOLUTION_PAGE_TABS,
  evolutionExperimentStatusText,
  evolutionGainLabel,
  evolutionPatternStatusExplanation,
  evolutionSurfaceAudienceLabel,
  evolutionSurfaceBenefit,
  evolutionSurfaceStableSummary,
  evolutionSurfaceSummary,
  evolutionTraceIdHint,
  EvolutionReviewEvidenceStrip,
  EvolutionRolloutBar,
  EvolutionStatStrip,
  type EvolutionPageTab,
  type EvolutionPatternCard,
  formatTraceDuration,
  learningCandidateReviewEvidence,
  learningEvidenceStatusColor,
  normalizeLearningEvidenceState,
  percentageLabel,
  promptCanaryActionSummary,
  promptCanaryReviewEvidence,
  ratioPercent,
  skillEvolutionActionLabel,
  skillEvolutionAlertSeverity,
  skillEvolutionChipColor,
  skillEvolutionMetricRows,
  skillReviewEvidence,
  stringList,
  summarizeEvolutionPatternRun,
  summarizeLearningEvidenceTools,
  titleCaseLabel,
  uniqueNonEmptyStrings,
} from "./traceEvolutionHelpers";
import {
  formatTimestampForHumans,
  promptCanarySafetyStatusColor,
  promptProposalRiskColor,
} from "./settingsPageHelpers";

type ReadinessDialogState = {
  title: string;
  readiness: JsonRecord;
};

function readinessRecord(value: unknown): JsonRecord | null {
  const record = asRecord(value);
  return Object.keys(record).length > 0 ? record : null;
}

function promptProposalLifecycle(row: JsonRecord): JsonRecord {
  return asRecord(row.lifecycle);
}

function promptProposalLifecycleStatus(row: JsonRecord): string {
  const lifecycle = promptProposalLifecycle(row);
  const status = str(lifecycle.status, "").trim();
  if (status) return status;
  const reviewStatus = str(row.review_status, "open").trim().toLowerCase();
  if (reviewStatus === "approved") return "approved_waiting_for_more_examples";
  if (reviewStatus === "rejected") return "dismissed";
  return "suggested";
}

function promptProposalLifecycleLabel(status: string): string {
  switch (status) {
    case "suggested":
      return "Needs decision";
    case "approved_waiting_for_more_examples":
      return "Collecting samples";
    case "queued_for_background_test":
      return "Queued";
    case "running_background_test":
      return "Running";
    case "background_test_completed":
      return "Candidate ready";
    case "candidate_rejected":
      return "Not promoted";
    case "testing":
      return "Testing";
    case "test_regression":
      return "Stop suggested";
    case "deployed":
      return "Deployed";
    case "rollback_suggested":
      return "Rollback suggested";
    case "rolled_back":
      return "Rolled back";
    case "test_stopped":
      return "Stopped";
    case "blocked":
      return "Blocked";
    case "dismissed":
      return "Dismissed";
    default:
      return humanizeStatusLabel(status || "suggested");
  }
}

function promptProposalLifecycleStep(status: string): number {
  switch (status) {
    case "suggested":
      return 0;
    case "approved_waiting_for_more_examples":
    case "blocked":
      return 1;
    case "queued_for_background_test":
    case "running_background_test":
    case "background_test_completed":
    case "candidate_rejected":
      return 2;
    case "testing":
    case "test_regression":
    case "test_stopped":
      return 3;
    case "deployed":
    case "rollback_suggested":
    case "rolled_back":
      return 4;
    default:
      return 1;
  }
}

function promptProposalLifecycleColor(status: string) {
  if (status === "deployed") return "success" as const;
  if (status === "testing" || status === "queued_for_background_test" || status === "running_background_test") {
    return "warning" as const;
  }
  if (status === "blocked" || status === "candidate_rejected" || status === "rolled_back" || status === "dismissed" || status === "test_regression" || status === "rollback_suggested") {
    return "error" as const;
  }
  return "default" as const;
}

function promptProposalSampleLabel(lifecycle: JsonRecord): string {
  const samples = num(lifecycle.sample_count, 0);
  const required = num(lifecycle.required_samples, 0);
  if (required > 0) return `${samples.toLocaleString()} / ${required.toLocaleString()} samples`;
  return `${samples.toLocaleString()} samples`;
}

function readinessChipColor(stage: string) {
  if (stage === "auto_ready") return "success" as const;
  if (stage === "review_ready") return "info" as const;
  return "warning" as const;
}

function readinessShortLabel(readiness: JsonRecord | null) {
  if (!readiness) return "Evidence: unavailable";
  const label = str(readiness.label, "");
  const score = num(readiness.score, NaN);
  const scoreText = Number.isFinite(score) ? ` ${Math.round(score)}%` : "";
  return `${label || "Still learning"}${scoreText}`;
}

function readinessSummary(readiness: JsonRecord | null) {
  if (!readiness) return "";
  return str(readiness.plain_summary, "");
}

function recordList(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => asRecord(item))
    .filter((item) => Object.keys(item).length > 0);
}

function formatMachineReasonCode(value: string): string {
  const normalized = value
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_\s-]+/g, " ")
    .trim()
    .toLowerCase();
  if (!normalized) return "";
  return normalized
    .split(" ")
    .filter(Boolean)
    .map((word, idx) => {
      if (word === "api") return "API";
      if (word === "gepa") return "GEPA";
      if (idx === 0) return `${word.charAt(0).toUpperCase()}${word.slice(1)}`;
      return word;
    })
    .join(" ");
}

function promptLifecycleBlockerMessage(reason: string): string | null {
  switch (reason.trim()) {
    case "learning_paused":
      return "Background optimization cannot start yet. Evolve is paused, so AgentArk will wait until Evolve is resumed.";
    case "gepa_disabled":
      return "Background optimization cannot start yet. The GEPA background optimizer is disabled in settings.";
    case "model_or_runtime_not_ready":
      return "Background optimization cannot start yet. Model or optimizer setup is incomplete; finish setup, then retry.";
    case "runtime_not_ready":
      return "Background optimization cannot start yet. The optimizer runtime is not ready; finish the Python/GEPA setup, then retry.";
    case "model_not_ready":
      return "Background optimization cannot start yet. The model or provider key is not ready; finish model setup, then retry.";
    case "budget_paused":
      return "Background optimization cannot start yet. The daily spending limit for background optimization is paused or exhausted, so AgentArk will wait instead of spending more budget.";
    case "work_already_scheduled":
      return "Another background optimization is already scheduled. Wait for it to finish before starting another one.";
    default:
      return null;
  }
}

function formatPromptLifecycleReason(reason: string): string {
  const raw = reason.trim();
  if (!raw) return "";
  const codeOnly = raw.match(/^[a-z][a-z0-9]*(?:[_-][a-z0-9]+)+$/i)?.[0] ?? "";
  const trailingCode =
    raw.match(/(?:^|:\s*|\s+)([a-z][a-z0-9]*(?:[_-][a-z0-9]+)+)\.?$/i)?.[1] ?? "";
  const blockerMessage = promptLifecycleBlockerMessage(codeOnly || trailingCode);
  if (blockerMessage) return blockerMessage;
  return raw
    .replace(/`?([a-z][a-z0-9]*(?:[_-][a-z0-9]+)+)`?/gi, (_match, token: string) =>
      formatMachineReasonCode(token),
    )
    .replace(/\s+\./g, ".");
}

function backgroundImprovementReason(reason: string) {
  const blockerMessage = promptLifecycleBlockerMessage(reason);
  if (blockerMessage) return blockerMessage;
  switch (reason) {
    case "learning_paused":
      return "Evolve is paused.";
    case "gepa_disabled":
      return "GEPA background optimizer is disabled.";
    case "model_or_runtime_not_ready":
      return "Finish model setup before background improvements can start.";
    case "budget_paused":
      return "Paused by the daily cost guardrail.";
    case "work_already_scheduled":
      return "A background improvement is already scheduled.";
    case "waiting_for_quiet_time":
      return "Waiting until AgentArk is quiet.";
    case "cooling_down":
      return "The last check ran recently.";
    case "waiting_for_more_evidence":
      return "Collecting more completed work before the next check.";
    case "queued_for_quiet_time":
      return "Queued and waiting for quiet time.";
    case "blocked":
      return "The last background improvement was blocked by readiness or budget gates.";
    case "failed":
      return "The last background improvement failed.";
    case "timed_out":
      return "The last background improvement timed out.";
    case "retry_pending":
      return "A failed background improvement will retry after AgentArk is quiet.";
    case "completed":
      return "The last background improvement completed.";
    default:
      return reason ? "Background improvement is waiting." : "Watching recent work.";
  }
}

type ExperimentSurfaceItem = {
  key: string;
  name: string;
  audienceLabel: string;
  summary: string;
  benefit: string;
  stableSummary: string;
  enabled: boolean;
  rollout: number;
  baseline: string;
  candidate: string;
  gate: string;
  last: string;
  metrics: JsonRecord[];
  canaryState: JsonRecord;
  primaryMetricLabel: string;
  replayGateReasons: JsonRecord[];
  stopAction?: JsonRecord;
  acceptAction?: JsonRecord;
  rollbackAction?: JsonRecord;
  rollbackAvailable?: boolean;
};

type ExperimentMetricSummary = {
  label: string;
  value: string;
  helper: string;
  tone?: "default" | "good" | "warn" | "info";
  /** Big numeric portion rendered large + tabular. When absent, `value` is shown muted. */
  accent?: string;
  /** Small dim unit suffix after `accent` (e.g. "tokens", "chars"). */
  unit?: string;
  /** 0..1 ratio rendered as a thin gauge bar under the value (e.g. confidence). */
  progress?: number | null;
};

type PromptDetailTab =
  | "proposal"
  | "background"
  | "candidate"
  | "deployment"
  | "monitoring";

function EvolutionLifecycle({
  steps,
  activeIndex,
}: {
  steps: string[];
  activeIndex: number;
}) {
  // Compact breadcrumb-style lifecycle. Current stage renders as a
  // small filled pill in AgentArk green; past stages are dim primary
  // mono text; future stages are dim secondary mono text; chevrons
  // separate them. Reads as a single progression line instead of five
  // evenly-spaced labels with a faint dot strip above.
  const ACTIVE_FG = "#78f2b0";
  const ACTIVE_BG = "rgba(120, 242, 176, 0.10)";
  const ACTIVE_BORDER = "rgba(120, 242, 176, 0.35)";
  const PAST_FG = "rgba(220, 224, 232, 0.78)";
  const FUTURE_FG = "rgba(184, 191, 201, 0.45)";
  const CHEVRON_FG = "rgba(184, 191, 201, 0.32)";
  return (
    <Box
      sx={{
        display: "flex",
        alignItems: "center",
        gap: 0.85,
        py: 0.35,
        flexWrap: "wrap",
        rowGap: 0.6,
      }}
    >
      <Box
        component="span"
        sx={{
          color: "rgba(184, 191, 201, 0.5)",
          fontFamily: "var(--font-mono)",
          fontSize: "0.62rem",
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          mr: 0.25,
        }}
      >
        Stage
      </Box>
      {steps.map((step, idx) => {
        const isActive = idx === activeIndex;
        const isPast = idx < activeIndex;
        const color = isActive ? ACTIVE_FG : isPast ? PAST_FG : FUTURE_FG;
        return (
          <Box
            key={`${step}-${idx}`}
            component="span"
            sx={{ display: "inline-flex", alignItems: "center", gap: 0.85 }}
          >
            <Box
              component="span"
              sx={{
                display: "inline-flex",
                alignItems: "center",
                gap: 0.5,
                px: isActive ? 0.85 : 0,
                py: isActive ? 0.2 : 0,
                borderRadius: 999,
                border: isActive ? `1px solid ${ACTIVE_BORDER}` : "none",
                background: isActive ? ACTIVE_BG : "transparent",
                color,
                fontFamily: "var(--font-mono)",
                fontSize: "0.68rem",
                fontWeight: isActive ? 600 : 500,
                letterSpacing: "0.04em",
                textTransform: "lowercase",
                whiteSpace: "nowrap",
                lineHeight: 1.5,
              }}
            >
              {step}
            </Box>
            {idx < steps.length - 1 ? (
              <Box
                component="span"
                aria-hidden="true"
                sx={{
                  color: CHEVRON_FG,
                  fontSize: "0.78rem",
                  lineHeight: 1,
                  userSelect: "none",
                }}
              >
                ›
              </Box>
            ) : null}
          </Box>
        );
      })}
    </Box>
  );
}

function finiteNumber(value: unknown): number | null {
  const parsed = num(value, Number.NaN);
  return Number.isFinite(parsed) ? parsed : null;
}

function findVersionMetric(rows: JsonRecord[], version: string): JsonRecord | null {
  const target = version.trim();
  if (!target || target === "-") return null;
  return (
    rows.find((row) => str(row.version, "").trim() === target) ?? null
  );
}

function findMatchingCanarySafetyEvent(
  rows: JsonRecord[],
  item: ExperimentSurfaceItem,
): JsonRecord | null {
  const candidate = item.candidate.trim();
  const baseline = item.baseline.trim();
  if (!candidate || candidate === "-") return null;
  return (
    rows.find((row) => {
      const rowCandidate = str(row.candidate_version, "").trim();
      if (rowCandidate !== candidate) return false;
      const rowBaseline = str(row.baseline_version, "").trim();
      return !baseline || baseline === "-" || !rowBaseline || rowBaseline === baseline;
    }) ?? null
  );
}

function formatSampleCount(value: number | null): string {
  if (value == null) return "No samples yet";
  const count = Math.max(0, Math.round(value));
  return `${count.toLocaleString()} sample${count === 1 ? "" : "s"}`;
}

function formatPercentRatio(value: number | null, digits = 1): string {
  if (value == null) return "-";
  return percentageLabel(value, digits) || "-";
}

function formatApproxMoney(value: number | null): string {
  if (value == null) return "-";
  if (Math.abs(value) < 0.0001) return "$0.0000";
  return `$${value.toFixed(4)}`;
}

function formatEstimatedTokenSavings(value: number | null): string {
  if (value == null || value <= 0) return "Pending";
  return `~${Math.round(value).toLocaleString()} tokens`;
}

function promptOptimizationRankLabel(index: number): string {
  return index === 0 ? "Top opportunity" : `Priority #${index + 1}`;
}

function promptOptimizationShareLabel(opportunity: JsonRecord): string {
  const sectionChars = finiteNumber(opportunity.p95_chars);
  const finalChars = finiteNumber(opportunity.p95_final_prompt_chars);
  if (sectionChars == null || finalChars == null || finalChars <= 0) return "-";
  return formatPercentRatio(Math.min(1, sectionChars / finalChars), 0);
}

function promptOptimizationFootprintValues(opportunity: JsonRecord) {
  const sectionChars = Math.max(0, finiteNumber(opportunity.p95_chars) ?? 0);
  const finalChars = Math.max(
    sectionChars,
    finiteNumber(opportunity.p95_final_prompt_chars) ?? sectionChars,
  );
  const restChars = Math.max(0, finalChars - sectionChars);
  const share = finalChars > 0 ? Math.min(1, sectionChars / finalChars) : 0;
  return {
    sectionChars,
    restChars,
    finalChars,
    sharePercent: Math.round(share * 100),
  };
}

function formatSignedPointDelta(value: number | null): string {
  if (value == null) return "Waiting for data";
  const points = value * 100;
  const sign = points > 0 ? "+" : "";
  return `${sign}${points.toFixed(1)} pts`;
}

function promptProposalOutcome(lifecycle: JsonRecord): {
  label: string;
  helper: string;
  tone: "default" | "good" | "warn" | "info";
} {
  const baselineSuccess = finiteNumber(lifecycle.baseline_success_rate);
  const candidateSuccess = finiteNumber(lifecycle.candidate_success_rate);
  const baselineError = finiteNumber(lifecycle.baseline_error_rate);
  const candidateError = finiteNumber(lifecycle.candidate_error_rate);
  if (toBool(lifecycle.rollback_recommended)) {
    return {
      label: "Needs attention",
      helper: "Monitoring found a rollback signal.",
      tone: "warn",
    };
  }
  if (baselineSuccess != null && candidateSuccess != null) {
    const delta = candidateSuccess - baselineSuccess;
    if (delta >= 0.01) {
      return {
        label: "Improving",
        helper: `${formatSignedPointDelta(delta)} success versus stable.`,
        tone: "good",
      };
    }
    if (delta <= -0.01) {
      return {
        label: "Regressing",
        helper: `${formatSignedPointDelta(delta)} success versus stable.`,
        tone: "warn",
      };
    }
  }
  if (baselineError != null && candidateError != null) {
    const delta = candidateError - baselineError;
    if (delta <= -0.01) {
      return {
        label: "Improving",
        helper: `${formatSignedPointDelta(-delta)} fewer errors.`,
        tone: "good",
      };
    }
    if (delta >= 0.01) {
      return {
        label: "Regressing",
        helper: `${formatSignedPointDelta(delta)} more errors.`,
        tone: "warn",
      };
    }
  }
  return {
    label: "Measuring",
    helper: "Waiting for enough before/after production samples.",
    tone: "info",
  };
}

function promptLifecycleTimeRows(lifecycle: JsonRecord): { label: string; value: string }[] {
  return [
    { label: "Approved", value: str(lifecycle.approved_at, "") },
    { label: "Queued", value: str(lifecycle.queued_at, "") },
    { label: "Deployed", value: str(lifecycle.deployed_at, "") },
  ]
    .filter((row) => row.value.trim())
    .map((row) => ({
      ...row,
      value: formatTimestampForHumans(row.value).label,
    }));
}

function promptMetricCards(lifecycle: JsonRecord): ExperimentMetricSummary[] {
  const baselineSuccess = finiteNumber(lifecycle.baseline_success_rate);
  const candidateSuccess = finiteNumber(lifecycle.candidate_success_rate);
  const successDelta =
    baselineSuccess != null && candidateSuccess != null
      ? candidateSuccess - baselineSuccess
      : null;
  const baselineError = finiteNumber(lifecycle.baseline_error_rate);
  const candidateError = finiteNumber(lifecycle.candidate_error_rate);
  const errorDelta =
    baselineError != null && candidateError != null
      ? candidateError - baselineError
      : null;
  const baselineLatency = finiteNumber(lifecycle.baseline_p95_latency_ms);
  const candidateLatency = finiteNumber(lifecycle.candidate_p95_latency_ms);
  const latencyDelta =
    baselineLatency != null && candidateLatency != null
      ? candidateLatency - baselineLatency
      : null;
  return [
    {
      label: "Outcome",
      value: promptProposalOutcome(lifecycle).label,
      helper: promptProposalOutcome(lifecycle).helper,
      tone: promptProposalOutcome(lifecycle).tone,
    },
    {
      label: "Success",
      value: formatPercentRatio(candidateSuccess),
      helper:
        successDelta == null
          ? `Stable ${formatPercentRatio(baselineSuccess)}`
          : `${formatSignedPointDelta(successDelta)} vs stable`,
      tone:
        successDelta == null ? "info" : successDelta >= 0 ? "good" : "warn",
    },
    {
      label: "Errors",
      value: formatPercentRatio(candidateError),
      helper:
        errorDelta == null
          ? `Stable ${formatPercentRatio(baselineError)}`
          : `${formatSignedPointDelta(errorDelta)} vs stable`,
      tone:
        errorDelta == null ? "info" : errorDelta <= 0 ? "good" : "warn",
    },
    {
      label: "p95 latency",
      value:
        candidateLatency == null
          ? "-"
          : `${Math.round(candidateLatency).toLocaleString()} ms`,
      helper:
        latencyDelta == null
          ? baselineLatency == null
            ? "Stable -"
            : `Stable ${Math.round(baselineLatency).toLocaleString()} ms`
          : `${latencyDelta > 0 ? "+" : ""}${Math.round(latencyDelta).toLocaleString()} ms vs stable`,
      tone:
        latencyDelta == null ? "info" : latencyDelta <= 0 ? "good" : "warn",
    },
  ];
}

function promptLifecycleSamplesOption(
  samples: number,
  requiredSamples: number,
  status: string,
) {
  const required = requiredSamples || samples || 1;
  return {
    backgroundColor: "transparent",
    grid: { left: 34, right: 12, top: 18, bottom: 26, containLabel: true },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(14, 18, 14, 0.96)",
      borderColor: "rgba(120, 242, 176, 0.24)",
      textStyle: { color: "#fff8ed" },
    },
    xAxis: {
      type: "category",
      data: ["Collected", "Needed"],
      axisLabel: { color: "#c8d8c9" },
      axisLine: { lineStyle: { color: "rgba(130,170,160,0.22)" } },
    },
    yAxis: {
      type: "value",
      axisLabel: { color: "#c8d8c9" },
      splitLine: { lineStyle: { color: "rgba(130,170,160,0.12)" } },
    },
    series: [
      {
        type: "bar",
        data: [samples, required],
        itemStyle: {
          color: status === "blocked" ? "#fb7185" : "#78f2b0",
          borderRadius: [4, 4, 0, 0],
        },
      },
    ],
  };
}

function promptMonitoringChartOption(lifecycle: JsonRecord) {
  const labels = ["Success", "Tool success", "Error rate"];
  const stable = [
    ratioPercent(lifecycle.baseline_success_rate),
    ratioPercent(lifecycle.baseline_tool_success_rate),
    ratioPercent(lifecycle.baseline_error_rate),
  ];
  const candidate = [
    ratioPercent(lifecycle.candidate_success_rate),
    ratioPercent(lifecycle.candidate_tool_success_rate),
    ratioPercent(lifecycle.candidate_error_rate),
  ];
  return {
    backgroundColor: "transparent",
    grid: { left: 40, right: 14, top: 34, bottom: 30, containLabel: true },
    legend: { top: 0, textStyle: { color: "#d8d0c4", fontSize: 11 } },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(14, 18, 14, 0.96)",
      borderColor: "rgba(120, 242, 176, 0.24)",
      textStyle: { color: "#fff8ed" },
      valueFormatter: (value: number) => `${Number(value).toFixed(1)}%`,
    },
    xAxis: {
      type: "category",
      data: labels,
      axisLabel: { color: "#c8d8c9" },
    },
    yAxis: {
      type: "value",
      max: 100,
      axisLabel: { color: "#c8d8c9", formatter: "{value}%" },
      splitLine: { lineStyle: { color: "rgba(130,170,160,0.12)" } },
    },
    series: [
      {
        name: "Stable",
        type: "bar",
        data: stable,
        itemStyle: { color: "rgba(148, 163, 184, 0.72)", borderRadius: [4, 4, 0, 0] },
      },
      {
        name: "Candidate",
        type: "bar",
        data: candidate,
        itemStyle: { color: "#14f195", borderRadius: [4, 4, 0, 0] },
      },
    ],
  };
}

function promptOptimizationRowMetrics(opportunity: JsonRecord): ExperimentMetricSummary[] {
  const savedTokens = finiteNumber(opportunity.estimated_saved_tokens_p95);
  const savedCost = finiteNumber(opportunity.estimated_saved_cost_usd_p95);
  const p95Chars = finiteNumber(opportunity.p95_chars);
  const samples = finiteNumber(opportunity.samples);
  const issueRate = finiteNumber(opportunity.issue_rate);
  const confidenceTarget = finiteNumber(opportunity.confidence_sample_target);
  const sampleConfidence = finiteNumber(opportunity.sample_confidence_score);
  const evidenceParts = [
    formatSampleCount(samples),
    confidenceTarget == null
      ? ""
      : `target ${Math.round(confidenceTarget).toLocaleString()}`,
    issueRate == null ? "" : `${formatPercentRatio(issueRate)} issue`,
  ].filter((part) => part.trim());
  const hasSavedTokens = savedTokens != null && savedTokens > 0;
  return [
    {
      label: "Estimated p95 savings",
      value: formatEstimatedTokenSavings(savedTokens),
      helper:
        savedCost == null
          ? "Before GEPA validation"
          : `${formatApproxMoney(savedCost)} p95 cost estimate`,
      tone: "good",
      accent: hasSavedTokens ? `~${Math.round(savedTokens!).toLocaleString()}` : undefined,
      unit: hasSavedTokens ? "tokens" : undefined,
    },
    {
      label: "Prompt weight",
      value: p95Chars == null ? "Not measured" : `${Math.round(p95Chars).toLocaleString()} chars`,
      helper: "Section size at p95",
      tone: "info",
      accent: p95Chars == null ? undefined : Math.round(p95Chars).toLocaleString(),
      unit: p95Chars == null ? undefined : "chars",
    },
    {
      label: "Evidence confidence",
      value: sampleConfidence == null ? formatSampleCount(samples) : formatPercentRatio(sampleConfidence),
      helper: evidenceParts.length > 0 ? evidenceParts.join(", ") : "Matched telemetry",
      tone: sampleConfidence != null && sampleConfidence >= 1 ? "good" : "info",
      accent: sampleConfidence == null ? undefined : formatPercentRatio(sampleConfidence),
      progress: sampleConfidence == null ? null : Math.max(0, Math.min(1, sampleConfidence)),
    },
  ];
}

/**
 * Futuristic-but-clean stat tile: micro uppercase label, a large tabular value
 * (with a dimmed unit), an optional thin confidence gauge, and a muted helper.
 * Deliberately borderless — column separation comes from an optional left
 * hairline divider, not an underline rule per value. AgentArk palette only.
 */
function EvolveStat({
  label,
  value,
  helper,
  tone = "default",
  accent,
  unit,
  progress,
  divider = false,
}: {
  label: string;
  value: string;
  helper?: string;
  tone?: "default" | "good" | "warn" | "info";
  accent?: string;
  unit?: string;
  progress?: number | null;
  divider?: boolean;
}) {
  const valueColor =
    tone === "good"
      ? "#aef7cf"
      : tone === "warn"
        ? "#ffe0b0"
        : tone === "info"
          ? "#eaf4ff"
          : "#e8f4ff";
  const gaugeColor =
    tone === "warn" ? "#ffbe63" : tone === "info" ? "#9cc0ff" : "#14f195";
  const pct =
    progress == null ? null : Math.round(Math.max(0, Math.min(1, progress)) * 100);
  return (
    <Box
      sx={{
        flex: "1 1 0",
        minWidth: 92,
        pl: divider ? { xs: 0, md: 2 } : 0,
        borderLeft: divider
          ? { xs: "none", md: "1px solid var(--ui-rgba-145-170-205-120)" }
          : "none",
      }}
    >
      <Typography
        variant="caption"
        sx={{
          color: "var(--text-faint)",
          display: "block",
          fontFamily: "var(--font-mono)",
          fontSize: "0.55rem",
          letterSpacing: 0.14,
          textTransform: "uppercase",
          lineHeight: 1.1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </Typography>
      <Box
        sx={{
          display: "flex",
          alignItems: "baseline",
          gap: 0.5,
          mt: 0.55,
          minWidth: 0,
        }}
      >
        {accent ? (
          <>
            <Box
              component="span"
              sx={{
                fontFamily: "var(--font-mono)",
                fontWeight: 700,
                fontSize: "1.2rem",
                lineHeight: 1,
                color: valueColor,
                fontVariantNumeric: "tabular-nums",
                textShadow: `0 0 18px ${valueColor}2e`,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {accent}
            </Box>
            {unit ? (
              <Box
                component="span"
                sx={{
                  fontFamily: "var(--font-mono)",
                  fontSize: "0.66rem",
                  letterSpacing: 0.06,
                  color: "var(--text-faint)",
                  textTransform: "uppercase",
                  flex: "0 0 auto",
                }}
              >
                {unit}
              </Box>
            ) : null}
          </>
        ) : (
          <Box
            component="span"
            sx={{
              fontFamily: "var(--font-mono)",
              fontSize: "0.9rem",
              fontWeight: 500,
              color: "var(--text-secondary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {value}
          </Box>
        )}
      </Box>
      {pct != null ? (
        <Box
          sx={{
            mt: 0.7,
            height: 3,
            maxWidth: 132,
            borderRadius: 999,
            background: "var(--ui-rgba-255-255-255-080)",
            overflow: "hidden",
          }}
        >
          <Box
            sx={{
              height: "100%",
              width: `${pct}%`,
              borderRadius: 999,
              background: `linear-gradient(90deg, ${gaugeColor}59, ${gaugeColor})`,
              boxShadow: `0 0 8px ${gaugeColor}5e`,
            }}
          />
        </Box>
      ) : null}
      {helper ? (
        <Typography
          variant="caption"
          sx={{
            color: "var(--text-secondary)",
            display: "block",
            lineHeight: 1.25,
            fontSize: "0.58rem",
            mt: 0.55,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {helper}
        </Typography>
      ) : null}
    </Box>
  );
}

function promptOpportunityIssueChartOption(opportunity: JsonRecord) {
  const labels = ["Failed", "Corrected", "Slow", "Expensive"];
  const values = [
    num(opportunity.failed_samples, 0),
    num(opportunity.corrected_samples, 0),
    num(opportunity.slow_samples, 0),
    num(opportunity.expensive_samples, 0),
  ];
  return {
    backgroundColor: "transparent",
    grid: { left: 30, right: 10, top: 12, bottom: 24, containLabel: true },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(14, 18, 14, 0.96)",
      borderColor: "rgba(120, 242, 176, 0.24)",
      textStyle: { color: "#fff8ed" },
    },
    xAxis: {
      type: "category",
      data: labels,
      axisLabel: { color: "#c8d8c9", fontSize: 10 },
      axisLine: { lineStyle: { color: "rgba(130,170,160,0.22)" } },
    },
    yAxis: {
      type: "value",
      axisLabel: { color: "#c8d8c9", fontSize: 10 },
      splitLine: { lineStyle: { color: "rgba(130,170,160,0.12)" } },
    },
    series: [
      {
        type: "bar",
        data: values,
        itemStyle: { color: "#14f195", borderRadius: [4, 4, 0, 0] },
      },
    ],
  };
}

function promptHoldoutLatencyChartOption(cases: JsonRecord[]) {
  const visibleCases = cases.slice(0, 5);
  const labels = visibleCases.map((_caseRow, idx) => `S${idx + 1}`);
  const values = visibleCases.map((caseRow) =>
    Math.max(0, finiteNumber(caseRow.latency_ms) ?? 0),
  );
  return {
    backgroundColor: "transparent",
    grid: { left: 40, right: 10, top: 12, bottom: 24, containLabel: true },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(14, 18, 14, 0.96)",
      borderColor: "rgba(120, 242, 176, 0.24)",
      textStyle: { color: "#fff8ed" },
      formatter: (items: Array<{ dataIndex: number; value: number }>) => {
        const first = items[0];
        const caseRow = visibleCases[first?.dataIndex ?? 0] ?? {};
        const label = humanizeStatusLabel(str(caseRow.outcome, "sample"));
        const latency = Math.round(Number(first?.value ?? 0)).toLocaleString();
        return `${label}<br/>${latency} ms`;
      },
    },
    xAxis: {
      type: "category",
      data: labels,
      axisLabel: { color: "#c8d8c9" },
      axisLine: { lineStyle: { color: "rgba(130,170,160,0.22)" } },
    },
    yAxis: {
      type: "value",
      axisLabel: {
        color: "#c8d8c9",
        fontSize: 10,
        formatter: (value: number) => `${Math.round(value / 1000)}k`,
      },
      splitLine: { lineStyle: { color: "rgba(130,170,160,0.12)" } },
    },
    series: [
      {
        type: "bar",
        data: values,
        itemStyle: {
          color: (params: { dataIndex: number }) => {
            const outcome = str(
              visibleCases[params.dataIndex]?.outcome,
              "",
            ).toLowerCase();
            if (outcome.includes("expensive")) return "#fbbf24";
            if (outcome.includes("slow")) return "#60a5fa";
            if (outcome.includes("failed")) return "#fb7185";
            if (outcome.includes("corrected")) return "#a78bfa";
            return "#14f195";
          },
          borderRadius: [4, 4, 0, 0],
        },
      },
    ],
  };
}

function promptHoldoutCaseLabel(row: JsonRecord): string {
  const traceId = str(row.trace_id, "").trim();
  const runId = str(row.run_id, "").trim();
  return traceId || runId || "Sample";
}

function PromptDetailSection({
  tab,
  active,
  children,
}: {
  tab: PromptDetailTab;
  active: PromptDetailTab;
  children: ReactNode;
}) {
  if (tab !== active) return null;
  return <Stack spacing={1.25}>{children}</Stack>;
}

function ResultSummaryCard({
  label,
  value,
  helper,
  tone = "default",
}: {
  label: string;
  value: string;
  helper: string;
  tone?: "default" | "good" | "warn" | "info";
}) {
  const accent =
    tone === "good"
      ? "#14f195"
      : tone === "warn"
        ? "#fbbf24"
        : tone === "info"
          ? "#78f2b0"
          : "#c8d8c9";
  return (
    <Box
      sx={{
        minWidth: 0,
        p: 1,
        border: "1px solid var(--ui-rgba-145-170-205-120)",
        borderRadius: 1,
        bgcolor: "rgba(8, 14, 24, 0.34)",
        borderLeft: `3px solid ${accent}`,
      }}
    >
      <Typography
        variant="caption"
        sx={{ color: "text.secondary", display: "block" }}
      >
        {label}
      </Typography>
      <Typography
        variant="h6"
        sx={{ color: "#e8f4ff", fontWeight: 750, lineHeight: 1.2, mt: 0.25 }}
      >
        {value}
      </Typography>
      <Typography
        variant="caption"
        sx={{ color: "text.secondary", display: "block", lineHeight: 1.35 }}
      >
        {helper}
      </Typography>
    </Box>
  );
}

function ResultProgressRow({
  label,
  value,
  helper,
  color,
}: {
  label: string;
  value: number;
  helper: string;
  color: string;
}) {
  return (
    <Box sx={{ minWidth: 0 }}>
      <Stack
        direction="row"
        spacing={1}
        sx={{ justifyContent: "space-between", alignItems: "baseline" }}
      >
        <Typography
          variant="body2"
          sx={{ color: "#e8f4ff", fontWeight: 650, minWidth: 0 }}
          noWrap
          title={label}
        >
          {label}
        </Typography>
        <Typography variant="caption" sx={{ color: "text.secondary" }}>
          {value.toFixed(1)}%
        </Typography>
      </Stack>
      <Box
        sx={{
          mt: 0.45,
          height: 7,
          overflow: "hidden",
          borderRadius: 999,
          bgcolor: "rgba(148, 163, 184, 0.16)",
        }}
      >
        <Box
          sx={{
            width: `${Math.max(0, Math.min(100, value))}%`,
            height: "100%",
            borderRadius: 999,
            bgcolor: color,
          }}
        />
      </Box>
      <Typography
        variant="caption"
        sx={{ color: "text.secondary", display: "block", mt: 0.35 }}
      >
        {helper}
      </Typography>
    </Box>
  );
}

function formatLatencyMs(value: unknown): string {
  const parsed = finiteNumber(value);
  if (parsed == null) return "";
  if (parsed >= 1000) {
    const seconds = parsed / 1000;
    return `${seconds >= 10 ? seconds.toFixed(0) : seconds.toFixed(1)}s`;
  }
  return `${Math.round(parsed).toLocaleString()}ms`;
}

function promotionGateSummary(data: JsonRecord): string {
  const report = asRecord(data.promotion_gate_report);
  return (
    str(report.summary, "").trim() ||
    str(data.promotion_gate_summary, "").trim() ||
    str(data.promotion_gate, "").trim()
  );
}

function replayGateReasonLabels(item: ExperimentSurfaceItem): string[] {
  return item.replayGateReasons
    .map((reason) => str(reason.label, "").trim())
    .filter(Boolean);
}

function buildExperimentMetricSummaries(
  item: ExperimentSurfaceItem,
  safetyEvent: JsonRecord | null,
): ExperimentMetricSummary[] {
  const baselineMetric = findVersionMetric(item.metrics, item.baseline);
  const candidateMetric = findVersionMetric(item.metrics, item.candidate);
  const baselineSuccess =
    finiteNumber(safetyEvent?.baseline_success_rate) ??
    finiteNumber(baselineMetric?.success_rate);
  const candidateSuccess =
    finiteNumber(safetyEvent?.candidate_success_rate) ??
    finiteNumber(candidateMetric?.success_rate);
  const successDelta =
    finiteNumber(safetyEvent?.success_delta) ??
    (baselineSuccess != null && candidateSuccess != null
      ? candidateSuccess - baselineSuccess
      : null);
  const baselineSamples =
    finiteNumber(safetyEvent?.baseline_samples) ??
    finiteNumber(baselineMetric?.samples);
  const candidateSamples =
    finiteNumber(safetyEvent?.candidate_samples) ??
    finiteNumber(candidateMetric?.samples);
  const baselineError = finiteNumber(baselineMetric?.error_rate);
  const candidateError = finiteNumber(candidateMetric?.error_rate);
  const baselineLatency = formatLatencyMs(baselineMetric?.p95_latency_ms);
  const candidateLatency = formatLatencyMs(candidateMetric?.p95_latency_ms);
  const cards: ExperimentMetricSummary[] = [
    {
      label: "Measures",
      value: item.primaryMetricLabel,
      helper:
        successDelta != null
          ? `${evolutionGainLabel(successDelta)} versus stable`
          : "Comparing the candidate with stable behavior",
      tone:
        successDelta == null
          ? "info"
          : successDelta >= 0
            ? "good"
            : "warn",
    },
    {
      label: "Stable",
      value: formatPercentRatio(baselineSuccess),
      helper: formatSampleCount(baselineSamples),
    },
    {
      label: "Experiment",
      value: formatPercentRatio(candidateSuccess),
      helper: formatSampleCount(candidateSamples),
      tone:
        successDelta == null
          ? "default"
          : successDelta >= 0
            ? "good"
            : "warn",
    },
  ];
  if (successDelta != null) {
    cards.push({
      label: "Difference",
      value: evolutionGainLabel(successDelta),
      helper: "Higher success is better",
      tone: successDelta >= 0 ? "good" : "warn",
    });
  } else if (baselineSamples != null || candidateSamples != null) {
    cards.push({
      label: "Samples",
      value: `${Math.round(baselineSamples ?? 0).toLocaleString()} / ${Math.round(
        candidateSamples ?? 0,
      ).toLocaleString()}`,
      helper: "Stable / experiment",
    });
  }
  if (baselineError != null || candidateError != null) {
    cards.push({
      label: "Error rate",
      value: formatPercentRatio(candidateError),
      helper: `Stable ${formatPercentRatio(baselineError)}`,
      tone:
        baselineError != null && candidateError != null && candidateError <= baselineError
          ? "good"
          : "default",
    });
  }
  if (baselineLatency || candidateLatency) {
    cards.push({
      label: "p95 latency",
      value: candidateLatency || "-",
      helper: `Stable ${baselineLatency || "-"}`,
    });
  }
  return cards.slice(0, 6);
}

function experimentStageText(
  item: ExperimentSurfaceItem,
  safetyEvent: JsonRecord | null,
): string {
  const reviewStatus = str(safetyEvent?.review_status, "").trim();
  if (reviewStatus === "open" || reviewStatus === "review_recommended") {
    return "Needs decision";
  }
  if (reviewStatus) return humanizeStatusLabel(reviewStatus);
  const gate = item.gate.trim();
  if (gate && gate !== "-") {
    return gate === "passed" ? "Gate passed" : "Evaluating gate";
  }
  const candidateSamples = finiteNumber(
    findVersionMetric(item.metrics, item.candidate)?.samples,
  );
  const minSamples = finiteNumber(item.canaryState.min_samples_per_version);
  if (candidateSamples != null && minSamples != null && candidateSamples < minSamples) {
    return "Collecting samples";
  }
  return "Testing candidate";
}

function experimentGuardrailText(item: ExperimentSurfaceItem): string {
  const minSamples = finiteNumber(item.canaryState.min_samples_per_version);
  const minGain = finiteNumber(item.canaryState.min_success_gain);
  const maxP = finiteNumber(item.canaryState.max_sign_test_p_value);
  const rules = [
    minSamples != null && minSamples > 0
      ? `${Math.round(minSamples).toLocaleString()} samples per version`
      : "",
    minGain != null ? `${evolutionGainLabel(minGain)} minimum success lift` : "",
    maxP != null ? `p <= ${maxP.toFixed(2)} sign test` : "",
  ].filter(Boolean);
  if (rules.length === 0) {
    return "Guardrail: candidate stays limited until evidence shows it is safe.";
  }
  return `Guardrail: promotion needs ${rules.join(", ")}.`;
}

function experimentLastActivityText(
  item: ExperimentSurfaceItem,
  safetyEvent: JsonRecord | null,
): string {
  const eventAt = str(safetyEvent?.created_at, "").trim();
  if (eventAt) return `Safety check ${humanTs(eventAt).label}`;
  const activatedAt = str(item.canaryState.activated_at, "").trim();
  if (activatedAt) return `Started ${humanTs(activatedAt).label}`;
  return "Waiting for the first recorded run";
}

export default function EvolutionPage({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<EvolutionPageTab>("review");
  const [showSuperseded, setShowSuperseded] = useState(false);
  const PROMPT_REVIEW_PAGE_SIZE = 6;
  const [promptReviewPage, setPromptReviewPage] = useState(0);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(
    getDeveloperModeEnabled,
  );
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [selectedPatternCard, setSelectedPatternCard] =
    useState<EvolutionPatternCard | null>(null);
  const [technicalDialogProposalId, setTechnicalDialogProposalId] = useState<
    string | null
  >(null);
  const [promptDetailTab, setPromptDetailTab] =
    useState<PromptDetailTab>("proposal");
  const [readinessDialog, setReadinessDialog] =
    useState<ReadinessDialogState | null>(null);
  // Default-closed so novice users see the narrative hero first. The
  // existing tabs and analytics stay one click away for power users.
  const [showDetails, setShowDetails] = useState(false);
  const [optimizationOpen, setOptimizationOpen] = useState(false);

  useEffect(() => {
    const refreshDeveloperMode = () =>
      setDeveloperModeEnabledState(getDeveloperModeEnabled());
    window.addEventListener(
      DEVELOPER_MODE_EVENT,
      refreshDeveloperMode as EventListener,
    );
    window.addEventListener("storage", refreshDeveloperMode);
    return () => {
      window.removeEventListener(
        DEVELOPER_MODE_EVENT,
        refreshDeveloperMode as EventListener,
      );
      window.removeEventListener("storage", refreshDeveloperMode);
    };
  }, []);

  useEffect(() => {
    if (!success) return;
    const timer = window.setTimeout(() => setSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [success]);

  const evolutionQ = useQuery({
    queryKey: ["settings-evolution"],
    queryFn: () => api.rawGet("/settings/evolution"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const evolutionDevQ = useQuery({
    queryKey: ["settings-evolution-dev", showSuperseded],
    queryFn: () =>
      api.rawGet(
        `/settings/evolution/dev?limit=${EVOLUTION_DEV_QUERY_LIMIT}${showSuperseded ? "&include_superseded=true" : ""}`,
      ),
    refetchInterval: autoRefresh ? EVOLUTION_DEV_REFRESH_MS : false,
  });
  const updateEvolutionMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/settings/evolution", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-evolution-dev"],
      });
    },
  });
  const runEvolutionActionMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/settings/evolution/dev/action", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-evolution-dev"],
      });
    },
  });

  const evolution = asRecord(evolutionQ.data);
  const evolutionDev = asRecord(evolutionDevQ.data);
  const canary = asRecord(evolution.canary);
  const strategyCanary = asRecord(evolution.strategy_canary);
  const promptCanary = asRecord(evolution.prompt_canary);
  const specialistCanary = asRecord(evolution.specialist_prompt_canary);
  const promptFragmentCanary = asRecord(evolution.prompt_fragment_canary);
  const learningQueue = asRecord(evolution.learning_queue);
  const gepaConfig = asRecord(evolution.gepa_config);
  const gepaReadiness = asRecord(evolution.gepa_readiness);
  const gepaBudget = asRecord(gepaReadiness.budget);
  const gepaAutoState = asRecord(evolution.gepa_auto_state);
  const gepaLastResult = asRecord(evolution.gepa_last_result);
  const gepaQueue = asRecord(evolution.gepa_queue);
  const gepaIssues = Array.isArray(gepaReadiness.issues)
    ? gepaReadiness.issues
        .map((item) => str(item, ""))
        .map((item) => item.trim())
        .filter(Boolean)
    : [];
  const gepaReady = toBool(gepaReadiness.ready);
  const gepaPendingItems = recordList(gepaQueue.pending);
  const gepaRunningItems = recordList(gepaQueue.running);
  const gepaCompletedItems = recordList(gepaQueue.completed);
  const gepaFailedItems = recordList(gepaQueue.failed);
  const gepaPendingJobs = gepaPendingItems.length;
  const gepaRunningJobs = gepaRunningItems.length;
  const gepaDailyBudgetUsd = num(gepaBudget.daily_budget_usd, 2);
  const gepaRemainingBudgetUsd = num(
    gepaBudget.remaining_today_usd,
    gepaDailyBudgetUsd,
  );
  const gepaBudgetAllowed = toBool(gepaBudget.allowed);
  const latestGepaQueueRecord = [...gepaCompletedItems, ...gepaFailedItems]
    .sort((a, b) =>
      str(b.recorded_at, "").localeCompare(str(a.recorded_at, "")),
    )[0];
  const latestGepaRecord =
    Object.keys(gepaLastResult).length > 0
      ? gepaLastResult
      : (latestGepaQueueRecord ?? {});
  const latestGepaInner = asRecord(latestGepaRecord.result);
  const latestGepaStatus = str(
    latestGepaInner.status,
    str(latestGepaRecord.status, ""),
  ).trim();
  const latestGepaError =
    str(latestGepaInner.error, "").trim() ||
    str(latestGepaInner.stderr_tail, "").trim() ||
    str(latestGepaRecord.error, "").trim();
  const latestGepaImport = asRecord(
    latestGepaInner.import_result ?? latestGepaRecord.import_result,
  );
  const latestGepaImportSummary = asRecord(latestGepaImport.summary);
  const latestGepaCandidateCount =
    num(latestGepaImportSummary.prompt_candidates, 0) +
    num(latestGepaImportSummary.specialist_prompt_candidates, 0) +
    num(latestGepaImportSummary.prompt_fragment_candidates, 0);
  const gepaAutoStatus = str(gepaAutoState.last_status, "").trim();
  const gepaAutoReason = str(gepaAutoState.last_reason, "");
  const gepaEvidenceSamples = num(gepaAutoState.last_evidence_samples, 0);
  const selfEvolveEnabled = toBool(evolution.self_evolve_enabled);
  const gepaOptimizerEnabled =
    gepaConfig.enabled == null ? true : toBool(gepaConfig.enabled);
  const backgroundImprovementPaused =
    !selfEvolveEnabled || !gepaOptimizerEnabled;
  const backgroundImprovementPauseText = !selfEvolveEnabled
    ? "Evolve is paused."
    : "GEPA background optimizer is disabled.";
  const backgroundImprovementNeedsAttention = [
    latestGepaStatus,
    gepaAutoStatus,
  ].some((status) => ["blocked", "failed", "timed_out", "error"].includes(status));
  const backgroundImprovementLabel = backgroundImprovementPaused
    ? "Paused"
    : gepaRunningJobs > 0
      ? "Running now"
      : gepaPendingJobs > 0
        ? "Waiting for quiet time"
        : !gepaReady
          ? "Needs model setup"
          : !gepaBudgetAllowed
            ? "Daily limit reached"
            : backgroundImprovementNeedsAttention
              ? "Needs attention"
              : latestGepaCandidateCount > 0
                ? "Safety checks"
                : gepaEvidenceSamples > 0
                  ? "Collecting samples"
                  : "Waiting for more data";
  const backgroundImprovementColor = backgroundImprovementPaused
    ? ("default" as const)
    : gepaRunningJobs > 0
      ? ("info" as const)
      : gepaPendingJobs > 0
        ? ("warning" as const)
      : !gepaReady || !gepaBudgetAllowed || backgroundImprovementNeedsAttention
          ? ("warning" as const)
          : latestGepaCandidateCount > 0
            ? ("success" as const)
            : ("info" as const);
  const promptInsights = asRecord(evolutionDev.prompt_insights);
  const classifierInsights = asRecord(
    evolutionDev.classifier_prompt_insights ?? evolutionDev.classifier_insights,
  );
  const specialistInsights = asRecord(evolutionDev.specialist_prompt_insights);
  const promptFragmentInsights = asRecord(
    evolutionDev.prompt_fragment_insights,
  );
  const policyMetrics = pickRecords(evolutionDev, "policy_metrics");
  const strategyMetrics = pickRecords(evolutionDev, "strategy_metrics");
  const promptMetrics = pickRecords(evolutionDev, "prompt_metrics");
  const specialistMetrics = pickRecords(
    evolutionDev,
    "specialist_prompt_metrics",
  );
  const promptFragmentMetrics = pickRecords(
    evolutionDev,
    "prompt_fragment_metrics",
  );
  const routingCanaryState = asRecord(evolutionDev.canary_state);
  const promptCanaryState = asRecord(evolutionDev.prompt_canary_state);
  const specialistPromptCanaryState = asRecord(
    evolutionDev.specialist_prompt_canary_state,
  );
  const promptFragmentCanaryState = asRecord(
    evolutionDev.prompt_fragment_canary_state,
  );
  const promptCanarySafetyEvents = pickRecords(
    evolutionDev,
    "prompt_canary_safety_events",
  );
  const promptOptimizationOpportunities = pickRecords(
    evolutionDev,
    "prompt_optimization_opportunities",
  );
  const arkdistillContextSummary = asRecord(
    evolutionDev.arkdistill_context_summary,
  );
  const arkdistillContextSamples = finiteNumber(
    arkdistillContextSummary.sample_count,
  );
  const arkdistillContextTopTools = pickRecords(
    arkdistillContextSummary,
    "top_tools",
  );
  const arkdistillTopTool = arkdistillContextTopTools[0] ?? {};
  const arkdistillTopToolLabel = [
    str(arkdistillTopTool.tool_name, ""),
    str(arkdistillTopTool.action, ""),
  ]
    .filter((part) => part.trim())
    .join(" / ");
  const arkdistillSavedTokens = finiteNumber(
    arkdistillContextSummary.estimated_saved_tokens,
  );
  const arkdistillCompressionRatio =
    finiteNumber(arkdistillContextSummary.savings_percent) == null
      ? null
      : finiteNumber(arkdistillContextSummary.savings_percent)! / 100;
  const arkdistillEvidenceScore = finiteNumber(
    arkdistillContextSummary.sample_confidence_score,
  );
  const arkdistillContextStats: ExperimentMetricSummary[] = [
    {
      label: "Saved",
      value: formatEstimatedTokenSavings(arkdistillSavedTokens),
      helper: `${Math.round(num(arkdistillContextSummary.saved_chars, 0)).toLocaleString()} chars`,
      tone: "good",
      accent:
        arkdistillSavedTokens != null && arkdistillSavedTokens > 0
          ? `~${Math.round(arkdistillSavedTokens).toLocaleString()}`
          : undefined,
      unit:
        arkdistillSavedTokens != null && arkdistillSavedTokens > 0 ? "tokens" : undefined,
    },
    {
      label: "Compression",
      value: formatPercentRatio(arkdistillCompressionRatio),
      helper: "Observed reduction",
      tone: "good",
      accent:
        arkdistillCompressionRatio == null
          ? undefined
          : formatPercentRatio(arkdistillCompressionRatio),
    },
    {
      label: "Evidence",
      value: formatPercentRatio(arkdistillEvidenceScore),
      helper: `${formatSampleCount(arkdistillContextSamples)}${
        arkdistillTopToolLabel ? `, top ${arkdistillTopToolLabel}` : ""
      }`,
      tone: "good",
      accent:
        arkdistillEvidenceScore == null
          ? undefined
          : formatPercentRatio(arkdistillEvidenceScore),
      progress:
        arkdistillEvidenceScore == null
          ? null
          : Math.max(0, Math.min(1, arkdistillEvidenceScore)),
    },
  ];
  const learningCandidates = pickRecords(evolutionDev, "learning_candidates");
  const learningPatterns = pickRecords(evolutionDev, "learning_patterns");
  const learningItems = pickRecords(evolutionDev, "learning_items");
  const skillEvolutions: JsonRecord[] = [];
  const experienceGraph = asRecord(evolutionDev.experience_graph);
  const experienceGraphNodes = pickRecords(experienceGraph, "nodes");
  const experienceGraphEdges = pickRecords(experienceGraph, "edges");
  const recentExperienceRuns = pickRecords(
    evolutionDev,
    "recent_experience_runs",
  );
  const reflectedHeuristics = learningItems.filter(
    (row) => str(row.origin, "").trim().toLowerCase() === "heuristic_reflection",
  );
  const skillReviewItems = skillEvolutions.filter(
    (row) => str(row.approval_status, "draft") === "draft",
  );
  const approvedSkillEvolutions = skillEvolutions.filter(
    (row) => str(row.approval_status, "").trim().toLowerCase() === "approved",
  );
  const skillHelpedItems = approvedSkillEvolutions.filter(
    (row) => str(row.impact_status, "").trim().toLowerCase() === "improved",
  );
  const skillObservedItems = approvedSkillEvolutions.filter(
    (row) => str(row.impact_status, "").trim().toLowerCase() !== "improved",
  );
  const nonSkillLearningCandidates = learningCandidates.filter(
    (row) => str(row.candidate_type, "") !== "skill_patch",
  );
  const lineageRows: JsonRecord[] = [
    ...pickRecords(evolutionDev, "prompt_lineage_recent").map(
      (row): JsonRecord => ({
        ...row,
        surface: "Prompt",
        gain: row.score_gain,
      }),
    ),
    ...pickRecords(evolutionDev, "specialist_prompt_lineage_recent").map(
      (row): JsonRecord => ({
        ...row,
        surface: "Specialist",
        gain: row.score_gain,
      }),
    ),
    ...pickRecords(evolutionDev, "prompt_fragment_lineage_recent").map(
      (row): JsonRecord => ({
        ...row,
        surface: "Prompt fragments",
        gain: row.score_gain,
      }),
    ),
    ...pickRecords(evolutionDev, "lineage_recent").map(
      (row): JsonRecord => ({
        ...row,
        surface: "Policy",
        gain: row.accuracy_gain,
      }),
    ),
  ].sort((a, b) =>
    str(b.timestamp_utc, "").localeCompare(str(a.timestamp_utc, "")),
  );
  const confirmedLineageRows = lineageRows.filter((row) => toBool(row.promoted));
  const confirmedRecentChangeCount =
    confirmedLineageRows.length + skillHelpedItems.length;
  const routingRollbackAvailable = toBool(evolution.routing_rollback_available);
  const promptRollbackAvailable = toBool(evolution.prompt_rollback_available);
  const specialistPromptRollbackAvailable = toBool(
    evolution.specialist_prompt_rollback_available,
  );
  const promptFragmentRollbackAvailable = toBool(
    evolution.prompt_fragment_rollback_available,
  );

  const tests: ExperimentSurfaceItem[] = [
    {
      key: "routing",
      name: "Routing policy",
      audienceLabel: evolutionSurfaceAudienceLabel("Routing policy"),
      summary: evolutionSurfaceSummary("Routing policy"),
      benefit: evolutionSurfaceBenefit("Routing policy"),
      stableSummary: evolutionSurfaceStableSummary("Routing policy"),
      enabled: toBool(canary.enabled),
      rollout: clampPercent(canary.rollout_percent),
      baseline: str(canary.baseline_version, "routing-policy-default-v1"),
      candidate: str(canary.candidate_version, "-"),
      gate: str(evolution.replay_gate_result, "-"),
      replayGateReasons: pickRecords(evolution, "replay_gate_reasons"),
      last: str(
        evolution.last_promotion_result,
        "No routing-policy promotion yet",
      ),
      metrics: policyMetrics,
      canaryState: routingCanaryState,
      primaryMetricLabel: "Task success rate",
      stopAction: { action: "disable_canary" },
      acceptAction: { action: "promote_candidate" },
      rollbackAction: { action: "rollback_baseline" },
      rollbackAvailable: routingRollbackAvailable,
    },
    {
      key: "prompt",
      name: "Main prompt bundle",
      audienceLabel: evolutionSurfaceAudienceLabel("Main prompt bundle"),
      summary: evolutionSurfaceSummary("Main prompt bundle"),
      benefit: evolutionSurfaceBenefit("Main prompt bundle"),
      stableSummary: evolutionSurfaceStableSummary("Main prompt bundle"),
      enabled: toBool(promptCanary.enabled),
      rollout: clampPercent(promptCanary.rollout_percent),
      baseline: str(promptCanary.baseline_version, "-"),
      candidate: str(promptCanary.candidate_version, "-"),
      gate: str(evolution.prompt_replay_gate_result, "-"),
      replayGateReasons: pickRecords(evolution, "prompt_replay_gate_reasons"),
      last: str(
        evolution.prompt_last_promotion_result,
        "No prompt promotion yet",
      ),
      metrics: promptMetrics,
      canaryState: promptCanaryState,
      primaryMetricLabel: "Reply success rate",
      stopAction: { action: "disable_prompt_canary", candidate_id: "prompt" },
      acceptAction: {
        action: "promote_prompt_canary_candidate",
        candidate_id: "prompt",
      },
      rollbackAction: {
        action: "rollback_prompt_baseline",
        candidate_id: "prompt",
      },
      rollbackAvailable: promptRollbackAvailable,
    },
    {
      key: "specialist",
      name: "Specialist prompts",
      audienceLabel: evolutionSurfaceAudienceLabel("Specialist prompts"),
      summary: evolutionSurfaceSummary("Specialist prompts"),
      benefit: evolutionSurfaceBenefit("Specialist prompts"),
      stableSummary: evolutionSurfaceStableSummary("Specialist prompts"),
      enabled: toBool(specialistCanary.enabled),
      rollout: clampPercent(specialistCanary.rollout_percent),
      baseline: str(specialistCanary.baseline_version, "-"),
      candidate: str(specialistCanary.candidate_version, "-"),
      gate: str(evolution.specialist_prompt_replay_gate_result, "-"),
      replayGateReasons: pickRecords(evolution, "specialist_prompt_replay_gate_reasons"),
      last: str(
        evolution.specialist_prompt_last_promotion_result,
        "No specialist promotion yet",
      ),
      metrics: specialistMetrics,
      canaryState: specialistPromptCanaryState,
      primaryMetricLabel: "Delegated work success",
      stopAction: {
        action: "disable_prompt_canary",
        candidate_id: "specialist_prompt",
      },
      acceptAction: {
        action: "promote_prompt_canary_candidate",
        candidate_id: "specialist_prompt",
      },
      rollbackAction: {
        action: "rollback_prompt_baseline",
        candidate_id: "specialist_prompt",
      },
      rollbackAvailable: specialistPromptRollbackAvailable,
    },
    {
      key: "prompt-fragments",
      name: "Prompt fragments",
      audienceLabel: evolutionSurfaceAudienceLabel("Prompt fragments"),
      summary: evolutionSurfaceSummary("Prompt fragments"),
      benefit: evolutionSurfaceBenefit("Prompt fragments"),
      stableSummary: evolutionSurfaceStableSummary("Prompt fragments"),
      enabled: toBool(promptFragmentCanary.enabled),
      rollout: clampPercent(promptFragmentCanary.rollout_percent),
      baseline: str(promptFragmentCanary.baseline_version, "-"),
      candidate: str(promptFragmentCanary.candidate_version, "-"),
      gate: str(evolution.prompt_fragment_replay_gate_result, "-"),
      replayGateReasons: pickRecords(evolution, "prompt_fragment_replay_gate_reasons"),
      last: str(
        evolution.prompt_fragment_last_promotion_result,
        "No prompt fragment promotion yet",
      ),
      metrics: promptFragmentMetrics,
      canaryState: promptFragmentCanaryState,
      primaryMetricLabel: "Prompted turn success",
      stopAction: {
        action: "disable_prompt_canary",
        candidate_id: "prompt_fragment",
      },
      acceptAction: {
        action: "promote_prompt_canary_candidate",
        candidate_id: "prompt_fragment",
      },
      rollbackAction: {
        action: "rollback_prompt_baseline",
        candidate_id: "prompt_fragment",
      },
      rollbackAvailable: promptFragmentRollbackAvailable,
    },
  ];
  const activeExperimentItems = tests.filter((item) => item.enabled);
  const stableExperimentItems = tests.filter((item) => !item.enabled);
  const activeTests = activeExperimentItems.length;
  const maxRollout = tests.reduce(
    (acc, item) => Math.max(acc, item.rollout),
    0,
  );
  const helpedLines = [
    ...skillHelpedItems.flatMap((row) => {
      const summary = stringList(asRecord(row.impact_assessment).summary);
      const prefix = str(row.skill_name, "Skill");
      return summary.map((line) => `${prefix}: ${line}`);
    }),
    ...stringList(promptInsights.summary),
    ...stringList(specialistInsights.summary),
    ...stringList(promptFragmentInsights.summary),
  ];
  const metricRows: JsonRecord[] = [
    ...promptMetrics
      .slice(0, 5)
      .map((row): JsonRecord => ({ ...row, surface: "Prompt" })),
    ...strategyMetrics
      .slice(0, 5)
      .map((row): JsonRecord => ({ ...row, surface: "Routing" })),
    ...specialistMetrics
      .slice(0, 3)
      .map((row): JsonRecord => ({ ...row, surface: "Specialist" })),
    ...promptFragmentMetrics
      .slice(0, 3)
      .map((row): JsonRecord => ({ ...row, surface: "Prompt fragments" })),
  ];
  const metricChartRows = metricRows.slice(0, 10);
  const experienceGraphOption = useMemo(() => {
    const categories = [
      { name: "Run" },
      { name: "Item" },
      { name: "Pattern" },
      { name: "Candidate" },
      { name: "Tool" },
    ];
    const kindCategory = (kind: string) => {
      if (kind === "experience_run") return 0;
      if (kind === "experience_item") return 1;
      if (kind === "procedural_pattern") return 2;
      if (kind === "learning_candidate") return 3;
      return 4;
    };
    const nodeIds = new Set(
      experienceGraphNodes.map((node) => str(node.id, "")).filter(Boolean),
    );
    const rawLinks = experienceGraphEdges
      .map((edge) => ({
        source: str(edge.source, ""),
        target: str(edge.target, ""),
        value: str(edge.edge_type, ""),
      }))
      .filter((edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target));
    const degreeByNode = new Map<string, number>();
    for (const edge of rawLinks) {
      degreeByNode.set(edge.source, (degreeByNode.get(edge.source) ?? 0) + 1);
      degreeByNode.set(edge.target, (degreeByNode.get(edge.target) ?? 0) + 1);
    }
    const visibleNodes = experienceGraphNodes
      .slice()
      .sort((a, b) => {
        const degreeDelta =
          (degreeByNode.get(str(b.id, "")) ?? 0) -
          (degreeByNode.get(str(a.id, "")) ?? 0);
        if (degreeDelta !== 0) return degreeDelta;
        return str(a.label, str(a.id, "")).localeCompare(
          str(b.label, str(b.id, "")),
        );
      })
      .slice(0, 64);
    const visibleIds = new Set(
      visibleNodes.map((node) => str(node.id, "")).filter(Boolean),
    );
    const nodeBaseSize = [10, 8, 9, 9, 7];
    const nodes = visibleNodes.map((node) => {
      const id = str(node.id, "");
      const category = kindCategory(str(node.kind, ""));
      const degree = degreeByNode.get(id) ?? 0;
      return {
        id,
        name: str(node.label, str(node.id, "Node")),
        category,
        symbolSize: Math.min(15, nodeBaseSize[category] + Math.min(5, degree)),
        value: str(node.status, ""),
      };
    });
    const links = rawLinks
      .filter((edge) => visibleIds.has(edge.source) && visibleIds.has(edge.target))
      .slice(0, 120);
    return {
      backgroundColor: "transparent",
      animationDurationUpdate: 260,
      color: ["#78f2b0", "#d8ad78", "#ffbe63", "#ff9b9b", "#b7a7ff"],
      tooltip: {
        backgroundColor: "rgba(14, 18, 14, 0.96)",
        borderColor: "rgba(120, 242, 176, 0.24)",
        textStyle: { color: "#fff8ed", fontSize: 12 },
        formatter: (params: { data?: JsonRecord; value?: unknown }) => {
          const data = asRecord(params.data);
          return [str(data.name, "Node"), str(data.value, "")]
            .filter(Boolean)
            .join("<br/>");
        },
      },
      legend: [
        {
          top: 0,
          right: 0,
          itemWidth: 7,
          itemHeight: 7,
          itemGap: 10,
          data: categories.map((category) => category.name),
          textStyle: { color: "#c8d8c9", fontSize: 10 },
        },
      ],
      series: [
        {
          type: "graph",
          layout: "force",
          roam: true,
          top: 28,
          right: 8,
          bottom: 8,
          left: 8,
          scaleLimit: { min: 0.7, max: 3 },
          categories,
          data: nodes,
          links,
          label: {
            show: false,
            position: "right",
            color: "#fff8ed",
            fontSize: 10,
            distance: 5,
            formatter: (params: { data?: JsonRecord }) =>
              str(asRecord(params.data).name, "Node"),
          },
          edgeLabel: { show: false },
          force: {
            repulsion: 82,
            edgeLength: [46, 86],
            friction: 0.62,
            gravity: 0.08,
          },
          itemStyle: {
            borderColor: "#0b1120",
            borderWidth: 1.1,
          },
          emphasis: {
            focus: "adjacency",
            label: { show: true },
            itemStyle: { borderColor: "#fff8ed", borderWidth: 1.4 },
            lineStyle: { opacity: 0.78, width: 1.25 },
          },
          blur: {
            itemStyle: { opacity: 0.32 },
            lineStyle: { opacity: 0.06 },
          },
          lineStyle: {
            color: "rgba(148, 163, 184, 0.52)",
            width: 0.85,
            opacity: 0.32,
            curveness: 0.04,
          },
        },
      ],
    };
  }, [experienceGraphEdges, experienceGraphNodes]);
  const evidenceCards = useMemo(
    () => buildEvolutionEvidenceCards(recentExperienceRuns),
    [recentExperienceRuns],
  );
  const learningPatternById = useMemo(() => {
    const next = new Map<string, JsonRecord>();
    for (const row of learningPatterns) {
      const id = str(row.id, "");
      if (!id) continue;
      next.set(id, row);
    }
    return next;
  }, [learningPatterns]);
  const learningItemById = useMemo(() => {
    const next = new Map<string, JsonRecord>();
    for (const row of learningItems) {
      const id = str(row.id, "");
      if (!id) continue;
      next.set(id, row);
    }
    return next;
  }, [learningItems]);
  const openPromptCanarySafetyEvents = promptCanarySafetyEvents.filter((row) => {
    const reviewStatus = str(row.review_status, str(row.status, "open"))
      .trim()
      .toLowerCase();
    return reviewStatus === "open" || reviewStatus === "review_recommended";
  });
  const promptOptimizationDecisionItems = promptOptimizationOpportunities.filter(
    (row) => {
      const reviewStatus = str(row.review_status, "open").trim().toLowerCase();
      return reviewStatus !== "approved" && reviewStatus !== "rejected";
    },
  );
  const activePromptOptimizationOpportunities = promptOptimizationOpportunities.filter(
    (row) => {
      const reviewStatus = str(row.review_status, "open").trim().toLowerCase();
      const lifecycleStatus = promptProposalLifecycleStatus(row);
      return reviewStatus !== "rejected" && lifecycleStatus !== "rolled_back";
    },
  );
  const visiblePromptCanarySafetyEvents = showSuperseded
    ? promptCanarySafetyEvents
    : openPromptCanarySafetyEvents;
  const visiblePromptOptimizationOpportunities = showSuperseded
    ? promptOptimizationOpportunities
    : activePromptOptimizationOpportunities;
  const openNonSkillLearningCandidates = nonSkillLearningCandidates.filter((row) => {
    const status = str(row.approval_status, "draft").trim().toLowerCase();
    return !status || status === "draft" || status === "open";
  });
  const visibleNonSkillLearningCandidates = showSuperseded
    ? nonSkillLearningCandidates
    : openNonSkillLearningCandidates;
  const needsApprovalCount =
    skillReviewItems.length +
    openNonSkillLearningCandidates.length +
    openPromptCanarySafetyEvents.length +
    promptOptimizationDecisionItems.length;
  const promotedChangeCount =
    lineageRows.filter((row) => toBool(row.promoted)).length +
    skillHelpedItems.length;
  const experienceGraphReady =
    experienceGraphNodes.length >= 4 && experienceGraphEdges.length > 0;
  const experienceNodePreview = experienceGraphNodes.slice(0, 5);
  const metricChartLabels = metricChartRows.map((row) => {
    const surface = str(row.surface, "-");
    const version = str(row.version, "").trim();
    return version ? `${surface} ${version}` : surface;
  });
  const optimizationMetricSummaries = metricChartRows.slice(0, 5).map((row) => {
    const label = str(row.surface, "-");
    const version = str(row.version, "").trim();
    const samples =
      finiteNumber(row.samples) ??
      finiteNumber(row.sample_count) ??
      finiteNumber(row.total_runs);
    return {
      key: `${label}-${version || str(row.id, "")}`,
      label: version ? `${label} ${version}` : label,
      success: ratioPercent(row.success_rate),
      error: ratioPercent(row.error_rate),
      helper:
        samples == null
          ? "Recent traffic"
          : `${Math.round(samples).toLocaleString()} recent sample${Math.round(samples) === 1 ? "" : "s"}`,
    };
  });
  const metricChartOption = {
    backgroundColor: "transparent",
    animationDuration: 350,
    grid: { left: 42, right: 14, top: 40, bottom: 34, containLabel: true },
    legend: { top: 0, textStyle: { color: "#d8d0c4", fontSize: 11 } },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(14, 18, 14, 0.96)",
      borderColor: "rgba(120, 242, 176, 0.24)",
      textStyle: { color: "#fff8ed" },
    },
    xAxis: {
      type: "category",
      data: metricChartLabels,
      axisTick: { alignWithLabel: true },
      axisLabel: {
        color: "#c8d8c9",
        fontSize: 10,
        interval: 0,
        rotate: metricChartRows.length > 4 ? 22 : 0,
        hideOverlap: true,
        overflow: "truncate",
        width: metricChartRows.length > 4 ? 92 : 120,
        margin: 12,
      },
    },
    yAxis: {
      type: "value",
      max: 100,
      axisLabel: { color: "#c8d8c9", formatter: "{value}%" },
      splitLine: { lineStyle: { color: "rgba(130, 170, 160, 0.14)" } },
    },
    series: [
      {
        name: "Success",
        type: "bar",
        data: metricChartRows.map((row) => ratioPercent(row.success_rate)),
        itemStyle: { color: "#14f195", borderRadius: [4, 4, 0, 0] },
        barMaxWidth: 26,
      },
      {
        name: "Error",
        type: "line",
        data: metricChartRows.map((row) => ratioPercent(row.error_rate)),
        smooth: true,
        lineStyle: { color: "#fb7185", width: 2 },
        itemStyle: { color: "#fb7185" },
      },
    ],
  };
  const lastRoutingOptimization = asRecord(evolutionDev.last_result);
  const lastRoutingPromoted = toBool(lastRoutingOptimization.promoted);
  const lastRoutingMode = str(lastRoutingOptimization.promotion_mode, "none");
  const lastRoutingGate = promotionGateSummary(lastRoutingOptimization);
  const lastRoutingAccuracyGain = num(
    lastRoutingOptimization.accuracy_gain,
    Number.NaN,
  );
  const lastRoutingSummary =
    Object.keys(lastRoutingOptimization).length === 0
      ? "No guided optimization has run yet."
      : lastRoutingMode === "canary"
        ? "A routing improvement is being tested on a small share of traffic."
        : lastRoutingMode === "baseline"
          ? "The last routing improvement passed checks and is now stable."
          : lastRoutingPromoted
            ? "A routing improvement passed offline checks and is waiting on rollout."
            : lastRoutingGate
              ? `Last check made no change: ${lastRoutingGate}.`
              : "Last check made no change.";
  const statusLoading = evolutionQ.isLoading;
  const detailLoading = evolutionDevQ.isLoading;
  const guidedOptimizationDisabled =
    statusLoading ||
    detailLoading ||
    updateEvolutionMutation.isPending ||
    runEvolutionActionMutation.isPending ||
    !toBool(evolution.self_evolve_enabled);
  const rollbackAvailableCount = [
    routingRollbackAvailable,
    promptRollbackAvailable,
    specialistPromptRollbackAvailable,
    promptFragmentRollbackAvailable,
  ].filter(Boolean).length;
  const anyRollbackAvailable = rollbackAvailableCount > 0;
  const reviewItemNoun = needsApprovalCount === 1 ? "suggestion" : "suggestions";
  const reviewVerb = needsApprovalCount === 1 ? "needs" : "need";
  const reviewDecisionSubject =
    needsApprovalCount === 1 ? "this idea" : "these ideas";
  const primaryStatusTitle = needsApprovalCount > 0
    ? `${needsApprovalCount} ${reviewItemNoun} ${reviewVerb} your review`
    : activeTests > 0
      ? "A limited test is running"
      : anyRollbackAvailable
        ? "A stable change is active"
        : gepaRunningJobs > 0
          ? "Running background check"
          : gepaPendingJobs > 0
            ? "Queued for quiet time"
            : !gepaReady
              ? "Waiting for model setup"
              : latestGepaCandidateCount > 0
                ? "Checking candidate safety"
                : "Nothing needs you now";
  const primaryStatusDetail = needsApprovalCount > 0
    ? `Nothing has changed yet. Open the review queue to decide whether Evolve should keep going with ${reviewDecisionSubject}.`
    : activeTests > 0
      ? "A small live test is active. You can view it, stop it, or make it stable from Live tests."
      : anyRollbackAvailable
        ? `${rollbackAvailableCount} stable change${rollbackAvailableCount === 1 ? "" : "s"} can be rolled back from Live tests.`
        : gepaRunningJobs > 0
          ? "Evolve is reviewing completed work. If it finds something useful, it will move into safety checks or review."
          : gepaPendingJobs > 0
            ? "A background check is waiting until AgentArk is quiet."
            : !gepaReady
              ? "Background improvement needs a working primary model before it can run."
              : latestGepaCandidateCount > 0
                ? "Candidate improvements were created and are going through safety checks."
                : "Evolve is watching completed work and will ask before it changes behavior.";
  const reviewLifecycleSteps = [
    "Suggested",
    "Approved",
    "Background test",
    "Live test",
    "Stable change",
  ];

  async function updateEvolution(payload: JsonRecord, message: string) {
    setError(null);
    setSuccess(null);
    try {
      await updateEvolutionMutation.mutateAsync(payload);
      setSuccess(message);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function runEvolutionAction(
    payload: JsonRecord,
    message: string,
    confirmMessage?: string,
  ) {
    if (confirmMessage && !window.confirm(confirmMessage)) return;
    setError(null);
    setSuccess(null);
    try {
      const result = await runEvolutionActionMutation.mutateAsync(payload);
      const resultMessage = str(asRecord(result).message, "");
      setSuccess(`${resultMessage || message}${evolutionTraceIdHint(result)}`);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  function openReviewQueue() {
    setTab("review");
    window.setTimeout(() => {
      const target = document.getElementById("ark-evolve-review-queue");
      if (!target) return;
      target.scrollIntoView({ behavior: "smooth", block: "start" });
      target.focus({ preventScroll: true });
    }, 0);
  }

  const statusError = evolutionQ.error ? errMessage(evolutionQ.error) : "";
  const detailError = evolutionDevQ.error
    ? errMessage(evolutionDevQ.error)
    : "";
  const activeError = error || statusError;
  const hasMeasuredHelp =
    confirmedRecentChangeCount > 0 ||
    helpedLines.length > 0;
  const resultSummarySeverity = detailError
    ? ("warning" as const)
    : hasMeasuredHelp
      ? ("success" as const)
      : ("info" as const);
  const resultSummaryTitle = detailLoading
    ? "Loading measured results"
    : detailError
      ? "Results are unavailable"
      : hasMeasuredHelp
        ? `${confirmedRecentChangeCount || helpedLines.length} confirmed improvement${(confirmedRecentChangeCount || helpedLines.length) === 1 ? "" : "s"}`
        : "No proven improvement yet";
  const resultSummaryDetail = detailLoading
    ? "Evolve is loading the recent evidence behind prompt, routing, specialist, memory, and strategy changes."
    : detailError
      ? "The detail endpoint did not return enough data to explain recent Evolve results."
      : hasMeasuredHelp
        ? "These are changes with measured evidence from recent runs. Live tests and review items are shown separately before anything risky becomes stable."
        : "Evolve has not found enough measured evidence to call a recent change useful. This page now shows that plainly instead of stretching empty panels or drawing weak charts.";
  const resultSummaryCards = [
    {
      label: "Confirmed wins",
      value: String(confirmedRecentChangeCount),
      helper:
        confirmedRecentChangeCount > 0
          ? "Recent evidence says these helped."
          : "Nothing has cleared the improvement threshold.",
      tone: confirmedRecentChangeCount > 0 ? ("good" as const) : ("info" as const),
    },
    {
      label: "Still measuring",
      value: String(skillObservedItems.length),
      helper:
        skillObservedItems.length > 0
          ? "Approved changes need more traffic."
          : "No approved change is waiting.",
      tone: skillObservedItems.length > 0 ? ("warn" as const) : ("default" as const),
    },
    {
      label: "Needs review",
      value: String(needsApprovalCount),
      helper:
        needsApprovalCount > 0
          ? "Suggestions wait for your decision."
          : "Nothing is waiting on you.",
      tone: needsApprovalCount > 0 ? ("warn" as const) : ("default" as const),
    },
    {
      label: "Live tests",
      value: String(activeTests),
      helper:
        activeTests > 0
          ? "Limited traffic is testing candidates."
          : "No candidate is live right now.",
      tone: activeTests > 0 ? ("info" as const) : ("default" as const),
    },
  ];
  const evidenceMetricCards = [
    {
      label: "Prompt",
      value: `Tool ${evolutionGainLabel(promptInsights.tool_success_uplift)}`,
      helper: `Delegation avoided ${num(promptInsights.delegation_avoided, 0).toFixed(1)}, clarification avoided ${num(promptInsights.clarification_avoided, 0).toFixed(1)}`,
    },
    {
      label: "Classifier",
      value: `Direct ${evolutionGainLabel(
        classifierInsights.successful_direct_resolution_uplift,
      )}`,
      helper: `Failed delegation reduction ${evolutionGainLabel(
        classifierInsights.failed_delegation_reduction,
      )}`,
    },
    {
      label: "Specialist",
      value: `Tool ${evolutionGainLabel(specialistInsights.tool_success_uplift)}`,
      helper: `p95 savings ${
        specialistInsights.latency_savings_p95_ms == null
          ? "-"
          : `${num(specialistInsights.latency_savings_p95_ms, 0)} ms`
      }`,
    },
  ];
  const selectedPatternRuns = selectedPatternCard?.runs ?? [];
  const selectedPatternRequests = uniqueNonEmptyStrings(
    selectedPatternRuns
      .map((run) => str(run.request_text, "").trim())
      .filter(Boolean),
  ).slice(0, 6);
  const selectedPatternPolicies = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.policy_version),
  );
  const selectedPatternStrategies = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.strategy_version),
  );
  const selectedPatternPrompts = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.prompt_version),
  );
  const selectedPatternSpecialistPrompts = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.specialist_prompt_version),
  );
  const selectedPatternVersionItems = [
    { label: "Policy", values: selectedPatternPolicies },
    { label: "Strategy", values: selectedPatternStrategies },
    { label: "Prompt", values: selectedPatternPrompts },
    { label: "Specialist prompt", values: selectedPatternSpecialistPrompts },
  ].filter((item) => item.values.length > 1);

  return (
    <WorkspacePageShell className="evolution-page" spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="ARK CORE"
        title="Evolve"
        description="Watches completed work, proposes improvements, and asks before anything lasting changes."
      />
      {success ? <Alert severity="success">{success}</Alert> : null}
      {activeError ? <Alert severity="error">{activeError}</Alert> : null}

      {/* EvolveHero removed entirely. The filter chip strip below
          ("Needs review (2)" / "Live tests (1)" / "Stable" / "Overview")
          carries the same headline number + state with far less screen
          real estate. The page no longer has a giant counter floating
          at the top — counts live inline on the action they belong to. */}

      {/* Previous "show details" Collapse + duplicate primaryStatusTitle
          header was removed. The EvolveHero above already surfaces the
          headline + action moments; repeating it below was the source
          of the "2 suggestions need your review" / "2 suggestions are
          waiting" duplication. Tabs are now always visible. */}
      {!gepaReady && !backgroundImprovementPaused ? (
        <Alert severity="info" sx={{ borderRadius: 1 }}>
          Background improvement starts automatically after Models has a working
          primary model{gepaIssues[0] ? `: ${gepaIssues[0]}` : "."}
        </Alert>
      ) : null}
      <Box className="list-shell workspace-page-subnav-shell">
        <Stack
          direction="row"
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <Tabs
            value={tab}
            onChange={(_event, next) => setTab(next as EvolutionPageTab)}
            variant="scrollable"
            allowScrollButtonsMobile
            className="workspace-page-subnav-tabs"
            sx={{ flex: 1 }}
          >
            <Tab value="review" label={`Needs Review (${needsApprovalCount})`} />
            <Tab value="helped" label={`Deployed (${promotedChangeCount})`} />
          </Tabs>
        </Stack>
      </Box>
      {statusLoading ? (
        <Box className="list-shell" sx={{ p: 1.5 }}>
          <Stack
            direction="row"
            spacing={1}
            sx={{
              alignItems: "center",
            }}
          >
            <CircularProgress size={18} />
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              Loading Evolve status...
            </Typography>
          </Stack>
        </Box>
      ) : null}
      {tab === "what" ? (
        <Stack spacing={1.5}>
          <Box className="list-shell" sx={{ p: 1.6 }}>
            {detailLoading ? (
              <Stack
                direction="row"
                spacing={1}
                sx={{
                  alignItems: "center",
                }}
              >
                <CircularProgress size={16} />
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Loading recent changes...
                </Typography>
              </Stack>
            ) : detailError ? (
              <Alert severity="warning" sx={{ borderRadius: 1 }}>
                Detailed Evolve history is unavailable: {detailError}
              </Alert>
            ) : confirmedRecentChangeCount === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No confirmed improvements yet. Evolve will list changes here
                only after they have proven measurable impact. In the meantime,
                the Review queue tab shows changes that are waiting on you.
              </Typography>
            ) : (
              <Stack spacing={1}>
                {skillHelpedItems.length > 0 ? (
                  <Box
                    sx={{
                      pb: 1,
                      borderBottom: "1px solid var(--ui-rgba-145-170-205-120)",
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                        mb: 0.75,
                      }}
                    >
                      Capability improvements
                    </Typography>
                    <Stack spacing={0.85}>
                      {skillHelpedItems.slice(0, 4).map((row, idx) => {
                        const when = humanTs(
                          str(row.reviewed_at || row.updated_at, "-"),
                        );
                        return (
                          <Alert
                            key={`skill-evolution-what-${str(row.id, String(idx))}`}
                            severity={skillEvolutionAlertSeverity(
                              str(
                                row.impact_status,
                                str(row.approval_status, "draft"),
                              ),
                            )}
                            sx={{ borderRadius: 1 }}
                          >
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{
                                alignItems: "center",
                                flexWrap: "wrap",
                                mb: 0.35,
                              }}
                            >
                              <Typography
                                variant="body2"
                                sx={{ color: "#e8f4ff", fontWeight: 600 }}
                              >
                                {canonicalSkillIdentifier(
                                  str(row.skill_name, "Skill"),
                                )}
                              </Typography>
                              <Chip
                                size="small"
                                label={skillEvolutionActionLabel(
                                  str(row.action, ""),
                                )}
                              />
                              <Chip
                                size="small"
                                color={skillEvolutionChipColor(
                                  str(
                                    row.impact_status,
                                    str(row.approval_status, "draft"),
                                  ),
                                )}
                                label={
                                  str(
                                    row.impact_status,
                                    str(row.approval_status, "draft"),
                                  ) || "draft"
                                }
                              />
                              <Typography
                                variant="caption"
                                sx={{ color: "text.secondary" }}
                              >
                                {when.label}
                              </Typography>
                            </Stack>
                            <Typography variant="body2">
                              {str(
                                row.diff_summary,
                                str(row.summary, "Reviewable capability change"),
                              )}
                            </Typography>
                          </Alert>
                        );
                      })}
                    </Stack>
                  </Box>
                ) : null}
                {confirmedLineageRows.slice(0, 8).map((row, idx) => {
                  const surfaceSummary = stringList(row.optimized_surfaces)
                    .concat(stringList(row.optimized_roles))
                    .join(", ");
                  const notesSummary = stringList(row.notes).join(" | ");
                  const focusSummary = pickRecords(row, "focus_cases")
                    .slice(0, 2)
                    .map(buildEvolutionFocusCaseLabel)
                    .join(" | ");
                  const fallbackSummary = str(
                    row.candidate_source,
                    "No summary recorded",
                  );
                  const summary =
                    surfaceSummary ||
                    notesSummary ||
                    focusSummary ||
                    fallbackSummary;
                  return (
                    <Box
                      key={`evolution-lineage-${str(row.entry_id, String(idx))}`}
                      sx={{
                        pb: 1,
                        borderBottom: "1px solid var(--ui-rgba-145-170-205-120)",
                      }}
                    >
                      <Stack
                        direction="row"
                        spacing={1}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap",
                        }}
                      >
                        <Chip size="small" label={str(row.surface, "Change")} />
                        <Typography
                          variant="body2"
                          title={humanTs(str(row.timestamp_utc, "-")).tip}
                        >
                          {humanTs(str(row.timestamp_utc, "-")).label}
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {toBool(row.promoted) ? "Promoted" : "Tested"}
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {evolutionGainLabel(row.gain)}
                        </Typography>
                      </Stack>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block",
                          mt: 0.35,
                        }}
                      >
                        {summary}
                      </Typography>
                    </Box>
                  );
                })}
              </Stack>
            )}
          </Box>

          {developerModeEnabled && evidenceCards.length > 0 ? (
            <Box className="list-shell" sx={{ p: 1.6 }}>
              <Typography variant="h6" sx={{ fontWeight: 700, mb: 0.5 }}>
                Recent patterns
              </Typography>
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                  mb: 1,
                }}
              >
                Select a row to inspect the grouped runs, observed tools, and
                why Evolve is still only watching the pattern.
              </Typography>
              {detailLoading ? (
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{ alignItems: "center" }}
                >
                  <CircularProgress size={16} />
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    Loading...
                  </Typography>
                </Stack>
              ) : detailError ? (
                <Alert severity="warning" sx={{ borderRadius: 1 }}>
                  {detailError}
                </Alert>
              ) : (
                (() => {
                  const patternPageSize = 10;
                  const patternPages = Math.max(
                    1,
                    Math.ceil(evidenceCards.length / patternPageSize),
                  );
                  const patternSlice = evidenceCards.slice(0, patternPageSize);
                  return (
                    <>
                      <TableContainer
                        className="table-shell"
                        sx={{ width: "100%", overflowX: "auto" }}
                      >
                        <Table size="small">
                          <TableHead>
                            <TableRow>
                              <TableCell width="25%">Pattern</TableCell>
                              <TableCell width="10%">Status</TableCell>
                              <TableCell width="40%">Detail</TableCell>
                              <TableCell width="25%">Why it matters</TableCell>
                            </TableRow>
                          </TableHead>
                          <TableBody>
                            {patternSlice.map((card) => (
                              <TableRow
                                key={card.key}
                                hover
                                role="button"
                                tabIndex={0}
                                sx={{ cursor: "pointer" }}
                                onClick={() => setSelectedPatternCard(card)}
                                onKeyDown={(event) => {
                                  if (
                                    event.key === "Enter" ||
                                    event.key === " "
                                  ) {
                                    event.preventDefault();
                                    setSelectedPatternCard(card);
                                  }
                                }}
                              >
                                <TableCell>
                                  <Typography
                                    variant="body2"
                                    sx={{ fontWeight: 600 }}
                                    noWrap
                                    title={card.title}
                                  >
                                    {card.title}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Chip
                                    size="small"
                                    color={learningEvidenceStatusColor(
                                      card.status,
                                    )}
                                    label={humanizeStatusLabel(card.status)}
                                  />
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    color="text.secondary"
                                    sx={{
                                      display: "-webkit-box",
                                      WebkitLineClamp: 2,
                                      WebkitBoxOrient: "vertical",
                                      overflow: "hidden",
                                    }}
                                  >
                                    {card.detail}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    color="text.secondary"
                                    noWrap
                                    title={card.rationale || ""}
                                  >
                                    {card.rationale || "-"}
                                  </Typography>
                                </TableCell>
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      </TableContainer>
                      {patternPages > 1 ? (
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ pt: 0.5 }}
                        >
                          Showing {patternSlice.length} of{" "}
                          {evidenceCards.length} patterns
                        </Typography>
                      ) : null}
                    </>
                  );
                })()
              )}
            </Box>
          ) : null}
        </Stack>
      ) : null}
      <Dialog
        open={selectedPatternCard != null}
        onClose={() => setSelectedPatternCard(null)}
        maxWidth="lg"
        fullWidth
        slotProps={{ paper: { sx: { borderRadius: "8px", border: "1px solid var(--surface-border)", background: "var(--surface-bg-elevated)", boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)" } } }}
      >
        <DialogTitle sx={{ pb: 0.5, display: "flex", alignItems: "center", gap: 1.5, borderBottom: "1px solid", borderColor: "divider" }}>
          <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>Observed pattern</Typography>
          {selectedPatternCard ? <Chip size="small" color={learningEvidenceStatusColor(selectedPatternCard.status)} label={humanizeStatusLabel(selectedPatternCard.status)} /> : null}
        </DialogTitle>
        <DialogContent>
          {selectedPatternCard ? (
            <Stack spacing={1.25}>
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "flex-start", md: "center" },
                }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography variant="h6" sx={{ fontWeight: 700 }}>
                    {selectedPatternCard.title}
                  </Typography>
                  <Typography
                    variant="body2"
                    sx={{ color: "text.secondary", mt: 0.35 }}
                  >
                    {evolutionPatternStatusExplanation(selectedPatternCard)}
                  </Typography>
                </Box>
                <Stack
                  direction="row"
                  spacing={0.75}
                  useFlexGap
                  sx={{ flexWrap: "wrap" }}
                >
                  <Chip
                    size="small"
                    color={learningEvidenceStatusColor(
                      selectedPatternCard.status,
                    )}
                    label={selectedPatternCard.status}
                  />
                  {selectedPatternCard.chips.map((chip) => (
                    <Chip
                      key={`${selectedPatternCard.key}-${chip}`}
                      size="small"
                      variant="outlined"
                      label={chip}
                    />
                  ))}
                </Stack>
              </Stack>

              <Grid2 container spacing={1.25}>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <Box className="metadata-box">
                    <Stack spacing={0.55}>
                      <Typography variant="subtitle2">What happened</Typography>
                      <Typography
                        variant="body2"
                        sx={{ whiteSpace: "pre-wrap" }}
                      >
                        {selectedPatternCard.detail}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", whiteSpace: "pre-wrap" }}
                      >
                        {selectedPatternCard.rationale ||
                          "AgentArk is still comparing repeated runs before changing future behavior."}
                      </Typography>
                    </Stack>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <Box className="metadata-box">
                    <Stack spacing={0.55}>
                      <Typography variant="subtitle2">
                        Why this is being tracked
                      </Typography>
                      {selectedPatternCard.latestSeen ? (
                        <Typography variant="body2">
                          <strong>Latest seen:</strong>{" "}
                          {selectedPatternCard.latestSeen}
                        </Typography>
                      ) : null}
                      {selectedPatternCard.toolSummary ? (
                        <Typography
                          variant="body2"
                          sx={{ whiteSpace: "pre-wrap" }}
                        >
                          <strong>Tools used:</strong>{" "}
                          {selectedPatternCard.toolSummary}
                        </Typography>
                      ) : null}
                      {selectedPatternCard.evidence ? (
                        selectedPatternCard.evidence
                          .split(" | ")
                          .map((line, idx) => (
                            <Typography
                              key={`${selectedPatternCard.key}-evidence-${idx}`}
                              variant="caption"
                              sx={{ color: "text.secondary", display: "block" }}
                            >
                              {line}
                            </Typography>
                          ))
                      ) : (
                        <Typography
                          variant="caption"
                          sx={{ color: "text.secondary" }}
                        >
                          No extra notes recorded yet.
                        </Typography>
                      )}
                    </Stack>
                  </Box>
                </Grid2>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <Box className="metadata-box">
                    <Stack spacing={0.55}>
                      <Typography variant="subtitle2">
                        Example user messages
                      </Typography>
                      {selectedPatternRequests.length > 0 ? (
                        selectedPatternRequests.map((requestText, idx) => (
                          <Typography
                            key={`${selectedPatternCard.key}-request-${idx}`}
                            variant="body2"
                            sx={{ whiteSpace: "pre-wrap" }}
                          >
                            {requestText}
                          </Typography>
                        ))
                      ) : (
                        <Typography
                          variant="caption"
                          sx={{ color: "text.secondary" }}
                        >
                          No user message was stored for these runs.
                        </Typography>
                      )}
                    </Stack>
                  </Box>
                </Grid2>
                {selectedPatternVersionItems.length > 0 ? (
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Box className="metadata-box">
                      <Stack spacing={0.55}>
                        <Typography variant="subtitle2">
                          Internal changes across these runs
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{ color: "text.secondary" }}
                        >
                          Shown only when the underlying setup changed between
                          related runs.
                        </Typography>
                        {selectedPatternVersionItems.map((item) => (
                          <Typography
                            key={`${selectedPatternCard.key}-${item.label}`}
                            variant="body2"
                            sx={{ whiteSpace: "pre-wrap" }}
                          >
                            <strong>{item.label}:</strong>{" "}
                            {item.values.join(", ")}
                          </Typography>
                        ))}
                      </Stack>
                    </Box>
                  </Grid2>
                ) : null}
              </Grid2>

              <Box className="list-shell" sx={{ p: 1.25 }}>
                <Typography variant="subtitle2" sx={{ mb: 1 }}>
                  Observed runs
                </Typography>
                <TableContainer className="table-shell">
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        <TableCell width="18%">When</TableCell>
                        <TableCell width="14%">Result</TableCell>
                        <TableCell width="20%">Tools used</TableCell>
                        <TableCell width="48%">What happened</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {selectedPatternRuns.map((run, idx) => {
                        const runState = titleCaseLabel(
                          normalizeLearningEvidenceState(run) || "observed",
                        );
                        const toolSummary = summarizeLearningEvidenceTools(
                          stringList(run.tool_names),
                        );
                        const summary = summarizeEvolutionPatternRun(run);
                        return (
                          <TableRow
                            key={`${selectedPatternCard.key}-run-${str(run.id, String(idx))}`}
                          >
                            <TableCell>
                              <Typography
                                variant="body2"
                                title={humanTs(str(run.created_at, "-")).tip}
                              >
                                {humanTs(str(run.created_at, "-")).label}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Chip
                                size="small"
                                color={learningEvidenceStatusColor(runState)}
                                label={runState}
                              />
                            </TableCell>
                            <TableCell>
                              <Typography
                                variant="body2"
                                noWrap
                                title={toolSummary || "-"}
                              >
                                {toolSummary || "-"}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              <Typography
                                variant="body2"
                                sx={{
                                  whiteSpace: "pre-wrap",
                                  wordBreak: "break-word",
                                }}
                              >
                                {summary}
                              </Typography>
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
              </Box>
            </Stack>
          ) : null}
        </DialogContent>
        <DialogActions sx={{ borderTop: "1px solid", borderColor: "divider", px: 2.5, py: 1.5 }}>
          <Button variant="outlined" color="secondary" onClick={() => setSelectedPatternCard(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      {tab === "helped" ? (
        <Stack spacing={1.5}>
          <Box className="list-shell" sx={{ p: 1.6 }}>
            <Stack spacing={1.2}>
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "flex-start", md: "center" },
                }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography
                    variant="h6"
                    sx={{ color: "#e8f4ff", fontWeight: 750 }}
                  >
                    Results in plain English
                  </Typography>
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    What Evolve can prove, what it is still measuring, and
                    whether anything needs your decision.
                  </Typography>
                </Box>
                <Chip
                  size="small"
                  color={
                    resultSummarySeverity === "success"
                      ? "success"
                      : resultSummarySeverity === "warning"
                        ? "warning"
                        : "info"
                  }
                  label={resultSummaryTitle}
                />
              </Stack>
              {/* Long "Evolve has not found enough measured evidence…"
                  Alert removed. The four stat cards already say "Confirmed
                  wins: 0", "Still measuring: 0", etc. — adding a paragraph
                  saying the same in prose was double-billing. Stat helpers
                  carry the same information in tighter form. */}
              <Box
                sx={{
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    sm: "repeat(2, minmax(0, 1fr))",
                    lg: "repeat(4, minmax(0, 1fr))",
                  },
                  gap: 1,
                }}
              >
                {resultSummaryCards.map((card) => (
                  <ResultSummaryCard
                    key={card.label}
                    label={card.label}
                    value={card.value}
                    helper={card.helper}
                    tone={card.tone}
                  />
                ))}
              </Box>
            </Stack>
          </Box>
          <Grid2 container spacing={1.5} sx={{ alignItems: "flex-start" }}>
          {/* "What helped" section is hidden entirely when there's no
              impact data yet. Showing a section header + a long info
              banner saying "no impact yet" + three 0.0-pt metric cards
              was the loudest version of "nothing to show" possible.
              Render nothing instead. */}
          {!detailLoading && !detailError && skillHelpedItems.length === 0 && helpedLines.length === 0 ? null : (
          <Grid2 size={{ xs: 12, lg: 7 }}>
            <Box className="list-shell" sx={{ p: 1.6 }}>
              <Typography
                variant="h6"
                sx={{ color: "#e8f4ff", fontWeight: 700 }}
              >
                What helped
              </Typography>
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                  mb: 1,
                }}
              >
                Evidence from recent runs that affected routing, prompts, or
                delegated work.
              </Typography>
              {detailLoading ? (
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{
                    alignItems: "center",
                  }}
                >
                  <CircularProgress size={16} />
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Loading impact data...
                  </Typography>
                </Stack>
              ) : detailError ? (
                <Alert severity="warning" sx={{ borderRadius: 1 }}>
                  Impact details are unavailable: {detailError}
                </Alert>
              ) : (
                <Stack spacing={1}>
                  {skillHelpedItems.map((row, idx) => {
                    const assessment = asRecord(row.impact_assessment);
                    const metricRows = skillEvolutionMetricRows(row);
                    return (
                      <Box
                        key={`skill-helped-${str(row.id, String(idx))}`}
                        sx={{
                          pb: 1,
                          borderBottom: "1px solid var(--ui-rgba-145-170-205-120)",
                        }}
                      >
                        <Stack
                          direction="row"
                          spacing={1}
                          useFlexGap
                          sx={{ alignItems: "center", flexWrap: "wrap" }}
                        >
                          <Typography
                            variant="body2"
                            sx={{ color: "#e8f4ff", fontWeight: 600 }}
                          >
                            {str(row.skill_name, "Skill")}
                          </Typography>
                          <Chip
                            size="small"
                            label={skillEvolutionActionLabel(
                              str(row.action, ""),
                            )}
                          />
                          <Chip
                            size="small"
                            color={skillEvolutionChipColor(
                              str(row.impact_status, "improved"),
                            )}
                            label={humanizeStatusLabel(str(row.impact_status, "improved"))}
                          />
                        </Stack>
                        <Typography
                          variant="body2"
                          sx={{ color: "text.secondary", mt: 0.45 }}
                        >
                          {str(
                            row.diff_summary,
                            str(row.summary, "Measured improvement recorded."),
                          )}
                        </Typography>
                        {stringList(assessment.summary).map(
                          (line, summaryIdx) => (
                            <Typography
                              key={`skill-helped-summary-${idx}-${summaryIdx}`}
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                                display: "block",
                                mt: 0.35,
                              }}
                            >
                              {line}
                            </Typography>
                          ),
                        )}
                        <Box
                          sx={{
                            mt: 1,
                            display: "grid",
                            gridTemplateColumns: {
                              xs: "1fr",
                              sm: "repeat(3, minmax(0,1fr))",
                            },
                            gap: 1,
                          }}
                        >
                          {metricRows.map((metric) => (
                            <Box
                              key={`${str(row.id, "skill")}-${metric.label}`}
                              sx={{
                                p: 0.9,
                                border: "1px solid var(--ui-rgba-145-170-205-120)",
                                borderRadius: 1,
                              }}
                            >
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                  display: "block",
                                }}
                              >
                                {metric.label}
                              </Typography>
                              <Typography variant="body2">
                                {metric.before}
                                {" -> "}
                                {metric.after}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{
                                  color:
                                    metric.positive == null
                                      ? "text.secondary"
                                      : metric.positive
                                        ? "#14f195"
                                        : "#fb7185",
                                }}
                              >
                                {metric.delta}
                              </Typography>
                            </Box>
                          ))}
                        </Box>
                      </Box>
                    );
                  })}
                  {helpedLines.slice(0, 8).map((line, idx) => (
                    <Alert
                      key={`evolution-helped-${idx}`}
                      severity="success"
                      sx={{ borderRadius: 1 }}
                    >
                      {line}
                    </Alert>
                  ))}
                </Stack>
              )}
              <Box
                sx={{
                  mt: 1.25,
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    md: "repeat(3, minmax(0, 1fr))",
                  },
                  gap: 1,
                }}
              >
                {evidenceMetricCards.map((metric) => (
                  <ResultSummaryCard
                    key={metric.label}
                    label={metric.label}
                    value={metric.value}
                    helper={metric.helper}
                    tone="info"
                  />
                ))}
              </Box>
            </Box>
          </Grid2>
          )}
          <Grid2 size={{ xs: 12, lg: 5 }}>
            <Stack spacing={1.5}>
              {/* "Still observing" section is hidden entirely when no
                  approved change is waiting on more evidence — the
                  long info banner saying "No approved changes are
                  waiting" was an empty state pretending to be content. */}
              {!detailLoading && !detailError && skillObservedItems.length === 0 ? null : (
              <Box className="list-shell" sx={{ p: 1.6 }}>
                <Typography
                  variant="h6"
                  sx={{ color: "#e8f4ff", fontWeight: 700 }}
                >
                  Still observing
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                    mb: 1,
                  }}
                >
                  Approved changes that have traffic, but have not cleared
                  the improvement threshold yet.
                </Typography>
                {detailLoading ? (
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: "center" }}
                  >
                    <CircularProgress size={16} />
                    <Typography
                      variant="body2"
                      sx={{ color: "text.secondary" }}
                    >
                      Loading observed skill metrics...
                    </Typography>
                  </Stack>
                ) : detailError ? (
                  <Alert severity="warning" sx={{ borderRadius: 1 }}>
                    Observed skill metrics are unavailable: {detailError}
                  </Alert>
                ) : (
                  <Stack spacing={1}>
                    {skillObservedItems.slice(0, 6).map((row, idx) => {
                      const metricRows = skillEvolutionMetricRows(row);
                      return (
                        <Box
                          key={`skill-observed-${str(row.id, String(idx))}`}
                          sx={{
                            pb: 1,
                            borderBottom: "1px solid var(--ui-rgba-145-170-205-120)",
                          }}
                        >
                          <Stack
                            direction="row"
                            spacing={0.75}
                            useFlexGap
                            sx={{
                              alignItems: "center",
                              flexWrap: "wrap",
                              mb: 0.35,
                            }}
                          >
                            <Typography
                              variant="body2"
                              sx={{ color: "#e8f4ff", fontWeight: 600 }}
                            >
                              {canonicalSkillIdentifier(
                                str(row.skill_name, "Skill"),
                              )}
                            </Typography>
                            <Chip
                              size="small"
                              label={skillEvolutionActionLabel(
                                str(row.action, ""),
                              )}
                            />
                            <Chip
                              size="small"
                              color={skillEvolutionChipColor(
                                str(row.impact_status, "pending"),
                              )}
                              label={humanizeStatusLabel(str(row.impact_status, "pending"))}
                            />
                          </Stack>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                              display: "block",
                              mb: 0.55,
                            }}
                          >
                            {str(
                              row.diff_summary,
                              str(row.summary, "Observed after approval."),
                            )}
                          </Typography>
                          {metricRows.map((metric) => (
                            <Typography
                              key={`${str(row.id, "skill-observed")}-${metric.label}`}
                              variant="caption"
                              sx={{ color: "text.secondary", display: "block" }}
                            >
                              {metric.label}: {metric.before}
                              {" -> "}
                              {metric.after} ({metric.delta})
                            </Typography>
                          ))}
                        </Box>
                      );
                    })}
                  </Stack>
                )}
              </Box>
              )}
              <Box className="list-shell" sx={{ p: 1.6 }}>
                <Typography
                  variant="h6"
                  sx={{ color: "#e8f4ff", fontWeight: 700 }}
                >
                  Experience graph
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                    mb: 1,
                  }}
                >
                  Saved runs, learned items, reusable patterns, and review candidates.
                </Typography>
                {detailLoading ? (
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: "center",
                    }}
                  >
                    <CircularProgress size={16} />
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      Loading graph...
                    </Typography>
                  </Stack>
                ) : detailError ? (
                  <Alert severity="warning" sx={{ borderRadius: 1 }}>
                    Experience graph is unavailable: {detailError}
                  </Alert>
                ) : experienceGraphNodes.length === 0 ? (
                  <Alert severity="info" sx={{ borderRadius: 1 }}>
                    No saved experience graph nodes yet.
                  </Alert>
                ) : !experienceGraphReady ? (
                  <Stack spacing={1}>
                    <Stack
                      direction="row"
                      spacing={0.75}
                      useFlexGap
                      sx={{ flexWrap: "wrap" }}
                    >
                      <Chip
                        size="small"
                        label={`${experienceGraphNodes.length} nodes`}
                      />
                      <Chip
                        size="small"
                        label={`${experienceGraphEdges.length} edges`}
                      />
                    </Stack>
                    <Alert severity="info" sx={{ borderRadius: 1 }}>
                      The graph is still forming. There are not enough connected
                      runs and learned items to draw a useful network yet.
                    </Alert>
                  </Stack>
                ) : (
                  <Stack spacing={1}>
                    <Stack
                      direction="row"
                      spacing={0.75}
                      useFlexGap
                      sx={{ flexWrap: "wrap" }}
                    >
                      <Chip
                        size="small"
                        label={`${experienceGraphNodes.length} nodes`}
                      />
                      <Chip
                        size="small"
                        label={`${experienceGraphEdges.length} edges`}
                      />
                      <Chip size="small" label="Global learning" />
                    </Stack>
                    <ReactECharts
                      option={experienceGraphOption}
                      style={{ height: 260, width: "100%" }}
                    />
                  </Stack>
                )}
              </Box>
              <Box className="list-shell" sx={{ p: 1.6 }}>
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{
                    alignItems: "center",
                    justifyContent: "space-between",
                    cursor: "pointer",
                  }}
                  onClick={() => setOptimizationOpen((value) => !value)}
                >
                  <Box sx={{ minWidth: 0 }}>
                    <Typography
                      variant="h6"
                      sx={{ color: "#e8f4ff", fontWeight: 700 }}
                    >
                      Optimization graph
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Success and error rates for the versions with recent traffic.
                    </Typography>
                  </Box>
                  <Button
                    size="small"
                    variant="text"
                    sx={{ flexShrink: 0, color: "text.secondary" }}
                  >
                    {optimizationOpen ? "Hide" : "Show"}
                  </Button>
                </Stack>
                <Collapse in={optimizationOpen} mountOnEnter timeout={220}>
                <Box sx={{ mt: 1 }}>
                {detailLoading ? (
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: "center",
                    }}
                  >
                    <CircularProgress size={16} />
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Loading optimization data...
                    </Typography>
                  </Stack>
                ) : detailError ? (
                  <Alert severity="warning" sx={{ borderRadius: 1 }}>
                    Optimization data is unavailable: {detailError}
                  </Alert>
                ) : metricChartRows.length === 0 ? (
                  <Alert severity="info" sx={{ borderRadius: 1 }}>
                    No version metrics yet.
                  </Alert>
                ) : metricChartRows.length === 1 ? (
                  <Stack spacing={1}>
                    {optimizationMetricSummaries.map((metric) => (
                      <Box
                        key={metric.key}
                        sx={{
                          p: 1,
                          border: "1px solid var(--ui-rgba-145-170-205-120)",
                          borderRadius: 1,
                          bgcolor: "rgba(8, 14, 24, 0.28)",
                        }}
                      >
                        <Typography
                          variant="body2"
                          sx={{ color: "#e8f4ff", fontWeight: 700 }}
                          noWrap
                          title={metric.label}
                        >
                          {metric.label}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block",
                            mb: 1,
                          }}
                        >
                          {metric.helper}
                        </Typography>
                        <Stack spacing={0.85}>
                          <ResultProgressRow
                            label="Success"
                            value={metric.success}
                            helper="Higher is better"
                            color="#14f195"
                          />
                          <ResultProgressRow
                            label="Error"
                            value={metric.error}
                            helper="Lower is better"
                            color="#fb7185"
                          />
                        </Stack>
                      </Box>
                    ))}
                  </Stack>
                ) : (
                  <ReactECharts
                    option={metricChartOption}
                    style={{ height: 260, width: "100%" }}
                  />
                )}
                </Box>
                </Collapse>
              </Box>
            </Stack>
          </Grid2>
          </Grid2>
        </Stack>
      ) : null}
      {tab === "tests" ? (
        <Stack spacing={1.5}>
          {activeExperimentItems.length === 0 ? (
            <Box className="list-shell" sx={{ p: 1.75 }}>
              <Stack spacing={1.2}>
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={1}
                  sx={{
                    justifyContent: "space-between",
                    alignItems: { xs: "flex-start", sm: "center" },
                  }}
                >
                  <Box>
                    <Typography
                      variant="h6"
                      sx={{ color: "#e8f4ff", fontWeight: 700 }}
                    >
                      No active experiments
                    </Typography>
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      Evolve is using the current stable behavior across reply
                      routing, main replies, adaptive prompt guidance, request
                      understanding, and specialist helpers.
                    </Typography>
                  </Box>
                  <Chip size="small" label="Stable" />
                </Stack>
                <Alert severity="info" sx={{ borderRadius: 1 }}>
                  When Evolve starts testing a new improvement, this page will
                  explain what is changing, why it could help, how much traffic
                  is included, and what decision is still pending.
                </Alert>
              </Stack>
            </Box>
          ) : (
            <Grid2 container spacing={1.5}>
              {activeExperimentItems.map((item) => {
                const reasonLabels = replayGateReasonLabels(item);
                return (
                <Grid2 key={item.key} size={{ xs: 12, lg: 6 }}>
                  <Box className="list-shell" sx={{ p: 1.6, minHeight: "100%" }}>
                    <Stack spacing={1.15}>
                      <Stack
                        direction={{ xs: "column", sm: "row" }}
                        spacing={1}
                        sx={{
                          justifyContent: "space-between",
                          alignItems: { xs: "flex-start", sm: "center" },
                        }}
                      >
                        <Box sx={{ minWidth: 0 }}>
                          <Typography
                            variant="h6"
                            sx={{ color: "#e8f4ff", fontWeight: 700 }}
                          >
                            {item.audienceLabel}
                          </Typography>
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary", mt: 0.35 }}
                          >
                            {item.summary}
                          </Typography>
                        </Box>
                        <Chip size="small" color="warning" label="Testing" />
                      </Stack>
                      <EvolutionRolloutBar
                        label="Recent traffic in this experiment"
                        percent={item.rollout}
                      />
                      <Box
                        sx={{
                          display: "grid",
                          gridTemplateColumns: { xs: "1fr", md: "repeat(2, minmax(0,1fr))" },
                          gap: 1,
                        }}
                      >
                        <Box>
                          <Typography
                            variant="caption"
                            sx={{ color: "text.secondary", display: "block" }}
                          >
                            Why this is being tested
                          </Typography>
                          <Typography variant="body2">{item.benefit}</Typography>
                        </Box>
                        <Box>
                          <Typography
                            variant="caption"
                            sx={{ color: "text.secondary", display: "block" }}
                          >
                            Current status
                          </Typography>
                          <Typography variant="body2">
                            {evolutionExperimentStatusText(item)}
                          </Typography>
                          {reasonLabels.length > 0 ? (
                            <Box component="ul" sx={{ pl: 2.25, mt: 0.5, mb: 0 }}>
                              {reasonLabels.map((label, reasonIdx) => (
                                <Typography
                                  key={`${item.key}-tests-reason-${reasonIdx}`}
                                  component="li"
                                  variant="body2"
                                  sx={{ color: "text.secondary", lineHeight: 1.5 }}
                                >
                                  {label}
                                </Typography>
                              ))}
                            </Box>
                          ) : null}
                        </Box>
                      </Box>
                      {(() => {
                        const metricSummaries = buildExperimentMetricSummaries(item, null);
                        if (metricSummaries.length === 0) return null;
                        return (
                          <Box
                            sx={{
                              display: "grid",
                              gridTemplateColumns: {
                                xs: "1fr 1fr",
                                md: "repeat(4, minmax(0, 1fr))",
                              },
                              gap: 0.75,
                            }}
                          >
                            {metricSummaries.map((metric) => {
                              const valueColor =
                                metric.tone === "good"
                                  ? "#8ee3b1"
                                  : metric.tone === "warn"
                                    ? "#ffd180"
                                    : "#e8f4ff";
                              return (
                                <Box
                                  key={`${item.key}-tests-${metric.label}`}
                                  sx={{
                                    minWidth: 0,
                                    p: 1,
                                    border: "1px solid var(--ui-rgba-145-170-205-120)",
                                    borderRadius: 1,
                                    bgcolor: "rgba(8, 14, 24, 0.38)",
                                  }}
                                >
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      lineHeight: 1.35,
                                    }}
                                  >
                                    {metric.label}
                                  </Typography>
                                  <Typography
                                    variant="body2"
                                    sx={{
                                      color: valueColor,
                                      fontWeight: 700,
                                      mt: 0.2,
                                      wordBreak: "break-word",
                                    }}
                                  >
                                    {metric.value}
                                  </Typography>
                                  {metric.helper ? (
                                    <Typography
                                      variant="caption"
                                      sx={{
                                        color: "text.secondary",
                                        display: "block",
                                        mt: 0.25,
                                        lineHeight: 1.4,
                                      }}
                                    >
                                      {metric.helper}
                                    </Typography>
                                  ) : null}
                                </Box>
                              );
                            })}
                          </Box>
                        );
                      })()}
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{ flexWrap: "wrap" }}
                      >
                        {item.acceptAction ? (
                          <Button
                            size="small"
                            variant="contained"
                            disabled={runEvolutionActionMutation.isPending}
                            onClick={() =>
                              void runEvolutionAction(
                                item.acceptAction!,
                                "Accepted as stable. Rollback is available.",
                                `Accept ${item.audienceLabel} as stable now?`,
                              )
                            }
                          >
                            Accept as stable
                          </Button>
                        ) : null}
                        {item.stopAction ? (
                          <Button
                            size="small"
                            color="inherit"
                            disabled={runEvolutionActionMutation.isPending}
                            onClick={() =>
                              void runEvolutionAction(
                                item.stopAction!,
                                "Live test stopped.",
                                `Stop the ${item.audienceLabel} live test now?`,
                              )
                            }
                          >
                            Stop test
                          </Button>
                        ) : null}
                      </Stack>
                      <Accordion disableGutters className="chat-workspace-section">
                        <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                          <Typography variant="body2">Technical details</Typography>
                        </AccordionSummary>
                        <AccordionDetails sx={{ pt: 0 }}>
                          <Stack spacing={1}>
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary", display: "block" }}
                            >
                              Internal surface: {item.name}
                            </Typography>
                            <Box
                              sx={{
                                display: "grid",
                                gridTemplateColumns: { xs: "1fr", sm: "1fr 1fr" },
                                gap: 1,
                              }}
                            >
                              <Box>
                                <Typography
                                  variant="caption"
                                  sx={{ color: "text.secondary", display: "block" }}
                                >
                                  Current baseline
                                </Typography>
                                <Typography variant="body2" title={item.baseline}>
                                  {item.baseline}
                                </Typography>
                              </Box>
                              <Box>
                                <Typography
                                  variant="caption"
                                  sx={{ color: "text.secondary", display: "block" }}
                                >
                                  Candidate
                                </Typography>
                                <Typography variant="body2" title={item.candidate}>
                                  {item.candidate}
                                </Typography>
                              </Box>
                            </Box>
                            <Alert severity="info" sx={{ borderRadius: 1 }}>
                              Gate result: {item.gate === "-" ? "No gate result yet." : item.gate}
                            </Alert>
                          </Stack>
                        </AccordionDetails>
                      </Accordion>
                    </Stack>
                  </Box>
                </Grid2>
                );
              })}
            </Grid2>
          )}
          {developerModeEnabled && stableExperimentItems.length > 0 ? (
            <Accordion disableGutters className="chat-workspace-section">
              <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                <Typography variant="body2">
                  Stable surfaces and baselines
                </Typography>
              </AccordionSummary>
              <AccordionDetails sx={{ pt: 0 }}>
                <Stack spacing={1}>
                  {stableExperimentItems.map((item) => (
                    <Box
                      key={`stable-${item.key}`}
                      sx={{
                        p: 1,
                        border: "1px solid var(--ui-rgba-145-170-205-120)",
                        borderRadius: 1,
                      }}
                    >
                      <Typography
                        variant="body2"
                        sx={{ color: "#e8f4ff", fontWeight: 600 }}
                      >
                        {item.audienceLabel}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", display: "block", mt: 0.35 }}
                      >
                        {item.stableSummary}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", display: "block", mt: 0.35 }}
                      >
                        Baseline: {item.baseline}
                      </Typography>
                      {item.rollbackAvailable && item.rollbackAction ? (
                        <Button
                          size="small"
                          color="inherit"
                          sx={{ mt: 0.75 }}
                          disabled={runEvolutionActionMutation.isPending}
                          onClick={() =>
                            void runEvolutionAction(
                              item.rollbackAction!,
                              "Rolled back to the previous stable behavior.",
                              `Roll back ${item.audienceLabel} to the previous stable version?`,
                            )
                          }
                        >
                          Roll back stable change
                        </Button>
                      ) : null}
                    </Box>
                  ))}
                </Stack>
              </AccordionDetails>
            </Accordion>
          ) : null}
        </Stack>
      ) : null}
      {tab === "review" ? (
        <Stack spacing={1.5}>
          <Box
            id="ark-evolve-review-queue"
            tabIndex={-1}
            className="list-shell"
            sx={{
              p: 1.6,
              scrollMarginTop: 16,
              "&:focus": { outline: "none" },
            }}
          >
            <Stack
              direction="row"
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: "flex-start",
                mb: 1.25,
                gap: 1.5,
              }}
            >
              <Box sx={{ minWidth: 0, flex: 1 }}>
                <Typography
                  variant="subtitle1"
                  sx={{
                    color: "var(--text-primary)",
                    fontWeight: 600,
                    fontSize: "0.95rem",
                    lineHeight: 1.3,
                  }}
                >
                  Review queue
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "var(--text-secondary)",
                    fontSize: "0.78rem",
                    lineHeight: 1.4,
                    mt: 0.25,
                  }}
                >
                  Improvements Evolve drafted from recent runs. Each one
                  waits on your decision before AgentArk behavior changes.
                  Ranked by estimated savings and recent slow, costly,
                  corrected, or failed samples.
                </Typography>
              </Box>
              <FormControlLabel
                control={
                  <Switch
                    size="small"
                    checked={showSuperseded}
                    onChange={(event) => setShowSuperseded(event.target.checked)}
                  />
                }
                label="Show past decisions"
                sx={{
                  m: 0,
                  flex: "0 0 auto",
                  alignSelf: "center",
                  "& .MuiTypography-root": {
                    fontFamily: "var(--font-mono)",
                    fontSize: "0.7rem",
                    textTransform: "uppercase",
                    color: "var(--text-secondary)",
                    letterSpacing: 0.04,
                  },
                }}
              />
            </Stack>
            {arkdistillContextSamples != null && arkdistillContextSamples > 0 ? (
              <Box
                sx={{
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    md: "minmax(220px, 1.2fr) repeat(3, minmax(120px, 0.7fr))",
                  },
                  gap: 1,
                  alignItems: "center",
                  borderTop: "1px solid var(--ui-rgba-145-170-205-120)",
                  borderBottom: "1px solid var(--ui-rgba-145-170-205-120)",
                  py: 1,
                  mb: 1.25,
                }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "var(--text-faint)",
                      display: "block",
                      fontFamily: "var(--font-mono)",
                      fontSize: "0.62rem",
                      textTransform: "uppercase",
                    }}
                  >
                    ArkDistill context savings
                  </Typography>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "var(--text-secondary)",
                      fontSize: "0.75rem",
                      lineHeight: 1.35,
                    }}
                  >
                    Tool-result context is being reduced before later prompts.
                    Prompt-section proposals below stay separate.
                  </Typography>
                </Box>
                {arkdistillContextStats.map((metric, metricIdx) => (
                  <EvolveStat
                    key={`arkdistill-context-${metric.label}`}
                    label={metric.label}
                    value={metric.value}
                    helper={metric.helper}
                    tone={metric.tone}
                    accent={metric.accent}
                    unit={metric.unit}
                    progress={metric.progress}
                    divider={metricIdx > 0}
                  />
                ))}
              </Box>
            ) : null}
            {detailLoading ? (
              <Stack
                direction="row"
                spacing={1}
                sx={{
                  alignItems: "center",
                }}
              >
                <CircularProgress size={16} />
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Loading approval queue...
                </Typography>
              </Stack>
            ) : detailError ? (
              <Alert severity="warning" sx={{ borderRadius: 1 }}>
                Approval details are unavailable: {detailError}
              </Alert>
            ) : skillReviewItems.length === 0 &&
              visibleNonSkillLearningCandidates.length === 0 &&
              visiblePromptCanarySafetyEvents.length === 0 &&
              visiblePromptOptimizationOpportunities.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                Nothing is waiting on you right now. Approved suggestions still
                in background testing are not deployed, so there is nothing to
                roll back.
              </Typography>
            ) : (
              <Stack spacing={1.5}>
                {visiblePromptCanarySafetyEvents.length > 0 ? (
                  <Box>
                    <Typography
                      variant="subtitle2"
                      sx={{ color: "#e8f4ff", mb: 1 }}
                    >
                      Experiment decisions
                    </Typography>
                    <Stack spacing={1}>
                      {visiblePromptCanarySafetyEvents.slice(0, 8).map((row, idx) => {
                        const eventId = str(row.id, "");
                        const status = str(row.status, "review_recommended");
                        const reviewStatus = str(
                          row.review_status,
                          status || "open",
                        );
                        const reviewedAt = str(row.reviewed_at, "");
                        const createdAt = str(row.created_at, "");
                        const baselineVersion = str(row.baseline_version, "");
                        const candidateVersion = str(row.candidate_version, "");
                        const baselineSamples = num(row.baseline_samples, 0);
                        const candidateSamples = num(row.candidate_samples, 0);
                        const baselineSuccessRate =
                          num(row.baseline_success_rate, 0) * 100;
                        const candidateSuccessRate =
                          num(row.candidate_success_rate, 0) * 100;
                        const successDelta = num(row.success_delta, 0) * 100;
                        const reviewEvidence = promptCanaryReviewEvidence(row);
                        const canReview =
                          status === "review_recommended" &&
                          reviewStatus === "open" &&
                          !!eventId;
                        return (
                          <Box
                            key={`prompt-canary-safety-${eventId || idx}`}
                            sx={{
                              p: 1.25,
                              border: "1px solid var(--ui-rgba-145-170-205-120)",
                              borderRadius: 1,
                            }}
                          >
                            <Stack
                              direction={{ xs: "column", md: "row" }}
                              spacing={1}
                              sx={{
                                justifyContent: "space-between",
                                alignItems: { xs: "flex-start", md: "center" },
                              }}
                            >
                              <Box sx={{ minWidth: 0 }}>
                                <Stack
                                  direction="row"
                                  spacing={0.75}
                                  useFlexGap
                                  sx={{
                                    alignItems: "center",
                                    flexWrap: "wrap",
                                    mb: 0.45,
                                  }}
                                >
                                  <Typography
                                    variant="subtitle1"
                                    sx={{ color: "#e8f4ff", fontWeight: 600 }}
                                  >
                                    {str(row.title, "Experiment needs attention")}
                                  </Typography>
                                  <Chip
                                    size="small"
                                    color={promptCanarySafetyStatusColor(
                                      reviewStatus,
                                    )}
                                    label={
                                      reviewStatus === "open"
                                        ? "Needs decision"
                                        : humanizeStatusLabel(reviewStatus)
                                    }
                                  />
                                </Stack>
                                <Typography variant="body1">
                                  {str(
                                    row.summary,
                                    "Recent traffic suggests this experiment needs a human decision.",
                                  )}
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                    display: "block",
                                    mt: 0.75,
                                  }}
                                >
                                  {promptCanaryActionSummary(row)}
                                </Typography>
                                {reviewedAt ? (
                                  <Typography
                                    variant="body2"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      mt: 0.45,
                                    }}
                                  >
                                    Reviewed {formatTimestampForHumans(reviewedAt).label}
                                  </Typography>
                                ) : createdAt ? (
                                  <Typography
                                    variant="body2"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      mt: 0.45,
                                    }}
                                  >
                                    Recorded {formatTimestampForHumans(createdAt).label}
                                  </Typography>
                                ) : null}
                              </Box>
                              {canReview ? (
                                <Stack
                                  direction="row"
                                  spacing={0.75}
                                  sx={{ flexShrink: 0 }}
                                >
                                  <Button
                                    size="small"
                                    variant="contained"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "disable_prompt_canary_candidate",
                                          candidate_id: eventId,
                                        },
                                        "Experiment stopped.",
                                        "Stop this experiment now?",
                                      )
                                    }
                                  >
                                    Stop test
                                  </Button>
                                  <Button
                                    size="small"
                                    color="inherit"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "keep_prompt_canary_candidate",
                                          candidate_id: eventId,
                                        },
                                        "Recorded decision to keep the experiment active.",
                                      )
                                    }
                                  >
                                    Keep testing
                                  </Button>
                                </Stack>
                              ) : null}
                            </Stack>
                            <Box
                              sx={{
                                display: "flex",
                                justifyContent: "flex-end",
                                mt: 1,
                              }}
                            >
                              <Button
                                size="small"
                                variant="text"
                                onClick={() =>
                                  setTechnicalDialogProposalId(`canary:${eventId}`)
                                }
                              >
                                See technical details
                              </Button>
                            </Box>
                            <Dialog
                              open={
                                technicalDialogProposalId === `canary:${eventId}`
                              }
                              onClose={() => setTechnicalDialogProposalId(null)}
                              maxWidth="md"
                              fullWidth
                            >
                              <DialogTitle>Technical details</DialogTitle>
                              <DialogContent>
                                <EvolutionReviewEvidenceStrip evidence={reviewEvidence} />
                                <Stack spacing={0.75} sx={{ mt: 2 }}>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    Stable version: {baselineVersion || "-"}
                                  </Typography>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    Experiment version: {candidateVersion || "-"}
                                  </Typography>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    Stable behavior: {baselineSuccessRate.toFixed(1)}% over {baselineSamples.toLocaleString()} runs
                                  </Typography>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    Experiment: {candidateSuccessRate.toFixed(1)}% over {candidateSamples.toLocaleString()} runs
                                  </Typography>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    Success delta: {successDelta.toFixed(1)} pts
                                  </Typography>
                                </Stack>
                              </DialogContent>
                              <DialogActions>
                                <Button
                                  onClick={() =>
                                    setTechnicalDialogProposalId(null)
                                  }
                                >
                                  Close
                                </Button>
                              </DialogActions>
                            </Dialog>
                          </Box>
                        );
                      })}
                    </Stack>
                  </Box>
                ) : null}
                {visiblePromptOptimizationOpportunities.length > 0
                  ? (() => {
                      const pageCount = Math.max(
                        1,
                        Math.ceil(
                          visiblePromptOptimizationOpportunities.length /
                            PROMPT_REVIEW_PAGE_SIZE,
                        ),
                      );
                      const page = Math.min(promptReviewPage, pageCount - 1);
                      return (
                  <Box>
                    {/* Removed "Suggestions before behavior changes" header.
                        The chip strip already says "Needs review (N)" — a
                        section sub-header below it was redundant. */}
                    <Stack spacing={1}>
                      {visiblePromptOptimizationOpportunities
                        .slice(
                          page * PROMPT_REVIEW_PAGE_SIZE,
                          page * PROMPT_REVIEW_PAGE_SIZE + PROMPT_REVIEW_PAGE_SIZE,
                        )
                        .map((row, pageIdx) => {
                        const idx = pageIdx + page * PROMPT_REVIEW_PAGE_SIZE;
                        const proposalId = str(row.id, "");
                        const reviewStatus = str(row.review_status, "open").trim().toLowerCase();
                        const lifecycle = promptProposalLifecycle(row);
                        const lifecycleStatus = promptProposalLifecycleStatus(row);
                        const lifecycleLabel = promptProposalLifecycleLabel(lifecycleStatus);
                        const lifecycleReason = str(lifecycle.reason, "");
                        const formattedLifecycleReason = formatPromptLifecycleReason(lifecycleReason);
                        const reasonAccent =
                          lifecycleStatus === "candidate_rejected" ? "#78f2b0" : "#ffbe63";
                        const lifecycleSamples = num(lifecycle.sample_count, 0);
                        const lifecycleRequiredSamples = num(lifecycle.required_samples, 0);
                        const monitoringSummary = stringList(lifecycle.monitoring_summary);
                        const monitoringRegressions = stringList(lifecycle.monitoring_regressions);
                        const rollbackRecommended = toBool(lifecycle.rollback_recommended);
                        const lifecycleChartOption = promptLifecycleSamplesOption(
                          lifecycleSamples,
                          lifecycleRequiredSamples,
                          lifecycleStatus,
                        );
                        const monitoringChartOption = promptMonitoringChartOption(lifecycle);
                        const metricCards = promptMetricCards(lifecycle);
                        const outcome = promptProposalOutcome(lifecycle);
                        // "Measuring" is the no-data fallback of the outcome badge.
                        // Only surface it as a header tag when the proposal is
                        // actually in an active before/after test — otherwise a
                        // never-approved "Needs decision" row misleadingly reads as
                        // "Measuring" when nothing is being measured for it.
                        const lifecycleIsActivelyMeasuring =
                          lifecycleStatus === "running_background_test" ||
                          lifecycleStatus === "testing";
                        const showOutcomeTag =
                          outcome.label.trim().length > 0 &&
                          (outcome.label !== "Measuring" || lifecycleIsActivelyMeasuring);
                        const lifecycleTimes = promptLifecycleTimeRows(lifecycle);
                        const riskLevel = str(row.risk_level, "default");
                        const riskRail =
                          riskLevel === "high"
                            ? "#ff9b9b"
                            : riskLevel === "medium"
                              ? "#ffbe63"
                              : "#78f2b0";
                        const evidence = stringList(row.evidence);
                        const expectedBenefit = stringList(row.expected_benefit);
                        const caveats = stringList(row.caveats);
                        const opportunity = asRecord(row.opportunity);
                        const opportunityChartOption = promptOpportunityIssueChartOption(opportunity);
                        const rowRankLabel = promptOptimizationRankLabel(idx);
                        const rowMetrics = promptOptimizationRowMetrics(opportunity);
                        const holdoutCases = pickRecords(opportunity, "holdout_cases");
                        const footprint = promptOptimizationFootprintValues(opportunity);
                        const holdoutLatencyChartOption = promptHoldoutLatencyChartOption(holdoutCases);
                        const promptShare = promptOptimizationShareLabel(opportunity);
                        const changePreview = asRecord(row.change_preview);
                        const previewBefore = stringList(changePreview.before);
                        const previewAfter = stringList(changePreview.after);
                        const impactEstimate = stringList(changePreview.impact_estimate);
                        const collapsedExplanation =
                          str(row.summary, "").trim() ||
                          expectedBenefit[0] ||
                          evidence[0] ||
                          "Evolve found a possible improvement from recent completed work. Nothing changes unless you approve it.";
                        const isPromptProposalApproved = reviewStatus === "approved";
                        const isPromptProposalDismissed = reviewStatus === "rejected";
                        const canApprove = !!proposalId && !isPromptProposalApproved;
                        const canDismiss =
                          !!proposalId && !isPromptProposalApproved && !isPromptProposalDismissed;
                        const decisionValue = isPromptProposalDismissed
                          ? "Dismissed"
                          : canApprove
                            ? "Needs approval"
                            : lifecycleLabel;
                        const decisionHelper = isPromptProposalDismissed
                          ? "Shown under past decisions; you can approve it later."
                          : canApprove
                            ? "Nothing changes until you approve it."
                            : lifecycleStatus === "candidate_rejected"
                              ? "Candidate was not promoted by the background evaluation gates."
                            : "State is tracked through the lifecycle.";
                        const canManagePromptCanary =
                          !!proposalId &&
                          str(lifecycle.live_surface, "") === "prompt" &&
                          (lifecycleStatus === "testing" ||
                            lifecycleStatus === "test_regression" ||
                            lifecycleStatus === "deployed" ||
                            lifecycleStatus === "rollback_suggested");
                        const hasEnoughLifecycleSamples =
                          lifecycleRequiredSamples <= 0 ||
                          lifecycleSamples >= lifecycleRequiredSamples;
                        const blockedByTerminalGepaJob =
                          lifecycleStatus === "blocked" &&
                          str(lifecycle.job_status, "").trim().toLowerCase() === "blocked";
                        const blockedByActiveGepaWork =
                          lifecycleStatus === "blocked" &&
                          str(lifecycle.job_status, "").trim().toLowerCase() ===
                            "blocked_by_active_gepa_work";
                        const canRunPromptBackgroundTest =
                          !!proposalId &&
                          reviewStatus === "approved" &&
                          hasEnoughLifecycleSamples &&
                          lifecycleStatus === "approved_waiting_for_more_examples";
                        const canRetryPromptBackgroundTest =
                          !!proposalId &&
                          reviewStatus === "approved" &&
                          hasEnoughLifecycleSamples &&
                          (lifecycleStatus === "blocked" ||
                            lifecycleStatus === "candidate_rejected") &&
                          !blockedByTerminalGepaJob &&
                          !blockedByActiveGepaWork;
                        const openPromptProposalDialog = () => {
                          if (!proposalId) return;
                          setPromptDetailTab(
                            canManagePromptCanary || lifecycleStatus === "deployed"
                              ? "deployment"
                              : canRunPromptBackgroundTest || canRetryPromptBackgroundTest
                                ? "background"
                                : lifecycleStatus === "candidate_rejected"
                                  ? "candidate"
                                : "proposal",
                          );
                          setTechnicalDialogProposalId(proposalId);
                        };
                        return (
                          <Box
                            key={`prompt-proposal-${proposalId || idx}`}
                          >
                            <Box
                              component="button"
                              type="button"
                              onClick={openPromptProposalDialog}
                            sx={{
                                width: "100%",
                                border: "1px solid var(--ui-rgba-145-170-205-120)",
                                borderLeft: `3px solid ${riskRail}`,
                                borderRadius: 1.5,
                                background:
                                  "linear-gradient(135deg, rgba(16, 24, 32, 0.55), rgba(8, 12, 18, 0.32))",
                                boxShadow: `inset 3px 0 16px -7px ${riskRail}`,
                                px: 1.9,
                                py: 1.4,
                                appearance: "none",
                                color: "inherit",
                                fontFamily: "inherit",
                                textAlign: "left",
                                cursor: proposalId ? "pointer" : "default",
                                display: "flex",
                                flexDirection: { xs: "column", md: "row" },
                                alignItems: { xs: "stretch", md: "flex-start" },
                                gap: 1.5,
                                transition:
                                  "background 140ms ease, border-color 140ms ease, box-shadow 140ms ease",
                                "&:hover": {
                                  background: proposalId
                                    ? "linear-gradient(135deg, rgba(22, 32, 42, 0.66), rgba(11, 17, 25, 0.42))"
                                    : "linear-gradient(135deg, rgba(16, 24, 32, 0.55), rgba(8, 12, 18, 0.32))",
                                  borderColor: "rgba(120, 242, 176, 0.28)",
                                  boxShadow: proposalId
                                    ? `inset 3px 0 16px -7px ${riskRail}, 0 10px 34px -20px rgba(20, 241, 149, 0.5)`
                                    : `inset 3px 0 16px -7px ${riskRail}`,
                                },
                                "&:focus-visible": {
                                  outline: "2px solid rgba(120, 242, 176, 0.8)",
                                  outlineOffset: 2,
                                },
                              }}
                            >
                              <Box sx={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 0.35 }}>
                                <Box
                                  sx={{
                                    display: "flex",
                                    alignItems: "center",
                                    gap: 0.85,
                                    minWidth: 0,
                                    flexWrap: "wrap",
                                  }}
                                >
                                  <Box
                                    component="span"
                                    sx={{
                                      color: idx === 0 ? "#78f2b0" : "var(--text-faint)",
                                      fontFamily: "var(--font-mono)",
                                      fontSize: "0.6rem",
                                      fontWeight: idx === 0 ? 700 : 500,
                                      textTransform: "uppercase",
                                      letterSpacing: 0.12,
                                      lineHeight: 1.2,
                                      whiteSpace: "nowrap",
                                    }}
                                  >
                                    {rowRankLabel}
                                  </Box>
                                  <Box
                                    component="span"
                                    sx={{ color: "var(--text-faint)", fontSize: "0.6rem", opacity: 0.7 }}
                                  >
                                    ·
                                  </Box>
                                  <Box
                                    component="span"
                                    sx={{
                                      color: "var(--text-faint)",
                                      fontFamily: "var(--font-mono)",
                                      fontSize: "0.6rem",
                                      textTransform: "uppercase",
                                      letterSpacing: 0.1,
                                      lineHeight: 1.2,
                                      whiteSpace: "nowrap",
                                    }}
                                  >
                                    {lifecycleLabel}
                                  </Box>
                                  {showOutcomeTag ? (
                                    <>
                                      <Box
                                        component="span"
                                        sx={{ color: "var(--text-faint)", fontSize: "0.6rem", opacity: 0.7 }}
                                      >
                                        ·
                                      </Box>
                                      <Box
                                        component="span"
                                        sx={{
                                          color: "var(--text-secondary)",
                                          fontFamily: "var(--font-mono)",
                                          fontSize: "0.6rem",
                                          textTransform: "uppercase",
                                          letterSpacing: 0.1,
                                          lineHeight: 1.2,
                                          whiteSpace: "nowrap",
                                        }}
                                      >
                                        {outcome.label}
                                      </Box>
                                    </>
                                  ) : null}
                                </Box>
                                <Typography
                                  variant="subtitle2"
                                  sx={{
                                    color: "#e8f4ff",
                                    fontWeight: 600,
                                    fontSize: "0.92rem",
                                    overflow: "hidden",
                                    textOverflow: "ellipsis",
                                    whiteSpace: "nowrap",
                                    lineHeight: 1.3,
                                  }}
                                >
                                  {str(row.title, "Suggested improvement")}
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "var(--text-secondary)",
                                    fontSize: "0.78rem",
                                    lineHeight: 1.4,
                                    overflow: "hidden",
                                    textOverflow: "ellipsis",
                                    whiteSpace: "nowrap",
                                  }}
                                >
                                  {expectedBenefit[0] || collapsedExplanation}
                                </Typography>
                                {lifecycleStatus === "blocked" || rollbackRecommended ? (
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color: rollbackRecommended ? "#fbbf24" : "var(--text-dim)",
                                      fontSize: "0.72rem",
                                    }}
                                  >
                                    {formattedLifecycleReason ||
                                      "Monitoring found a regression; rollback is recommended."}
                                  </Typography>
                                ) : null}
                              </Box>
                              <Box
                                sx={{
                                  display: "flex",
                                  alignItems: "flex-start",
                                  flexWrap: { xs: "wrap", md: "nowrap" },
                                  gap: { xs: 1.75, md: 0 },
                                  width: { xs: "100%", md: "min(46%, 540px)" },
                                  flex: { md: "0 0 min(46%, 540px)" },
                                  alignSelf: "center",
                                }}
                              >
                                {rowMetrics.map((metric, metricIdx) => (
                                  <EvolveStat
                                    key={`${proposalId}-row-metric-${metric.label}`}
                                    label={metric.label}
                                    value={metric.value}
                                    helper={metric.helper}
                                    tone={metric.tone}
                                    accent={metric.accent}
                                    unit={metric.unit}
                                    progress={metric.progress}
                                    divider={metricIdx > 0}
                                  />
                                ))}
                              </Box>
                              {/* Inline risk indicator — coloured dot + label,
                                  no chip background. Reads as part of the row,
                                  not as a clickable element. */}
                              <Box
                                sx={{
                                  display: "flex",
                                  alignItems: "center",
                                  gap: 0.65,
                                  flex: "0 0 auto",
                                  alignSelf: "center",
                                  pr: 1,
                                }}
                              >
                                <Box
                                  sx={{
                                    width: 7,
                                    height: 7,
                                    borderRadius: "50%",
                                    background: riskRail,
                                    boxShadow: `0 0 8px ${riskRail}`,
                                    flex: "0 0 auto",
                                  }}
                                />
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "var(--text-secondary)",
                                    fontFamily: "var(--font-mono)",
                                    fontSize: "0.62rem",
                                    letterSpacing: 0.18,
                                    textTransform: "uppercase",
                                    whiteSpace: "nowrap",
                                  }}
                                >
                                  {`${riskLevel || "unknown"} risk`}
                                </Typography>
                              </Box>
                              <Box
                                aria-hidden
                                sx={{
                                  flex: "0 0 auto",
                                  alignSelf: "center",
                                  color: "var(--text-faint)",
                                  fontFamily: "var(--font-mono)",
                                  fontSize: "1.1rem",
                                  lineHeight: 1,
                                  pr: 0.5,
                                }}
                              >
                                ›
                              </Box>
                            </Box>
                            <Dialog
                              open={technicalDialogProposalId === proposalId}
                              onClose={() => setTechnicalDialogProposalId(null)}
                              maxWidth="lg"
                              fullWidth
                            >
                              <DialogTitle>
                                <Stack spacing={0.6}>
                                  <Stack
                                    direction={{ xs: "column", md: "row" }}
                                    spacing={1}
                                    sx={{
                                      alignItems: { xs: "flex-start", md: "center" },
                                      justifyContent: "space-between",
                                    }}
                                  >
                                    <Typography variant="h6" sx={{ color: "#e8f4ff", fontWeight: 750 }}>
                                      {str(row.title, "Evolve change")}
                                    </Typography>
                                    <Stack direction="row" spacing={0.75} sx={{ flexWrap: "wrap" }}>
                                      <Chip
                                        size="small"
                                        color={promptProposalLifecycleColor(lifecycleStatus)}
                                        label={lifecycleLabel}
                                      />
                                      <Chip
                                        size="small"
                                        color={promptProposalRiskColor(riskLevel)}
                                        label={`${riskLevel || "unknown"} risk`}
                                      />
                                    </Stack>
                                  </Stack>
                                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                    {collapsedExplanation}
                                  </Typography>
                                </Stack>
                              </DialogTitle>
                              <DialogContent>
                                <Box sx={{ borderBottom: 1, borderColor: "divider", mt: 2, mb: 1.5 }}>
                                  <Tabs
                                    value={promptDetailTab}
                                    onChange={(_event, next) => setPromptDetailTab(next as PromptDetailTab)}
                                    variant="scrollable"
                                    allowScrollButtonsMobile
                                  >
                                    <Tab value="proposal" label="Proposal" />
                                    <Tab value="background" label="Background Test" />
                                    <Tab value="candidate" label="Candidate" />
                                    <Tab value="deployment" label="Deployment" />
                                    <Tab value="monitoring" label="Monitoring" />
                                  </Tabs>
                                </Box>
                                <PromptDetailSection tab="proposal" active={promptDetailTab}>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
                                      gap: 0.75,
                                    }}
                                  >
                                    <ResultSummaryCard
                                      label="Estimated p95 savings"
                                      value={formatEstimatedTokenSavings(
                                        finiteNumber(opportunity.estimated_saved_tokens_p95),
                                      )}
                                      helper={
                                        finiteNumber(opportunity.estimated_saved_cost_usd_p95) == null
                                          ? "Before GEPA validation"
                                          : `${formatApproxMoney(finiteNumber(opportunity.estimated_saved_cost_usd_p95))} p95 cost estimate`
                                      }
                                      tone="good"
                                    />
                                    <ResultSummaryCard
                                      label="Prompt footprint"
                                      value={promptShare}
                                      helper={
                                        finiteNumber(opportunity.p95_chars) == null
                                          ? "Waiting for section telemetry."
                                          : `${Math.round(num(opportunity.p95_chars, 0)).toLocaleString()} chars at p95`
                                      }
                                      tone={promptShare === "-" ? "info" : "warn"}
                                    />
                                    <ResultSummaryCard
                                      label="Decision"
                                      value={decisionValue}
                                      helper={decisionHelper}
                                      tone={canApprove && !isPromptProposalDismissed ? "warn" : "info"}
                                    />
                                  </Box>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: {
                                        xs: "1fr",
                                        lg: "0.8fr 1fr 1fr",
                                      },
                                      gap: 1,
                                    }}
                                  >
                                    <Box
                                      sx={{
                                        minWidth: 0,
                                        border: "1px solid rgba(145, 170, 205, 0.16)",
                                        borderRadius: 1,
                                        p: 1.1,
                                        bgcolor: "rgba(148, 163, 184, 0.04)",
                                      }}
                                    >
                                      <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                        Prompt footprint
                                      </Typography>
                                      <Typography
                                        variant="body2"
                                        sx={{ color: "text.secondary", mb: 0.75 }}
                                      >
                                        Target section is {promptShare} of the p95 prompt.
                                      </Typography>
                                      <Box
                                        sx={{
                                          height: 10,
                                          borderRadius: 999,
                                          overflow: "hidden",
                                          bgcolor: "rgba(148, 163, 184, 0.24)",
                                          display: "flex",
                                        }}
                                      >
                                        <Box
                                          sx={{
                                            width: `${footprint.sharePercent}%`,
                                            bgcolor: "#14f195",
                                          }}
                                        />
                                      </Box>
                                      <Stack spacing={0.55} sx={{ mt: 1 }}>
                                        <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between" }}>
                                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                            Target section
                                          </Typography>
                                          <Typography variant="caption" sx={{ color: "#e8f4ff", fontWeight: 700 }}>
                                            {Math.round(footprint.sectionChars).toLocaleString()} chars
                                          </Typography>
                                        </Stack>
                                        <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between" }}>
                                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                            Rest of prompt
                                          </Typography>
                                          <Typography variant="caption" sx={{ color: "#e8f4ff", fontWeight: 700 }}>
                                            {Math.round(footprint.restChars).toLocaleString()} chars
                                          </Typography>
                                        </Stack>
                                      </Stack>
                                    </Box>
                                    <Box
                                      sx={{
                                        minWidth: 0,
                                        border: "1px solid rgba(145, 170, 205, 0.16)",
                                        borderRadius: 1,
                                        p: 1,
                                        bgcolor: "rgba(148, 163, 184, 0.04)",
                                      }}
                                    >
                                      <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                        Issue concentration
                                      </Typography>
                                      <ReactECharts
                                        option={opportunityChartOption}
                                        style={{ height: 160, width: "100%" }}
                                      />
                                    </Box>
                                    <Box
                                      sx={{
                                        minWidth: 0,
                                        border: "1px solid rgba(145, 170, 205, 0.16)",
                                        borderRadius: 1,
                                        p: 1,
                                        bgcolor: "rgba(148, 163, 184, 0.04)",
                                      }}
                                    >
                                      <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                        Validation samples
                                      </Typography>
                                      {holdoutCases.length > 0 ? (
                                        <ReactECharts
                                          option={holdoutLatencyChartOption}
                                          style={{ height: 160, width: "100%" }}
                                        />
                                      ) : (
                                        <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                          Waiting for enough outcome-heavy samples to build a validation set.
                                        </Typography>
                                      )}
                                    </Box>
                                  </Box>
                                  <Accordion
                                    disableGutters
                                    sx={{
                                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                                      borderRadius: 1,
                                      bgcolor: "rgba(8, 14, 24, 0.24)",
                                      "&::before": { display: "none" },
                                    }}
                                  >
                                    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                                      <Typography variant="body2" sx={{ color: "#e8f4ff", fontWeight: 650 }}>
                                        Evidence details
                                      </Typography>
                                    </AccordionSummary>
                                    <AccordionDetails sx={{ pt: 0 }}>
                                      <Box
                                        sx={{
                                          display: "grid",
                                          gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                                          gap: 1,
                                        }}
                                      >
                                        {[
                                          ["What Evolve saw", evidence],
                                          ["Why this may help", expectedBenefit],
                                          ["What GEPA will try", previewAfter],
                                          ["Checks before rollout", caveats],
                                        ].map(([label, lines]) => {
                                          const detailLines = Array.isArray(lines)
                                            ? lines
                                            : [];
                                          return detailLines.length > 0 ? (
                                            <Box key={`${proposalId}-details-${label}`}>
                                              <Typography variant="caption" sx={{ color: "text.secondary", display: "block", mb: 0.35 }}>
                                                {label}
                                              </Typography>
                                              <Stack spacing={0.35}>
                                                {detailLines.slice(0, 4).map((line, lineIdx) => (
                                                  <Typography
                                                    key={`${proposalId}-detail-${label}-${lineIdx}`}
                                                    variant="caption"
                                                    sx={{ color: "#fff8ed", display: "block" }}
                                                  >
                                                    - {line}
                                                  </Typography>
                                                ))}
                                              </Stack>
                                            </Box>
                                          ) : null;
                                        })}
                                      </Box>
                                    </AccordionDetails>
                                  </Accordion>
                                </PromptDetailSection>
                                <PromptDetailSection tab="background" active={promptDetailTab}>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                                      gap: 1,
                                    }}
                                  >
                                    <Box>
                                      <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                        Stage
                                      </Typography>
                                      <EvolutionLifecycle
                                        steps={reviewLifecycleSteps}
                                        activeIndex={promptProposalLifecycleStep(lifecycleStatus)}
                                      />
                                      <Box sx={{ mt: 1 }}>
                                        <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                          Job: {humanizeStatusLabel(str(lifecycle.job_status, "not queued"))}
                                        </Typography>
                                        <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                          Samples: {promptProposalSampleLabel(lifecycle)}
                                        </Typography>
                                        {formattedLifecycleReason ? (
                                          <Box
                                            sx={{
                                              mt: 1,
                                              p: 1,
                                              border: "1px solid var(--ui-rgba-145-170-205-120)",
                                              borderLeft: `3px solid ${reasonAccent}`,
                                              borderRadius: 1,
                                              background: "rgba(8, 14, 24, 0.36)",
                                            }}
                                          >
                                            <Typography
                                              variant="caption"
                                              sx={{
                                                color: reasonAccent,
                                                display: "block",
                                                fontFamily: "var(--font-mono)",
                                                fontWeight: 700,
                                                letterSpacing: 0.12,
                                                textTransform: "uppercase",
                                                mb: 0.35,
                                              }}
                                            >
                                              Reason
                                            </Typography>
                                            <Typography variant="body2" sx={{ color: "#e8f4ff" }}>
                                              {formattedLifecycleReason}
                                            </Typography>
                                          </Box>
                                        ) : null}
                                      </Box>
                                      {lifecycleTimes.length > 0 ? (
                                        <Stack spacing={0.35} sx={{ mt: 1 }}>
                                          {lifecycleTimes.map((item) => (
                                            <Typography
                                              key={`${proposalId}-time-${item.label}`}
                                              variant="caption"
                                              sx={{ color: "text.secondary" }}
                                            >
                                              {item.label}: {item.value}
                                            </Typography>
                                          ))}
                                        </Stack>
                                      ) : null}
                                    </Box>
                                    <ReactECharts
                                      option={lifecycleChartOption}
                                      style={{ height: 220, width: "100%" }}
                                    />
                                  </Box>
                                </PromptDetailSection>
                                <PromptDetailSection tab="candidate" active={promptDetailTab}>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
                                      gap: 0.75,
                                    }}
                                  >
                                    <ResultSummaryCard
                                      label="Context reduction"
                                      value={
                                        finiteNumber(opportunity.estimated_saved_tokens_p95) == null
                                          ? impactEstimate[0]
                                            ? "Estimated"
                                            : "Not measured yet"
                                          : `${Math.round(num(opportunity.estimated_saved_tokens_p95, 0)).toLocaleString()} tokens`
                                      }
                                      helper={impactEstimate[0] || "Needs structured token/context delta from GEPA run output."}
                                      tone={finiteNumber(opportunity.estimated_saved_tokens_p95) != null || impactEstimate[0] ? "good" : "info"}
                                    />
                                    <ResultSummaryCard
                                      label="Cost impact"
                                      value={formatApproxMoney(finiteNumber(opportunity.estimated_saved_cost_usd_p95))}
                                      helper={
                                        finiteNumber(opportunity.p95_cost_usd) == null
                                          ? "Waiting for cost-bearing samples."
                                          : `Matching samples reached ${formatApproxMoney(finiteNumber(opportunity.p95_cost_usd))} p95 cost.`
                                      }
                                      tone={finiteNumber(opportunity.estimated_saved_cost_usd_p95) != null ? "good" : "info"}
                                    />
                                    <ResultSummaryCard
                                      label="Safety posture"
                                      value={toBool(row.reversible) ? "Reversible" : "Needs care"}
                                      helper="Prompt/profile changes stay reviewable and rollbackable."
                                      tone={toBool(row.reversible) ? "good" : "warn"}
                                    />
                                  </Box>
                                  {(previewBefore.length > 0 || previewAfter.length > 0) ? (
                                    <Box
                                      sx={{
                                        display: "grid",
                                        gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                                        gap: 1,
                                      }}
                                    >
                                      <Box>
                                        <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                          Before
                                        </Typography>
                                        <Stack spacing={0.5}>
                                          {previewBefore.map((line, lineIdx) => (
                                            <Typography
                                              key={`${proposalId}-before-${lineIdx}`}
                                              variant="body2"
                                              sx={{ color: "text.secondary" }}
                                            >
                                              {line}
                                            </Typography>
                                          ))}
                                        </Stack>
                                      </Box>
                                      <Box>
                                        <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                          Candidate direction
                                        </Typography>
                                        <Stack spacing={0.5}>
                                          {previewAfter.map((line, lineIdx) => (
                                            <Typography
                                              key={`${proposalId}-after-${lineIdx}`}
                                              variant="body2"
                                              sx={{ color: "text.secondary" }}
                                            >
                                              {line}
                                            </Typography>
                                          ))}
                                        </Stack>
                                      </Box>
                                    </Box>
                                  ) : null}
                                  {impactEstimate.length > 0 ? (
                                    <Box>
                                      <Typography variant="subtitle2" sx={{ color: "#e8f4ff", mb: 0.5 }}>
                                        Estimated impact
                                      </Typography>
                                      <Stack spacing={0.5}>
                                        {impactEstimate.map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-impact-${lineIdx}`}
                                            variant="body2"
                                            sx={{ color: "text.secondary" }}
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Box>
                                  ) : null}
                                </PromptDetailSection>
                                <PromptDetailSection tab="deployment" active={promptDetailTab}>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
                                      gap: 0.75,
                                    }}
                                  >
                                    <ResultSummaryCard
                                      label="Live state"
                                      value={lifecycleLabel}
                                      helper={str(lifecycle.live_surface, "") ? `Surface: ${str(lifecycle.live_surface, "")}` : "Not live yet."}
                                      tone={canManagePromptCanary ? "good" : "info"}
                                    />
                                    <ResultSummaryCard
                                      label="Rollout"
                                      value={finiteNumber(lifecycle.rollout_percent) == null ? "-" : `${num(lifecycle.rollout_percent, 0)}%`}
                                      helper="Canary rollout share for live prompt tests."
                                      tone="info"
                                    />
                                    <ResultSummaryCard
                                      label="Rollback"
                                      value={toBool(lifecycle.rollback_available) ? "Available" : "Not available"}
                                      helper={rollbackRecommended ? "Rollback is recommended." : "Rollback appears here after stable deployment."}
                                      tone={rollbackRecommended ? "warn" : "info"}
                                    />
                                  </Box>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                                      gap: 1,
                                    }}
                                  >
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Stable: {str(lifecycle.baseline_version, "-")}
                                    </Typography>
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Candidate: {str(lifecycle.candidate_version, "-")}
                                    </Typography>
                                  </Box>
                                  {(lifecycleStatus === "blocked" ||
                                    lifecycleStatus === "candidate_rejected") &&
                                  formattedLifecycleReason ? (
                                    <Alert
                                      severity={lifecycleStatus === "candidate_rejected" ? "info" : "warning"}
                                      sx={{ borderRadius: 1 }}
                                    >
                                      <Typography variant="body2" sx={{ fontWeight: 650, mb: 0.35 }}>
                                        {lifecycleStatus === "candidate_rejected"
                                          ? "Candidate was not promoted"
                                          : "Block reason"}
                                      </Typography>
                                      <Typography variant="body2">
                                        {formattedLifecycleReason}
                                      </Typography>
                                    </Alert>
                                  ) : null}
                                </PromptDetailSection>
                                <PromptDetailSection tab="monitoring" active={promptDetailTab}>
                                  <Box
                                    sx={{
                                      display: "grid",
                                      gridTemplateColumns: {
                                        xs: "1fr 1fr",
                                        md: "repeat(4, minmax(0, 1fr))",
                                      },
                                      gap: 0.75,
                                    }}
                                  >
                                    {metricCards.map((metric) => (
                                      <ResultSummaryCard
                                        key={`${proposalId}-metric-${metric.label}`}
                                        label={metric.label}
                                        value={metric.value}
                                        helper={metric.helper}
                                        tone={metric.tone}
                                      />
                                    ))}
                                  </Box>
                                  <ReactECharts
                                    option={monitoringChartOption}
                                    style={{ height: 240, width: "100%" }}
                                  />
                                  {monitoringRegressions.length > 0 ? (
                                    <Alert
                                      severity={rollbackRecommended ? "error" : "warning"}
                                      sx={{ borderRadius: 1 }}
                                    >
                                      <Stack spacing={0.4}>
                                        {monitoringRegressions.map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-monitoring-regression-${lineIdx}`}
                                            variant="body2"
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Alert>
                                  ) : null}
                                  {monitoringSummary.length > 0 ? (
                                    <Alert severity="info" sx={{ borderRadius: 1 }}>
                                      <Stack spacing={0.4}>
                                        {monitoringSummary.map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-monitoring-summary-${lineIdx}`}
                                            variant="body2"
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Alert>
                                  ) : null}
                                </PromptDetailSection>
                              </DialogContent>
                              <DialogActions>
                                {canDismiss ? (
                                  <Button
                                    color="inherit"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "reject_prompt_optimization_proposal",
                                          candidate_id: proposalId,
                                        },
                                        "Suggestion dismissed. AgentArk behavior has not changed.",
                                        "Dismiss this suggestion? It will move to past decisions, and you can still approve it later from Show past decisions.",
                                      )
                                    }
                                  >
                                    Dismiss
                                  </Button>
                                ) : null}
                                {canApprove ? (
                                  <Button
                                    variant="contained"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "approve_prompt_optimization_proposal",
                                          candidate_id: proposalId,
                                        },
                                        "Approved for the next stage. Background optimization is now attached to this row.",
                                      )
                                    }
                                  >
                                    Approve next stage
                                  </Button>
                                ) : null}
                                {canManagePromptCanary &&
                                (lifecycleStatus === "testing" ||
                                  lifecycleStatus === "test_regression") ? (
                                  <Button
                                    color="inherit"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "disable_prompt_canary",
                                          candidate_id: "prompt",
                                          proposal_id: proposalId,
                                        },
                                        "Live test stopped.",
                                        "Stop this live prompt test now?",
                                      )
                                    }
                                  >
                                    Stop test
                                  </Button>
                                ) : null}
                                {canManagePromptCanary &&
                                lifecycleStatus === "testing" ? (
                                  <Button
                                    variant="contained"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "promote_prompt_canary_candidate",
                                          candidate_id: "prompt",
                                          proposal_id: proposalId,
                                        },
                                        "Accepted as stable. Monitoring continues on this row.",
                                        "Deploy this prompt candidate to AgentArk?",
                                      )
                                    }
                                  >
                                    Deploy to AgentArk
                                  </Button>
                                ) : null}
                                {canManagePromptCanary &&
                                (lifecycleStatus === "deployed" ||
                                  lifecycleStatus === "rollback_suggested") &&
                                toBool(lifecycle.rollback_available) ? (
                                  <Button
                                    color="error"
                                    variant={rollbackRecommended ? "contained" : "outlined"}
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "rollback_prompt_baseline",
                                          candidate_id: "prompt",
                                          proposal_id: proposalId,
                                        },
                                        "Rolled back and recorded as a bad optimization outcome.",
                                        "Roll back this deployed prompt change?",
                                      )
                                    }
                                  >
                                    Roll back
                                  </Button>
                                ) : null}
                                {canRunPromptBackgroundTest || canRetryPromptBackgroundTest ? (
                                  <Button
                                    variant="contained"
                                    disabled={runEvolutionActionMutation.isPending}
                                    onClick={() =>
                                      void runEvolutionAction(
                                        {
                                          action: "approve_prompt_optimization_proposal",
                                          candidate_id: proposalId,
                                        },
                                        "Background test queued.",
                                      )
                                    }
                                  >
                                    {canRetryPromptBackgroundTest
                                      ? "Retry background test"
                                      : "Run background test"}
                                  </Button>
                                ) : null}
                                <Button
                                  onClick={() =>
                                    setTechnicalDialogProposalId(null)
                                  }
                                >
                                  Close
                                </Button>
                              </DialogActions>
                            </Dialog>
                          </Box>
                        );
                      })}
                    </Stack>
                    {pageCount > 1 ? (
                      <Box
                        sx={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          gap: 2,
                          mt: 1.5,
                          pt: 1,
                          borderTop: "1px solid var(--ui-rgba-145-170-205-120)",
                        }}
                      >
                        <Typography
                          sx={{
                            fontFamily: "var(--font-mono)",
                            fontSize: "0.66rem",
                            color: "var(--text-faint)",
                            letterSpacing: 0.04,
                          }}
                        >
                          {`Showing ${page * PROMPT_REVIEW_PAGE_SIZE + 1}–${Math.min(
                            (page + 1) * PROMPT_REVIEW_PAGE_SIZE,
                            visiblePromptOptimizationOpportunities.length,
                          )} of ${visiblePromptOptimizationOpportunities.length} proposals`}
                        </Typography>
                        <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
                          <Button
                            size="small"
                            variant="text"
                            disabled={page <= 0}
                            onClick={() =>
                              setPromptReviewPage((p) => Math.max(0, p - 1))
                            }
                            sx={{
                              minWidth: 28,
                              height: 28,
                              px: 0.75,
                              borderRadius: "7px",
                              fontFamily: "var(--font-mono)",
                              fontSize: "0.95rem",
                              color: "var(--text-faint)",
                              border: "1px solid transparent",
                              "&:hover": {
                                borderColor: "var(--ui-rgba-145-170-205-120)",
                                color: "#78f2b0",
                              },
                              "&.Mui-disabled": { opacity: 0.35 },
                            }}
                            aria-label="Previous page"
                          >
                            ‹
                          </Button>
                          {Array.from({ length: pageCount }).map((_unused, i) => (
                            <Button
                              key={`prompt-review-page-${i}`}
                              size="small"
                              variant="text"
                              onClick={() => setPromptReviewPage(i)}
                              sx={{
                                minWidth: 30,
                                height: 28,
                                px: 0.75,
                                borderRadius: "7px",
                                fontFamily: "var(--font-mono)",
                                fontSize: "0.72rem",
                                color: i === page ? "#78f2b0" : "var(--text-secondary)",
                                border:
                                  i === page
                                    ? "1px solid rgba(120, 242, 176, 0.4)"
                                    : "1px solid transparent",
                                background:
                                  i === page ? "rgba(120, 242, 176, 0.08)" : "transparent",
                                "&:hover": {
                                  borderColor:
                                    i === page
                                      ? "rgba(120, 242, 176, 0.4)"
                                      : "var(--ui-rgba-145-170-205-120)",
                                  color: i === page ? "#78f2b0" : "var(--text-primary)",
                                },
                              }}
                            >
                              {i + 1}
                            </Button>
                          ))}
                          <Button
                            size="small"
                            variant="text"
                            disabled={page >= pageCount - 1}
                            onClick={() =>
                              setPromptReviewPage((p) =>
                                Math.min(pageCount - 1, p + 1),
                              )
                            }
                            sx={{
                              minWidth: 28,
                              height: 28,
                              px: 0.75,
                              borderRadius: "7px",
                              fontFamily: "var(--font-mono)",
                              fontSize: "0.95rem",
                              color: "var(--text-faint)",
                              border: "1px solid transparent",
                              "&:hover": {
                                borderColor: "var(--ui-rgba-145-170-205-120)",
                                color: "#78f2b0",
                              },
                              "&.Mui-disabled": { opacity: 0.35 },
                            }}
                            aria-label="Next page"
                          >
                            ›
                          </Button>
                        </Box>
                      </Box>
                    ) : null}
                  </Box>
                      );
                    })()
                  : null}
                {skillReviewItems.length > 0 ? (
                  <Box>
                    <Typography
                      variant="subtitle2"
                      sx={{ color: "#e8f4ff", mb: 1 }}
                    >
                      Skill changes
                    </Typography>
                    <Stack spacing={1}>
                      {skillReviewItems.map((row, idx) => {
                        const candidateId = str(row.id, "");
                        const status = str(row.approval_status, "draft");
                        const evidence = asRecord(row.evidence);
                        const diffPreview = asRecord(row.diff_preview);
                        const added = stringList(diffPreview.added);
                        const removed = stringList(diffPreview.removed);
                        const headings = stringList(diffPreview.headings);
                        const baseline = asRecord(row.impact_baseline);
                        const failureReasons = stringList(
                          evidence.recent_failure_reasons,
                        );
                        const selectedExamples = stringList(
                          evidence.selected_failure_examples,
                        );
                        const reviewEvidence = skillReviewEvidence(row);
                        const replayGate = asRecord(row.replay_gate);
                        const replayGateStatus = str(replayGate.status, "");
                        const replayGateReason = str(replayGate.reason, "");
                        const replayGateAllows = toBool(replayGate.allow_approval);
                        const readiness = readinessRecord(row.readiness);
                        const readinessAllowsReview = readiness
                          ? toBool(readiness.allows_review)
                          : true;
                        const canReview =
                          !!candidateId &&
                          status !== "approved" &&
                          status !== "rejected";
                        const canApprove =
                          canReview && replayGateAllows && readinessAllowsReview;
                        const readinessBlocker = stringList(
                          readiness?.blockers,
                        )[0];
                        return (
                          <Box
                            key={`skill-review-${candidateId || idx}`}
                            sx={{
                              p: 1.25,
                              border: "1px solid var(--ui-rgba-145-170-205-120)",
                              borderRadius: 1,
                            }}
                          >
                            <Stack
                              direction={{ xs: "column", md: "row" }}
                              spacing={1}
                              sx={{
                                justifyContent: "space-between",
                                alignItems: { xs: "flex-start", md: "center" },
                              }}
                            >
                              <Box sx={{ minWidth: 0 }}>
                                <Stack
                                  direction="row"
                                  spacing={0.75}
                                  useFlexGap
                                  sx={{
                                    alignItems: "center",
                                    flexWrap: "wrap",
                                    mb: 0.45,
                                  }}
                                >
                                  <Typography
                                    variant="subtitle1"
                                    sx={{ color: "#e8f4ff", fontWeight: 600 }}
                                  >
                                    {canonicalSkillIdentifier(
                                      str(
                                        row.skill_name,
                                        str(row.title, "Skill candidate"),
                                      ),
                                    )}
                                  </Typography>
                                  <Chip
                                    size="small"
                                    label={skillEvolutionActionLabel(
                                      str(row.action, ""),
                                    )}
                                  />
                                  <Chip
                                    size="small"
                                    color={skillEvolutionChipColor(status)}
                                    label={
                                      status === "draft"
                                        ? "Needs decision"
                                        : humanizeStatusLabel(status)
                                    }
                                  />
                                  <Chip
                                    size="small"
                                    variant="outlined"
                                    label={`${ratioPercent(row.confidence).toFixed(0)}% confidence`}
                                  />
                                  {replayGateStatus ? (
                                    <Chip
                                      size="small"
                                      color={
                                        replayGateStatus === "passed"
                                          ? "success"
                                          : replayGateStatus === "needs_more_data"
                                            ? "warning"
                                            : "error"
                                      }
                                      label={`Replay: ${humanizeStatusLabel(replayGateStatus)}`}
                                    />
                                  ) : null}
                                  {readiness ? (
                                    <Chip
                                      size="small"
                                      clickable
                                      color={readinessChipColor(
                                        str(readiness.stage, ""),
                                      )}
                                      label={readinessShortLabel(readiness)}
                                      onClick={() =>
                                        setReadinessDialog({
                                          title: str(
                                            row.title,
                                            "Skill change readiness",
                                          ),
                                          readiness,
                                        })
                                      }
                                    />
                                  ) : null}
                                </Stack>
                                <Typography variant="body1">
                                  {str(
                                    row.diff_summary,
                                    str(row.summary, "Reviewable capability change"),
                                  )}
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{ color: "text.secondary", display: "block", mt: 0.75 }}
                                >
                                  Based on {num(baseline.matched_runs, 0)} matched runs with{" "}
                                  {percentageLabel(baseline.success_rate, 1) || "-"} success and{" "}
                                  {percentageLabel(baseline.failure_rate, 1) || "-"} failure.
                                </Typography>
                                {replayGateReason ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary", display: "block", mt: 0.45 }}
                                  >
                                    Replay gate: {replayGateReason}
                                  </Typography>
                                ) : null}
                                {readinessSummary(readiness) ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary", display: "block", mt: 0.45 }}
                                  >
                                    Readiness: {readinessSummary(readiness)}
                                  </Typography>
                                ) : null}
                                {!canApprove && readinessBlocker ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "warning.main", display: "block", mt: 0.45 }}
                                  >
                                    Waiting: {readinessBlocker}
                                  </Typography>
                                ) : null}
                              </Box>
                              <Stack
                                direction="row"
                                spacing={0.75}
                                sx={{ flexShrink: 0 }}
                              >
                                <Button
                                  size="small"
                                  variant="contained"
                                  disabled={
                                    runEvolutionActionMutation.isPending || !canApprove
                                  }
                                  onClick={() =>
                                    void runEvolutionAction(
                                      {
                                        action: "approve_learning_candidate",
                                        candidate_id: candidateId,
                                      },
                                      "Skill change approved.",
                                    )
                                  }
                                >
                                  Approve
                                </Button>
                                <Button
                                  size="small"
                                  color="inherit"
                                  disabled={
                                    runEvolutionActionMutation.isPending || !canReview
                                  }
                                  onClick={() =>
                                    void runEvolutionAction(
                                      {
                                        action: "reject_learning_candidate",
                                        candidate_id: candidateId,
                                      },
                                      "Skill change rejected.",
                                    )
                                  }
                                >
                                  Reject
                                </Button>
                              </Stack>
                            </Stack>
                            <Box
                              sx={{
                                display: "flex",
                                justifyContent: "flex-end",
                                mt: 1,
                              }}
                            >
                              <Button
                                size="small"
                                variant="text"
                                onClick={() =>
                                  setTechnicalDialogProposalId(
                                    `skill:${candidateId}`,
                                  )
                                }
                              >
                                See technical details
                              </Button>
                            </Box>
                            <Dialog
                              open={
                                technicalDialogProposalId ===
                                `skill:${candidateId}`
                              }
                              onClose={() => setTechnicalDialogProposalId(null)}
                              maxWidth="md"
                              fullWidth
                            >
                              <DialogTitle>Technical details</DialogTitle>
                              <DialogContent>
                                <EvolutionReviewEvidenceStrip evidence={reviewEvidence} />
                                <Box
                                  sx={{
                                    display: "grid",
                                    gridTemplateColumns: {
                                      xs: "1fr",
                                      md: "repeat(4, minmax(0,1fr))",
                                    },
                                    gap: 1,
                                    mt: 2,
                                  }}
                                >
                                  <Box>
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Matched runs
                                    </Typography>
                                    <Typography variant="body1">
                                      {num(baseline.matched_runs, 0)}
                                    </Typography>
                                  </Box>
                                  <Box>
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Success
                                    </Typography>
                                    <Typography variant="body1">
                                      {percentageLabel(baseline.success_rate, 1) || "-"}
                                    </Typography>
                                  </Box>
                                  <Box>
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Failure
                                    </Typography>
                                    <Typography variant="body1">
                                      {percentageLabel(baseline.failure_rate, 1) || "-"}
                                    </Typography>
                                  </Box>
                                  <Box>
                                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                                      Tool errors
                                    </Typography>
                                    <Typography variant="body1">
                                      {percentageLabel(baseline.tool_error_rate, 1) || "-"}
                                    </Typography>
                                  </Box>
                                </Box>
                                {headings.length > 0 ? (
                                  <Stack
                                    direction="row"
                                    spacing={0.75}
                                    useFlexGap
                                    sx={{ flexWrap: "wrap", mt: 1.5 }}
                                  >
                                    {headings.map((heading) => (
                                      <Chip
                                        key={`${candidateId}-${heading}`}
                                        size="small"
                                        variant="outlined"
                                        label={heading}
                                      />
                                    ))}
                                  </Stack>
                                ) : null}
                                <Box
                                  sx={{
                                    mt: 1.5,
                                    display: "grid",
                                    gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                                    gap: 1.5,
                                  }}
                                >
                                  <Box>
                                    <Typography
                                      variant="body2"
                                      sx={{
                                        color: "text.secondary",
                                        display: "block",
                                        mb: 0.5,
                                        fontWeight: 600,
                                      }}
                                    >
                                      Added / changed
                                    </Typography>
                                    {added.length === 0 ? (
                                      <Typography
                                        variant="body2"
                                        sx={{ color: "text.secondary" }}
                                      >
                                        No added lines recorded.
                                      </Typography>
                                    ) : (
                                      <Stack spacing={0.5}>
                                        {added.slice(0, 5).map((line, lineIdx) => (
                                          <Typography
                                            key={`${candidateId}-added-${lineIdx}`}
                                            variant="body2"
                                            sx={{
                                              color: "#fff8ed",
                                              display: "block",
                                            }}
                                          >
                                            + {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    )}
                                  </Box>
                                  <Box>
                                    <Typography
                                      variant="body2"
                                      sx={{
                                        color: "text.secondary",
                                        display: "block",
                                        mb: 0.5,
                                        fontWeight: 600,
                                      }}
                                    >
                                      Removed / replaced
                                    </Typography>
                                    {removed.length === 0 ? (
                                      <Typography
                                        variant="body2"
                                        sx={{ color: "text.secondary" }}
                                      >
                                        No removed lines recorded.
                                      </Typography>
                                    ) : (
                                      <Stack spacing={0.5}>
                                        {removed.slice(0, 5).map((line, lineIdx) => (
                                          <Typography
                                            key={`${candidateId}-removed-${lineIdx}`}
                                            variant="body2"
                                            sx={{
                                              color: "#fdb4c0",
                                              display: "block",
                                            }}
                                          >
                                            - {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    )}
                                  </Box>
                                </Box>
                                {failureReasons.length > 0 ||
                                selectedExamples.length > 0 ? (
                                  <Box sx={{ mt: 1.5 }}>
                                    <Typography
                                      variant="body2"
                                      sx={{
                                        color: "text.secondary",
                                        display: "block",
                                        mb: 0.5,
                                        fontWeight: 600,
                                      }}
                                    >
                                      Evidence
                                    </Typography>
                                    <Stack spacing={0.5}>
                                      {failureReasons
                                        .slice(0, 3)
                                        .map((line, lineIdx) => (
                                          <Typography
                                            key={`${candidateId}-failure-${lineIdx}`}
                                            variant="body2"
                                            sx={{
                                              color: "text.secondary",
                                              display: "block",
                                            }}
                                          >
                                            Failure: {line}
                                          </Typography>
                                        ))}
                                      {selectedExamples
                                        .slice(0, 2)
                                        .map((line, lineIdx) => (
                                          <Typography
                                            key={`${candidateId}-selected-${lineIdx}`}
                                            variant="body2"
                                            sx={{
                                              color: "text.secondary",
                                              display: "block",
                                            }}
                                          >
                                            Mismatch: {line}
                                          </Typography>
                                        ))}
                                    </Stack>
                                  </Box>
                                ) : null}
                              </DialogContent>
                              <DialogActions>
                                <Button
                                  onClick={() =>
                                    setTechnicalDialogProposalId(null)
                                  }
                                >
                                  Close
                                </Button>
                              </DialogActions>
                            </Dialog>
                          </Box>
                        );
                      })}
                    </Stack>
                  </Box>
                ) : null}
                {visibleNonSkillLearningCandidates.length > 0 ? (
                  <Box>
                    <Typography
                      variant="subtitle2"
                      sx={{ color: "#e8f4ff", mb: 1 }}
                    >
                      Other suggestions
                    </Typography>
                    <Stack spacing={1}>
                      {visibleNonSkillLearningCandidates.map((row, idx) => {
                        const candidateId = str(row.id, "");
                        const status = str(row.approval_status, "draft");
                        const reviewEvidence = learningCandidateReviewEvidence(
                          row,
                          {
                            strategyBaselineVersion: str(
                              strategyCanary.baseline_version,
                              "",
                            ),
                            patternById: learningPatternById,
                            itemById: learningItemById,
                          },
                        );
                        const replayGate = asRecord(row.replay_gate);
                        const replayGateStatus = str(replayGate.status, "");
                        const replayGateReason = str(replayGate.reason, "");
                        const replayGateAllows = toBool(replayGate.allow_approval);
                        const readiness = readinessRecord(row.readiness);
                        const readinessAllowsReview = readiness
                          ? toBool(readiness.allows_review)
                          : true;
                        const canReview =
                          !!candidateId &&
                          status !== "approved" &&
                          status !== "rejected";
                        const canApprove =
                          canReview && replayGateAllows && readinessAllowsReview;
                        const readinessBlocker = stringList(
                          readiness?.blockers,
                        )[0];
                        return (
                          <Box
                            key={`learning-candidate-${candidateId || idx}`}
                            sx={{
                              p: 1.25,
                              border: "1px solid var(--ui-rgba-145-170-205-120)",
                              borderRadius: 1,
                            }}
                          >
                            <Stack
                              direction={{ xs: "column", md: "row" }}
                              spacing={1}
                              sx={{
                                justifyContent: "space-between",
                                alignItems: { xs: "flex-start", md: "center" },
                              }}
                            >
                              <Box sx={{ minWidth: 0 }}>
                                <Stack
                                  direction="row"
                                  spacing={0.75}
                                  useFlexGap
                                  sx={{
                                    alignItems: "center",
                                    flexWrap: "wrap",
                                    mb: 0.45,
                                  }}
                                >
                                  <Typography
                                    variant="subtitle1"
                                    sx={{ color: "#e8f4ff", fontWeight: 600 }}
                                  >
                                    {str(row.title, str(row.proposed_name, "Suggestion"))}
                                  </Typography>
                                  <Chip
                                    size="small"
                                    label={humanizeStatusLabel(str(row.candidate_type, "candidate"))}
                                  />
                                  <Chip
                                    size="small"
                                    color={
                                      status === "approved"
                                        ? "success"
                                        : status === "draft"
                                          ? "warning"
                                          : "default"
                                    }
                                    label={
                                      status === "draft"
                                        ? "Needs decision"
                                        : humanizeStatusLabel(status)
                                    }
                                  />
                                  <Chip
                                    size="small"
                                    variant="outlined"
                                    label={`${ratioPercent(row.confidence).toFixed(0)}% confidence`}
                                  />
                                  {replayGateStatus ? (
                                    <Chip
                                      size="small"
                                      color={
                                        replayGateStatus === "passed"
                                          ? "success"
                                          : replayGateStatus === "needs_more_data"
                                            ? "warning"
                                            : "error"
                                      }
                                      label={`Replay: ${humanizeStatusLabel(replayGateStatus)}`}
                                    />
                                  ) : null}
                                  {readiness ? (
                                    <Chip
                                      size="small"
                                      clickable
                                      color={readinessChipColor(
                                        str(readiness.stage, ""),
                                      )}
                                      label={readinessShortLabel(readiness)}
                                      onClick={() =>
                                        setReadinessDialog({
                                          title: str(row.title, "Suggestion readiness"),
                                          readiness,
                                        })
                                      }
                                    />
                                  ) : null}
                                </Stack>
                                <Typography variant="body1">
                                  {str(row.summary, str(row.preview, "-"))}
                                </Typography>
                                {replayGateReason ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary", display: "block", mt: 0.45 }}
                                  >
                                    Replay gate: {replayGateReason}
                                  </Typography>
                                ) : null}
                                {readinessSummary(readiness) ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary", display: "block", mt: 0.45 }}
                                  >
                                    Readiness: {readinessSummary(readiness)}
                                  </Typography>
                                ) : null}
                                {!canApprove && readinessBlocker ? (
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "warning.main", display: "block", mt: 0.45 }}
                                  >
                                    Waiting: {readinessBlocker}
                                  </Typography>
                                ) : null}
                              </Box>
                              <Stack
                                direction="row"
                                spacing={0.75}
                                sx={{ flexShrink: 0 }}
                              >
                                <Button
                                  size="small"
                                  variant="contained"
                                  disabled={
                                    runEvolutionActionMutation.isPending || !canApprove
                                  }
                                  onClick={() =>
                                    void runEvolutionAction(
                                      {
                                        action: "approve_learning_candidate",
                                        candidate_id: candidateId,
                                      },
                                      "Suggestion approved.",
                                    )
                                  }
                                >
                                  Approve
                                </Button>
                                <Button
                                  size="small"
                                  color="inherit"
                                  disabled={
                                    runEvolutionActionMutation.isPending || !canReview
                                  }
                                  onClick={() =>
                                    void runEvolutionAction(
                                      {
                                        action: "reject_learning_candidate",
                                        candidate_id: candidateId,
                                      },
                                      "Suggestion rejected.",
                                    )
                                  }
                                >
                                  Reject
                                </Button>
                              </Stack>
                            </Stack>
                            <Box
                              sx={{
                                display: "flex",
                                justifyContent: "flex-end",
                                mt: 1,
                              }}
                            >
                              <Button
                                size="small"
                                variant="text"
                                onClick={() =>
                                  setTechnicalDialogProposalId(
                                    `learning:${candidateId}`,
                                  )
                                }
                              >
                                See technical details
                              </Button>
                            </Box>
                            <Dialog
                              open={
                                technicalDialogProposalId ===
                                `learning:${candidateId}`
                              }
                              onClose={() => setTechnicalDialogProposalId(null)}
                              maxWidth="sm"
                              fullWidth
                            >
                              <DialogTitle>Technical details</DialogTitle>
                              <DialogContent>
                                <EvolutionReviewEvidenceStrip evidence={reviewEvidence} />
                                <Stack spacing={0.75} sx={{ mt: 2 }}>
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary" }}
                                  >
                                    Type: {canonicalSkillIdentifier(str(row.candidate_type, "-"))}
                                  </Typography>
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary" }}
                                  >
                                    Confidence: {ratioPercent(row.confidence).toFixed(0)}%
                                  </Typography>
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary" }}
                                  >
                                    Status: {humanizeStatusLabel(status)}
                                  </Typography>
                                </Stack>
                              </DialogContent>
                              <DialogActions>
                                <Button
                                  onClick={() =>
                                    setTechnicalDialogProposalId(null)
                                  }
                                >
                                  Close
                                </Button>
                              </DialogActions>
                            </Dialog>
                          </Box>
                        );
                      })}
                    </Stack>
                  </Box>
                ) : null}
              </Stack>
            )}
          </Box>
        </Stack>
      ) : null}
      <Dialog
        open={!!readinessDialog}
        onClose={() => setReadinessDialog(null)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>{readinessDialog?.title || "Readiness details"}</DialogTitle>
        <DialogContent dividers>
          {readinessDialog ? (
            <Stack spacing={1.25}>
              <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
                <Chip
                  size="small"
                  color={readinessChipColor(str(readinessDialog.readiness.stage, ""))}
                  label={readinessShortLabel(readinessDialog.readiness)}
                />
                <Chip
                  size="small"
                  variant="outlined"
                  label={
                    toBool(readinessDialog.readiness.allows_auto)
                      ? "Auto-run allowed"
                      : toBool(readinessDialog.readiness.allows_review)
                        ? "Review allowed"
                        : "Watching only"
                  }
                />
              </Stack>
              <Typography variant="body2">
                {readinessSummary(readinessDialog.readiness) ||
                  "Evolve is still collecting enough evidence."}
              </Typography>
              {stringList(readinessDialog.readiness.blockers).length > 0 ? (
                <Alert severity="warning" sx={{ borderRadius: 1 }}>
                  <Stack spacing={0.5}>
                    {stringList(readinessDialog.readiness.blockers).map((line, idx) => (
                      <Typography key={`readiness-blocker-${idx}`} variant="body2">
                        {line}
                      </Typography>
                    ))}
                  </Stack>
                </Alert>
              ) : null}
              {stringList(readinessDialog.readiness.reasons).length > 0 ? (
                <Box>
                  <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                    Evidence
                  </Typography>
                  <Stack spacing={0.4}>
                    {stringList(readinessDialog.readiness.reasons).map((line, idx) => (
                      <Typography
                        key={`readiness-reason-${idx}`}
                        variant="body2"
                        sx={{ color: "text.secondary" }}
                      >
                        {line}
                      </Typography>
                    ))}
                  </Stack>
                </Box>
              ) : null}
              <Accordion disableGutters>
                <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                  <Typography variant="body2">Power-user signals</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <Box
                    component="pre"
                    sx={{
                      m: 0,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      fontSize: 12,
                      color: "text.secondary",
                    }}
                  >
                    {JSON.stringify(readinessDialog.readiness.signals ?? {}, null, 2)}
                  </Box>
                </AccordionDetails>
              </Accordion>
            </Stack>
          ) : null}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setReadinessDialog(null)}>Close</Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
