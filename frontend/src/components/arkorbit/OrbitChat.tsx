import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Box, Divider, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import AddCommentRoundedIcon from "@mui/icons-material/AddCommentRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import CodeRoundedIcon from "@mui/icons-material/CodeRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import FolderOpenRoundedIcon from "@mui/icons-material/FolderOpenRounded";
import HistoryRoundedIcon from "@mui/icons-material/HistoryRounded";
import PersonRoundedIcon from "@mui/icons-material/PersonRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import SendRoundedIcon from "@mui/icons-material/SendRounded";
import StopCircleRoundedIcon from "@mui/icons-material/StopCircleRounded";
import AgentLogo from "../../assets/logo.svg";
import { arkorbitApi } from "./api";
import type {
  OrbitChatFileChip,
  OrbitChatHistoryMessage,
  OrbitChatMessageStatus,
  OrbitChatTranscript,
  OrbitChatUsage,
  OrbitFileEntry,
  OrbitId,
} from "./types";

type Role = "user" | "assistant";
type OrbitFileOperation = "wrote" | "edited";

type ChatMessage = {
  id: string;
  role: Role;
  text: string;
  files?: OrbitChatFileChip[];
  usage?: OrbitChatUsage;
  activity?: string;
  active?: boolean;
  status?: OrbitChatMessageStatus;
};

type OrbitChatPanelState = "idle" | "loading" | "running" | "failed" | "stopped" | "archived";
type OrbitChatTab = "chat" | "files";

type Props = {
  orbitId: OrbitId;
  runtimeNotices?: string[];
  onRuntimeNoticesSubmitted?: (notices: string[]) => void;
  onFileWritten?: (path: string) => void;
  onClose?: () => void;
};

type StreamHandlers = {
  onToken: (content: string) => void;
  onFileWritten: (path: string, operation: OrbitFileOperation, bytes?: number) => void;
  onRead: (path: string) => void;
  onStatus: (message: string) => void;
  onUsage: (usage: OrbitChatUsage) => void;
  onError: (message: string) => void;
  onDone: () => void;
};

type OrbitChatComposerProps = {
  readOnly: boolean;
  streaming: boolean;
  resetSignal: number;
  onSend: (message: string) => void;
  onStop: () => void;
};

const HISTORY_PAGE_SIZE = 5;

function newId(prefix = "m"): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}_${Date.now().toString(36)}`;
}

function formatBytes(value: number | undefined): string {
  if (!value || !Number.isFinite(value) || value <= 0) return "";
  if (value < 1024) return `${Math.round(value)} ${Math.round(value) === 1 ? "byte" : "bytes"}`;
  const kb = value / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

function fileUpdateSentence(
  operation: OrbitFileOperation,
  path: string,
  bytes?: number,
): string {
  const action = operation === "edited" ? "Edited" : "Wrote";
  const size = formatBytes(bytes);
  return size ? `${action} ${path} (${size}).` : `${action} ${path}.`;
}

function fileActivityLabel(
  operation: OrbitFileOperation,
  path: string,
  bytes?: number,
): string {
  const action = operation === "edited" ? "Saved" : "Wrote";
  const size = formatBytes(bytes);
  return size ? `${action} ${path} (${size}).` : `${action} ${path}.`;
}

function normalizeOperation(value: string): OrbitFileOperation {
  return value.toLowerCase() === "edited" ? "edited" : "wrote";
}

function positiveMetric(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function normalizeUsage(value: Partial<OrbitChatUsage>): OrbitChatUsage | undefined {
  const usage: OrbitChatUsage = {};
  const model = typeof value.model === "string" ? value.model.trim() : "";
  const inputTokens = positiveMetric(value.input_tokens);
  const outputTokens = positiveMetric(value.output_tokens);
  const cachedPromptTokens = positiveMetric(value.cached_prompt_tokens);
  const cacheCreationPromptTokens = positiveMetric(
    value.cache_creation_prompt_tokens,
  );
  const totalTokens =
    positiveMetric(value.total_tokens) ??
    (inputTokens || outputTokens
      ? (inputTokens ?? 0) + (outputTokens ?? 0)
      : undefined);
  const costUsd = positiveMetric(value.cost_usd);
  const durationMs = positiveMetric(value.duration_ms);
  const timeToFirstTokenMs = positiveMetric(value.time_to_first_token_ms);
  if (model) usage.model = model;
  if (inputTokens) usage.input_tokens = inputTokens;
  if (outputTokens) usage.output_tokens = outputTokens;
  if (totalTokens) usage.total_tokens = totalTokens;
  if (cachedPromptTokens) usage.cached_prompt_tokens = cachedPromptTokens;
  if (cacheCreationPromptTokens) {
    usage.cache_creation_prompt_tokens = cacheCreationPromptTokens;
  }
  if (costUsd) usage.cost_usd = costUsd;
  if (typeof value.estimated === "boolean") usage.estimated = value.estimated;
  if (durationMs) usage.duration_ms = durationMs;
  if (timeToFirstTokenMs) usage.time_to_first_token_ms = timeToFirstTokenMs;
  return Object.keys(usage).length > 0 ? usage : undefined;
}

function formatDurationMs(value: number | undefined): string {
  if (!value || !Number.isFinite(value) || value <= 0) return "0ms";
  if (value < 1000) return `${Math.round(value)}ms`;
  if (value < 60_000) return `${(value / 1000).toFixed(value < 10_000 ? 1 : 0)}s`;
  const minutes = Math.floor(value / 60_000);
  const seconds = Math.round((value % 60_000) / 1000);
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
}

function orbitUsageMetricItems(usage?: OrbitChatUsage): Array<{
  label: string;
  value: string;
}> {
  const normalized = usage ? normalizeUsage(usage) : undefined;
  if (!normalized) return [];
  const inputTokens = normalized.input_tokens ?? 0;
  const outputTokens = normalized.output_tokens ?? 0;
  const cachedPromptTokens = normalized.cached_prompt_tokens ?? 0;
  const cacheCreationPromptTokens = normalized.cache_creation_prompt_tokens ?? 0;
  const totalTokens = normalized.total_tokens ?? inputTokens + outputTokens;
  if (totalTokens <= 0) return [];
  const items = [
    { label: "Total tokens", value: Math.round(totalTokens).toLocaleString() },
    { label: "Input tokens", value: Math.round(inputTokens).toLocaleString() },
    { label: "Output tokens", value: Math.round(outputTokens).toLocaleString() },
  ];
  if (cachedPromptTokens > 0) {
    items.push({
      label: "Cached prompt",
      value: Math.round(cachedPromptTokens).toLocaleString(),
    });
  }
  if (cacheCreationPromptTokens > 0) {
    items.push({
      label: "Cache write",
      value: Math.round(cacheCreationPromptTokens).toLocaleString(),
    });
  }
  return items;
}

function orbitUsageTitle(usage?: OrbitChatUsage): string | undefined {
  const normalized = usage ? normalizeUsage(usage) : undefined;
  if (!normalized) return undefined;
  const details: string[] = [];
  if (normalized.model) details.push(`Model: ${normalized.model}`);
  if (normalized.cost_usd) details.push(`Cost: $${normalized.cost_usd.toFixed(6)}`);
  if (normalized.estimated) details.push("Token counts are estimated");
  if (normalized.duration_ms) details.push(`Duration: ${formatDurationMs(normalized.duration_ms)}`);
  return details.length > 0 ? details.join(" | ") : undefined;
}

function orbitStatusLabel(status?: OrbitChatMessageStatus): string {
  switch (status) {
    case "running":
      return "Running";
    case "failed":
      return "Failed";
    case "stopped":
      return "Stopped";
    case "completed":
      return "";
    default:
      return "";
  }
}

function orbitStatusActivity(status?: OrbitChatMessageStatus): string {
  switch (status) {
    case "running":
      return "Waiting for live Orbit progress.";
    case "failed":
      return "Failed before a response was completed.";
    case "stopped":
      return "Stopped in this browser.";
    default:
      return "";
  }
}

function OrbitRunningDots({ className = "" }: { className?: string }) {
  return (
    <span
      className={`orbit-chat-running-dots${className ? ` ${className}` : ""}`}
      aria-label="Running"
      role="status"
    >
      <i />
      <i />
      <i />
    </span>
  );
}

function orbitPanelStateLabel(state: OrbitChatPanelState): string {
  switch (state) {
    case "loading":
      return "Loading conversation";
    case "running":
      return "Running";
    case "failed":
      return "Failed";
    case "stopped":
      return "Stopped";
    case "archived":
      return "Archived conversation";
    default:
      return "Current conversation";
  }
}

function fileChip(
  path: string,
  operation: OrbitFileOperation,
  bytes?: number,
): OrbitChatFileChip {
  return { id: newId("f"), path, operation, bytes };
}

function addFileChip(
  files: OrbitChatFileChip[] | undefined,
  path: string,
  operation: OrbitFileOperation,
  bytes?: number,
): OrbitChatFileChip[] {
  const current = files ?? [];
  if (current.some((file) => file.path === path && file.operation === operation)) {
    return current.map((file) =>
      file.path === path && file.operation === operation
        ? { ...file, bytes: bytes ?? file.bytes }
        : file,
    );
  }
  return [...current, fileChip(path, operation, bytes)];
}

function extractFileUpdate(line: string):
  | { operation: OrbitFileOperation; path: string }
  | null {
  const legacy = line.match(/^\s*\[(wrote|edited)\s+`([^`]+)`\]\s*$/i);
  if (legacy) {
    return {
      operation: normalizeOperation(legacy[1]),
      path: legacy[2].trim(),
    };
  }
  const readable = line.match(/^\s*I\s+(wrote|edited)\s+([A-Za-z0-9_./-]+)\.\s*$/i);
  if (readable) {
    return {
      operation: normalizeOperation(readable[1]),
      path: readable[2].trim(),
    };
  }
  return null;
}

function normalizeAssistantContent(content: string): {
  text: string;
  files: OrbitChatFileChip[];
} {
  const files: OrbitChatFileChip[] = [];
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const text = lines
    .map((line) => {
      const update = extractFileUpdate(line);
      if (!update?.path) return line;
      files.push(fileChip(update.path, update.operation));
      return fileUpdateSentence(update.operation, update.path);
    })
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return { text, files };
}

function historyToChatMessage(message: OrbitChatHistoryMessage): ChatMessage | null {
  if (message.role !== "user" && message.role !== "assistant") return null;
  const normalized =
    message.role === "assistant"
      ? normalizeAssistantContent(message.content)
      : { text: message.content, files: [] };
  const legacyEmptyAssistant =
    message.role === "assistant" &&
    !normalized.text.trim() &&
    normalized.files.length === 0 &&
    !message.status &&
    !message.activity;
  if (
    message.role === "assistant" &&
    !normalized.text.trim() &&
    normalized.files.length === 0 &&
    (!message.status || message.status === "completed") &&
    !message.activity &&
    !legacyEmptyAssistant
  ) {
    return null;
  }
  return {
    id: message.id || newId("h"),
    role: message.role,
    text: normalized.text,
    files: normalized.files,
    status: legacyEmptyAssistant ? "failed" : message.status,
    activity: legacyEmptyAssistant
      ? "No response was recorded for this turn."
      : message.activity,
    active: message.status === "running",
    usage: message.role === "assistant" ? normalizeUsage(message) : undefined,
  };
}

function sameChatMessages(left: ChatMessage[], right: ChatMessage[]): boolean {
  if (left.length !== right.length) return false;
  return left.every((message, index) => {
    const other = right[index];
    return (
      other &&
      message.id === other.id &&
      message.role === other.role &&
      message.text === other.text &&
      message.status === other.status &&
      message.activity === other.activity &&
      (message.files?.length ?? 0) === (other.files?.length ?? 0) &&
      JSON.stringify(normalizeUsage(message.usage ?? {})) ===
        JSON.stringify(normalizeUsage(other.usage ?? {}))
    );
  });
}

function extractFetchError(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return "";
  try {
    const payload = JSON.parse(trimmed) as Record<string, unknown>;
    return (
      (typeof payload.error === "string" && payload.error) ||
      (typeof payload.message === "string" && payload.message) ||
      (typeof payload.detail === "string" && payload.detail) ||
      trimmed
    );
  } catch {
    return trimmed;
  }
}

function orbitFileName(path: string): string {
  const normalized = path.replace(/\\/g, "/").trim();
  return normalized.split("/").filter(Boolean).pop() || normalized || "file";
}

async function readOrbitChatFileText(
  orbitId: OrbitId,
  path: string,
): Promise<string> {
  const response = await fetch(arkorbitApi.orbitFileUrl(orbitId, path), {
    credentials: "include",
    cache: "no-store",
  });
  const text = await response.text();
  if (!response.ok) throw new Error(text || `File request failed (${response.status}).`);
  return text;
}

function OrbitChatFilesView({
  files,
  selectedPath,
  content,
  loadingFiles,
  loadingContent,
  error,
  onSelect,
  onRefresh,
}: {
  files: OrbitFileEntry[];
  selectedPath: string | null;
  content: string;
  loadingFiles: boolean;
  loadingContent: boolean;
  error: string | null;
  onSelect: (path: string) => void;
  onRefresh: () => void;
}) {
  const selectedFile =
    files.find((file) => file.path === selectedPath) ?? files[0] ?? null;
  const selectedMeta = selectedFile
    ? formatBytes(selectedFile.bytes) || "0 bytes"
    : "";

  return (
    <Box className="orbit-chat-files-view">
      <Box className="orbit-chat-files-toolbar">
        <Stack direction="row" spacing={0.6} sx={{ alignItems: "center", minWidth: 0 }}>
          <FolderOpenRoundedIcon fontSize="small" />
          <Typography variant="caption" className="orbit-chat-files-title">
            Files
          </Typography>
          <span className="orbit-chat-files-count">{files.length}</span>
        </Stack>
        <Tooltip title="Refresh files">
          <span>
            <IconButton
              size="small"
              className="orbit-chat-tool"
              onClick={onRefresh}
              disabled={loadingFiles}
              aria-label="Refresh Orbit files"
            >
              <RefreshRoundedIcon fontSize="small" />
            </IconButton>
          </span>
        </Tooltip>
      </Box>
      {error ? <Box className="orbit-chat-files-error">{error}</Box> : null}
      {loadingFiles && files.length === 0 ? (
        <Box className="orbit-chat-files-empty">Loading files...</Box>
      ) : files.length === 0 ? (
        <Box className="orbit-chat-files-empty">No Orbit files yet.</Box>
      ) : (
        <Box className="orbit-chat-files-layout">
          <Box className="orbit-chat-files-list" role="listbox" aria-label="Orbit files">
            {files.map((file) => {
              const active = file.path === selectedFile?.path;
              return (
                <button
                  key={file.path}
                  type="button"
                  className={`orbit-chat-file-row${active ? " is-active" : ""}`}
                  onClick={() => onSelect(file.path)}
                  aria-selected={active}
                  role="option"
                >
                  <span className="orbit-chat-file-name">{orbitFileName(file.path)}</span>
                  <span className="orbit-chat-file-path">{file.path}</span>
                  <span className="orbit-chat-file-size">
                    {formatBytes(file.bytes) || "0 bytes"}
                  </span>
                </button>
              );
            })}
          </Box>
          <Box className="orbit-chat-file-preview">
            <Box className="orbit-chat-file-preview-head">
              <Stack direction="row" spacing={0.55} sx={{ alignItems: "center", minWidth: 0 }}>
                <CodeRoundedIcon fontSize="small" />
                <span className="orbit-chat-file-preview-path">
                  {selectedFile?.path ?? "No file selected"}
                </span>
              </Stack>
              {selectedMeta ? (
                <span className="orbit-chat-file-preview-meta">{selectedMeta}</span>
              ) : null}
            </Box>
            {loadingContent ? (
              <Box className="orbit-chat-files-empty">Loading file...</Box>
            ) : (
              <pre className="orbit-chat-file-code">
                <code>{content || "Select a file to preview it."}</code>
              </pre>
            )}
          </Box>
        </Box>
      )}
    </Box>
  );
}

function parseEventBlock(block: string): { event: string; data: string } | null {
  let event = "message";
  const data: string[] = [];
  for (const line of block.split(/\r?\n/)) {
    if (!line || line.startsWith(":")) continue;
    const idx = line.indexOf(":");
    const field = idx >= 0 ? line.slice(0, idx) : line;
    const value = idx >= 0 ? line.slice(idx + 1).replace(/^ /, "") : "";
    if (field === "event") event = value;
    if (field === "data") data.push(value);
  }
  if (data.length === 0) return null;
  return { event, data: data.join("\n") };
}

async function readOrbitChatStream(
  response: Response,
  handlers: StreamHandlers,
): Promise<void> {
  const reader = response.body?.getReader();
  if (!reader) throw new Error("Stream body unavailable");
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
      let splitAt = buffer.indexOf("\n\n");
      while (splitAt >= 0) {
        const block = buffer.slice(0, splitAt);
        buffer = buffer.slice(splitAt + 2);
        const parsed = parseEventBlock(block);
        if (parsed) dispatchStreamEvent(parsed, handlers);
        splitAt = buffer.indexOf("\n\n");
      }
    }
    buffer += decoder.decode();
    const parsed = parseEventBlock(buffer.trim());
    if (parsed) dispatchStreamEvent(parsed, handlers);
  } finally {
    try {
      await reader.cancel();
    } catch {
      // Reader may already be closed.
    }
  }
}

function dispatchStreamEvent(
  event: { event: string; data: string },
  handlers: StreamHandlers,
) {
  let payload: Record<string, unknown> = {};
  try {
    payload = JSON.parse(event.data) as Record<string, unknown>;
  } catch {
    payload = {};
  }
  if (event.event === "token") {
    const content = typeof payload.content === "string" ? payload.content : "";
    if (content) handlers.onToken(content);
    return;
  }
  if (event.event === "file_written") {
    const path = typeof payload.path === "string" ? payload.path : "";
    const operation =
      typeof payload.operation === "string" ? normalizeOperation(payload.operation) : "wrote";
    const bytes =
      typeof payload.bytes === "number" && Number.isFinite(payload.bytes) && payload.bytes > 0
        ? payload.bytes
        : undefined;
    if (path) handlers.onFileWritten(path, operation, bytes);
    return;
  }
  if (event.event === "read") {
    const path = typeof payload.path === "string" ? payload.path : "";
    if (path) handlers.onRead(path);
    return;
  }
  if (event.event === "status") {
    const message =
      (typeof payload.message === "string" && payload.message) ||
      (typeof payload.status === "string" && payload.status) ||
      "";
    if (message) handlers.onStatus(message);
    return;
  }
  if (event.event === "usage") {
    const usage = normalizeUsage(payload as Partial<OrbitChatUsage>);
    if (usage) handlers.onUsage(usage);
    return;
  }
  if (event.event === "error") {
    const message =
      (typeof payload.message === "string" && payload.message) ||
      (typeof payload.error === "string" && payload.error) ||
      "Orbit chat failed.";
    handlers.onError(message);
    return;
  }
  if (event.event === "done") {
    handlers.onDone();
  }
}

const OrbitChatComposer = memo(function OrbitChatComposer({
  readOnly,
  streaming,
  resetSignal,
  onSend,
  onStop,
}: OrbitChatComposerProps) {
  const [draft, setDraft] = useState("");
  const trimmed = draft.trim();

  useEffect(() => {
    setDraft("");
  }, [resetSignal]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    let stored: string | null = null;
    try {
      stored = window.sessionStorage.getItem("arkorbit.composerPrefill");
    } catch {
      stored = null;
    }
    if (!stored) return;
    try {
      window.sessionStorage.removeItem("arkorbit.composerPrefill");
    } catch {
      // best-effort cleanup
    }
    setDraft(stored);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const submit = useCallback(
    (event?: FormEvent<HTMLFormElement>) => {
      event?.preventDefault();
      if (!trimmed || streaming || readOnly) return;
      onSend(trimmed);
      setDraft("");
    },
    [onSend, readOnly, streaming, trimmed],
  );

  return (
    <Box component="form" className="orbit-chat-composer" onSubmit={submit}>
      <textarea
        className="orbit-chat-input"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        disabled={readOnly}
        placeholder={readOnly ? "Archived chat is read-only." : "Build on this orbit..."}
        rows={2}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          }
        }}
      />
      <Stack direction="row" spacing={0.5} sx={{ alignItems: "center" }}>
        {streaming ? (
          <Tooltip title="Stop streaming">
            <IconButton size="small" onClick={onStop} className="orbit-chat-stop">
              <StopCircleRoundedIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        ) : null}
        <Tooltip title="Send">
          <span>
            <IconButton
              type="submit"
              color="primary"
              disabled={readOnly || streaming || trimmed.length === 0}
              className="orbit-chat-send-icon"
            >
              <SendRoundedIcon fontSize="small" />
            </IconButton>
          </span>
        </Tooltip>
      </Stack>
    </Box>
  );
});

export function OrbitChat({
  orbitId,
  runtimeNotices = [],
  onRuntimeNoticesSubmitted,
  onFileWritten,
  onClose,
}: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [transcripts, setTranscripts] = useState<OrbitChatTranscript[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyPage, setHistoryPage] = useState(0);
  const [activeTranscriptId, setActiveTranscriptId] = useState("current");
  const [streaming, setStreaming] = useState(false);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [composerResetSignal, setComposerResetSignal] = useState(0);
  const [activeTab, setActiveTab] = useState<OrbitChatTab>("chat");
  const [files, setFiles] = useState<OrbitFileEntry[]>([]);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState("");
  const [loadingFiles, setLoadingFiles] = useState(false);
  const [loadingFileContent, setLoadingFileContent] = useState(false);
  const [filesError, setFilesError] = useState<string | null>(null);
  const [filesReloadSignal, setFilesReloadSignal] = useState(0);
  const abortRef = useRef<AbortController | null>(null);
  const activeAssistantRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const readOnly = activeTranscriptId !== "current";
  const historyPageCount = Math.max(1, Math.ceil(transcripts.length / HISTORY_PAGE_SIZE));
  const normalizedHistoryPage = Math.min(historyPage, historyPageCount - 1);
  const visibleTranscripts = useMemo(
    () =>
      transcripts.slice(
        normalizedHistoryPage * HISTORY_PAGE_SIZE,
        normalizedHistoryPage * HISTORY_PAGE_SIZE + HISTORY_PAGE_SIZE,
      ),
    [normalizedHistoryPage, transcripts],
  );
  const visibleTurnState = useMemo<OrbitChatPanelState>(() => {
    if (readOnly) return "archived";
    if (loadingHistory) return "loading";
    if (streaming) return "running";
    const lastAssistant = [...messages]
      .reverse()
      .find((message) => message.role === "assistant" && message.status);
    if (lastAssistant?.status === "running") return "running";
    if (lastAssistant?.status === "failed") return "failed";
    if (lastAssistant?.status === "stopped") return "stopped";
    return "idle";
  }, [loadingHistory, messages, readOnly, streaming]);
  const visibleTurnStatus = orbitPanelStateLabel(visibleTurnState);

  useEffect(() => {
    if (historyPage !== normalizedHistoryPage) {
      setHistoryPage(normalizedHistoryPage);
    }
  }, [historyPage, normalizedHistoryPage]);

  const refreshTranscripts = useCallback(
    async (cancelled?: () => boolean) => {
      try {
        const next = await arkorbitApi.listTranscripts(orbitId);
        if (!cancelled?.()) setTranscripts(next);
      } catch {
        if (!cancelled?.()) setTranscripts([]);
      }
    },
    [orbitId],
  );

  const loadTranscript = useCallback(
    async (transcriptId: string, cancelled?: () => boolean) => {
      setLoadingHistory(true);
      try {
        const history =
          transcriptId === "current"
            ? await arkorbitApi.listMessages(orbitId)
            : await arkorbitApi.getTranscriptMessages(orbitId, transcriptId);
        if (cancelled?.()) return;
        setMessages(history.map(historyToChatMessage).filter(Boolean) as ChatMessage[]);
        setActiveTranscriptId(transcriptId);
      } catch (err) {
        if (cancelled?.()) return;
        const detail = err instanceof Error ? err.message : String(err);
        setMessages([
          {
            id: newId("history_error"),
            role: "assistant",
            text: `Could not load orbit chat history: ${detail}`,
            status: "failed",
          },
        ]);
        setActiveTranscriptId("current");
      } finally {
        if (!cancelled?.()) setLoadingHistory(false);
      }
    },
    [orbitId],
  );

  useEffect(() => {
    let cancelled = false;
    abortRef.current?.abort();
    abortRef.current = null;
    activeAssistantRef.current = null;
    setStreaming(false);
    setHistoryOpen(false);
    setHistoryPage(0);
    setActiveTranscriptId("current");
    setMessages([]);
    setActiveTab("chat");
    setFiles([]);
    setSelectedFilePath(null);
    setFileContent("");
    setFilesError(null);
    setComposerResetSignal((value) => value + 1);
    const isCancelled = () => cancelled;
    void loadTranscript("current", isCancelled);
    void refreshTranscripts(isCancelled);

    return () => {
      cancelled = true;
    };
  }, [orbitId, loadTranscript, refreshTranscripts]);

  useEffect(() => {
    const node = scrollRef.current;
    if (activeTab === "chat" && node) node.scrollTop = node.scrollHeight;
  }, [activeTab, messages]);

  useEffect(() => () => abortRef.current?.abort(), []);

  useEffect(() => {
    if (activeTranscriptId !== "current" || streaming) return undefined;
    let cancelled = false;
    let timer: number | null = null;
    const refreshCurrentMessages = async () => {
      if (typeof document !== "undefined" && document.hidden) return;
      try {
        const history = await arkorbitApi.listMessages(orbitId);
        if (cancelled) return;
        const next = history
          .map(historyToChatMessage)
          .filter(Boolean) as ChatMessage[];
        setMessages((prev) => (sameChatMessages(prev, next) ? prev : next));
      } catch {
        // The visible chat should not flicker on a transient polling failure.
      }
    };
    const scheduleRefresh = () => {
      timer = window.setTimeout(() => {
        void refreshCurrentMessages().finally(() => {
          if (!cancelled) scheduleRefresh();
        });
      }, 10_000);
    };
    const handleVisibilityChange = () => {
      if (typeof document !== "undefined" && !document.hidden) {
        void refreshCurrentMessages();
      }
    };
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", handleVisibilityChange);
    }
    void refreshCurrentMessages().finally(() => {
      if (!cancelled) scheduleRefresh();
    });
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", handleVisibilityChange);
      }
    };
  }, [activeTranscriptId, orbitId, streaming]);

  useEffect(() => {
    let cancelled = false;
    setLoadingFiles(true);
    void arkorbitApi
      .listFiles(orbitId)
      .then((next) => {
        if (cancelled) return;
        setFiles(next);
        setFilesError(null);
        setSelectedFilePath((current) =>
          current && next.some((file) => file.path === current)
            ? current
            : next[0]?.path ?? null,
        );
      })
      .catch((err) => {
        if (cancelled) return;
        setFiles([]);
        setSelectedFilePath(null);
        setFileContent("");
        setFilesError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoadingFiles(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filesReloadSignal, orbitId]);

  useEffect(() => {
    if (activeTab !== "files") return undefined;
    if (!selectedFilePath) {
      setFileContent("");
      return undefined;
    }
    let cancelled = false;
    setLoadingFileContent(true);
    void readOrbitChatFileText(orbitId, selectedFilePath)
      .then((text) => {
        if (!cancelled) {
          setFileContent(text);
          setFilesError(null);
        }
      })
      .catch((err) => {
        if (cancelled) return;
        setFileContent("");
        setFilesError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoadingFileContent(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeTab, filesReloadSignal, orbitId, selectedFilePath]);

  const updateAssistant = useCallback(
    (id: string, mutate: (message: ChatMessage) => ChatMessage) => {
      setMessages((prev) => prev.map((msg) => (msg.id === id ? mutate(msg) : msg)));
    },
    [],
  );

  const stop = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    const activeId = activeAssistantRef.current;
    if (activeId) {
      updateAssistant(activeId, (message) => ({
        ...message,
        status: "stopped",
        active: false,
        activity: message.text ? undefined : "Stopped.",
      }));
    }
    activeAssistantRef.current = null;
    setStreaming(false);
  }, [updateAssistant]);

  const newChat = useCallback(async () => {
    if (streaming) stop();
    try {
      await arkorbitApi.resetChat(orbitId);
      setMessages([]);
      setComposerResetSignal((value) => value + 1);
      setHistoryOpen(false);
      setHistoryPage(0);
      setActiveTranscriptId("current");
      void refreshTranscripts();
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      setMessages((prev) => [
        ...prev,
        {
          id: newId("new_chat_error"),
          role: "assistant",
          text: `Could not start a new chat: ${detail}`,
          status: "failed",
        },
      ]);
    }
  }, [orbitId, refreshTranscripts, stop, streaming]);

  const send = useCallback(
    async (trimmed: string) => {
      if (!trimmed || streaming || readOnly) return;

      const assistantId = newId();
      const submittedRuntimeNotices = runtimeNotices.slice(0, 6);
      onRuntimeNoticesSubmitted?.(submittedRuntimeNotices);
      setMessages((prev) => [
        ...prev,
        { id: newId(), role: "user", text: trimmed },
        {
          id: assistantId,
          role: "assistant",
          text: "",
          files: [],
          status: "running",
          active: true,
          activity: "Thinking...",
        },
      ]);
      setStreaming(true);
      activeAssistantRef.current = assistantId;

      const controller = new AbortController();
      abortRef.current = controller;
      try {
        const response = await fetch(arkorbitApi.orbitChatUrl(orbitId), {
          method: "POST",
          credentials: "include",
          signal: controller.signal,
          headers: {
            "Content-Type": "application/json",
            Accept: "text/event-stream",
          },
          body: JSON.stringify({
            message: trimmed,
            runtime_notices: submittedRuntimeNotices,
          }),
        });
        if (!response.ok) {
          const detail = extractFetchError(await response.text());
          updateAssistant(assistantId, (message) => ({
            ...message,
            text: detail || `Stream failed (HTTP ${response.status}).`,
            status: "failed",
            active: false,
            activity: undefined,
          }));
          return;
        }
        await readOrbitChatStream(response, {
          onToken: (content) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              text: message.text + content,
              status: "running",
              active: true,
              activity: undefined,
            })),
          onFileWritten: (path, operation, bytes) => {
            updateAssistant(assistantId, (message) => ({
              ...message,
              files: addFileChip(message.files, path, operation, bytes),
              status: "running",
              activity: fileActivityLabel(operation, path, bytes),
              active: true,
            }));
            setFilesReloadSignal((value) => value + 1);
            onFileWritten?.(path);
          },
          onRead: (_path) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              status: "running",
              activity: "Reading the canvas...",
              active: true,
            })),
          onStatus: (messageText) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              status: "running",
              activity: messageText,
              active: true,
            })),
          onUsage: (usage) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              usage,
            })),
          onError: (messageText) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              text: message.text.trim()
                ? `${message.text.trimEnd()}\n\n${messageText}`
                : messageText,
              status: "failed",
              active: false,
              activity: undefined,
            })),
          onDone: () =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              status:
                message.status === "failed" || message.status === "stopped"
                  ? message.status
                  : "completed",
              active: false,
              activity: undefined,
            })),
        });
        void refreshTranscripts();
      } catch (err) {
        if ((err as { name?: string })?.name !== "AbortError") {
          const detail = err instanceof Error ? err.message : String(err);
          updateAssistant(assistantId, (message) => ({
            ...message,
            text: detail || "Stream failed.",
            status: "failed",
            active: false,
            activity: undefined,
          }));
        }
      } finally {
        if (abortRef.current === controller) abortRef.current = null;
        if (activeAssistantRef.current === assistantId) activeAssistantRef.current = null;
        setStreaming(false);
      }
    },
    [
      orbitId,
      onFileWritten,
      onRuntimeNoticesSubmitted,
      readOnly,
      refreshTranscripts,
      runtimeNotices,
      streaming,
      updateAssistant,
    ],
  );

  return (
    <Box
      className={`orbit-chat-shell orbit-chat-panel-${visibleTurnState}${
        streaming ? " is-streaming" : ""
      }`}
    >
      <Box className="orbit-chat-header">
        <Stack sx={{ minWidth: 0 }}>
          <Typography variant="caption" className="orbit-chat-title">
            Orbit chat
          </Typography>
          <Typography variant="caption" className="orbit-chat-subtitle">
            {visibleTurnState === "running" ? (
              <OrbitRunningDots className="orbit-chat-running-dots-subtitle" />
            ) : (
              visibleTurnStatus
            )}
          </Typography>
        </Stack>
        <Stack direction="row" spacing={0.5} sx={{ alignItems: "center" }}>
          {readOnly ? (
            <Tooltip title="Current chat">
              <IconButton
                size="small"
                className="orbit-chat-tool"
                onClick={() => {
                  setActiveTab("chat");
                  setHistoryOpen(false);
                  setHistoryPage(0);
                  void loadTranscript("current");
                }}
                aria-label="Return to current chat"
              >
                <ArrowBackRoundedIcon fontSize="small" />
              </IconButton>
            </Tooltip>
          ) : null}
          <Tooltip title="Conversation history">
            <IconButton
              size="small"
              className="orbit-chat-tool"
              onClick={() => {
                setActiveTab("chat");
                setHistoryOpen((open) => {
                  const next = !open;
                  if (next) setHistoryPage(0);
                  return next;
                });
                void refreshTranscripts();
              }}
              aria-label="Conversation history"
            >
              <HistoryRoundedIcon fontSize="small" />
            </IconButton>
          </Tooltip>
          <Tooltip title="New chat">
            <IconButton
              size="small"
              className="orbit-chat-tool"
              onClick={() => {
                setActiveTab("chat");
                void newChat();
              }}
              aria-label="New chat"
            >
              <AddCommentRoundedIcon fontSize="small" />
            </IconButton>
          </Tooltip>
          {onClose ? (
            <Tooltip title="Close chat">
              <IconButton
                size="small"
                className="orbit-chat-tool"
                onClick={onClose}
                aria-label="Close chat"
              >
                <CloseRoundedIcon fontSize="small" />
              </IconButton>
            </Tooltip>
          ) : null}
        </Stack>
      </Box>
      <Box className="orbit-chat-tabs" role="tablist" aria-label="Orbit chat panels">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "chat"}
          className={`orbit-chat-tab${activeTab === "chat" ? " is-active" : ""}`}
          onClick={() => setActiveTab("chat")}
        >
          Chat
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "files"}
          className={`orbit-chat-tab${activeTab === "files" ? " is-active" : ""}`}
          onClick={() => setActiveTab("files")}
        >
          Files
          <span>{files.length}</span>
        </button>
      </Box>
      {activeTab === "chat" ? (
        <>
      {historyOpen ? (
        <Box className="orbit-chat-history">
          {transcripts.length === 0 ? (
            <Typography variant="caption" className="orbit-chat-history-empty">
              No previous conversations.
            </Typography>
          ) : (
            visibleTranscripts.map((transcript) => (
              <button
                key={transcript.id}
                type="button"
                className={`orbit-chat-history-item${
                  transcript.id === activeTranscriptId ? " is-active" : ""
                }`}
                onClick={() => {
                  setHistoryOpen(false);
                  setHistoryPage(0);
                  void loadTranscript(transcript.id);
                }}
              >
                <span className="orbit-chat-history-title">{transcript.title}</span>
                <span className="orbit-chat-history-meta">
                  {transcript.current ? "Current" : "Archived"} · {transcript.message_count} messages
                </span>
              </button>
            ))
          )}
          {transcripts.length > HISTORY_PAGE_SIZE ? (
            <Box className="orbit-chat-history-pager">
              <Tooltip title="Previous conversations">
                <span>
                  <IconButton
                    size="small"
                    className="orbit-chat-history-page-button"
                    disabled={normalizedHistoryPage === 0}
                    onClick={() => setHistoryPage((page) => Math.max(0, page - 1))}
                    aria-label="Previous conversation page"
                  >
                    <ChevronLeftRoundedIcon fontSize="small" />
                  </IconButton>
                </span>
              </Tooltip>
              <span className="orbit-chat-history-page-label">
                {normalizedHistoryPage + 1} / {historyPageCount}
              </span>
              <Tooltip title="More conversations">
                <span>
                  <IconButton
                    size="small"
                    className="orbit-chat-history-page-button"
                    disabled={normalizedHistoryPage >= historyPageCount - 1}
                    onClick={() =>
                      setHistoryPage((page) => Math.min(historyPageCount - 1, page + 1))
                    }
                    aria-label="Next conversation page"
                  >
                    <ChevronRightRoundedIcon fontSize="small" />
                  </IconButton>
                </span>
              </Tooltip>
            </Box>
          ) : null}
          <Divider className="orbit-chat-history-divider" />
        </Box>
      ) : null}
      <Box className="orbit-chat-messages" ref={scrollRef}>
        {loadingHistory ? (
          <Typography variant="body2" className="orbit-chat-empty">
            Loading chat...
          </Typography>
        ) : messages.length === 0 ? (
          <Typography variant="body2" className="orbit-chat-empty">
            No messages yet.
          </Typography>
        ) : (
          messages
            .filter(
              (msg) =>
                msg.role !== "assistant" ||
                Boolean(
                  msg.text.trim() ||
                    msg.activity ||
                    (msg.status && msg.status !== "completed") ||
                    msg.active ||
                    msg.files?.length,
                ),
            )
            .map((msg) => {
              const usageMetricItems =
                msg.role === "assistant" ? orbitUsageMetricItems(msg.usage) : [];
              const statusLabel = msg.role === "assistant" ? orbitStatusLabel(msg.status) : "";
              const displayActivity =
                msg.activity || (msg.role === "assistant" ? orbitStatusActivity(msg.status) : "");
              // Token usage metric items are still computed for backwards
              // compat but no longer rendered. They're operator telemetry
              // (total/input/output token counts) that competed for
              // attention with the actual message. The data is still
              // available via `msg.usage` for anyone wiring an explicit
              // dev-mode toggle later.
              void usageMetricItems;
              return (
                <Box
                  key={msg.id}
                  className={`orbit-chat-msg orbit-chat-msg-${msg.role}${
                    msg.active && displayActivity ? " orbit-chat-msg-active" : ""
                  }${msg.status ? ` orbit-chat-msg-status-${msg.status}` : ""}`}
                >
                  {/* HUD glyph — small icon in the top-left that says who
                      is talking. No "YOU" / "ORBIT" text label (those
                      were noisy); the glyph plus the left-edge accent
                      stripe carry identity together. Running status is
                      rendered inline next to the glyph so the user
                      knows when Orbit is mid-thought without a separate
                      status row. */}
                  <span className="orbit-chat-msg-glyph-row" aria-hidden="true">
                    <span
                      className={`orbit-chat-msg-glyph orbit-chat-msg-glyph-${msg.role}`}
                    >
                      {msg.role === "user" ? (
                        <PersonRoundedIcon className="orbit-chat-role-user-icon" />
                      ) : (
                        <img
                          src={AgentLogo}
                          alt=""
                          className="orbit-chat-role-agent-logo"
                        />
                      )}
                    </span>
                    {msg.status === "running" ? (
                      <OrbitRunningDots className="orbit-chat-running-dots-chip" />
                    ) : statusLabel ? (
                      <span className="orbit-chat-status-chip">
                        {statusLabel}
                      </span>
                    ) : null}
                  </span>
                  {msg.text ? (
                    <span className="orbit-chat-msg-text">{msg.text}</span>
                  ) : null}
                  {msg.role === "assistant" && msg.active && displayActivity ? (
                    <span className="orbit-chat-activity" aria-live="polite">
                      <span className="orbit-chat-activity-pulse" aria-hidden="true" />
                      <span className="orbit-chat-activity-label">{displayActivity}</span>
                      <span className="orbit-chat-activity-dots" aria-hidden="true">
                        <i />
                        <i />
                        <i />
                      </span>
                    </span>
                  ) : !msg.text && displayActivity ? (
                    <span className="orbit-chat-msg-text">{displayActivity}</span>
                  ) : null}
                  {msg.files?.length ? (
                    <Stack direction="row" spacing={0.75} className="orbit-file-chip-row">
                      {msg.files.map((file) => (
                        <span key={file.id} className="orbit-file-chip">
                          {file.operation === "edited" ? "Edited " : "Wrote "}
                          {file.path}
                          {file.bytes ? ` (${formatBytes(file.bytes)})` : ""}
                        </span>
                      ))}
                    </Stack>
                  ) : null}
                </Box>
              );
            })
        )}
      </Box>
      <OrbitChatComposer
        readOnly={readOnly}
        streaming={streaming}
        resetSignal={composerResetSignal}
        onSend={send}
        onStop={stop}
      />
        </>
      ) : (
        <OrbitChatFilesView
          files={files}
          selectedPath={selectedFilePath}
          content={fileContent}
          loadingFiles={loadingFiles}
          loadingContent={loadingFileContent}
          error={filesError}
          onSelect={setSelectedFilePath}
          onRefresh={() => setFilesReloadSignal((value) => value + 1)}
        />
      )}
    </Box>
  );
}

export default OrbitChat;
