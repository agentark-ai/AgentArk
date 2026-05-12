// Terminal-style live output for shell / code_execute / build / deploy steps.

import { useEffect, useState } from "react";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";

import type { ChatStepCard } from "../types";
import { extractCommand, extractSurfaceBody } from "../dispatch";
import { surfaceFromCard } from "../surface";
import { LinkifiedText } from "./LinkifiedText";
import { buildReadableToolPresentation } from "./presentation";

export interface TerminalViewProps {
  card: ChatStepCard;
  live?: boolean;
}

type TerminalTone = "idle" | "run" | "ok" | "fail";

function pickTone(card: ChatStepCard, live: boolean): TerminalTone {
  const status = surfaceFromCard(card)?.status;
  if (status === "error") return "fail";
  if (status === "done") return "ok";
  if (status === "running" || status === "waiting" || status === "pending")
    return live ? "run" : "idle";
  const k = (card.kind || "").toLowerCase();
  if (k.includes("issue") || k.includes("error") || k.includes("fail")) return "fail";
  if (k.includes("done") || k.includes("complete") || k.includes("success")) return "ok";
  if (live || k.includes("running") || k.includes("planning")) return "run";
  return "idle";
}

function pickBody(card: ChatStepCard): string {
  return (
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    card.summary ||
    card.payloadView?.body ||
    ""
  );
}

const TONE_LABEL: Record<TerminalTone, string> = {
  run: "running",
  ok: "done",
  fail: "failed",
  idle: "idle",
};

export function TerminalView({ card, live = false }: TerminalViewProps) {
  const command = extractCommand(card);
  const presentation = buildReadableToolPresentation(card);
  const structuredBody = extractSurfaceBody(card);
  const body =
    structuredBody ||
    (presentation.isStructured ? presentation.body : pickBody(card));
  const tone = pickTone(card, live);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  async function handleCopy() {
    if (!body) return;
    try {
      await navigator.clipboard.writeText(body);
      setCopied(true);
    } catch {
      // Clipboard access can be denied in insecure contexts.
    }
  }

  return (
    <Box className="cview cview-terminal">
      <Box className={`cview-terminal-head tone-${tone}`}>
        <span className="cview-terminal-prompt" aria-hidden="true">$</span>
        <span
          className="cview-terminal-cmd"
          title={command || card.label}
        >
          {command || card.label}
        </span>
        <span className={`cview-terminal-pill tone-${tone}`}>
          {TONE_LABEL[tone]}
        </span>
        <Tooltip
          title={copied ? "Copied" : "Copy terminal output"}
          placement="top"
          arrow
        >
          <span>
            <IconButton
              className="cview-terminal-copy"
              size="small"
              disabled={!body}
              onClick={handleCopy}
              aria-label="Copy terminal output"
            >
              <ContentCopyRoundedIcon fontSize="inherit" />
            </IconButton>
          </span>
        </Tooltip>
      </Box>
      <pre className="cview-terminal-body">
        {body ? <LinkifiedText text={body} /> : live ? "..." : "(no output captured)"}
        {live ? <span className="cview-terminal-caret" aria-hidden="true">|</span> : null}
      </pre>
      {card.detail && card.detail !== body ? (
        <Typography variant="caption" className="cview-terminal-detail">
          <LinkifiedText text={card.detail} />
        </Typography>
      ) : null}
    </Box>
  );
}

export default TerminalView;
