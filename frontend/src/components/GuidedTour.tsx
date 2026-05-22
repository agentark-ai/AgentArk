import { Box, Button, IconButton, Stack, Typography } from "@mui/material";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useUiStore } from "../store/uiStore";

type TourStepDef = {
  id: string;
  view: string;
  targetSelector: string;
  title: string;
  body: string;
  placement: "bottom" | "top" | "left" | "right";
  spotlightPadding?: number;
  settingsInitialTab?: number;
};

const TOUR_STEPS: TourStepDef[] = [
  {
    id: "mission-control",
    view: "overview",
    targetSelector:
      "[data-tour-target='overview-dashboard'], [data-tour-target='welcome-hero']",
    title: "Mission Control",
    body: "Start here for live work, attention items, suggestions, and recent activity. It is the main status surface before you jump into a specific tool.",
    placement: "bottom",
    spotlightPadding: 12,
  },
  {
    id: "chat-workspace",
    view: "chat",
    targetSelector:
      "[data-tour-target='chat-workspace'], [data-tour-target='nav-chat']",
    title: "Chat",
    body: "Ask for summaries, reminders, drafts, research, app work, or direct action here. Longer work can become a task when it needs scheduling, approval, or retry.",
    placement: "bottom",
    spotlightPadding: 10,
  },
  {
    id: "tasks-work-queue",
    view: "tasks",
    targetSelector:
      "[data-tour-target='tasks-work-queue'], [data-tour-target='nav-tasks']",
    title: "Tasks",
    body: "Long-running jobs, approvals, paused work, and scheduled automations land in this queue so the system can resume work without losing state.",
    placement: "top",
    spotlightPadding: 10,
  },
  {
    id: "arkmemory",
    view: "arkmemory",
    targetSelector:
      "[data-tour-target='arkmemory-tabs'], [data-tour-target='nav-arkmemory']",
    title: "Memory",
    body: "Saved facts, preferences, and memory review queues live here. Use these tabs to see what AgentArk currently remembers and what changed.",
    placement: "bottom",
    spotlightPadding: 10,
  },
  {
    id: "trace-runs",
    view: "trace",
    targetSelector:
      "[data-tour-target='trace-tabs'], [data-tour-target='nav-trace']",
    title: "Trace",
    body: "Recent runs, runtime activity, sync work, exports, and security events are split into these tabs for inspection and debugging.",
    placement: "bottom",
    spotlightPadding: 10,
  },
  {
    id: "apps-registry",
    view: "apps",
    targetSelector:
      "[data-tour-target='apps-registry'], [data-tour-target='nav-apps']",
    title: "Apps",
    body: "Generated apps and managed launchers show up here with health, links, restore status, and runtime controls.",
    placement: "top",
    spotlightPadding: 10,
  },
  {
    id: "settings-models",
    view: "settings",
    targetSelector:
      "[data-tour-target='settings-models'], [data-tour-target='settings-trigger']",
    title: "Models and settings",
    body: "Model routing, integrations, security, and advanced controls live in Settings. The Model Pool is where you connect providers before AgentArk runs real work.",
    placement: "left",
    spotlightPadding: 10,
    settingsInitialTab: 1,
  },
];

type Rect = { top: number; left: number; width: number; height: number };
type BackdropSlice = {
  top: number;
  left: number;
  width: number;
  height: number;
};

const TARGET_MEASURE_RETRY_LIMIT = 30;
const TARGET_MEASURE_RETRY_DELAY_MS = 200;

function getElementRect(selector: string): Rect | null {
  const el = document.querySelector(selector);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

function tooltipPosition(
  target: Rect | null,
  placement: TourStepDef["placement"],
  pad: number,
): { top: number; left: number } {
  const tooltipWidth = 380;
  const tooltipHeight = 220;
  const gap = 14;
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;

  if (!target) {
    return {
      top: viewportHeight / 2 - tooltipHeight / 2,
      left: viewportWidth / 2 - tooltipWidth / 2,
    };
  }

  let top = 0;
  let left = 0;

  switch (placement) {
    case "bottom":
      top = target.top + target.height + pad + gap;
      left = target.left + target.width / 2 - tooltipWidth / 2;
      break;
    case "top":
      top = target.top - pad - gap - tooltipHeight;
      left = target.left + target.width / 2 - tooltipWidth / 2;
      break;
    case "right":
      top = target.top + target.height / 2 - tooltipHeight / 2;
      left = target.left + target.width + pad + gap;
      break;
    case "left":
      top = target.top + target.height / 2 - tooltipHeight / 2;
      left = target.left - pad - gap - tooltipWidth;
      break;
  }

  if (left < 16) left = 16;
  if (left + tooltipWidth > viewportWidth - 16) left = viewportWidth - 16 - tooltipWidth;
  if (top < 16) top = 16;
  if (top + tooltipHeight > viewportHeight - 16) top = viewportHeight - 16 - tooltipHeight;

  return { top, left };
}

function backdropSlices(target: Rect, pad: number): BackdropSlice[] {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const top = Math.max(0, target.top - pad);
  const left = Math.max(0, target.left - pad);
  const right = Math.min(viewportWidth, target.left + target.width + pad);
  const bottom = Math.min(viewportHeight, target.top + target.height + pad);

  return [
    { top: 0, left: 0, width: viewportWidth, height: top },
    {
      top,
      left: 0,
      width: left,
      height: Math.max(0, bottom - top),
    },
    {
      top,
      left: right,
      width: Math.max(0, viewportWidth - right),
      height: Math.max(0, bottom - top),
    },
    {
      top: bottom,
      left: 0,
      width: viewportWidth,
      height: Math.max(0, viewportHeight - bottom),
    },
  ].filter((slice) => slice.width > 0 && slice.height > 0);
}

type Props = {
  openTourStep: (view: string, options?: { settingsInitialTab?: number }) => void;
  currentView: string;
};

export function GuidedTour({ openTourStep, currentView }: Props) {
  const tourActive = useUiStore((s) => s.tourActive);
  const tourStep = useUiStore((s) => s.tourStep);
  const nextTourStep = useUiStore((s) => s.nextTourStep);
  const prevTourStep = useUiStore((s) => s.prevTourStep);
  const skipTour = useUiStore((s) => s.skipTour);
  const completeTour = useUiStore((s) => s.completeTour);

  const [targetRect, setTargetRect] = useState<Rect | null>(null);
  const [renderKey, setRenderKey] = useState(0);
  const retryRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const stepDef = TOUR_STEPS[tourStep] as TourStepDef | undefined;

  useEffect(() => {
    if (!tourActive || !stepDef) return;
    openTourStep(stepDef.view, {
      settingsInitialTab: stepDef.settingsInitialTab,
    });
  }, [tourActive, tourStep, openTourStep, stepDef]);

  useLayoutEffect(() => {
    if (!tourActive || !stepDef) return;
    setTargetRect(null);

    const measure = (attempt: number) => {
      const rect = getElementRect(stepDef.targetSelector);
      if (rect) {
        setTargetRect(rect);
        setRenderKey((key) => key + 1);
      } else if (attempt < TARGET_MEASURE_RETRY_LIMIT) {
        retryRef.current = setTimeout(
          () => measure(attempt + 1),
          TARGET_MEASURE_RETRY_DELAY_MS,
        );
      }
    };

    retryRef.current = setTimeout(() => measure(0), 150);
    return () => {
      if (retryRef.current) clearTimeout(retryRef.current);
    };
  }, [tourActive, tourStep, currentView, stepDef]);

  useEffect(() => {
    if (!tourActive || !stepDef) return;
    const update = () => {
      const rect = getElementRect(stepDef.targetSelector);
      if (rect) setTargetRect(rect);
    };
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [tourActive, stepDef]);

  useEffect(() => {
    if (!tourActive) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") skipTour();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [tourActive, skipTour]);

  const handleNext = useCallback(() => {
    if (tourStep >= TOUR_STEPS.length - 1) {
      completeTour();
    } else {
      nextTourStep();
    }
  }, [tourStep, completeTour, nextTourStep]);

  if (!tourActive || !stepDef) return null;

  const pad = stepDef.spotlightPadding ?? 8;
  const isFirst = tourStep === 0;
  const isLast = tourStep === TOUR_STEPS.length - 1;
  const pos = tooltipPosition(targetRect, stepDef.placement, pad);
  const slices = targetRect ? backdropSlices(targetRect, pad) : [];

  return (
    <>
      <Box
        className="tour-backdrop"
        sx={{
          position: "fixed",
          inset: 0,
          zIndex: 9998,
          pointerEvents: "none",
        }}
      >
        <svg
          width="100%"
          height="100%"
          style={{ position: "absolute", inset: 0, pointerEvents: "none" }}
        >
          <defs>
            <mask id="tour-spotlight-mask">
              <rect x="0" y="0" width="100%" height="100%" fill="white" />
              {targetRect ? (
                <rect
                  x={Math.max(0, targetRect.left - pad)}
                  y={Math.max(0, targetRect.top - pad)}
                  rx="14"
                  ry="14"
                  width={targetRect.width + pad * 2}
                  height={targetRect.height + pad * 2}
                  fill="black"
                />
              ) : null}
            </mask>
          </defs>
          <rect
            x="0"
            y="0"
            width="100%"
            height="100%"
            fill="var(--ui-rgba-3-8-17-760)"
            mask="url(#tour-spotlight-mask)"
          />
        </svg>
      </Box>
      {targetRect ? (
        slices.map((slice, index) => (
          <Box
            key={`tour-hitbox-${stepDef.id}-${renderKey}-${index}`}
            onClick={skipTour}
            sx={{
              position: "fixed",
              zIndex: 9998,
              top: slice.top,
              left: slice.left,
              width: slice.width,
              height: slice.height,
              background: "transparent",
            }}
          />
        ))
      ) : (
        <Box
          onClick={skipTour}
          sx={{
            position: "fixed",
            inset: 0,
            zIndex: 9998,
          }}
        />
      )}
      {targetRect ? (
        <Box
          key={renderKey}
          className="tour-spotlight-ring"
          sx={{
            position: "fixed",
            zIndex: 9999,
            pointerEvents: "none",
            top: Math.max(0, targetRect.top - pad),
            left: Math.max(0, targetRect.left - pad),
            width: targetRect.width + pad * 2,
            height: targetRect.height + pad * 2,
            borderRadius: 2,
            border: "2px solid var(--green)",
            boxShadow:
              "0 0 0 1px var(--ui-rgba-130-247-193-220), 0 0 34px var(--ui-rgba-130-247-193-220), inset 0 0 24px var(--ui-rgba-0-255-170-060)",
            animation: "tour-ring-pulse 2s ease-in-out infinite",
          }}
        />
      ) : null}
      <Box
        key={`${stepDef.id}-${renderKey}`}
        className="tour-tooltip"
        sx={{
          position: "fixed",
          top: pos.top,
          left: pos.left,
          width: 380,
          maxWidth: "calc(100vw - 32px)",
          minHeight: 220,
          zIndex: 10000,
          borderRadius: 2,
          border: "1px solid var(--ui-rgba-130-247-193-320)",
          background:
            "radial-gradient(circle at 12% 0%, var(--ui-rgba-0-255-170-080), transparent 34%), linear-gradient(160deg, var(--cyber-panel-raised), var(--cyber-panel))",
          backdropFilter: "blur(18px)",
          WebkitBackdropFilter: "blur(18px)",
          boxShadow:
            "0 18px 48px var(--ui-rgba-0-0-0-400), 0 0 0 1px var(--ui-rgba-0-255-170-040)",
          p: 2,
          display: "flex",
          flexDirection: "column",
          pointerEvents: "auto",
          animation: "tour-tooltip-enter 180ms ease",
        }}
      >
        <Stack
          direction="row"
          spacing={1}
          sx={{
            justifyContent: "space-between",
            alignItems: "flex-start"
          }}>
          <Box>
            <Typography
              variant="overline"
              sx={{ color: "var(--green)", letterSpacing: 0 }}
            >
              Guided Tour
            </Typography>
            <Typography variant="h6" sx={{ mt: 0.4, fontWeight: 700 }}>
              {stepDef.title}
            </Typography>
          </Box>
          <IconButton size="small" onClick={skipTour} aria-label="Close tour">
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </Stack>

        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
            mt: 1.3,
            lineHeight: 1.6
          }}>
          {stepDef.body}
        </Typography>

        <Box sx={{ flex: 1 }} />

        <Stack
          direction="row"
          sx={{
            justifyContent: "space-between",
            alignItems: "center",
            mt: 2
          }}>
          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            Step {tourStep + 1} of {TOUR_STEPS.length}
          </Typography>
          <Stack direction="row" spacing={1}>
            {!isFirst ? (
              <Button variant="text" onClick={prevTourStep} sx={{ textTransform: "none" }}>
                Back
              </Button>
            ) : null}
            <Button variant="text" onClick={skipTour} sx={{ textTransform: "none" }}>
              Skip
            </Button>
            <Button variant="contained" onClick={handleNext} sx={{ textTransform: "none" }}>
              {isLast ? "Finish" : "Next"}
            </Button>
          </Stack>
        </Stack>
      </Box>
    </>
  );
}
