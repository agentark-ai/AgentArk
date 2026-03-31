import { Box, Button, Card, CardContent, Chip, Stack, Typography } from "@mui/material";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import PauseCircleOutlineRoundedIcon from "@mui/icons-material/PauseCircleOutlineRounded";
import PlayCircleOutlineRoundedIcon from "@mui/icons-material/PlayCircleOutlineRounded";
import ListAltRoundedIcon from "@mui/icons-material/ListAltRounded";
import { useEffect, useMemo, useState } from "react";

type Props = {
  onGoChat?: () => void;
  onRunBriefing?: () => void;
  onViewTasks?: () => void;
  onTogglePause?: () => void;
  agentPaused?: boolean;
  briefingLoading?: boolean;
  pauseLoading?: boolean;
  prompts?: string[];
  currentTaskDesc?: string;
};

export function WelcomeHero({
  onGoChat,
  onRunBriefing,
  onViewTasks,
  onTogglePause,
  agentPaused = false,
  briefingLoading = false,
  pauseLoading = false,
  prompts,
  currentTaskDesc,
}: Props) {
  const heroPrompts = useMemo(
    () =>
      prompts && prompts.length > 0
        ? prompts
        : [
            "Review recent changes and list only the critical risks.",
            "Build a small app to track competitor launches and deploy it.",
            "Import this skill URL and wire up any required secrets.",
            "Summarize the current project state and name the next decision.",
            "Inspect active automations and surface anything that needs intervention.",
          ],
    [prompts]
  );
  const [promptIndex, setPromptIndex] = useState(0);
  const promptSignature = heroPrompts.join("\n");
  const activePrompt = heroPrompts[promptIndex] || heroPrompts[0] || "";
  const activeObjective = currentTaskDesc?.trim()
    ? currentTaskDesc.trim()
    : agentPaused
      ? "Autonomy is paused. Resume it when you want background tasks and watchers to continue."
      : "No active objective is pinned. Mission Control is ready for a new directive.";

  useEffect(() => {
    setPromptIndex(0);
  }, [promptSignature]);

  useEffect(() => {
    if (heroPrompts.length <= 1 || typeof window === "undefined") {
      return undefined;
    }

    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const rotateEveryMs = reduceMotion ? 6400 : 4600;
    const timer = window.setInterval(() => {
      setPromptIndex((prev) => (prev + 1) % heroPrompts.length);
    }, rotateEveryMs);

    return () => window.clearInterval(timer);
  }, [heroPrompts]);

  return (
    <Card
      className="welcome-hero-card mission-panel mission-panel--hero"
      sx={{
        height: "100%",
        borderRadius: 4,
        border: "1px solid rgba(108, 156, 212, 0.18)",
        background:
          "radial-gradient(circle at 18% 0%, rgba(47, 212, 255, 0.18), rgba(0,0,0,0) 34%)," +
          "linear-gradient(160deg, rgba(9, 21, 39, 0.97), rgba(8, 18, 33, 0.84))",
        boxShadow: "0 22px 44px rgba(0, 0, 0, 0.22)",
        overflow: "hidden",
      }}
    >
      <CardContent sx={{ p: { xs: 1.35, md: 1.55 }, position: "relative", height: "100%" }}>
        <Stack spacing={1.15} className="mission-panel-content" sx={{ position: "relative", zIndex: 1 }}>
          <Stack spacing={1.15} className="mission-panel-section">
            <Stack
              direction={{ xs: "column", md: "row" }}
              spacing={1.2}
              justifyContent="space-between"
              alignItems={{ xs: "flex-start", md: "flex-start" }}
            >
              <Stack spacing={0.95} sx={{ minWidth: 0, flex: 1 }}>
                <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
                  <Box
                    component="img"
                    src="/logo.svg"
                    alt="AgentArk"
                    sx={{
                      width: { xs: 40, md: 46 },
                      height: { xs: 40, md: 46 },
                      flexShrink: 0,
                      filter: "drop-shadow(0 0 14px rgba(47, 212, 255, 0.22))",
                    }}
                  />
                  <Box sx={{ minWidth: 0 }}>
                    <Typography
                      variant="overline"
                      sx={{ color: "rgba(142, 191, 234, 0.74)", letterSpacing: "0.12em", display: "block" }}
                    >
                      Mission Control
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700, lineHeight: 1.1, letterSpacing: "-0.03em" }}>
                      Direct the agent from outcomes, not menus.
                    </Typography>
                  </Box>
                </Stack>
                <Typography variant="body2" color="text.secondary" className="mission-card-copy">
                  This surface should tell you what matters, what the system is doing, and what high-confidence move to make next without drowning you in dashboard chrome.
                </Typography>
              </Stack>

              <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                <Chip size="small" color={agentPaused ? "warning" : "success"} label={agentPaused ? "Autonomy Paused" : "Autonomy Active"} />
                <Chip size="small" label="Outcome-first" />
                <Chip size="small" label="Operator cockpit" />
              </Stack>
            </Stack>

            <Box
              sx={{
                borderRadius: 3,
                border: "1px solid rgba(108, 156, 212, 0.18)",
                background: "rgba(7, 18, 32, 0.58)",
                px: 1.15,
                py: 0.95,
              }}
            >
              <Typography
                variant="caption"
                sx={{ color: "rgba(137, 213, 255, 0.8)", letterSpacing: "0.08em", textTransform: "uppercase" }}
              >
                Active Objective
              </Typography>
              <Typography variant="body2" sx={{ mt: 0.35, color: "rgba(225, 239, 255, 0.96)", fontWeight: 600 }}>
                {activeObjective}
              </Typography>
            </Box>

            <Box
              sx={{
                borderRadius: 3,
                border: "1px solid rgba(108, 156, 212, 0.22)",
                background: "rgba(8, 19, 34, 0.58)",
                px: 1.05,
                py: 0.85,
                display: "grid",
                gridTemplateColumns: { xs: "1fr", md: "auto minmax(0, 1fr)" },
                alignItems: "center",
                columnGap: 0.8,
                rowGap: 0.45,
                maxWidth: "100%",
                minWidth: 0,
                overflow: "hidden",
                minHeight: 52,
              }}
            >
              <Typography
                variant="caption"
                sx={{
                  color: "rgba(137, 213, 255, 0.8)",
                  letterSpacing: "0.08em",
                  textTransform: "uppercase",
                  flexShrink: 0,
                }}
              >
                Suggested directive
              </Typography>
              <Box
                sx={{
                  display: "flex",
                  alignItems: "center",
                  minWidth: 0,
                  flex: 1,
                  color: "rgba(196, 230, 255, 0.96)",
                  fontSize: "0.88rem",
                }}
              >
                <Box
                  component="span"
                  key={`${promptIndex}-${activePrompt}`}
                  title={activePrompt}
                  className="welcome-hero-directive"
                  sx={{
                    minWidth: 0,
                    display: "block",
                    overflow: "hidden",
                    whiteSpace: "nowrap",
                    textOverflow: "ellipsis",
                  }}
                >
                  {activePrompt}
                </Box>
              </Box>
            </Box>
          </Stack>

          <Stack direction={{ xs: "column", sm: "row" }} spacing={0.85} className="mission-panel-footer" sx={{ width: { xs: "100%", sm: "auto" } }}>
            {onGoChat ? (
              <Button
                size="medium"
                variant="contained"
                startIcon={<ChatRoundedIcon />}
                onClick={onGoChat}
              >
                Open Chat
              </Button>
            ) : null}
            {onRunBriefing ? (
              <Button
                size="medium"
                variant="outlined"
                startIcon={<AutoAwesomeRoundedIcon />}
                onClick={onRunBriefing}
                disabled={briefingLoading}
              >
                {briefingLoading ? "Running..." : "Run Daily Brief"}
              </Button>
            ) : null}
            {onViewTasks ? (
              <Button
                size="medium"
                variant="outlined"
                startIcon={<ListAltRoundedIcon />}
                onClick={onViewTasks}
              >
                Open Task Queue
              </Button>
            ) : null}
            {onTogglePause ? (
              <Button
                size="medium"
                variant="outlined"
                startIcon={agentPaused ? <PlayCircleOutlineRoundedIcon /> : <PauseCircleOutlineRoundedIcon />}
                onClick={onTogglePause}
                disabled={pauseLoading}
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
