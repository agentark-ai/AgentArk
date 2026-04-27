import { Box, Button, Card, CardContent, Chip, Collapse, Stack, Typography } from "@mui/material";
import { useState } from "react";
import type { TraceSummary } from "../types";

type Props = {
  traces: TraceSummary[];
  onViewAll: () => void;
};

function humanizeTrace(trace: TraceSummary): string {
  const message = (trace.message_preview || "").trim();
  const channel = (trace.channel || "").toLowerCase();

  if (channel.includes("gmail") || message.toLowerCase().includes("email")) {
    if (message.toLowerCase().includes("send")) return `Sent email: ${message.slice(0, 50)}`;
    return `Processed email: ${message.slice(0, 50)}`;
  }
  if (channel.includes("telegram")) return `Telegram: ${message.slice(0, 60)}`;
  if (channel.includes("whatsapp")) return `WhatsApp: ${message.slice(0, 60)}`;
  if (message.toLowerCase().includes("briefing") || message.toLowerCase().includes("brief")) return "Generated daily briefing";
  if (message.toLowerCase().includes("research")) return `Research: ${message.slice(0, 50)}`;
  if (message.toLowerCase().includes("calendar")) return `Calendar: ${message.slice(0, 50)}`;
  if (message.toLowerCase().includes("search")) return `Search: ${message.slice(0, 50)}`;

  return message.slice(0, 80) || "Task executed";
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
  const normalized = (status || "").toLowerCase();
  if (normalized.includes("completed") || normalized.includes("done") || normalized.includes("success")) return "success";
  if (normalized.includes("fail") || normalized.includes("error")) return "error";
  if (normalized.includes("running") || normalized.includes("progress")) return "warning";
  return "default";
}

function statusLabel(status: string): string {
  const normalized = (status || "").toLowerCase();
  if (normalized.includes("completed") || normalized.includes("done") || normalized.includes("success")) return "done";
  if (normalized.includes("fail") || normalized.includes("error")) return "failed";
  if (normalized.includes("running") || normalized.includes("progress")) return "running";
  return status || "done";
}

export function ActivityFeed({ traces, onViewAll }: Props) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const items = (traces || []).slice(0, 5);

  return (
    <Card className="mission-panel mission-panel--lower mission-panel--adaptive">
      <CardContent sx={{ p: 1.55, display: "flex", flexDirection: "column" }}>
        <Stack spacing={1} className="mission-panel-content">
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center"
            }}>
            <Box>
              <Typography variant="h6" sx={{ fontWeight: 700 }}>
                Runtime Activity
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Recent supervised runs and operator-visible outcomes.
              </Typography>
            </Box>
            {traces.length > 5 ? (
              <Button variant="outlined" size="small" onClick={onViewAll} sx={{ textTransform: "none" }}>
                Open trace
              </Button>
            ) : null}
          </Stack>

          {items.length === 0 ? (
            <Box className="mission-empty-copy">
              <Typography variant="body2" sx={{
                color: "text.secondary"
              }}>
                No recent activity.
              </Typography>
            </Box>
          ) : (
            <Stack spacing={0.7} className="mission-panel-section">
              {items.map((trace) => (
                <Box
                  key={trace.id}
                  className="activity-item console-line"
                  onClick={() => setExpandedId(expandedId === trace.id ? null : trace.id)}
                  sx={{
                    cursor: "pointer",
                    borderRadius: 2,
                    background: "linear-gradient(180deg, var(--ui-rgba-24-24-28-920), var(--ui-rgba-15-15-18-880))",
                  }}
                >
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: "center",
                      justifyContent: "space-between"
                    }}>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        flexShrink: 0,
                        minWidth: 62,
                        fontFamily: "JetBrains Mono, monospace"
                      }}>
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
                    <Box sx={{ mt: 0.75, pl: 1, borderLeft: "2px solid var(--ui-rgba-255-255-255-120)" }}>
                      {trace.duration_ms != null ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block"
                          }}>
                          Completed in {trace.duration_ms}ms
                        </Typography>
                      ) : null}
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block"
                        }}>
                        {trace.step_count} step{trace.step_count !== 1 ? "s" : ""} executed
                      </Typography>
                      {trace.channel ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block"
                          }}>
                          Channel: {trace.channel}
                        </Typography>
                      ) : null}
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block",
                          mt: 0.25
                        }}>
                        {trace.message_preview}
                      </Typography>
                    </Box>
                  </Collapse>
                </Box>
              ))}
            </Stack>
          )}
        </Stack>
      </CardContent>
    </Card>
  );
}
