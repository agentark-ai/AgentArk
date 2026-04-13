import { useEffect, useRef, useState } from "react";
import { Box, Button, Card, CardContent, Chip, Stack, Typography } from "@mui/material";
import type { TraceOperationalEvent, TraceSummary } from "../types";

type Props = {
  history: TraceSummary[];
  events?: TraceOperationalEvent[];
  compact?: boolean;
  onHideAdvanced?: () => void;
};

type ConsoleTone = "running" | "success" | "warning" | "error" | "idle";

type ConsoleStage = {
  label: string;
  tone: ConsoleTone;
  detail: string;
};

type ConsoleEntry = {
  id: string;
  title: string;
  detail: string;
  timestamp: string;
  tone: ConsoleTone;
  badge: string;
  meta: string[];
};

function formatLabel(value?: string | null, fallback = "Unknown"): string {
  if (!value) return fallback;
  const normalized = value.replace(/[_-]+/g, " ").replace(/\s+/g, " ").trim();
  if (!normalized) return fallback;
  return normalized.replace(/\b\w/g, (char) => char.toUpperCase());
}

function shortTimestamp(value?: string | null): string {
  if (!value) return "--";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "--";
  return parsed.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function formatDuration(value?: number | null): string {
  if (value == null || Number.isNaN(value) || value < 0) return "";
  if (value < 1000) return `${Math.round(value)} ms`;
  const seconds = value / 1000;
  if (seconds < 60) {
    return `${seconds >= 10 ? Math.round(seconds) : seconds.toFixed(1)} s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.round(seconds % 60);
  return `${minutes}m ${String(remainingSeconds).padStart(2, "0")}s`;
}

function pickLatestByTime<T>(items: T[], getTimestamp: (item: T) => string | null | undefined): T | undefined {
  let latest: T | undefined;
  let latestTime = Number.NEGATIVE_INFINITY;

  for (const item of items) {
    const rawTimestamp = getTimestamp(item);
    const parsedTime = rawTimestamp ? Date.parse(rawTimestamp) : Number.NaN;

    if (!latest) {
      latest = item;
      latestTime = parsedTime;
      continue;
    }

    if (!Number.isNaN(parsedTime) && (Number.isNaN(latestTime) || parsedTime > latestTime)) {
      latest = item;
      latestTime = parsedTime;
    }
  }

  return latest;
}

function eventTone(event: TraceOperationalEvent): ConsoleTone {
  if (!event.success) return "error";
  const normalized = `${event.event_type} ${event.outcome} ${event.tool_name || ""}`.toLowerCase();
  if (normalized.includes("blocked") || normalized.includes("warning") || normalized.includes("review")) {
    return "warning";
  }
  if (
    normalized.includes("complete") ||
    normalized.includes("done") ||
    normalized.includes("final") ||
    normalized.includes("success") ||
    normalized.includes("ok")
  ) {
    return "success";
  }
  if (normalized.includes("idle") || normalized.includes("waiting")) {
    return "idle";
  }
  return "running";
}

function traceTone(item: TraceSummary): ConsoleTone {
  const status = `${item.status || ""}`.toLowerCase();
  if (status.includes("fail") || status.includes("error") || status.includes("cancel")) return "error";
  if (status.includes("complete") || status.includes("done") || status.includes("success")) return "success";
  if (status.includes("pending") || status.includes("queued") || status.includes("waiting")) return "warning";
  if (status.includes("idle")) return "idle";
  return "running";
}

function stageFromEvent(event?: TraceOperationalEvent): ConsoleStage {
  if (!event) {
    return {
      label: "Idle",
      tone: "idle",
      detail: "Waiting for the next run.",
    };
  }

  const tone = eventTone(event);
  const normalized = `${event.event_type} ${event.outcome} ${event.tool_name || ""}`.toLowerCase();

  if (tone === "error") {
    return {
      label: "Attention",
      tone,
      detail: "A branch needs review before the run can settle.",
    };
  }

  if (tone === "warning") {
    return {
      label: "Checking",
      tone,
      detail: "The console is validating a branch before it moves on.",
    };
  }

  if (normalized.includes("summary") || normalized.includes("synth") || normalized.includes("final")) {
    return {
      label: "Finalizing",
      tone: "running",
      detail: "Packaging the latest research into a stable answer.",
    };
  }

  if (normalized.includes("plan") || normalized.includes("route") || normalized.includes("prepare")) {
    return {
      label: "Planning",
      tone: "running",
      detail: "Breaking the request into the next actions.",
    };
  }

  if (tone === "success") {
    return {
      label: "Completed",
      tone,
      detail: "The latest run reached a stable finish.",
    };
  }

  if (
    normalized.includes("tool") ||
    normalized.includes("search") ||
    normalized.includes("browser") ||
    normalized.includes("fetch") ||
    normalized.includes("execute")
  ) {
    return {
      label: "Executing",
      tone: "running",
      detail: "Running tools and collecting fresh signals.",
    };
  }

  return {
    label: "Running",
    tone: "running",
    detail: "Streaming the latest execution path.",
  };
}

function stageFromTrace(trace?: TraceSummary): ConsoleStage {
  if (!trace) {
    return {
      label: "Idle",
      tone: "idle",
      detail: "Waiting for the next run.",
    };
  }

  const tone = traceTone(trace);
  const status = `${trace.status || ""}`.toLowerCase();

  if (tone === "error") {
    return {
      label: "Attention",
      tone,
      detail: "The latest run ended with an issue that needs review.",
    };
  }

  if (tone === "warning") {
    return {
      label: "Queued",
      tone,
      detail: "The next run is lined up and waiting to start.",
    };
  }

  if (tone === "success" || status.includes("complete")) {
    return {
      label: "Completed",
      tone: "success",
      detail: "Recent trace history is available and stable.",
    };
  }

  return {
    label: "Running",
    tone: "running",
    detail: "Recent trace history is still moving.",
  };
}

function badgeFromTone(tone: ConsoleTone): string {
  switch (tone) {
    case "success":
      return "Stable";
    case "warning":
      return "Check";
    case "error":
      return "Attention";
    case "idle":
      return "Standby";
    default:
      return "Live";
  }
}

function buildEventEntry(event: TraceOperationalEvent): ConsoleEntry {
  const tone = eventTone(event);
  const title = event.tool_name
    ? `${formatLabel(event.event_type)} / ${event.tool_name}`
    : formatLabel(event.event_type);
  const detail = event.outcome ? formatLabel(event.outcome) : "Live operational signal";
  const meta = [
    event.channel ? `${formatLabel(event.channel)} channel` : "",
    event.latency_ms != null ? formatDuration(event.latency_ms) : "",
  ].filter(Boolean);

  return {
    id: event.id,
    title,
    detail,
    timestamp: shortTimestamp(event.created_at),
    tone,
    badge: badgeFromTone(tone),
    meta,
  };
}

function buildTraceEntry(trace: TraceSummary): ConsoleEntry {
  const tone = traceTone(trace);
  const meta = [
    trace.channel ? `${formatLabel(trace.channel)} channel` : "",
    Number.isFinite(trace.step_count) ? `${trace.step_count} steps` : "",
    trace.duration_ms != null ? formatDuration(trace.duration_ms) : "",
  ].filter(Boolean);

  return {
    id: trace.id,
    title: trace.message_preview || "(empty prompt)",
    detail: `${formatLabel(trace.status, "Running")} trace`,
    timestamp: shortTimestamp(trace.started_at),
    tone,
    badge: tone === "success" ? "Completed" : badgeFromTone(tone),
    meta,
  };
}

export function LiveEventConsole({ history, events = [], compact = false, onHideAdvanced }: Props) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [shouldStickToBottom, setShouldStickToBottom] = useState(true);
  const visibleEvents = events.slice(0, compact ? 4 : 6).reverse();
  const visibleHistory = history.slice(0, compact ? 4 : 6).reverse();
  const latestEvent = pickLatestByTime(events, (item) => item.created_at);
  const latestTrace = pickLatestByTime(history, (item) => item.started_at);
  const stage = latestEvent ? stageFromEvent(latestEvent) : stageFromTrace(latestTrace);
  const sourceLabel = latestEvent?.channel || latestTrace?.channel
    ? formatLabel(latestEvent?.channel || latestTrace?.channel)
    : "Standby";
  const updatedLabel = latestEvent?.created_at || latestTrace?.started_at
    ? shortTimestamp(latestEvent?.created_at || latestTrace?.started_at)
    : "--";
  const feedLabel = events.length > 0 ? `${events.length} live signals` : history.length > 0 ? `${history.length} recent runs` : "Standby";
  const stageDetail = sourceLabel === "Standby" ? stage.detail : `${stage.detail} Source: ${sourceLabel}.`;
  const entries = visibleEvents.length > 0 ? visibleEvents.map(buildEventEntry) : visibleHistory.map(buildTraceEntry);
  const entriesKey = entries.map((entry) => entry.id).join("|");

  useEffect(() => {
    const container = scrollRef.current;
    if (!container || !shouldStickToBottom) return;

    const frame = window.requestAnimationFrame(() => {
      container.scrollTop = container.scrollHeight;
    });

    return () => window.cancelAnimationFrame(frame);
  }, [entriesKey, shouldStickToBottom]);

  function handleFeedScroll() {
    const container = scrollRef.current;
    if (!container) return;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    const isNearBottom = distanceFromBottom <= 28;
    setShouldStickToBottom((current) => (current === isNearBottom ? current : isNearBottom));
  }

  return (
    <Card className={`live-console-card${compact ? " live-console-card-compact" : ""}`} sx={compact ? { minHeight: 0, height: "100%" } : { minHeight: 270 }}>
      <CardContent
        className="live-console-content"
        sx={{
          p: compact ? 1.25 : 1.5,
          height: "100%",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        }}
      >
        <Stack direction={{ xs: "column", sm: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", sm: "flex-start" }} spacing={1.25} className="live-console-header">
          <Stack spacing={0.7} sx={{ minWidth: 0 }}>
            <Typography className="live-console-eyebrow">AgentArk</Typography>
            <Typography className="live-console-title">Execution Console</Typography>
            <Typography className="live-console-copy">{stageDetail}</Typography>
          </Stack>
          <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
            {onHideAdvanced ? (
              <Button
                size="small"
                variant="outlined"
                color="warning"
                onClick={onHideAdvanced}
                sx={{ textTransform: "none" }}
              >
                Hide advanced
              </Button>
            ) : null}
            <Chip
              size="small"
              label={stage.label}
              className={`live-console-status-chip tone-${stage.tone}`}
            />
          </Stack>
        </Stack>

        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} className="live-console-summary">
          <Box className="live-console-summary-item">
            <Typography className="live-console-summary-label">Status</Typography>
            <Typography className="live-console-summary-value">{stage.label}</Typography>
          </Box>
          <Box className="live-console-summary-item">
            <Typography className="live-console-summary-label">Feed</Typography>
            <Typography className="live-console-summary-value">{feedLabel}</Typography>
          </Box>
          <Box className="live-console-summary-item">
            <Typography className="live-console-summary-label">Updated</Typography>
            <Typography className="live-console-summary-value">{updatedLabel}</Typography>
          </Box>
        </Stack>

        <Stack
          ref={scrollRef}
          spacing={1}
          className="console-scroll live-console-feed"
          sx={{ flex: 1, minHeight: 0, maxHeight: "none" }}
          onScroll={handleFeedScroll}
        >
          {entries.length === 0 ? (
            <Stack spacing={0.75} className="live-console-empty">
              <Typography className="live-console-empty-title">Console standing by</Typography>
              <Typography className="live-console-empty-copy">
                Start a run and the latest planning, tool, and completion signals will stream here.
              </Typography>
            </Stack>
          ) : (
            entries.map((entry) => (
              <Stack
                key={entry.id}
                direction="row"
                spacing={1.25}
                alignItems="stretch"
                className={`live-console-entry tone-${entry.tone}`}
              >
                <Box className="live-console-entry-rail" />
                <Stack spacing={0.45} sx={{ flex: 1, minWidth: 0 }} className="live-console-entry-main">
                  <Stack
                    direction={{ xs: "column", sm: "row" }}
                    spacing={0.75}
                    justifyContent="space-between"
                    alignItems={{ xs: "flex-start", sm: "center" }}
                    className="live-console-entry-head"
                  >
                    <Typography className="live-console-entry-title">{entry.title}</Typography>
                    <Typography className="live-console-entry-time">{entry.timestamp}</Typography>
                  </Stack>
                  <Typography className="live-console-entry-detail">{entry.detail}</Typography>
                  {entry.meta.length > 0 ? (
                    <Typography className="live-console-entry-meta">{entry.meta.join(" / ")}</Typography>
                  ) : null}
                </Stack>
                <Box component="span" className={`live-console-entry-badge tone-${entry.tone}`}>
                  {entry.badge}
                </Box>
              </Stack>
            ))
          )}
        </Stack>
      </CardContent>
    </Card>
  );
}
