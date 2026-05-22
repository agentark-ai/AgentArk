import { Box, Button, Card, Stack, Typography } from "@mui/material";
import AssignmentTurnedInRoundedIcon from "@mui/icons-material/AssignmentTurnedInRounded";
import ScienceRoundedIcon from "@mui/icons-material/ScienceRounded";
import HistoryRoundedIcon from "@mui/icons-material/HistoryRounded";
import PowerSettingsNewRoundedIcon from "@mui/icons-material/PowerSettingsNewRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import ExpandLessRoundedIcon from "@mui/icons-material/ExpandLessRounded";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import { useMemo } from "react";

export type EvolveHeroProps = {
  loading: boolean;
  // The plain-English summary the page already computes. We never compose
  // our own routing-style copy here — the page owns the narrative; the
  // hero owns the presentation.
  title: string;
  detail: string;
  // Numeric inputs. The hero picks ONE of these as the headline number
  // based on priority: pending reviews > live tests > steady state.
  needsApprovalCount: number;
  activeTests: number;
  rollbackAvailableCount: number;
  selfEvolveEnabled: boolean;
  showDetails: boolean;
  onToggleDetails: () => void;
  // Optional CTA handlers — the hero hides any card whose handler is
  // missing rather than render an inert button.
  onOpenReviewQueue?: () => void;
  onOpenLiveTests?: () => void;
  onOpenRollback?: () => void;
};

type Headline = {
  value: string;
  unitLabel: string;
  caption: string;
  // Visual treatment. `attention` = warm orange-ish glow for things that
  // wait on the user. `live` = info glow for active tests. `steady` =
  // green glow for a healthy idle state. `paused` = neutral, no glow.
  tone: "attention" | "live" | "steady" | "paused";
};

function headlineFromCounts(props: EvolveHeroProps): Headline {
  if (props.needsApprovalCount > 0) {
    return {
      value: String(props.needsApprovalCount),
      unitLabel:
        props.needsApprovalCount === 1
          ? "suggestion needs review"
          : "suggestions need review",
      caption: "ON YOU",
      tone: "attention",
    };
  }
  if (props.activeTests > 0) {
    return {
      value: String(props.activeTests),
      unitLabel: props.activeTests === 1 ? "live test running" : "live tests running",
      caption: "IN PROGRESS",
      tone: "live",
    };
  }
  if (!props.selfEvolveEnabled) {
    return {
      value: "Off",
      unitLabel: "Self-evolve is paused",
      caption: "OPT-IN ANYTIME",
      tone: "paused",
    };
  }
  return {
    value: "Steady",
    unitLabel: "AgentArk is watching for improvements",
    caption: "NOTHING NEEDS YOU",
    tone: "steady",
  };
}

type Moment = {
  id: string;
  icon: React.ElementType;
  accent: string;
  title: string;
  sentence: string;
  cta?: { label: string; onClick: () => void; primary?: boolean };
};

function buildMoments(props: EvolveHeroProps): Moment[] {
  const moments: Moment[] = [];
  if (props.needsApprovalCount > 0) {
    moments.push({
      id: "review",
      icon: AssignmentTurnedInRoundedIcon,
      accent: "rgba(255, 190, 99, 0.9)",
      title:
        props.needsApprovalCount === 1
          ? "A suggestion is waiting for you"
          : `${props.needsApprovalCount} suggestions are waiting`,
      sentence:
        "Nothing has actually changed yet. Decide whether Evolve should try them out.",
      cta: props.onOpenReviewQueue
        ? {
            label: "Open review",
            onClick: props.onOpenReviewQueue,
            primary: true,
          }
        : undefined,
    });
  }
  if (props.activeTests > 0) {
    moments.push({
      id: "live",
      icon: ScienceRoundedIcon,
      accent: "rgba(120, 242, 176, 0.9)",
      title:
        props.activeTests === 1
          ? "One live test is running"
          : `${props.activeTests} live tests are running`,
      sentence:
        "Evolve is trying small changes on a slice of traffic. You can view, stop, or make them stable.",
      cta: props.onOpenLiveTests
        ? { label: "View tests", onClick: props.onOpenLiveTests }
        : undefined,
    });
  }
  if (props.rollbackAvailableCount > 0) {
    moments.push({
      id: "rollback",
      icon: HistoryRoundedIcon,
      accent: "rgba(213, 145, 255, 0.85)",
      title:
        props.rollbackAvailableCount === 1
          ? "A stable change can be rolled back"
          : `${props.rollbackAvailableCount} stable changes can be rolled back`,
      sentence:
        "If a recent change isn't working for you, you can return to the previous behavior.",
      cta: props.onOpenRollback
        ? { label: "See options", onClick: props.onOpenRollback }
        : undefined,
    });
  }
  moments.push({
    id: "self_evolve",
    icon: PowerSettingsNewRoundedIcon,
    accent: props.selfEvolveEnabled
      ? "rgba(120, 242, 176, 0.9)"
      : "rgba(213, 228, 255, 0.5)",
    title: props.selfEvolveEnabled
      ? "Self-evolve is on"
      : "Self-evolve is paused",
    sentence: props.selfEvolveEnabled
      ? "AgentArk learns from completed work and asks before lasting changes."
      : "Turn it on in settings whenever you want AgentArk to start learning again.",
  });
  return moments.slice(0, 4);
}

export default function EvolveHero(props: EvolveHeroProps) {
  const headline = useMemo(() => headlineFromCounts(props), [props]);
  const moments = useMemo(() => buildMoments(props), [props]);

  return (
    <Card
      sx={{
        position: "relative",
        overflow: "hidden",
        p: { xs: 2.5, md: 3.5 },
        background:
          "radial-gradient(120% 80% at 0% 0%, rgba(120, 242, 176, 0.04) 0%, transparent 60%), var(--surface-bg-elevated)",
        border: "1px solid var(--surface-border)",
        borderRadius: 2,
        animation: "evolveHeroFadeIn 320ms ease-out",
        "@keyframes evolveHeroFadeIn": {
          from: { opacity: 0, transform: "translateY(8px)" },
          to: { opacity: 1, transform: "translateY(0)" },
        },
      }}
    >
      {props.loading ? <HeroSkeleton /> : <HeroBody headline={headline} moments={moments} {...props} />}
    </Card>
  );
}

function HeroSkeleton() {
  return (
    <Stack spacing={2.5}>
      <Box sx={{ height: 12, width: 96, background: "rgba(255,255,255,0.04)", borderRadius: 999 }} />
      <Box sx={{ height: 28, width: "55%", background: "rgba(255,255,255,0.04)", borderRadius: 8 }} />
      <Box sx={{ height: 56, width: 168, background: "rgba(255,255,255,0.04)", borderRadius: 8 }} />
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "1fr", sm: "repeat(2, 1fr)", lg: "repeat(4, 1fr)" },
          gap: 1.4,
        }}
      >
        {[0, 1, 2, 3].map((index) => (
          <Box key={index} sx={{ height: 110, background: "rgba(255,255,255,0.03)", borderRadius: 10 }} />
        ))}
      </Box>
    </Stack>
  );
}

type HeroBodyProps = EvolveHeroProps & { headline: Headline; moments: Moment[] };

function HeroBody({ title, detail, headline, moments, showDetails, onToggleDetails }: HeroBodyProps) {
  return (
    <Stack spacing={{ xs: 2.4, md: 3 }}>
      <Stack
        direction={{ xs: "column", lg: "row" }}
        spacing={{ xs: 2, lg: 4 }}
        sx={{ alignItems: { xs: "flex-start", lg: "center" } }}
      >
        <Stack spacing={1} sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            sx={{
              fontFamily: "var(--font-mono)",
              fontSize: "0.72rem",
              letterSpacing: 0.6,
              textTransform: "uppercase",
              color: "var(--text-secondary)",
            }}
          >
            How AgentArk is learning
          </Typography>
          <Typography
            variant="h3"
            sx={{
              fontSize: { xs: "1.6rem", md: "2rem" },
              fontWeight: 600,
              lineHeight: 1.18,
              color: "var(--text-primary)",
              maxWidth: 720,
            }}
          >
            {title}
          </Typography>
          <Typography
            sx={{
              fontSize: "0.94rem",
              lineHeight: 1.55,
              color: "var(--text-secondary)",
              maxWidth: 640,
            }}
          >
            {detail}
          </Typography>
        </Stack>
        <HeadlineDisplay headline={headline} />
      </Stack>

      {moments.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gap: 1.4,
            gridTemplateColumns: {
              xs: "1fr",
              sm: "repeat(2, 1fr)",
              lg: `repeat(${moments.length}, minmax(0, 1fr))`,
            },
          }}
        >
          {moments.map((moment) => (
            <MomentCard key={moment.id} moment={moment} />
          ))}
        </Box>
      ) : null}

    </Stack>
  );
}

function HeadlineDisplay({ headline }: { headline: Headline }) {
  // Tone palette is kept consistent with the rest of the app — the
  // headline tells the user at a glance whether something wants their
  // attention, is in progress, is steady, or is paused. Glow intensity
  // tracks tone urgency: attention > live > steady > paused (no glow).
  const tonePalette: Record<Headline["tone"], { color: string; glow: string }> = {
    attention: {
      color: "#ffbe63",
      glow:
        "0 0 24px rgba(255, 190, 99, 0.32), 0 0 60px rgba(255, 190, 99, 0.10)",
    },
    live: {
      color: "#78f2b0",
      glow:
        "0 0 24px rgba(120, 242, 176, 0.28), 0 0 60px rgba(120, 242, 176, 0.08)",
    },
    steady: {
      color: "#c8d8c9",
      glow:
        "0 0 24px rgba(200, 216, 201, 0.18), 0 0 60px rgba(200, 216, 201, 0.06)",
    },
    paused: {
      color: "var(--text-secondary)",
      glow: "none",
    },
  };
  const palette = tonePalette[headline.tone];
  return (
    <Box
      sx={{
        textAlign: { xs: "left", lg: "right" },
        minWidth: { lg: 220 },
        flex: { lg: "0 0 auto" },
      }}
      aria-label={`${headline.value} ${headline.unitLabel}`}
    >
      <Typography
        component="div"
        sx={{
          fontFamily: "var(--font-mono)",
          fontWeight: 700,
          fontSize: { xs: "3rem", md: "4rem", lg: "4.5rem" },
          lineHeight: 1,
          letterSpacing: -1.4,
          color: palette.color,
          textShadow: palette.glow,
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {headline.value}
      </Typography>
      <Typography
        sx={{
          mt: 0.6,
          fontSize: "0.84rem",
          fontWeight: 500,
          color: "var(--text-primary)",
        }}
      >
        {headline.unitLabel}
      </Typography>
      <Typography
        sx={{
          fontFamily: "var(--font-mono)",
          fontSize: "0.72rem",
          letterSpacing: 0.4,
          textTransform: "uppercase",
          color: "var(--text-secondary)",
          mt: 0.2,
        }}
      >
        {headline.caption}
      </Typography>
    </Box>
  );
}

function MomentCard({ moment }: { moment: Moment }) {
  const Icon = moment.icon;
  return (
    <Box
      sx={{
        p: 1.6,
        borderRadius: 1.5,
        border: "1px solid var(--surface-border)",
        background: "var(--surface-bg-elevated-stronger, rgba(255,255,255,0.02))",
        transition: "border-color 180ms ease, transform 180ms ease",
        display: "flex",
        flexDirection: "column",
        gap: 1,
        "&:hover": {
          borderColor: "var(--surface-border-strong)",
          transform: "translateY(-1px)",
        },
      }}
    >
      <Box
        sx={{
          width: 32,
          height: 32,
          display: "grid",
          placeItems: "center",
          borderRadius: 1,
          color: moment.accent,
          background: "rgba(255,255,255,0.03)",
        }}
      >
        <Icon fontSize="small" />
      </Box>
      <Typography
        sx={{
          fontSize: "0.92rem",
          fontWeight: 600,
          color: "var(--text-primary)",
          lineHeight: 1.32,
          display: "-webkit-box",
          WebkitLineClamp: 2,
          WebkitBoxOrient: "vertical",
          overflow: "hidden",
        }}
      >
        {moment.title}
      </Typography>
      <Typography
        sx={{
          fontSize: "0.8rem",
          lineHeight: 1.5,
          color: "var(--text-secondary)",
          display: "-webkit-box",
          WebkitLineClamp: 3,
          WebkitBoxOrient: "vertical",
          overflow: "hidden",
          flexGrow: 1,
        }}
      >
        {moment.sentence}
      </Typography>
      {moment.cta ? (
        <Button
          size="small"
          variant={moment.cta.primary ? "contained" : "outlined"}
          color={moment.cta.primary ? "warning" : "primary"}
          startIcon={moment.cta.primary ? <PlayArrowRoundedIcon /> : undefined}
          onClick={moment.cta.onClick}
          sx={{ alignSelf: "flex-start", mt: 0.4 }}
        >
          {moment.cta.label}
        </Button>
      ) : null}
    </Box>
  );
}
