import { Box, Stack, Typography } from "@mui/material";

type Props = {
  serverStatus?: { at: number; rtt_ms: number; status: import("../types").StatusResponse };
  serverError: boolean;
  serverLoading: boolean;
  currentTaskDesc?: string;
};

export function AgentStatusBar({ serverStatus, serverError, serverLoading, currentTaskDesc }: Props) {
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

  return (
    <Box
      className="status-bar"
      sx={{
        display: "flex",
        alignItems: { xs: "flex-start", sm: "center" },
        justifyContent: "space-between",
        flexDirection: { xs: "column", sm: "row" },
        gap: { xs: 1, sm: 1.5 },
        minHeight: 48,
        px: 2,
        py: 1,
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1.25}>
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
        <Typography variant="body2" fontWeight={600} sx={{ color: "rgba(195, 221, 252, 0.95)" }}>
          {label}
        </Typography>
      </Stack>

      {status ? (
        <Stack direction="row" spacing={2} useFlexGap flexWrap="wrap">
          <Typography variant="caption" color="text.secondary">
            {status.memory_entries} memories
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {status.skills_loaded ?? status.actions_loaded ?? 0} skills
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {status.tasks_pending} pending
          </Typography>
        </Stack>
      ) : null}
    </Box>
  );
}
