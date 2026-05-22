import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";

import type { ChatStepCard } from "../types";
import { LinkifiedText } from "./LinkifiedText";

export interface TraceEventViewProps {
  card: ChatStepCard;
}

type EventKind = "metric" | "phase" | "tool" | "reasoning" | "generic";

const TOOL_NAMES = new Set([
  "search",
  "fetch",
  "browse",
  "code_exec",
  "resource_rw",
  "memory_rw",
  "delegate",
]);

const REASONING_HINTS = ["reasoning", "thinking", "model_text", "model text"];

const PHASE_HINTS = [
  "precheck",
  "phase",
  "stage",
  "inbound",
  "outbound",
  "spine",
];

function lowerText(value: string | null | undefined): string {
  return (value || "").trim().toLowerCase();
}

function detectEventKind(card: ChatStepCard): EventKind {
  const label = lowerText(card.label);
  const kind = lowerText(card.kind);
  const stepType = lowerText(card.stepType);
  const detail = lowerText(card.detail);

  if (TOOL_NAMES.has(label) || kind.includes("tool_call")) {
    return "tool";
  }
  for (const hint of REASONING_HINTS) {
    if (kind.includes(hint) || label.includes(hint)) return "reasoning";
  }
  if (/\b(after|in|took|elapsed)\s+\d/.test(detail) && /\bms|seconds?\b/.test(detail)) {
    return "metric";
  }
  if (kind.endsWith("_completed") || kind.endsWith("_started")) {
    return "metric";
  }
  for (const hint of PHASE_HINTS) {
    if (kind.includes(hint) || label.includes(hint) || stepType.includes(hint)) {
      return "phase";
    }
  }
  return "generic";
}

function extractDurationMs(text: string): number | null {
  const match = text.match(/(\d+(?:\.\d+)?)\s*ms\b/i);
  if (match) return parseFloat(match[1]);
  const sec = text.match(/(\d+(?:\.\d+)?)\s*(?:s|sec|seconds?)\b/i);
  if (sec) return parseFloat(sec[1]) * 1000;
  return null;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(seconds >= 10 ? 0 : 1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.round(seconds % 60);
  return remaining > 0 ? `${minutes}m ${remaining}s` : `${minutes}m`;
}

function stripDurationFromLabel(text: string): string {
  return text
    .replace(/\s*after\s+\d+(?:\.\d+)?\s*ms\b\.?/i, "")
    .replace(/\s*in\s+\d+(?:\.\d+)?\s*ms\b\.?/i, "")
    .replace(/\s*took\s+\d+(?:\.\d+)?\s*ms\b\.?/i, "")
    .replace(/\s*elapsed\s+\d+(?:\.\d+)?\s*ms\b\.?/i, "")
    .replace(/\s*after\s+\d+(?:\.\d+)?\s*(?:s|sec|seconds?)\b\.?/i, "")
    .replace(/\.$/, "")
    .trim();
}

type DescriptorPair = { label: string; value: string };

function parsePayloadRecord(card: ChatStepCard): Record<string, unknown> | null {
  const sources: Array<string | undefined> = [
    card.payloadView?.body,
    card.rawDetailFull,
    card.detailFull,
  ];
  for (const source of sources) {
    const text = (source || "").trim();
    if (!text || text[0] !== "{") continue;
    try {
      const parsed = JSON.parse(text) as unknown;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      // ignore
    }
  }
  return null;
}

const HEADER_DUPLICATE_KEYS = new Set([
  "title",
  "detail",
  "details",
  "detail_full",
  "step_type",
  "stepType",
  "step",
  "icon",
  "data",
  "event_type",
  "status",
  "kind",
  "summary",
  "label",
  "id",
  "ts",
  "time",
  "timestamp",
]);

function humanizeKey(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim()
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

function valueToText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value.trim();
  if (typeof value === "number") {
    if (!Number.isFinite(value)) return "";
    return Number.isInteger(value) ? value.toLocaleString() : String(value);
  }
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (Array.isArray(value)) {
    if (value.length === 0) return "—";
    if (value.length <= 3) {
      return value.map((item) => valueToText(item)).filter(Boolean).join(", ");
    }
    return `${value.length} items`;
  }
  if (typeof value === "object") {
    const keys = Object.keys(value as object).filter((k) => !HEADER_DUPLICATE_KEYS.has(k));
    if (keys.length === 0) return "";
    return keys.slice(0, 3).join(", ") + (keys.length > 3 ? ` +${keys.length - 3}` : "");
  }
  return String(value);
}

function extractDescriptors(card: ChatStepCard, max = 6): DescriptorPair[] {
  const record = parsePayloadRecord(card);
  if (!record) return [];
  const pairs: DescriptorPair[] = [];
  for (const [key, raw] of Object.entries(record)) {
    if (pairs.length >= max) break;
    if (HEADER_DUPLICATE_KEYS.has(key) || key.startsWith("__")) continue;
    const value = valueToText(raw);
    if (!value) continue;
    pairs.push({ label: humanizeKey(key), value });
  }
  return pairs;
}

function MetricEvent({ card }: { card: ChatStepCard }) {
  const text = card.detail || card.detailFull || card.summary || "";
  const ms = extractDurationMs(text);
  const label = stripDurationFromLabel(text) || card.label;
  return (
    <Box className="cview-trace-event cview-trace-metric">
      {ms !== null ? (
        <Typography className="cview-trace-metric-value">{formatDuration(ms)}</Typography>
      ) : null}
      {label ? (
        <Typography className="cview-trace-metric-label">{label}</Typography>
      ) : null}
    </Box>
  );
}

function PhaseEvent({ card }: { card: ChatStepCard }) {
  const descriptors = extractDescriptors(card, 4);
  const summary = card.detail || card.summary || "";
  return (
    <Box className="cview-trace-event cview-trace-phase">
      {summary && descriptors.length === 0 ? (
        <Typography className="cview-trace-phase-summary">
          <LinkifiedText text={summary} />
        </Typography>
      ) : null}
      {descriptors.length > 0 ? (
        <Box className="cview-trace-inline-pairs">
          {descriptors.map((pair) => (
            <span key={pair.label} className="cview-trace-inline-pair">
              <span className="cview-trace-inline-label">{pair.label.toLowerCase()}</span>
              <span className="cview-trace-inline-sep">·</span>
              <span className="cview-trace-inline-value">{pair.value}</span>
            </span>
          ))}
        </Box>
      ) : null}
    </Box>
  );
}

function ToolEvent({ card }: { card: ChatStepCard }) {
  const descriptors = extractDescriptors(card, 6);
  const detail = card.detail || card.summary || "";
  return (
    <Box className="cview-trace-event cview-trace-tool">
      {descriptors.length > 0 ? (
        <Box className="cview-trace-keyvals">
          {descriptors.map((pair) => (
            <Box className="cview-trace-keyval" key={pair.label}>
              <span className="cview-trace-keyval-label">{pair.label.toLowerCase()}</span>
              <span className="cview-trace-keyval-value">
                <LinkifiedText text={pair.value} />
              </span>
            </Box>
          ))}
        </Box>
      ) : null}
      {detail ? (
        <Typography className="cview-trace-tool-summary">
          <LinkifiedText text={detail} />
        </Typography>
      ) : null}
    </Box>
  );
}

function ReasoningEvent({ card }: { card: ChatStepCard }) {
  const text =
    card.detailFull || card.detail || card.summary || card.rawDetailFull || "";
  return (
    <Box className="cview-trace-event cview-trace-reasoning">
      <Typography className="cview-trace-reasoning-text">
        <LinkifiedText text={text} />
      </Typography>
    </Box>
  );
}

function GenericEvent({ card }: { card: ChatStepCard }) {
  const descriptors = extractDescriptors(card, 6);
  const detail = card.detail || card.summary || "";
  return (
    <Box className="cview-trace-event cview-trace-generic">
      {detail ? (
        <Typography className="cview-trace-generic-detail">
          <LinkifiedText text={detail} />
        </Typography>
      ) : null}
      {descriptors.length > 0 ? (
        <Box className="cview-trace-keyvals">
          {descriptors.map((pair) => (
            <Box className="cview-trace-keyval" key={pair.label}>
              <span className="cview-trace-keyval-label">{pair.label.toLowerCase()}</span>
              <span className="cview-trace-keyval-value">
                <LinkifiedText text={pair.value} />
              </span>
            </Box>
          ))}
        </Box>
      ) : null}
    </Box>
  );
}

export function TraceEventView({ card }: TraceEventViewProps) {
  const kind = detectEventKind(card);
  switch (kind) {
    case "metric":
      return <MetricEvent card={card} />;
    case "phase":
      return <PhaseEvent card={card} />;
    case "tool":
      return <ToolEvent card={card} />;
    case "reasoning":
      return <ReasoningEvent card={card} />;
    default:
      return <GenericEvent card={card} />;
  }
}

export default TraceEventView;
