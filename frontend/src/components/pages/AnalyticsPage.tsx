import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
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
  TextField,
  Tabs,
  Tooltip,
  Typography,
} from "@mui/material";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../../api/client";
import {
  formatUiDateOnly,
  formatUiDateRange,
  formatUiDateTime,
} from "../../lib/dateFormat";
import { resolveCssToken } from "../../lib/designTokens";
import { humanizeMachineLabel } from "../../lib/displayLabels";
import type {
  LlmAnalyticsBreakdownRow,
  LlmAnalyticsResponse,
} from "../../types";
import { MetricBarCard } from "../analytics/MetricBarCard";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { asRecord, errMessage, num, pickRecords, str } from "./pageHelpers";

const EVOLUTION_DEV_QUERY_LIMIT = 250;

type AnalyticsPageProps = {
  autoRefresh: boolean;
};

export default function AnalyticsPage({ autoRefresh }: AnalyticsPageProps) {
  type AnalyticsRange =
    | "1h"
    | "2h"
    | "6h"
    | "24h"
    | "3d"
    | "7d"
    | "14d"
    | "21d"
    | "30d"
    | "45d"
    | "60d"
    | "90d"
    | "custom";
  type BreakdownView = "model" | "channel" | "purpose";

  const RANGE_PRESETS: {
    value: AnalyticsRange;
    label: string;
    hours: number;
  }[] = [
    { value: "1h", label: "1 hour", hours: 1 },
    { value: "2h", label: "2 hours", hours: 2 },
    { value: "6h", label: "6 hours", hours: 6 },
    { value: "24h", label: "24 hours", hours: 24 },
    { value: "3d", label: "3 days", hours: 72 },
    { value: "7d", label: "7 days", hours: 168 },
    { value: "14d", label: "14 days", hours: 336 },
    { value: "21d", label: "21 days", hours: 504 },
    { value: "30d", label: "30 days", hours: 720 },
    { value: "45d", label: "45 days", hours: 1080 },
    { value: "60d", label: "60 days", hours: 1440 },
    { value: "90d", label: "90 days", hours: 2160 },
  ];

  function bucketForHours(hours: number): "hour" | "day" | "week" {
    if (hours <= 72) return "hour";
    if (hours <= 24 * 120) return "day";
    return "week";
  }

  function toLocalDatetimeInput(date: Date): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
  }

  function parseInputDate(value: string): Date | null {
    const t = Date.parse(value);
    return Number.isFinite(t) ? new Date(t) : null;
  }

  function compactNumber(value: number): string {
    if (!Number.isFinite(value)) return "0";
    if (Math.abs(value) >= 1_000_000)
      return `${(value / 1_000_000).toFixed(2)}M`;
    if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
    return value.toLocaleString();
  }

  function shortVersionLabel(value: string, max = 28): string {
    if (!value || value.length <= max) return value;
    const head = Math.max(10, Math.floor((max - 3) / 2));
    const tail = Math.max(8, max - head - 3);
    return `${value.slice(0, head)}...${value.slice(-tail)}`;
  }

  function formatAnalyticsBucketLabel(
    value: string,
    bucket: "hour" | "day" | "week",
  ): string {
    return bucket === "hour"
      ? formatUiDateTime(value, { fallback: value })
      : formatUiDateOnly(value, { fallback: value });
  }

  function formatPolicyVersionLabel(value: string): string {
    const cleaned = value
      .trim()
      .replace(/^routing:/i, "")
      .replace(/[_-]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();
    return cleaned || "policy";
  }

  function formatPolicyVersionTickLabel(value: string): string {
    const short = shortVersionLabel(formatPolicyVersionLabel(value), 22);
    const words = short.split(/\s+/);
    if (words.length <= 2) return short;
    return `${words.slice(0, 2).join(" ")}\n${words.slice(2).join(" ")}`;
  }

  function formatUsd(value: number | null | undefined, digits?: number): string {
    if (typeof value !== "number" || !Number.isFinite(value)) return "n/a";
    const resolvedDigits =
      typeof digits === "number" ? digits : Math.abs(value) >= 1 ? 2 : 4;
    return `$${value.toFixed(resolvedDigits)}`;
  }

  function formatBreakdownLabel(
    row: LlmAnalyticsBreakdownRow,
    view: BreakdownView,
  ): string {
    if (view === "model") {
      const provider = str(row.provider, "");
      const model = str(row.model, "");
      return [provider, model].filter(Boolean).join(" / ") || "Unknown model";
    }
    if (view === "channel")
      return humanizeMachineLabel(str(row.channel, ""), "Unknown channel");
    return humanizeMachineLabel(str(row.purpose, ""), "Unknown purpose");
  }

  const [activeRange, setActiveRange] = useState<AnalyticsRange>("24h");
  const [breakdownView, setBreakdownView] = useState<BreakdownView>("model");
  const [customDialogOpen, setCustomDialogOpen] = useState(false);
  const defaultCustomTo = useMemo(() => toLocalDatetimeInput(new Date()), []);
  const defaultCustomFrom = useMemo(
    () => toLocalDatetimeInput(new Date(Date.now() - 30 * 24 * 60 * 60 * 1000)),
    [],
  );
  const [customFrom, setCustomFrom] = useState(defaultCustomFrom);
  const [customTo, setCustomTo] = useState(defaultCustomTo);
  const [appliedCustomFrom, setAppliedCustomFrom] = useState(defaultCustomFrom);
  const [appliedCustomTo, setAppliedCustomTo] = useState(defaultCustomTo);

  const customFromDate = useMemo(
    () => parseInputDate(customFrom),
    [customFrom],
  );
  const customToDate = useMemo(() => parseInputDate(customTo), [customTo]);
  const appliedFromDate = useMemo(
    () => parseInputDate(appliedCustomFrom),
    [appliedCustomFrom],
  );
  const appliedToDate = useMemo(
    () => parseInputDate(appliedCustomTo),
    [appliedCustomTo],
  );
  const customRangeInvalid =
    !customFromDate ||
    !customToDate ||
    customFromDate.getTime() >= customToDate.getTime();

  // Compute effective from/to ISO strings and bucket for the active range
  const { effectiveFrom, effectiveTo, effectiveBucket } = useMemo(() => {
    if (activeRange === "custom") {
      const from = appliedFromDate?.toISOString() ?? "";
      const to = appliedToDate?.toISOString() ?? "";
      const diffMs =
        (appliedToDate?.getTime() ?? 0) - (appliedFromDate?.getTime() ?? 0);
      const diffHours = diffMs / (1000 * 60 * 60);
      return {
        effectiveFrom: from,
        effectiveTo: to,
        effectiveBucket: bucketForHours(diffHours),
      };
    }
    const preset = RANGE_PRESETS.find((p) => p.value === activeRange);
    const hours = preset?.hours ?? 24;
    const now = new Date();
    const from = new Date(now.getTime() - hours * 60 * 60 * 1000);
    return {
      effectiveFrom: from.toISOString(),
      effectiveTo: now.toISOString(),
      effectiveBucket: bucketForHours(hours),
    };
  }, [activeRange, appliedFromDate, appliedToDate]);

  const analyticsQ = useQuery({
    queryKey: [
      "llm-analytics",
      activeRange,
      effectiveFrom,
      effectiveTo,
      effectiveBucket,
    ],
    queryFn: () =>
      api.getLlmAnalytics({
        range: activeRange === "custom" ? "custom" : activeRange,
        bucket: effectiveBucket,
        from: effectiveFrom || undefined,
        to: effectiveTo || undefined,
      }),
    enabled:
      activeRange !== "custom" || Boolean(appliedFromDate && appliedToDate),
    refetchInterval: autoRefresh
      ? effectiveBucket === "hour"
        ? 30000
        : 120000
      : false,
  });

  const handleRangeChange = (range: AnalyticsRange) => {
    if (range === "custom") {
      setCustomFrom(defaultCustomFrom);
      setCustomTo(toLocalDatetimeInput(new Date()));
      setCustomDialogOpen(true);
      return;
    }
    setActiveRange(range);
  };

  const applyCustomRange = () => {
    if (customRangeInvalid) return;
    setAppliedCustomFrom(customFrom);
    setAppliedCustomTo(customTo);
    setActiveRange("custom");
    setCustomDialogOpen(false);
  };
  const policyMetricsQ = useQuery({
    queryKey: ["analytics-policy-metrics"],
    queryFn: () =>
      api.rawGet(`/settings/evolution/dev?limit=${EVOLUTION_DEV_QUERY_LIMIT}`),
    refetchInterval: autoRefresh ? 120000 : false,
  });

  const resp = analyticsQ.data as LlmAnalyticsResponse | undefined;
  const activeError = analyticsQ.error;
  const totals = resp?.totals;
  const policyMetricsPayload = asRecord(policyMetricsQ.data);
  const policyMetricsRows = pickRecords(policyMetricsPayload, "policy_metrics")
    .slice()
    .sort((a, b) => num(b.samples, 0) - num(a.samples, 0))
    .slice(0, 8);
  const byModelRows = (resp?.by_model || []).slice(0, 4);
  const breakdownRows =
    breakdownView === "model"
      ? resp?.by_model || []
      : breakdownView === "channel"
        ? resp?.by_channel || []
        : resp?.by_purpose || [];

  const palette = [
    "#d8ad78",
    "#14f195",
    "#fbbf24",
    "#d946ef",
    "#b7a7ff",
    "#f97316",
  ];
  const analyticsSeries = resp?.series || [];
  const analyticsBucketLabels = analyticsSeries.map((point) =>
    formatAnalyticsBucketLabel(point.bucket_start, effectiveBucket),
  );
  const analyticsRangeLabel = formatUiDateRange(
    str(asRecord(resp?.range).since, ""),
    str(asRecord(resp?.range).until, ""),
    "-",
  );
  const analyticsTruncated =
    Boolean(resp?.truncated) || Boolean(asRecord(resp?.range).truncated);
  const totalPolicySamples = policyMetricsRows.reduce(
    (sum, row) => sum + num(row.samples, 0),
    0,
  );
  const weightedPolicySuccessRate = totalPolicySamples
    ? policyMetricsRows.reduce(
        (sum, row) => sum + num(row.samples, 0) * num(row.success_rate, 0),
        0,
      ) / totalPolicySamples
    : 0;
  const weightedPolicyErrorRate = totalPolicySamples
    ? policyMetricsRows.reduce(
        (sum, row) => sum + num(row.samples, 0) * num(row.error_rate, 0),
        0,
      ) / totalPolicySamples
    : 0;
  const policyLatencyValues = policyMetricsRows
    .map((row) =>
      row.p95_latency_ms == null ? null : num(row.p95_latency_ms, 0),
    )
    .filter((value): value is number => value != null);
  const slowestPolicyLatency =
    policyLatencyValues.length > 0 ? Math.max(...policyLatencyValues) : null;
  const leadingPolicy = policyMetricsRows[0] || null;
  const leadingPolicyLabel = leadingPolicy
    ? formatPolicyVersionLabel(str(leadingPolicy.version, "policy"))
    : "No policy yet";
  const policyChartLabels = policyMetricsRows.map((row) =>
    formatPolicyVersionTickLabel(str(row.version, "policy")),
  );
  const policyLatencyCeiling = slowestPolicyLatency
    ? Math.max(100, Math.ceil((slowestPolicyLatency * 1.15) / 100) * 100)
    : 100;
  const chartTokens = useMemo(
    () => ({
      tooltipBg: resolveCssToken("--cyber-panel"),
      tooltipBorder: "rgba(120, 242, 176, 0.24)",
      axisLine: "rgba(130, 170, 160, 0.28)",
      splitLine: "rgba(130, 170, 160, 0.14)",
      axisLabel: "#c8d8c9",
      tooltipText: "#fff8ed",
      legendText: "#d8d0c4",
      zoomBorder: "rgba(130, 170, 160, 0.22)",
      zoomFill: "rgba(120, 242, 176, 0.14)",
    }),
    [],
  );

  const spendValue = formatUsd(totals?.cost_usd);
  const requestsValue = compactNumber(num(totals?.request_count, 0));
  const tokensValue = compactNumber(num(totals?.total_tokens, 0));
  const arkDistillTotals = resp?.arkdistill?.totals;
  const arkDistillSeries = resp?.arkdistill?.series || [];
  const arkDistillSavedTokens = num(
    arkDistillTotals?.estimated_saved_tokens,
    0,
  );
  const arkDistillSavedCost = arkDistillTotals?.estimated_prompt_cost_saved_usd;
  const arkDistillSavedCostLabel =
    typeof arkDistillSavedCost === "number"
      ? formatUsd(arkDistillSavedCost, arkDistillSavedCost >= 1 ? 2 : 4)
      : "pricing n/a";
  const arkDistillReductionRatio = num(
    arkDistillTotals?.average_reduction_ratio,
    0,
  );
  const arkDistillSavingsPercent =
    typeof arkDistillTotals?.savings_percent === "number"
      ? arkDistillTotals.savings_percent
      : arkDistillReductionRatio * 100;
  const spendBucketSeries = analyticsSeries.map((point) => point.cost_usd ?? 0);
  const requestBucketSeries = analyticsSeries.map((point) =>
    num(point.request_count, 0),
  );
  const totalTokenBucketSeries = analyticsSeries.map((point) =>
    num(point.total_tokens, 0),
  );
  const arkDistillSavedTokenBucketSeries = arkDistillSeries.map((point) =>
    num(point.estimated_saved_tokens, 0),
  );
  const cachedPromptTokensTotal = num(totals?.cached_prompt_tokens, 0);
  const cacheCreationPromptTokensTotal = num(
    totals?.cache_creation_prompt_tokens,
    0,
  );
  const promptTokensTotal = num(totals?.prompt_tokens, 0);
  const cachedPromptBucketSeries = analyticsSeries.map((point) =>
    num(point.cached_prompt_tokens, 0),
  );
  const cacheCreationPromptBucketSeries = analyticsSeries.map((point) =>
    num(point.cache_creation_prompt_tokens, 0),
  );
  const cacheReadShare =
    promptTokensTotal > 0 ? cachedPromptTokensTotal / promptTokensTotal : 0;
  const activeRangeLabel =
    activeRange === "custom"
      ? `Custom / ${formatUiDateRange(appliedCustomFrom, appliedCustomTo)}`
      : RANGE_PRESETS.find((preset) => preset.value === activeRange)?.label ??
        activeRange;
  const averageSpendPerBucket =
    spendBucketSeries.length > 0
      ? spendBucketSeries.reduce((sum, value) => sum + value, 0) /
        spendBucketSeries.length
      : 0;
  let peakSpendPoint: (typeof analyticsSeries)[number] | null = null;
  let peakSpendAmount = 0;
  for (const point of analyticsSeries) {
    const amount = point.cost_usd ?? 0;
    if (peakSpendPoint == null || amount > peakSpendAmount) {
      peakSpendPoint = point;
      peakSpendAmount = amount;
    }
  }
  const peakSpendLabel = peakSpendPoint
    ? formatAnalyticsBucketLabel(peakSpendPoint.bucket_start, effectiveBucket)
    : "Waiting for spend data";
  const primaryTokensTotal = analyticsSeries.reduce(
    (sum, point) => sum + num(point.primary_total_tokens, 0),
    0,
  );
  const helperTokensTotal = analyticsSeries.reduce(
    (sum, point) => sum + num(point.helper_total_tokens, 0),
    0,
  );
  const helperTokenShare =
    num(totals?.total_tokens, 0) > 0
      ? helperTokensTotal / num(totals?.total_tokens, 0)
      : 0;
  const dominantModel = (resp?.by_model || [])[0] || null;
  const dominantModelLabel = dominantModel
    ? shortVersionLabel(formatBreakdownLabel(dominantModel, "model"), 34)
    : "No traffic yet";
  const dominantModelDetail = dominantModel
    ? `${compactNumber(num(dominantModel.request_count, 0))} requests / ${formatUsd(dominantModel.cost_usd, 4)}`
    : "Waiting for model activity.";
  const secondaryBreakdownView: BreakdownView =
    (resp?.by_channel || []).length > 0 ? "channel" : "purpose";
  const secondaryBreakdownTitle =
    secondaryBreakdownView === "channel" ? "Channel Mix" : "Purpose Mix";
  const secondaryBreakdownRows =
    secondaryBreakdownView === "channel"
      ? resp?.by_channel || []
      : resp?.by_purpose || [];
  const breakdownTitle =
    breakdownView === "model"
      ? "By Model"
      : breakdownView === "channel"
        ? "By Channel"
        : "By Purpose";
  const breakdownDescription =
    breakdownView === "model"
      ? "Provider and model usage across the selected range."
      : breakdownView === "channel"
        ? "Which AgentArk surfaces are generating the most LLM traffic."
        : "How traffic is split between response generation and helper passes.";
  const previewBreakdownRows = breakdownRows.slice(0, 24);
  const heroSummaryCards = [
    {
      label: "Peak bucket",
      value: formatUsd(peakSpendAmount),
      detail: peakSpendLabel,
    },
    {
      label: "Dominant model",
      value: dominantModelLabel,
      detail: dominantModelDetail,
    },
    {
      label: "Helper share",
      value: `${(helperTokenShare * 100).toFixed(1)}%`,
      detail: `${compactNumber(helperTokensTotal)} helper tokens in range`,
    },
    {
      label: "Prompt cache",
      value: `${compactNumber(cachedPromptTokensTotal)} read`,
      detail: `${compactNumber(cacheCreationPromptTokensTotal)} written / ${(
        cacheReadShare * 100
      ).toFixed(1)}% read share`,
    },
    {
      label: "ArkDistill saved",
      value: `${arkDistillSavingsPercent.toFixed(1)}%`,
      detail: `${compactNumber(arkDistillSavedTokens)} tokens / ${arkDistillSavedCostLabel}`,
    },
  ];
  const railCards = [
    {
      label: "Selected spend",
      value: spendValue,
      detail:
        peakSpendPoint == null
          ? "No spend recorded in this range yet."
          : `Peak ${formatUsd(peakSpendAmount)} on ${peakSpendLabel}`,
      values: spendBucketSeries,
      color: "#d8ad78",
      chartType: "bar" as const,
    },
    {
      label: "Total tokens",
      value: tokensValue,
      detail: `${compactNumber(primaryTokensTotal)} primary / ${compactNumber(helperTokensTotal)} helper`,
      values: totalTokenBucketSeries,
      color: "#14f195",
      chartType: "line" as const,
    },
    {
      label: "Prompt cache",
      value: `${compactNumber(cachedPromptTokensTotal)} read`,
      detail: `${compactNumber(cacheCreationPromptTokensTotal)} cache-write tokens`,
      values: cachedPromptBucketSeries,
      color: "#60a5fa",
      chartType: "line" as const,
    },
    {
      label: "ArkDistill savings",
      value: `${arkDistillSavingsPercent.toFixed(1)}%`,
      detail: `${compactNumber(arkDistillSavedTokens)} tokens saved / ${arkDistillSavedCostLabel}`,
      values: arkDistillSavedTokenBucketSeries,
      color: "#b7a7ff",
      chartType: "bar" as const,
    },
    {
      label: "Total requests",
      value: requestsValue,
      detail: `${analyticsSeries.length} bucket${analyticsSeries.length === 1 ? "" : "s"} in range`,
      values: requestBucketSeries,
      color: "#d8ad78",
      chartType: "bar" as const,
    },
  ];
  const modelMixMax = byModelRows.reduce(
    (max, row) => Math.max(max, num(row.request_count, 0)),
    1,
  );
  const spendTickStride = Math.max(
    1,
    Math.ceil(Math.max(analyticsBucketLabels.length, 1) / 6),
  );
  const analyticsNeedsDataZoom = analyticsBucketLabels.length > 16;
  const analyticsZoomStart = analyticsNeedsDataZoom
    ? Math.max(0, 100 - (16 / analyticsBucketLabels.length) * 100)
    : 0;
  const visibleAnalyticsBucketCount = analyticsNeedsDataZoom
    ? 16
    : Math.max(analyticsBucketLabels.length, 1);
  const spendChartBarMaxWidth =
    visibleAnalyticsBucketCount <= 1
      ? 44
      : visibleAnalyticsBucketCount <= 3
        ? 36
        : visibleAnalyticsBucketCount <= 8
          ? 28
          : 18;
  const sparklineBarMaxWidthForCount = (count: number) =>
    count <= 1 ? 10 : count <= 3 ? 9 : count <= 8 ? 7 : 5;
  const policySummaryCards = [
    {
      label: "Dominant policy",
      value: leadingPolicyLabel,
      detail: leadingPolicy
        ? `${compactNumber(num(leadingPolicy.samples, 0))} samples`
        : "Waiting for routing traffic.",
    },
    {
      label: "Weighted success",
      value: `${(weightedPolicySuccessRate * 100).toFixed(1)}%`,
      detail: `${(weightedPolicyErrorRate * 100).toFixed(1)}% error rate`,
    },
    {
      label: "Slowest p95",
      value:
        slowestPolicyLatency == null
          ? "-"
          : `${num(slowestPolicyLatency, 0).toLocaleString()}ms`,
      detail:
        slowestPolicyLatency == null
          ? "Latency pending"
          : `${policyMetricsRows.length} active version${policyMetricsRows.length === 1 ? "" : "s"}`,
    },
  ];
  const buildSparklineOption = (
    values: number[],
    color: string,
    chartType: "line" | "bar",
  ) => {
    const safeValues = values.length > 0 ? values : [0];
    return {
      backgroundColor: "transparent",
      animationDuration: 350,
      grid: { left: 0, right: 0, top: 4, bottom: 0 },
      tooltip: { show: false },
      xAxis: {
        type: "category",
        data: safeValues.map((_, index) => index),
        show: false,
      },
      yAxis: {
        type: "value",
        show: false,
      },
      series: [
        chartType === "bar"
          ? {
              type: "bar",
              data: safeValues,
              barWidth: "38%",
              barMaxWidth: sparklineBarMaxWidthForCount(safeValues.length),
              barMinWidth: 2,
              itemStyle: {
                color,
                borderRadius: [3, 3, 1, 1],
                opacity: 0.82,
              },
            }
          : {
              type: "line",
              data: safeValues,
              smooth: true,
              symbol: "none",
              lineStyle: { color, width: 2 },
              itemStyle: { color },
              areaStyle: {
                opacity: 0.18,
                color: {
                  type: "linear",
                  x: 0,
                  y: 0,
                  x2: 0,
                  y2: 1,
                  colorStops: [
                    { offset: 0, color: `${color}88` },
                    { offset: 1, color: `${color}00` },
                  ],
                },
              },
            },
      ],
    };
  };
  const policyMetricsOption = {
    backgroundColor: "transparent",
    animationDuration: 400,
    legend: {
      bottom: 0,
      textStyle: { color: "rgba(226, 218, 208, 0.76)" },
      itemWidth: 10,
      itemHeight: 10,
    },
    grid: { left: 48, right: 58, top: 18, bottom: 52 },
    tooltip: {
      trigger: "axis",
      backgroundColor: chartTokens.tooltipBg,
      borderColor: chartTokens.tooltipBorder,
      textStyle: { color: "#fff8ed" },
      formatter: (
        params: Array<{
          marker?: string;
          seriesName?: string;
          value?: number | string | null;
        }>,
      ) => {
        const rows = params
          .filter((item) => item.value !== null && item.value !== undefined)
          .map((item) => {
            const label = item.seriesName || "";
            const rawValue = Number(item.value);
            const value = label.toLowerCase().includes("latency")
              ? `${Number.isFinite(rawValue) ? rawValue.toLocaleString() : item.value}ms`
              : `${Number.isFinite(rawValue) ? rawValue.toFixed(1) : item.value}%`;
            return `${item.marker || ""}${label}: ${value}`;
          });
        return rows.join("<br/>");
      },
    },
    xAxis: {
      type: "category",
      data: policyChartLabels,
      axisLabel: { color: "rgba(226, 218, 208, 0.7)", fontSize: 10 },
      axisLine: {
        lineStyle: { color: chartTokens.axisLine },
      },
    },
    yAxis: [
      {
        type: "value",
        name: "success/error %",
        min: 0,
        max: 100,
        axisLabel: {
          color: "rgba(226, 218, 208, 0.7)",
          formatter: "{value}%",
        },
        nameTextStyle: { color: "rgba(226, 218, 208, 0.7)" },
        splitLine: {
          lineStyle: { color: chartTokens.splitLine },
        },
      },
      {
        type: "value",
        name: "p95 ms",
        min: 0,
        max: policyLatencyCeiling,
        axisLabel: {
          color: "rgba(226, 218, 208, 0.7)",
          formatter: "{value}",
        },
        nameTextStyle: { color: "rgba(226, 218, 208, 0.7)" },
        splitLine: { show: false },
      },
    ],
    series: [
      {
        name: "Success",
        type: "bar",
        data: policyMetricsRows.map((row) =>
          Number((num(row.success_rate, 0) * 100).toFixed(1)),
        ),
        itemStyle: {
          color: "#14f195",
          borderRadius: [4, 4, 0, 0],
        },
        barMaxWidth: 28,
      },
      {
        name: "Errors",
        type: "bar",
        data: policyMetricsRows.map((row) =>
          Number((num(row.error_rate, 0) * 100).toFixed(1)),
        ),
        itemStyle: {
          color: "#f87171",
          borderRadius: [4, 4, 0, 0],
        },
        barMaxWidth: 28,
      },
      {
        name: "p95 latency",
        type: "line",
        yAxisIndex: 1,
        data: policyMetricsRows.map((row) =>
          row.p95_latency_ms == null ? null : num(row.p95_latency_ms, 0),
        ),
        symbol: "circle",
        symbolSize: 9,
        lineStyle: { width: 2, color: "#f0b86a" },
        itemStyle: { color: "#f0b86a" },
      },
    ],
  };

  return (
    <WorkspacePageShell spacing={1.35}>
      <WorkspacePageHeader
        eyebrow="Data"
        title="Analytics"
        description="LLM usage, policy performance, and model mix across the selected range."
        actions={
          <TextField
            select
            className="workspace-page-select"
            size="small"
            value={activeRange}
            onChange={(e) => {
              const val = e.target.value as AnalyticsRange | "open_custom";
              if (val === "open_custom") {
                handleRangeChange("custom");
              } else {
                handleRangeChange(val);
              }
            }}
            sx={{ minWidth: 168, flexShrink: 0 }}
          >
            <MenuItem
              disabled
              sx={{ fontSize: "0.75rem", opacity: 0.6, py: 0.25 }}
            >
              Hours
            </MenuItem>
            <MenuItem value="1h">1 hour</MenuItem>
            <MenuItem value="2h">2 hours</MenuItem>
            <MenuItem value="6h">6 hours</MenuItem>
            <MenuItem value="24h">24 hours</MenuItem>
            <MenuItem
              disabled
              sx={{ fontSize: "0.75rem", opacity: 0.6, py: 0.25 }}
            >
              Days
            </MenuItem>
            <MenuItem value="3d">3 days</MenuItem>
            <MenuItem value="7d">7 days</MenuItem>
            <MenuItem value="14d">14 days</MenuItem>
            <MenuItem value="21d">21 days</MenuItem>
            <MenuItem value="30d">30 days</MenuItem>
            <MenuItem value="45d">45 days</MenuItem>
            <MenuItem value="60d">60 days</MenuItem>
            <MenuItem value="90d">90 days</MenuItem>
            <Divider />
            {activeRange === "custom" ? (
              <MenuItem value="custom">
                Custom ({formatUiDateRange(appliedCustomFrom, appliedCustomTo)})
              </MenuItem>
            ) : null}
            <MenuItem value={"open_custom" as string}>Custom range...</MenuItem>
          </TextField>
        }
      />
      {analyticsTruncated ? (
        <Alert severity="warning" variant="outlined">
          The selected range reached the server analytics row cap. Charts and
          totals show the available slice, not the complete range.
        </Alert>
      ) : null}
      <Dialog
        open={customDialogOpen}
        onClose={() => setCustomDialogOpen(false)}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>Custom Date Range</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ mt: 1 }}>
            <TextField
              size="small"
              label="From"
              type="datetime-local"
              value={customFrom}
              onChange={(e) => setCustomFrom(e.target.value)}
              fullWidth
              slotProps={{
                inputLabel: { shrink: true },
              }}
            />
            <TextField
              size="small"
              label="To"
              type="datetime-local"
              value={customTo}
              onChange={(e) => setCustomTo(e.target.value)}
              fullWidth
              error={customRangeInvalid}
              helperText={
                customRangeInvalid ? "To must be later than From." : undefined
              }
              slotProps={{
                inputLabel: { shrink: true },
              }}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCustomDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={applyCustomRange}
            disabled={customRangeInvalid}
          >
            Apply
          </Button>
        </DialogActions>
      </Dialog>
      {activeError ? (
        <Alert severity="error">{String(activeError)}</Alert>
      ) : null}

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "minmax(0, 1fr)",
            lg: "minmax(0, 1.62fr) minmax(320px, 0.78fr)",
          },
          gap: 1.5,
          alignItems: "start",
          minWidth: 0,
        }}
      >
          <Box
            className="list-shell"
            sx={{
              gridColumn: { lg: "1" },
              minWidth: 0,
              display: "flex",
              flexDirection: "column",
              gap: 1.1,
              p: { xs: 1.2, md: 1.55 },
              overflow: "hidden",
              background:
                "radial-gradient(circle at top left, var(--ui-rgba-15-240-179-070), transparent 30%), linear-gradient(180deg, var(--ui-rgba-15-15-18-940), var(--ui-rgba-12-18-28-960))",
            }}
          >
            <Stack
              direction={{ xs: "column", md: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", md: "flex-start" },
              }}
            >
              <Stack spacing={0.25}>
                <Typography
                  variant="overline"
                  sx={{ color: "text.secondary", letterSpacing: 0 }}
                >
                  Usage
                </Typography>
                <Typography variant="h4" sx={{ color: "#f6f0e8" }}>
                  {spendValue}
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Approximate spend across the selected analytics range.
                </Typography>
              </Stack>
              <Stack
                direction="row"
                spacing={0.75}
                sx={{
                  flexWrap: "wrap",
                  justifyContent: { xs: "flex-start", md: "flex-end" },
                }}
              >
                {[
                  `Range / ${activeRangeLabel}`,
                  `Bucket / ${effectiveBucket}`,
                  analyticsRangeLabel,
                ].map((pill) => (
                  <Box
                    key={pill}
                    sx={{
                      px: 1.1,
                      py: 0.7,
                      borderRadius: 999,
                      border: "1px solid rgba(130, 170, 160, 0.18)",
                      background:
                        "linear-gradient(180deg, var(--ui-rgba-22-22-26-920), var(--ui-rgba-15-15-18-880))",
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{ color: "#fff8ed", whiteSpace: "nowrap" }}
                    >
                      {pill}
                    </Typography>
                  </Box>
                ))}
              </Stack>
            </Stack>

            <Stack
              direction="row"
              spacing={0.45}
              sx={{ alignItems: "center", color: "text.secondary" }}
            >
              <Tooltip
                title="Actual provider pricing can differ from the amounts shown here. Treat spend and cost figures as rough estimates."
                arrow
                placement="top-start"
              >
                <IconButton
                  size="small"
                  aria-label="Analytics spend estimate details"
                  sx={{ p: 0.2, color: "text.secondary" }}
                >
                  <InfoOutlinedIcon sx={{ fontSize: 16 }} />
                </IconButton>
              </Tooltip>
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                Spend is approximate.
              </Typography>
            </Stack>

            <Box
              sx={{
                borderRadius: "12px",
                border: "1px solid rgba(130, 170, 160, 0.12)",
                background:
                  "linear-gradient(180deg, var(--ui-rgba-17-17-20-920), var(--ui-rgba-12-18-28-960))",
                px: { xs: 0.9, md: 1.3 },
                py: { xs: 0.5, md: 0.8 },
                minWidth: 0,
              }}
            >
              <ReactECharts
                style={{ height: 320 }}
                option={{
                  backgroundColor: "transparent",
                  animationDuration: 450,
                  grid: {
                    left: 18,
                    right: 12,
                    top: 18,
                    bottom: analyticsNeedsDataZoom ? 52 : 28,
                    containLabel: true,
                  },
                  dataZoom: analyticsNeedsDataZoom
                    ? [
                        {
                          type: "inside",
                          xAxisIndex: 0,
                          start: analyticsZoomStart,
                          end: 100,
                          throttle: 80,
                        },
                        {
                          type: "slider",
                          xAxisIndex: 0,
                          start: analyticsZoomStart,
                          end: 100,
                          height: 18,
                          bottom: 6,
                          borderColor: chartTokens.zoomBorder,
                          fillerColor: chartTokens.zoomFill,
                          handleStyle: { color: chartTokens.axisLabel },
                          textStyle: { color: chartTokens.axisLabel },
                        },
                      ]
                    : undefined,
                  tooltip: {
                    trigger: "axis",
                    backgroundColor: chartTokens.tooltipBg,
                    borderColor: chartTokens.tooltipBorder,
                    textStyle: { color: chartTokens.tooltipText },
                    formatter: (
                      params: Array<{
                        axisValue?: string;
                        dataIndex?: number;
                        value?: number | string | null;
                      }>,
                    ) => {
                      const index = params[0]?.dataIndex ?? 0;
                      const point = analyticsSeries[index];
                      const spend = typeof point?.cost_usd === "number" ? point.cost_usd : 0;
                      return [
                        analyticsBucketLabels[index] || params[0]?.axisValue || "Bucket",
                        `Spend: ${formatUsd(spend)}`,
                        `Requests: ${compactNumber(num(point?.request_count, 0))}`,
                        `Tokens: ${compactNumber(num(point?.total_tokens, 0))}`,
                      ].join("<br/>");
                    },
                  },
                  xAxis: {
                    type: "category",
                    data: analyticsBucketLabels,
                    axisTick: { show: false },
                    axisLabel: {
                      color: chartTokens.axisLabel,
                      fontSize: 10,
                      interval: 0,
                      formatter: (value: string, index: number) =>
                        index === 0 ||
                        index === analyticsBucketLabels.length - 1 ||
                        index % spendTickStride === 0
                          ? value
                          : "",
                    },
                    axisLine: {
                      lineStyle: { color: chartTokens.axisLine },
                    },
                  },
                  yAxis: {
                    type: "value",
                    axisLabel: {
                      color: chartTokens.axisLabel,
                      formatter: (value: number) => formatUsd(value),
                    },
                    splitLine: {
                      lineStyle: { color: chartTokens.splitLine },
                    },
                  },
                  series: [
                    {
                      type: "bar",
                      data: spendBucketSeries,
                      barWidth: "42%",
                      barMaxWidth: spendChartBarMaxWidth,
                      barMinWidth: 3,
                      itemStyle: {
                        borderRadius: [5, 5, 1, 1],
                        color: {
                          type: "linear",
                          x: 0,
                          y: 0,
                          x2: 0,
                          y2: 1,
                          colorStops: [
                            { offset: 0, color: "#f1d6ad" },
                            { offset: 0.36, color: "#d8ad78" },
                            { offset: 1, color: "#8d6841" },
                          ],
                        },
                      },
                      emphasis: {
                        itemStyle: {
                          color: "#f4ddb9",
                        },
                      },
                      markLine:
                        averageSpendPerBucket > 0
                          ? {
                              silent: true,
                              symbol: "none",
                              lineStyle: {
                                color: "rgba(216, 173, 120, 0.55)",
                                type: "dashed",
                              },
                              label: {
                                formatter: `avg ${formatUsd(averageSpendPerBucket)}`,
                                color: "#f1d6ad",
                                backgroundColor: "rgba(12, 18, 28, 0.92)",
                                padding: [3, 6],
                                borderRadius: 999,
                              },
                              data: [{ yAxis: averageSpendPerBucket }],
                            }
                          : undefined,
                    },
                  ],
                }}
              />
            </Box>

            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: {
                  xs: "1fr",
                  md: "repeat(2, minmax(0, 1fr))",
                  lg: "repeat(5, minmax(0, 1fr))",
                },
                gap: 1,
              }}
            >
              {heroSummaryCards.map((card) => (
                <Box
                  key={card.label}
                  sx={{
                    minWidth: 0,
                    borderRadius: "10px",
                    border: "1px solid rgba(130, 170, 160, 0.12)",
                    background:
                      "linear-gradient(180deg, var(--ui-rgba-22-22-26-920), var(--ui-rgba-15-15-18-880))",
                    px: { xs: 1.15, lg: 1 },
                    py: 0.9,
                    minHeight: 72,
                  }}
                >
                  <Typography
                    variant="overline"
                    title={card.label}
                    sx={{
                      color: "text.secondary",
                      display: "block",
                      fontSize: "0.64rem",
                      letterSpacing: 0,
                      lineHeight: 1.2,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {card.label}
                  </Typography>
                  <Typography
                    variant="subtitle1"
                    sx={{
                      mt: 0.25,
                      color: "#fff8ed",
                      fontSize: { xs: "1rem", lg: "0.92rem" },
                      fontWeight: 700,
                      lineHeight: 1.2,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                    title={card.value}
                  >
                    {card.value}
                  </Typography>
                  <Typography
                    variant="caption"
                    title={card.detail}
                    sx={{
                      color: "text.secondary",
                      display: "block",
                      fontSize: "0.68rem",
                      lineHeight: 1.25,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {card.detail}
                  </Typography>
                </Box>
              ))}
            </Box>
          </Box>

          <Stack
            spacing={1.5}
            sx={{
              gridColumn: { lg: "2" },
              gridRow: { lg: "1 / span 3" },
              minWidth: 0,
            }}
          >
            <Stack spacing={1} sx={{ minWidth: 0 }}>
              {railCards.map((card) => (
                <Box
                  key={card.label}
                  sx={{
                    minWidth: 0,
                    borderRadius: "12px",
                    border: "1px solid rgba(130, 170, 160, 0.12)",
                    background:
                      "linear-gradient(180deg, var(--ui-rgba-17-17-20-920), var(--ui-rgba-12-18-28-920))",
                    px: 1.2,
                    py: 1.1,
                  }}
                >
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: "center",
                      justifyContent: "space-between",
                    }}
                  >
                    <Box sx={{ minWidth: 0 }}>
                      <Typography
                        variant="overline"
                        sx={{ color: "text.secondary", letterSpacing: 0 }}
                      >
                        {card.label}
                      </Typography>
                      <Typography
                        variant="h6"
                        sx={{ color: "#f6f0e8", mt: 0.1 }}
                      >
                        {card.value}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block",
                          lineHeight: 1.45,
                        }}
                      >
                        {card.detail}
                      </Typography>
                    </Box>
                    <Box sx={{ width: 96, flex: "0 0 auto" }}>
                      <ReactECharts
                        style={{ height: 48 }}
                        option={buildSparklineOption(
                          card.values,
                          card.color,
                          card.chartType,
                        )}
                      />
                    </Box>
                  </Stack>
                </Box>
              ))}

              <Box
                sx={{
                  minWidth: 0,
                  borderRadius: "12px",
                  border: "1px solid rgba(130, 170, 160, 0.12)",
                  background:
                    "linear-gradient(180deg, var(--ui-rgba-17-17-20-920), var(--ui-rgba-12-18-28-920))",
                  px: 1.2,
                  py: 1.1,
                }}
              >
                <Typography
                  variant="overline"
                  sx={{ color: "text.secondary", letterSpacing: 0 }}
                >
                  Model leaders
                </Typography>
                <Stack spacing={0.95} sx={{ mt: 0.85 }}>
                  {byModelRows.length === 0 ? (
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      No model usage yet in the selected range.
                    </Typography>
                  ) : (
                    byModelRows.map((row, index) => {
                      const fill = Math.max(
                        10,
                        (num(row.request_count, 0) / modelMixMax) * 100,
                      );
                      const color = palette[index % palette.length];
                      const label = shortVersionLabel(
                        formatBreakdownLabel(row, "model"),
                        28,
                      );
                      return (
                        <Box key={`${label}-${index}`} sx={{ minWidth: 0 }}>
                          <Stack
                            direction="row"
                            spacing={1}
                            sx={{
                              justifyContent: "space-between",
                              alignItems: "center",
                              mb: 0.35,
                            }}
                          >
                            <Typography
                              variant="body2"
                              sx={{
                                color: "#fff8ed",
                                minWidth: 0,
                                overflow: "hidden",
                                textOverflow: "ellipsis",
                                whiteSpace: "nowrap",
                              }}
                              title={formatBreakdownLabel(row, "model")}
                            >
                              {label}
                            </Typography>
                            <Typography
                              variant="caption"
                              sx={{ color: "text.secondary", flexShrink: 0 }}
                            >
                              {compactNumber(num(row.request_count, 0))}
                            </Typography>
                          </Stack>
                          <Box
                            sx={{
                              height: 5,
                              borderRadius: 999,
                              background: "rgba(130, 170, 160, 0.08)",
                              overflow: "hidden",
                            }}
                          >
                            <Box
                              sx={{
                                width: `${fill}%`,
                                height: "100%",
                                borderRadius: 999,
                                background: color,
                                boxShadow: `0 0 14px ${color}55`,
                              }}
                            />
                          </Box>
                        </Box>
                      );
                    })
                  )}
                </Stack>
              </Box>
            </Stack>

            <Stack spacing={1.5} sx={{ minWidth: 0 }}>
              <MetricBarCard
                title="Model Mix"
                value={`${resp?.by_model?.length ?? 0} live`}
                values={byModelRows.map((row) => num(row.request_count, 0))}
                rows={byModelRows.map((row) => ({
                  label: shortVersionLabel(
                    formatBreakdownLabel(row, "model"),
                    28,
                  ),
                  value: compactNumber(num(row.request_count, 0)),
                }))}
                palette={palette}
                compact
              />
              <MetricBarCard
                title={secondaryBreakdownTitle}
                value={`${secondaryBreakdownRows.length} active`}
                values={secondaryBreakdownRows
                  .slice(0, 4)
                  .map((row) => num(row.request_count, 0))}
                rows={secondaryBreakdownRows.slice(0, 4).map((row) => ({
                  label: shortVersionLabel(
                    formatBreakdownLabel(row, secondaryBreakdownView),
                    28,
                  ),
                  value: compactNumber(num(row.request_count, 0)),
                }))}
                palette={palette}
                compact
              />
            </Stack>

            <Box
              className="list-shell"
              sx={{
                minWidth: 0,
                display: "flex",
                flexDirection: "column",
              }}
            >
              <Typography
                variant="h6"
                sx={{ color: "#fff8ed", fontWeight: 600 }}
              >
                Routing Policy Performance
              </Typography>
              <Typography
                variant="body2"
                sx={{ color: "text.secondary", mb: 1.2 }}
              >
                Success, errors, and tail latency across routing policy versions.
              </Typography>
              {policyMetricsQ.isLoading ? (
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Loading policy metrics...
                </Typography>
              ) : policyMetricsQ.error ? (
                <Alert severity="error">{errMessage(policyMetricsQ.error)}</Alert>
              ) : policyMetricsRows.length === 0 ? (
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  No routing policy metrics yet.
                </Typography>
              ) : (
                <Stack spacing={1.15}>
                  <ReactECharts
                    option={policyMetricsOption}
                    style={{ height: 286 }}
                  />
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: {
                        xs: "1fr",
                        sm: "repeat(3, minmax(0, 1fr))",
                      },
                      gap: 1,
                    }}
                  >
                    {policySummaryCards.map((card) => (
                      <Box
                        key={card.label}
                        sx={{
                          minWidth: 0,
                          borderRadius: "10px",
                          border: "1px solid rgba(130, 170, 160, 0.12)",
                          background:
                            "linear-gradient(180deg, var(--ui-rgba-22-22-26-920), var(--ui-rgba-15-15-18-880))",
                          px: 1,
                          py: 0.95,
                        }}
                      >
                        <Typography
                          variant="overline"
                          sx={{ color: "text.secondary", letterSpacing: 0 }}
                        >
                          {card.label}
                        </Typography>
                        <Typography
                          variant="subtitle2"
                          sx={{
                            mt: 0.2,
                            color: "#fff8ed",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                          title={card.value}
                        >
                          {card.value}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{ color: "text.secondary", display: "block" }}
                        >
                          {card.detail}
                        </Typography>
                      </Box>
                    ))}
                  </Box>
                  <TableContainer
                    className="table-shell"
                    sx={{ maxHeight: 220, overflow: "auto" }}
                  >
                    <Table stickyHeader size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Version</TableCell>
                          <TableCell align="right">Samples</TableCell>
                          <TableCell align="right">Success</TableCell>
                          <TableCell align="right">p95</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {policyMetricsRows.slice(0, 5).map((row, idx) => (
                          <TableRow key={`${str(row.version, "policy")}-${idx}`}>
                            <TableCell title={str(row.version, "-")}>
                              {shortVersionLabel(
                                formatPolicyVersionLabel(str(row.version, "-")),
                                24,
                              )}
                            </TableCell>
                            <TableCell align="right">
                              {num(row.samples, 0)}
                            </TableCell>
                            <TableCell align="right">
                              {(num(row.success_rate, 0) * 100).toFixed(1)}%
                            </TableCell>
                            <TableCell align="right">
                              {row.p95_latency_ms == null
                                ? "-"
                                : `${num(row.p95_latency_ms, 0)}ms`}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </TableContainer>
                </Stack>
              )}
            </Box>
          </Stack>

          <Box
            className="list-shell"
            sx={{ gridColumn: { lg: "1" }, p: 1.6, minWidth: 0 }}
          >
            <Typography
              variant="subtitle1"
              sx={{ color: "#fff8ed", fontWeight: 600, mb: 0.25 }}
            >
              Tokens Over Time
            </Typography>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                display: "block",
                mb: 0.75,
              }}
            >
              All LLM traffic, split into primary response generation vs
              helper/classifier passes, with prompt cache reads and writes.
            </Typography>
            <ReactECharts
              style={{ height: 248 }}
              option={{
                backgroundColor: "transparent",
                animationDuration: 400,
                grid: {
                  left: 56,
                  right: 16,
                  top: 20,
                  bottom: analyticsNeedsDataZoom ? 54 : 32,
                },
                dataZoom: analyticsNeedsDataZoom
                  ? [
                      {
                        type: "inside",
                        xAxisIndex: 0,
                        start: analyticsZoomStart,
                        end: 100,
                        throttle: 80,
                      },
                      {
                        type: "slider",
                        xAxisIndex: 0,
                        start: analyticsZoomStart,
                        end: 100,
                        height: 18,
                        bottom: 6,
                        borderColor: chartTokens.zoomBorder,
                        fillerColor: chartTokens.zoomFill,
                        handleStyle: { color: chartTokens.axisLabel },
                        textStyle: { color: chartTokens.axisLabel },
                      },
                    ]
                  : undefined,
                legend: {
                  top: 0,
                  textStyle: { color: chartTokens.legendText, fontSize: 11 },
                },
                tooltip: {
                  trigger: "axis",
                  backgroundColor: chartTokens.tooltipBg,
                  borderColor: chartTokens.tooltipBorder,
                  textStyle: { color: chartTokens.tooltipText },
                },
                xAxis: {
                  type: "category",
                  data: analyticsBucketLabels,
                  axisLabel: { color: chartTokens.axisLabel, fontSize: 10 },
                  axisLine: {
                    lineStyle: { color: chartTokens.axisLine },
                  },
                },
                yAxis: {
                  type: "value",
                  axisLabel: { color: chartTokens.axisLabel },
                  splitLine: {
                    lineStyle: { color: chartTokens.splitLine },
                  },
                },
                series: [
                  {
                    type: "line",
                    name: "Primary prompt",
                    data: analyticsSeries.map(
                      (point) => point.primary_prompt_tokens,
                    ),
                    smooth: true,
                    areaStyle: { opacity: 0.12 },
                    lineStyle: { color: "#14f195", width: 2 },
                    itemStyle: { color: "#14f195" },
                  },
                  {
                    type: "line",
                    name: "Primary completion",
                    data: analyticsSeries.map(
                      (point) => point.primary_completion_tokens,
                    ),
                    smooth: true,
                    areaStyle: { opacity: 0.12 },
                    lineStyle: { color: "#d8ad78", width: 2 },
                    itemStyle: { color: "#d8ad78" },
                  },
                  {
                    type: "line",
                    name: "Helper prompt",
                    data: analyticsSeries.map(
                      (point) => point.helper_prompt_tokens,
                    ),
                    smooth: true,
                    lineStyle: { color: "#fbbf24", width: 2, type: "dashed" },
                    itemStyle: { color: "#fbbf24" },
                  },
                  {
                    type: "line",
                    name: "Helper completion",
                    data: analyticsSeries.map(
                      (point) => point.helper_completion_tokens,
                    ),
                    smooth: true,
                    lineStyle: { color: "#c084fc", width: 2, type: "dashed" },
                    itemStyle: { color: "#c084fc" },
                  },
                  {
                    type: "line",
                    name: "Prompt cache read",
                    data: cachedPromptBucketSeries,
                    smooth: true,
                    lineStyle: { color: "#60a5fa", width: 2, type: "dashed" },
                    itemStyle: { color: "#60a5fa" },
                  },
                  {
                    type: "line",
                    name: "Prompt cache write",
                    data: cacheCreationPromptBucketSeries,
                    smooth: true,
                    lineStyle: { color: "#fb923c", width: 2, type: "dashed" },
                    itemStyle: { color: "#fb923c" },
                  },
                ],
              }}
            />
          </Box>

          <Box
            className="list-shell"
            sx={{
              gridColumn: { lg: "1" },
              minWidth: 0,
              display: "flex",
              flexDirection: "column",
            }}
          >
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
                <Typography variant="h6">{breakdownTitle}</Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  {breakdownDescription}
                </Typography>
              </Box>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                Range: {analyticsRangeLabel}
              </Typography>
            </Stack>
            <Box
              sx={{
                mb: 1.1,
                px: 0.45,
                borderRadius: 1.2,
                border: "1px solid rgba(130, 170, 160, 0.12)",
                background:
                  "linear-gradient(180deg, var(--ui-rgba-22-22-26-920), var(--ui-rgba-15-15-18-880))",
              }}
            >
              <Tabs
                value={breakdownView}
                onChange={(_, value: BreakdownView) => setBreakdownView(value)}
                variant="scrollable"
                allowScrollButtonsMobile
                className="workspace-page-subnav-tabs"
              >
                <Tab value="model" label="By model" />
                <Tab value="channel" label="By channel" />
                <Tab value="purpose" label="By purpose" />
              </Tabs>
            </Box>
            {previewBreakdownRows.length === 0 ? (
              <Typography
                variant="body2"
                sx={{ color: "text.secondary" }}
              >
                No analytics data yet for the selected range.
              </Typography>
            ) : (
              <TableContainer
                className="table-shell"
                sx={{
                  maxHeight: { xs: 360, md: 560 },
                  overflow: "auto",
                }}
              >
                <Table stickyHeader size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>
                        {breakdownView === "model"
                          ? "Model"
                          : breakdownView === "channel"
                            ? "Channel"
                            : "Purpose"}
                      </TableCell>
                      <TableCell align="right">Requests</TableCell>
                      <TableCell align="right">Tokens</TableCell>
                      <TableCell align="right">Cache read</TableCell>
                      <TableCell align="right">Cache write</TableCell>
                      <TableCell align="right">Cost</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {previewBreakdownRows.map((row, idx) => {
                      const label = formatBreakdownLabel(row, breakdownView);
                      return (
                        <TableRow key={`${label}-${idx}`}>
                          <TableCell sx={{ maxWidth: 340 }}>
                            <Typography variant="body2" noWrap title={label}>
                              {label}
                            </Typography>
                          </TableCell>
                          <TableCell align="right">
                            {num(row.request_count, 0).toLocaleString()}
                          </TableCell>
                          <TableCell align="right">
                            {num(row.total_tokens, 0).toLocaleString()}
                          </TableCell>
                          <TableCell align="right">
                            {num(row.cached_prompt_tokens, 0).toLocaleString()}
                          </TableCell>
                          <TableCell align="right">
                            {num(
                              row.cache_creation_prompt_tokens,
                              0,
                            ).toLocaleString()}
                          </TableCell>
                          <TableCell align="right">
                            {formatUsd(row.cost_usd, 4)}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
      </Box>
    </WorkspacePageShell>
  );
}
