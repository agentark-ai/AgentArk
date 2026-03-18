import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography
} from "@mui/material";
import CloseIcon from "@mui/icons-material/Close";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api/client";

const REFRESH_MS = 8000;
const HISTORY_LIMIT = 200;

type JsonRecord = Record<string, unknown>;

type Props = {
  autoRefresh: boolean;
};

type ProvisionedAgent = {
  id: string;
  name: string;
  provider: string;
  model: string;
  capabilities: string[];
  createdAt: string;
  status: string;
};

type HistoryItem = {
  id: string;
  agentName: string;
  triggerText: string;
  workText: string;
  status: string;
  timestamp: string;
  detail: string;
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function asRecords(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  if (Array.isArray(value)) return asRecords(value);
  const obj = asRecord(value);
  return asRecords(obj[key]);
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function boolText(value: unknown): string {
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return value === 0 ? "false" : "true";
  if (typeof value === "string" && value.trim()) return value;
  return "false";
}

function errMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Request failed.";
}

function statusChipColor(status: string): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "provisioned" || normalized === "idle" || normalized === "completed") return "success";
  if (normalized === "busy" || normalized === "running") return "warning";
  if (normalized === "offline" || normalized === "failed" || normalized === "cancelled") return "error";
  return "default";
}

function normalizeLifecycleStatus(status: unknown): string {
  const normalized = str(status, "").trim().toLowerCase();
  if (normalized === "busy" || normalized === "running") return "running";
  if (normalized === "completed" || normalized === "success") return "completed";
  if (normalized === "failed" || normalized === "error") return "failed";
  if (normalized === "cancelled" || normalized === "canceled") return "cancelled";
  if (normalized === "offline") return "offline";
  if (normalized === "disabled") return "disabled";
  if (normalized === "idle" || normalized === "provisioned") return "provisioned";
  return normalized || "provisioned";
}

function statusChipLabel(status: unknown): string {
  switch (normalizeLifecycleStatus(status)) {
    case "running":
      return "Running";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "offline":
      return "Offline";
    case "disabled":
      return "Disabled";
    default:
      return "Provisioned";
  }
}

function formatTimestamp(value: unknown): string {
  const raw = str(value, "").trim();
  if (!raw) return "-";
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return raw;
  return parsed.toLocaleString();
}

function compactChatId(value: string): string {
  const trimmed = value.trim();
  return trimmed ? trimmed.slice(0, 8) : "";
}

function parseCsv(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function formatCapabilities(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        if (typeof item === "string") return item.trim();
        const rec = asRecord(item);
        return str(rec.name, "").trim() || str(rec.description, "").trim();
      })
      .filter(Boolean);
  }
  const raw = str(value, "").trim();
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed)) return formatCapabilities(parsed);
  } catch {
    return parseCsv(raw);
  }
  return [];
}

export function SwarmManager({ autoRefresh }: Props) {
  const [historyOpen, setHistoryOpen] = useState(false);

  const statusQ = useQuery({
    queryKey: ["swarm-status"],
    queryFn: () => api.rawGet("/swarm/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const agentsQ = useQuery({
    queryKey: ["swarm-agents"],
    queryFn: () => api.rawGet("/swarm/agents"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const configQ = useQuery({
    queryKey: ["swarm-config"],
    queryFn: () => api.rawGet("/swarm/config"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const delegationsQ = useQuery({
    queryKey: ["swarm-delegations"],
    queryFn: () => api.rawGet("/swarm/delegations?limit=all"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const status = asRecord(statusQ.data);
  const config = asRecord(configQ.data);
  const agents = pickRecords(agentsQ.data, "agents");
  const delegations = pickRecords(delegationsQ.data, "delegations");
  const liveAgents = pickRecords(status.agents, "agents");
  const liveById = new Map(
    liveAgents.map((agent) => [str(agent.id, ""), normalizeLifecycleStatus(agent.status)])
  );
  const agentNameById = new Map(agents.map((agent) => [str(agent.id, ""), str(agent.name, "Agent")]));
  const swarmEnabled = boolText(status.enabled || config.enabled) === "true";

  const provisionedAgents: ProvisionedAgent[] = agents
    .map((agent) => {
      const id = str(agent.id, "");
      const enabled = boolText(agent.enabled) === "true";
      return {
        id,
        name: str(agent.name, "Agent"),
        provider: str(agent.llm_provider, "ollama"),
        model: str(agent.llm_model, "-"),
        capabilities: formatCapabilities(agent.capabilities),
        createdAt: str(agent.created_at, ""),
        status: enabled
          ? liveById.get(id) || normalizeLifecycleStatus(agent.status)
          : "disabled"
      };
    })
    .sort((left, right) => {
      const leftTs = Date.parse(left.createdAt || "");
      const rightTs = Date.parse(right.createdAt || "");
      return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
    });

  const runningAgents = provisionedAgents.filter((agent) => agent.status === "running");
  const historyItems: HistoryItem[] = delegations
    .map((row) => {
      const completedAt = str(row.completed_at, "");
      const createdAt = str(row.created_at, "");
      const success = boolText(row.success) === "true";
      const resultText = str(row.result, "").toLowerCase();
      const agentId = str(row.agent_id, "");
      const duration = num(row.execution_time_ms, 0);
      const channel = str(row.channel, str(row.source, "")).trim();
      const chatId = str(row.chat_id, str(row.conversation_id, "")).trim();
      const historicalStatus = !completedAt
        ? "running"
        : success
          ? "completed"
          : resultText.includes("cancel")
            ? "cancelled"
            : "failed";
      const triggerParts: string[] = [];
      if (channel) triggerParts.push(channel);
      if (chatId) triggerParts.push(`chat ${compactChatId(chatId)}`);
      const detailParts: string[] = [];
      if (duration > 0) detailParts.push(`${duration}ms`);
      if (createdAt && completedAt) detailParts.push(`finished ${formatTimestamp(completedAt)}`);
      return {
        id: `delegation-${str(row.id, agentId || "history")}`,
        agentName: agentNameById.get(agentId) || agentId || "Agent",
        triggerText: triggerParts.length > 0 ? `Triggered by ${triggerParts.join(" | ")}` : "Triggered internally",
        workText: str(row.task, "Delegated task"),
        status: historicalStatus,
        timestamp: completedAt || createdAt,
        detail: detailParts.length > 0 ? detailParts.join(" | ") : "Delegation run"
      };
    })
    .sort((left, right) => {
      const leftTs = Date.parse(left.timestamp || "");
      const rightTs = Date.parse(right.timestamp || "");
      return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
    })
    .slice(0, HISTORY_LIMIT);

  const runningCount = runningAgents.length;
  const completedCount = historyItems.filter((item) => item.status === "completed").length;
  const failedCount = historyItems.filter((item) => item.status === "failed").length;
  const cancelledCount = historyItems.filter((item) => item.status === "cancelled").length;
  const queryError = statusQ.error || configQ.error || agentsQ.error || delegationsQ.error;

  return (
    <Stack spacing={2.5}>
      <Stack
        direction={{ xs: "column", sm: "row" }}
        justifyContent="space-between"
        alignItems={{ xs: "flex-start", sm: "center" }}
        gap={1.5}
      >
        <Stack direction="row" spacing={1.5} alignItems="center">
          <Typography variant="h6" sx={{ fontWeight: 700 }}>
            Agents
          </Typography>
          <Chip
            size="small"
            color={swarmEnabled ? "success" : "default"}
            variant={swarmEnabled ? "filled" : "outlined"}
            label={swarmEnabled ? "Swarm enabled" : "Swarm disabled"}
          />
        </Stack>
        <Stack direction="row" spacing={1} alignItems="center">
          <Chip
            size="small"
            variant="outlined"
            label={`${runningCount} running`}
            color={runningCount > 0 ? "warning" : "default"}
          />
          <Button
            size="small"
            variant="outlined"
            onClick={() => setHistoryOpen(true)}
            sx={{ textTransform: "none" }}
          >
            History ({historyItems.length})
          </Button>
        </Stack>
      </Stack>

      {queryError ? (
        <Alert severity="error">{errMessage(queryError)}</Alert>
      ) : null}

      {runningAgents.length === 0 ? (
        <Box
          sx={{
            p: { xs: 2, md: 2.5 },
            borderRadius: "14px",
            background:
              "linear-gradient(180deg, rgba(255,255,255,0.028) 0%, rgba(255,255,255,0.016) 100%)",
            border: "1px solid rgba(255,255,255,0.06)"
          }}
        >
          <Stack spacing={1.5}>
            <Stack
              direction={{ xs: "column", md: "row" }}
              justifyContent="space-between"
              alignItems={{ xs: "flex-start", md: "center" }}
              gap={1}
            >
              <Box>
                <Typography variant="h6" sx={{ fontWeight: 700 }}>
                  No running agents
                </Typography>
                <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5, maxWidth: 720 }}>
                  AgentArk only shows live agents here while they are actively running. Finished work moves into history,
                  and idle specialists stay hidden from this page.
                </Typography>
              </Box>
              <Button
                size="small"
                variant="outlined"
                onClick={() => setHistoryOpen(true)}
                sx={{ textTransform: "none" }}
              >
                View history
              </Button>
            </Stack>

            <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
              <Chip
                size="small"
                variant="outlined"
                label={`${completedCount} completed`}
                color={completedCount > 0 ? "success" : "default"}
              />
              <Chip
                size="small"
                variant="outlined"
                label={`${failedCount} failed`}
                color={failedCount > 0 ? "error" : "default"}
              />
              <Chip
                size="small"
                variant="outlined"
                label={`${cancelledCount} cancelled`}
                color={cancelledCount > 0 ? "warning" : "default"}
              />
            </Stack>

            <Box
              sx={{
                px: 1.25,
                py: 1,
                borderRadius: "10px",
                background: "rgba(47, 212, 255, 0.05)",
                border: "1px solid rgba(47, 212, 255, 0.12)"
              }}
            >
              <Typography variant="caption" sx={{ color: "text.secondary", display: "block" }}>
                Ask in chat for monitoring, escalation, deep research, or multi-step execution. AgentArk decides when
                specialist agents are actually needed instead of keeping idle workers around.
              </Typography>
            </Box>
          </Stack>
        </Box>
      ) : (
        <Stack spacing={1}>
          {runningAgents.map((agent) => (
            <Box
              key={agent.id}
              sx={{
                p: 1.5,
                borderRadius: "10px",
                background: "rgba(255,255,255,0.02)",
                border: "1px solid rgba(255,255,255,0.05)"
              }}
            >
              <Stack
                direction={{ xs: "column", md: "row" }}
                justifyContent="space-between"
                alignItems={{ xs: "flex-start", md: "center" }}
                gap={1}
              >
                <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                  <Typography variant="body2" sx={{ fontWeight: 700 }}>
                    {agent.name}
                  </Typography>
                  <Chip
                    size="small"
                    color={statusChipColor(agent.status)}
                    label={statusChipLabel(agent.status)}
                  />
                  <Typography variant="caption" color="text.secondary">
                    {agent.provider} / {agent.model}
                  </Typography>
                </Stack>
                <Stack direction="row" spacing={0.5} useFlexGap flexWrap="wrap">
                  {agent.capabilities.slice(0, 5).map((capability) => (
                    <Chip
                      key={`${agent.id}-${capability}`}
                      size="small"
                      variant="outlined"
                      label={capability}
                      sx={{ height: 20, fontSize: "0.65rem" }}
                    />
                  ))}
                </Stack>
              </Stack>
            </Box>
          ))}
        </Stack>
      )}

      <Dialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        maxWidth="md"
        fullWidth
        PaperProps={{
          sx: {
            background: "rgba(10, 15, 28, 0.97)",
            border: "1px solid rgba(47, 212, 255, 0.18)",
            backdropFilter: "blur(20px)"
          }
        }}
      >
        <DialogTitle>
          <Stack direction="row" justifyContent="space-between" alignItems="center">
            <Typography variant="h6" sx={{ fontWeight: 600 }}>
              Agent History
            </Typography>
            <IconButton size="small" onClick={() => setHistoryOpen(false)}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" sx={{ mb: historyItems.length > 0 ? 1.25 : 0 }}>
            <Chip
              size="small"
              variant="outlined"
              label={`${completedCount} completed`}
              color={completedCount > 0 ? "success" : "default"}
            />
            <Chip
              size="small"
              variant="outlined"
              label={`${failedCount} failed`}
              color={failedCount > 0 ? "error" : "default"}
            />
            <Chip
              size="small"
              variant="outlined"
              label={`${cancelledCount} cancelled`}
              color={cancelledCount > 0 ? "warning" : "default"}
            />
          </Stack>

          {historyItems.length === 0 ? (
            <Typography variant="body2" color="text.secondary" sx={{ py: 3, textAlign: "center" }}>
              No agent history recorded yet.
            </Typography>
          ) : (
            <Stack spacing={0} divider={<Box sx={{ borderBottom: "1px solid rgba(62,143,214,0.10)" }} />}>
              {historyItems.map((item) => (
                <Box key={item.id} sx={{ py: 1 }}>
                  <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                    <Chip
                      size="small"
                      color={statusChipColor(item.status)}
                      label={statusChipLabel(item.status)}
                      sx={{ height: 20, fontSize: "0.65rem" }}
                    />
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {item.agentName}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {item.triggerText} | {item.workText}
                    </Typography>
                    <Typography variant="caption" color="text.secondary" sx={{ ml: "auto !important" }}>
                      {formatTimestamp(item.timestamp)}
                    </Typography>
                  </Stack>
                  <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.25 }}>
                    {item.detail}
                  </Typography>
                </Box>
              ))}
            </Stack>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setHistoryOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}
