import { useEffect, useState } from "react";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";

import type { ChatStepCard, SurfaceArtifact, SurfacePayload } from "../types";
import {
  firstSurfaceText,
  surfaceDisplayTitle,
  surfaceFromCard,
  surfacePayloads,
} from "../surface";
import { LinkifiedText } from "./LinkifiedText";

export interface GenericSurfaceViewProps {
  card: ChatStepCard;
}

type SurfaceRow = {
  key: string;
  label: string;
  contentType: string;
  body: string;
};

function bodyFromItem(item: SurfacePayload | SurfaceArtifact): string {
  if (item.text) return item.text;
  if (item.json != null) {
    try {
      return JSON.stringify(item.json, null, 2);
    } catch {
      return "";
    }
  }
  if (item.preview) return item.preview;
  if (item.uri) return item.uri;
  if (item.path) return item.path;
  return "";
}

function normalizedBodyKey(body: string): string {
  return body.trim().replace(/\s+/g, " ");
}

function labelForItem(item: SurfacePayload | SurfaceArtifact): string {
  return ("label" in item && item.label) || item.role;
}

function uniqueSurfaceRows(items: Array<SurfacePayload | SurfaceArtifact>) {
  const rows: SurfaceRow[] = [];
  const rowByBody = new Map<string, number>();

  for (const item of items) {
    const body = bodyFromItem(item);
    const bodyKey = normalizedBodyKey(body);
    if (!bodyKey) continue;

    const label = labelForItem(item);
    const existingIndex = rowByBody.get(bodyKey);
    if (existingIndex != null) {
      const existing = rows[existingIndex];
      const labels = existing.label
        .split(" / ")
        .map((value) => value.trim())
        .filter(Boolean);
      if (label && !labels.includes(label)) {
        existing.label = [...labels, label].join(" / ");
      }
      continue;
    }

    rowByBody.set(bodyKey, rows.length);
    rows.push({
      key: `${item.role}-${rows.length}`,
      label,
      contentType: item.contentType,
      body,
    });
  }

  return rows;
}

function SurfacePayloadPager({ rows }: { rows: SurfaceRow[] }) {
  const [page, setPage] = useState(0);
  const [copied, setCopied] = useState(false);
  const pageCount = rows.length;
  const safePage = Math.min(Math.max(page, 0), Math.max(0, pageCount - 1));
  const row = rows[safePage] || null;

  useEffect(() => {
    setPage((current) =>
      Math.min(Math.max(current, 0), Math.max(0, pageCount - 1)),
    );
  }, [pageCount]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  async function handleCopy() {
    if (!row?.body) return;
    try {
      await navigator.clipboard.writeText(row.body);
      setCopied(true);
    } catch {
      // Clipboard access can be denied in insecure contexts.
    }
  }

  if (!row) return null;

  return (
    <Box className="cview-generic-pager">
      <Box className="cview-generic-page-head">
        <Box className="cview-generic-page-title">
          <span className="cview-generic-item-label" title={row.label}>
            {row.label}
          </span>
          <span className="cview-generic-item-meta">
            <span className="cview-generic-item-type">{row.contentType}</span>
            <span className="cview-generic-item-bytes">
              {row.body.length.toLocaleString()} chars
            </span>
          </span>
        </Box>
        <Box className="cview-generic-page-controls">
          {pageCount > 1 ? (
            <>
              <Tooltip title="Previous page" placement="top" arrow>
                <span>
                  <IconButton
                    className="cview-generic-action"
                    size="small"
                    disabled={safePage <= 0}
                    onClick={() => setPage((current) => Math.max(0, current - 1))}
                    aria-label="Previous payload page"
                  >
                    <ChevronLeftRoundedIcon fontSize="inherit" />
                  </IconButton>
                </span>
              </Tooltip>
              <span className="cview-generic-page-count">
                {safePage + 1} / {pageCount}
              </span>
              <Tooltip title="Next page" placement="top" arrow>
                <span>
                  <IconButton
                    className="cview-generic-action"
                    size="small"
                    disabled={safePage >= pageCount - 1}
                    onClick={() =>
                      setPage((current) => Math.min(pageCount - 1, current + 1))
                    }
                    aria-label="Next payload page"
                  >
                    <ChevronRightRoundedIcon fontSize="inherit" />
                  </IconButton>
                </span>
              </Tooltip>
            </>
          ) : null}
          <Tooltip
            title={copied ? "Copied" : "Copy visible payload"}
            placement="top"
            arrow
          >
            <span>
              <IconButton
                className="cview-generic-action"
                size="small"
                disabled={!row.body}
                onClick={handleCopy}
                aria-label="Copy visible payload"
              >
                <ContentCopyRoundedIcon fontSize="inherit" />
              </IconButton>
            </span>
          </Tooltip>
        </Box>
      </Box>
      <pre className="cview-generic-body cview-generic-page-body" tabIndex={0}>
        <LinkifiedText text={row.body} />
      </pre>
    </Box>
  );
}

export function GenericSurfaceView({ card }: GenericSurfaceViewProps) {
  const surface = surfaceFromCard(card);
  const title = surfaceDisplayTitle(card);
  const items = surfacePayloads(card);
  const fallback =
    firstSurfaceText(card) ||
    card.payloadView?.body ||
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    card.summary ||
    "";

  return (
    <Box className="cview cview-generic">
      <Box className="cview-generic-head">
        <AutoAwesomeRoundedIcon fontSize="small" className="cview-generic-icon" />
        <Typography variant="subtitle2" className="cview-generic-title">
          {title}
        </Typography>
        {surface?.renderer.id ? (
          <span className="cview-generic-renderer">{surface.renderer.id}</span>
        ) : null}
      </Box>
      {items.length > 0 ? (
        <StacklessSurfaceItems items={items} />
      ) : fallback ? (
        <SurfacePayloadPager
          rows={[
            {
              key: "raw-payload",
              label: "Raw payload",
              contentType: "text/plain",
              body: fallback,
            },
          ]}
        />
      ) : (
        <Typography variant="body2" className="cview-generic-empty">
          No structured artifact was captured for this step.
        </Typography>
      )}
    </Box>
  );
}

function StacklessSurfaceItems({
  items,
}: {
  items: Array<SurfacePayload | SurfaceArtifact>;
}) {
  const rows = uniqueSurfaceRows(items);
  if (rows.length === 0) {
    return (
      <Typography variant="body2" className="cview-generic-empty">
        No structured artifact was captured for this step.
      </Typography>
    );
  }

  return (
    <div className="cview-generic-items">
      <SurfacePayloadPager rows={rows} />
    </div>
  );
}

export default GenericSurfaceView;
