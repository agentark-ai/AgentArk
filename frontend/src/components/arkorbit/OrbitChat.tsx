import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Box, Divider, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import AddCommentRoundedIcon from "@mui/icons-material/AddCommentRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import HistoryRoundedIcon from "@mui/icons-material/HistoryRounded";
import PersonRoundedIcon from "@mui/icons-material/PersonRounded";
import SendRoundedIcon from "@mui/icons-material/SendRounded";
import StopCircleRoundedIcon from "@mui/icons-material/StopCircleRounded";
import AgentLogo from "../../assets/logo.svg";
import { arkorbitApi } from "./api";
import type {
  OrbitChatFileChip,
  OrbitChatHistoryMessage,
  OrbitChatTranscript,
  OrbitChatUsage,
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
};

type Props = {
  orbitId: OrbitId;
  onFileWritten?: (path: string) => void;
  onClose?: () => void;
};

type StreamHandlers = {
  onToken: (content: string) => void;
  onFileWritten: (path: string, operation: OrbitFileOperation) => void;
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

function newId(prefix = "m"): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}_${Date.now().toString(36)}`;
}

function fileUpdateSentence(operation: OrbitFileOperation, path: string): string {
  return `I ${operation === "edited" ? "edited" : "wrote"} ${path}.`;
}

function fileActivityLabel(operation: OrbitFileOperation, path: string): string {
  return `I ${operation === "edited" ? "edited" : "wrote"} ${path}`;
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
  const totalTokens = normalized.total_tokens ?? inputTokens + outputTokens;
  if (totalTokens <= 0 && !normalized.time_to_first_token_ms) return [];
  return [
    { label: "Total tokens", value: Math.round(totalTokens).toLocaleString() },
    { label: "Input tokens", value: Math.round(inputTokens).toLocaleString() },
    { label: "Output tokens", value: Math.round(outputTokens).toLocaleString() },
    { label: "TTFT", value: formatDurationMs(normalized.time_to_first_token_ms) },
  ];
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

function fileChip(path: string, operation: OrbitFileOperation): OrbitChatFileChip {
  return { id: newId("f"), path, operation };
}

function addFileChip(
  files: OrbitChatFileChip[] | undefined,
  path: string,
  operation: OrbitFileOperation,
): OrbitChatFileChip[] {
  const current = files ?? [];
  if (current.some((file) => file.path === path && file.operation === operation)) {
    return current;
  }
  return [...current, fileChip(path, operation)];
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
  if (
    message.role === "assistant" &&
    !normalized.text.trim() &&
    normalized.files.length === 0
  ) {
    return null;
  }
  return {
    id: message.id || newId("h"),
    role: message.role,
    text: normalized.text,
    files: normalized.files,
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
    if (path) handlers.onFileWritten(path, operation);
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

export function OrbitChat({ orbitId, onFileWritten, onClose }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [transcripts, setTranscripts] = useState<OrbitChatTranscript[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [activeTranscriptId, setActiveTranscriptId] = useState("current");
  const [streaming, setStreaming] = useState(false);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [composerResetSignal, setComposerResetSignal] = useState(0);
  const abortRef = useRef<AbortController | null>(null);
  const activeAssistantRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const readOnly = activeTranscriptId !== "current";

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
    setActiveTranscriptId("current");
    setMessages([]);
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
    if (node) node.scrollTop = node.scrollHeight;
  }, [messages]);

  useEffect(() => () => abortRef.current?.abort(), []);

  useEffect(() => {
    if (activeTranscriptId !== "current" || streaming) return undefined;
    let cancelled = false;
    const refreshCurrentMessages = async () => {
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
    const timer = window.setInterval(() => {
      void refreshCurrentMessages();
    }, 2500);
    void refreshCurrentMessages();
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeTranscriptId, orbitId, streaming]);

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
        },
      ]);
    }
  }, [orbitId, refreshTranscripts, stop, streaming]);

  const send = useCallback(
    async (trimmed: string) => {
      if (!trimmed || streaming || readOnly) return;

      const assistantId = newId();
      setMessages((prev) => [
        ...prev,
        { id: newId(), role: "user", text: trimmed },
        {
          id: assistantId,
          role: "assistant",
          text: "",
          files: [],
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
          body: JSON.stringify({ message: trimmed }),
        });
        if (!response.ok) {
          const detail = extractFetchError(await response.text());
          updateAssistant(assistantId, (message) => ({
            ...message,
            text: detail || `Stream failed (HTTP ${response.status}).`,
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
              active: false,
              activity: undefined,
            })),
          onFileWritten: (path, operation) => {
            updateAssistant(assistantId, (message) => ({
              ...message,
              files: addFileChip(message.files, path, operation),
              activity: fileActivityLabel(operation, path),
              active: true,
            }));
            onFileWritten?.(path);
          },
          onRead: (path) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
              activity: `I'm reading this file: ${path}`,
              active: true,
            })),
          onStatus: (messageText) =>
            updateAssistant(assistantId, (message) => ({
              ...message,
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
              text: messageText,
              active: false,
              activity: undefined,
            })),
          onDone: () =>
            updateAssistant(assistantId, (message) => ({
              ...message,
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
    [orbitId, onFileWritten, readOnly, refreshTranscripts, streaming, updateAssistant],
  );

  return (
    <Box className="orbit-chat-shell">
      <Box className="orbit-chat-header">
        <Stack sx={{ minWidth: 0 }}>
          <Typography variant="caption" className="orbit-chat-title">
            Orbit chat
          </Typography>
          <Typography variant="caption" className="orbit-chat-subtitle">
            {readOnly ? "Archived conversation" : "Current conversation"}
          </Typography>
        </Stack>
        <Stack direction="row" spacing={0.5} sx={{ alignItems: "center" }}>
          {readOnly ? (
            <Tooltip title="Current chat">
              <IconButton
                size="small"
                className="orbit-chat-tool"
                onClick={() => {
                  setHistoryOpen(false);
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
                setHistoryOpen((open) => !open);
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
              onClick={() => void newChat()}
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
      {historyOpen ? (
        <Box className="orbit-chat-history">
          {transcripts.length === 0 ? (
            <Typography variant="caption" className="orbit-chat-history-empty">
              No previous conversations.
            </Typography>
          ) : (
            transcripts.map((transcript) => (
              <button
                key={transcript.id}
                type="button"
                className={`orbit-chat-history-item${
                  transcript.id === activeTranscriptId ? " is-active" : ""
                }`}
                onClick={() => {
                  setHistoryOpen(false);
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
                Boolean(msg.text.trim() || msg.activity || msg.active || msg.files?.length),
            )
            .map((msg) => {
              const usageMetricItems =
                msg.role === "assistant" ? orbitUsageMetricItems(msg.usage) : [];
              return (
                <Box
                  key={msg.id}
                  className={`orbit-chat-msg orbit-chat-msg-${msg.role}${
                    msg.active && msg.activity ? " orbit-chat-msg-active" : ""
                  }`}
                >
                  <span className="orbit-chat-msg-role">
                    <span className="orbit-chat-role-avatar" aria-hidden="true">
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
                    <span>{msg.role === "user" ? "You" : "AgentArk"}</span>
                  </span>
                  {msg.text ? (
                    <span className="orbit-chat-msg-text">{msg.text}</span>
                  ) : null}
                  {msg.role === "assistant" && msg.active && msg.activity ? (
                    <span className="orbit-chat-activity" aria-live="polite">
                      <span className="orbit-chat-activity-pulse" aria-hidden="true" />
                      <span className="orbit-chat-activity-label">{msg.activity}</span>
                      <span className="orbit-chat-activity-dots" aria-hidden="true">
                        <i />
                        <i />
                        <i />
                      </span>
                    </span>
                  ) : !msg.text && msg.activity ? (
                    <span className="orbit-chat-msg-text">{msg.activity}</span>
                  ) : null}
                  {msg.files?.length ? (
                    <Stack direction="row" spacing={0.75} className="orbit-file-chip-row">
                      {msg.files.map((file) => (
                        <span key={file.id} className="orbit-file-chip">
                          {file.operation === "edited" ? "Edited " : "Wrote "}
                          {file.path}
                        </span>
                      ))}
                    </Stack>
                  ) : null}
                  {usageMetricItems.length > 0 ? (
                    <Box
                      className="orbit-chat-run-metrics"
                      aria-label="Orbit run metrics"
                      title={orbitUsageTitle(msg.usage)}
                    >
                      {usageMetricItems.map((item) => (
                        <span
                          key={`${msg.id}:${item.label}`}
                          className="orbit-chat-run-metric"
                        >
                          <span className="orbit-chat-run-metric-label">
                            {item.label}
                          </span>
                          <span className="orbit-chat-run-metric-value">
                            {item.value}
                          </span>
                        </span>
                      ))}
                    </Box>
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
    </Box>
  );
}

export default OrbitChat;
