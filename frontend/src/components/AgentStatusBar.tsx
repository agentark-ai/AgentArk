import { Box, Stack, Typography } from "@mui/material";

type AutomationCounts = {
  tasks: number;
  watchers: number;
  apps: number;
  integrations: number;
};

type Props = {
  serverStatus?: { at: number; rtt_ms: number; status: import("../types").StatusResponse };
  serverError: boolean;
  serverLoading: boolean;
  currentTaskDesc?: string;
  agentPaused?: boolean;
  hasLlmConfigured?: boolean;
  automationCounts?: AutomationCounts;
  recentFailureTitle?: string | null;
};

function summarizeRuntime(automationCounts?: AutomationCounts): string {
  if (!automationCounts) return "No runtime inventory yet";
  const total =
    automationCounts.tasks +
    automationCounts.watchers +
    automationCounts.apps +
    automationCounts.integrations;
  if (total <= 0) return "No active automation surfaces";
  return `${total} runtime surfaces`;
}

export function AgentStatusBar({
  serverStatus,
  serverError,
  serverLoading,
  currentTaskDesc,
  agentPaused = false,
  hasLlmConfigured = true,
  automationCounts,
  recentFailureTitle,
}: Props) {
  let dotColor = "#ff9800";
  let label = "Connecting...";
  let pulse = false;

  if (serverError) {
    dotColor = "#f44336";
    label = "Offline";
  } else if (serverStatus) {
    if (currentTaskDesc) {
      dotColor = "#d5d9e1";
      label = `Working: ${currentTaskDesc.length > 60 ? currentTaskDesc.slice(0, 57) + "..." : currentTaskDesc}`;
      pulse = true;
    } else {
      dotColor = "#4caf50";
      label = "Idle \u2022 Ready";
      pulse = true;
    }
  } else if (serverLoading) {
    dotColor = "#ff9800";
    label = "Connecting...";
  }

  const status = serverStatus?.status;
  const runtimeCount =
    (automationCounts?.tasks || 0) +
    (automationCounts?.watchers || 0) +
    (automationCounts?.apps || 0) +
    (automationCounts?.integrations || 0);
  const pendingCount = status?.tasks_pending ?? 0;
  const postureItems = [
    {
      label: "Autonomy",
      value: agentPaused ? "Paused" : "Active",
      detail: agentPaused
        ? "Background autonomy paused; scheduled reminders still fire"
        : "Background execution enabled",
      tone: agentPaused ? "#ffb84d" : "#74f7bf",
    },
    {
      label: "Model",
      value: hasLlmConfigured ? "Configured" : "Needs setup",
      detail: hasLlmConfigured ? "Primary reasoning stack ready" : "Add an LLM in Settings",
      tone: hasLlmConfigured ? "#d7dae1" : "#ff8f8f",
    },
    {
      label: "Runtime",
      value: String(runtimeCount),
      detail:
        pendingCount > 0
          ? `${summarizeRuntime(automationCounts)} \u2022 ${pendingCount} pending`
          : summarizeRuntime(automationCounts),
      tone: runtimeCount > 0 || pendingCount > 0 ? "#d7dae1" : "#9fa6b2",
    },
  ];

  return (
    <Box
      className="status-bar mission-panel mission-panel--adaptive mission-side-panel"
      sx={{
        display: "flex",
        flexDirection: "column",
        gap: 0.9,
        px: { xs: 1.15, md: 1.25 },
        py: { xs: 1.0, md: 1.1 },
      }}
    >
      <Stack
        direction="row"
        spacing={1}
        sx={{
          justifyContent: "space-between",
          alignItems: "flex-start"
        }}>
        <Stack spacing={0.45} sx={{ minWidth: 0, flex: 1 }}>
          <Typography
            variant="overline"
            sx={{ color: "rgba(183, 188, 196, 0.68)", letterSpacing: 0, display: "block" }}
          >
            System Posture
          </Typography>
          <Stack direction="row" spacing={1} sx={{
            alignItems: "center"
          }}>
            <Box
              className={pulse ? "status-dot status-dot--pulse" : "status-dot"}
              style={{
                backgroundColor: dotColor,
                boxShadow: pulse ? `0 0 6px 1px ${dotColor}` : "none",
              }}
            />
            <Typography variant="subtitle2" sx={{ color: "rgba(244, 245, 247, 0.96)", fontWeight: 700 }}>
              {label}
            </Typography>
          </Stack>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              lineHeight: 1.45
            }}>
            Live reasoning posture, queue pressure, model readiness, and runtime health.
          </Typography>
        </Stack>
        <Box
          sx={{
            px: 1,
            py: 0.5,
            borderRadius: 999,
            border: "1px solid rgba(255, 255, 255, 0.08)",
            background: "rgba(255, 255, 255, 0.03)",
            color: agentPaused ? "#ffbc7c" : "#82f7c1",
            fontSize: "0.66rem",
            fontWeight: 700,
            letterSpacing: 0,
            textTransform: "uppercase",
            flexShrink: 0,
          }}
        >
          {agentPaused ? "Paused" : "Ready"}
        </Box>
      </Stack>
      <Stack spacing={0.75}>
        {postureItems.map((item) => (
          <Stack
            key={item.label}
            direction="row"
            spacing={1}
            className="mission-metric-card mission-metric-card--rail"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              px: 1.1,
              py: 0.75
            }}>
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <Typography
                variant="caption"
                className="mission-metric-card__label"
              >
                {item.label}
              </Typography>
              <Typography variant="caption" className="mission-metric-card__detail" sx={{ display: "block", mt: 0.2 }}>
                {item.detail}
              </Typography>
            </Box>
            <Typography
              variant="caption"
              className="mission-metric-card__value"
              sx={{ color: item.tone, textAlign: "right", fontSize: "0.84rem" }}
            >
              {item.value}
            </Typography>
          </Stack>
        ))}
      </Stack>
      <Stack spacing={0.7} sx={{ pt: 0.2 }}>
        <Stack direction="row" spacing={1.2} useFlexGap sx={{
          flexWrap: "wrap"
        }}>
          {status ? (
            <>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {status.memory_entries} memories
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {status.skills_loaded ?? status.actions_loaded ?? 0} skills
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                RTT {serverStatus?.rtt_ms ?? "-"}ms
              </Typography>
            </>
          ) : null}
        </Stack>
        {recentFailureTitle ? (
          <Typography variant="caption" sx={{ color: "rgba(255, 177, 177, 0.82)" }}>
            Latest degraded run: {recentFailureTitle}
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
}
