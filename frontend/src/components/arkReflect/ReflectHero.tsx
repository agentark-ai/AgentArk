import { Box, Button, Card, Stack, Typography } from "@mui/material";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import MemoryRoundedIcon from "@mui/icons-material/MemoryRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import AutoGraphRoundedIcon from "@mui/icons-material/AutoGraphRounded";
import BubbleChartRoundedIcon from "@mui/icons-material/BubbleChartRounded";
import TimelineRoundedIcon from "@mui/icons-material/TimelineRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import ExpandLessRoundedIcon from "@mui/icons-material/ExpandLessRounded";
import { useMemo } from "react";
import {
  HeadlineNumber,
  HeroSentence,
  Moment,
  NarrativeInput,
  NextStep,
  SourceFamily,
  hasMeaningfulActivity,
  headlineNumber,
  heroSentence,
  nextStep,
  topMoments,
} from "./reflectNarrative";

type ReflectHeroProps = {
  input: NarrativeInput | null;
  loading: boolean;
  showDetails: boolean;
  onToggleDetails: () => void;
  onLaunchPrompt?: (prompt: string, source: string) => void;
};

const FAMILY_ICON: Record<SourceFamily, React.ElementType> = {
  conversations: ChatRoundedIcon,
  memory: MemoryRoundedIcon,
  apps: HubRoundedIcon,
  background: AutoGraphRoundedIcon,
  system: BubbleChartRoundedIcon,
  mixed: TimelineRoundedIcon,
};

// Per-family tint — kept low-saturation against the dark surfaces so the
// hero's single green accent (the headline number) stays dominant.
const FAMILY_ACCENT: Record<SourceFamily, string> = {
  conversations: "rgba(57, 208, 255, 0.78)",
  memory: "rgba(139, 214, 165, 0.78)",
  apps: "rgba(255, 190, 99, 0.78)",
  background: "rgba(213, 145, 255, 0.78)",
  system: "rgba(124, 231, 255, 0.78)",
  mixed: "rgba(213, 228, 255, 0.62)",
};

export default function ReflectHero({
  input,
  loading,
  showDetails,
  onToggleDetails,
  onLaunchPrompt,
}: ReflectHeroProps) {
  const story = useMemo(() => {
    if (!input) return null;
    return {
      sentence: heroSentence(input),
      headline: headlineNumber(input),
      moments: topMoments(input, 5),
      next: nextStep(input),
      meaningful: hasMeaningfulActivity(input),
    };
  }, [input]);

  return (
    <Card
      sx={{
        position: "relative",
        overflow: "hidden",
        p: { xs: 2.5, md: 3.5 },
        // Soft top-left to bottom-right surface gradient anchored on
        // AgentArk's green accent at low opacity. Keeps the surface
        // calm — the only loud thing is the headline number itself.
        background:
          "radial-gradient(120% 80% at 0% 0%, var(--ui-rgba-120-242-176-040, rgba(120, 242, 176, 0.04)) 0%, transparent 60%), var(--surface-bg-elevated)",
        border: "1px solid var(--surface-border)",
        borderRadius: 2,
        animation: "reflectHeroFadeIn 320ms ease-out",
        "@keyframes reflectHeroFadeIn": {
          from: { opacity: 0, transform: "translateY(8px)" },
          to: { opacity: 1, transform: "translateY(0)" },
        },
      }}
    >
      {loading && !story ? (
        <HeroSkeleton />
      ) : !story || !story.meaningful ? (
        <HeroEmpty sentence={story?.sentence ?? null} />
      ) : (
        <HeroBody
          sentence={story.sentence}
          headline={story.headline}
          moments={story.moments}
          next={story.next}
          showDetails={showDetails}
          onToggleDetails={onToggleDetails}
          onLaunchPrompt={onLaunchPrompt}
        />
      )}
    </Card>
  );
}

function HeroSkeleton() {
  // Skeleton uses shimmering blocks at the same sizes as the live body
  // so the layout doesn't jump on first data. Respects reduced motion
  // implicitly because we use opacity, not transforms.
  return (
    <Stack spacing={2.5}>
      <Box>
        <Box
          sx={{
            height: 12,
            width: 96,
            background: "var(--surface-bg-elevated-stronger)",
            opacity: 0.6,
            borderRadius: 999,
            mb: 1.4,
          }}
        />
        <Box
          sx={{
            height: 28,
            width: "60%",
            background: "var(--surface-bg-elevated-stronger)",
            opacity: 0.4,
            borderRadius: 8,
          }}
        />
      </Box>
      <Box>
        <Box
          sx={{
            height: 56,
            width: 168,
            background: "var(--surface-bg-elevated-stronger)",
            opacity: 0.32,
            borderRadius: 8,
            mb: 1,
          }}
        />
        <Box
          sx={{
            height: 12,
            width: 220,
            background: "var(--surface-bg-elevated-stronger)",
            opacity: 0.4,
            borderRadius: 999,
          }}
        />
      </Box>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "1fr",
            sm: "repeat(2, 1fr)",
            md: "repeat(3, 1fr)",
            lg: "repeat(5, 1fr)",
          },
          gap: 1.4,
        }}
      >
        {[0, 1, 2, 3, 4].map((index) => (
          <Box
            key={index}
            sx={{
              height: 96,
              background: "var(--surface-bg-elevated-stronger)",
              opacity: 0.28,
              borderRadius: 10,
            }}
          />
        ))}
      </Box>
    </Stack>
  );
}

function HeroEmpty({ sentence }: { sentence: HeroSentence | null }) {
  return (
    <Stack
      spacing={1.2}
      sx={{
        textAlign: { xs: "left", md: "left" },
        py: { xs: 1, md: 1.5 },
      }}
    >
      <Typography
        sx={{
          fontFamily: "var(--font-mono)",
          fontSize: "0.72rem",
          letterSpacing: 0.6,
          textTransform: "uppercase",
          color: "var(--text-secondary)",
        }}
      >
        ArkReflect
      </Typography>
      <Typography
        variant="h3"
        sx={{
          fontSize: { xs: "1.6rem", md: "2rem" },
          fontWeight: 600,
          lineHeight: 1.2,
          color: "var(--text-primary)",
          maxWidth: 720,
        }}
      >
        {sentence?.text ?? "Nothing reflected yet."}
      </Typography>
      <Typography
        sx={{
          fontSize: "0.96rem",
          lineHeight: 1.55,
          color: "var(--text-secondary)",
          maxWidth: 640,
        }}
      >
        {sentence?.detail ??
          "ArkReflect will pull together what happened across your chats, memory, apps, and background work once there's something meaningful to show."}
      </Typography>
    </Stack>
  );
}

type HeroBodyProps = {
  sentence: HeroSentence;
  headline: HeadlineNumber;
  moments: Moment[];
  next: NextStep;
  showDetails: boolean;
  onToggleDetails: () => void;
  onLaunchPrompt?: (prompt: string, source: string) => void;
};

function HeroBody({
  sentence,
  headline,
  moments,
  next,
  showDetails,
  onToggleDetails,
  onLaunchPrompt,
}: HeroBodyProps) {
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
            What happened
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
            {sentence.text}
          </Typography>
          <Typography
            sx={{
              fontSize: "0.94rem",
              lineHeight: 1.55,
              color: "var(--text-secondary)",
              maxWidth: 640,
            }}
          >
            {sentence.detail}
          </Typography>
        </Stack>
        <HeadlineNumberDisplay headline={headline} />
      </Stack>

      {moments.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gap: 1.4,
            gridTemplateColumns: {
              xs: "1fr",
              sm: "repeat(2, 1fr)",
              md: "repeat(3, 1fr)",
              lg: `repeat(${Math.min(Math.max(moments.length, 1), 5)}, minmax(0, 1fr))`,
            },
          }}
        >
          {moments.map((moment) => (
            <MomentCard key={moment.id} moment={moment} />
          ))}
        </Box>
      ) : null}

      {next ? (
        <NextStepCard next={next} onLaunchPrompt={onLaunchPrompt} />
      ) : null}

      <Stack direction="row" sx={{ justifyContent: "flex-end" }}>
        <Button
          variant="text"
          onClick={onToggleDetails}
          endIcon={
            showDetails ? <ExpandLessRoundedIcon /> : <ExpandMoreRoundedIcon />
          }
          sx={{
            color: "var(--text-secondary)",
            "&:hover": { color: "var(--text-primary)" },
          }}
        >
          {showDetails ? "Hide details" : "Show details"}
        </Button>
      </Stack>
    </Stack>
  );
}

function HeadlineNumberDisplay({ headline }: { headline: HeadlineNumber }) {
  // The single load-bearing visual on the page: a very large display
  // number with a soft green glow when there's positive activity.
  // Anything else (icons, sublabels) sits in the supporting tier so
  // this number is what users see first when scanning the hero.
  const glow = headline.positive
    ? "0 0 24px rgba(120, 242, 176, 0.24), 0 0 60px rgba(120, 242, 176, 0.08)"
    : "none";
  return (
    <Box
      sx={{
        textAlign: { xs: "left", lg: "right" },
        minWidth: { lg: 220 },
        flex: { lg: "0 0 auto" },
      }}
      aria-label={`${headline.value} ${headline.unitLabel} ${headline.caption}`}
    >
      <Typography
        component="div"
        sx={{
          fontFamily: "var(--font-mono)",
          fontWeight: 700,
          fontSize: { xs: "3rem", md: "4rem", lg: "4.5rem" },
          lineHeight: 1,
          letterSpacing: -1.4,
          color: headline.positive ? "#78f2b0" : "var(--text-primary)",
          textShadow: glow,
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
  const Icon = FAMILY_ICON[moment.sourceFamily];
  const accent = FAMILY_ACCENT[moment.sourceFamily];
  return (
    <Box
      sx={{
        p: 1.6,
        borderRadius: 1.5,
        border: "1px solid var(--surface-border)",
        background: "var(--surface-bg-elevated-stronger, rgba(255,255,255,0.02))",
        transition: "border-color 180ms ease, transform 180ms ease",
        "&:hover": {
          borderColor: "var(--surface-border-strong)",
          transform: "translateY(-1px)",
        },
      }}
    >
      <Stack spacing={1} sx={{ height: "100%" }}>
        <Box
          sx={{
            width: 32,
            height: 32,
            display: "grid",
            placeItems: "center",
            borderRadius: 1,
            color: accent,
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
            // Clamp to two lines so cards stay aligned in the grid.
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
          }}
        >
          {moment.sentence}
        </Typography>
      </Stack>
    </Box>
  );
}

function NextStepCard({
  next,
  onLaunchPrompt,
}: {
  next: NonNullable<NextStep>;
  onLaunchPrompt?: (prompt: string, source: string) => void;
}) {
  // Single suggestion card with one CTA. Placed below the moments so the
  // user has skimmed the "what happened" answer before being asked to
  // act. The CTA reuses the AgentArk green via the success colour so
  // it's visually linked to the headline number.
  return (
    <Box
      sx={{
        p: { xs: 1.6, md: 2 },
        borderRadius: 1.5,
        border: "1px solid rgba(120, 242, 176, 0.22)",
        background:
          "linear-gradient(180deg, rgba(120, 242, 176, 0.06), rgba(120, 242, 176, 0.02))",
      }}
    >
      <Stack
        direction={{ xs: "column", md: "row" }}
        spacing={1.6}
        sx={{ alignItems: { xs: "stretch", md: "center" } }}
      >
        <Stack spacing={0.6} sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            sx={{
              fontFamily: "var(--font-mono)",
              fontSize: "0.7rem",
              letterSpacing: 0.6,
              textTransform: "uppercase",
              color: "#78f2b0",
            }}
          >
            Suggested next step
          </Typography>
          <Typography
            sx={{
              fontSize: "1rem",
              fontWeight: 600,
              color: "var(--text-primary)",
              lineHeight: 1.32,
            }}
          >
            {next.title}
          </Typography>
          <Typography
            sx={{
              fontSize: "0.86rem",
              lineHeight: 1.5,
              color: "var(--text-secondary)",
            }}
          >
            {next.reason}
          </Typography>
        </Stack>
        <Button
          variant="contained"
          color="success"
          startIcon={<PlayArrowRoundedIcon />}
          onClick={() => onLaunchPrompt?.(next.prompt, "arkreflect_hero_next_step")}
          disabled={!onLaunchPrompt}
          sx={{
            minHeight: 40,
            alignSelf: { xs: "stretch", md: "center" },
            whiteSpace: "nowrap",
          }}
        >
          Try this in Chat
        </Button>
      </Stack>
    </Box>
  );
}
