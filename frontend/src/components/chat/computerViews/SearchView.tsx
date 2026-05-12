// Search view for web_search / search_files / lookup-style steps.

import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";

import type { ChatStepCard } from "../types";
import { extractSurfaceBody } from "../dispatch";
import { LinkifiedText } from "./LinkifiedText";
import { buildReadableToolPresentation } from "./presentation";

export interface SearchViewProps {
  card: ChatStepCard;
}

function splitResults(body: string): string[] {
  if (!body) return [];
  const seen = new Set<string>();
  const parts: string[] = [];
  body
    .split(/\r?\n\r?\n+|^\s*\d+[.)]\s+/m)
    .map((s) => s.trim())
    .filter(Boolean)
    .forEach((entry) => {
      const key = entry.replace(/\s+/g, " ").trim().toLowerCase();
      if (!key || seen.has(key)) return;
      seen.add(key);
      parts.push(entry);
    });
  return parts.slice(0, 12);
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

export function SearchView({ card }: SearchViewProps) {
  const presentation = buildReadableToolPresentation(card);
  const structuredBody = extractSurfaceBody(card);
  const body = structuredBody || (presentation.isStructured ? presentation.body : pickBody(card));
  const query = presentation.query || presentation.title || card.label;
  const results = presentation.isStructured
    ? presentation.rows.length > 0
      ? presentation.rows
      : presentation.summary
        ? [presentation.summary]
        : []
    : splitResults(body);
  return (
    <Box className="cview cview-search">
      <Box className="cview-search-head">
        <SearchRoundedIcon className="cview-search-icon" aria-hidden="true" />
        <span className="cview-search-query" title={query}>
          {query}
        </span>
      </Box>
      {results.length > 0 ? (
        <ol className="cview-search-results">
          {results.map((entry, idx) => (
            <li key={idx} className="cview-search-result">
              <pre className="cview-search-result-body">
                <LinkifiedText text={entry} />
              </pre>
            </li>
          ))}
        </ol>
      ) : (
        <Typography variant="body2" className="cview-search-empty">
          No results captured for this query.
        </Typography>
      )}
    </Box>
  );
}

export default SearchView;
