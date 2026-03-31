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
      dotColor = "#2fd4ff";
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
  const postureItems = [
    {
      label: "Autonomy",
      value: agentPaused ? "Paused" : "Active",
      detail: agentPaused ? "Background execution suspended" : "Background execution enabled",
      tone: agentPaused ? "#ffb84d" : "#74f7bf",
    },
    {
      label: "Model",
      value: hasLlmConfigured ? "Configured" : "Needs setup",
      detail: hasLlmConfigured ? "Primary reasoning stack ready" : "Add an LLM in Settings",
      tone: hasLlmConfigured ? "#84d8ff" : "#ff8f8f",
    },
    {
      label: "Queue",
      value: String(status?.tasks_pending ?? 0),
      detail: `${status?.tasks_pending ?? 0} pending task${(status?.tasks_pending ?? 0) === 1 ? "" : "s"}`,
      tone: (status?.tasks_pending ?? 0) > 0 ? "#ffd27c" : "#9fb6cf",
    },
    {
      label: "Runtime",
      value: String(runtimeCount),
      detail: summarizeRuntime(automationCounts),
      tone: runtimeCount > 0 ? "#84d8ff" : "#9fb6cf",
    },
  ];

  return (
    <Box
      className="status-bar mission-panel mission-panel--adaptive"
      sx={{
        display: "flex",
        flexDirection: "column",
        gap: 1.2,
        px: { xs: 1.35, md: 1.5 },
        py: { xs: 1.2, md: 1.35 },
      }}
    >
      <Stack spacing={0.45}>
        <Typography
          variant="overline"
          sx={{ color: "rgba(142, 191, 234, 0.74)", letterSpacing: "0.12em", display: "block" }}
        >
          System Posture
        </Typography>
        <Stack direction="row" alignItems="center" spacing={1}>
          <Box
            sx={{
              width: 10,
              height: 10,
              borderRadius: "50%",
              backgroundColor: dotColor,
              boxShadow: pulse ? `0 0 8px 2px ${dotColor}` : "none",
              animation: pulse ? "pulse-dot 2s ease-in-out infinite" : "none",
              flexShrink: 0,
              "@keyframes pulse-dot": {
                "0%, 100%": { boxShadow: `0 0 4px 1px ${dotColor}` },
                "50%": { boxShadow: `0 0 10px 4px ${dotColor}` },
              },
            }}
          />
          <Typography variant="subtitle1" sx={{ color: "rgba(232, 243, 255, 0.96)", fontWeight: 600 }}>
            {label}
          </Typography>
        </Stack>
        <Typography variant="body2" color="text.secondary">
          Live operator summary for autonomy, model readiness, queue pressure, and runtime posture.
        </Typography>
      </Stack>

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 1,
        }}
      >
        {postureItems.map((item) => (
          <Box
            key={item.label}
            sx={{
              borderRadius: 2.5,
              border: "1px solid rgba(108, 156, 212, 0.16)",
              background: "linear-gradient(180deg, rgba(8, 18, 34, 0.78), rgba(6, 14, 28, 0.68))",
              px: 1.1,
              py: 0.95,
              minWidth: 0,
            }}
          >
            <Typography
              variant="caption"
              sx={{ color: "rgba(138, 177, 212, 0.68)", textTransform: "uppercase", letterSpacing: "0.08em" }}
            >
              {item.label}
            </Typography>
            <Typography variant="subtitle2" sx={{ mt: 0.25, color: item.tone, fontWeight: 700 }}>
              {item.value}
            </Typography>
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.2 }}>
              {item.detail}
            </Typography>
          </Box>
        ))}
      </Box>

      <Stack spacing={0.7} sx={{ pt: 0.2 }}>
        {currentTaskDesc ? (
          <Box
            sx={{
              borderRadius: 2.5,
              border: "1px solid rgba(74, 195, 255, 0.18)",
              background: "rgba(9, 22, 40, 0.62)",
              px: 1.1,
              py: 0.95,
            }}
          >
            <Typography variant="caption" sx={{ color: "rgba(137, 213, 255, 0.8)", letterSpacing: "0.08em", textTransform: "uppercase" }}>
              Active Objective
            </Typography>
            <Typography variant="body2" sx={{ mt: 0.35, color: "rgba(225, 239, 255, 0.96)", fontWeight: 600 }}>
              {currentTaskDesc}
            </Typography>
          </Box>
        ) : null}
        <Stack direction="row" spacing={1.2} useFlexGap flexWrap="wrap">
          {status ? (
            <>
              <Typography variant="caption" color="text.secondary">
                {status.memory_entries} memories
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {status.skills_loaded ?? status.actions_loaded ?? 0} skills
              </Typography>
              <Typography variant="caption" color="text.secondary">
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
