import { Box, Button, Card, CardContent, Chip, Collapse, Stack, Typography } from "@mui/material";
import { useState } from "react";
import type { TraceSummary } from "../types";

type Props = {
  traces: TraceSummary[];
  onViewAll: () => void;
};

function humanizeTrace(trace: TraceSummary): string {
  const msg = (trace.message_preview || "").trim();
  const ch = (trace.channel || "").toLowerCase();

  if (ch.includes("gmail") || msg.toLowerCase().includes("email")) {
    if (msg.toLowerCase().includes("send")) return `Sent email: ${msg.slice(0, 50)}`;
    return `Processed email: ${msg.slice(0, 50)}`;
  }
  if (ch.includes("telegram")) return `Telegram: ${msg.slice(0, 60)}`;
  if (ch.includes("whatsapp")) return `WhatsApp: ${msg.slice(0, 60)}`;
  if (msg.toLowerCase().includes("briefing") || msg.toLowerCase().includes("brief"))
    return "Generated daily briefing";
  if (msg.toLowerCase().includes("research")) return `Research: ${msg.slice(0, 50)}`;
  if (msg.toLowerCase().includes("calendar")) return `Calendar: ${msg.slice(0, 50)}`;
  if (msg.toLowerCase().includes("search")) return `Search: ${msg.slice(0, 50)}`;

  return msg.slice(0, 80) || "Task executed";
}

function relativeTime(isoStr: string): string {
  const then = new Date(isoStr).getTime();
  if (!then) return "";
  const diffMs = Date.now() - then;
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function statusColor(status: string): "success" | "error" | "warning" | "default" {
  const s = (status || "").toLowerCase();
  if (s.includes("completed") || s.includes("done") || s.includes("success")) return "success";
  if (s.includes("fail") || s.includes("error")) return "error";
  if (s.includes("running") || s.includes("progress")) return "warning";
  return "default";
}

function statusLabel(status: string): string {
  const s = (status || "").toLowerCase();
  if (s.includes("completed") || s.includes("done") || s.includes("success")) return "done";
  if (s.includes("fail") || s.includes("error")) return "failed";
  if (s.includes("running") || s.includes("progress")) return "running";
  return status || "done";
}

export function ActivityFeed({ traces, onViewAll }: Props) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const items = (traces || []).slice(0, 5);

  return (
    <Card sx={{ height: "100%" }}>
      <CardContent sx={{ p: 1.55 }}>
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Box>
            <Typography variant="h6" sx={{ fontWeight: 700 }}>
              Runtime Activity
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Recent supervised runs and operator-visible outcomes.
            </Typography>
          </Box>
          {traces.length > 5 ? (
            <Button size="small" onClick={onViewAll} sx={{ textTransform: "none" }}>
              Open trace
            </Button>
          ) : null}
        </Stack>

        {items.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No recent activity.
          </Typography>
        ) : (
          <Stack spacing={0.7}>
            {items.map((trace) => (
              <Box
                key={trace.id}
                className="activity-item console-line"
                onClick={() => setExpandedId(expandedId === trace.id ? null : trace.id)}
                sx={{
                  cursor: "pointer",
                  borderRadius: 2.5,
                  background: "linear-gradient(180deg, rgba(6, 16, 30, 0.82), rgba(4, 12, 24, 0.76))",
                }}
              >
                <Stack
                  direction="row"
                  spacing={1}
                  alignItems="center"
                  justifyContent="space-between"
                >
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ flexShrink: 0, minWidth: 62, fontFamily: "JetBrains Mono, monospace" }}
                  >
                    {relativeTime(trace.started_at)}
                  </Typography>
                  <Typography
                    variant="body2"
                    noWrap
                    sx={{ flex: 1, minWidth: 0 }}
                    title={trace.message_preview}
                  >
                    {humanizeTrace(trace)}
                  </Typography>
                  <Chip
                    label={statusLabel(trace.status)}
                    color={statusColor(trace.status)}
                    size="small"
                    variant="outlined"
                    sx={{ height: 22, fontSize: "0.7rem" }}
                  />
                </Stack>

                <Collapse in={expandedId === trace.id}>
                  <Box sx={{ mt: 0.75, pl: 1, borderLeft: "2px solid rgba(47, 212, 255, 0.25)" }}>
                    {trace.duration_ms != null ? (
                      <Typography variant="caption" color="text.secondary" display="block">
                        Completed in {trace.duration_ms}ms
                      </Typography>
                    ) : null}
                    <Typography variant="caption" color="text.secondary" display="block">
                      {trace.step_count} step{trace.step_count !== 1 ? "s" : ""} executed
                    </Typography>
                    {trace.channel ? (
                      <Typography variant="caption" color="text.secondary" display="block">
                        Channel: {trace.channel}
                      </Typography>
                    ) : null}
                    <Typography variant="caption" color="text.secondary" display="block" mt={0.25}>
                      {trace.message_preview}
                    </Typography>
                  </Box>
                </Collapse>
              </Box>
            ))}
          </Stack>
        )}
      </CardContent>
    </Card>
  );
}
