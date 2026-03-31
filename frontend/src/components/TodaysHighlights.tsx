import { Box, Card, CardContent, Stack, Typography } from "@mui/material";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import TrendingUpRoundedIcon from "@mui/icons-material/TrendingUpRounded";
import TrendingDownRoundedIcon from "@mui/icons-material/TrendingDownRounded";
import ScheduleRoundedIcon from "@mui/icons-material/ScheduleRounded";
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { api } from "../api/client";
import type { LlmAnalyticsResponse, Task, TraceSummary } from "../types";

type Props = {
  tasks: Task[];
  traces: TraceSummary[];
};

function Sparkline({ values }: { values: number[] }) {
  if (!values || values.length < 2) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = Math.max(1e-9, max - min);
  const width = 120;
  const height = 28;
  const xs = values.map((_, index) => (width * index) / (values.length - 1));
  const ys = values.map((value) => 2 + (height - 4) * (1 - (value - min) / range));
  const line = xs.map((x, index) => `${x.toFixed(1)},${ys[index].toFixed(1)}`).join(" ");
  const area = `0,${height} ${line} ${width},${height}`;

  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden>
      <polygon points={area} fill="rgba(20, 241, 149, 0.15)" />
      <polyline
        points={line}
        fill="none"
        stroke="rgba(20, 241, 149, 0.8)"
        strokeWidth="2"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

function formatCompact(value: number): string {
  if (!Number.isFinite(value)) return "-";
  return new Intl.NumberFormat("en-US", {
    notation: value >= 1000 ? "compact" : "standard",
    maximumFractionDigits: value >= 1000 ? 1 : 0,
  }).format(value);
}

function formatSpend(value?: number | null): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "-";
  if (value >= 100) return `$${value.toFixed(0)}`;
  if (value >= 10) return `$${value.toFixed(1)}`;
  return `$${value.toFixed(2)}`;
}

export function TodaysHighlights({ tasks, traces }: Props) {
  const todayAnalyticsQ = useQuery({
    queryKey: ["mission-control-llm-analytics-24h"],
    queryFn: () => api.getLlmAnalytics({ range: "24h", bucket: "hour" }),
    staleTime: 60_000,
    refetchInterval: false,
  });
  const analytics30dQ = useQuery({
    queryKey: ["mission-control-llm-analytics-30d"],
    queryFn: () => api.getLlmAnalytics({ range: "30d", bucket: "day" }),
    staleTime: 60_000,
    refetchInterval: false,
  });

  const { completedToday, completedList, nextScheduled, trendPct, weekCounts, todayTraceCount } = useMemo(() => {
    const now = new Date();
    const todayStr = now.toISOString().slice(0, 10);
    const allTasks = Array.isArray(tasks) ? tasks : [];
    const todayCompleted = allTasks.filter((task) => {
      const status = String(task?.status || "").toLowerCase();
      return (status.includes("completed") || status.includes("done")) && (task.created_at ? task.created_at.startsWith(todayStr) : false);
    });
    const pending = allTasks.filter((task) => {
      const status = String(task?.status || "").toLowerCase();
      return status.includes("pending") && task.cron;
    });
    const allTraces = Array.isArray(traces) ? traces : [];
    const dayMs = 86_400_000;
    const counts: number[] = [];
    for (let dayOffset = 6; dayOffset >= 0; dayOffset -= 1) {
      const dayStart = new Date(now.getTime() - dayOffset * dayMs).toISOString().slice(0, 10);
      counts.push(allTraces.filter((trace) => (trace.started_at || "").startsWith(dayStart)).length);
    }
    const recentAvg = counts.slice(0, 6).reduce((sum, value) => sum + value, 0) / Math.max(1, 6);
    const todayCount = counts[counts.length - 1] || 0;
    const pct = recentAvg > 0 ? Math.round(((todayCount - recentAvg) / recentAvg) * 100) : 0;

    return {
      completedToday: todayCompleted.length,
      completedList: todayCompleted.slice(0, 3),
      nextScheduled: pending.length > 0 ? pending[0] : null,
      trendPct: pct,
      weekCounts: counts,
      todayTraceCount: todayCount,
    };
  }, [tasks, traces]);

  const timeSavedMin = completedToday * 10;
  const todayAnalytics = todayAnalyticsQ.data as LlmAnalyticsResponse | undefined;
  const analytics30d = analytics30dQ.data as LlmAnalyticsResponse | undefined;
  const todayUsageRows = [
    { label: "Today spend", value: formatSpend(todayAnalytics?.totals?.cost_usd ?? null) },
    { label: "Today requests", value: formatCompact(todayAnalytics?.totals?.request_count ?? 0) },
    { label: "Today tokens", value: formatCompact(todayAnalytics?.totals?.total_tokens ?? 0) },
  ];
  const fallbackRows = [
    { label: "Last 30 days spend", value: formatSpend(analytics30d?.totals?.cost_usd ?? null) },
    { label: "Last 30 days requests", value: formatCompact(analytics30d?.totals?.request_count ?? 0) },
    { label: "Last 30 days tokens", value: formatCompact(analytics30d?.totals?.total_tokens ?? 0) },
  ];
  const todayUsagePresent =
    (todayAnalytics?.totals?.request_count ?? 0) > 0 ||
    (todayAnalytics?.totals?.total_tokens ?? 0) > 0 ||
    (todayAnalytics?.totals?.cost_usd ?? 0) > 0;
  const noTodayData = completedToday === 0 && todayTraceCount === 0 && !todayUsagePresent;
  const summaryCards = noTodayData
    ? fallbackRows
    : [
        { label: "Completed", value: formatCompact(completedToday) },
        { label: "Live runs", value: formatCompact(todayTraceCount) },
        {
          label: todayUsagePresent ? "Today spend" : "Requests",
          value: todayUsagePresent
            ? formatSpend(todayAnalytics?.totals?.cost_usd ?? null)
            : formatCompact(todayAnalytics?.totals?.request_count ?? 0),
        },
      ];

  return (
    <Card className="mission-panel mission-panel--lower">
      <CardContent sx={{ p: 1.55, height: "100%", display: "flex", flexDirection: "column" }}>
        <Stack spacing={1.15} className="mission-panel-content">
          <Box>
            <Typography variant="h6" sx={{ fontWeight: 700 }}>
              Operational Summary
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Compact view of today's completion pace, runtime activity, and usage footprint.
            </Typography>
          </Box>

          {noTodayData ? (
            <Typography variant="body2" color="text.secondary">
              No meaningful activity yet today. Falling back to the trailing 30-day baseline.
            </Typography>
          ) : (
            <Stack direction="row" alignItems="center" spacing={0.55} useFlexGap flexWrap="wrap">
              <Typography variant="body2" sx={{ color: "rgba(225, 239, 255, 0.96)", fontWeight: 600 }}>
                {completedToday > 0 ? `${completedToday} completions` : "No completions yet"}
              </Typography>
              {trendPct !== 0 ? (
                <Stack direction="row" alignItems="center" spacing={0.25}>
                  {trendPct > 0 ? (
                    <TrendingUpRoundedIcon sx={{ fontSize: 15, color: "#14f195" }} />
                  ) : (
                    <TrendingDownRoundedIcon sx={{ fontSize: 15, color: "#ff9800" }} />
                  )}
                  <Typography
                    variant="caption"
                    fontWeight={700}
                    sx={{ color: trendPct > 0 ? "#14f195" : "#ff9800" }}
                  >
                    {trendPct > 0 ? "+" : ""}
                    {trendPct}% vs avg
                  </Typography>
                </Stack>
              ) : null}
            </Stack>
          )}

          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(3, minmax(0, 1fr))" },
              gap: 1,
            }}
          >
            {summaryCards.map((row) => (
              <Box
                key={row.label}
                sx={{
                  minWidth: 0,
                  px: 1.05,
                  py: 0.9,
                  borderRadius: "12px",
                  border: "1px solid rgba(108,156,212,0.16)",
                  background: "rgba(7, 18, 32, 0.56)",
                }}
              >
                <Typography variant="caption" color="text.secondary">
                  {row.label}
                </Typography>
                <Typography variant="h6" sx={{ mt: 0.2, fontWeight: 700, color: "#f3fbff" }}>
                  {row.value}
                </Typography>
              </Box>
            ))}
          </Box>

          {completedList.length > 0 ? (
            <Stack spacing={0.55} className="mission-panel-section">
              {completedList.map((task, index) => (
                <Stack key={task.id || index} direction="row" spacing={0.75} alignItems="center">
                  <CheckCircleRoundedIcon sx={{ fontSize: 14, color: "#14f195", flexShrink: 0 }} />
                  <Typography variant="body2" noWrap sx={{ minWidth: 0 }} title={String(task.description || "")}>
                    {String(task.description || "Task completed")}
                  </Typography>
                </Stack>
              ))}
            </Stack>
          ) : (
            <Box className="mission-empty-copy" sx={{ justifyContent: "flex-start", py: 0.35 }}>
              <Typography variant="body2" color="text.secondary">
                No completed tasks have landed yet today.
              </Typography>
            </Box>
          )}

          {todayUsagePresent ? (
            <Stack direction={{ xs: "column", sm: "row" }} spacing={0.85} useFlexGap flexWrap="wrap">
              {todayUsageRows.map((row) => (
                <Typography key={row.label} variant="caption" color="text.secondary">
                  {row.label}: <span style={{ color: "rgba(230, 241, 255, 0.94)" }}>{row.value}</span>
                </Typography>
              ))}
            </Stack>
          ) : null}

          {nextScheduled ? (
            <Stack direction="row" spacing={0.75} alignItems="center">
              <ScheduleRoundedIcon sx={{ fontSize: 14, color: "#2fd4ff", flexShrink: 0 }} />
              <Typography variant="body2" color="text.secondary">
                Next scheduled: {String(nextScheduled.description || "Scheduled task").slice(0, 56)}
              </Typography>
            </Stack>
          ) : null}

          <Box className="mission-sparkline-shell" sx={{ opacity: 0.9 }}>
            <Sparkline values={weekCounts} />
          </Box>

          <Typography variant="caption" color="text.secondary" display="block">
            {timeSavedMin > 0
              ? `Estimated operator time reclaimed today: ~${timeSavedMin} min`
              : "Time reclaimed will appear here as automated work completes."}
          </Typography>
        </Stack>
      </CardContent>
    </Card>
  );
}
