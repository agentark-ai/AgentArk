// Right-side "Computer" pane: one focused live artifact surface with compact
// activity history. This is intentionally closer to a runtime console
// than a second copy of the chat timeline.

import {
  memo,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import Box from "@mui/material/Box";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Collapse from "@mui/material/Collapse";
import Dialog from "@mui/material/Dialog";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import CloseIcon from "@mui/icons-material/Close";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import FiberManualRecordRoundedIcon from "@mui/icons-material/FiberManualRecordRounded";
import KeyboardArrowDownRoundedIcon from "@mui/icons-material/KeyboardArrowDownRounded";
import KeyboardArrowUpRoundedIcon from "@mui/icons-material/KeyboardArrowUpRounded";
import KeyboardArrowRightRoundedIcon from "@mui/icons-material/KeyboardArrowRightRounded";
import FolderRoundedIcon from "@mui/icons-material/FolderRounded";
import InsertDriveFileRoundedIcon from "@mui/icons-material/InsertDriveFileRounded";
import TerminalRoundedIcon from "@mui/icons-material/TerminalRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";

import type { ChatStepCard, ComputerPaneFile, ComputerPaneTab, SurfaceStatus } from "./types";
import {
  isOmittedContentPlaceholder,
  resolveComputerPaneFileContent,
} from "./computerPaneFileContent";
import { extractFilePath, pickComputerView } from "./dispatch";
import {
  AGENTARK_RENDERERS,
  rendererIdForCard,
  surfaceDisplayTitle,
  surfaceFromCard,
  surfacePayloads,
  surfaceStatus,
} from "./surface";
import {
  delegationRecordFromValue,
  humanizePayloadStatus,
  isReadableRecord as isRecord,
  readableFieldsFromRecord,
  readableNumber,
  readablePayloadFromValue,
  readableString,
  formatReadableDurationMs,
  type ReadablePayloadTone,
} from "./readablePayload";
import { buildRunPayloadViewFromSources } from "./runPayloadView";
import { buildReadableToolPresentation } from "./computerViews/presentation";
import {
  FileView,
  SurfaceRenderer,
  StatusView,
  WorkingView,
} from "./computerViews";

export interface ComputerPaneProps {
  liveCards: ChatStepCard[];
  allCards: ChatStepCard[];
  activeStepId: string | null;
  onActivate: (id: string | null) => void;
  onClose: () => void;
  /** Optional rendered node for the Activity tab (e.g. existing classic timeline). */
  activityNode?: ReactNode;
  /** Status text used as fallback heading when no step is active yet. */
  nowDoingLabel?: string;
  /** Active workspace snippet path/content (used by FileView when relevant). */
  snippetPath?: string;
  snippetContent?: string;
  isStreaming?: boolean;
  startedAt?: string | number | null;
  tokenPreview?: string;
  runMetrics?: Array<{ label: string; value: string }>;
  /** Live planner/classifier reasoning text. Surfaced by `WorkingView` as
   * a fallback while the assistant content stream has not started yet. */
  reasoningPreview?: string;
  /** Structural reasoning phase such as "classifier", "planner", or "model". */
  reasoningPhase?: string;
  taskProgress?: {
    done: number;
    total: number;
  } | null;
  showSnippet?: boolean;
  workspaceFiles?: ComputerPaneFile[];
  /** Path of a file currently being written by the agent, if any. When set
   * and the user has not manually picked a different file, the pane will
   * auto-focus this file and stream its content live. */
  liveWritePath?: string | null;
  /** Latest streamed body for `liveWritePath`. Updates while the agent
   * generates the file token-by-token. */
  liveWriteContent?: string;
  /** True while `liveWritePath` is still being written. Once the write is
   * complete the pane stops auto-following and the user can navigate freely. */
  liveWriteActive?: boolean;
}

function pickActiveCard(
  pool: ChatStepCard[],
  activeStepId: string | null,
): ChatStepCard | null {
  if (!pool || pool.length === 0) return null;
  if (activeStepId) {
    const found = pool.find((c) => c.id === activeStepId);
    if (found) return found;
  }
  for (let i = pool.length - 1; i >= 0; i -= 1) {
    if (!pool[i].isHeartbeat) return pool[i];
  }
  return pool[pool.length - 1] ?? null;
}

// Pick a terminal glyph + tone for a console story line from the card's renderer
// and status. Tones map to AgentArk accents: run=green, info=signal-blue,
// ask=orange, err=red, reason=dim.
function storyGlyphMeta(card: ChatStepCard, isReasoning: boolean): { glyph: string; tone: string } {
  if (isReasoning) return { glyph: "~", tone: "reason" };
  const rid = rendererIdForCard(card);
  const hay = `${card.kind} ${card.stepType} ${card.label}`.toLowerCase();
  let glyph = "·";
  let tone = "info";
  if (/deploy|app_deploy|publish|\bapp\b/.test(hay)) {
    glyph = "▣";
    tone = "run";
  } else if (rid === AGENTARK_RENDERERS.SEARCH) {
    glyph = "*";
    tone = "info";
  } else if (rid === AGENTARK_RENDERERS.FILE) {
    glyph = "→";
    tone = "info";
  } else if (rid === AGENTARK_RENDERERS.BROWSER) {
    glyph = "◇";
    tone = "info";
  } else if (rid === AGENTARK_RENDERERS.TERMINAL) {
    glyph = "$";
    tone = "run";
  } else if (/model|turn|reply|respond|complete|generat/.test(hay)) {
    glyph = "●";
    tone = "run";
  } else if (/ask|clarify|question|await|waiting|approval/.test(hay)) {
    glyph = "~";
    tone = "ask";
  } else if (rid === AGENTARK_RENDERERS.WORKING) {
    glyph = "~";
    tone = "run";
  }
  const status = surfaceStatus(card);
  if (status === "error") {
    glyph = "✕";
    tone = "err";
  } else if (status === "done" && glyph === "·") {
    glyph = "✓";
    tone = "run";
  }
  return { glyph, tone };
}

function truncateStorySub(text: string, limit = 96): string {
  const trimmed = (text || "").replace(/\s+/g, " ").trim();
  return trimmed.length > limit ? `${trimmed.slice(0, limit - 1)}…` : trimmed;
}

// Readable one-line subtitle for a story line. Never surface raw JSON: if the
// best candidate is a JSON blob, pull its activity_label, otherwise show nothing
// (the glyph + label already carry the line; the expanded detail has the rest).
function storySubText(
  presentation: { query: string; summary: string },
  card: ChatStepCard,
): string {
  const raw = (presentation.query || presentation.summary || card.summary || "").trim();
  if (!raw) return "";
  if (raw[0] === "{" || raw[0] === "[") {
    const match = raw.match(/"activity_label"\s*:\s*"([^"]+)"/);
    return match ? match[1] : "";
  }
  return raw;
}

function looksLikeStructuredPanePayload(text: string): boolean {
  const trimmed = (text || "").trim();
  if (!trimmed) return false;
  return (
    ((trimmed.startsWith("{") || trimmed.startsWith("[")) &&
      /["}\]]\s*[:,]|^\{\s*"|^\[\s*(\{|"|\])/.test(trimmed)) ||
    /^<artifact\b/i.test(trimmed)
  );
}

function safePaneText(value: string, fallback = ""): string {
  const trimmed = (value || "").replace(/\s+/g, " ").trim();
  if (!trimmed) return fallback;
  if (looksLikeStructuredPanePayload(trimmed)) return fallback;
  return trimmed.length > 180 ? `${trimmed.slice(0, 177).trimEnd()}...` : trimmed;
}

function activityListCardsEqual(left: ChatStepCard, right: ChatStepCard): boolean {
  return (
    left.id === right.id &&
    left.index === right.index &&
    left.stepType === right.stepType &&
    left.rawTitle === right.rawTitle &&
    left.tone === right.tone &&
    left.kind === right.kind &&
    left.label === right.label &&
    left.detail === right.detail &&
    left.detailFull === right.detailFull &&
    left.summary === right.summary &&
    left.rawDetailFull === right.rawDetailFull &&
    left.traceJson === right.traceJson &&
    left.time === right.time &&
    left.payloadView?.body === right.payloadView?.body &&
    left.payloadView?.preview === right.payloadView?.preview
  );
}

const ActivityListRow = memo(function ActivityListRow({
  card,
  isActive,
  expanded,
  onToggle,
}: {
  card: ChatStepCard;
  isActive: boolean;
  expanded: boolean;
  onToggle: (id: string) => void;
}) {
  const time = card.time || "";
  const detail = safePaneText(card.summary || card.detail || "");
  const detailsId = `computer-pane-activity-details-${card.id}`.replace(
    /[^a-zA-Z0-9_-]+/g,
    "-",
  );

  return (
    <li
      className={`computer-pane-activity-row tone-${card.tone}${isActive ? " is-active" : ""}${expanded ? " is-expanded" : ""}`}
    >
      <button
        type="button"
        className="computer-pane-activity-button"
        aria-expanded={expanded}
        aria-controls={detailsId}
        onClick={() => onToggle(card.id)}
      >
        <span className="computer-pane-activity-kind">
          {card.kind || "Update"}
        </span>
        <span className="computer-pane-activity-label">{card.label}</span>
        {detail ? (
          <span className="computer-pane-activity-detail">{detail}</span>
        ) : null}
        {time ? (
          <span className="computer-pane-activity-time">{time}</span>
        ) : null}
        <KeyboardArrowDownRoundedIcon
          fontSize="small"
          className={`computer-pane-activity-chevron${expanded ? " is-expanded" : ""}`}
          aria-hidden="true"
        />
      </button>
      <Collapse in={expanded} mountOnEnter unmountOnExit>
        <ActivityExpandedDetails card={card} detailsId={detailsId} />
      </Collapse>
    </li>
  );
}, (prev, next) =>
  prev.isActive === next.isActive &&
  prev.expanded === next.expanded &&
  activityListCardsEqual(prev.card, next.card),
);

function ActivityList({
  cards,
  activeStepId,
}: {
  cards: ChatStepCard[];
  activeStepId: string | null;
}) {
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());

  const toggleExpanded = useCallback((id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  if (!cards || cards.length === 0) {
    return (
      <Box className="computer-pane-activity-empty">
        <Typography variant="body2" className="computer-pane-activity-empty-copy">
          No activity yet. When AgentArk runs a tool, the steps land here.
        </Typography>
      </Box>
    );
  }
  return (
    <ol className="computer-pane-activity-list">
      {cards.map((card) => {
        const isActive = card.id === activeStepId;
        const expanded = expandedIds.has(card.id);
        return (
          <ActivityListRow
            key={`activity-${card.id}`}
            card={card}
            isActive={isActive}
            expanded={expanded}
            onToggle={toggleExpanded}
          />
        );
      })}
    </ol>
  );
}

type ActivityDisplayField = {
  label: string;
  value: string;
};

type JsonRecord = Record<string, unknown>;

const COMPUTER_ACTIVITY_INTERNAL_FIELDS = new Set([
  "__omitted_keys",
  "__streamKey",
  "agent_id",
  "chat_visible",
  "conversation_id",
  "delegation_id",
  "id",
  "plan_id",
  "plan_revision",
  "plan_step_id",
  "run_id",
  "task_id",
  "trace_id",
  "ts",
]);

function isComputerActivityInternalField(key: string): boolean {
  return key.startsWith("__") || COMPUTER_ACTIVITY_INTERNAL_FIELDS.has(key);
}

function formatActivityFieldLabel(value: string): string {
  const normalized = (value || "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized) return "Value";
  return normalized
    .split(" ")
    .map((part) =>
      part.length <= 3 && part === part.toLowerCase()
        ? part.toUpperCase()
        : `${part.charAt(0).toUpperCase()}${part.slice(1)}`,
    )
    .join(" ");
}

function activityDisplayText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value.trim();
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return "None";
    const scalarItems = value
      .slice(0, 6)
      .map(activityDisplayText)
      .filter(Boolean);
    if (scalarItems.length === Math.min(value.length, 6)) {
      const suffix = value.length > scalarItems.length ? ` +${value.length - scalarItems.length} more` : "";
      return `${scalarItems.join(", ")}${suffix}`;
    }
    return `${value.length} item${value.length === 1 ? "" : "s"}`;
  }
  const record = asPaneRecord(value);
  const keys = Object.keys(record);
  if (keys.length === 0) return "";
  return `${keys.slice(0, 5).map(formatActivityFieldLabel).join(", ")}${keys.length > 5 ? ` +${keys.length - 5} more` : ""}`;
}

function collectActivityFields(
  value: unknown,
  options?: { prefix?: string; depth?: number; limit?: number },
): ActivityDisplayField[] {
  const prefix = options?.prefix || "";
  const depth = options?.depth ?? 0;
  const limit = options?.limit ?? 18;
  const record = asPaneRecord(value);
  const fields: ActivityDisplayField[] = [];
  for (const [key, fieldValue] of Object.entries(record)) {
    if (fields.length >= limit) break;
    if (isComputerActivityInternalField(key)) continue;
    const fieldLabel = prefix
      ? `${prefix} ${formatActivityFieldLabel(key)}`
      : formatActivityFieldLabel(key);
    const child = asPaneRecord(fieldValue);
    const shouldFlatten =
      depth < 2 &&
      Object.keys(child).length > 0 &&
      ["data", "payload", "arguments", "args", "input", "output", "result"].includes(
        key.trim().toLowerCase(),
      );
    if (shouldFlatten) {
      const nested = collectActivityFields(child, {
        prefix,
        depth: depth + 1,
        limit: limit - fields.length,
      });
      fields.push(...nested);
      continue;
    }
    const text = activityDisplayText(fieldValue);
    if (!text) continue;
    fields.push({ label: fieldLabel, value: text });
  }
  return fields;
}

function buildActivityDetails(card: ChatStepCard) {
  const record = tryParseRecord(card.traceJson || "") || {};
  const data = asPaneRecord(record.data);
  const payloadView = buildRunPayloadViewFromSources(
    Object.keys(data).length > 0 ? data : null,
    Object.keys(record).length > 0 ? record : null,
    card.rawDetailFull,
    card.detailFull,
  );
  const readable = readablePayloadFromValue(data) || readablePayloadFromValue(record);
  const rawOverview =
    payloadView?.preview ||
    readable?.detail ||
    str(data.content_snapshot, "") ||
    str(data.content, "") ||
    str(record.detail, "") ||
    card.rawDetailFull ||
    card.detailFull ||
    card.summary ||
    card.detail ||
    "Activity update.";
  const overview =
    readablePayloadFromValue(rawOverview)?.detail ||
    (looksLikeStructuredPanePayload(rawOverview)
      ? readablePayloadFromValue(rawOverview)?.title || "Received structured activity details."
      : rawOverview);
  const statusFields: ActivityDisplayField[] = [
    { label: "Status", value: card.kind || "Update" },
    { label: "Title", value: card.label || card.rawTitle || "Activity update" },
    { label: "Step Type", value: card.stepType },
    { label: "Time", value: card.time },
  ].filter((field) => field.value.trim());
  const traceFields =
    payloadView?.items && payloadView.items.length > 0
      ? payloadView.items
      : readable?.fields && readable.fields.length > 0
      ? readable.fields
      : collectActivityFields(Object.keys(data).length > 0 ? data : record, {
          limit: 20,
        });
  const displayFields = [
    ...statusFields,
    ...traceFields.filter(
      (field) =>
        !statusFields.some(
          (existing) =>
            existing.label === field.label && existing.value === field.value,
        ),
    ),
  ];
  const copyText = [
    `Overview:\n${overview}`,
    displayFields
      .map((field) => `${field.label}:\n${field.value}`)
      .join("\n\n"),
  ]
    .filter(Boolean)
    .join("\n\n");
  const rawPayload =
    payloadView?.body ||
    (looksLikeStructuredPanePayload(card.rawDetailFull)
      ? card.rawDetailFull
      : looksLikeStructuredPanePayload(card.detailFull)
        ? card.detailFull
        : card.traceJson || "");
  return { overview, displayFields, traceJson: rawPayload, copyText };
}

function ActivityExpandedDetails({
  card,
  detailsId,
}: {
  card: ChatStepCard;
  detailsId: string;
}) {
  const [copied, setCopied] = useState(false);
  const details = useMemo(() => buildActivityDetails(card), [card]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copyActivity = async () => {
    if (!details.copyText) return;
    try {
      await navigator.clipboard.writeText(details.copyText);
      setCopied(true);
    } catch {
      // Clipboard access can be unavailable outside secure browser contexts.
    }
  };

  return (
    <Box id={detailsId} className="computer-pane-activity-expanded">
      <Stack
        direction="row"
        spacing={0.5}
        className="computer-pane-activity-expanded-head"
        sx={{ alignItems: "center", justifyContent: "space-between" }}
      >
        <span className="computer-pane-activity-expanded-title">
          Activity details
        </span>
        <Tooltip
          title={copied ? "Copied" : "Copy activity"}
          placement="top"
          arrow
        >
          <span>
            <IconButton
              size="small"
              className="computer-pane-activity-copy"
              disabled={!details.copyText}
              onClick={copyActivity}
              aria-label="Copy activity details"
            >
              <ContentCopyRoundedIcon fontSize="inherit" />
            </IconButton>
          </span>
        </Tooltip>
      </Stack>
      <Box className="computer-pane-activity-readable">
        <Box className="computer-pane-activity-overview">
          <span className="computer-pane-activity-overview-label">
            What happened
          </span>
          <p>{details.overview}</p>
        </Box>
        {details.displayFields.length > 0 ? (
          <Box className="computer-pane-activity-field-grid">
            {details.displayFields.map((field, index) => (
              <Box
                key={`${detailsId}-field-${index}`}
                className="computer-pane-activity-field"
              >
                <span className="computer-pane-activity-field-label">
                  {field.label}
                </span>
                <span className="computer-pane-activity-field-value">
                  {field.value}
                </span>
              </Box>
            ))}
          </Box>
        ) : null}
        {details.traceJson ? (
          <details className="computer-pane-activity-raw">
            <summary>Raw payload</summary>
            <pre className="computer-pane-activity-full">{details.traceJson}</pre>
          </details>
        ) : null}
      </Box>
    </Box>
  );
}

function normalizePath(value: string): string {
  return (value || "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+/, "")
    .toLowerCase();
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function tryParseRecord(raw: string): Record<string, unknown> | null {
  const trimmed = (raw || "").trim();
  if (!trimmed || !trimmed.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function asPaneRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function paneRecordFromMaybeJson(value: unknown): Record<string, unknown> {
  if (typeof value === "string") return tryParseRecord(value) || {};
  return asPaneRecord(value);
}

function structuredCardRecord(card: ChatStepCard): Record<string, unknown> | null {
  return (
    tryParseRecord(card.payloadView?.body || "") ||
    tryParseRecord(card.rawDetailFull || "") ||
    tryParseRecord(card.detailFull || "") ||
    tryParseRecord(card.traceJson || "") ||
    null
  );
}

function reasoningCardPhase(card: ChatStepCard): string {
  const record = structuredCardRecord(card);
  const data = paneRecordFromMaybeJson(record?.data);
  return str(record?.phase, str(data.phase, str(record?.step_type, ""))).trim();
}

function reasoningCardMetadataContent(card: ChatStepCard): string {
  const record = structuredCardRecord(card) || {};
  const data = paneRecordFromMaybeJson(record.data);
  const merged: Record<string, unknown> = { ...record, ...data };
  const lines: string[] = [];
  const seen = new Set<string>();
  const push = (label: string, value: unknown) => {
    const text = activityDisplayText(value);
    if (!text) return;
    const key = `${label}:${text}`.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    lines.push(`${label}: ${text}`);
  };

  push("Step", card.label || card.rawTitle || card.stepType);
  push("Kind", card.kind);
  push("Time", card.time);
  for (const field of collectActivityFields(merged, { limit: 32 })) {
    push(field.label, field.value);
  }

  const detail = str(record.detail, str(data.detail, card.detail || card.summary)).trim();
  push("Detail", detail);
  return lines.join("\n");
}

function reasoningCardContent(card: ChatStepCard): string {
  const record = structuredCardRecord(card);
  const data = paneRecordFromMaybeJson(record?.data);
  // Prefer the full streamed chain-of-thought (content_snapshot/content/delta)
  // over a possibly-short plain-text detail, so the Thinking step shows the real
  // reasoning rather than a one-line phase summary. directDetail stays as a
  // fallback for cards whose reasoning lives only in plain text.
  const snapshot =
    str(record?.content_snapshot, "") ||
    str(data.content_snapshot, "") ||
    str(record?.content, "") ||
    str(data.content, "") ||
    str(record?.content_delta, "") ||
    str(data.content_delta, "");
  if (snapshot.trim()) return snapshot;
  const directDetail =
    [card.rawDetailFull, card.detailFull].find(
      (value) => value.trim() && !looksLikeStructuredPanePayload(value),
    ) || "";
  if (directDetail) return directDetail;
  return (
    card.rawDetailFull ||
    card.detailFull ||
    reasoningCardMetadataContent(card) ||
    str(record?.detail, "") ||
    str(data.detail, "") ||
    str(record?.text, "") ||
    str(data.text, "") ||
    card.detail ||
    card.summary ||
    ""
  );
}

// Phases that denote genuine model chain-of-thought. A card only counts as
// reasoning via the heuristic branch if its phase is one of these AND it carries
// no tool/phase identity — so phase/tool steps (e.g. inbound_precheck) can never
// be mistaken for reasoning just because their payload happens to carry content.
const VISIBLE_REASONING_PHASES = new Set([
  "model",
  "model_summary",
  "reasoning",
  "reasoning_summary",
]);

function isReasoningOnlyCard(card: ChatStepCard): boolean {
  const record = structuredCardRecord(card);
  const data = paneRecordFromMaybeJson(record?.data);
  const kind = str(record?.kind, str(data.kind, "")).trim().toLowerCase();
  const phase = str(record?.phase, str(data.phase, "")).trim().toLowerCase();
  const stepType = (card.stepType || "").trim().toLowerCase();
  const recordStepType = str(record?.step_type, str(data.step_type, ""))
    .trim()
    .toLowerCase();
  if (
    kind === "reasoning_delta" ||
    stepType === "reasoning_delta" ||
    recordStepType === "reasoning_delta"
  ) {
    return true;
  }
  // A tool/phase step carries an identity (tool_name/name/file/path or a `stage`,
  // e.g. inbound_precheck) — never reasoning, even if its payload has `content`.
  const stage = str(record?.stage, str(data.stage, "")).trim();
  const hasToolOrPhaseIdentity =
    Boolean(str(record?.tool_name, "")) ||
    Boolean(str(record?.name, "")) ||
    Boolean(str(record?.file, "")) ||
    Boolean(str(record?.path, "")) ||
    Boolean(stage);
  return Boolean(
    record &&
      !hasToolOrPhaseIdentity &&
      VISIBLE_REASONING_PHASES.has(phase) &&
      (str(record.content, "") ||
        str(record.content_delta, "") ||
        str(record.content_snapshot, "")),
  );
}

// A reasoning row only earns a place in the story when it carries actual
// chain-of-thought. Placeholder steps whose entire body is empty or merely
// restates the row's own label ("Thinking") are structural noise — they appear
// while a reasoning stream is still warming up or when the runtime emits a
// phase marker with no content. Filtering is content-based, so the same card
// appears as soon as real reasoning text arrives.
function reasoningCardHasMeaningfulContent(card: ChatStepCard): boolean {
  const normalize = (value: string) =>
    value.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
  const body = normalize(reasoningCardContent(card));
  if (!body) return false;
  const label = normalize(card.label || card.rawTitle || card.stepType || "");
  return body !== label;
}

// Wider than isReasoningOnlyCard: identifies every card that PRESENTS as a
// "Thinking" row — including contentless phase markers and synthetic thinking
// steps that the strict classifier (which requires content) misses. Cards with
// a tool/stage identity are never reasoning-shaped.
function isReasoningShapedCard(card: ChatStepCard): boolean {
  if (isReasoningOnlyCard(card)) return true;
  const stepType = (card.stepType || "").trim().toLowerCase();
  if (stepType === "thinking" || stepType === "reasoning_delta") return true;
  const record = structuredCardRecord(card);
  const data = paneRecordFromMaybeJson(record?.data);
  const phase = str(record?.phase, str(data.phase, "")).trim().toLowerCase();
  const stage = str(record?.stage, str(data.stage, "")).trim();
  const hasToolOrPhaseIdentity =
    Boolean(str(record?.tool_name, "")) ||
    Boolean(str(record?.name, "")) ||
    Boolean(stage);
  return !hasToolOrPhaseIdentity && VISIBLE_REASONING_PHASES.has(phase);
}

// The agent runtime wraps every progress / streaming step in an envelope
// shaped like `{flow_kind, tool_name, run_id, seq, ts, content, ...}` where
// `content` is a short progress message ("Drafting vite.config.ts"), NOT the
// file body. If we naively used that envelope as file content the FileView
// would render the wrapper JSON to the user. Detect & refuse it.
const STEP_ENVELOPE_KEYS = ["flow_kind", "tool_name", "run_id", "seq", "ts"];
function isStepEnvelope(record: Record<string, unknown>): boolean {
  let hits = 0;
  for (const key of STEP_ENVELOPE_KEYS) {
    if (key in record) hits += 1;
    if (hits >= 3) return true;
  }
  return false;
}

function pickCardBody(card: ChatStepCard): string {
  return (
    card.payloadView?.body ||
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    ""
  );
}

function contentFromRecordForPath(
  record: Record<string, unknown>,
  targetPath: string,
): string {
  const target = normalizePath(targetPath);
  const recordPath = normalizePath(
    str(record.path, str(record.file, str(record.name, ""))),
  );
  const directContent = str(
    record.raw_content,
    str(record.file_content, str(record.content_snapshot, str(record.content, ""))),
  );
  if (recordPath && (recordPath === target || recordPath.endsWith(`/${target}`))) {
    return directContent;
  }
  const files = record.files;
  if (files && typeof files === "object" && !Array.isArray(files)) {
    for (const [path, content] of Object.entries(files as Record<string, unknown>)) {
      const normalized = normalizePath(path);
      if (normalized === target || normalized.endsWith(`/${target}`)) {
        return str(content, "");
      }
    }
  }
  return "";
}

function findFileContentForPath(cards: ChatStepCard[], path: string): string {
  const target = normalizePath(path);
  if (!target) return "";
  for (let idx = cards.length - 1; idx >= 0; idx -= 1) {
    const card = cards[idx];
    const body = pickCardBody(card);
    const parsed = tryParseRecord(body);
    // Step envelopes never carry the file body in a useful form; their
    // `content` is a progress string. Skip them so we don't fall back to
    // rendering the envelope JSON as if it were the file.
    if (parsed && isStepEnvelope(parsed)) continue;
    if (parsed) {
      const parsedContent = contentFromRecordForPath(parsed, path);
      if (parsedContent.trim()) return parsedContent;
    }
    const cardPath = normalizePath(extractFilePath(card));
    if (cardPath && (cardPath === target || cardPath.endsWith(`/${target}`))) {
      // Only return the raw body when it doesn't parse as JSON. A parseable
      // body without an inner content/files match is almost always a wrapper,
      // never the file we want.
      if (!parsed) return body;
    }
  }
  return "";
}

function findWorkspaceFileContent(
  files: ComputerPaneFile[],
  path: string,
): string {
  const target = normalizePath(path);
  if (!target) return "";
  for (let idx = files.length - 1; idx >= 0; idx -= 1) {
    const file = files[idx];
    const filePath = normalizePath(file.path);
    if (filePath && (filePath === target || filePath.endsWith(`/${target}`))) {
      return isOmittedContentPlaceholder(file.content || "") ? "" : file.content || "";
    }
  }
  return "";
}

function filePathsMatch(left: string, right: string): boolean {
  const lhs = normalizePath(left);
  const rhs = normalizePath(right);
  if (!lhs || !rhs) return false;
  return lhs === rhs || lhs.endsWith(`/${rhs}`) || rhs.endsWith(`/${lhs}`);
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function fileNameFromPath(path: string): string {
  const normalized = (path || "").replace(/\\/g, "/").trim();
  return normalized.split("/").filter(Boolean).pop() || normalized || "file";
}

function fileDisplayPath(file: ComputerPaneFile): string {
  return (file.displayPath || file.path || "").replace(/\\/g, "/").trim();
}

function workspaceFileMeta(file: ComputerPaneFile, live: boolean): string {
  if (live) return "writing";
  const lineCount = file.content ? file.content.split(/\r?\n/).length : 0;
  const byteCount = new Blob([file.content || ""]).size;
  if (lineCount > 0) {
    return `${lineCount} line${lineCount === 1 ? "" : "s"} / ${formatBytes(byteCount)}`;
  }
  return "queued";
}

interface WorkspaceFileRow {
  file: ComputerPaneFile;
  live: boolean;
  selected: boolean;
  name: string;
  meta: string;
}

interface MutableWorkspaceFileFolder {
  kind: "folder";
  name: string;
  path: string;
  folders: Map<string, MutableWorkspaceFileFolder>;
  files: WorkspaceFileRow[];
  selected: boolean;
  live: boolean;
}

interface WorkspaceFileFolderNode {
  kind: "folder";
  name: string;
  path: string;
  children: WorkspaceFileTreeNode[];
  fileCount: number;
  selected: boolean;
  live: boolean;
}

interface WorkspaceFileLeafNode {
  kind: "file";
  row: WorkspaceFileRow;
}

type WorkspaceFileTreeNode = WorkspaceFileFolderNode | WorkspaceFileLeafNode;

function createWorkspaceFileFolder(name: string, path: string): MutableWorkspaceFileFolder {
  return {
    kind: "folder",
    name,
    path,
    folders: new Map(),
    files: [],
    selected: false,
    live: false,
  };
}

function sortWorkspaceFileRows(rows: WorkspaceFileRow[]): WorkspaceFileRow[] {
  return [...rows].sort((a, b) =>
    normalizePath(fileDisplayPath(a.file)).localeCompare(
      normalizePath(fileDisplayPath(b.file)),
    ),
  );
}

function materializeWorkspaceFileFolder(
  folder: MutableWorkspaceFileFolder,
): WorkspaceFileFolderNode {
  const childFolders = [...folder.folders.values()]
    .map(materializeWorkspaceFileFolder)
    .sort((a, b) => a.name.localeCompare(b.name));
  const childFiles = sortWorkspaceFileRows(folder.files).map<WorkspaceFileLeafNode>((row) => ({
    kind: "file",
    row,
  }));
  const children: WorkspaceFileTreeNode[] = [...childFolders, ...childFiles];
  const nestedFileCount = childFolders.reduce((sum, child) => sum + child.fileCount, 0);
  return {
    kind: "folder",
    name: folder.name,
    path: folder.path,
    children,
    fileCount: nestedFileCount + childFiles.length,
    selected: folder.selected || childFolders.some((child) => child.selected),
    live: folder.live || childFolders.some((child) => child.live),
  };
}

function buildWorkspaceFileTree(rows: WorkspaceFileRow[]): WorkspaceFileTreeNode[] {
  const root = createWorkspaceFileFolder("", "");
  for (const row of rows) {
    const normalized = normalizePath(fileDisplayPath(row.file));
    const segments = normalized.split("/").filter(Boolean);
    if (segments.length <= 1) {
      root.files.push(row);
      root.selected = root.selected || row.selected;
      root.live = root.live || row.live;
      continue;
    }

    let folder = root;
    for (let index = 0; index < segments.length - 1; index += 1) {
      const name = segments[index];
      const path = segments.slice(0, index + 1).join("/");
      let next = folder.folders.get(name);
      if (!next) {
        next = createWorkspaceFileFolder(name, path);
        folder.folders.set(name, next);
      }
      next.selected = next.selected || row.selected;
      next.live = next.live || row.live;
      folder = next;
    }
    folder.files.push(row);
    folder.selected = folder.selected || row.selected;
    folder.live = folder.live || row.live;
  }
  return materializeWorkspaceFileFolder(root).children;
}

function syntheticFileCard(source: ChatStepCard, path: string): ChatStepCard {
  return {
    ...source,
    id: `${source.id}:file:${path}`,
    stepType: "file_read",
    kind: "File",
    label: path,
    detail: "",
    detailFull: "",
    rawDetailFull: "",
    summary: "",
    payloadView: null,
    surface: {
      protocolVersion: 1,
      renderer: {
        id: AGENTARK_RENDERERS.FILE,
        version: 1,
        fallback: "generic-artifact",
      },
      call: {
        runId: surfaceFromCard(source)?.call.runId,
        callId: `${surfaceFromCard(source)?.call.callId || source.id}:file:${path}`,
        sequence: surfaceFromCard(source)?.call.sequence,
      },
      tool: {
        id: "workspace_file",
        displayName: "File",
      },
      status: "done",
      title: path,
      artifacts: [
        {
          id: `file:${path}`,
          role: "file",
          contentType: "text/plain",
          path,
          label: path,
        },
      ],
    },
  };
}

function progressLabel(done: number, total: number): string {
  if (total <= 0) return "Task Progress";
  return `Task Progress ${done}/${total}`;
}

function statusTone(status: SurfaceStatus | null): "working" | "waiting" | "error" | "idle" {
  if (status === "error") return "error";
  if (status === "waiting" || status === "pending") return "waiting";
  if (status === "running") return "working";
  return "idle";
}

type DelegationPaneAgent = {
  id: string;
  name: string;
  role: string;
  model: string;
  task: string;
  status: string;
  update: string;
  elapsed: string;
  specialist: boolean;
  sequence: number;
  dependencyCount: number;
  resolvedDependencyCount: number;
  memoryCount: number;
  actionCount: number;
  contextMode: string;
  restored: boolean;
  outputPreview: string;
};

type DelegationPaneRun = {
  id: string;
  request: string;
  status: string;
  summary: string;
  agentCount: number;
  agents: DelegationPaneAgent[];
  updatedAtIndex: number;
};

function readableBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  const text = readableString(value).trim().toLowerCase();
  return text === "true" || text === "1" || text === "yes";
}

function normalizeDelegationStatus(value: unknown, fallback = "running"): string {
  const status = readableString(value, String(value ?? "")).trim().toLowerCase();
  if (!status) return fallback;
  if (status === "cancelled" || status === "canceled") return "interrupted";
  if (status === "timeout" || status === "timed out") return "timed_out";
  return status;
}

function delegationStatusTone(status: string): ReadablePayloadTone {
  const normalized = normalizeDelegationStatus(status, "running");
  if (["completed", "success", "done"].includes(normalized)) return "success";
  if (["failed", "timed_out", "panicked", "interrupted", "error"].includes(normalized)) {
    return "error";
  }
  if (["partial", "degraded"].includes(normalized)) return "warning";
  if (["assigned", "running", "synthesizing"].includes(normalized)) return "running";
  return "idle";
}

function delegationStatusLabel(status: string): string {
  const normalized = normalizeDelegationStatus(status, "running");
  switch (normalized) {
    case "assigned":
      return "Assigned";
    case "running":
      return "Running";
    case "synthesizing":
      return "Synthesizing";
    case "completed":
    case "success":
    case "done":
      return "Completed";
    case "partial":
      return "Partial";
    case "timed_out":
      return "Timed out";
    case "panicked":
      return "Panicked";
    case "failed":
    case "error":
      return "Failed";
    case "interrupted":
      return "Stopped";
    default:
      return humanizePayloadStatus(normalized || "queued", "Queued");
  }
}

function delegationRunStatus(agents: DelegationPaneAgent[], fallback = "running"): string {
  if (agents.some((agent) => ["assigned", "running", "synthesizing"].includes(normalizeDelegationStatus(agent.status)))) {
    return "running";
  }
  if (
    agents.length > 0 &&
    agents.every((agent) =>
      ["completed", "success", "done"].includes(normalizeDelegationStatus(agent.status)),
    )
  ) {
    return "completed";
  }
  if (
    agents.some((agent) =>
      ["completed", "success", "done"].includes(normalizeDelegationStatus(agent.status)),
    )
  ) {
    return "partial";
  }
  if (agents.some((agent) => normalizeDelegationStatus(agent.status) === "timed_out")) {
    return "timed_out";
  }
  if (agents.some((agent) => normalizeDelegationStatus(agent.status) === "failed")) {
    return "failed";
  }
  return normalizeDelegationStatus(fallback);
}

function cleanDelegationUpdate(value: string): string {
  const text = (value || "").replace(/\s+/g, " ").trim();
  if (!text || /^\[omitted\s+\d+\s+chars?\]$/i.test(text)) return "";
  if (looksLikeStructuredPanePayload(text)) {
    return readablePayloadFromValue(text)?.detail || "Received structured update.";
  }
  return text.length > 260 ? `${text.slice(0, 257).trimEnd()}...` : text;
}

function compactDelegationText(value: string, maxLen = 180): string {
  const text = (value || "").replace(/\s+/g, " ").trim();
  if (!text) return "";
  return text.length > maxLen ? `${text.slice(0, Math.max(0, maxLen - 3)).trimEnd()}...` : text;
}

function fullDelegationText(value: string): string {
  return (value || "")
    .replace(/\r\n/g, "\n")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .trim();
}

type DelegationStructuredGoal = {
  title: string;
  detail: string;
  criteria: string[];
};

type DelegationStructuredContext = {
  requirements: string[];
  goals: DelegationStructuredGoal[];
};

type ExtractedJsonBlock = {
  start: number;
  end: number;
  value: unknown;
};

function extractJsonBlocks(text: string): ExtractedJsonBlock[] {
  const blocks: ExtractedJsonBlock[] = [];
  let start = -1;
  let depth = 0;
  let inString = false;
  let escaped = false;

  for (let index = 0; index < text.length; index += 1) {
    const ch = text[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === "\\") {
        escaped = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }

    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "{") {
      if (depth === 0) start = index;
      depth += 1;
      continue;
    }
    if (ch !== "}" || depth === 0) continue;
    depth -= 1;
    if (depth === 0 && start >= 0) {
      const raw = text.slice(start, index + 1);
      try {
        blocks.push({ start, end: index + 1, value: JSON.parse(raw) });
      } catch {
        // Ignore malformed embedded JSON and keep it in the prose fallback.
      }
      start = -1;
    }
  }

  return blocks;
}

function stringsFromUnknown(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        if (typeof item === "string") return fullDelegationText(item);
        const record = isRecord(item) ? item : {};
        return fullDelegationText(
          readableString(record.text) ||
            readableString(record.outcome) ||
            readableString(record.title) ||
            readableString(record.summary),
        );
      })
      .filter(Boolean);
  }
  if (typeof value === "string") {
    const text = fullDelegationText(value);
    return text ? [text] : [];
  }
  return [];
}

function structuredContextFromRecord(record: Record<string, unknown>): DelegationStructuredContext {
  const requirements = stringsFromUnknown(record.requirements);
  const goals = Array.isArray(record.goals)
    ? record.goals
        .map((item): DelegationStructuredGoal | null => {
          if (!isRecord(item)) return null;
          const title = fullDelegationText(
            readableString(item.outcome) ||
              readableString(item.title) ||
              readableString(item.summary),
          );
          const detail = fullDelegationText(
            readableString(item.capability_need) ||
              readableString(item.detail) ||
              readableString(item.description),
          );
          const criteria = stringsFromUnknown(item.success_criteria);
          if (!title && !detail && criteria.length === 0) return null;
          return {
            title: title || detail || "Delegated goal",
            detail: detail && detail !== title ? detail : "",
            criteria,
          };
        })
        .filter((item): item is DelegationStructuredGoal => Boolean(item))
    : [];
  return { requirements, goals };
}

function mergeStructuredContexts(
  left: DelegationStructuredContext,
  right: DelegationStructuredContext,
): DelegationStructuredContext {
  const requirementSet = new Set(left.requirements);
  const requirements = [...left.requirements];
  for (const item of right.requirements) {
    if (!requirementSet.has(item)) {
      requirementSet.add(item);
      requirements.push(item);
    }
  }

  const goalSet = new Set(left.goals.map((goal) => `${goal.title}\n${goal.detail}`));
  const goals = [...left.goals];
  for (const goal of right.goals) {
    const key = `${goal.title}\n${goal.detail}`;
    if (!goalSet.has(key)) {
      goalSet.add(key);
      goals.push(goal);
    }
  }
  return { requirements, goals };
}

function extractedStructuredContext(blocks: ExtractedJsonBlock[]): DelegationStructuredContext {
  return blocks.reduce<DelegationStructuredContext>(
    (acc, block) =>
      isRecord(block.value)
        ? mergeStructuredContexts(acc, structuredContextFromRecord(block.value))
        : acc,
    { requirements: [], goals: [] },
  );
}

function textWithoutJsonBlocks(text: string, blocks: ExtractedJsonBlock[]): string {
  if (blocks.length === 0) return text;
  let out = "";
  let cursor = 0;
  for (const block of blocks) {
    out += text.slice(cursor, block.start);
    cursor = block.end;
  }
  out += text.slice(cursor);
  return out
    .replace(/\b[A-Z][A-Za-z _-]{0,40}:\s*$/gm, "")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function displayLinesFromDelegationText(text: string): string[] {
  const expanded = fullDelegationText(text)
    .replace(/\\n/g, "\n")
    .replace(/\s+(#{1,6}\s+)/g, "\n\n$1")
    .replace(/\s+(\*\*[^*\n]{1,90}\*\*:)/g, "\n$1")
    .replace(/\s+(\*\*[^*\n]{1,90}\*\*)/g, "\n$1")
    .replace(/\s+([-*]\s+)/g, "\n$1")
    .replace(/\s+(\d+\.\s+)/g, "\n$1");

  return expanded
    .split(/\n+/)
    .map((line) =>
      line
        .replace(/^#{1,6}\s*/, "")
        .replace(/\*\*/g, "")
        .replace(/\s+/g, " ")
        .trim(),
    )
    .filter(Boolean);
}

function DelegationReadableText({
  text,
  empty,
}: {
  text: string;
  empty: string;
}) {
  const prepared = useMemo(() => {
    const normalized = fullDelegationText(text);
    const blocks = extractJsonBlocks(normalized);
    const structured = extractedStructuredContext(blocks);
    const prose = textWithoutJsonBlocks(normalized, blocks);
    const lines = displayLinesFromDelegationText(prose);
    return { structured, lines };
  }, [text]);

  if (
    prepared.lines.length === 0 &&
    prepared.structured.requirements.length === 0 &&
    prepared.structured.goals.length === 0
  ) {
    return <p className="delegation-readable-empty">{empty}</p>;
  }

  return (
    <Box className="delegation-readable-text">
      {prepared.lines.length > 0 ? (
        <Box className="delegation-readable-section">
          {prepared.lines.map((line, index) => {
            const isBullet = /^[-*]\s+/.test(line) || /^\d+\.\s+/.test(line);
            return (
              <p
                className={isBullet ? "delegation-readable-bullet" : "delegation-readable-line"}
                key={`delegation-line-${index}`}
              >
                {line.replace(/^[-*]\s+/, "").replace(/^\d+\.\s+/, "")}
              </p>
            );
          })}
        </Box>
      ) : null}
      {prepared.structured.requirements.length > 0 ? (
        <Box className="delegation-readable-section">
          <span className="delegation-readable-heading">Requirements</span>
          {prepared.structured.requirements.map((item, index) => (
            <p className="delegation-readable-line" key={`delegation-req-${index}`}>
              {item}
            </p>
          ))}
        </Box>
      ) : null}
      {prepared.structured.goals.length > 0 ? (
        <Box className="delegation-readable-goals">
          {prepared.structured.goals.map((goal, index) => (
            <Box className="delegation-readable-goal" key={`delegation-goal-${index}`}>
              <span className="delegation-readable-goal-index">{index + 1}</span>
              <Box className="delegation-readable-goal-copy">
                <span className="delegation-readable-goal-title">{goal.title}</span>
                {goal.detail ? (
                  <span className="delegation-readable-goal-detail">{goal.detail}</span>
                ) : null}
                {goal.criteria.length > 0 ? (
                  <ul className="delegation-readable-criteria">
                    {goal.criteria.map((criterion, criterionIndex) => (
                      <li key={`delegation-goal-${index}-criterion-${criterionIndex}`}>
                        {criterion}
                      </li>
                    ))}
                  </ul>
                ) : null}
              </Box>
            </Box>
          ))}
        </Box>
      ) : null}
    </Box>
  );
}

function delegationRecordRunId(record: JsonRecord): string {
  return (
    readableString(record.delegation_id).trim() ||
    readableString(record.run_id).trim() ||
    "delegation"
  );
}

function delegationRecordsFromCard(card: ChatStepCard): JsonRecord[] {
  const candidates: unknown[] = [
    structuredCardRecord(card),
    tryParseRecord(card.traceJson || ""),
    card.payloadView?.body,
    card.rawDetailFull,
    card.detailFull,
  ];
  for (const item of surfacePayloads(card)) {
    candidates.push(item.json, item.text, item.preview);
  }
  const records: JsonRecord[] = [];
  const seen = new Set<string>();
  for (const candidate of candidates) {
    const record = delegationRecordFromValue(candidate);
    if (!record) continue;
    const key = [
      readableString(record.kind),
      readableString(record.delegation_id),
      readableString(record.agent_id),
      readableString(record.agent_name),
      readableString(record.status),
      readableString(record.reason),
      readableString(record.task),
    ].join("|");
    if (seen.has(key)) continue;
    seen.add(key);
    records.push(record);
  }
  return records;
}

function cardHasDelegationPayload(card: ChatStepCard | null): boolean {
  return Boolean(card && delegationRecordsFromCard(card).length > 0);
}

function delegationRunIdFromCard(card: ChatStepCard | null): string {
  if (!card) return "";
  const record = delegationRecordsFromCard(card)[0];
  return record ? delegationRecordRunId(record) : "";
}

function buildDelegationRunsFromCards(cards: ChatStepCard[]): DelegationPaneRun[] {
  const runs = new Map<
    string,
    {
      id: string;
      request: string;
      status: string;
      summary: string;
      agentCount: number;
      agents: Map<string, DelegationPaneAgent>;
      order: string[];
      updatedAtIndex: number;
    }
  >();

  cards.forEach((card, index) => {
    for (const payload of delegationRecordsFromCard(card)) {
      const kind = readableString(payload.kind).trim().toLowerCase();
      const runId = delegationRecordRunId(payload);
      let run = runs.get(runId);
      if (!run) {
        run = {
          id: runId,
          request: "",
          status: "running",
          summary: "",
          agentCount: 0,
          agents: new Map<string, DelegationPaneAgent>(),
          order: [],
          updatedAtIndex: index,
        };
        runs.set(runId, run);
      }

      const readable = readablePayloadFromValue(payload);
      run.updatedAtIndex = index;
      run.request = readableString(payload.request, run.request).trim() || run.request;
      run.summary =
        cleanDelegationUpdate(readableString(payload.summary)) ||
        readable?.detail ||
        run.summary;
      run.agentCount = Math.max(run.agentCount, readableNumber(payload.agent_count, 0));

      if (kind === "delegation_started") {
        run.status = normalizeDelegationStatus(payload.status, "running");
      } else if (kind === "delegation_synthesis_started") {
        run.status = normalizeDelegationStatus(payload.status, "synthesizing");
      } else if (kind === "delegation_completed") {
        run.status = normalizeDelegationStatus(payload.status, "completed");
      }

      const agentName = readableString(payload.agent_name).trim();
      const agentRole = readableString(payload.agent_role).trim();
      const agentId =
        readableString(payload.agent_id).trim() ||
        [agentName, agentRole, readableString(payload.sequence)].filter(Boolean).join(":");
      if (!agentId) continue;

      let agent = run.agents.get(agentId);
      if (!agent) {
        agent = {
          id: agentId,
          name: agentName || "Agent",
          role: agentRole,
          model: readableString(payload.model_name).trim(),
          task: readableString(payload.task).trim(),
          status: "assigned",
          update: "",
          elapsed: "",
          specialist: readableBool(payload.is_specialist),
          sequence: Math.max(1, readableNumber(payload.sequence, run.order.length + 1)),
          dependencyCount: 0,
          resolvedDependencyCount: 0,
          memoryCount: 0,
          actionCount: 0,
          contextMode: "",
          restored: false,
          outputPreview: "",
        };
        run.agents.set(agentId, agent);
        run.order.push(agentId);
      }

      agent.name = agentName || agent.name;
      agent.role = agentRole || agent.role;
      agent.model = readableString(payload.model_name, agent.model).trim() || agent.model;
      agent.task = readableString(payload.task, agent.task).trim() || agent.task;
      agent.specialist = readableBool(payload.is_specialist) || agent.specialist;
      agent.elapsed = formatReadableDurationMs(payload.elapsed_ms) || agent.elapsed;
      agent.dependencyCount = Math.max(
        agent.dependencyCount,
        readableNumber(payload.dependency_count, 0),
      );
      agent.resolvedDependencyCount = Math.max(
        agent.resolvedDependencyCount,
        readableNumber(payload.resolved_dependency_count, 0),
      );
      agent.memoryCount = Math.max(agent.memoryCount, readableNumber(payload.memory_count, 0));
      agent.actionCount = Math.max(agent.actionCount, readableNumber(payload.action_count, 0));
      agent.contextMode = readableString(payload.context_mode, agent.contextMode).trim() || agent.contextMode;
      agent.restored = readableBool(payload.restored) || agent.restored;
      agent.outputPreview =
        fullDelegationText(readableString(payload.output_preview)) ||
        agent.outputPreview;
      agent.update =
        cleanDelegationUpdate(readableString(payload.content)) ||
        cleanDelegationUpdate(readableString(payload.summary)) ||
        readable?.detail ||
        agent.update;

      if (kind === "delegation_assignment") {
        agent.status = "assigned";
      } else if (kind === "delegation_agent_started" || kind === "delegation_agent_progress") {
        agent.status = normalizeDelegationStatus(payload.status, "running");
      } else if (kind === "delegation_agent_completed") {
        agent.status = normalizeDelegationStatus(payload.status, "completed");
      } else if (kind === "delegation_agent_failed") {
        agent.status = normalizeDelegationStatus(
          payload.status,
          /timeout/i.test(readableString(payload.reason)) ? "timed_out" : "failed",
        );
        agent.update =
          cleanDelegationUpdate(readableString(payload.reason)) ||
          agent.update ||
          "This delegated agent did not finish.";
      }
    }
  });

  return Array.from(runs.values())
    .map((run) => {
      const agents = run.order
        .map((agentId) => run.agents.get(agentId))
        .filter((agent): agent is DelegationPaneAgent => Boolean(agent))
        .sort((left, right) => left.sequence - right.sequence);
      return {
        id: run.id,
        request: run.request,
        status: delegationRunStatus(agents, run.status),
        summary: run.summary,
        agentCount: Math.max(run.agentCount, agents.length),
        agents,
        updatedAtIndex: run.updatedAtIndex,
      };
    })
    .sort((left, right) => right.updatedAtIndex - left.updatedAtIndex);
}

function delegationAgentKey(runId: string, agentId: string): string {
  return `${runId}::${agentId}`;
}

function agentContextLine(agent: DelegationPaneAgent): string {
  const contextMode = agent.contextMode.trim();
  const parts = [
    contextMode || "Own delegated context",
    agent.dependencyCount > 0
      ? `${agent.resolvedDependencyCount}/${agent.dependencyCount} dependencies`
      : "",
    agent.memoryCount > 0 ? `${agent.memoryCount} memories` : "",
    agent.actionCount > 0 ? `${agent.actionCount} tools` : "",
  ].filter(Boolean);
  return parts.join(" · ");
}

function delegationAgentTypeLabel(agent: DelegationPaneAgent): string {
  const role = agent.role.trim();
  if (role) return agent.specialist ? `${role} specialist` : role;
  return agent.specialist ? "Specialist agent" : "Delegated agent";
}

function DelegationRunView({
  cards,
  activeRunId,
}: {
  cards: ChatStepCard[];
  activeRunId?: string;
}) {
  const runs = useMemo(() => buildDelegationRunsFromCards(cards), [cards]);
  const visibleRuns = useMemo(() => {
    if (!activeRunId) return runs.slice(0, 1);
    const selected = runs.find((run) => run.id === activeRunId);
    return selected ? [selected] : runs.slice(0, 1);
  }, [activeRunId, runs]);
  const [selectedAgentKey, setSelectedAgentKey] = useState<string | null>(null);
  const selectedAgent = useMemo(() => {
    if (!selectedAgentKey) return null;
    for (const run of visibleRuns) {
      const agent = run.agents.find(
        (candidate) => delegationAgentKey(run.id, candidate.id) === selectedAgentKey,
      );
      if (agent) return { run, agent };
    }
    return null;
  }, [selectedAgentKey, visibleRuns]);

  useEffect(() => {
    if (selectedAgentKey && !selectedAgent) setSelectedAgentKey(null);
  }, [selectedAgent, selectedAgentKey]);

  if (runs.length === 0) {
    return (
      <StatusView
        title="Agent swarm"
        detail="Delegated agent activity will appear here."
      />
    );
  }

  const totalAgents = visibleRuns.reduce(
    (sum, run) => sum + Math.max(run.agentCount, run.agents.length),
    0,
  );
  const allAgents = visibleRuns.flatMap((run) => run.agents);
  const completedAgents = allAgents.filter((agent) =>
    ["completed", "success", "done"].includes(normalizeDelegationStatus(agent.status)),
  ).length;
  const runningAgents = allAgents.filter((agent) =>
    ["assigned", "running", "synthesizing"].includes(normalizeDelegationStatus(agent.status)),
  ).length;
  const issueAgents = allAgents.filter((agent) =>
    ["failed", "timed_out", "panicked", "interrupted", "error"].includes(
      normalizeDelegationStatus(agent.status),
    ),
  ).length;

  return (
    <Box className="delegation-console-view">
      <Box className="delegation-console-head">
        <Box>
          <Typography variant="subtitle2" className="delegation-console-title">
            Agent swarm
          </Typography>
          <Typography variant="body2" className="delegation-console-detail">
            {visibleRuns.length} active run{visibleRuns.length === 1 ? "" : "s"} with {totalAgents} delegated agent{totalAgents === 1 ? "" : "s"}.
          </Typography>
        </Box>
        <Box className="delegation-run-metrics" aria-label="Agent swarm summary">
          <span className="delegation-run-metric">
            <span className="delegation-run-metric-label">Done</span>
            <span className="delegation-run-metric-value">{completedAgents}</span>
          </span>
          <span className="delegation-run-metric">
            <span className="delegation-run-metric-label">Running</span>
            <span className="delegation-run-metric-value">{runningAgents}</span>
          </span>
          <span className="delegation-run-metric">
            <span className="delegation-run-metric-label">Needs attention</span>
            <span className="delegation-run-metric-value">{issueAgents}</span>
          </span>
        </Box>
      </Box>
      <Stack spacing={1.1}>
        {visibleRuns.map((run) => (
          <Box className="delegation-run" key={run.id}>
            <Box className="delegation-run-head">
              <Box className="delegation-run-copy">
                <Typography variant="body2" className="delegation-run-title">
                  {compactDelegationText(run.request || "Delegated run", 180)}
                </Typography>
                <Typography variant="caption" className="delegation-run-summary">
                  {compactDelegationText(run.summary || `${run.agents.length} delegated agents tracked.`, 180)}
                </Typography>
              </Box>
              <span className={`delegation-status-pill tone-${delegationStatusTone(run.status)}`}>
                {delegationStatusLabel(run.status)}
              </span>
            </Box>
            <Box className="delegation-run-progress" aria-hidden="true">
              {Array.from({ length: Math.max(run.agentCount, run.agents.length, 1) }).map(
                (_, index) => {
                  const agent = run.agents[index];
                  const tone = delegationStatusTone(agent?.status || run.status);
                  return (
                    <span
                      className={`delegation-run-progress-segment tone-${tone}`}
                      key={`${run.id}-progress-${index}`}
                    />
                  );
                },
              )}
            </Box>
            <Typography variant="caption" className="delegation-section-label">
              Agents launched
            </Typography>
            <Box className="delegation-agent-list">
              {run.agents.map((agent) => {
                const agentTitle = agent.role
                  ? `${agent.name} / ${agent.role}`
                  : agent.name;
                const contextLine = agentContextLine(agent);
                return (
                  <button
                    type="button"
                    className="delegation-agent-row"
                    key={agent.id}
                    onClick={() => setSelectedAgentKey(delegationAgentKey(run.id, agent.id))}
                    aria-label={`Open details for ${agentTitle}`}
                  >
                    <span className="delegation-agent-row-main">
                      <span
                        className={`delegation-agent-dot tone-${delegationStatusTone(agent.status)}`}
                        aria-hidden="true"
                      />
                      <span className="delegation-agent-row-copy">
                        <span className="delegation-agent-name">{agentTitle}</span>
                        <span className="delegation-agent-subtitle">
                          {delegationAgentTypeLabel(agent)}
                        </span>
                        <span className="delegation-agent-task">
                          {compactDelegationText(agent.task || agent.update || "Waiting for task details.", 150)}
                        </span>
                      </span>
                    </span>
                    <span className="delegation-agent-row-meta">
                      {agent.elapsed ? (
                        <span className="delegation-agent-mini">{agent.elapsed}</span>
                      ) : null}
                      {contextLine ? (
                        <span className="delegation-agent-context">
                          {compactDelegationText(contextLine, 90)}
                        </span>
                      ) : null}
                      <span className={`delegation-status-pill tone-${delegationStatusTone(agent.status)}`}>
                        {delegationStatusLabel(agent.status)}
                      </span>
                      <ChevronRightRoundedIcon fontSize="small" aria-hidden="true" />
                    </span>
                  </button>
                );
              })}
            </Box>
          </Box>
        ))}
      </Stack>
      <Dialog
        open={Boolean(selectedAgent)}
        onClose={() => setSelectedAgentKey(null)}
        maxWidth="md"
        fullWidth
        slotProps={{ paper: { className: "delegation-agent-dialog-paper" } }}
      >
        <DialogTitle className="delegation-agent-dialog-title">
          <Box className="delegation-agent-dialog-heading">
            <Typography variant="subtitle2" className="delegation-console-title">
              {selectedAgent
                ? selectedAgent.agent.role
                  ? `${selectedAgent.agent.name} / ${selectedAgent.agent.role}`
                  : selectedAgent.agent.name
                : "Agent details"}
            </Typography>
            <Typography variant="caption" className="delegation-console-detail">
              {selectedAgent ? delegationAgentTypeLabel(selectedAgent.agent) : "Delegated agent"}
            </Typography>
          </Box>
          <IconButton
            size="small"
            className="delegation-agent-dialog-close"
            onClick={() => setSelectedAgentKey(null)}
            aria-label="Close agent details"
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers className="delegation-agent-dialog-content">
          {selectedAgent ? (
            <Stack spacing={1.4}>
              <Box className="delegation-agent-dialog-fields">
                {readableFieldsFromRecord(
                  {
                    status: delegationStatusLabel(selectedAgent.agent.status),
                    elapsed: selectedAgent.agent.elapsed,
                    agent_type: selectedAgent.agent.specialist ? "Specialist" : "Generalist",
                    context_mode: selectedAgent.agent.contextMode || "Delegated context",
                    dependencies:
                      selectedAgent.agent.dependencyCount > 0
                        ? `${selectedAgent.agent.resolvedDependencyCount}/${selectedAgent.agent.dependencyCount}`
                        : "",
                    memory_count: selectedAgent.agent.memoryCount || "",
                    action_count: selectedAgent.agent.actionCount || "",
                    restored: selectedAgent.agent.restored ? "Restored checkpoint" : "",
                  },
                  12,
                ).map((field, index) => (
                  <Box
                    className="delegation-agent-dialog-field"
                    key={`${selectedAgent.agent.id}-dialog-field-${index}`}
                  >
                    <span className="delegation-agent-dialog-field-label">
                      {field.label}
                    </span>
                    <span className="delegation-agent-dialog-field-value">
                      {field.value}
                    </span>
                  </Box>
                ))}
              </Box>
              <Box className="delegation-agent-dialog-section">
                <span className="delegation-section-label">Agent output</span>
                <DelegationReadableText
                  text={selectedAgent.agent.outputPreview || selectedAgent.agent.update}
                  empty="Waiting for an output update."
                />
              </Box>
              <Box className="delegation-agent-dialog-section">
                <span className="delegation-section-label">Assigned task</span>
                <DelegationReadableText
                  text={selectedAgent.agent.task}
                  empty="No task text was provided."
                />
              </Box>
            </Stack>
          ) : null}
        </DialogContent>
      </Dialog>
    </Box>
  );
}

const COMPUTER_PANE_TAB_LABEL: Record<ComputerPaneTab, string> = {
  computer: "Console",
  files: "Files",
  activity: "Activity",
};

function ComputerPaneInner({
  liveCards,
  allCards,
  activeStepId,
  onActivate,
  onClose,
  activityNode,
  nowDoingLabel,
  snippetPath,
  snippetContent,
  isStreaming,
  startedAt,
  tokenPreview,
  runMetrics = [],
  reasoningPreview,
  reasoningPhase,
  taskProgress = null,
  showSnippet,
  workspaceFiles = [],
  liveWritePath = null,
  liveWriteContent = "",
  liveWriteActive = false,
}: ComputerPaneProps) {
  const [tab, setTab] = useState<ComputerPaneTab>("computer");
  const [deployFilePath, setDeployFilePath] = useState<string | null>(null);
  const [userPickedDeployFile, setUserPickedDeployFile] = useState(false);
  const [filesListCollapsed, setFilesListCollapsed] = useState(false);
  const [expandedFileFolders, setExpandedFileFolders] = useState<Set<string>>(
    () => new Set(),
  );
  const filesListId = useId();
  const lastLiveWritePathRef = useRef<string | null>(null);
  const hasWorkspaceFiles = workspaceFiles.length > 0;
  const snippetFileAvailable = Boolean(showSnippet && (snippetPath || snippetContent));
  const deployFilePathAvailable = Boolean(
    deployFilePath &&
      ((!!liveWritePath && filePathsMatch(deployFilePath, liveWritePath)) ||
        workspaceFiles.some((file) => filePathsMatch(file.path, deployFilePath)) ||
        (!!snippetFileAvailable &&
          !!snippetPath &&
          filePathsMatch(snippetPath, deployFilePath))),
  );
  const selectedDeployFilePath = deployFilePathAvailable ? deployFilePath : null;
  const hasFileTab =
    hasWorkspaceFiles || Boolean(liveWritePath || selectedDeployFilePath) || snippetFileAvailable;
  const autoFocusFilePath = liveWritePath || workspaceFiles[0]?.path || null;
  const followedLiveWritePath =
    liveWriteActive && liveWritePath && !userPickedDeployFile
      ? liveWritePath
      : "";

  // While a file is actively being written, auto-focus it in the pane so the
  // user watches the code stream in (Bolt/Lovable-style) without having to
  // click. Stops following once the user picks a different file manually,
  // and re-engages on the next live write. When restored after a refresh,
  // keep the last captured workspace file open even if the write has finished.
  useEffect(() => {
    if (!autoFocusFilePath) return;
    if (userPickedDeployFile) return;
    if (!liveWriteActive && selectedDeployFilePath) return;
    setDeployFilePath(autoFocusFilePath);
  }, [
    autoFocusFilePath,
    liveWriteActive,
    selectedDeployFilePath,
    userPickedDeployFile,
  ]);
  useEffect(() => {
    if (!deployFilePath || deployFilePathAvailable) return;
    setDeployFilePath(null);
    setUserPickedDeployFile(false);
  }, [deployFilePath, deployFilePathAvailable]);
  useEffect(() => {
    if (!liveWriteActive) setUserPickedDeployFile(false);
  }, [liveWriteActive]);
  useEffect(() => {
    if (!liveWriteActive || !liveWritePath) return;
    const normalized = normalizePath(liveWritePath);
    if (lastLiveWritePathRef.current === normalized) return;
    lastLiveWritePathRef.current = normalized;
    setUserPickedDeployFile(false);
    setDeployFilePath(liveWritePath);
    setTab("files");
  }, [liveWriteActive, liveWritePath]);
  useEffect(() => {
    if (tab === "files" && !hasFileTab) setTab("computer");
  }, [hasFileTab, tab]);
  useEffect(() => {
    if (liveWriteActive && hasFileTab) setTab("files");
  }, [hasFileTab, liveWriteActive]);

  const cardsForRun = useMemo(
    () => (liveCards.length > 0 ? liveCards : allCards),
    [liveCards, allCards],
  );
  const activityCards = useMemo(
    () => cardsForRun.filter((card) => !card.isHeartbeat),
    [cardsForRun],
  );
  const reasoningCardIds = useMemo(() => {
    const ids = new Set<string>();
    for (const card of activityCards) {
      if (isReasoningOnlyCard(card)) ids.add(card.id);
    }
    return ids;
  }, [activityCards]);
  const primaryActivityCards = useMemo(
    () => activityCards.filter((card) => !reasoningCardIds.has(card.id)),
    [activityCards, reasoningCardIds],
  );
  const navPool = useMemo(
    () =>
      activityCards.filter(
        (card) =>
          !isReasoningShapedCard(card) ||
          reasoningCardHasMeaningfulContent(card),
      ),
    [activityCards],
  );
  const headerFilePath =
    followedLiveWritePath ||
    selectedDeployFilePath ||
    liveWritePath ||
    workspaceFiles[0]?.path ||
    snippetPath ||
    "";
  const workspaceFileRows = useMemo(
    () =>
      workspaceFiles.map((file) => {
        const live =
          !!liveWritePath &&
          filePathsMatch(file.path, liveWritePath) &&
          liveWriteActive;
        const selected =
          !!headerFilePath && filePathsMatch(file.path, headerFilePath);
        return {
          file,
          live,
          selected,
          name: fileNameFromPath(fileDisplayPath(file)),
          meta: workspaceFileMeta(file, live),
        };
      }),
    [headerFilePath, liveWriteActive, liveWritePath, workspaceFiles],
  );
  const workspaceFileTree = useMemo(
    () => buildWorkspaceFileTree(workspaceFileRows),
    [workspaceFileRows],
  );
  const headerDisplayFilePath = useMemo(() => {
    if (!headerFilePath) return "";
    return (
      workspaceFiles.find((file) => filePathsMatch(file.path, headerFilePath))
        ?.displayPath || headerFilePath
    );
  }, [headerFilePath, workspaceFiles]);

  const activeCard = useMemo(
    () => pickActiveCard(navPool, activeStepId),
    [navPool, activeStepId],
  );
  const latestActivityCard = useMemo(
    () => pickActiveCard(primaryActivityCards, activeStepId),
    [primaryActivityCards, activeStepId],
  );

  // Index of the pinned step (−1 in follow mode); used by progress + live-surface
  // checks. The story is the navigation now, so there is no prev/next chrome.
  const activeIndex = useMemo(
    () => (activeStepId != null ? navPool.findIndex((c) => c.id === activeStepId) : -1),
    [navPool, activeStepId],
  );

  const view = activeCard ? pickComputerView(activeCard) : AGENTARK_RENDERERS.GENERIC;
  const activeCardIsReasoning = activeCard
    ? reasoningCardIds.has(activeCard.id) || isReasoningOnlyCard(activeCard)
    : false;
  const delegationCardsForRun = useMemo(
    () => cardsForRun.filter(cardHasDelegationPayload),
    [cardsForRun],
  );
  const activeCardHasDelegation = activeCard ? cardHasDelegationPayload(activeCard) : false;
  const activeDelegationRunId = useMemo(
    () => delegationRunIdFromCard(activeCard),
    [activeCard],
  );
  const delegationCardsForActiveRun = useMemo(() => {
    if (!activeDelegationRunId) return delegationCardsForRun;
    const filtered = delegationCardsForRun.filter(
      (card) => delegationRunIdFromCard(card) === activeDelegationRunId,
    );
    return filtered.length > 0 ? filtered : delegationCardsForRun;
  }, [activeDelegationRunId, delegationCardsForRun]);
  const activeSurfaceStatus = activeCard ? surfaceStatus(activeCard, Boolean(isStreaming)) : null;
  const fileHeaderText =
    liveWriteActive && headerDisplayFilePath
      ? `Writing ${headerDisplayFilePath}`
      : headerDisplayFilePath
        ? `Files: ${headerDisplayFilePath}`
        : "Files";
  const consoleHeaderText = activeCard
    ? surfaceDisplayTitle(activeCard)
    : nowDoingLabel || "Working";
  const headerText = safePaneText(
    tab === "files" && hasFileTab ? fileHeaderText : consoleHeaderText,
    "Working",
  );
  const completedCount = navPool.filter(
    (card) => surfaceStatus(card, false) === "done",
  ).length;
  const fileCount = hasWorkspaceFiles ? workspaceFiles.length : hasFileTab ? 1 : 0;
  const fileProgressText =
    fileCount > 0
      ? `${fileCount} file${fileCount === 1 ? "" : "s"}`
      : "Files";
  const consoleProgressText =
    taskProgress && taskProgress.total > 0
      ? progressLabel(
          Math.max(0, Math.min(taskProgress.done, taskProgress.total)),
          taskProgress.total,
        )
      : progressLabel(
          Math.max(completedCount, activeIndex + 1, 0),
          navPool.length,
        );
  const progressText =
    tab === "files" && hasFileTab ? fileProgressText : consoleProgressText;
  const deployFileIsLiveWrite =
    !!selectedDeployFilePath &&
    !!liveWritePath &&
    filePathsMatch(selectedDeployFilePath, liveWritePath);
  const deployFileContent = useMemo(() => {
    if (!selectedDeployFilePath) return "";
    return resolveComputerPaneFileContent({
      workspaceContent: findWorkspaceFileContent(workspaceFiles, selectedDeployFilePath),
      fallbackContent: findFileContentForPath(cardsForRun, selectedDeployFilePath),
      liveWriteContent,
      isLiveWrite: deployFileIsLiveWrite,
      liveWriteActive,
    });
  }, [
    cardsForRun,
    deployFileIsLiveWrite,
    liveWriteActive,
    liveWriteContent,
    selectedDeployFilePath,
    workspaceFiles,
  ]);
  const focusedFilePath =
    followedLiveWritePath ||
    selectedDeployFilePath ||
    (liveWriteActive ? liveWritePath || "" : "");
  const focusedFileIsLiveWrite =
    !!focusedFilePath &&
    !!liveWritePath &&
    filePathsMatch(focusedFilePath, liveWritePath);
  const focusedFileContent = useMemo(() => {
    if (!focusedFilePath) return "";
    if (selectedDeployFilePath && filePathsMatch(focusedFilePath, selectedDeployFilePath)) {
      return deployFileContent;
    }
    return resolveComputerPaneFileContent({
      workspaceContent: findWorkspaceFileContent(workspaceFiles, focusedFilePath),
      fallbackContent: findFileContentForPath(cardsForRun, focusedFilePath),
      liveWriteContent,
      isLiveWrite: focusedFileIsLiveWrite,
      liveWriteActive,
    });
  }, [
    cardsForRun,
    deployFileContent,
    focusedFileIsLiveWrite,
    focusedFilePath,
    liveWriteActive,
    liveWriteContent,
    selectedDeployFilePath,
    workspaceFiles,
  ]);
  const activeFilePath =
    activeCard && view === AGENTARK_RENDERERS.FILE
      ? extractFilePath(activeCard) || activeCard.label || ""
      : "";
  const activeFileIsLiveWrite =
    !!activeFilePath &&
    !!liveWritePath &&
    filePathsMatch(activeFilePath, liveWritePath);
  const activeFileContent = useMemo(() => {
    if (!activeFilePath) return "";
    return resolveComputerPaneFileContent({
      workspaceContent: findWorkspaceFileContent(workspaceFiles, activeFilePath),
      fallbackContent: findFileContentForPath(cardsForRun, activeFilePath),
      liveWriteContent,
      isLiveWrite: activeFileIsLiveWrite,
      liveWriteActive,
    });
  }, [
    activeFilePath,
    activeFileIsLiveWrite,
    cardsForRun,
    liveWriteActive,
    liveWriteContent,
    workspaceFiles,
  ]);
  const activeSurfaceLive =
    view === AGENTARK_RENDERERS.FILE
      ? activeFileIsLiveWrite && liveWriteActive
      : Boolean(isStreaming) && activeIndex === navPool.length - 1;
  const fallbackFilePath =
    !activeCard && (selectedDeployFilePath || liveWritePath || workspaceFiles[0]?.path)
      ? selectedDeployFilePath || liveWritePath || workspaceFiles[0]?.path || ""
      : "";
  const fallbackFileIsLiveWrite =
    !!fallbackFilePath &&
    !!liveWritePath &&
    filePathsMatch(fallbackFilePath, liveWritePath);
  const fallbackFileContent = useMemo(() => {
    if (!fallbackFilePath) return "";
    return resolveComputerPaneFileContent({
      workspaceContent: findWorkspaceFileContent(workspaceFiles, fallbackFilePath),
      fallbackContent: findFileContentForPath(cardsForRun, fallbackFilePath),
      liveWriteContent,
      isLiveWrite: fallbackFileIsLiveWrite,
      liveWriteActive,
    });
  }, [
    cardsForRun,
    fallbackFileIsLiveWrite,
    fallbackFilePath,
    liveWriteActive,
    liveWriteContent,
    workspaceFiles,
  ]);
  const fallbackFileSourceCard = latestActivityCard || activeCard;
  const fallbackFileCard =
    fallbackFilePath && fallbackFileSourceCard
      ? syntheticFileCard(fallbackFileSourceCard, fallbackFilePath)
      : fallbackFilePath
        ? syntheticFileCard(
            {
              id: "workspace-file",
              index: 0,
              stepType: "file_read",
              rawTitle: "",
              tone: "default",
              kind: "File",
              label: fallbackFilePath,
              detail: "",
              detailFull: "",
              summary: "",
              rawDetailFull: "",
              payloadView: null,
              isHeartbeat: false,
              time: "",
            },
            fallbackFilePath,
          )
        : null;
  const focusedFileSourceCard = activeCard || latestActivityCard;
  const focusedFileCard =
    focusedFilePath && focusedFileSourceCard
      ? syntheticFileCard(focusedFileSourceCard, focusedFilePath)
      : focusedFilePath
        ? syntheticFileCard(
            {
              id: "workspace-live-file",
              index: 0,
              stepType: "file_read",
              rawTitle: "",
              tone: "default",
              kind: "File",
              label: focusedFilePath,
              detail: "",
              detailFull: "",
              summary: "",
              rawDetailFull: "",
              payloadView: null,
              isHeartbeat: false,
              time: "",
            },
            focusedFilePath,
          )
        : null;
  const snippetCard =
    showSnippet && (snippetPath || snippetContent)
      ? syntheticFileCard(
          activeCard || latestActivityCard || {
            id: "workspace-snippet",
            index: 0,
            stepType: "file_read",
            rawTitle: "",
            tone: "default",
            kind: "File",
            label: snippetPath || "Code",
            detail: "",
            detailFull: "",
            summary: "",
            rawDetailFull: "",
            payloadView: null,
            isHeartbeat: false,
            time: "",
          },
          snippetPath || "Code",
        )
      : null;
  const copyStepText = useCallback((text: string) => {
    const value = (text || "").trim();
    if (!value) return;
    try {
      void navigator.clipboard?.writeText(value);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }, []);
  // Scroll container for the console story; auto-follows the newest step while
  // streaming and not pinned to a specific step.
  const storyScrollRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (activeStepId != null || !isStreaming) return;
    const el = storyScrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    // Re-pin on streamed-text growth (reasoning/now-doing), not just new cards.
  }, [
    activeStepId,
    isStreaming,
    navPool.length,
    reasoningPreview,
    nowDoingLabel,
    latestActivityCard?.detail,
  ]);

  // Activity is folded into the Console story; the pane is Console + Files only.
  const visibleTabs: ComputerPaneTab[] = hasFileTab
    ? ["computer", "files"]
    : ["computer"];
  // Only treat the active step as "expanded" when it is genuinely in the pool, so
  // expansion and the trailing live indicator stay mutually exclusive (never blank).
  const storyExpandedId =
    activeStepId != null && navPool.some((card) => card.id === activeStepId)
      ? activeStepId
      : null;
  const headerModeLabel =
    tab === "files" && hasFileTab
      ? "Live files"
      : activeCard
        ? surfaceFromCard(activeCard)?.tool?.displayName ||
          surfaceFromCard(activeCard)?.renderer.id ||
          "Artifact"
        : isStreaming
          ? "Live output"
          : "";
  const toggleFileFolder = (path: string) => {
    setExpandedFileFolders((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };
  const renderWorkspaceFileNode = (
    node: WorkspaceFileTreeNode,
    depth: number,
  ): ReactNode => {
    const depthStyle = { "--file-tree-indent": `${depth * 16}px` } as CSSProperties;
    if (node.kind === "folder") {
      const expanded = expandedFileFolders.has(node.path);
      return (
        <div
          key={`folder:${node.path}`}
          className={`computer-pane-file-tree-folder${expanded ? " is-expanded" : ""}${node.selected ? " is-selected" : ""}${node.live ? " is-live" : ""}`}
        >
          <button
            type="button"
            className="computer-pane-file-folder"
            style={depthStyle}
            onClick={() => toggleFileFolder(node.path)}
            aria-expanded={expanded}
            title={node.path}
          >
            <span className="computer-pane-file-folder-chevron" aria-hidden="true">
              {expanded ? (
                <KeyboardArrowDownRoundedIcon fontSize="inherit" />
              ) : (
                <KeyboardArrowRightRoundedIcon fontSize="inherit" />
              )}
            </span>
            <FolderRoundedIcon
              fontSize="inherit"
              className="computer-pane-file-folder-icon"
              aria-hidden="true"
            />
            <span className="computer-pane-file-folder-name">{node.name}</span>
            <span className="computer-pane-file-folder-meta">
              {node.fileCount} file{node.fileCount === 1 ? "" : "s"}
            </span>
          </button>
          {expanded ? (
            <div className="computer-pane-file-folder-children">
              {node.children.map((child) => renderWorkspaceFileNode(child, depth + 1))}
            </div>
          ) : null}
        </div>
      );
    }

    const { file, live, selected, name, meta } = node.row;
    const displayPath = fileDisplayPath(file) || file.path;
    return (
      <button
        key={`file:${file.path}`}
        type="button"
        className={`computer-pane-file-pill is-tree-file${selected ? " is-selected" : ""}${live ? " is-live" : ""}`}
        style={depthStyle}
        onClick={() => {
          setUserPickedDeployFile(!filePathsMatch(file.path, liveWritePath || ""));
          setDeployFilePath(file.path);
          onActivate(null);
        }}
        title={displayPath}
      >
        <InsertDriveFileRoundedIcon
          fontSize="inherit"
          className="computer-pane-file-icon"
          aria-hidden="true"
        />
        <span className="computer-pane-file-name">{name}</span>
        <span className="computer-pane-file-path">{displayPath}</span>
        <span className="computer-pane-file-meta">{meta}</span>
      </button>
    );
  };

  return (
    <Box
      className="computer-pane"
      sx={{
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        height: "100%",
      }}
    >
      <Stack
        direction="row"
        spacing={1}
        className="computer-pane-toolbar"
        sx={{ alignItems: "center" }}
      >
        <Box className="computer-pane-title">
          <Box className="computer-pane-brand-row">
            <TerminalRoundedIcon fontSize="small" className="computer-pane-brand-icon" />
            <Typography variant="subtitle2" className="computer-pane-heading">
              AgentArk's Console
            </Typography>
          </Box>
          {/* Progress/status subtitle removed: the
              "Task Progress X/Y | Run Completed | Artifact" line was
              inaccurate and not useful. */}
        </Box>
        <Box sx={{ flex: 1 }} />
        <Box className="computer-pane-tabs" role="tablist" aria-label="Console pane view">
          {visibleTabs.map((value) => {
            const active = tab === value;
            return (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={active}
                className={`computer-pane-tab${active ? " is-active" : ""}`}
                onClick={() => setTab(value)}
              >
                {COMPUTER_PANE_TAB_LABEL[value]}
              </button>
            );
          })}
        </Box>
        <Tooltip title="Close console">
          <IconButton
            size="small"
            aria-label="Close AgentArk Console"
            onClick={onClose}
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>

      {tab === "computer" ? (
        <Box
          className="computer-pane-body computer-pane-body-computer"
          sx={{
            flex: 1,
            minHeight: 0,
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          }}
        >
          <Box
            ref={storyScrollRef}
            className="computer-pane-stage"
            sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
          >
            {navPool.length === 0 ? (
              isStreaming || latestActivityCard || reasoningPreview ? (
                <WorkingView
                  phaseLabel={nowDoingLabel || "Working..."}
                  detail={latestActivityCard?.detail || latestActivityCard?.summary || ""}
                  startedAt={startedAt}
                  tokenPreview={tokenPreview}
                  reasoningPreview={reasoningPreview}
                  reasoningPhase={reasoningPhase}
                />
              ) : (
                <StatusView
                  title="Idle"
                  detail="When AgentArk runs a tool, its live output will land here."
                />
              )
            ) : (
              <div className="term-story">
                {navPool.map((card) => {
                  const expanded = storyExpandedId != null && card.id === storyExpandedId;
                  const isReasoning = reasoningCardIds.has(card.id);
                  const meta = storyGlyphMeta(card, isReasoning);
                  const presentation = buildReadableToolPresentation(card);
                  const lineLabel = card.label || presentation.title || "Step";
                  const lineSub = isReasoning ? "" : storySubText(presentation, card);
                  const copyText =
                    presentation.body || card.detailFull || card.summary || lineLabel;
                  return (
                    <div key={card.id} className="term-story-item">
                      <button
                        type="button"
                        className={`term-story-line is-${meta.tone}${expanded ? " is-active" : ""}`}
                        aria-expanded={expanded}
                        aria-label={lineLabel}
                        onClick={() => onActivate(expanded ? null : card.id)}
                      >
                        <span className={`term-story-glyph tg-${meta.tone}`} aria-hidden="true">
                          {meta.glyph}
                        </span>
                        <span className="term-story-main">
                          <span className="term-story-label">{lineLabel}</span>
                          {lineSub ? (
                            <span className="term-story-sub"> · {truncateStorySub(lineSub)}</span>
                          ) : null}
                          <span className="term-story-caret" aria-hidden="true">
                            {expanded ? "▾" : "▸"}
                          </span>
                        </span>
                      </button>
                      <button
                        type="button"
                        className="term-story-copy"
                        aria-label="Copy step output"
                        onClick={(event) => {
                          event.stopPropagation();
                          copyStepText(copyText);
                        }}
                      >
                        <ContentCopyRoundedIcon sx={{ fontSize: 13 }} />
                      </button>
                      {expanded ? (
                        <div className="term-story-detail">
                          {activeCardIsReasoning && activeCard ? (
                            <WorkingView
                              phaseLabel={activeCard.label || "Thinking"}
                              detail={activeCard.summary || activeCard.detail || ""}
                              startedAt={activeCard.time || startedAt}
                              reasoningPreview={reasoningCardContent(activeCard)}
                              reasoningPhase={reasoningCardPhase(activeCard)}
                              persisted
                            />
                          ) : activeCardHasDelegation && activeCard ? (
                            <DelegationRunView
                              cards={
                                delegationCardsForActiveRun.length > 0
                                  ? delegationCardsForActiveRun
                                  : [activeCard]
                              }
                              activeRunId={activeDelegationRunId}
                            />
                          ) : activeCard ? (
                            <SurfaceRenderer
                              card={activeCard}
                              live={activeSurfaceLive}
                              snippetPath={activeFilePath || snippetPath}
                              snippetContent={activeFileContent}
                              workspaceFiles={workspaceFiles}
                              deployFilePath={null}
                              deployFileContent=""
                              deployFileCard={null}
                              deployFileLive={false}
                              onOpenDeployFile={(path) => {
                                setUserPickedDeployFile(
                                  !filePathsMatch(path, liveWritePath || ""),
                                );
                                setDeployFilePath(path);
                                setTab("files");
                                onActivate(null);
                              }}
                            />
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  );
                })}
                {/* No trailing "now doing" panel: the story lines + the header
                    status already convey the live run, and a pinned working
                    block reads as a stray duplicate of the Thinking step. */}
              </div>
            )}
          </Box>
        </Box>
      ) : tab === "files" && hasFileTab ? (
        <Box
          className="computer-pane-body computer-pane-body-files"
          sx={{
            flex: 1,
            minHeight: 0,
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          }}
        >
          {hasWorkspaceFiles ? (
            <Box
              className={`computer-pane-files-section${filesListCollapsed ? " is-collapsed" : ""}`}
            >
              <Box className="computer-pane-files-head">
                <Box className="computer-pane-files-head-main">
                  <Typography variant="caption" className="computer-pane-files-title">
                    Files
                  </Typography>
                  <Typography variant="caption" className="computer-pane-files-count">
                    {workspaceFiles.length}
                  </Typography>
                </Box>
                <Tooltip title={filesListCollapsed ? "Show files" : "Collapse files"}>
                  <IconButton
                    size="small"
                    className="computer-pane-files-collapse"
                    aria-label={
                      filesListCollapsed ? "Show files" : "Collapse files"
                    }
                    aria-expanded={!filesListCollapsed}
                    aria-controls={filesListId}
                    onClick={() => setFilesListCollapsed((prev) => !prev)}
                  >
                    {filesListCollapsed ? (
                      <KeyboardArrowDownRoundedIcon fontSize="small" />
                    ) : (
                      <KeyboardArrowUpRoundedIcon fontSize="small" />
                    )}
                  </IconButton>
                </Tooltip>
              </Box>
              {!filesListCollapsed ? (
                <div id={filesListId} className="computer-pane-files-list">
                  {workspaceFileTree.map((node) => renderWorkspaceFileNode(node, 0))}
                </div>
              ) : null}
            </Box>
          ) : null}
          <Box
            className="computer-pane-stage"
            sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
          >
            {focusedFileCard ? (
              <FileView
                card={focusedFileCard}
                snippetPath={focusedFilePath}
                snippetContent={focusedFileContent}
                live={focusedFileIsLiveWrite && liveWriteActive}
              />
            ) : fallbackFileCard ? (
              <FileView
                card={fallbackFileCard}
                snippetPath={fallbackFilePath}
                snippetContent={fallbackFileContent}
                live={fallbackFileIsLiveWrite && liveWriteActive}
              />
            ) : snippetCard ? (
              <FileView
                card={snippetCard}
                snippetPath={snippetPath}
                snippetContent={snippetContent}
              />
            ) : (
              <StatusView
                title="No file selected"
                detail="Generated files and live writes will appear here."
              />
            )}
          </Box>
        </Box>
      ) : tab === "activity" ? (
        <Box
          className="computer-pane-body computer-pane-body-activity"
          sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
        >
          {activityNode || (
            <ActivityList
              cards={activityCards}
              activeStepId={activeStepId}
            />
          )}
        </Box>
      ) : null}
    </Box>
  );
}

function areComputerPanePropsEqual(
  prev: ComputerPaneProps,
  next: ComputerPaneProps,
) {
  return (
    prev.liveCards === next.liveCards &&
    prev.allCards === next.allCards &&
    prev.activeStepId === next.activeStepId &&
    prev.onActivate === next.onActivate &&
    prev.onClose === next.onClose &&
    prev.activityNode === next.activityNode &&
    prev.nowDoingLabel === next.nowDoingLabel &&
    prev.snippetPath === next.snippetPath &&
    prev.snippetContent === next.snippetContent &&
    prev.isStreaming === next.isStreaming &&
    prev.startedAt === next.startedAt &&
    prev.tokenPreview === next.tokenPreview &&
    prev.runMetrics === next.runMetrics &&
    prev.reasoningPreview === next.reasoningPreview &&
    prev.reasoningPhase === next.reasoningPhase &&
    prev.taskProgress === next.taskProgress &&
    prev.showSnippet === next.showSnippet &&
    prev.workspaceFiles === next.workspaceFiles &&
    prev.liveWritePath === next.liveWritePath &&
    prev.liveWriteContent === next.liveWriteContent &&
    prev.liveWriteActive === next.liveWriteActive
  );
}

export const ComputerPane = memo(ComputerPaneInner, areComputerPanePropsEqual);
ComputerPane.displayName = "ComputerPane";

export default ComputerPane;
