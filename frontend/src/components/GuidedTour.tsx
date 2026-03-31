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
    id: "welcome-models",
    view: "settings",
    targetSelector: "[data-tour-target='settings-models']",
    title: "Welcome! Let's add your first AI model",
    body: "AgentArk needs at least one LLM to work. Add an OpenAI, Anthropic, Ollama, or OpenRouter model here. You can configure multiple models for different tasks - primary, fast, code, and more.",
    placement: "left",
    spotlightPadding: 10,
    settingsInitialTab: 1,
  },
  {
    id: "chat",
    view: "chat",
    targetSelector: "[data-tour-target='nav-chat']",
    title: "Start in chat",
    body: "This is the main working surface. Quick questions stay chat-native, while builds, imports, research, and file-changing work can promote into durable tasks without leaving the thread.",
    placement: "right",
    spotlightPadding: 6,
  },
  {
    id: "tasks",
    view: "tasks",
    targetSelector: "[data-tour-target='nav-tasks']",
    title: "Tasks stay durable",
    body: "Use Tasks for long-running execution, retries, approvals, and anything that should outlive a single chat turn.",
    placement: "right",
    spotlightPadding: 6,
  },
  {
    id: "apps",
    view: "apps",
    targetSelector: "[data-tour-target='nav-apps']",
    title: "Apps stay directly accessible",
    body: "Built apps live here with their links, guard settings, and operations. You do not need to hunt through a separate library surface to manage them.",
    placement: "right",
    spotlightPadding: 6,
  },
  {
    id: "attention",
    view: "overview",
    targetSelector: "[data-tour-target='overview-attention']",
    title: "Mission Control holds the attention queue",
    body: "Approvals, pauses, failures, and urgent alerts now land in one needs-attention queue inside Mission Control, with Tasks and Trace handling the deeper follow-up.",
    placement: "bottom",
    spotlightPadding: 12,
  },
  {
    id: "overview",
    view: "overview",
    targetSelector: "[data-tour-target='overview-dashboard']",
    title: "Mission Control stays lightweight",
    body: "Mission Control is the daily summary surface: attention items, suggestions, highlights, and recent activity. It should support chat, not compete with it.",
    placement: "bottom",
    spotlightPadding: 12,
  },
  {
    id: "done",
    view: "chat",
    targetSelector: "[data-tour-target='workspace-shell']",
    title: "You're all set!",
    body: "Start in chat, open deeper operational panels only when needed, and use Mission Control when the run needs you. You can re-run this tour anytime from Settings > Advanced.",
    placement: "bottom",
    spotlightPadding: 10,
  },
];

type Rect = { top: number; left: number; width: number; height: number };

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
      } else if (attempt < 8) {
        retryRef.current = setTimeout(() => measure(attempt + 1), 200);
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

  return (
    <>
      <Box
        className="tour-backdrop"
        onClick={skipTour}
        sx={{
          position: "fixed",
          inset: 0,
          zIndex: 9998,
          pointerEvents: "auto",
        }}
      >
        <svg width="100%" height="100%" style={{ position: "absolute", inset: 0 }}>
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
            fill="rgba(3, 8, 17, 0.76)"
            mask="url(#tour-spotlight-mask)"
          />
        </svg>
      </Box>

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
            borderRadius: 3,
            border: "2px solid rgba(47, 212, 255, 0.95)",
            boxShadow:
              "0 0 0 1px rgba(47, 212, 255, 0.2), 0 0 34px rgba(47, 212, 255, 0.2), inset 0 0 24px rgba(47, 212, 255, 0.06)",
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
          borderRadius: 3,
          border: "1px solid rgba(47, 212, 255, 0.22)",
          background:
            "linear-gradient(160deg, rgba(10, 18, 34, 0.97), rgba(7, 14, 28, 0.95))",
          backdropFilter: "blur(18px)",
          WebkitBackdropFilter: "blur(18px)",
          boxShadow: "0 18px 48px rgba(0, 0, 0, 0.4)",
          p: 2,
          display: "flex",
          flexDirection: "column",
          animation: "tour-tooltip-enter 180ms ease",
        }}
      >
        <Stack direction="row" justifyContent="space-between" alignItems="flex-start" spacing={1}>
          <Box>
            <Typography
              variant="overline"
              sx={{ color: "rgba(47, 212, 255, 0.86)", letterSpacing: "0.1em" }}
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

        <Typography variant="body2" color="text.secondary" sx={{ mt: 1.3, lineHeight: 1.6 }}>
          {stepDef.body}
        </Typography>

        <Box sx={{ flex: 1 }} />

        <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mt: 2 }}>
          <Typography variant="caption" color="text.secondary">
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
