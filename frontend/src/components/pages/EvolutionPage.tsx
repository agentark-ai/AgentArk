import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  MenuItem,
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
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
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
  promptProposalScopeLabel,
  promptOptimizationReviewEvidence,
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
  charsLabel,
  formatTimestampForHumans,
  promptCanarySafetyStatusColor,
  promptProposalRiskColor,
  promptProposalStatusColor,
} from "./settingsPageHelpers";

type ReadinessDialogState = {
  title: string;
  readiness: JsonRecord;
};

function readinessRecord(value: unknown): JsonRecord | null {
  const record = asRecord(value);
  return Object.keys(record).length > 0 ? record : null;
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

export default function EvolutionPage({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<EvolutionPageTab>("what");
  const [showSuperseded, setShowSuperseded] = useState(false);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(
    getDeveloperModeEnabled,
  );
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [selectedPatternCard, setSelectedPatternCard] =
    useState<EvolutionPatternCard | null>(null);
  const [showArkEvolveInternals, setShowArkEvolveInternals] = useState(false);
  const [technicalDialogProposalId, setTechnicalDialogProposalId] = useState<
    string | null
  >(null);
  const [readinessDialog, setReadinessDialog] =
    useState<ReadinessDialogState | null>(null);

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
  const classifierCanary = asRecord(evolution.classifier_prompt_canary);
  const specialistCanary = asRecord(evolution.specialist_prompt_canary);
  const learningQueue = asRecord(evolution.learning_queue);
  const promptInsights = asRecord(evolutionDev.prompt_insights);
  const classifierInsights = asRecord(evolutionDev.classifier_prompt_insights);
  const specialistInsights = asRecord(evolutionDev.specialist_prompt_insights);
  const strategyMetrics = pickRecords(evolutionDev, "strategy_metrics");
  const promptMetrics = pickRecords(evolutionDev, "prompt_metrics");
  const classifierMetrics = pickRecords(
    evolutionDev,
    "classifier_prompt_metrics",
  );
  const specialistMetrics = pickRecords(
    evolutionDev,
    "specialist_prompt_metrics",
  );
  const promptCanarySafetyEvents = pickRecords(
    evolutionDev,
    "prompt_canary_safety_events",
  );
  const promptTelemetrySummary = asRecord(
    evolutionDev.prompt_telemetry_summary,
  );
  const promptOptimizationOpportunities = pickRecords(
    evolutionDev,
    "prompt_optimization_opportunities",
  );
  const promptTelemetrySections = pickRecords(
    promptTelemetrySummary,
    "top_sections",
  );
  const learningCandidates = pickRecords(evolutionDev, "learning_candidates");
  const learningPatterns = pickRecords(evolutionDev, "learning_patterns");
  const learningItems = pickRecords(evolutionDev, "learning_items");
  const skillEvolutions = pickRecords(evolutionDev, "skill_evolutions");
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
    ...pickRecords(evolutionDev, "classifier_prompt_lineage_recent").map(
      (row): JsonRecord => ({
        ...row,
        surface: "Classifier",
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

  const tests = [
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
      last: str(
        evolution.last_promotion_result,
        "No routing-policy promotion yet",
      ),
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
      last: str(
        evolution.prompt_last_promotion_result,
        "No prompt promotion yet",
      ),
    },
    {
      key: "classifier",
      name: "Request classifier",
      audienceLabel: evolutionSurfaceAudienceLabel("Request classifier"),
      summary: evolutionSurfaceSummary("Request classifier"),
      benefit: evolutionSurfaceBenefit("Request classifier"),
      stableSummary: evolutionSurfaceStableSummary("Request classifier"),
      enabled: toBool(classifierCanary.enabled),
      rollout: clampPercent(classifierCanary.rollout_percent),
      baseline: str(classifierCanary.baseline_version, "-"),
      candidate: str(classifierCanary.candidate_version, "-"),
      gate: str(evolution.classifier_prompt_replay_gate_result, "-"),
      last: str(
        evolution.classifier_prompt_last_promotion_result,
        "No classifier promotion yet",
      ),
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
      last: str(
        evolution.specialist_prompt_last_promotion_result,
        "No specialist promotion yet",
      ),
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
    ...stringList(classifierInsights.summary),
    ...stringList(specialistInsights.summary),
  ];
  const metricRows: JsonRecord[] = [
    ...promptMetrics
      .slice(0, 5)
      .map((row): JsonRecord => ({ ...row, surface: "Prompt" })),
    ...strategyMetrics
      .slice(0, 5)
      .map((row): JsonRecord => ({ ...row, surface: "Routing" })),
    ...classifierMetrics
      .slice(0, 3)
      .map((row): JsonRecord => ({ ...row, surface: "Classifier" })),
    ...specialistMetrics
      .slice(0, 3)
      .map((row): JsonRecord => ({ ...row, surface: "Specialist" })),
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
      color: ["#5b7cfa", "#84cc6a", "#f4c35d", "#f87171", "#5fbad3"],
      tooltip: {
        backgroundColor: "var(--ui-rgba-6-14-28-950)",
        borderColor: "var(--ui-rgba-84-198-255-250)",
        textStyle: { color: "#d8edff", fontSize: 12 },
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
          textStyle: { color: "#9fb3c8", fontSize: 10 },
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
            color: "#eef6ff",
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
            itemStyle: { borderColor: "#e8f4ff", borderWidth: 1.4 },
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
  const openPromptOptimizationOpportunities = promptOptimizationOpportunities.filter(
    (row) => {
      const reviewStatus = str(row.review_status, "open").trim().toLowerCase();
      return reviewStatus !== "approved" && reviewStatus !== "rejected";
    },
  );
  const visiblePromptCanarySafetyEvents = showSuperseded
    ? promptCanarySafetyEvents
    : openPromptCanarySafetyEvents;
  const visiblePromptOptimizationOpportunities = showSuperseded
    ? promptOptimizationOpportunities
    : openPromptOptimizationOpportunities;
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
    openPromptOptimizationOpportunities.length;
  const promotedChangeCount =
    lineageRows.filter((row) => toBool(row.promoted)).length +
    skillHelpedItems.length;
  const metricChartOption = {
    backgroundColor: "transparent",
    animationDuration: 350,
    grid: { left: 48, right: 16, top: 36, bottom: 58 },
    legend: { top: 0, textStyle: { color: "#9fc3e6", fontSize: 11 } },
    tooltip: {
      trigger: "axis",
      backgroundColor: "var(--ui-rgba-6-14-28-950)",
      borderColor: "var(--ui-rgba-84-198-255-250)",
      textStyle: { color: "#d8edff" },
    },
    xAxis: {
      type: "category",
      data: metricChartRows.map(
        (row) =>
          `${str(row.surface, "-")} ${str(row.version, "").slice(0, 16)}`,
      ),
      axisLabel: { color: "#8fb2d1", fontSize: 10, rotate: 18 },
    },
    yAxis: {
      type: "value",
      max: 100,
      axisLabel: { color: "#8fb2d1", formatter: "{value}%" },
      splitLine: { lineStyle: { color: "var(--ui-rgba-108-156-212-100)" } },
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
      setSuccess(`${message}${evolutionTraceIdHint(result)}`);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const statusLoading = evolutionQ.isLoading;
  const detailLoading = evolutionDevQ.isLoading;
  const statusError = evolutionQ.error ? errMessage(evolutionQ.error) : "";
  const detailError = evolutionDevQ.error
    ? errMessage(evolutionDevQ.error)
    : "";
  const activeError = error || statusError;
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
  const selectedPatternClassifierPrompts = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.classifier_prompt_version),
  );
  const selectedPatternSpecialistPrompts = uniqueNonEmptyStrings(
    selectedPatternRuns.map((run) => run.specialist_prompt_version),
  );
  const selectedPatternVersionItems = [
    { label: "Policy", values: selectedPatternPolicies },
    { label: "Strategy", values: selectedPatternStrategies },
    { label: "Prompt", values: selectedPatternPrompts },
    { label: "Classifier prompt", values: selectedPatternClassifierPrompts },
    { label: "Specialist prompt", values: selectedPatternSpecialistPrompts },
  ].filter((item) => item.values.length > 1);

  return (
    <WorkspacePageShell className="evolution-page" spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Agent"
        title="ArkEvolve"
        descriptionNoWrap
        description="ArkEvolve learns from repeated runs, tests low-risk improvements, and asks before lasting changes become permanent."
        actions={
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{
              alignItems: "center",
              flexWrap: "wrap",
            }}
          >
            <Chip
              size="small"
              color={
                toBool(evolution.self_evolve_enabled) ? "success" : "default"
              }
              label={
                statusLoading
                  ? "Self-evolve loading"
                  : toBool(evolution.self_evolve_enabled)
                    ? "Self-evolve on"
                    : "Self-evolve off"
              }
            />
            <Chip
              size="small"
              color={activeTests > 0 ? "warning" : "default"}
              label={
                statusLoading
                  ? "Experiments loading"
                  : `${activeTests} active experiment${activeTests === 1 ? "" : "s"}`
              }
            />
            <FormControlLabel
              control={
                <Switch
                  size="small"
                  checked={showArkEvolveInternals}
                  onChange={(event) => {
                    const next = event.target.checked;
                    setShowArkEvolveInternals(next);
                    if (!next) setSelectedPatternCard(null);
                  }}
                />
              }
              label={
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  Show ArkEvolve internals
                </Typography>
              }
              sx={{ ml: 0.5, mr: 0 }}
            />
          </Stack>
        }
      />
      {success ? <Alert severity="success">{success}</Alert> : null}
      {activeError ? <Alert severity="error">{activeError}</Alert> : null}
      <EvolutionStatStrip
        items={[
          {
            label: "Improvement mode",
            value: toBool(evolution.self_evolve_enabled) ? "On" : "Off",
            helper: "Learns in the background and asks before lasting changes",
            tone: toBool(evolution.self_evolve_enabled) ? "good" : "default",
          },
          {
            label: "Experiments",
            value: activeTests,
            helper:
              activeTests > 0
                ? `${maxRollout.toFixed(0)}% of recent traffic in test`
                : "No active rollout",
            tone: activeTests > 0 ? "warn" : "info",
          },
          {
            label: "Needs approval",
            value: needsApprovalCount,
            helper:
              needsApprovalCount > 0
                ? "Meaningful changes are waiting on you"
                : "Nothing is waiting on you",
            tone: needsApprovalCount > 0 ? "warn" : "default",
          },
          {
            label: "Reusable lessons",
            value: num(learningQueue.active_patterns, 0),
            helper: showArkEvolveInternals
              ? `${reflectedHeuristics.length} internal heuristics captured`
              : "Shown after measurable impact",
            tone: "info",
          },
          {
            label: "Confirmed changes",
            value: promotedChangeCount,
            helper:
              promotedChangeCount > 0
                ? "Measured improvements that stuck"
                : "No permanent improvements yet",
            tone:
              promotedChangeCount > 0
                ? "good"
                : needsApprovalCount > 0
                  ? "warn"
                  : "default",
          },
        ]}
      />
      <Box className="list-shell" sx={{ p: 0.75 }}>
        <Tabs
          value={tab}
          onChange={(_, next) => setTab(next as EvolutionPageTab)}
          variant="scrollable"
          scrollButtons="auto"
          aria-label="ArkEvolve page sections"
          className="workspace-page-subnav-tabs"
        >
          {EVOLUTION_PAGE_TABS.map((item) => (
            <Tab key={item.value} value={item.value} label={item.label} />
          ))}
        </Tabs>
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
              Loading ArkEvolve status...
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
                Detailed ArkEvolve history is unavailable: {detailError}
              </Alert>
            ) : confirmedRecentChangeCount === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No confirmed improvements yet. ArkEvolve will list changes here
                only after they have proven measurable impact. In the meantime,
                the Needs approval tab shows changes that are waiting on you.
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
                      Skill improvements
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
                                str(row.summary, "Reviewable skill change"),
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

          {showArkEvolveInternals && evidenceCards.length > 0 ? (
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
                why ArkEvolve is still only watching the pattern.
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
                                    label={card.status}
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
          {selectedPatternCard ? <Chip size="small" color={learningEvidenceStatusColor(selectedPatternCard.status)} label={selectedPatternCard.status} /> : null}
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
        <Grid2 container spacing={1.5}>
          <Grid2 size={{ xs: 12, lg: 7 }}>
            <Box className="list-shell" sx={{ p: 1.6, minHeight: "100%" }}>
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
              ) : skillHelpedItems.length === 0 && helpedLines.length === 0 ? (
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Not enough measured evidence yet.
                </Typography>
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
                            label={str(row.impact_status, "improved")}
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
              <Stack spacing={0.7} sx={{ mt: 1.25 }}>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Prompt: delegation avoided{" "}
                  {num(promptInsights.delegation_avoided, 0).toFixed(1)},
                  clarification avoided{" "}
                  {num(promptInsights.clarification_avoided, 0).toFixed(1)},
                  tool success{" "}
                  {evolutionGainLabel(promptInsights.tool_success_uplift)}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Classifier: direct resolution{" "}
                  {evolutionGainLabel(
                    classifierInsights.successful_direct_resolution_uplift,
                  )}
                  , failed delegation reduction{" "}
                  {evolutionGainLabel(
                    classifierInsights.failed_delegation_reduction,
                  )}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Specialist: tool success{" "}
                  {evolutionGainLabel(specialistInsights.tool_success_uplift)},
                  p95 savings{" "}
                  {specialistInsights.latency_savings_p95_ms == null
                    ? "-"
                    : `${num(specialistInsights.latency_savings_p95_ms, 0)} ms`}
                </Typography>
              </Stack>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12, lg: 5 }}>
            <Stack spacing={1.5}>
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
                  Approved skill changes that have traffic, but have not cleared
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
                ) : skillObservedItems.length === 0 ? (
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    No approved skill changes are waiting on more evidence.
                  </Typography>
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
                              label={str(row.impact_status, "pending")}
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
              <Box className="list-shell" sx={{ p: 1.6, minHeight: "100%" }}>
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
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    No saved experience graph nodes yet.
                  </Typography>
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
                      style={{ height: 320 }}
                    />
                  </Stack>
                )}
              </Box>
              <Box className="list-shell" sx={{ p: 1.6, minHeight: "100%" }}>
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
                    mb: 1,
                  }}
                >
                  Success and error rates for the versions with recent traffic.
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
                      Loading optimization data...
                    </Typography>
                  </Stack>
                ) : detailError ? (
                  <Alert severity="warning" sx={{ borderRadius: 1 }}>
                    Optimization data is unavailable: {detailError}
                  </Alert>
                ) : metricChartRows.length === 0 ? (
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    No version metrics yet.
                  </Typography>
                ) : (
                  <ReactECharts
                    option={metricChartOption}
                    style={{ height: 320 }}
                  />
                )}
              </Box>
            </Stack>
          </Grid2>
        </Grid2>
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
                      ArkEvolve is using the current stable behavior across reply
                      routing, main replies, request understanding, and
                      specialist helpers.
                    </Typography>
                  </Box>
                  <Chip size="small" label="Stable" />
                </Stack>
                <Alert severity="info" sx={{ borderRadius: 1 }}>
                  When ArkEvolve starts testing a new improvement, this page will
                  explain what is changing, why it could help, how much traffic
                  is included, and what decision is still pending.
                </Alert>
              </Stack>
            </Box>
          ) : (
            <Grid2 container spacing={1.5}>
              {activeExperimentItems.map((item) => (
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
                        </Box>
                      </Box>
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
              ))}
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
          <Box className="list-shell" sx={{ p: 1.6 }}>
            <Stack
              direction={{ xs: "column", md: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", md: "center" },
                mb: 1,
              }}
            >
              <Box>
                <Typography
                  variant="h6"
                  sx={{ color: "#e8f4ff", fontWeight: 700 }}
                >
                  Needs approval
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  ArkEvolve keeps meaningful changes as suggestions until you
                  decide.
                </Typography>
              </Box>
              <FormControlLabel
                control={
                  <Switch
                    checked={showSuperseded}
                    onChange={(event) => setShowSuperseded(event.target.checked)}
                  />
                }
                label="Show older suggestions"
              />
            </Stack>
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
                Nothing is waiting on you right now.
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
                {visiblePromptOptimizationOpportunities.length > 0 ? (
                  <Box>
                    <Typography
                      variant="subtitle2"
                      sx={{ color: "#e8f4ff", mb: 1 }}
                    >
                      Suggested improvements
                    </Typography>
                    <Stack spacing={1}>
                      {visiblePromptOptimizationOpportunities.map((row, idx) => {
                        const proposalId = str(row.id, "");
                        const reviewStatus = str(row.review_status, "open");
                        const riskLevel = str(row.risk_level, "default");
                        const evidence = stringList(row.evidence);
                        const expectedBenefit = stringList(row.expected_benefit);
                        const caveats = stringList(row.caveats);
                        const reviewedAt = str(row.reviewed_at, "");
                        const reviewEvidence =
                          promptOptimizationReviewEvidence(row);
                        const canApprove =
                          !!proposalId &&
                          reviewStatus !== "approved" &&
                          reviewStatus !== "rejected";
                        return (
                          <Box
                            key={`prompt-proposal-${proposalId || idx}`}
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
                                    {str(row.title, "Suggested improvement")}
                                  </Typography>
                                  <Chip
                                    size="small"
                                    color={promptProposalStatusColor(reviewStatus)}
                                    label={
                                      reviewStatus === "open"
                                        ? "Needs decision"
                                        : humanizeStatusLabel(reviewStatus)
                                    }
                                  />
                                  <Chip
                                    size="small"
                                    variant="outlined"
                                    color={promptProposalRiskColor(riskLevel)}
                                    label={`${riskLevel || "unknown"} risk`}
                                  />
                                </Stack>
                                <Typography variant="body1">
                                  {str(
                                    row.summary,
                                    "ArkEvolve found a reviewable prompt improvement.",
                                  )}
                                </Typography>
                                {expectedBenefit[0] ? (
                                  <Typography
                                    variant="body2"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      mt: 0.75,
                                    }}
                                  >
                                    Potential benefit: {expectedBenefit[0]}
                                  </Typography>
                                ) : null}
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                    display: "block",
                                    mt: expectedBenefit[0] ? 0.45 : 0.75,
                                  }}
                                >
                                  Review only: marking this worth pursuing does not change
                                  runtime prompt behavior.
                                </Typography>
                                {caveats[0] ? (
                                  <Typography
                                    variant="body2"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                      mt: 0.45,
                                    }}
                                  >
                                    Watch out for: {caveats[0]}
                                  </Typography>
                                ) : null}
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
                                        action:
                                          "approve_prompt_optimization_proposal",
                                        candidate_id: proposalId,
                                      },
                                      "Suggestion marked worth pursuing.",
                                    )
                                  }
                                >
                                  Worth pursuing
                                </Button>
                                <Button
                                  size="small"
                                  color="inherit"
                                  disabled={
                                    runEvolutionActionMutation.isPending || !canApprove
                                  }
                                  onClick={() =>
                                    void runEvolutionAction(
                                      {
                                        action:
                                          "reject_prompt_optimization_proposal",
                                        candidate_id: proposalId,
                                      },
                                      "Suggestion dismissed.",
                                    )
                                  }
                                >
                                  Dismiss
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
                                  setTechnicalDialogProposalId(proposalId)
                                }
                              >
                                See technical details
                              </Button>
                            </Box>
                            <Dialog
                              open={technicalDialogProposalId === proposalId}
                              onClose={() => setTechnicalDialogProposalId(null)}
                              maxWidth="md"
                              fullWidth
                            >
                              <DialogTitle>Technical details</DialogTitle>
                              <DialogContent>
                                <EvolutionReviewEvidenceStrip evidence={reviewEvidence} />
                                <Stack spacing={1.25} sx={{ mt: 2 }}>
                                  <Typography
                                    variant="body2"
                                    sx={{ color: "text.secondary" }}
                                  >
                                    Target area: {promptProposalScopeLabel(str(row.target_scope, "prompt_profile"))}
                                  </Typography>
                                  {evidence.length > 0 ? (
                                    <Box>
                                      <Typography
                                        variant="body2"
                                        sx={{
                                          color: "text.secondary",
                                          mb: 0.5,
                                          fontWeight: 600,
                                        }}
                                      >
                                        Evidence
                                      </Typography>
                                      <Stack spacing={0.5}>
                                        {evidence.map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-evidence-${lineIdx}`}
                                            variant="body2"
                                            sx={{ color: "text.secondary" }}
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Box>
                                  ) : null}
                                  {expectedBenefit.length > 1 ? (
                                    <Box>
                                      <Typography
                                        variant="body2"
                                        sx={{
                                          color: "text.secondary",
                                          mb: 0.5,
                                          fontWeight: 600,
                                        }}
                                      >
                                        More expected benefits
                                      </Typography>
                                      <Stack spacing={0.5}>
                                        {expectedBenefit.slice(1).map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-benefit-${lineIdx}`}
                                            variant="body2"
                                            sx={{ color: "text.secondary" }}
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Box>
                                  ) : null}
                                  {caveats.length > 1 ? (
                                    <Alert severity="warning" sx={{ borderRadius: 1 }}>
                                      <Stack spacing={0.5}>
                                        {caveats.slice(1).map((line, lineIdx) => (
                                          <Typography
                                            key={`${proposalId}-caveat-${lineIdx}`}
                                            variant="body2"
                                            sx={{ display: "block" }}
                                          >
                                            {line}
                                          </Typography>
                                        ))}
                                      </Stack>
                                    </Alert>
                                  ) : null}
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
                                    str(row.summary, "Reviewable skill change"),
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
                                              color: "#d8edff",
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
          {showArkEvolveInternals &&
          (num(promptTelemetrySummary.sample_count, 0) > 0 ||
            promptTelemetrySections.length > 0) ? (
            <Accordion disableGutters className="chat-workspace-section">
              <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                <Typography variant="body2">
                  Technical evidence from recent prompt traffic
                </Typography>
              </AccordionSummary>
              <AccordionDetails sx={{ pt: 0 }}>
                <Alert severity="info" sx={{ borderRadius: 1, mb: 1 }}>
                  These metrics explain why ArkEvolve generated some review
                  suggestions. They are technical evidence, not required reading
                  before you approve or reject.
                </Alert>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: {
                      xs: "1fr 1fr",
                      lg: "repeat(5, minmax(0,1fr))",
                    },
                    gap: 1,
                    mb: promptTelemetrySections.length > 0 ? 1 : 0,
                  }}
                >
                  <Box
                    sx={{
                      p: 1,
                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                      borderRadius: 1,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Samples
                    </Typography>
                    <Typography variant="body2">
                      {num(promptTelemetrySummary.sample_count, 0).toLocaleString()}
                    </Typography>
                  </Box>
                  <Box
                    sx={{
                      p: 1,
                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                      borderRadius: 1,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      p95 final prompt
                    </Typography>
                    <Typography variant="body2">
                      {charsLabel(promptTelemetrySummary.p95_final_prompt_chars)}
                    </Typography>
                  </Box>
                  <Box
                    sx={{
                      p: 1,
                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                      borderRadius: 1,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      p95 tool schema
                    </Typography>
                    <Typography variant="body2">
                      {charsLabel(promptTelemetrySummary.p95_tool_schema_chars)}
                    </Typography>
                  </Box>
                  <Box
                    sx={{
                      p: 1,
                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                      borderRadius: 1,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      p95 request size
                    </Typography>
                    <Typography variant="body2">
                      {charsLabel(
                        promptTelemetrySummary.p95_estimated_total_request_chars,
                      )}
                    </Typography>
                  </Box>
                  <Box
                    sx={{
                      p: 1,
                      border: "1px solid var(--ui-rgba-145-170-205-120)",
                      borderRadius: 1,
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Avg tools
                    </Typography>
                    <Typography variant="body2">
                      {num(promptTelemetrySummary.avg_tool_count, -1) >= 0
                        ? num(promptTelemetrySummary.avg_tool_count, 0).toFixed(2)
                        : "-"}
                    </Typography>
                  </Box>
                </Box>
                {promptTelemetrySections.length > 0 ? (
                  <Box>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                        mb: 0.45,
                      }}
                    >
                      Largest prompt sections
                    </Typography>
                    <Stack spacing={0.45}>
                      {promptTelemetrySections.slice(0, 6).map((row, idx) => (
                        <Typography
                          key={`prompt-section-${str(row.section, idx.toString())}`}
                          variant="caption"
                          sx={{ color: "text.secondary", display: "block" }}
                        >
                          {str(row.section, "section")}: p95 {charsLabel(row.p95_chars)},
                          {" "}p50 {charsLabel(row.p50_chars)}, avg {charsLabel(row.avg_chars)}
                        </Typography>
                      ))}
                    </Stack>
                  </Box>
                ) : null}
              </AccordionDetails>
            </Accordion>
          ) : null}
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
                  "ArkEvolve is still collecting enough evidence."}
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
