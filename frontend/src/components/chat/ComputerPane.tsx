// Right-side "Computer" pane: one focused live artifact surface with compact
// activity history. This is intentionally closer to a runtime console
// than a second copy of the chat timeline.

import {
  memo,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import Box from "@mui/material/Box";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Collapse from "@mui/material/Collapse";
import CloseIcon from "@mui/icons-material/Close";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import FiberManualRecordRoundedIcon from "@mui/icons-material/FiberManualRecordRounded";
import KeyboardArrowDownRoundedIcon from "@mui/icons-material/KeyboardArrowDownRounded";
import KeyboardArrowUpRoundedIcon from "@mui/icons-material/KeyboardArrowUpRounded";
import TerminalRoundedIcon from "@mui/icons-material/TerminalRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";

import type { ChatStepCard, ComputerPaneFile, ComputerPaneTab, SurfaceStatus } from "./types";
import { extractFilePath, pickComputerView, prepareChipCards } from "./dispatch";
import {
  AGENTARK_RENDERERS,
  rendererIdForCard,
  surfaceDisplayTitle,
  surfaceFromCard,
  surfaceStatus,
} from "./surface";
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

function ActivityList({
  cards,
  activeStepId,
}: {
  cards: ChatStepCard[];
  activeStepId: string | null;
}) {
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());

  const toggleExpanded = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

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
        const time = card.time || "";
        const detail = safePaneText(card.summary || card.detail || "");
        const expanded = expandedIds.has(card.id);
        const detailsId = `computer-pane-activity-details-${card.id}`.replace(
          /[^a-zA-Z0-9_-]+/g,
          "-",
        );
        return (
          <li
            key={`activity-${card.id}`}
            className={`computer-pane-activity-row tone-${card.tone}${isActive ? " is-active" : ""}${expanded ? " is-expanded" : ""}`}
          >
            <button
              type="button"
              className="computer-pane-activity-button"
              aria-expanded={expanded}
              aria-controls={detailsId}
              onClick={() => toggleExpanded(card.id)}
            >
              <span className="computer-pane-activity-kind">
                {card.kind || "Update"}
              </span>
              <span className="computer-pane-activity-label">{card.label}</span>
              {detail ? (
                <span className="computer-pane-activity-detail">
                  {detail}
                </span>
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
      })}
    </ol>
  );
}

type ActivityDisplayField = {
  label: string;
  value: string;
};

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
  const overview =
    str(data.content_snapshot, "") ||
    str(data.content, "") ||
    str(record.detail, "") ||
    card.rawDetailFull ||
    card.detailFull ||
    card.summary ||
    card.detail ||
    "Activity update.";
  const statusFields: ActivityDisplayField[] = [
    { label: "Status", value: card.kind || "Update" },
    { label: "Title", value: card.label || card.rawTitle || "Activity update" },
    { label: "Step Type", value: card.stepType },
    { label: "Time", value: card.time },
  ].filter((field) => field.value.trim());
  const traceFields = collectActivityFields(record, { limit: 20 });
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
  const traceJson = (card.traceJson || "").trim();
  const copyText = [
    `Overview:\n${overview}`,
    displayFields
      .map((field) => `${field.label}:\n${field.value}`)
      .join("\n\n"),
    traceJson ? `Raw Trace JSON:\n${traceJson}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
  return { overview, displayFields, traceJson, copyText };
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
            <summary>Raw trace JSON</summary>
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
  const data = asPaneRecord(record?.data);
  return str(record?.phase, str(data.phase, "")).trim();
}

function reasoningCardContent(card: ChatStepCard): string {
  const record = structuredCardRecord(card);
  const data = asPaneRecord(record?.data);
  return (
    str(record?.content_snapshot, "") ||
    str(data.content_snapshot, "") ||
    str(record?.content, "") ||
    str(data.content, "") ||
    str(record?.content_delta, "") ||
    str(data.content_delta, "") ||
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    card.summary ||
    ""
  );
}

function isReasoningOnlyCard(card: ChatStepCard): boolean {
  const record = structuredCardRecord(card);
  const kind = str(record?.kind, "").trim().toLowerCase();
  const phase = str(record?.phase, "").trim();
  const stepType = (card.stepType || "").trim().toLowerCase();
  if (kind === "reasoning_delta" || stepType === "reasoning_delta") {
    return true;
  }
  return Boolean(
    phase &&
      record &&
      !str(record.tool_name, "") &&
      !str(record.name, "") &&
      !str(record.file, "") &&
      !str(record.path, "") &&
      (str(record.content, "") ||
        str(record.content_delta, "") ||
        str(record.content_snapshot, "")),
  );
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
      return file.content || "";
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

function workspaceFileMeta(file: ComputerPaneFile, live: boolean): string {
  if (live) return "writing";
  const lineCount = file.content ? file.content.split(/\r?\n/).length : 0;
  const byteCount = new Blob([file.content || ""]).size;
  if (lineCount > 0) {
    return `${lineCount} line${lineCount === 1 ? "" : "s"} / ${formatBytes(byteCount)}`;
  }
  return "queued";
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
  const filesListId = useId();
  const lastLiveWritePathRef = useRef<string | null>(null);
  const hasWorkspaceFiles = workspaceFiles.length > 0;
  const snippetFileAvailable = Boolean(showSnippet && (snippetPath || snippetContent));
  const hasFileTab =
    hasWorkspaceFiles || Boolean(liveWritePath || deployFilePath) || snippetFileAvailable;
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
    if (!liveWriteActive && deployFilePath) return;
    setDeployFilePath(autoFocusFilePath);
  }, [
    autoFocusFilePath,
    deployFilePath,
    liveWriteActive,
    userPickedDeployFile,
  ]);
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
  const primaryActivityCards = useMemo(
    () => activityCards.filter((card) => !isReasoningOnlyCard(card)),
    [activityCards],
  );
  const navPool = useMemo(
    () => {
      const workspaceSurfaceCards = prepareChipCards(cardsForRun).filter(
        (card) => {
          const rendererId = rendererIdForCard(card);
          return (
            rendererId !== AGENTARK_RENDERERS.WORKING &&
            rendererId !== AGENTARK_RENDERERS.FILE
          );
        },
      );
      const reasoningCards = activityCards.filter(isReasoningOnlyCard);
      return [...reasoningCards, ...workspaceSurfaceCards].sort(
        (left, right) => left.index - right.index,
      );
    },
    [activityCards, cardsForRun],
  );
  const headerFilePath =
    followedLiveWritePath ||
    deployFilePath ||
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
          name: fileNameFromPath(file.path),
          meta: workspaceFileMeta(file, live),
        };
      }),
    [headerFilePath, liveWriteActive, liveWritePath, workspaceFiles],
  );

  const activeCard = useMemo(
    () => pickActiveCard(navPool, activeStepId),
    [navPool, activeStepId],
  );
  const latestActivityCard = useMemo(
    () => pickActiveCard(primaryActivityCards, activeStepId),
    [primaryActivityCards, activeStepId],
  );

  const activeIndex = useMemo(
    () => (activeCard ? navPool.findIndex((c) => c.id === activeCard.id) : -1),
    [navPool, activeCard],
  );
  const canPrev = activeIndex > 0;
  const canNext = activeIndex >= 0 && activeIndex < navPool.length - 1;
  const view = activeCard ? pickComputerView(activeCard) : AGENTARK_RENDERERS.GENERIC;
  const activeCardIsReasoning = activeCard ? isReasoningOnlyCard(activeCard) : false;
  const activeSurfaceStatus = activeCard ? surfaceStatus(activeCard, Boolean(isStreaming)) : null;
  const fileHeaderText =
    liveWriteActive && headerFilePath
      ? `Writing ${headerFilePath}`
      : headerFilePath
        ? `Files: ${headerFilePath}`
        : "Files";
  const consoleHeaderText = activeCard
    ? surfaceDisplayTitle(activeCard)
    : nowDoingLabel || "Working";
  const headerText = safePaneText(
    tab === "files" && hasFileTab ? fileHeaderText : consoleHeaderText,
    "Working",
  );
  const completedCount = navPool.filter((card) =>
    /done|complete|success/i.test(card.kind || ""),
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
    !!deployFilePath &&
    !!liveWritePath &&
    filePathsMatch(deployFilePath, liveWritePath);
  const deployFileContent = useMemo(() => {
    if (!deployFilePath) return "";
    // While the file is streaming, prefer the live buffer over any captured
    // workspace snapshot so the user sees the just-written line, not stale
    // content from a previous run.
    if (deployFileIsLiveWrite && liveWriteContent) return liveWriteContent;
    return (
      findWorkspaceFileContent(workspaceFiles, deployFilePath) ||
      findFileContentForPath(cardsForRun, deployFilePath)
    );
  }, [
    cardsForRun,
    deployFilePath,
    deployFileIsLiveWrite,
    liveWriteContent,
    workspaceFiles,
  ]);
  const focusedFilePath =
    followedLiveWritePath || deployFilePath || (liveWriteActive ? liveWritePath || "" : "");
  const focusedFileIsLiveWrite =
    !!focusedFilePath &&
    !!liveWritePath &&
    filePathsMatch(focusedFilePath, liveWritePath);
  const focusedFileContent = useMemo(() => {
    if (!focusedFilePath) return "";
    if (deployFilePath && filePathsMatch(focusedFilePath, deployFilePath)) {
      return deployFileContent;
    }
    if (focusedFileIsLiveWrite && liveWriteContent) return liveWriteContent;
    return (
      findWorkspaceFileContent(workspaceFiles, focusedFilePath) ||
      findFileContentForPath(cardsForRun, focusedFilePath)
    );
  }, [
    cardsForRun,
    deployFileContent,
    deployFilePath,
    focusedFileIsLiveWrite,
    focusedFilePath,
    liveWriteContent,
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
    if (activeFileIsLiveWrite && liveWriteContent) return liveWriteContent;
    return (
      findWorkspaceFileContent(workspaceFiles, activeFilePath) ||
      findFileContentForPath(cardsForRun, activeFilePath)
    );
  }, [
    activeFilePath,
    activeFileIsLiveWrite,
    cardsForRun,
    liveWriteContent,
    workspaceFiles,
  ]);
  const activeSurfaceLive =
    view === AGENTARK_RENDERERS.FILE
      ? activeFileIsLiveWrite && liveWriteActive
      : Boolean(isStreaming) && activeIndex === navPool.length - 1;
  const fallbackFilePath =
    !activeCard && (deployFilePath || liveWritePath || workspaceFiles[0]?.path)
      ? deployFilePath || liveWritePath || workspaceFiles[0]?.path || ""
      : "";
  const fallbackFileIsLiveWrite =
    !!fallbackFilePath &&
    !!liveWritePath &&
    filePathsMatch(fallbackFilePath, liveWritePath);
  const fallbackFileContent = useMemo(() => {
    if (!fallbackFilePath) return "";
    if (fallbackFileIsLiveWrite && liveWriteContent) {
      return liveWriteContent;
    }
    return (
      findWorkspaceFileContent(workspaceFiles, fallbackFilePath) ||
      findFileContentForPath(cardsForRun, fallbackFilePath)
    );
  }, [
    cardsForRun,
    fallbackFileIsLiveWrite,
    fallbackFilePath,
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
  const visibleTabs: ComputerPaneTab[] = hasFileTab
    ? ["computer", "files", "activity"]
    : ["computer", "activity"];
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
          <Stack
            direction="row"
            spacing={0.8}
            className="computer-pane-progress-row"
            sx={{ alignItems: "center" }}
          >
            <FiberManualRecordRoundedIcon
              fontSize="inherit"
              className={`computer-pane-status-dot tone-${statusTone(liveWriteActive ? "running" : activeSurfaceStatus)}`}
            />
            <Typography variant="caption" className="computer-pane-progress">
              {progressText}
            </Typography>
            <span className="computer-pane-step-sep">|</span>
            <Typography variant="caption" className="computer-pane-current-step">
              {headerText}
            </Typography>
            {headerModeLabel ? (
              <>
                <span className="computer-pane-step-sep">|</span>
                <Typography variant="caption" className="computer-pane-current-step">
                  {headerModeLabel}
                </Typography>
              </>
            ) : null}
          </Stack>
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
          <Stack
            direction="row"
            spacing={0.5}
            className="computer-pane-nav"
            sx={{ alignItems: "center" }}
          >
            <IconButton
              size="small"
              disabled={!canPrev}
              aria-label="Previous artifact"
              onClick={() => {
                if (!canPrev) return;
                setDeployFilePath(null);
                onActivate(navPool[activeIndex - 1].id);
              }}
            >
              <ChevronLeftRoundedIcon fontSize="small" />
            </IconButton>
            <Typography variant="caption" className="computer-pane-nav-pos">
              {activeIndex >= 0 ? `${activeIndex + 1} / ${navPool.length}` : "working"}
            </Typography>
            <IconButton
              size="small"
              disabled={!canNext}
              aria-label="Next artifact"
              onClick={() => {
                if (!canNext) return;
                setDeployFilePath(null);
                onActivate(navPool[activeIndex + 1].id);
              }}
            >
              <ChevronRightRoundedIcon fontSize="small" />
            </IconButton>
            <Box sx={{ flex: 1 }} />
            {isStreaming ? (
              <Stack
                direction="row"
                spacing={0.4}
                sx={{ alignItems: "center" }}
                className="computer-pane-live"
              >
                <FiberManualRecordRoundedIcon
                  fontSize="inherit"
                  className="computer-pane-live-dot"
                />
                <Typography variant="caption" className="computer-pane-live-label">
                  live
                </Typography>
              </Stack>
            ) : null}
            {activeStepId ? (
              <button
                type="button"
                className="computer-pane-follow-button"
                onClick={() => {
                  setDeployFilePath(null);
                  onActivate(null);
                }}
                title="Resume following the latest step"
              >
                Follow latest
              </button>
            ) : null}
          </Stack>
          <Box
            className="computer-pane-stage"
            sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
          >
            {!activeCard ? (
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
            ) : activeCardIsReasoning ? (
              <WorkingView
                phaseLabel={activeCard.label || "Thinking"}
                detail={activeCard.summary || activeCard.detail || ""}
                startedAt={activeCard.time || startedAt}
                reasoningPreview={reasoningCardContent(activeCard)}
                reasoningPhase={reasoningCardPhase(activeCard)}
                persisted
              />
            ) : (
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
                  {workspaceFileRows.map(({ file, live, selected, name, meta }) => {
                    return (
                      <button
                        key={file.path}
                        type="button"
                        className={`computer-pane-file-pill${selected ? " is-selected" : ""}${live ? " is-live" : ""}`}
                        onClick={() => {
                          setUserPickedDeployFile(
                            !filePathsMatch(file.path, liveWritePath || ""),
                          );
                          setDeployFilePath(file.path);
                          onActivate(null);
                        }}
                        title={file.path}
                      >
                        <span
                          className="computer-pane-file-dot"
                          aria-hidden="true"
                        />
                        <span className="computer-pane-file-name">{name}</span>
                        <span className="computer-pane-file-path">
                          {file.path}
                        </span>
                        <span className="computer-pane-file-meta">{meta}</span>
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </Box>
          ) : null}
          <Box
            className="computer-pane-stage"
            sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
          >
            {snippetCard ? (
              <FileView
                card={snippetCard}
                snippetPath={snippetPath}
                snippetContent={snippetContent}
              />
            ) : focusedFileCard ? (
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
