import { Box, Button, Card, CardContent, Chip, Stack, Typography } from "@mui/material";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import PauseCircleOutlineRoundedIcon from "@mui/icons-material/PauseCircleOutlineRounded";
import PlayCircleOutlineRoundedIcon from "@mui/icons-material/PlayCircleOutlineRounded";
import ListAltRoundedIcon from "@mui/icons-material/ListAltRounded";
import { useMemo } from "react";

type Props = {
  onGoChat?: () => void;
  onRunBriefing?: () => void;
  onViewTasks?: () => void;
  onTogglePause?: () => void;
  agentPaused?: boolean;
  briefingLoading?: boolean;
  pauseLoading?: boolean;
};

export function WelcomeHero({
  onGoChat,
  onRunBriefing,
  onViewTasks,
  onTogglePause,
  agentPaused = false,
  briefingLoading = false,
  pauseLoading = false,
}: Props) {
  const greeting = useMemo(() => {
    const h = new Date().getHours();
    if (h < 5) return "Welcome back";
    if (h < 12) return "Good morning";
    if (h < 18) return "Good afternoon";
    return "Good evening";
  }, []);

  return (
    <Card
      className="welcome-hero-card"
      sx={{
        borderRadius: 5,
        border: "1px solid rgba(108, 156, 212, 0.18)",
        background:
          "radial-gradient(circle at 50% 0%, rgba(47, 212, 255, 0.2), rgba(0,0,0,0) 40%)," +
          "linear-gradient(160deg, rgba(9, 21, 39, 0.97), rgba(8, 18, 33, 0.78))",
        boxShadow: "0 28px 60px rgba(0, 0, 0, 0.24)",
        overflow: "hidden",
      }}
    >
      <CardContent sx={{ p: { xs: 2.25, md: 3.5 }, textAlign: { xs: "left", md: "center" }, position: "relative" }}>
        <Box className="welcome-hero-watermark">A</Box>
        <Stack spacing={{ xs: 1.5, md: 2 }} alignItems={{ xs: "flex-start", md: "center" }} sx={{ position: "relative", zIndex: 1 }}>
          <Box
            component="img"
            src="/logo.svg"
            alt="AgentArk"
            sx={{
              width: { xs: 60, md: 72 },
              height: { xs: 60, md: 72 },
              flexShrink: 0,
              filter: "drop-shadow(0 0 18px rgba(47, 212, 255, 0.26))"
            }}
          />
          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" justifyContent="center">
            <Chip size="small" color={agentPaused ? "warning" : "success"} label={agentPaused ? "Autonomy Paused" : "Autonomy Active"} />
            <Chip size="small" label="Chat-first workspace" />
            <Chip size="small" label="Projects, tools, traces" />
          </Stack>
          <Box sx={{ maxWidth: 820 }}>
            <Typography
              variant="h2"
              sx={{
                fontWeight: 700,
                lineHeight: 1.08,
                letterSpacing: "-0.04em",
                fontSize: { xs: "2.2rem", md: "3.5rem" }
              }}
            >
              {greeting}. What should AgentArk handle next?
            </Typography>
            <Typography variant="body1" color="text.secondary" sx={{ mt: 1.1, maxWidth: 660, mx: { md: "auto" } }}>
              Describe the result once. AgentArk keeps the active task centered, while projects, tools, and automation stay one click away instead of competing for space.
            </Typography>
            <Typography
              variant="body2"
              sx={{
                mt: 1.2,
                color: "rgba(196, 230, 255, 0.96)",
                px: 1.25,
                py: 0.9,
                borderRadius: 999,
                display: "inline-flex",
                border: "1px solid rgba(108, 156, 212, 0.22)",
                background: "rgba(8, 19, 34, 0.58)"
              }}
            >
              Try: "Review recent changes and list only the critical risks."
            </Typography>
          </Box>
          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ width: { xs: "100%", sm: "auto" } }}>
            {onGoChat ? (
              <Button
                size="large"
                variant="contained"
                startIcon={<ChatRoundedIcon />}
                onClick={onGoChat}
                sx={{ borderRadius: 999, px: 2.5, textTransform: "none" }}
              >
                Open Chat
              </Button>
            ) : null}
            {onRunBriefing ? (
              <Button
                size="large"
                variant="outlined"
                startIcon={<AutoAwesomeRoundedIcon />}
                onClick={onRunBriefing}
                disabled={briefingLoading}
                sx={{ borderRadius: 999, px: 2.3, textTransform: "none" }}
              >
                {briefingLoading ? "Running..." : "Run Briefing"}
              </Button>
            ) : null}
            {onViewTasks ? (
              <Button
                size="large"
                variant="outlined"
                startIcon={<ListAltRoundedIcon />}
                onClick={onViewTasks}
                sx={{ borderRadius: 999, px: 2.3, textTransform: "none" }}
              >
                View Tasks
              </Button>
            ) : null}
            {onTogglePause ? (
              <Button
                size="large"
                variant="text"
                startIcon={agentPaused ? <PlayCircleOutlineRoundedIcon /> : <PauseCircleOutlineRoundedIcon />}
                onClick={onTogglePause}
                disabled={pauseLoading}
                sx={{ borderRadius: 999, px: 1.5, textTransform: "none" }}
              >
                {agentPaused ? "Resume Autonomy" : "Pause Autonomy"}
              </Button>
            ) : null}
          </Stack>
        </Stack>
      </CardContent>
    </Card>
  );
}
