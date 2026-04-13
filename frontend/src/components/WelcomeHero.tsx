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
            "Give me my morning brief with weather, calendar, top tasks, and anything urgent.",
            "Remember that I prefer concise answers and send daily updates to Telegram.",
            "Watch my inbox for urgent client messages and alert me before I miss them.",
            "Draft a reply to this message and ask before sending it.",
            "Build a small app to track competitor launches and deploy it.",
          ],
    [prompts]
  );
  const [promptIndex, setPromptIndex] = useState(0);
  const [typedPrompt, setTypedPrompt] = useState("");
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(false);
  const promptSignature = heroPrompts.join("\n");
  const activePrompt = heroPrompts[promptIndex] || heroPrompts[0] || "";
  const displayPrompt = useMemo(() => {
    const trimmed = activePrompt.trim();
    return trimmed.length > 96 ? `${trimmed.slice(0, 93).trimEnd()}\u2026` : trimmed;
  }, [activePrompt]);
  const activeObjective = currentTaskDesc?.trim()
    ? currentTaskDesc.trim()
    : agentPaused
      ? "Background help is paused. Resume it when you want briefs, reminders, and automations to continue."
      : "No focus is pinned yet. Start with a question, a reminder, or your next daily brief.";
  const objectiveState = currentTaskDesc?.trim() ? "Active now" : agentPaused ? "Paused" : "Ready";

  useEffect(() => {
    setPromptIndex(0);
  }, [promptSignature]);

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) {
      return undefined;
    }

    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncPreference = () => {
      setPrefersReducedMotion(media.matches);
    };
    syncPreference();

    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", syncPreference);
      return () => media.removeEventListener("change", syncPreference);
    }

    media.addListener(syncPreference);
    return () => media.removeListener(syncPreference);
  }, []);

  useEffect(() => {
    setTypedPrompt(prefersReducedMotion ? displayPrompt : "");
  }, [displayPrompt, prefersReducedMotion]);

  useEffect(() => {
    if (!displayPrompt || typeof window === "undefined") {
      return undefined;
    }

    if (prefersReducedMotion) {
      if (heroPrompts.length <= 1) {
        return undefined;
      }

      const timer = window.setTimeout(() => {
        setPromptIndex((prev) => (prev + 1) % heroPrompts.length);
      }, 5600);

      return () => window.clearTimeout(timer);
    }

    if (typedPrompt.length < displayPrompt.length) {
      const nextChar = displayPrompt[typedPrompt.length];
      const delay = /[.,!?]/.test(nextChar) ? 48 : nextChar === " " ? 16 : 24;
      const timer = window.setTimeout(() => {
        setTypedPrompt(displayPrompt.slice(0, typedPrompt.length + 1));
      }, delay);

      return () => window.clearTimeout(timer);
    }

    if (heroPrompts.length <= 1) {
      return undefined;
    }

    const holdMs = Math.max(1800, Math.min(3200, displayPrompt.length * 28));
    const timer = window.setTimeout(() => {
      setPromptIndex((prev) => (prev + 1) % heroPrompts.length);
    }, holdMs);

    return () => window.clearTimeout(timer);
  }, [displayPrompt, heroPrompts.length, prefersReducedMotion, typedPrompt]);

  return (
    <Card
      className="welcome-hero-card mission-panel mission-panel--hero"
      sx={{
        height: "100%",
        borderRadius: 2,
        border: "1px solid rgba(255, 255, 255, 0.08)",
        background:
          "radial-gradient(circle at 18% 0%, rgba(255, 255, 255, 0.06), rgba(0,0,0,0) 34%)," +
          "linear-gradient(160deg, rgba(24, 24, 28, 0.98), rgba(15, 15, 18, 0.94))",
        boxShadow: "0 18px 34px rgba(0, 0, 0, 0.16)",
        overflow: "hidden",
      }}
    >
      <CardContent sx={{ p: { xs: 1.15, md: 1.4 }, position: "relative", height: "100%" }}>
        <Stack spacing={1.05} className="mission-panel-content" sx={{ position: "relative", zIndex: 1 }}>
          <Stack spacing={1} className="mission-panel-section">
            <Box className="welcome-hero-header">
              <Stack spacing={0.8} sx={{ minWidth: 0, flex: 1 }} className="welcome-hero-copy">
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{
                    alignItems: "center",
                    minWidth: 0
                  }}>
                  <Box
                    component="img"
                    src="/logo.svg"
                    alt="AgentArk"
                    sx={{
                      width: { xs: 44, md: 52 },
                      height: { xs: 44, md: 52 },
                      flexShrink: 0,
                      filter: "drop-shadow(0 0 14px rgba(255, 255, 255, 0.08))",
                    }}
                  />
                  <Box sx={{ minWidth: 0 }}>
                    <Typography
                      variant="overline"
                      sx={{ color: "rgba(183, 188, 196, 0.68)", letterSpacing: 0, display: "block", lineHeight: 1 }}
                    >
                      AgentArk | Secure Daily Assistant
                    </Typography>
                    <Typography
                      variant="h5"
                      sx={{
                        fontWeight: 700,
                        lineHeight: 1.08,
                        letterSpacing: 0,
                        fontSize: { xs: "1.32rem", md: "1.52rem" },
                      }}
                    >
                      Your secure daily AI assistant, ready before you ask.
                    </Typography>
                  </Box>
                </Stack>
                <Typography
                  variant="body2"
                  className="mission-card-copy"
                  title="Private by default, useful every day: memory, daily briefings, safe actions, and deeper automation when you want it."
                  sx={{
                    color: "text.secondary"
                  }}
                >
                  Private by default, useful every day: memory, daily briefings, safe actions, and deeper automation when you want it.
                </Typography>
              </Stack>

              <Stack direction="row" spacing={0.75} useFlexGap className="welcome-hero-status-row" sx={{
                flexWrap: "wrap"
              }}>
                <Chip
                  size="small"
                  color={agentPaused ? "warning" : "success"}
                  label={agentPaused ? "Background Help Paused" : "Background Help On"}
                />
                <Chip size="small" label="Secure first" />
              </Stack>
            </Box>

            <Box className="welcome-hero-command-deck">
              <Box className="welcome-hero-command-row">
                <Box className="welcome-hero-command-meta">
                  <Typography variant="caption" className="welcome-hero-command-label">
                    Current Focus
                  </Typography>
                  <Typography variant="caption" className="welcome-hero-command-state">
                    {objectiveState}
                  </Typography>
                </Box>
                <Typography
                  variant="body2"
                  title={activeObjective}
                  className="welcome-hero-command-body welcome-hero-command-body--objective"
                >
                  {activeObjective}
                </Typography>
              </Box>

              <Box className="welcome-hero-command-divider" />

              <Box className="welcome-hero-command-row welcome-hero-command-row--directive">
                <Box className="welcome-hero-command-meta">
                  <Typography variant="caption" className="welcome-hero-command-label">
                    Suggested Next Step
                  </Typography>
                  <Typography variant="caption" className="welcome-hero-command-state">
                    Daily use
                  </Typography>
                </Box>
                <Box className="welcome-hero-command-body welcome-hero-command-body--directive">
                  <Box className="welcome-hero-typewriter-stage" title={activePrompt}>
                    <Box component="span" className="welcome-hero-typewriter-text">
                      {prefersReducedMotion ? displayPrompt : typedPrompt}
                    </Box>
                    {!prefersReducedMotion ? (
                      <Box component="span" aria-hidden className="welcome-hero-typewriter-caret" />
                    ) : null}
                  </Box>
                  <Typography variant="caption" className="welcome-hero-command-caption">
                    Rotates through useful assistant tasks from your routines, recent work, and unattended runs.
                  </Typography>
                </Box>
              </Box>
            </Box>
          </Stack>

          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={0.8}
            className="mission-panel-footer welcome-hero-footer"
            sx={{ width: { xs: "100%", sm: "auto" } }}
          >
            {onGoChat ? (
              <Button
                size="small"
                variant="contained"
                startIcon={<ChatRoundedIcon />}
                onClick={onGoChat}
              >
                Ask AgentArk
              </Button>
            ) : null}
            {onRunBriefing ? (
              <Button
                size="small"
                variant="outlined"
                startIcon={<AutoAwesomeRoundedIcon />}
                onClick={onRunBriefing}
                disabled={briefingLoading}
              >
                {briefingLoading ? "Running..." : "Generate Daily Brief"}
              </Button>
            ) : null}
            {onViewTasks ? (
              <Button
                size="small"
                variant="outlined"
                startIcon={<ListAltRoundedIcon />}
                onClick={onViewTasks}
              >
                Review Tasks
              </Button>
            ) : null}
            {onTogglePause ? (
              <Button
                size="small"
                variant="outlined"
                startIcon={agentPaused ? <PlayCircleOutlineRoundedIcon /> : <PauseCircleOutlineRoundedIcon />}
                onClick={onTogglePause}
                disabled={pauseLoading}
              >
                {agentPaused ? "Resume Background Help" : "Pause Background Help"}
              </Button>
            ) : null}
          </Stack>
        </Stack>
      </CardContent>
    </Card>
  );
}
