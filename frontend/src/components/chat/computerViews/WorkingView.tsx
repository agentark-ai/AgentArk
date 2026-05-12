// Live working view shown while the agent is mid-turn but has not emitted a
// tool artifact yet.

import { useEffect, useRef, useState } from "react";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import { LinkifiedText } from "./LinkifiedText";

export interface WorkingViewProps {
  phaseLabel?: string;
  detail?: string;
  startedAt?: string | number | null;
  tokenPreview?: string;
  /** Live planner/classifier reasoning text. Surfaced when the assistant
   * content stream has not started yet. Source of truth: structural
   * `reasoning_delta` events from the backend, never phrase-matched. */
  reasoningPreview?: string;
  /** Structural phase label for the reasoning stream: "classifier" or
   * "planner". Drives the small label pill above the preview. */
  reasoningPhase?: string;
  /** True when rendering saved reasoning from a completed run. */
  persisted?: boolean;
}

const REASONING_PHASE_LABELS: Record<string, string> = {
  classifier: "Reviewing intent",
  planner: "Planning",
  model: "Reasoning",
  model_summary: "Reasoning summary",
};
const WORKING_PREVIEW_MAX_CHARS = 12_000;

function tailPreview(value: string): string {
  return value.length > WORKING_PREVIEW_MAX_CHARS
    ? value.slice(-WORKING_PREVIEW_MAX_CHARS)
    : value;
}

function normalizeStartedAt(value: WorkingViewProps["startedAt"]): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string" && value.length > 0) {
    const parsed = Number(new Date(value));
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function WorkingView({
  phaseLabel,
  detail,
  startedAt,
  tokenPreview,
  reasoningPreview,
  reasoningPhase,
  persisted = false,
}: WorkingViewProps) {
  const startedAtMs = normalizeStartedAt(startedAt);
  const [now, setNow] = useState<number>(() => Date.now());
  const [copied, setCopied] = useState(false);
  const previewRef = useRef<HTMLPreElement | null>(null);
  const followPreviewRef = useRef(true);
  const previewModeRef = useRef("");

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(id);
  }, [copied]);

  const elapsedLabel = startedAtMs !== null ? formatElapsed(now - startedAtMs) : "...";
  const assistantContent =
    typeof tokenPreview === "string" && tokenPreview.length > 0
      ? tokenPreview
      : "";
  const reasoningContent =
    typeof reasoningPreview === "string" && reasoningPreview.length > 0
      ? reasoningPreview
      : "";
  // Precedence: real assistant content wins. Reasoning is only shown when
  // the assistant stream has not started — structural fallback, no phrase
  // matching.
  const isReasoning = !assistantContent && Boolean(reasoningContent);
  const previewContent = tailPreview(assistantContent || reasoningContent);
  const previewMode = isReasoning
    ? "reasoning"
    : assistantContent
      ? "assistant"
      : "empty";
  const reasoningPill =
    isReasoning && reasoningPhase
      ? REASONING_PHASE_LABELS[reasoningPhase] || null
      : null;
  const terminalPhase =
    /failed|stopped|cancelled|blocked|complete/i.test(phaseLabel || "") ||
    /failed|stopped|cancelled|blocked|complete/i.test(detail || "");
  const emptyPreviewText = detail
    ? terminalPhase
      ? "No further steps are running."
      : "Preparing the next step..."
    : "Preparing the response...";

  const handleCopyPreview = async () => {
    if (!previewContent) return;
    try {
      await navigator.clipboard.writeText(previewContent);
      setCopied(true);
    } catch {
      // Clipboard access can be unavailable outside secure browser contexts.
    }
  };

  useEffect(() => {
    if (previewModeRef.current !== previewMode) {
      previewModeRef.current = previewMode;
      followPreviewRef.current = true;
    }
  }, [previewMode]);

  useEffect(() => {
    const node = previewRef.current;
    if (!node || !followPreviewRef.current) return;
    node.scrollTop = node.scrollHeight;
  }, [previewContent]);

  const handlePreviewScroll = () => {
    const node = previewRef.current;
    if (!node) return;
    const distanceFromBottom =
      node.scrollHeight - node.scrollTop - node.clientHeight;
    followPreviewRef.current = distanceFromBottom < 32;
  };

  return (
    <Box className="cview cview-working">
      <Box className="cview-working-head">
        <Box className="cview-working-icon" aria-hidden="true">
          <AutoAwesomeRoundedIcon fontSize="small" />
        </Box>
        <Typography variant="subtitle1" className="cview-working-label">
          {phaseLabel || "Working..."}
        </Typography>
        <Box className="cview-working-actions">
          {!persisted ? (
            <Typography
              variant="body2"
              className="cview-working-elapsed"
              aria-live="polite"
            >
              {elapsedLabel}
            </Typography>
          ) : null}
          {previewContent ? (
            <Tooltip title={copied ? "Copied" : "Copy thinking"} placement="top" arrow>
              <span>
                <IconButton
                  size="small"
                  className="cview-working-copy"
                  onClick={handleCopyPreview}
                  aria-label="Copy thinking"
                >
                  <ContentCopyRoundedIcon fontSize="inherit" />
                </IconButton>
              </span>
            </Tooltip>
          ) : null}
        </Box>
      </Box>
      {detail ? (
        <Typography variant="body2" className="cview-working-detail">
          <LinkifiedText text={detail} />
        </Typography>
      ) : null}
      {reasoningPill ? (
        <Box className="cview-working-reasoning-pill" aria-live="polite">
          <Typography variant="caption">{reasoningPill}</Typography>
        </Box>
      ) : null}
      <Box
        className={
          isReasoning
            ? "cview-working-preview cview-working-preview-reasoning"
            : "cview-working-preview"
        }
      >
        <Box className="cview-working-preview-fade" aria-hidden="true" />
        {previewContent ? (
          <pre ref={previewRef} onScroll={handlePreviewScroll}>
            <LinkifiedText text={previewContent} />
          </pre>
        ) : (
          <Typography variant="body2" className="cview-working-preview-empty">
            {emptyPreviewText}
          </Typography>
        )}
      </Box>
    </Box>
  );
}

export default WorkingView;
