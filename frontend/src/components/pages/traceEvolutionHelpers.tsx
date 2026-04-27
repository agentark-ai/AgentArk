import { Box, Chip, Stack, Typography } from "@mui/material";
import type { JSX, ReactNode } from "react";
import {
  asRecord,
  isRecord,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";
import { humanizeStatusLabel } from "./workspaceCore";

export function formatTraceDuration(durationMs: unknown): string {
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

export function buildEvolutionFocusCaseLabel(row: JsonRecord): string {
  const surface = str(row.surface, "case").trim();
  const delta = num(row.score_delta, Number.NaN);
  const preview = str(row.prompt_preview, "").trim();
  const invalidBefore = toBool(row.invalid_json_before);
  const invalidAfter = toBool(row.invalid_json_after);
  const parts = [surface];
  if (Number.isFinite(delta))
    parts.push(`${delta >= 0 ? "+" : ""}${(delta * 100).toFixed(0)} pts`);
  if (invalidBefore !== invalidAfter)
    parts.push(invalidAfter ? "JSON regressed" : "JSON stabilized");
  if (preview) parts.push(preview);
  return parts.join(" / ");
}

export function traceStatusColor(
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

export function traceStepColor(
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

export function formatTraceData(value: unknown): string {
  if (typeof value !== "string") return str(value, "");
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

export type TraceEvidenceItem = {
  title: string;
  detail: string;
  type: string;
};

export type TraceStepConsoleView = {
  detail: string;
  dataText: string;
};

export function pickTraceStepArtifacts(step: JsonRecord): JsonRecord[] {
  return pickRecords(step, "artifacts");
}

export function traceArtifactLabel(artifact: JsonRecord): string {
  const explicit = str(artifact.label, "").trim();
  if (explicit) return explicit;
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

export function traceArtifactKindLabel(artifact: JsonRecord): string {
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

export function traceArtifactFormat(artifact: JsonRecord): string {
  return str(artifact.format, "").trim().toUpperCase();
}

export function traceArtifactBody(artifact: JsonRecord): string {
  const raw = artifact.data;
  if (typeof raw === "string") return formatTraceData(raw);
  if (raw == null) return "";
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return str(raw, "");
  }
}

export function traceArtifactSummary(artifact: JsonRecord): string {
  const explicit = str(artifact.summary, "").trim();
  if (explicit) return explicit;
  const body = collapseInlineWhitespace(traceArtifactBody(artifact));
  return body ? truncateTraceEvidence(body, 180) : "";
}

export function traceArtifactChipLabel(artifact: JsonRecord): string {
  const label = traceArtifactLabel(artifact);
  const summary = traceArtifactSummary(artifact);
  if (summary && summary.length <= 56) {
    return `${label}: ${summary}`;
  }
  return label;
}

export function summarizeTraceArtifactsInline(artifacts: JsonRecord[]): string {
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

export function buildTraceArtifactBlocks(artifacts: JsonRecord[]): string {
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

export function truncateTraceEvidence(value: string, max = 240): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, Math.max(0, max - 3)).trimEnd()}...`;
}

export function summarizeTraceOutcome(trace: JsonRecord): string {
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

export function isExecutionProofStep(step: JsonRecord): boolean {
  const combined =
    `${str(step.title, "")}\n${str(step.detail, "")}\n${formatTraceData(step.data)}`.toLowerCase();
  return /execution record saved|execution proof generated|verification id:|proof id:/.test(
    combined,
  );
}

export function buildTraceEvidenceItems(steps: JsonRecord[]): TraceEvidenceItem[] {
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

export function extractTraceArtifacts(
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

export function buildExecutionProofConsoleEvidence(
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

export function buildTraceStepConsoleView(
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

export function parseTraceDataRecord(value: unknown): JsonRecord {
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

export function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => str(item, "").trim()).filter(Boolean);
}

export function percentageLabel(value: unknown, digits = 1): string {
  const parsed = num(value, Number.NaN);
  if (!Number.isFinite(parsed)) return "";
  return `${(parsed * 100).toFixed(digits)}%`;
}

export type EvolutionReviewCard = {
  key: string;
  title: string;
  status: string;
  detail: string;
  chips: string[];
  rationale?: string;
  example?: string;
  evidence?: string;
};

export type EvolutionPatternCard = EvolutionReviewCard & {
  runs: JsonRecord[];
  latestSeen?: string;
  toolSummary?: string;
  completedCount: number;
  failedCount: number;
  acceptedCount: number;
};

export function collapseInlineWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

export function truncateUiText(value: string, maxChars = 120): string {
  const normalized = collapseInlineWhitespace(value);
  if (normalized.length <= maxChars) return normalized;
  return `${normalized.slice(0, Math.max(0, maxChars - 1)).trimEnd()}...`;
}

export function titleCaseLabel(value: string): string {
  return value
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(" ");
}

export function learningEvidenceStatusColor(
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

export function learningEvidenceTimestampMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function latestLearningEvidenceTimestamp(runs: JsonRecord[]): number {
  return runs.reduce(
    (latest, run) =>
      Math.max(latest, learningEvidenceTimestampMs(str(run.created_at, ""))),
    0,
  );
}

export function learningEvidenceToolLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "schedule_task") return "Scheduled task";
  if (normalized === "calendar_create") return "Calendar event";
  return titleCaseLabel(normalized);
}

export function summarizeLearningEvidenceTools(values: string[]): string {
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

export function uniqueNonEmptyStrings(values: Array<unknown>): string[] {
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

export function summarizeEvolutionPatternRun(run: JsonRecord): string {
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

export function evolutionPatternStatusExplanation(card: EvolutionPatternCard): string {
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

export function normalizeLearningEvidenceState(run: JsonRecord): string {
  const decision = asRecord(run.decision_summary);
  return collapseInlineWhitespace(
    str(
      run.correction_state,
      str(decision.completion_status, str(run.success_state, "observed")),
    ),
  ).toLowerCase();
}

export function inferLearningEvidenceTitle(
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

export function buildEvolutionReviewCards(steps: JsonRecord[]): EvolutionReviewCard[] {
  const cards: EvolutionReviewCard[] = [];
  steps.forEach((step, idx) => {
    const data = parseTraceDataRecord(step.data);
    const traceKind = str(data.trace_kind, "").trim().toLowerCase();
    if (!traceKind.startsWith("self_evolve.")) return;

    const status = str(step.type, str(step.step_type, "info")).trim() || "info";
    const title = str(step.title, "ArkEvolve").trim();
    const detail = str(step.detail, "").trim();
    const chips: string[] = [];
    const evidence: string[] = [];
    let rationale = "";

    if (traceKind === "self_evolve.request") {
      const mode = str(data.mode, "policy");
      const request = str(data.request, "").trim();
      chips.push(`Mode ${mode}`);
      if (toBool(data.apply_promotion)) chips.push("Promotion enabled");
      if (toBool(data.allow_code_writes)) chips.push("Code writes allowed");
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
      rationale = `Gate: ${str(data.promotion_gate, "unknown")}`;
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
      rationale = `Gate: ${str(data.promotion_gate, "unknown")}`;
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
    } else if (
      traceKind === "self_evolve.classifier_prompt.result" ||
      traceKind === "self_evolve.specialist_prompt.result"
    ) {
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
      rationale = `Gate: ${str(data.promotion_gate, "unknown")}`;
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
    } else if (
      traceKind === "self_evolve.classifier_prompt.promotion" ||
      traceKind === "self_evolve.specialist_prompt.promotion"
    ) {
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
    } else if (traceKind === "self_evolve.code.blocked") {
      chips.push("Code evolution");
      chips.push("Blocked");
      rationale = str(data.request, "").trim();
    } else if (traceKind === "self_evolve.code.result") {
      const filesChanged = stringList(data.files_changed);
      const securityWarnings = stringList(data.security_warnings);
      const iterations = num(data.iterations_used, 0);
      chips.push(
        `${filesChanged.length} file${filesChanged.length === 1 ? "" : "s"}`,
      );
      chips.push(`${iterations} iteration${iterations === 1 ? "" : "s"}`);
      if (toBool(data.push_recommended)) chips.push("Push suggested");
      rationale = str(data.diff_summary, "").trim();
      if (filesChanged.length)
        evidence.push(`Files changed: ${filesChanged.join(", ")}`);
      if (securityWarnings.length)
        evidence.push(`Security warnings: ${securityWarnings.join(" | ")}`);
      const error = str(data.error, "").trim();
      if (error) evidence.push(`Error: ${error}`);
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

export function evolutionTraceIdHint(payload: unknown): string {
  const traceId = str(asRecord(payload).trace_id, "").trim();
  return traceId ? ` Trace ${traceId.slice(0, 8)} recorded.` : "";
}

export function syncRunStatusColor(
  status: string,
): "success" | "warning" | "error" | "default" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "completed") return "success";
  if (normalized === "failed") return "error";
  if (normalized === "blocked") return "warning";
  return "default";
}

export function syncRunTriggerLabel(trigger: string): string {
  const normalized = trigger.trim().toLowerCase();
  if (normalized === "manual") return "Manual";
  if (normalized === "background") return "Background";
  return normalized ? normalized.replace(/_/g, " ") : "Unknown";
}

export type TraceRange = "1h" | "6h" | "24h" | "7d" | "14d" | "30d";
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

export function traceRangeHours(range: TraceRange): number {
  return TRACE_RANGE_PRESETS.find((p) => p.value === range)?.hours || 168;
}

export function traceRangeSinceISO(range: TraceRange): string {
  const ms = traceRangeHours(range) * 3600 * 1000;
  return new Date(Date.now() - ms).toISOString();
}

export type TraceBucket = { label: string; ts: number };

export function buildTraceTrendBuckets(range: TraceRange): TraceBucket[] {
  const hours = traceRangeHours(range);
  const bucketCount = Math.min(
    hours <= 6 ? hours : hours <= 24 ? 12 : hours <= 168 ? 7 : 14,
    14,
  );
  const spanMs = hours * 3600 * 1000;
  const bucketMs = spanMs / bucketCount;
  const now = Date.now();
  const buckets: TraceBucket[] = [];
  for (let i = 0; i < bucketCount; i++) {
    const ts = now - spanMs + (i + 1) * bucketMs;
    const d = new Date(ts);
    const label =
      hours <= 24
        ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : d.toLocaleDateString([], { month: "short", day: "numeric" });
    buckets.push({ label, ts });
  }
  return buckets;
}

export function bucketizeTraceItems<T>(
  items: T[],
  getTs: (item: T) => string,
  buckets: TraceBucket[],
): number[] {
  const counts = new Array(buckets.length).fill(0) as number[];
  for (const item of items) {
    const ts = new Date(getTs(item)).getTime();
    if (!ts || isNaN(ts)) continue;
    for (let i = buckets.length - 1; i >= 0; i--) {
      const lo = i === 0 ? 0 : buckets[i - 1].ts;
      if (ts > lo && ts <= buckets[i].ts) {
        counts[i]++;
        break;
      }
    }
  }
  return counts;
}

export function traceSecurityEventTypeLabel(eventType: string): string {
  const normalized = (eventType || "").trim().toLowerCase();
  if (!normalized) return "Unknown";
  return normalized
    .replace(/_/g, " ")
    .replace(/\b\w/g, (m) => m.toUpperCase());
}


export function buildEvolutionEvidenceCards(
  runs: JsonRecord[],
): EvolutionPatternCard[] {
  const groupedRuns = new Map<string, JsonRecord[]>();
  runs.forEach((run, idx) => {
    const requestText = collapseInlineWhitespace(str(run.request_text, ""));
    const intentKey = collapseInlineWhitespace(str(run.intent_key, ""));
    const key = requestText
      ? `request:${requestText.toLowerCase()}`
      : intentKey
        ? `intent:${intentKey.toLowerCase()}`
        : `run:${str(run.id, String(idx))}`;
    const existing = groupedRuns.get(key);
    if (existing) {
      existing.push(run);
    } else {
      groupedRuns.set(key, [run]);
    }
  });

  return Array.from(groupedRuns.values())
    .sort(
      (left, right) =>
        latestLearningEvidenceTimestamp(right) -
        latestLearningEvidenceTimestamp(left),
    )
    .map((groupRuns, idx): EvolutionPatternCard | null => {
      const latestRun = groupRuns.reduce((best, current) => {
        return learningEvidenceTimestampMs(str(current.created_at, "")) >=
          learningEvidenceTimestampMs(str(best.created_at, ""))
          ? current
          : best;
      }, groupRuns[0]);
      const taskType = str(latestRun.task_type, "").trim();
      const requestPreview = collapseInlineWhitespace(
        str(latestRun.request_text, ""),
      );
      const title = inferLearningEvidenceTitle(
        groupRuns,
        requestPreview,
        taskType,
      );
      const latestSeen = humanTs(str(latestRun.created_at, "")).label;

      const allToolNames: string[] = [];
      const successfulToolNames: string[] = [];
      const failedToolNames: string[] = [];
      const failureReasons: string[] = [];
      let completedCount = 0;
      let failedCount = 0;
      let acceptedCount = 0;

      groupRuns.forEach((run) => {
        const state = normalizeLearningEvidenceState(run);
        const toolNames = stringList(run.tool_names);
        const failureReason = collapseInlineWhitespace(
          str(run.failure_reason, str(run.outcome_summary, "")),
        );
        toolNames.forEach((toolName) => {
          allToolNames.push(toolName);
          if (
            state === "completed" ||
            state === "success" ||
            state === "succeeded"
          )
            successfulToolNames.push(toolName);
          if (state === "failed" || state === "error")
            failedToolNames.push(toolName);
        });
        if (
          state === "completed" ||
          state === "success" ||
          state === "succeeded"
        )
          completedCount += 1;
        else if (state === "failed" || state === "error") {
          failedCount += 1;
          if (failureReason) failureReasons.push(failureReason);
        } else if (state === "accepted") acceptedCount += 1;
      });

      if (!title && !requestPreview && allToolNames.length === 0) return null;

      const runCount = groupRuns.length;
      const allToolSet = new Set(
        allToolNames.map((name) => name.trim().toLowerCase()).filter(Boolean),
      );
      const successfulToolSet = new Set(
        successfulToolNames
          .map((name) => name.trim().toLowerCase())
          .filter(Boolean),
      );
      const failedToolSet = new Set(
        failedToolNames
          .map((name) => name.trim().toLowerCase())
          .filter(Boolean),
      );
      const dominantBlocker = failureReasons[0]
        ? truncateUiText(failureReasons[0], 110)
        : "";
      const isPreferencePattern =
        acceptedCount > 0 && completedCount === 0 && failedCount === 0;
      const recoveredReminderPath =
        completedCount > 0 &&
        failedCount > 0 &&
        (allToolSet.has("schedule_task") ||
          successfulToolSet.has("schedule_task")) &&
        (allToolSet.has("calendar_create") ||
          failedToolSet.has("calendar_create"));

      let status = "Collecting examples";
      if (isPreferencePattern) status = "User preference captured";
      else if (completedCount > 0 && failedCount > 0) status = "Mixed results";
      else if (completedCount > 1) status = "Repeating successfully";
      else if (completedCount === 1) status = "Seen once";
      else if (failedCount > 0) status = "Needs review";

      let detail = `Captured ${runCount} related run${runCount === 1 ? "" : "s"} for comparison.`;
      let rationale =
        "ArkEvolve is still collecting enough examples to decide whether a product change is warranted.";

      if (isPreferencePattern) {
        detail = `Captured an explicit user correction that should steer similar requests from the start.`;
        rationale =
          "Direct user constraints are stronger evidence than a default integration guess or a broad tool match.";
      } else if (recoveredReminderPath) {
        detail = `Observed ${runCount} similar reminder requests. Earlier attempts went through Calendar and failed. Later runs completed by creating scheduled in-app reminders instead.`;
        rationale =
          "This is concrete evidence that date-based 'notify me' requests do not need Calendar when a local reminder already satisfies the request.";
      } else if (completedCount > 1 && failedCount === 0) {
        detail = `Observed ${runCount} similar runs completing with a repeatable path.`;
        rationale =
          "Repeated success is strong enough to keep the current routing and tool-selection choice under watch.";
      } else if (completedCount > 0 && failedCount > 0) {
        detail = `Observed ${runCount} related runs with mixed outcomes. The latest path completed, but earlier attempts show the routing still needs refinement.`;
        rationale =
          "ArkEvolve can use the failures to narrow when the alternate path should be avoided.";
      } else if (failedCount > 0) {
        detail = `Observed ${runCount} failed run${runCount === 1 ? "" : "s"}${dominantBlocker ? `, mostly blocked by ${dominantBlocker}` : ""}.`;
        rationale =
          "This is evidence for a guardrail or tighter trigger before retrying the same path.";
      } else if (completedCount === 1) {
        detail =
          "Observed one completed run. ArkEvolve usually waits for repetition before treating it as a stable lesson.";
        rationale =
          "A single success is useful context, but not enough to claim that behavior improved.";
      }

      const chips = [`${runCount} run${runCount === 1 ? "" : "s"}`];
      if (completedCount > 0) chips.push(`${completedCount} completed`);
      if (failedCount > 0) chips.push(`${failedCount} failed`);
      if (acceptedCount > 0)
        chips.push(
          `${acceptedCount} correction${acceptedCount === 1 ? "" : "s"}`,
        );

      const evidence: string[] = [];
      const toolSummary = summarizeLearningEvidenceTools(allToolNames);
      if (toolSummary) evidence.push(`Tools used: ${toolSummary}`);
      if (failedCount > 0 && dominantBlocker)
        evidence.push(`Main blocker: ${dominantBlocker}`);

      return {
        key: `${str(latestRun.id, "run")}-${idx}`,
        title,
        status,
        detail,
        chips,
        rationale,
        example: requestPreview
          ? truncateUiText(requestPreview, 120)
          : undefined,
        evidence: evidence.join(" | "),
        runs: groupRuns,
        latestSeen: latestSeen !== "-" ? latestSeen : undefined,
        toolSummary,
        completedCount,
        failedCount,
        acceptedCount,
      };
    })
    .filter((card): card is EvolutionPatternCard => card !== null)
    .slice(0, 4);
}

export function skillEvolutionChipColor(
  status: string,
): "default" | "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved" || normalized === "improved") return "success";
  if (
    normalized === "draft" ||
    normalized === "pending" ||
    normalized === "inconclusive"
  )
    return "warning";
  if (normalized === "regressed" || normalized === "rejected") return "error";
  if (normalized === "unchanged") return "info";
  return "default";
}

export function skillEvolutionAlertSeverity(
  status: string,
): "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved" || normalized === "improved") return "success";
  if (normalized === "regressed" || normalized === "rejected") return "error";
  if (normalized === "unchanged") return "info";
  return "warning";
}

export function skillEvolutionActionLabel(action: string): string {
  const normalized = action.trim().toLowerCase();
  if (normalized === "create_skill") return "Create skill";
  if (normalized === "optimize_description") return "Tune trigger";
  if (normalized === "improve_skill") return "Improve skill";
  return action || "Skill change";
}

export function canonicalSkillIdentifier(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const compact = trimmed.toLowerCase().replace(/[^a-z0-9]+/g, "");
  if (compact === "trendprophet") return "trend-prophet";
  return trimmed;
}

export function skillEvolutionMetricRows(
  row: JsonRecord,
): Array<{
  label: string;
  before: string;
  after: string;
  delta: string;
  positive: boolean | null;
}> {
  const baseline = asRecord(row.impact_baseline);
  const observed = asRecord(row.impact_observed);
  const successDelta =
    num(observed.success_rate, 0) - num(baseline.success_rate, 0);
  const failureDelta =
    num(baseline.failure_rate, 0) - num(observed.failure_rate, 0);
  const toolErrorDelta =
    num(baseline.tool_error_rate, 0) - num(observed.tool_error_rate, 0);
  return [
    {
      label: "Success",
      before: percentageLabel(baseline.success_rate, 1) || "-",
      after: percentageLabel(observed.success_rate, 1) || "-",
      delta: evolutionGainLabel(successDelta),
      positive: Number.isFinite(successDelta) ? successDelta >= 0 : null,
    },
    {
      label: "Failure",
      before: percentageLabel(baseline.failure_rate, 1) || "-",
      after: percentageLabel(observed.failure_rate, 1) || "-",
      delta: evolutionGainLabel(failureDelta),
      positive: Number.isFinite(failureDelta) ? failureDelta >= 0 : null,
    },
    {
      label: "Tool errors",
      before: percentageLabel(baseline.tool_error_rate, 1) || "-",
      after: percentageLabel(observed.tool_error_rate, 1) || "-",
      delta: evolutionGainLabel(toolErrorDelta),
      positive: Number.isFinite(toolErrorDelta) ? toolErrorDelta >= 0 : null,
    },
  ];
}

export function evolutionSurfaceAudienceLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy") return "Reply routing";
  if (normalized === "main prompt bundle") return "Main replies";
  if (normalized === "request classifier") return "Request understanding";
  if (normalized === "specialist prompts") return "Specialist helpers";
  return value || "Experiment";
}

export function evolutionSurfaceSummary(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Tests how often the assistant should answer directly versus handing work off.";
  if (normalized === "main prompt bundle")
    return "Tests a different set of reply instructions for the main assistant response.";
  if (normalized === "request classifier")
    return "Tests a different way to classify incoming requests before choosing a path.";
  if (normalized === "specialist prompts")
    return "Tests different instructions for helper specialists used during delegated work.";
  return "Tests a candidate behavior against the current stable setup.";
}

export function evolutionSurfaceBenefit(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Can reduce unnecessary handoffs and keep simple requests faster.";
  if (normalized === "main prompt bundle")
    return "Can make replies clearer, more reliable, or less repetitive.";
  if (normalized === "request classifier")
    return "Can improve how quickly the assistant recognizes the right handling path.";
  if (normalized === "specialist prompts")
    return "Can make delegated specialist work more accurate and consistent.";
  return "Can improve how future requests are handled if the candidate keeps performing well.";
}

export function evolutionSurfaceStableSummary(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Reply routing is using the current stable logic.";
  if (normalized === "main prompt bundle")
    return "Main replies are using the current stable prompt bundle.";
  if (normalized === "request classifier")
    return "Request understanding is using the current stable classifier.";
  if (normalized === "specialist prompts")
    return "Specialist helpers are using the current stable prompt bundle.";
  return "This area is currently on the stable baseline.";
}

export function evolutionExperimentStatusText(item: {
  gate: string;
  last: string;
  enabled: boolean;
}): string {
  const gate = str(item.gate, "").trim();
  const last = str(item.last, "").trim();
  if (!item.enabled) return "No active experiment is running here.";
  if (gate && gate !== "-") return `Current gate signal: ${gate}.`;
  if (last && !/^no .* runs yet$/i.test(last)) return last;
  return "This experiment is running against the current stable behavior.";
}

export function promptProposalScopeLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "prompt_profile") return "Main replies";
  if (normalized === "classifier_prompt_profile") return "Request understanding";
  if (normalized === "specialist_prompt_profile") return "Specialist helpers";
  return humanizeStatusLabel(value || "prompt profile");
}

export function promptCanaryActionSummary(row: JsonRecord): string {
  const baselineSuccessRate = num(row.baseline_success_rate, 0) * 100;
  const candidateSuccessRate = num(row.candidate_success_rate, 0) * 100;
  const baselineSamples = num(row.baseline_samples, 0);
  const candidateSamples = num(row.candidate_samples, 0);
  return `Stable behavior is at ${baselineSuccessRate.toFixed(1)}% over ${baselineSamples.toLocaleString()} recent runs. The experiment is at ${candidateSuccessRate.toFixed(1)}% over ${candidateSamples.toLocaleString()} runs.`;
}

export type EvolutionReviewEvidence = {
  metrics: Array<{ label: string; value: string }>;
  current: string[];
  proposed: string[];
  impact: string[];
};

export function formatSignedPoints(value: number): string {
  if (!Number.isFinite(value)) return "-";
  return `${value >= 0 ? "+" : ""}${value.toFixed(1)} pts`;
}

export function cleanEvidenceLines(lines: unknown[], limit = 3): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of lines) {
    const line = str(raw, "").trim();
    if (!line || seen.has(line)) continue;
    seen.add(line);
    out.push(line);
    if (out.length >= limit) break;
  }
  return out;
}

export function EvolutionReviewEvidenceStrip({
  evidence,
}: {
  evidence: EvolutionReviewEvidence;
}): JSX.Element | null {
  const metrics = evidence.metrics
    .filter((item) => str(item.label, "").trim() && str(item.value, "").trim())
    .slice(0, 4);
  const sections = [
    { label: "Current", lines: cleanEvidenceLines(evidence.current, 3) },
    { label: "Proposed", lines: cleanEvidenceLines(evidence.proposed, 3) },
    { label: "Expected effect", lines: cleanEvidenceLines(evidence.impact, 3) },
  ].filter((section) => section.lines.length > 0);
  if (metrics.length === 0 && sections.length === 0) return null;
  return (
    <Box
      sx={{
        mt: 1,
        pt: 1,
        borderTop: "1px solid var(--ui-rgba-145-170-205-120)",
      }}
    >
      {metrics.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: {
              xs: "1fr 1fr",
              md: "repeat(4, minmax(0,1fr))",
            },
            gap: 1,
            mb: sections.length > 0 ? 1 : 0,
          }}
        >
          {metrics.map((item) => (
            <Box key={`${item.label}-${item.value}`} sx={{ minWidth: 0 }}>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                {item.label}
              </Typography>
              <Typography variant="body2">{item.value}</Typography>
            </Box>
          ))}
        </Box>
      ) : null}
      {sections.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: {
              xs: "1fr",
              md: "repeat(auto-fit, minmax(0,1fr))",
            },
            gap: 1,
          }}
        >
          {sections.map((section) => (
            <Box key={section.label} sx={{ minWidth: 0 }}>
              <Typography
                variant="caption"
                sx={{ color: "text.secondary", display: "block", mb: 0.35 }}
              >
                {section.label}
              </Typography>
              <Stack spacing={0.35}>
                {section.lines.map((line, idx) => (
                  <Typography
                    key={`${section.label}-${idx}`}
                    variant="caption"
                    sx={{ color: "#d8edff", display: "block" }}
                  >
                    - {line}
                  </Typography>
                ))}
              </Stack>
            </Box>
          ))}
        </Box>
      ) : null}
    </Box>
  );
}

export function promptCanaryReviewEvidence(row: JsonRecord): EvolutionReviewEvidence {
  const baselineSuccessRate = num(row.baseline_success_rate, 0) * 100;
  const candidateSuccessRate = num(row.candidate_success_rate, 0) * 100;
  const baselineSamples = num(row.baseline_samples, 0);
  const candidateSamples = num(row.candidate_samples, 0);
  const successDelta = candidateSuccessRate - baselineSuccessRate;
  return {
    metrics: [
      { label: "Stable", value: `${baselineSuccessRate.toFixed(1)}%` },
      { label: "Experiment", value: `${candidateSuccessRate.toFixed(1)}%` },
      { label: "Gap", value: formatSignedPoints(successDelta) },
      {
        label: "Samples",
        value: `${baselineSamples.toLocaleString()} / ${candidateSamples.toLocaleString()}`,
      },
    ],
    current: [
      `Stable version ${str(row.baseline_version, "-")} is carrying ${baselineSamples.toLocaleString()} recent runs.`,
    ],
    proposed: [
      `Experiment version ${str(row.candidate_version, "-")} is still active on ${candidateSamples.toLocaleString()} runs.`,
    ],
    impact: [
      successDelta < 0
        ? `The experiment is currently down ${Math.abs(successDelta).toFixed(1)} points versus stable.`
        : `The experiment is currently up ${successDelta.toFixed(1)} points versus stable.`,
      `Wins versus losses: ${num(row.wins, 0).toLocaleString()} / ${num(row.losses, 0).toLocaleString()}.`,
      num(row.regression_p_value, Number.NaN) >= 0
        ? `Regression check p-value: ${num(row.regression_p_value, 0).toFixed(4)}.`
        : "",
    ],
  };
}

export function promptOptimizationReviewEvidence(
  row: JsonRecord,
): EvolutionReviewEvidence {
  const preview = asRecord(row.change_preview);
  const current = stringList(preview.before);
  const proposed = stringList(preview.after);
  const impact = stringList(preview.impact_estimate);
  const targetArea = promptProposalScopeLabel(str(row.target_scope, "prompt_profile"));
  const riskLevel = str(row.risk_level, "default");
  const reviewStatus = str(row.review_status, "open");
  return {
    metrics: [
      { label: "Area", value: targetArea },
      { label: "Risk", value: `${riskLevel || "unknown"} risk` },
      {
        label: "Decision",
        value:
          reviewStatus === "open"
            ? "Needs decision"
            : humanizeStatusLabel(reviewStatus),
      },
    ],
    current:
      current.length > 0
        ? current
        : stringList(row.evidence).slice(0, 2),
    proposed:
      proposed.length > 0
        ? proposed
        : stringList(row.expected_benefit).slice(0, 2),
    impact:
      impact.length > 0
        ? impact
        : [
            ...stringList(row.expected_benefit).slice(0, 2),
            ...stringList(row.caveats).slice(0, 1),
          ],
  };
}

export function skillReviewEvidence(row: JsonRecord): EvolutionReviewEvidence {
  const diffPreview = asRecord(row.diff_preview);
  const baseline = asRecord(row.impact_baseline);
  const evidence = asRecord(row.evidence);
  const added = stringList(diffPreview.added);
  const removed = stringList(diffPreview.removed);
  const failureReasons = stringList(evidence.recent_failure_reasons);
  const selectedExamples = stringList(evidence.selected_failure_examples);
  return {
    metrics: [
      {
        label: "Confidence",
        value: `${ratioPercent(row.confidence).toFixed(0)}%`,
      },
      {
        label: "Matched runs",
        value: num(baseline.matched_runs, 0).toLocaleString(),
      },
      {
        label: "Baseline success",
        value: percentageLabel(baseline.success_rate, 1) || "-",
      },
      {
        label: "Baseline failure",
        value: percentageLabel(baseline.failure_rate, 1) || "-",
      },
    ],
    current:
      removed.length > 0
        ? removed.slice(0, 2).map((line) => `Replace: ${line}`)
        : failureReasons.slice(0, 2).map((line) => `Current failure: ${line}`),
    proposed:
      added.length > 0
        ? added.slice(0, 2)
        : cleanEvidenceLines([
            str(row.diff_summary, ""),
            str(row.summary, ""),
          ]),
    impact: cleanEvidenceLines([
      `Targets ${num(baseline.matched_runs, 0).toLocaleString()} matched runs with ${percentageLabel(baseline.failure_rate, 1) || "-"} failure and ${percentageLabel(baseline.tool_error_rate, 1) || "-"} tool errors.`,
      ...failureReasons.slice(0, 1).map((line) => `Focus area: ${line}`),
      ...selectedExamples.slice(0, 1).map((line) => `Recent mismatch: ${line}`),
    ]),
  };
}

export function learningCandidateReviewEvidence(
  row: JsonRecord,
  context: {
    strategyBaselineVersion: string;
    patternById: Map<string, JsonRecord>;
    itemById: Map<string, JsonRecord>;
  },
): EvolutionReviewEvidence {
  const type = str(row.candidate_type, "");
  const preview = asRecord(row.proposed_content_preview);
  const confidence = `${ratioPercent(row.confidence).toFixed(0)}%`;
  if (type === "strategy") {
    const pattern = context.patternById.get(str(row.pattern_id, ""));
    const defaultGuidance = stringList(preview.default_guidance);
    const taskGuidance = stringList(preview.task_guidance);
    const toolSequence = stringList(asRecord(pattern).tool_sequence);
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        {
          label: "Pattern runs",
          value: num(asRecord(pattern).sample_count, 0).toLocaleString(),
        },
        {
          label: "Pattern success",
          value: percentageLabel(asRecord(pattern).success_rate, 1) || "-",
        },
        {
          label: "Candidate",
          value: str(preview.strategy_version, "-"),
        },
      ],
      current: cleanEvidenceLines([
        context.strategyBaselineVersion
          ? `Matching requests still use stable strategy ${context.strategyBaselineVersion}.`
          : "Matching requests still use the current stable strategy.",
        pattern
          ? `Observed pattern ${str(pattern.title, "pattern")} succeeded on ${num(asRecord(pattern).sample_count, 0).toLocaleString()} runs.`
          : "",
      ]),
      proposed: cleanEvidenceLines([
        preview.strategy_version
          ? `Approve strategy version ${str(preview.strategy_version, "-")}.`
          : "",
        ...defaultGuidance.slice(0, 1),
        ...taskGuidance.slice(0, 2),
      ]),
      impact: cleanEvidenceLines([
        toolSequence.length > 0
          ? `Would steer matching work toward ${toolSequence.join(" -> ")}.`
          : "",
        `Confidence ${confidence} from repeated pattern evidence.`,
      ]),
    };
  }
  if (
    type === "memory_add" ||
    type === "memory_update" ||
    type === "memory_retract"
  ) {
    const operationType = str(preview.operation_type, type);
    const semanticKey = str(preview.semantic_key, str(row.subject_key, "memory"));
    const valuePreview = str(preview.value_preview, "");
    const scope = humanizeStatusLabel(str(preview.scope, "global"));
    const durability = humanizeStatusLabel(str(preview.durability, ""));
    const sensitive = toBool(preview.looks_sensitive);
    const sensitiveReason = str(preview.sensitive_reason, "");
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        { label: "Kind", value: humanizeStatusLabel(str(preview.memory_kind, "memory")) },
        { label: "Scope", value: scope },
        {
          label: "Duration",
          value: durability || "-",
        },
      ],
      current: [
        operationType === "memory_add"
          ? `Future turns do not yet store ${semanticKey} as reusable memory.`
          : operationType === "memory_update"
            ? `Future turns still use the current saved value for ${semanticKey}.`
            : `Future turns still treat ${semanticKey} as active memory.`,
      ],
      proposed: cleanEvidenceLines([
        operationType === "memory_retract"
          ? `Retract saved memory ${semanticKey}.`
          : valuePreview
            ? `${humanizeStatusLabel(operationType)} ${semanticKey}: ${valuePreview}`
            : `${humanizeStatusLabel(operationType)} ${semanticKey}.`,
        `Apply at ${scope}${durability ? ` with ${durability} duration` : ""}.`,
      ]),
      impact: cleanEvidenceLines([
        operationType === "memory_add"
          ? "Future replies can use this fact automatically."
          : operationType === "memory_update"
            ? "Future replies will rely on the updated value instead of the older one."
            : "Future replies will stop leaning on the retracted memory.",
        sensitive
          ? `Value stays masked because it looks sensitive${sensitiveReason ? `: ${sensitiveReason}` : "."}`
          : `Confidence ${confidence}.`,
      ]),
    };
  }
  if (type === "memory_deprecate") {
    const item = context.itemById.get(str(preview.item_id, ""));
    const nextStatus = humanizeStatusLabel(str(preview.next_status, "deprecated"));
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        {
          label: "Support",
          value: num(asRecord(item).support_count, 0).toLocaleString(),
        },
        {
          label: "Contradictions",
          value: num(asRecord(item).contradiction_count, 0).toLocaleString(),
        },
      ],
      current: cleanEvidenceLines([
        item
          ? `${str(item.title, "This learned item")} is still active.`
          : "This learned item is still active.",
        item
          ? `Support versus contradictions: ${num(asRecord(item).support_count, 0).toLocaleString()} / ${num(asRecord(item).contradiction_count, 0).toLocaleString()}.`
          : "",
      ]),
      proposed: [
        `Set this learned item to ${nextStatus}.`,
      ],
      impact: [
        "Stops stale guidance from influencing future turns.",
      ],
    };
  }
  if (type === "memory_merge") {
    const target = context.itemById.get(str(preview.target_item_id, ""));
    const source = context.itemById.get(str(preview.source_item_id, ""));
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        { label: "Target", value: str(asRecord(target).title, "Kept item") },
        { label: "Duplicate", value: str(asRecord(source).title, "Merged item") },
      ],
      current: cleanEvidenceLines([
        target && source
          ? `${str(asRecord(target).title, "Target item")} and ${str(asRecord(source).title, "source item")} are both still active.`
          : "Two overlapping memories are still active separately.",
      ]),
      proposed: cleanEvidenceLines([
        target
          ? `Keep ${str(asRecord(target).title, "the stronger memory")} as the surviving record.`
          : "Keep the stronger memory as the surviving record.",
        source
          ? `Deprecate duplicate memory ${str(asRecord(source).title, "the duplicate")}.`
          : "Deprecate the duplicate memory.",
      ]),
      impact: cleanEvidenceLines([
        "Reduces duplicate recall and conflicting memory retrieval.",
        str(preview.reason, "") === "duplicate_content"
          ? "The merge is based on overlapping content, not surface wording."
          : "",
      ]),
    };
  }
  return {
    metrics: [{ label: "Confidence", value: confidence }],
    current: cleanEvidenceLines([str(row.summary, ""), str(row.preview, "")]),
    proposed: cleanEvidenceLines([str(row.preview, ""), str(row.title, "")]),
    impact: cleanEvidenceLines([
      `Confidence ${confidence}.`,
      "ArkEvolve will measure impact after approval if this change goes live.",
    ]),
  };
}

export type EvolutionPageTab = "what" | "helped" | "tests" | "review";

export const EVOLUTION_PAGE_TABS: Array<{ value: EvolutionPageTab; label: string }> = [
  { value: "what", label: "Recent changes" },
  { value: "helped", label: "What improved" },
  { value: "tests", label: "Experiments" },
  { value: "review", label: "Needs approval" },
];

export function clampPercent(value: unknown): number {
  const parsed = num(value, 0);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(100, parsed));
}

export function ratioPercent(value: unknown): number {
  const parsed = num(value, 0);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(100, parsed * 100));
}

export function evolutionGainLabel(value: unknown): string {
  const parsed = num(value, Number.NaN);
  if (!Number.isFinite(parsed)) return "-";
  return `${parsed >= 0 ? "+" : ""}${(parsed * 100).toFixed(1)} pts`;
}

export function EvolutionStatStrip({
  items,
}: {
  items: Array<{
    label: string;
    value: ReactNode;
    helper: ReactNode;
    tone?: "default" | "good" | "warn" | "info";
  }>;
}) {
  return (
    <Box className="list-shell stat-strip">
      {items.map((item) => (
        <div
          key={String(item.label)}
          className="stat-strip-item"
          data-tone={item.tone !== "default" ? item.tone : undefined}
        >
          <span className="stat-strip-label">{item.label}</span>
          <span className="stat-strip-value">{item.value}</span>
          <span className="stat-strip-helper">{item.helper}</span>
        </div>
      ))}
    </Box>
  );
}

export function EvolutionRolloutBar({
  label,
  percent,
}: {
  label: string;
  percent: number;
}) {
  const pct = clampPercent(percent);
  return (
    <Box>
      <Stack
        direction="row"
        spacing={1}
        sx={{
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Typography variant="body2">{label}</Typography>
        <Typography
          variant="caption"
          sx={{
            color: "text.secondary",
          }}
        >
          {pct.toFixed(0)}%
        </Typography>
      </Stack>
      <Box
        role="meter"
        aria-label={`${label} rollout ${pct.toFixed(0)} percent`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        sx={{
          mt: 0.5,
          height: 8,
          borderRadius: 1,
          bgcolor: "var(--ui-rgba-145-170-205-140)",
          overflow: "hidden",
        }}
      >
        <Box
          sx={{
            width: `${pct}%`,
            height: "100%",
            borderRadius: 1,
            bgcolor: pct > 0 ? "#fbbf24" : "var(--ui-rgba-84-198-255-450)",
          }}
        />
      </Box>
    </Box>
  );
}

