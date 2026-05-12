// Browser view for browse / watch / fetch_url steps.
// Renders a faux browser chrome (URL bar) above a snapshot of captured page text.

import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import Link from "@mui/material/Link";

import type { ChatStepCard } from "../types";
import { extractSurfaceBody, extractUrl } from "../dispatch";
import { LinkifiedText } from "./LinkifiedText";
import { buildReadableToolPresentation } from "./presentation";

export interface BrowseViewProps {
  card: ChatStepCard;
}

function pickSnapshot(card: ChatStepCard): string {
  return (
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    card.summary ||
    card.payloadView?.body ||
    ""
  );
}

export function BrowseView({ card }: BrowseViewProps) {
  const url = extractUrl(card);
  const presentation = buildReadableToolPresentation(card);
  const structuredSnapshot = extractSurfaceBody(card);
  const snapshot = structuredSnapshot || (presentation.isStructured
    ? presentation.body
    : pickSnapshot(card));
  return (
    <Box className="cview cview-browse">
      <Box className="cview-browse-head">
        <span className="cview-browse-dots" aria-hidden="true">
          <span /><span /><span />
        </span>
        {url ? (
          <Link
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="cview-browse-url"
            underline="hover"
          >
            {url}
          </Link>
        ) : (
          <span className="cview-browse-url" title={card.label}>
            {card.label}
          </span>
        )}
      </Box>
      <Box className="cview-browse-body">
        {snapshot ? (
          <pre className="cview-browse-snapshot">
            <LinkifiedText text={snapshot} />
          </pre>
        ) : (
          <Typography variant="body2" className="cview-browse-empty">
            No page snapshot captured for this step.
          </Typography>
        )}
      </Box>
    </Box>
  );
}

export default BrowseView;
