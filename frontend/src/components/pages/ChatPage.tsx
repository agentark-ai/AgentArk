import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Autocomplete,
  Avatar,
  Box,
  Button,
  ButtonBase,
  Checkbox,
  CircularProgress,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Drawer,
  Divider,
  IconButton,
  List,
  ListItem,
  ListItemText,
  Link,
  Menu,
  MenuItem,
  Stack,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ArrowDropDownRoundedIcon from "@mui/icons-material/ArrowDropDownRounded";
import AttachFileRoundedIcon from "@mui/icons-material/AttachFileRounded";
import TravelExploreRoundedIcon from "@mui/icons-material/TravelExploreRounded";
import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import ErrorOutlineRoundedIcon from "@mui/icons-material/ErrorOutlineRounded";
import FileDownloadRoundedIcon from "@mui/icons-material/FileDownloadRounded";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import OpenInFullRoundedIcon from "@mui/icons-material/OpenInFullRounded";
import ArticleRoundedIcon from "@mui/icons-material/ArticleRounded";
import PictureAsPdfRoundedIcon from "@mui/icons-material/PictureAsPdfRounded";
import RadioButtonUncheckedRoundedIcon from "@mui/icons-material/RadioButtonUncheckedRounded";
import StarBorderRoundedIcon from "@mui/icons-material/StarBorderRounded";
import StarRoundedIcon from "@mui/icons-material/StarRounded";
import ArrowUpwardRoundedIcon from "@mui/icons-material/ArrowUpwardRounded";
import StopRoundedIcon from "@mui/icons-material/StopRounded";
import CloseIcon from "@mui/icons-material/Close";
import FilterListRoundedIcon from "@mui/icons-material/FilterListRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Sparkles,
  UserRound,
  Box as CubeIcon,
  Eye,
  Image as ImageIcon,
  Search,
  Network,
  Globe,
  Lock,
  ArrowRight,
  type LucideIcon,
} from "lucide-react";
import "./chatLanding.css";
import {
  Fragment,
  memo,
  isValidElement,
  useCallback,
  useEffect,
  useDeferredValue,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type CSSProperties,
  type DragEvent,
  type JSX,
  type MouseEvent,
  type ReactNode,
} from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  apiOutputPathFromHref,
  apiUrl,
  downloadApiFile,
} from "../../api/client";
import AgentLogo from "../../assets/logo.svg";
import { MetricBarCard } from "../analytics/MetricBarCard";
import { CompanionDevicesPanel } from "../CompanionDevicesPanel";
import { IntegrationQuickstartPanel } from "../IntegrationQuickstartPanel";
import { IntegrationsPanel } from "../IntegrationsPanel";
import { LiveEventConsole } from "../LiveEventConsole";
import { ObservabilityPanel } from "../ObservabilityPanel";
import { PluginSdkPanel } from "../PluginSdkPanel";
import {
  SuggestionRunDialog,
  type SuggestionRunState,
} from "../SuggestionRunDialog";
import { WebhooksPanel } from "../WebhooksPanel";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { isOmittedContentPlaceholder } from "../chat/computerPaneFileContent";
import {
  getTunnelAccessMeta,
  getTunnelPanelPasswordPrompt,
  getTunnelPanelResumeMessage,
  getTunnelPanelStartMessage,
  getTunnelPanelStartingMessage,
  getTunnelPanelWarning,
  getTunnelProviderHelp,
  getTunnelStartButtonLabel,
  getTunnelStopButtonLabel,
  getTunnelUrlFieldLabel,
} from "../../lib/tunnelAccess";
import {
  formatUiDateOnly,
  formatUiDateRange,
  formatUiTime,
  formatUiDateTime,
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
  getRequestUiTimeZone,
} from "../../lib/dateFormat";
import { humanizeMachineLabel } from "../../lib/displayLabels";
import {
  isBackgroundSessionVisibleInUi,
  isOneShotReminderTask,
  taskActionDisplay,
  taskKind,
  taskKindLabel,
} from "../../lib/backgroundSessions";
import {
  TASK_CANCEL_CONTROLS_ENABLED,
  TASK_RETRY_CONTROLS_ENABLED,
} from "../../lib/featureFlags";
import {
  buildRunPayloadViewFromSources,
  type RunPayloadItem,
  type RunPayloadView,
} from "../chat/runPayloadView";
import type {
  PulseRemediationSpec,
  PulseRunFixRequest,
  BackgroundSessionSummary,
  SkillImportResponse,
  Task,
  TraceOperationalEvent,
  TraceSummary,
} from "../../types";
import { ComputerPane } from "../chat";
import type { SurfaceDescriptor } from "../chat/types";
import { surfaceFromCard, surfaceFromValue } from "../chat/surface";
import { readablePayloadFromValue } from "../chat/readablePayload";
import {
  InlineAgentArkChart,
  isAgentArkChartFence,
} from "../chat/InlineAgentArkChart";
import { guessCodeLanguage, renderCodeBlockLines } from "../chat/codeHighlight";
import {
  buildChatRunMetricItems,
  chatRunMetricMessageFieldsFromPayload,
  chatRunMetricsFromPayload,
  type ChatRunMetricItem,
  type ChatRunMetrics,
} from "./chatRunMetrics";
import { buildChatLiveRunArchive } from "./chatLiveRunArchive";

// Chat layout mode: "split" runs the prose+action-row chat with a focused
// Computer pane on the right. "classic" keeps the original inline timeline
// view. Flip to "classic" to instantly revert if the new layout regresses.
const CHAT_LAYOUT_MODE: "split" | "classic" = "split";

const REFRESH_MS = 8000;
const EVOLUTION_DEV_QUERY_LIMIT = 250;
const EVOLUTION_DEV_REFRESH_MS = 30000;
const DEVELOPER_MODE_STORAGE_KEY = "agentark.developer_mode";
const DEVELOPER_MODE_EVENT = "agentark:developer-mode-change";
const OLLAMA_DEFAULT_BASE_URL = "http://localhost:11434";
const OPENROUTER_DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const SHOW_EXPERIMENTAL_AUTONOMY_TOOLS = false;
const CHAT_LAST_CONVERSATION_STORAGE_KEY = "agentark.chat.lastConversationId";
const CHAT_DRAFT_MODE_STORAGE_KEY = "agentark.chat.draftMode";
const CHAT_COMPOSER_PREFILL_STORAGE_KEY = "agentark.chat.composerPrefill";
const CHAT_COMPOSER_PREFILL_EVENT = "agentark.chat.composer-prefill";
const ARKREFLECT_COMPOSER_PREFILL_STORAGE_KEY = "arkreflect.composerPrefill";
const CHAT_PENDING_RUN_STORAGE_KEY = "agentark.chat.pendingRun";
const CHAT_BACKGROUND_RUN_STORAGE_KEY = "agentark.chat.backgroundRun";
const CHAT_PENDING_LAUNCH_STORAGE_KEY = "agentark.chat.pendingLaunch";
const CHAT_WORKSPACE_SNAPSHOTS_STORAGE_KEY = "agentark.chat.workspaceSnapshots";
const CHAT_PENDING_RUN_TTL_MS = 45 * 60 * 1000;
const CHAT_WORKING_CHATS_MAX = 3;
const CHAT_BACKGROUND_RUN_SNAPSHOTS_MAX = 12;
const CHAT_WORKSPACE_SNAPSHOT_TTL_MS = 12 * 60 * 60 * 1000;
const CHAT_EARLY_ACCESS_NOTICE_STORAGE_KEY =
  "agentark.chat.earlyAccessNoticeDismissedUntil";
const CHAT_EARLY_ACCESS_NOTICE_DISMISS_MS = 7 * 24 * 60 * 60 * 1000;
const CHAT_WORKSPACE_SNAPSHOT_MAX_CONVERSATIONS = 10;
const CHAT_WORKSPACE_SNAPSHOT_MAX_FILES = 24;
const CHAT_WORKSPACE_SNAPSHOT_MAX_FILE_CHARS = 60_000;
const CHAT_WORKSPACE_SNAPSHOT_MAX_TOTAL_CHARS = 240_000;
const CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS = 16000;
const CHAT_PENDING_STREAM_STEPS_MAX = 48;
const CHAT_STREAMING_STEPS_UI_MAX = 120;
const CHAT_WORKSPACE_ACTIVITY_RENDER_MAX = 60;
const CHAT_ACTIVITY_PAYLOAD_STRING_MAX_CHARS = 1600;
const CHAT_ACTIVITY_STREAM_STRING_MAX_CHARS = 6000;
const CHAT_ACTIVITY_TRACE_JSON_MAX_CHARS = 4000;
const CHAT_ACTIVITY_PAYLOAD_ARRAY_MAX_ITEMS = 80;
const CHAT_ACTIVITY_PAYLOAD_OBJECT_MAX_KEYS = 80;
const CHAT_ACTIVITY_PAYLOAD_DEPTH_MAX = 5;
const CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_CHARS = 480;
const CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_ITEMS = 12;
const CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_KEYS = 12;
const CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_DEPTH_MAX = 2;
const CHAT_WORKSPACE_UI_MAX_FILES = 120;
const CHAT_WORKSPACE_UI_MAX_FILE_CHARS = 80_000;
const CHAT_WORKSPACE_UI_MAX_TOTAL_CHARS = 480_000;
const CHAT_STREAMING_STEP_FLUSH_MS = 180;
const CHAT_REASONING_PREVIEW_FLUSH_MS = 180;
const CHAT_PENDING_RUN_SNAPSHOT_FLUSH_MS = 1200;
const CHAT_WORKSPACE_SNAPSHOT_FLUSH_MS = 300;
const CHAT_TRACE_STATE_CACHE_MAX = 8;
const CHAT_TRACE_EAGER_LOAD_MAX = 2;
const CHAT_TRACE_EAGER_LOAD_IDLE_DELAY_MS = 350;
const CHAT_COMPUTER_TOKEN_PREVIEW_MAX_CHARS = 12_000;
const CHAT_PROGRESS_MEMORY_MAX_CONVERSATIONS = 12;
const CHAT_PENDING_RUN_RECOVERY_GRACE_MS = 12_000;
const CHAT_INLINE_CONVERSATIONS_MIN_WIDTH = 1600;
const CHAT_INLINE_ACTIVITY_MIN_WIDTH = 1820;
const RESTART_NOTICE_DURATION_MS = 10_000;
const UPDATE_NOTICE_DURATION_MS = 120_000;
const CHAT_LAUNCH_RUN_EVENT = "agentark.chat.launch-run";
const CHAT_RUN_STATUS_EVENT = "agentark.chat.run-status";
const CHAT_CONVERSATIONS_PAGE_SIZE = 20;
const CHAT_STARRED_LIMIT = 3;

function isChatRoutePath(pathname: string): boolean {
  const normalized = pathname.replace(/\/+$/, "") || "/";
  return normalized === "/ui/chat" || normalized === "/ui/v2/chat";
}

function readChatRouteConversationId(): string | null {
  if (typeof window === "undefined" || !isChatRoutePath(window.location.pathname)) {
    return null;
  }
  try {
    const params = new URLSearchParams(window.location.search);
    return (
      params.get("conversation") ||
      params.get("conversation_id") ||
      params.get("conversationId") ||
      params.get("cid") ||
      ""
    ).trim() || null;
  } catch {
    return null;
  }
}

function writeChatRouteConversationId(conversationId: string | null): void {
  if (typeof window === "undefined" || !isChatRoutePath(window.location.pathname)) {
    return;
  }
  const params = new URLSearchParams(window.location.search);
  params.delete("conversation_id");
  params.delete("conversationId");
  params.delete("cid");
  const normalizedConversationId = (conversationId || "").trim();
  if (normalizedConversationId) {
    params.set("conversation", normalizedConversationId);
  } else {
    params.delete("conversation");
  }
  const nextSearch = params.toString();
  const nextUrl = `/ui/chat${nextSearch ? `?${nextSearch}` : ""}${window.location.hash}`;
  const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (nextUrl !== currentUrl) {
    window.history.replaceState(null, "", nextUrl);
  }
}

function readChatDraftMode(): boolean {
  if (typeof window === "undefined" || readChatRouteConversationId()) {
    return false;
  }
  try {
    return window.sessionStorage.getItem(CHAT_DRAFT_MODE_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeChatDraftMode(active: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (active) {
      window.sessionStorage.setItem(CHAT_DRAFT_MODE_STORAGE_KEY, "1");
    } else {
      window.sessionStorage.removeItem(CHAT_DRAFT_MODE_STORAGE_KEY);
    }
  } catch {
    // Ignore storage failures.
  }
}

function readEarlyAccessNoticeDismissed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    const raw = window.localStorage.getItem(
      CHAT_EARLY_ACCESS_NOTICE_STORAGE_KEY,
    );
    const dismissedUntil = raw ? Number(raw) : 0;
    if (Number.isFinite(dismissedUntil) && dismissedUntil > Date.now()) {
      return true;
    }
    window.localStorage.removeItem(CHAT_EARLY_ACCESS_NOTICE_STORAGE_KEY);
  } catch {
    // Ignore storage failures and show the notice.
  }
  return false;
}

function dismissEarlyAccessNoticeForSevenDays(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      CHAT_EARLY_ACCESS_NOTICE_STORAGE_KEY,
      String(Date.now() + CHAT_EARLY_ACCESS_NOTICE_DISMISS_MS),
    );
  } catch {
    // Ignore storage failures; the current render still dismisses locally.
  }
}

type RestartNoticeState = {
  text: string;
  durationMs: number;
  etaLabel: string;
};
const AUTO_APPROVE_BLOCKED_ACTIONS = [
  "shell",
  "bash",
  "code_execute",
  "file_write",
  "file_delete",
  "file_move",
  "docker_exec",
  "http_request",
  "lan_discover",
  "gmail_send",
] as const;
const AUTO_APPROVE_ACTION_OPTIONS = [
  "web_search",
  "research",
  "generate_image",
  "generate_video",
  "browse",
  "file_read",
  "http_get",
  "schedule_task",
  "list_tasks",
  "clipboard_read",
  "clipboard_write",
  "gmail_scan",
  "gmail_reply",
] as const;
type ChatPendingRunMode = "fresh" | "resume";
type ChatPendingRunPhase = "running" | "interrupted" | "awaiting_confirmation";

type ChatTurnAttachment = {
  name: string;
  kind: "document" | "visual" | "file";
  id?: string;
  detail?: string;
};

type ChatPendingRunSnapshot = {
  conversationId: string;
  message: string;
  startedAt: number;
  initialMessageCount?: number;
  runId?: string;
  mode?: ChatPendingRunMode;
  phase?: ChatPendingRunPhase;
  taskId?: string;
  streamingResponse?: string;
  streamingSteps?: JsonRecord[];
  failedUserMessage?: string;
  lastRunSeq?: number;
  attachments?: ChatTurnAttachment[];
};

type ChatPendingRunSnapshotMap = Record<string, ChatPendingRunSnapshot>;

type ChatWorkspaceSnapshot = {
  conversationId: string;
  updatedAt: number;
  deployedFiles: WorkspaceFileEntry[];
  liveFileWrites: Record<string, LiveFileWriteState>;
  streamedWorkspaceApp?: JsonRecord | null;
  codeViewerFileIdx?: number;
};

type ChatLaunchRunDetail = {
  message: string;
  conversationId?: string;
  newConversation?: boolean;
  taskId?: string;
  launchMode?: "message" | "resume_task";
  navigateToChat?: boolean;
  source?: string;
  resolve?: (started: boolean) => void;
  reject?: (message: string) => void;
};

type ChatPendingLaunch = {
  createdAt: number;
  launchMode: "message" | "resume_task";
  message?: string;
  conversationId?: string;
  newConversation?: boolean;
  taskId?: string;
  source?: string;
  acceptedSuggestionId?: string;
  sentinelProposalId?: string;
};

type ChatRunStatusDetail = {
  conversationId: string;
  status: "completed" | "error";
  source?: string;
  message: string;
};

type ChatExecutionMode = "auto" | "chat" | "task";

type WorkspaceFileEntry = {
  name: string;
  content: string;
};

type WorkspaceSnippetEntry = {
  id: string;
  name: string;
  displayName: string;
  content: string;
  languageHint: string;
  sourceMessageId: string;
  sourceLabel: string;
};

type CodePreviewOpenRequest = {
  snippetId?: string;
  fileName?: string;
  code?: string;
  languageHint?: string;
};

type ResearchReportPreview = {
  kind: "research" | "deep";
  title: string;
  summary: string;
  summaryPreview: string;
  keyFindings: string[];
  keyFindingCount: number;
  openQuestions: string[];
  contradictions: string[];
  highlights: string[];
  sourceCount: number;
  tableCount: number;
  chartCount: number;
  openQuestionCount: number;
  contradictionCount: number;
  mainContent: string;
  evidenceBrief: string;
  content: string;
};

type ResearchReportDialogState = {
  report: ResearchReportPreview;
  messageId: string;
  previousUserPrompt: string;
  timestamp?: string;
  traceId?: string;
};

const MODEL_FALLBACKS_BY_PROVIDER: Record<string, string[]> = {
  openai: ["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini", "o3"],
  anthropic: [
    "claude-opus-4-20250514",
    "claude-sonnet-4-20250514",
    "claude-3-7-sonnet-latest",
    "claude-3-5-haiku-latest",
  ],
  openrouter: [
    "openai/gpt-5",
    "anthropic/claude-sonnet-4",
    "google/gemini-2.5-pro",
  ],
  "openai-compatible": [],
  ollama: [],
};

const MODEL_PROVIDER_OPTIONS = [
  { value: "ollama", label: "ollama" },
  { value: "anthropic", label: "anthropic" },
  { value: "openai", label: "openai" },
  { value: "openrouter", label: "openrouter" },
  { value: "huggingface", label: "huggingface inference" },
  { value: "openai-compatible", label: "openai-compatible" },
];

function getDeveloperModeEnabled(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(DEVELOPER_MODE_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function setDeveloperModeEnabled(next: boolean): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(DEVELOPER_MODE_STORAGE_KEY, next ? "1" : "0");
  } catch {
    // Ignore storage write errors and still emit event for current session.
  }
  window.dispatchEvent(
    new CustomEvent(DEVELOPER_MODE_EVENT, { detail: { enabled: next } }),
  );
}

type JsonRecord = Record<string, unknown>;

function pruneRecordToAllowedKeys<T>(
  value: Record<string, T>,
  allowedKeys: Set<string>,
): Record<string, T> {
  const entries = Object.entries(value);
  if (entries.every(([key]) => allowedKeys.has(key))) return value;
  const nextEntries = entries.filter(([key]) => allowedKeys.has(key));
  if (nextEntries.length === entries.length) return value;
  return Object.fromEntries(nextEntries) as Record<string, T>;
}

type ChatClarificationChoice = {
  label: string;
  submitText: string;
  kind?: string;
  approval?: {
    id: string;
    decision: "approve" | "reject";
    actionName: string;
    steps?: ChatApprovalStep[];
  };
};
type ChatApprovalStep = {
  actionName: string;
  argumentsPreview?: unknown;
};

function parseInternalApprovalSubmitToken(
  value: string,
): ChatClarificationChoice["approval"] | null {
  const parts = value
    .trim()
    .split(":")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length < 3) return null;

  const approvalId = parts[parts.length - 1] ?? "";
  if (
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      approvalId,
    )
  ) {
    return null;
  }

  const protocol = parts.slice(0, -1).join(":").toLowerCase();
  if (!protocol.includes("direct_chat") || !protocol.includes("approval")) {
    return null;
  }

  const decision = parts
    .slice(0, -1)
    .map((part) => part.toLowerCase())
    .find((part) => part === "approve" || part === "reject");
  if (decision !== "approve" && decision !== "reject") return null;

  return {
    id: approvalId,
    decision,
    actionName: "",
  };
}

function isDirectChatApprovalChoice(choice: ChatClarificationChoice): boolean {
  return Boolean(
    choice.approval ||
      choice.kind === "direct_chat_approval" ||
      choice.kind === "direct_chat_chain_approval" ||
      parseInternalApprovalSubmitToken(choice.submitText),
  );
}

type PasswordDialogMode = "set" | "change" | "remove";
type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

type LiveFileWriteState = {
  content: string;
  line: number;
  totalLines: number;
  done: boolean;
};

type StreamPhaseStatus = {
  toolName: string;
  phase: string;
  label: string;
  detail: string;
  status: string;
  elapsedSecs: number;
  streamKey: string;
  planStepId: number | null;
  planStepTitle: string;
};

const APP_DELIVERY_PLAN_TOOL_PREFIX = "app_delivery:";
const CAPABILITY_SETUP_PLAN_TOOL_PREFIX = "capability_setup:";
const APP_DELIVERY_PHASE_ORDER = [
  "planning",
  "deploying",
  "generating_files",
  "preparing_runtime",
  "installing",
  "starting_runtime",
  "waiting_for_inputs",
  "completed",
] as const;
const APP_DELIVERY_PHASE_INDEX: ReadonlyMap<string, number> = new Map(
  APP_DELIVERY_PHASE_ORDER.map((phase, index) => [phase, index]),
);
const CAPABILITY_SETUP_PHASE_ORDER = [
  "resolve_target",
  "inspect_local_catalog",
  "resolve_ambiguity",
  "install_or_scaffold",
  "configure_auth",
  "verify_registration",
  "report_controls",
] as const;
const CAPABILITY_SETUP_PHASE_INDEX: ReadonlyMap<string, number> = new Map(
  CAPABILITY_SETUP_PHASE_ORDER.map((phase, index) => [phase, index]),
);

const CODE_PREVIEW_LANGUAGE_LABELS: Record<string, string> = {
  bash: "Bash",
  c: "C",
  cpp: "C++",
  csharp: "C#",
  css: "CSS",
  go: "Go",
  html: "HTML",
  java: "Java",
  javascript: "JavaScript",
  json: "JSON",
  jsx: "JSX",
  kotlin: "Kotlin",
  less: "Less",
  markdown: "Markdown",
  php: "PHP",
  powershell: "PowerShell",
  python: "Python",
  ruby: "Ruby",
  rust: "Rust",
  scss: "SCSS",
  sql: "SQL",
  toml: "TOML",
  tsx: "TSX",
  typescript: "TypeScript",
  xml: "XML",
  yaml: "YAML",
};

function normalizeCodeFenceLanguage(raw = ""): string {
  const normalized = raw
    .trim()
    .replace(/^language-/i, "")
    .toLowerCase();
  if (!normalized) return "";
  const aliases: Record<string, string> = {
    cjs: "javascript",
    env: "toml",
    htm: "html",
    js: "javascript",
    md: "markdown",
    ps1: "powershell",
    py: "python",
    rs: "rust",
    sh: "bash",
    shell: "bash",
    ts: "typescript",
    yml: "yaml",
    zsh: "bash",
  };
  return aliases[normalized] || normalized;
}

function inferCodePreviewFileName(languageHint = "", code = ""): string {
  const normalized = normalizeCodeFenceLanguage(languageHint);
  switch (normalized) {
    case "html":
    case "xml":
      return "index.html";
    case "css":
      return "styles.css";
    case "scss":
      return "styles.scss";
    case "less":
      return "styles.less";
    case "json":
      return "data.json";
    case "python":
      return "main.py";
    case "sql":
      return "query.sql";
    case "bash":
      return "script.sh";
    case "powershell":
      return "script.ps1";
    case "markdown":
      return "README.md";
    case "yaml":
      return "config.yml";
    case "toml":
      return "config.toml";
    case "typescript":
      return "main.ts";
    case "tsx":
      return "App.tsx";
    case "javascript":
      return "main.js";
    case "jsx":
      return "App.jsx";
    case "go":
      return "main.go";
    case "java":
      return "Main.java";
    case "kotlin":
      return "Main.kt";
    case "php":
      return "index.php";
    case "ruby":
      return "main.rb";
    case "rust":
      return "main.rs";
    case "c":
      return "main.c";
    case "cpp":
      return "main.cpp";
    case "csharp":
      return "Program.cs";
    default:
      break;
  }

  switch (guessCodeLanguage("", code)) {
    case "markup":
      return "index.html";
    case "css":
      return "styles.css";
    case "json":
      return "data.json";
    case "python":
      return "main.py";
    case "sql":
      return "query.sql";
    case "shell":
      return "script.sh";
    case "markdown":
      return "README.md";
    case "config":
      return "config.toml";
    case "script":
      return "main.ts";
    default:
      return "snippet.txt";
  }
}

function formatCodePreviewLanguage(
  languageHint = "",
  fileName = "",
  code = "",
): string {
  const normalized = normalizeCodeFenceLanguage(languageHint);
  if (normalized)
    return CODE_PREVIEW_LANGUAGE_LABELS[normalized] || normalized.toUpperCase();

  switch (guessCodeLanguage(fileName, code)) {
    case "markup":
      return "HTML";
    case "css":
      return "CSS";
    case "json":
      return "JSON";
    case "python":
      return "Python";
    case "sql":
      return "SQL";
    case "shell":
      return "Shell";
    case "markdown":
      return "Markdown";
    case "config":
      return "Config";
    case "script":
      return "Code";
    default:
      return "Text";
  }
}

function isWorkspaceCodePreview(
  languageHint = "",
  code = "",
  fileName = "",
): boolean {
  const normalizedLanguage = normalizeCodeFenceLanguage(languageHint);
  const explicitPlainText =
    normalizedLanguage === "text" ||
    normalizedLanguage === "txt" ||
    normalizedLanguage === "plain" ||
    normalizedLanguage === "plaintext";
  if (explicitPlainText) return false;
  if (normalizedLanguage && normalizedLanguage !== "markdown") return true;

  const trimmed = (code || "").trim();
  if (!trimmed) return false;
  const guessed = guessCodeLanguage(fileName, trimmed);
  if (guessed === "text") return false;
  if (guessed === "markdown") {
    const lineCount = trimmed
      .split(/\r?\n/)
      .filter((line) => line.trim()).length;
    return Boolean(normalizedLanguage) && lineCount >= 3;
  }
  return true;
}

function reactNodeToPlainText(node: ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node))
    return node.map((child) => reactNodeToPlainText(child)).join("");
  return "";
}

// Like reactNodeToPlainText but recurses into element children, so we can read
// the text of already-rendered markdown nodes (e.g. a blockquote's paragraphs)
// to detect GitHub-style callout markers.
function deepNodeText(node: ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node))
    return node.map((child) => deepNodeText(child)).join("");
  if (isValidElement<{ children?: ReactNode }>(node))
    return deepNodeText(node.props.children);
  return "";
}

// GitHub-style callout / admonition kinds. Pairs with the .chat-md-callout-*
// styles in styles/20-chat-core.css.
const CHAT_CALLOUT_META: Record<string, { label: string; icon: string }> = {
  note: { label: "Note", icon: "ⓘ" },
  tip: { label: "Tip", icon: "✦" },
  important: { label: "Important", icon: "❉" },
  warning: { label: "Warning", icon: "⚠" },
  caution: { label: "Caution", icon: "⚠" },
};

function extractMarkdownCodeBlock(
  children: ReactNode,
): { className?: string; code: string } | null {
  const nodes = Array.isArray(children) ? children : [children];
  for (const node of nodes) {
    if (!isValidElement<{ className?: string; children?: ReactNode }>(node))
      continue;
    const code = reactNodeToPlainText(node.props.children)
      .replace(/\r\n/g, "\n")
      .replace(/\n$/, "");
    if (!code) continue;
    return {
      className: str(node.props.className, ""),
      code,
    };
  }
  return null;
}

// memo: code blocks are pure functions of their props; skipping re-render
// here avoids re-highlighting every fence whenever a parent re-renders.
const InlineCodePreview = memo(function InlineCodePreview({
  code,
  languageHint,
  fileName,
  snippetId,
  onOpenInWorkspace,
}: {
  code: string;
  languageHint?: string;
  fileName?: string;
  snippetId?: string;
  onOpenInWorkspace?: (request: CodePreviewOpenRequest) => void;
}) {
  const [copied, setCopied] = useState(false);
  const normalizedCode = (code || "").replace(/\r\n/g, "\n").replace(/\n$/, "");
  if (!normalizedCode) return null;

  const resolvedFileName =
    fileName || inferCodePreviewFileName(languageHint, normalizedCode);
  const languageLabel = formatCodePreviewLanguage(
    languageHint,
    resolvedFileName,
    normalizedCode,
  );
  const shouldUseWorkspaceChrome = isWorkspaceCodePreview(
    languageHint,
    normalizedCode,
    resolvedFileName,
  );

  if (!shouldUseWorkspaceChrome) {
    return (
      <pre className="chat-md-code chat-md-code-plain">
        <code>{normalizedCode}</code>
      </pre>
    );
  }

  return (
    <Box className="chat-md-ide">
      <Box className="chat-md-ide-bar">
        <Box className="chat-md-ide-controls" aria-hidden="true">
          <span className="chat-md-ide-dot chat-md-ide-dot-close" />
          <span className="chat-md-ide-dot chat-md-ide-dot-minimize" />
          <span className="chat-md-ide-dot chat-md-ide-dot-expand" />
        </Box>
        <span className="chat-md-ide-tab" title={resolvedFileName}>
          {resolvedFileName}
        </span>
        <span className="chat-md-ide-meta">{languageLabel}</span>
        <button
          type="button"
          className="chat-md-ide-copy"
          onClick={(event) => {
            event.stopPropagation();
            void navigator.clipboard?.writeText(normalizedCode);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1400);
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
        {onOpenInWorkspace ? (
          <button
            type="button"
            className="chat-md-ide-open"
            onClick={(event) => {
              event.stopPropagation();
              onOpenInWorkspace({
                snippetId,
                fileName: resolvedFileName,
                code: normalizedCode,
                languageHint,
              });
            }}
          >
            Open in workspace
          </button>
        ) : null}
      </Box>
      <pre className="code-viewer-pre chat-md-ide-pre">
        <code>
          {renderCodeBlockLines(normalizedCode, { fileName: resolvedFileName })}
        </code>
      </pre>
    </Box>
  );
});

// Allocation-free newline count; `value.split(/\r?\n/).length` allocates one
// string per line, which turns per-delta line counting over a growing file
// into serious GC pressure during code-generation runs.
function countContentLines(value: string): number {
  if (!value) return 0;
  let lines = 1;
  let idx = value.indexOf("\n");
  while (idx !== -1) {
    lines += 1;
    idx = value.indexOf("\n", idx + 1);
  }
  return lines;
}

function canonicalizeLiveFileWrites(
  current: Record<string, LiveFileWriteState>,
  appDir = "",
): Record<string, LiveFileWriteState> {
  const next: Record<string, LiveFileWriteState> = {};
  for (const [name, state] of Object.entries(current)) {
    const normalizedName = normalizeWorkspaceFileName(name, appDir);
    if (!normalizedName || !isLikelyWorkspaceFileName(normalizedName)) continue;
    const existing = next[normalizedName];
    if (!existing) {
      next[normalizedName] = {
        ...state,
        content: compactWorkspacePreviewContent(
          choosePreferredWorkspaceFileContent("", state.content),
          CHAT_WORKSPACE_UI_MAX_FILE_CHARS,
        ),
      };
      continue;
    }
    next[normalizedName] = {
      content: compactWorkspacePreviewContent(
        choosePreferredWorkspaceFileContent(existing.content, state.content),
        CHAT_WORKSPACE_UI_MAX_FILE_CHARS,
      ),
      line: Math.max(existing.line, state.line),
      totalLines: Math.max(existing.totalLines, state.totalLines),
      done: existing.done || state.done,
    };
  }
  return next;
}

function compactWorkspacePreviewContent(content: string, maxChars: number): string {
  if (!content || content.length <= maxChars) return content || "";
  const suffix = "\n\n/* UI preview truncated. */";
  if (maxChars <= suffix.length) return content.slice(0, Math.max(0, maxChars));
  const sliceLength = Math.max(0, maxChars - suffix.length);
  return `${content.slice(0, sliceLength).trimEnd()}${suffix}`;
}

type ExecutionPlanItem = {
  id: number;
  title: string;
  description: string;
  status: string;
  action?: string | null;
  arguments?: JsonRecord | null;
  tool_hint: string | null;
  substeps: ExecutionPlanSubstepItem[];
};

type ExecutionPlanSubstepItem = {
  id: number;
  title: string;
  description: string;
  status: string;
  tool_hint: string | null;
};

type ExecutionPlanState = {
  plan_id: string;
  revision: number;
  summary: string;
  steps: ExecutionPlanItem[];
};

type TaskProgressState = {
  done: number;
  total: number;
};

function isTerminalPlanStatus(status: unknown): boolean {
  return ["completed", "failed", "skipped"].includes(
    str(status, "").trim().toLowerCase(),
  );
}

function taskProgressFromExecutionPlan(
  plan: ExecutionPlanState | null | undefined,
): TaskProgressState | null {
  const steps = plan?.steps || [];
  if (steps.length === 0) return null;
  return {
    done: steps.filter((step) => isTerminalPlanStatus(step.status)).length,
    total: steps.length,
  };
}

function taskProgressFromActivityStep(step: JsonRecord): TaskProgressState | null {
  const data = asRecord(step.data);
  const progress = asRecord(data.progress);
  const total = Math.max(
    0,
    num(progress.total, num(data.goal_count, num(data.total, 0))),
  );
  if (total <= 0) return null;
  const done = Math.max(
    0,
    num(
      progress.settled,
      num(
        data.settled_goal_count,
        num(progress.completed, num(data.completed_goal_count, 0)),
      ),
    ),
  );
  return {
    done: Math.min(done, total),
    total,
  };
}

function latestTaskProgressFromSteps(
  steps: JsonRecord[],
): TaskProgressState | null {
  for (let idx = steps.length - 1; idx >= 0; idx -= 1) {
    const progress = taskProgressFromActivityStep(steps[idx]);
    if (progress) return progress;
  }
  return null;
}

type PlanConfirmationStage =
  | "awaiting_confirmation"
  | "running"
  | "completed"
  | "failed"
  | "interrupted";

type PlanConfirmationStepDraft = ExecutionPlanItem & {
  draft_id: string;
  enabled: boolean;
};

type PlanConfirmationDraft = {
  summary: string;
  steps: PlanConfirmationStepDraft[];
};

type PlanConfirmationState = {
  stage: PlanConfirmationStage;
  taskId: string | null;
  source: string;
  originalPlan: ExecutionPlanState | null;
  draft: PlanConfirmationDraft | null;
  editing: boolean;
  messageId: string | null;
};

const PLAN_CONFIRMATION_SOURCE_DEEP_RESEARCH = "deep_research";

function isDeepResearchPlanSource(source: string | null | undefined): boolean {
  return (
    str(source, "").trim().toLowerCase() ===
    PLAN_CONFIRMATION_SOURCE_DEEP_RESEARCH
  );
}

function planConfirmationSourceValue(
  source: string | null | undefined,
): string {
  return isDeepResearchPlanSource(source) ? "deep_research" : "execution";
}

function planConfirmationDisplayLabel(
  source: string | null | undefined,
): string {
  return isDeepResearchPlanSource(source)
    ? "Deep research plan"
    : "Execution plan";
}

function planConfirmationOutlineLabel(
  source: string | null | undefined,
): string {
  return isDeepResearchPlanSource(source)
    ? "Research outline"
    : "Execution outline";
}

type ToolProgressPresentation = {
  title: string;
  detail: string;
  streamKey?: string;
};

type TrustApprovalPreset = {
  id: string;
  label: string;
  actionKind: string;
  detailLabel: string;
  detailPlaceholder: string;
  buildPayload: (detail: string) => JsonRecord;
};

const TRUST_APPROVAL_PRESETS: TrustApprovalPreset[] = [
  {
    id: "run_terminal_command",
    label: "Run a terminal command",
    actionKind: "shell",
    detailLabel: "Command",
    detailPlaceholder: "ls -la",
    buildPayload: (detail) => ({ command: detail }),
  },
  {
    id: "read_file",
    label: "Read a file",
    actionKind: "file_read",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/report.txt",
    buildPayload: (detail) => ({ path: detail }),
  },
  {
    id: "write_file",
    label: "Create or edit a file",
    actionKind: "file_write",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/notes.txt",
    buildPayload: (detail) => ({ path: detail, operation: "write" }),
  },
  {
    id: "open_url",
    label: "Open a URL or call an API",
    actionKind: "http_get",
    detailLabel: "URL",
    detailPlaceholder: "https://api.example.com/status",
    buildPayload: (detail) => ({ url: detail }),
  },
  {
    id: "run_code",
    label: "Run generated code",
    actionKind: "code_execute",
    detailLabel: "What should the code do?",
    detailPlaceholder: "Summarize CSV rows and return totals",
    buildPayload: (detail) => ({ instruction: detail }),
  },
  {
    id: "email_action",
    label: "Read or send an email",
    actionKind: "gmail_reply",
    detailLabel: "Email task",
    detailPlaceholder: "Reply with a short status update",
    buildPayload: (detail) => ({ message: detail }),
  },
];

function normalizeExecutionPlanSubsteps(
  rawSubsteps: unknown[],
): ExecutionPlanSubstepItem[] {
  return rawSubsteps.map((value, index) => {
    const record =
      value && typeof value === "object"
        ? (value as Record<string, unknown>)
        : {};
    const id = typeof record.id === "number" ? record.id : index + 1;
    return {
      id,
      title: typeof record.title === "string" ? record.title : `Substep ${id}`,
      description:
        typeof record.description === "string" ? record.description : "",
      status: typeof record.status === "string" ? record.status : "pending",
      tool_hint: typeof record.tool_hint === "string" ? record.tool_hint : null,
    };
  });
}

function deriveExecutionPlanStepStatus(
  rawStatus: unknown,
  substeps: ExecutionPlanSubstepItem[],
): string {
  const normalized =
    str(rawStatus, "pending").trim().toLowerCase() || "pending";
  if (["completed", "failed", "skipped"].includes(normalized)) {
    return normalized;
  }
  if (substeps.length === 0) {
    return normalized;
  }

  const substepStatuses = substeps.map(
    (substep) =>
      str(substep.status, "pending").trim().toLowerCase() || "pending",
  );
  if (substepStatuses.some((status) => status === "running")) {
    return "running";
  }
  if (
    substepStatuses.every(
      (status) => status === "completed" || status === "skipped",
    )
  ) {
    return "completed";
  }
  if (
    substepStatuses.some((status) => status === "failed") &&
    !substepStatuses.some((status) => status === "pending")
  ) {
    return "failed";
  }
  if (
    substepStatuses.some((status) =>
      ["completed", "failed", "skipped"].includes(status),
    )
  ) {
    return "running";
  }
  return normalized;
}

function normalizeExecutionPlanSteps(rawSteps: unknown[]): ExecutionPlanItem[] {
  const steps = rawSteps.map((value, index) => {
    const record =
      value && typeof value === "object"
        ? (value as Record<string, unknown>)
        : {};
    const id = typeof record.id === "number" ? record.id : index + 1;
    const rawSubsteps = Array.isArray(record.substeps) ? record.substeps : [];
    const substeps = normalizeExecutionPlanSubsteps(rawSubsteps);
    return {
      id,
      title: typeof record.title === "string" ? record.title : `Step ${id}`,
      description:
        typeof record.description === "string" ? record.description : "",
      status: deriveExecutionPlanStepStatus(record.status, substeps),
      action: typeof record.action === "string" ? record.action : null,
      arguments:
        record.arguments &&
        typeof record.arguments === "object" &&
        !Array.isArray(record.arguments)
          ? (record.arguments as JsonRecord)
          : null,
      tool_hint: typeof record.tool_hint === "string" ? record.tool_hint : null,
      substeps,
    };
  });

  return steps;
}
function normalizeExecutionPlanState(
  rawPlan: unknown,
): ExecutionPlanState | null {
  const record =
    rawPlan && typeof rawPlan === "object" ? (rawPlan as JsonRecord) : {};
  const rawSteps = Array.isArray(record.steps) ? record.steps : [];
  if (rawSteps.length === 0) return null;
  return {
    plan_id: str(record.plan_id, ""),
    revision: num(record.revision, 0),
    summary: str(record.summary, ""),
    steps: normalizeExecutionPlanSteps(rawSteps),
  };
}

function resetExecutionPlanProgress(
  plan: ExecutionPlanState | null,
): ExecutionPlanState | null {
  if (!plan) return null;
  return {
    ...plan,
    steps: plan.steps.map((step) => ({
      ...step,
      status: "pending",
      substeps: step.substeps.map((substep) => ({
        ...substep,
        status: "pending",
      })),
    })),
  };
}

function isAppDeliveryPlanSubstep(substep: ExecutionPlanSubstepItem): boolean {
  return str(substep.tool_hint, "").startsWith(APP_DELIVERY_PLAN_TOOL_PREFIX);
}

function isCapabilitySetupPlanSubstep(
  substep: ExecutionPlanSubstepItem,
): boolean {
  return str(substep.tool_hint, "").startsWith(
    CAPABILITY_SETUP_PLAN_TOOL_PREFIX,
  );
}

function appDeliveryPlanSubstepPhase(
  substep: ExecutionPlanSubstepItem,
): string {
  const hint = str(substep.tool_hint, "");
  return hint.startsWith(APP_DELIVERY_PLAN_TOOL_PREFIX)
    ? hint.slice(APP_DELIVERY_PLAN_TOOL_PREFIX.length)
    : "";
}

function capabilitySetupPlanSubstepPhase(
  substep: ExecutionPlanSubstepItem,
): string {
  const hint = str(substep.tool_hint, "");
  return hint.startsWith(CAPABILITY_SETUP_PLAN_TOOL_PREFIX)
    ? hint.slice(CAPABILITY_SETUP_PLAN_TOOL_PREFIX.length)
    : "";
}

function appDeliveryPhaseMarksComplete(phaseStatus: StreamPhaseStatus): boolean {
  const phase = str(phaseStatus.phase, "").trim().toLowerCase();
  const status = str(phaseStatus.status, "").trim().toLowerCase();
  return phase === "completed" || status === "completed";
}

function updateAppDeliverySubstepForPhase(
  substep: ExecutionPlanSubstepItem,
  currentPhaseIndex: number,
  phaseStatus: StreamPhaseStatus,
): ExecutionPlanSubstepItem {
  if (!isAppDeliveryPlanSubstep(substep)) return substep;
  const substepPhase = appDeliveryPlanSubstepPhase(substep);
  const substepIndex = APP_DELIVERY_PHASE_INDEX.get(substepPhase);
  if (substepIndex == null) return substep;

  let status = "pending";
  if (appDeliveryPhaseMarksComplete(phaseStatus)) {
    status = "completed";
  } else if (substepIndex < currentPhaseIndex) {
    status = "completed";
  } else if (substepIndex === currentPhaseIndex) {
    status = phaseStatus.status === "failed" ? "failed" : "running";
  }
  return substep.status === status ? substep : { ...substep, status };
}

function updateCapabilitySetupSubstepForPhase(
  substep: ExecutionPlanSubstepItem,
  currentPhaseIndex: number,
  phaseStatus: StreamPhaseStatus,
): ExecutionPlanSubstepItem {
  if (!isCapabilitySetupPlanSubstep(substep)) return substep;
  const substepPhase = capabilitySetupPlanSubstepPhase(substep);
  const substepIndex = CAPABILITY_SETUP_PHASE_INDEX.get(substepPhase);
  if (substepIndex == null) return substep;

  let status = "pending";
  if (str(phaseStatus.status, "").trim().toLowerCase() === "completed") {
    status = "completed";
  } else if (substepIndex < currentPhaseIndex) {
    status = "completed";
  } else if (substepIndex === currentPhaseIndex) {
    status = phaseStatus.status === "failed" ? "failed" : "running";
  }
  return substep.status === status ? substep : { ...substep, status };
}

function applyAppDeliveryPhaseStatusToExecutionPlan(
  plan: ExecutionPlanState | null,
  phaseStatus: StreamPhaseStatus,
): ExecutionPlanState | null {
  if (!plan) return plan;
  const phase = str(phaseStatus.phase, "").trim().toLowerCase();
  const currentPhaseIndex = APP_DELIVERY_PHASE_INDEX.get(phase);
  if (currentPhaseIndex == null) return plan;
  const explicitAppStep =
    phaseStatus.planStepId != null
      ? plan.steps.find(
          (step) =>
            step.id === phaseStatus.planStepId &&
            step.substeps.some(isAppDeliveryPlanSubstep),
        )
      : null;

  let changed = false;
  const steps = plan.steps.map((step) => {
    const hasAppDeliverySubsteps = step.substeps.some(isAppDeliveryPlanSubstep);
    if (!hasAppDeliverySubsteps) return step;
    if (explicitAppStep && step.id !== explicitAppStep.id) return step;

    const substeps = step.substeps.map((substep) =>
      updateAppDeliverySubstepForPhase(substep, currentPhaseIndex, phaseStatus),
    );
    const status = deriveExecutionPlanStepStatus(phaseStatus.status, substeps);
    if (
      status === step.status &&
      substeps.every((substep, index) => substep === step.substeps[index])
    ) {
      return step;
    }
    changed = true;
    return {
      ...step,
      status,
      substeps,
    };
  });

  return changed ? { ...plan, steps } : plan;
}

function applyCapabilitySetupPhaseStatusToExecutionPlan(
  plan: ExecutionPlanState | null,
  phaseStatus: StreamPhaseStatus,
): ExecutionPlanState | null {
  if (!plan) return plan;
  const phase = str(phaseStatus.phase, "").trim().toLowerCase();
  const currentPhaseIndex = CAPABILITY_SETUP_PHASE_INDEX.get(phase);
  if (currentPhaseIndex == null) return plan;
  const explicitSetupStep =
    phaseStatus.planStepId != null
      ? plan.steps.find(
          (step) =>
            step.id === phaseStatus.planStepId &&
            step.substeps.some(isCapabilitySetupPlanSubstep),
        )
      : null;

  let changed = false;
  const steps = plan.steps.map((step) => {
    const hasCapabilitySetupSubsteps = step.substeps.some(
      isCapabilitySetupPlanSubstep,
    );
    if (!hasCapabilitySetupSubsteps) return step;
    if (explicitSetupStep && step.id !== explicitSetupStep.id) return step;

    const substeps = step.substeps.map((substep) =>
      updateCapabilitySetupSubstepForPhase(
        substep,
        currentPhaseIndex,
        phaseStatus,
      ),
    );
    const status = deriveExecutionPlanStepStatus(phaseStatus.status, substeps);
    if (
      status === step.status &&
      substeps.every((substep, index) => substep === step.substeps[index])
    ) {
      return step;
    }
    changed = true;
    return {
      ...step,
      status,
      substeps,
    };
  });

  return changed ? { ...plan, steps } : plan;
}

function createPlanConfirmationDraft(
  plan: ExecutionPlanState | null,
): PlanConfirmationDraft | null {
  if (!plan) return null;
  const pendingPlan = resetExecutionPlanProgress(plan) ?? plan;
  return {
    summary: pendingPlan.summary,
    steps: pendingPlan.steps.map((step, index) => ({
      ...step,
      draft_id: `${pendingPlan.plan_id || "plan"}:${index}:${step.id}`,
      enabled: true,
    })),
  };
}

function buildExecutionPlanFromDraft(
  draft: PlanConfirmationDraft | null,
  basePlan: ExecutionPlanState | null,
): ExecutionPlanState | null {
  if (!draft) return null;
  const enabledSteps = draft.steps.filter((step) => step.enabled);
  if (enabledSteps.length === 0) return null;
  return {
    plan_id: basePlan?.plan_id || "",
    revision: basePlan?.revision || 1,
    summary: draft.summary.trim() || basePlan?.summary || "",
    steps: enabledSteps.map((step, index) => ({
      id: index + 1,
      title: step.title,
      description: step.description,
      status: "pending",
      action: step.action ?? null,
      arguments: step.arguments ?? null,
      tool_hint: step.tool_hint ?? null,
      substeps: step.substeps ?? [],
    })),
  };
}

function describeExecutionPlanStep(
  step: Pick<ExecutionPlanItem, "title" | "description">,
  fallbackTitle: string,
): { title: string; description: string } {
  const rawTitle = str(step.title, "").trim() || fallbackTitle;
  const description = str(step.description, "").trim();
  return {
    title: rawTitle,
    description: description && description !== rawTitle ? description : "",
  };
}

function describeExecutionPlanSubstep(
  substep: Pick<
    ExecutionPlanSubstepItem,
    "title" | "description" | "tool_hint"
  >,
  fallbackTitle: string,
): { title: string; description: string } {
  const rawTitle = str(substep.title, "").trim() || fallbackTitle;
  const description = str(substep.description, "").trim();
  const normalizedTitle = rawTitle.toLowerCase().replace(/[_-]+/g, " ").trim();
  const normalizedToolHint = str(substep.tool_hint, "")
    .trim()
    .toLowerCase()
    .replace(/[_-]+/g, " ");
  const title =
    description && normalizedToolHint && normalizedTitle === normalizedToolHint
      ? description
      : rawTitle;
  return {
    title,
    description: description && description !== title ? description : "",
  };
}

function mergeExecutionPlanProgress(
  basePlan: ExecutionPlanState | null,
  livePlan: ExecutionPlanState | null,
): ExecutionPlanState | null {
  if (!basePlan) return livePlan;
  if (!livePlan) return basePlan;

  const stepIdentity = (
    step: Pick<ExecutionPlanItem, "title" | "tool_hint" | "description">,
  ) => {
    const title = step.title.trim().toLowerCase();
    if (title) return title;
    const description = str(step.description, "").trim().toLowerCase();
    if (description) return description;
    return str(step.tool_hint, "").trim().toLowerCase();
  };
  const matchedLiveIndexes = new Set<number>();
  const mergedStepIdentities = new Set<string>();
  const mergedSteps = basePlan.steps.map((step, index) => {
    const liveIndex = livePlan.steps.findIndex((candidate, candidateIndex) => {
      if (matchedLiveIndexes.has(candidateIndex)) return false;
      if (candidate.id === step.id) return true;
      if (
        step.tool_hint &&
        candidate.tool_hint &&
        candidate.tool_hint === step.tool_hint
      )
        return true;
      return (
        candidate.title.trim().toLowerCase() === step.title.trim().toLowerCase()
      );
    });
    const liveStep =
      liveIndex >= 0
        ? livePlan.steps[liveIndex]
        : livePlan.steps[index] || null;
    if (liveIndex >= 0) matchedLiveIndexes.add(liveIndex);
    if (!liveStep) return step;
    if (liveIndex < 0 && livePlan.steps[index]) matchedLiveIndexes.add(index);
    const mergedStep = {
      ...step,
      status: liveStep.status || step.status,
      substeps:
        liveStep.substeps.length > 0 ? liveStep.substeps : step.substeps,
    };
    mergedStepIdentities.add(stepIdentity(mergedStep));
    return mergedStep;
  });
  mergedSteps.forEach((step) => {
    mergedStepIdentities.add(stepIdentity(step));
  });

  const appendedLiveSteps = livePlan.steps
    .filter((_step, index) => !matchedLiveIndexes.has(index))
    .filter((step) => {
      const identity = stepIdentity(step);
      if (mergedStepIdentities.has(identity)) return false;
      mergedStepIdentities.add(identity);
      return true;
    })
    .map((step, index) => ({
      ...step,
      id: mergedSteps.length + index + 1,
    }));

  return {
    ...basePlan,
    revision: livePlan.revision || basePlan.revision,
    steps: [...mergedSteps, ...appendedLiveSteps],
  };
}

function executionPlanHasStarted(plan: ExecutionPlanState | null): boolean {
  if (!plan) return false;
  return plan.steps.some((step) => {
    const stepStatus = str(step.status, "pending").trim().toLowerCase();
    if (["running", "completed", "failed", "skipped"].includes(stepStatus)) {
      return true;
    }
    return step.substeps.some((substep) => {
      const substepStatus = str(substep.status, "pending").trim().toLowerCase();
      return ["running", "completed", "failed", "skipped"].includes(
        substepStatus,
      );
    });
  });
}

function activityStepIndicatesPlanExecution(step: JsonRecord): boolean {
  const stepType = str(step.step_type, "").trim().toLowerCase();
  if (stepType === "plan_step_update") {
    const status = str(step.status, "").trim().toLowerCase();
    return ["running", "completed", "failed", "skipped"].includes(status);
  }
  return (
    stepType === "tool_start" ||
    stepType === "tool_progress" ||
    stepType === "tool_result"
  );
}

function activityStepIsHeartbeatLike(step: JsonRecord): boolean {
  const stepType = str(step.step_type, str(step.type, ""))
    .trim()
    .toLowerCase();
  const icon = str(step.icon, "").trim().toLowerCase();
  const data = asRecord(step.data);
  return (
    stepType === "heartbeat" ||
    icon === "wait" ||
    toBool(step.is_heartbeat) ||
    str(data.kind, "").trim().toLowerCase() === "heartbeat"
  );
}

function shouldKeepPlanInApprovalState(
  plan: ExecutionPlanState | null,
  steps: JsonRecord[],
  pendingMode: ChatPendingRunMode,
): boolean {
  if (!plan || pendingMode === "resume") return false;
  if (executionPlanHasStarted(plan)) return false;
  const trimmedSteps = [...steps];
  while (trimmedSteps.length > 0) {
    const candidate = trimmedSteps[trimmedSteps.length - 1];
    if (!activityStepIsHeartbeatLike(candidate)) break;
    trimmedSteps.pop();
  }
  let latestPlanReadyIndex = -1;
  for (let index = trimmedSteps.length - 1; index >= 0; index -= 1) {
    if (
      str(trimmedSteps[index]?.step_type, "").trim().toLowerCase() ===
      "plan_ready_for_confirmation"
    ) {
      latestPlanReadyIndex = index;
      break;
    }
  }
  if (latestPlanReadyIndex >= 0) {
    for (
      let index = latestPlanReadyIndex + 1;
      index < trimmedSteps.length;
      index += 1
    ) {
      if (activityStepIndicatesPlanExecution(trimmedSteps[index])) {
        return false;
      }
    }
    return true;
  }
  return activityStepsHaveExecutionPlanContext(trimmedSteps);
}

function executionPlanFromStructuredValue(
  value: unknown,
): ExecutionPlanState | null {
  const direct = normalizeExecutionPlanState(value);
  if (direct) return direct;

  const record = asRecord(value);
  const preview = asRecord(record._plan_preview);
  return (
    normalizeExecutionPlanState(record.plan) ||
    normalizeExecutionPlanState(record.current_plan) ||
    normalizeExecutionPlanState(record.original_plan) ||
    normalizeExecutionPlanState(preview.current_plan) ||
    normalizeExecutionPlanState(preview.original_plan) ||
    null
  );
}

function executionPlanFromMaybeJsonValue(
  value: unknown,
): ExecutionPlanState | null {
  const structured = executionPlanFromStructuredValue(value);
  if (structured) return structured;
  if (typeof value !== "string" || !value.trim()) return null;
  try {
    return executionPlanFromStructuredValue(JSON.parse(value));
  } catch {
    return null;
  }
}

function extractExecutionPlanFromTraceSteps(
  steps: JsonRecord[],
): ExecutionPlanState | null {
  for (const step of [...steps].reverse()) {
    const structuredPlan =
      executionPlanFromStructuredValue(step.plan) ||
      executionPlanFromStructuredValue(step.plan_preview) ||
      executionPlanFromStructuredValue(step._plan_preview) ||
      executionPlanFromMaybeJsonValue(step.data);
    if (structuredPlan) return structuredPlan;

    const title =
      typeof step.title === "string" ? step.title.trim().toLowerCase() : "";
    const stepType =
      typeof step.step_type === "string"
        ? step.step_type.trim().toLowerCase()
        : "";
    if (stepType !== "plan" && title !== "execution plan") continue;

    const rawData = step.data;
    if (rawData && typeof rawData === "object") {
      const parsed =
        normalizeExecutionPlanState(rawData) ||
        normalizeExecutionPlanState(asRecord(rawData).plan);
      if (parsed) return parsed;
    }

    if (typeof rawData === "string" && rawData.trim()) {
      try {
        const parsed = JSON.parse(rawData) as unknown;
        const normalized =
          normalizeExecutionPlanState(parsed) ||
          normalizeExecutionPlanState(asRecord(parsed).plan);
        if (normalized) return normalized;
      } catch {
        // Ignore malformed trace payloads and continue scanning.
      }
    }
  }

  return null;
}

function activityStepsRepresentAwaitingPlanConfirmation(
  steps: JsonRecord[],
): boolean {
  const trimmedSteps = [...steps];
  while (trimmedSteps.length > 0) {
    const candidate = trimmedSteps[trimmedSteps.length - 1];
    if (!activityStepIsHeartbeatLike(candidate)) break;
    trimmedSteps.pop();
  }
  return shouldKeepPlanInApprovalState(
    extractExecutionPlanFromTraceSteps(trimmedSteps),
    trimmedSteps,
    "fresh",
  );
}

function extractPlanConfirmationSourceFromSteps(steps: JsonRecord[]): string {
  for (const step of [...steps].reverse()) {
    const stepType = str(step.step_type, str(step.type, ""))
      .trim()
      .toLowerCase();
    if (
      stepType !== "plan_ready_for_confirmation" &&
      stepType !== "plan_generated" &&
      stepType !== "plan_revised"
    )
      continue;
    const data = asRecord(step.data);
    const preview = asRecord(data._plan_preview);
    const source = str(
      step.source,
      str(data.source, str(preview.source, "")),
    ).trim();
    if (source) return planConfirmationSourceValue(source);
  }
  return "";
}

function activityStepsHaveExecutionPlanContext(steps: JsonRecord[]): boolean {
  return steps.some((step) => {
    const title = str(step.title, "").trim().toLowerCase();
    const stepType = str(step.step_type, "").trim().toLowerCase();
    return (
      stepType === "plan_generated" ||
      stepType === "plan_revised" ||
      stepType === "plan_ready_for_confirmation" ||
      stepType === "plan_step_update" ||
      stepType === "plan_unavailable" ||
      title === "execution plan" ||
      title === "execution plan revised" ||
      title === "execution plan unavailable"
    );
  });
}

function extractExecutionPlanFailureFromTraceSteps(
  steps: JsonRecord[],
): string {
  for (const step of [...steps].reverse()) {
    const title = str(step.title, "").trim().toLowerCase();
    const stepType = str(step.step_type, "").trim().toLowerCase();
    if (
      stepType === "plan_unavailable" ||
      title === "execution plan unavailable"
    ) {
      return str(step.detail, "");
    }
  }
  if (!activityStepsHaveExecutionPlanContext(steps)) return "";
  return "";
}

function extractLatestRunStatusSummary(
  steps: JsonRecord[],
): { status: string; detail: string } | null {
  for (const step of [...steps].reverse()) {
    const stepType = str(step.step_type, "").trim().toLowerCase();
    const title = str(step.title, "").trim();
    const payload = asRecord(step.data);
    const nested = asRecord(payload.payload);
    const userOutcome = asRecord(payload.user_outcome ?? nested.user_outcome);
    const runStatus = str(
      payload.run_status,
      str(payload.status, str(nested.status, str(payload.stage, ""))),
    )
      .trim()
      .toLowerCase();
    const requestState = str(
      userOutcome.request_state,
      str(nested.request_state, ""),
    )
      .trim()
      .toLowerCase();
    const outcomeStatus = str(
      userOutcome.status,
      str(nested.user_outcome_status, ""),
    )
      .trim()
      .toLowerCase();
    const normalizedStatus =
      outcomeStatus === "service_unavailable" ||
      requestState === "hard_service_outage"
        ? "service_unavailable"
        : requestState || runStatus || outcomeStatus;
    if (!normalizedStatus) {
      if (
        stepType !== "run_status" &&
        !title.toLowerCase().startsWith("run status:")
      ) {
        continue;
      }
    }
    const fallbackTitleStatus = title
      .split(":")
      .slice(1)
      .join(":")
      .trim()
      .toLowerCase()
      .replace(/\s+/g, "_");
    return {
      status: normalizedStatus || fallbackTitleStatus || "updated",
      detail: str(step.detail, "").trim(),
    };
  }
  return null;
}

function isTerminalDeepResearchFailureStatus(status: string): boolean {
  const normalized = (status || "").trim().toLowerCase();
  if (!normalized) return false;
  return [
    "failed",
    "platform_failed",
    "service_unavailable",
    "degraded",
    "hard_service_outage",
  ].includes(normalized);
}

function isSearchBackendSetupIssue(text: string): boolean {
  const normalized = (text || "").replace(/\s+/g, " ").trim().toLowerCase();
  if (!normalized) return false;
  const searchContext =
    /\b(search|research|sources?|backend|provider)\b/.test(normalized) ||
    normalized.includes("no usable sources");
  if (!searchContext) return false;
  return (
    normalized.includes("no search backend") ||
    (normalized.includes("search backend") &&
      normalized.includes("not configured")) ||
    normalized.includes("no usable sources") ||
    normalized.includes("all search angles failed") ||
    normalized.includes("available search backends") ||
    normalized.includes("configure serper") ||
    normalized.includes("brave search api") ||
    normalized.includes("searxng")
  );
}

export type WorkspaceView =
  | "chat"
  | "connections"
  | "channels"
  | "routing"
  | "webhooks"
  | "devices"
  | "browser"
  | "gatewayops"
  | "failover"
  | "tasks"
  | "sessions"
  | "skills"
  | "apps"
  | "goals"
  | "autonomy"
  | "evolution"
  | "arkmemory"
  | "sentinel"
  | "documents"
  | "swarm"
  | "trace"
  | "status"
  | "analytics"
  | "arkpulse"
  | "arkorbit"
  | "search"
  | "settings";

const SEARCH_API_PROVIDER_OPTIONS = [
  {
    id: "serper",
    label: "Serper",
    keyField: "search_serper_key",
    configuredField: "search_serper_configured",
    editingField: "search_serper_editing",
    clearField: "search_serper_clear",
  },
  {
    id: "brave_api",
    label: "Brave API",
    keyField: "search_brave_key",
    configuredField: "search_brave_configured",
    editingField: "search_brave_editing",
    clearField: "search_brave_clear",
  },
  {
    id: "exa",
    label: "Exa",
    keyField: "search_exa_key",
    configuredField: "search_exa_configured",
    editingField: "search_exa_editing",
    clearField: "search_exa_clear",
  },
  {
    id: "tavily",
    label: "Tavily",
    keyField: "search_tavily_key",
    configuredField: "search_tavily_configured",
    editingField: "search_tavily_editing",
    clearField: "search_tavily_clear",
  },
  {
    id: "perplexity",
    label: "Perplexity",
    keyField: "search_perplexity_key",
    configuredField: "search_perplexity_configured",
    editingField: "search_perplexity_editing",
    clearField: "search_perplexity_clear",
  },
  {
    id: "firecrawl",
    label: "Firecrawl",
    keyField: "search_firecrawl_key",
    configuredField: "search_firecrawl_configured",
    editingField: "search_firecrawl_editing",
    clearField: "search_firecrawl_clear",
  },
] as const;

const SEARCH_PROVIDER_OPTIONS = [
  ...SEARCH_API_PROVIDER_OPTIONS,
  { id: "searxng", label: "SearXNG" },
] as const;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function asRecords(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord);
}

function clarificationChoices(value: unknown): ChatClarificationChoice[] {
  return asRecords(value)
    .map<ChatClarificationChoice | null>((choice) => {
      const label = str(choice.label, "").trim();
      const submitText = str(
        choice.submit_text,
        str(choice.submitText, ""),
      ).trim();
      if (!label || !submitText) return null;
      const kind = str(choice.kind, "").trim();
      const approval = asRecord(choice.approval);
      const submitApproval = parseInternalApprovalSubmitToken(submitText);
      const effectiveKind =
        kind ||
        (submitApproval ? "direct_chat_chain_approval" : "");
      const approvalId =
        str(approval.id, "").trim() || submitApproval?.id || "";
      const approvalDecision =
        str(approval.decision, "").trim().toLowerCase() ||
        submitApproval?.decision ||
        "";
      const approvalActionName = str(
        approval.action_name,
        str(approval.actionName, ""),
      ).trim() || submitApproval?.actionName || "";
      const approvalSteps = asRecords(approval.steps)
        .map<ChatApprovalStep | null>((step) => {
          const actionName = str(
            step.action_name,
            str(step.actionName, ""),
          ).trim();
          if (!actionName) return null;
          const argumentsPreview =
            step.arguments_preview ?? step.argumentsPreview ?? undefined;
          return {
            actionName,
            ...(argumentsPreview !== undefined ? { argumentsPreview } : {}),
          };
        })
        .filter((step): step is ChatApprovalStep => step !== null);
      const isToolApprovalKind =
        effectiveKind === "direct_chat_approval" ||
        effectiveKind === "direct_chat_chain_approval";
      const parsedApproval: ChatClarificationChoice["approval"] =
        isToolApprovalKind &&
        approvalId &&
        (approvalDecision === "approve" || approvalDecision === "reject")
          ? {
              id: approvalId,
              decision: approvalDecision as "approve" | "reject",
              actionName: approvalActionName,
              ...(approvalSteps.length > 0 ? { steps: approvalSteps } : {}),
            }
          : undefined;
      return {
        label,
        submitText,
        ...(effectiveKind ? { kind: effectiveKind } : {}),
        ...(parsedApproval ? { approval: parsedApproval } : {}),
      };
    })
    .filter((choice): choice is ChatClarificationChoice => choice !== null);
}

function pickRecords(value: unknown, ...keys: string[]): JsonRecord[] {
  if (Array.isArray(value)) return asRecords(value);
  const obj = asRecord(value);
  for (const key of keys) {
    if (Array.isArray(obj[key])) return asRecords(obj[key]);
  }
  return [];
}

function str(value: unknown, fallback = "-"): string {
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return fallback;
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function streamPayloadConversationId(payload: unknown, fallback = ""): string {
  const obj = asRecord(payload);
  const nested = asRecord(obj.payload);
  return str(
    obj.conversation_id,
    str(
      obj.conversationId,
      str(
        obj.cid,
        str(
          nested.conversation_id,
          str(nested.conversationId, str(nested.cid, fallback)),
        ),
      ),
    ),
  ).trim();
}

function isSyntheticStreamTokenPayload(payload: unknown): boolean {
  return asRecord(payload).synthetic === true;
}

function streamPayloadRunId(payload: unknown, fallback = ""): string {
  const obj = asRecord(payload);
  const nested = asRecord(obj.payload);
  return str(
    obj.run_id,
    str(obj.runId, str(nested.run_id, str(nested.runId, fallback))),
  ).trim();
}

function streamPayloadContent(payload: unknown): string {
  const obj = asRecord(payload);
  const nested = asRecord(obj.payload);
  return stripAgentInternalReasoningLeaks(
    str(obj.content, str(nested.content, "")),
  ).trim();
}

function extractLatestRunAssistantContentPayload(
  latestRunPayload: unknown,
  fallbackConversationId = "",
  expectedRunId = "",
): JsonRecord | null {
  const root = asRecord(latestRunPayload);
  const run = asRecord(root.run);
  const runId = str(run.id, expectedRunId).trim();
  const conversationId = str(
    run.conversation_id,
    str(run.conversationId, fallbackConversationId),
  ).trim();
  const requiredRunId = expectedRunId.trim();
  const events = asRecords(root.events);

  for (let idx = events.length - 1; idx >= 0; idx -= 1) {
    const event = events[idx];
    const kind = str(
      event.kind,
      str(event.event, str(event.event_name, str(event.type, ""))),
    )
      .trim()
      .toLowerCase();
    if (kind !== "content") continue;
    const payload = asRecord(event.payload);
    const content = streamPayloadContent(payload);
    if (!content) continue;
    const eventRunId = streamPayloadRunId(
      payload,
      str(event.run_id, str(event.runId, runId)),
    );
    if (requiredRunId && eventRunId && eventRunId !== requiredRunId) continue;
    const eventConversationId = streamPayloadConversationId(
      payload,
      conversationId || fallbackConversationId,
    );
    return {
      ...payload,
      content,
      conversation_id: eventConversationId || conversationId || fallbackConversationId,
      run_id: eventRunId || runId || requiredRunId,
    };
  }

  const directContent = streamPayloadContent(root);
  if (!directContent) return null;
  const directRunId = streamPayloadRunId(root, runId);
  if (requiredRunId && directRunId && directRunId !== requiredRunId) {
    return null;
  }
  return {
    ...root,
    content: directContent,
    conversation_id:
      streamPayloadConversationId(root, conversationId || fallbackConversationId) ||
      conversationId ||
      fallbackConversationId,
    run_id: directRunId || runId || requiredRunId,
  };
}

function streamPayloadRunStatus(payload: unknown): string {
  const obj = asRecord(payload);
  const nested = asRecord(obj.payload);
  return str(
    obj.run_status,
    str(obj.status, str(nested.run_status, str(nested.status, ""))),
  )
    .trim()
    .toLowerCase();
}

function isTerminalChatTaskStatus(status: string): boolean {
  const normalized = (status || "").trim().toLowerCase();
  if (!normalized) return false;
  return ![
    "pending",
    "queued",
    "running",
    "in_progress",
    "paused",
    "awaiting_approval",
  ].includes(normalized);
}

function luhnValidDigits(digits: string): boolean {
  if (digits.length < 8 || !/^\d+$/.test(digits)) return false;
  let sum = 0;
  let double = false;
  for (let idx = digits.length - 1; idx >= 0; idx -= 1) {
    let value = Number(digits[idx]);
    if (double) {
      value *= 2;
      if (value > 9) value -= 9;
    }
    sum += value;
    double = !double;
  }
  return sum % 10 === 0;
}

function hasNearbyShortNumericCode(
  text: string,
  start: number,
  end: number,
): boolean {
  const proximity = 96;
  const searchStart = Math.max(0, start - proximity);
  const searchEnd = Math.min(text.length, end + proximity);
  const nearby = text.slice(searchStart, searchEnd);
  const shortCodePattern = /\b\d{3,4}\b/g;
  let match: RegExpExecArray | null;
  while ((match = shortCodePattern.exec(nearby)) !== null) {
    const codeStart = searchStart + match.index;
    const codeEnd = codeStart + match[0].length;
    if (codeEnd <= start || codeStart >= end) return true;
  }
  return false;
}

function maskRanges(
  text: string,
  ranges: Array<{ start: number; end: number }>,
  replacement: string,
): string {
  if (ranges.length === 0) return text;
  const sorted = ranges
    .filter((range) => range.end > range.start)
    .sort((left, right) => left.start - right.start || left.end - right.end);
  const merged: Array<{ start: number; end: number }> = [];
  for (const range of sorted) {
    const last = merged[merged.length - 1];
    if (last && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }
  let cursor = 0;
  let result = "";
  for (const range of merged) {
    result += text.slice(cursor, range.start);
    result += replacement;
    cursor = range.end;
  }
  return result + text.slice(cursor);
}

function maskPaymentLikeSequences(text: string): string {
  const paymentNumberPattern = /\b\d(?:[\s.-]?\d){11,18}\b/g;
  const shortCodePattern = /\b\d{3,4}\b/g;
  const paymentRanges: Array<{ start: number; end: number }> = [];
  let match: RegExpExecArray | null;
  while ((match = paymentNumberPattern.exec(text)) !== null) {
    const value = match[0];
    const digits = value.replace(/\D/g, "");
    if (digits.length < 12 || digits.length > 19) continue;
    const start = match.index;
    const end = start + value.length;
    if (luhnValidDigits(digits) || hasNearbyShortNumericCode(text, start, end)) {
      paymentRanges.push({ start, end });
    }
  }
  if (paymentRanges.length === 0) return text;
  const ranges = [...paymentRanges];
  while ((match = shortCodePattern.exec(text)) !== null) {
    const start = match.index;
    const end = start + match[0].length;
    if (
      paymentRanges.some(
        (range) => end >= range.start - 96 && start <= range.end + 96,
      )
    ) {
      ranges.push({ start, end });
    }
  }
  return maskRanges(text, ranges, "[PAYMENT_DATA]");
}

function maskSensitiveChatPreview(text: string): string {
  let result = maskPaymentLikeSequences(text);
  result = result.replace(
    /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g,
    "[EMAIL]",
  );
  result = result.replace(/\b\d{3}-\d{2}-\d{4}\b/g, "[SSN]");
  result = result.replace(
    /\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b/g,
    "[IP]",
  );
  result = result.replace(
    /(?:\+?\d{1,3}[-.\s]?)?\(?\d{2,4}\)?[-.\s]?\d{3,4}[-.\s]?\d{4}/g,
    "[PHONE]",
  );
  return result;
}

function boolText(value: unknown): string {
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "string") return value;
  if (typeof value === "number") return value === 0 ? "false" : "true";
  return "false";
}

function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return normalized === "true" || normalized === "1" || normalized === "yes";
  }
  return false;
}

function workspaceAppRootName(appDir = ""): string {
  const normalized = appDir.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  if (!normalized) return "";
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] || "";
}

const WORKSPACE_EXTENSIONLESS_FILE_NAMES = new Set([
  "cname",
  "dockerfile",
  "gemfile",
  "license",
  "makefile",
  "procfile",
  "rakefile",
  "readme",
]);

const WORKSPACE_METADATA_FIELD_NAMES = new Set([
  "app_id",
  "cid",
  "conversation_id",
  "description",
  "entry_command",
  "runtime_mode",
  "session_id",
  "slug",
  "start_command",
  "title",
  "trace_id",
]);

function isLikelyWorkspaceFileName(pathOrName: string): boolean {
  const normalized = (pathOrName || "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+/, "")
    .replace(/\/+$/, "");
  if (!normalized) return false;
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length === 0) return false;
  const base = parts[parts.length - 1].trim();
  const lowerBase = base.toLowerCase();
  if (!base || base === "." || base === "..") return false;
  if (WORKSPACE_METADATA_FIELD_NAMES.has(lowerBase)) return false;
  if (/[<>:"|?*]/.test(base)) return false;
  if (parts.length > 1) return true;
  if (base.startsWith(".")) return true;
  if (base.includes(".")) return true;
  return WORKSPACE_EXTENSIONLESS_FILE_NAMES.has(lowerBase);
}

function isLikelyWorkspaceFileContent(value: string): boolean {
  const trimmed = (value || "").trim();
  if (!trimmed) return false;
  if (isOmittedContentPlaceholder(trimmed)) return false;
  if (looksLikeTrailingMarkupFragment(trimmed)) return false;
  if (
    /^(written|saved|created|updated|deleted|moved|renamed)\b/i.test(trimmed) &&
    trimmed.split(/\r?\n/).length <= 3
  ) {
    return false;
  }
  if (
    /^(file|app)\s+(saved|written|created|updated)\b/i.test(trimmed) &&
    trimmed.length < 240
  ) {
    return false;
  }
  return true;
}

function looksLikeTrailingMarkupFragment(value: string): boolean {
  const compact = value.trim();
  if (!compact || compact.length > 120) return false;
  return /^(?:<\/[a-z][\w:-]*>\s*)+$/i.test(compact);
}

function choosePreferredWorkspaceFileContent(
  current = "",
  incoming = "",
): string {
  const currentContent = isLikelyWorkspaceFileContent(current) ? current : "";
  const incomingContent = isLikelyWorkspaceFileContent(incoming)
    ? incoming
    : "";
  if (!currentContent) return incomingContent;
  if (!incomingContent) return currentContent;
  return incomingContent.length >= currentContent.length
    ? incomingContent
    : currentContent;
}

function normalizeWorkspaceFileName(pathOrName: unknown, appDir = ""): string {
  const raw = str(pathOrName, "").trim();
  if (!raw) return "";
  let normalized = raw.replace(/\\/g, "/");
  const normalizedAppDir = appDir
    .trim()
    .replace(/\\/g, "/")
    .replace(/\/+$/, "");
  if (
    normalizedAppDir &&
    normalized.toLowerCase().startsWith(`${normalizedAppDir.toLowerCase()}/`)
  ) {
    normalized = normalized.slice(normalizedAppDir.length + 1);
  }
  normalized = normalized.replace(/^.*\/apps\/[^/]+\//i, "");
  const appRootName = workspaceAppRootName(normalizedAppDir);
  if (
    appRootName &&
    normalized.toLowerCase().startsWith(`${appRootName.toLowerCase()}/`)
  ) {
    normalized = normalized.slice(appRootName.length + 1);
  }
  normalized = normalized.replace(/^\/+/, "");
  return normalized || raw;
}

function workspaceFileDisplayPath(
  pathOrName: string,
  appId = "",
  appDir = "",
): string {
  const normalized = normalizeWorkspaceFileName(pathOrName, appDir);
  if (!normalized) return "";
  const root = appId.trim()
    ? `apps/${appId.trim()}`
    : workspaceAppRootName(appDir)
      ? `workspace/${workspaceAppRootName(appDir)}`
      : "workspace";
  const rootLower = root.toLowerCase();
  const normalizedLower = normalized.toLowerCase();
  if (
    normalizedLower === rootLower ||
    normalizedLower.startsWith(`${rootLower}/`)
  ) {
    return normalized;
  }
  return `${root}/${normalized}`.replace(/\/+/g, "/");
}

function progressFileTargetPath(
  payload: JsonRecord,
  appDir: string,
  fileName: string,
): string {
  const direct = str(
    payload.target_path,
    str(payload.absolute_path, str(payload.full_path, "")),
  ).trim();
  if (direct) return direct;
  const appRoot = (appDir || "").trim().replace(/[\\/]+$/, "");
  if (appRoot && fileName) return `${appRoot}/${fileName}`.replace(/\\/g, "/");
  return fileName;
}

function progressLineLabel(lineNo: number, totalLines: number): string {
  if (totalLines > 0) {
    return `Line ${Math.min(lineNo, totalLines)}/${totalLines}`;
  }
  if (lineNo > 0) return `Line ${lineNo}`;
  return "";
}

function mergeWorkspaceFiles(
  current: WorkspaceFileEntry[],
  incoming: WorkspaceFileEntry[],
  appDir = "",
): WorkspaceFileEntry[] {
  const merged = new Map<string, WorkspaceFileEntry>();
  for (const file of [...current, ...incoming]) {
    const name = normalizeWorkspaceFileName(file.name, appDir);
    if (!name || !isLikelyWorkspaceFileName(name)) continue;
    const existing = merged.get(name);
    if (!existing) {
      merged.set(name, {
        name,
        content: choosePreferredWorkspaceFileContent("", file.content),
      });
      continue;
    }
    merged.set(name, {
      name,
      content: choosePreferredWorkspaceFileContent(
        existing.content,
        file.content,
      ),
    });
  }
  return compactWorkspaceFilesForUi(Array.from(merged.values()));
}

function compactWorkspaceFilesForUi(
  files: WorkspaceFileEntry[],
): WorkspaceFileEntry[] {
  const out: WorkspaceFileEntry[] = [];
  let totalChars = 0;
  for (const file of files) {
    const name = str(file.name, "").trim();
    if (!name) continue;
    if (out.length >= CHAT_WORKSPACE_UI_MAX_FILES) break;
    const content = choosePreferredWorkspaceFileContent("", file.content);
    const remaining = Math.max(
      0,
      CHAT_WORKSPACE_UI_MAX_TOTAL_CHARS - totalChars,
    );
    const compacted =
      remaining > 0
        ? compactWorkspacePreviewContent(
            content,
            Math.min(CHAT_WORKSPACE_UI_MAX_FILE_CHARS, remaining),
          )
        : "";
    totalChars += compacted.length;
    out.push({ name, content: compacted });
  }
  return out;
}

function extractWorkspaceAppFromStreamPayload(
  name: string,
  payload: unknown,
): JsonRecord | null {
  const obj = asRecord(payload);
  const source = name === "app_inspect" ? asRecord(obj.matched_app) : obj;
  const appId = str(source.app_id, str(source.id, "")).trim();
  const appDir = str(source.app_dir, "").trim();
  const url = str(source.local_url, str(source.url, "")).trim();
  const accessUrl = str(
    source.local_access_url,
    str(source.access_url, ""),
  ).trim();
  if (!appId && !appDir && !accessUrl) return null;
  return {
    id: appId,
    app_id: appId,
    title: str(source.title, "App"),
    url: str(source.url, url),
    access_url: str(source.access_url, accessUrl),
    local_url: url,
    local_access_url: accessUrl,
    app_dir: appDir,
    enabled: source.enabled ?? true,
    running: source.running ?? true,
    is_static: source.is_static ?? true,
    expose_public: source.expose_public ?? false,
    runtime_mode: str(
      source.runtime_mode,
      toBool(source.is_static) ? "static" : "unknown",
    ),
  };
}

function extractWorkspaceFilesFromStreamPayload(
  name: string,
  payload: unknown,
): WorkspaceFileEntry[] {
  const obj = asRecord(payload);
  const source = name === "app_inspect" ? asRecord(obj.matched_app) : obj;
  const appDir = str(source.app_dir, str(obj.app_dir, "")).trim();
  const filesValue = source.files ?? obj.files;
  const filePreviews = {
    ...asRecord(obj.file_previews),
    ...asRecord(source.file_previews),
  };
  const previewContentForPath = (path: unknown): string => {
    const target = normalizeWorkspaceFileName(path, appDir);
    if (!target) return "";
    for (const [previewPath, previewContent] of Object.entries(filePreviews)) {
      if (typeof previewContent !== "string") continue;
      const normalizedPreviewPath = normalizeWorkspaceFileName(
        previewPath,
        appDir,
      );
      if (
        normalizedPreviewPath === target ||
        normalizedPreviewPath.endsWith(`/${target}`) ||
        target.endsWith(`/${normalizedPreviewPath}`)
      ) {
        return previewContent;
      }
    }
    return "";
  };
  if (Array.isArray(filesValue)) {
    return filesValue
      .map((row) => {
        const entry = asRecord(row);
        const rawPath = entry.path ?? entry.file ?? entry.name;
        const content = choosePreferredWorkspaceFileContent(
          str(
            entry.content,
            str(entry.text, str(entry.body, str(entry.file_content, ""))),
          ),
          choosePreferredWorkspaceFileContent(
            str(entry.raw_content, ""),
            previewContentForPath(rawPath),
          ),
        );
        return {
          name: normalizeWorkspaceFileName(rawPath, appDir),
          content,
        };
      })
      .filter((file) => !!file.name);
  }

  const filesMap = asRecord(filesValue);
  const mappedFiles = Object.entries(filesMap)
    .filter(([, value]) => typeof value === "string")
    .map(([path, value]) => ({
      name: normalizeWorkspaceFileName(path, appDir),
      content: value as string,
    }))
    .filter((file) => !!file.name);
  if (mappedFiles.length > 0) {
    return mappedFiles;
  }

  const previewFiles = Object.entries(filePreviews)
    .filter(([, value]) => typeof value === "string")
    .map(([path, value]) => ({
      name: normalizeWorkspaceFileName(path, appDir),
      content: value as string,
    }))
    .filter((file) => !!file.name);
  if (previewFiles.length > 0) {
    return previewFiles;
  }

  const singleFilePath = source.path ?? source.file ?? obj.path ?? obj.file;
  const singleFileName = normalizeWorkspaceFileName(singleFilePath, appDir);
  if (singleFileName) {
    const structuredContent = str(
      source.content_snapshot,
      str(
        source.file_content,
        str(
          source.raw_content,
          str(
            obj.content_snapshot,
            str(obj.file_content, str(obj.raw_content, "")),
          ),
        ),
      ),
    );
    const fallbackContent = str(
      source.content_delta,
      str(source.content, str(obj.content_delta, str(obj.content, ""))),
    );
    return [
      {
        name: singleFileName,
        content: choosePreferredWorkspaceFileContent(
          structuredContent,
          fallbackContent,
        ),
      },
    ];
  }

  const fileNames = Array.isArray(source.file_names)
    ? source.file_names
    : Array.isArray(obj.file_names)
      ? obj.file_names
      : [];
  return fileNames
    .map((value) => ({
      name: normalizeWorkspaceFileName(value, appDir),
      content: "",
    }))
    .filter((file) => !!file.name);
}

type WorkspaceStateFromActivitySteps = {
  deployedFiles: WorkspaceFileEntry[];
  liveFileWrites: Record<string, LiveFileWriteState>;
  app: JsonRecord | null;
};

function mergeLiveFileWriteStates(
  current: Record<string, LiveFileWriteState>,
  incoming: Record<string, LiveFileWriteState>,
  appDir = "",
): Record<string, LiveFileWriteState> {
  const next: Record<string, LiveFileWriteState> = { ...current };
  for (const [rawName, state] of Object.entries(incoming)) {
    const name = normalizeWorkspaceFileName(rawName, appDir);
    if (!name || !isLikelyWorkspaceFileName(name)) continue;
    const existing = next[name];
    if (!existing) {
      next[name] = {
        content: choosePreferredWorkspaceFileContent("", state.content),
        line: Math.max(0, state.line),
        totalLines: Math.max(0, state.totalLines),
        done: Boolean(state.done),
      };
      continue;
    }
    const content = choosePreferredWorkspaceFileContent(
      existing.content,
      state.content,
    );
    next[name] = {
      content,
      line: Math.max(existing.line, state.line),
      totalLines: Math.max(existing.totalLines, state.totalLines),
      done: existing.done || state.done,
    };
  }
  return canonicalizeLiveFileWrites(next, appDir);
}

function liveFileWriteStateFromPayload(
  payload: JsonRecord,
  appDir: string,
  existing: LiveFileWriteState | undefined,
): { name: string; state: LiveFileWriteState } | null {
  const kind = str(payload.kind, "").trim().toLowerCase();
  const fileName = normalizeWorkspaceFileName(
    payload.file ?? payload.path,
    appDir,
  );
  if (!fileName || !isLikelyWorkspaceFileName(fileName)) return null;
  if (
    kind !== "draft_file" &&
    kind !== "file_write" &&
    !str(payload.content_snapshot, "") &&
    !str(payload.content_delta, "") &&
    !str(payload.text, "")
  ) {
    return null;
  }

  const snapshot = choosePreferredWorkspaceFileContent(
    str(payload.content_snapshot, ""),
    str(payload.file_content, str(payload.raw_content, "")),
  );
  const directContent = choosePreferredWorkspaceFileContent(
    snapshot,
    str(payload.content, ""),
  );
  const delta = str(payload.content_delta, "");
  const text = str(payload.text, "");
  const lineNo = Math.max(0, num(payload.line, 0));
  const totalLines = Math.max(0, num(payload.total_lines, 0));
  let content = existing?.content || "";
  if (directContent) {
    content = choosePreferredWorkspaceFileContent(content, directContent);
  } else if (delta) {
    content = `${content}${delta}`;
  } else if (text) {
    const currentLine = existing?.line ?? 0;
    if (!existing || lineNo >= currentLine) {
      content = `${content}${text}${text.endsWith("\n") ? "" : "\n"}`;
    }
  }
  const contentLines = content ? content.split(/\r?\n/).length : 0;
  const nextTotalLines = Math.max(existing?.totalLines ?? 0, totalLines, contentLines);
  const nextLine = Math.max(existing?.line ?? 0, lineNo, contentLines);
  return {
    name: fileName,
    state: {
      content,
      line: nextLine,
      totalLines: nextTotalLines,
      done: toBool(payload.done) || (nextTotalLines > 0 && nextLine >= nextTotalLines),
    },
  };
}

function workspacePayloadCandidates(root: JsonRecord): JsonRecord[] {
  const out: JsonRecord[] = [];
  const push = (value: JsonRecord) => {
    if (Object.keys(value).length === 0) return;
    if (out.includes(value)) return;
    out.push(value);
  };
  push(root);
  push(asRecord(root.payload));
  push(asRecord(root.arguments));
  push(asRecord(root.args));
  push(asRecord(root.matched_app));
  return out;
}

function payloadToolName(root: JsonRecord, candidate: JsonRecord): string {
  return str(
    candidate.tool_name,
    str(
      candidate.name,
      str(
        candidate.action_name,
        str(root.tool_name, str(root.name, str(root.action_name, ""))),
      ),
    ),
  ).trim();
}

function workspaceStateFromActivitySteps(
  steps: JsonRecord[],
): WorkspaceStateFromActivitySteps {
  let deployedFiles: WorkspaceFileEntry[] = [];
  let liveFileWrites: Record<string, LiveFileWriteState> = {};
  let app: JsonRecord | null = null;
  let appDir = "";

  for (const step of steps) {
    const root = activityDataRecord(step.data);
    if (Object.keys(root).length === 0) continue;
    for (const candidate of workspacePayloadCandidates(root)) {
      const toolName = payloadToolName(root, candidate);
      const capturedApp = extractWorkspaceAppFromStreamPayload(
        toolName,
        candidate,
      );
      if (capturedApp) {
        app = { ...(app || {}), ...capturedApp };
        appDir = str(capturedApp.app_dir, appDir);
      }
      const effectiveAppDir = appDir || str(app?.app_dir, "");
      const capturedFiles = extractWorkspaceFilesFromStreamPayload(
        toolName,
        candidate,
      );
      if (capturedFiles.length > 0) {
        deployedFiles = mergeWorkspaceFiles(
          deployedFiles,
          capturedFiles,
          effectiveAppDir,
        );
      }
      const liveWrite = liveFileWriteStateFromPayload(
        candidate,
        effectiveAppDir,
        liveFileWrites[
          normalizeWorkspaceFileName(candidate.file ?? candidate.path, effectiveAppDir)
        ],
      );
      if (liveWrite) {
        liveFileWrites = mergeLiveFileWriteStates(
          liveFileWrites,
          { [liveWrite.name]: liveWrite.state },
          effectiveAppDir,
        );
      }
    }
  }

  const finalAppDir = str(app?.app_dir, appDir);
  const liveFiles = Object.entries(liveFileWrites)
    .map(([name, state]) => ({ name, content: state.content }))
    .filter((file) => !!file.name);
  return {
    deployedFiles: mergeWorkspaceFiles(deployedFiles, liveFiles, finalAppDir),
    liveFileWrites: canonicalizeLiveFileWrites(liveFileWrites, finalAppDir),
    app,
  };
}

function tunnelCheckAlertSeverity(
  status: unknown,
): "success" | "info" | "warning" | "error" {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass" || normalized === "healthy" || normalized === "ok")
    return "success";
  if (normalized === "fail" || normalized === "error" || normalized === "down")
    return "error";
  if (
    normalized === "warn" ||
    normalized === "warning" ||
    normalized === "degraded"
  )
    return "warning";
  return "info";
}

function tunnelCheckChipColor(
  status: unknown,
): "success" | "info" | "warning" | "error" | "default" {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass" || normalized === "healthy" || normalized === "ok")
    return "success";
  if (normalized === "fail" || normalized === "error" || normalized === "down")
    return "error";
  if (
    normalized === "warn" ||
    normalized === "warning" ||
    normalized === "degraded"
  )
    return "warning";
  if (normalized === "info") return "info";
  return "default";
}

function tunnelCheckLabel(status: unknown): string {
  const normalized = str(status, "info").trim().toLowerCase();
  if (normalized === "pass") return "Ready";
  if (normalized === "fail") return "Needs action";
  if (normalized === "warn") return "Check";
  if (!normalized) return "Info";
  return normalized.charAt(0).toUpperCase() + normalized.slice(1);
}

type ActivityPayloadView = RunPayloadView;

type ActivityPayloadItem = RunPayloadItem;

type ActivityTimelineCard = {
  id: string;
  index: number;
  stepType: string;
  rawTitle: string;
  tone: string;
  kind: string;
  label: string;
  detail: string;
  detailFull: string;
  summary: string;
  rawDetailFull: string;
  traceJson?: string;
  payloadView: ActivityPayloadView | null;
  isHeartbeat: boolean;
  time: string;
  surface?: SurfaceDescriptor | null;
};

type ChatTranscriptActionStatus = "running" | "done" | "issue";

type ChatTranscriptActionDetail = {
  id: string;
  label: string;
  detail: string;
  status: ChatTranscriptActionStatus;
  card: ActivityTimelineCard;
};

type ChatTranscriptCommandAudit = {
  commandLabel: string;
  command: string;
  outputLabel: string;
  output: string;
};

type ChatTranscriptItem =
  | {
      kind: "prose";
      id: string;
      text: string;
    }
  | {
      kind: "reasoning";
      id: string;
      title: string;
      detail: string;
      status: ChatTranscriptActionStatus;
      details: ChatTranscriptActionDetail[];
    }
  | {
      kind: "action";
      id: string;
      card: ActivityTimelineCard;
      toolName: string;
      title: string;
      detail: string;
      status: ChatTranscriptActionStatus;
      details: ChatTranscriptActionDetail[];
      count?: number;
    };

function isTranscriptOmittedPlaceholder(value: string): boolean {
  return /^\[omitted\s+[\d,]+\s+chars?(?:\s*\/\s*[\d,]+\s+lines?)?\]$/i.test(
    value.trim(),
  );
}

function normalizeTranscriptContractKey(value: string): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function normalizeTranscriptFieldKey(value: string): string {
  return (value || "")
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function truncateTranscriptAuditBlock(value: string, maxChars = 2400): string {
  const trimmed = value.trim();
  if (!trimmed || isTranscriptOmittedPlaceholder(trimmed)) return "";
  if (trimmed.length <= maxChars) return trimmed;
  return `${trimmed.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function firstTranscriptString(records: JsonRecord[], keys: string[]): string {
  const normalizedKeys = keys.map(normalizeTranscriptFieldKey);
  for (const record of records) {
    for (const [key, value] of Object.entries(record)) {
      if (!normalizedKeys.includes(normalizeTranscriptFieldKey(key))) continue;
      if (typeof value === "string" && value.trim()) {
        const trimmed = value.trim();
        if (isTranscriptOmittedPlaceholder(trimmed)) continue;
        return trimmed;
      }
      if (typeof value === "number" || typeof value === "boolean") return String(value);
      if (value && typeof value === "object") {
        const serialized = compactUnknown(value, 1600);
        if (serialized && !isTranscriptOmittedPlaceholder(serialized)) return serialized;
      }
    }
  }
  return "";
}

function isInternalChatTranscriptStep(step: JsonRecord): boolean {
  const data = activityDataRecord(step.data);
  const keys = [
    activityStepType(step),
    str(step.kind, ""),
    str(data.kind, ""),
    str(step.name, ""),
    str(data.name, ""),
    str(step.tool_name, ""),
    str(data.tool_name, ""),
    str(step.title, ""),
    str(data.title, ""),
    str(step.label, ""),
    str(data.label, ""),
  ].map(normalizeTranscriptContractKey);
  return keys.includes("inbound_precheck");
}

function transcriptCardStep(card: ActivityTimelineCard): JsonRecord {
  const parsed = tryParseActivityJson(card.traceJson || "");
  return asRecord(parsed);
}

function transcriptCardPayloadRecords(card: ActivityTimelineCard): JsonRecord[] {
  const step = transcriptCardStep(card);
  const data = activityDataRecord(step.data);
  const nestedPayload = asRecord(data.payload);
  const nestedResult = asRecord(data.result);
  return [data, nestedPayload, nestedResult, step].filter(
    (record) => Object.keys(record).length > 0,
  );
}

function transcriptCardArtifacts(card: ActivityTimelineCard): JsonRecord[] {
  return asRecords(transcriptCardStep(card).artifacts);
}

function transcriptArtifactBody(artifact: JsonRecord): string {
  const raw = artifact.data;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    return isTranscriptOmittedPlaceholder(trimmed) ? "" : trimmed;
  }
  if (raw == null) return "";
  const compacted = compactUnknown(raw, 3200);
  return isTranscriptOmittedPlaceholder(compacted) ? "" : compacted;
}

function transcriptArtifactMatches(
  artifact: JsonRecord,
  candidates: string[],
): boolean {
  const normalizedCandidates = candidates.map((candidate) =>
    candidate.trim().toLowerCase(),
  );
  const haystack = [
    str(artifact.id, ""),
    str(artifact.kind, ""),
    str(artifact.label, ""),
    str(artifact.title, ""),
  ]
    .join(" ")
    .replace(/[_-]+/g, " ")
    .toLowerCase();
  return normalizedCandidates.some((candidate) => {
    const normalized = candidate.replace(/[_-]+/g, " ");
    return haystack === normalized || haystack.includes(normalized);
  });
}

function transcriptArtifactBodies(
  artifacts: JsonRecord[],
  candidates: string[],
): Array<{ label: string; body: string }> {
  return artifacts
    .filter((artifact) => transcriptArtifactMatches(artifact, candidates))
    .map((artifact) => ({
      label:
        str(artifact.label, str(artifact.title, str(artifact.kind, "Artifact"))).trim() ||
        "Artifact",
      body: transcriptArtifactBody(artifact),
    }))
    .filter((entry) => Boolean(entry.body));
}

function transcriptToolNameFromCard(card: ActivityTimelineCard): string {
  const explicit = firstTranscriptString(transcriptCardPayloadRecords(card), [
    "tool_name",
    "name",
    "action",
    "action_kind",
  ]);
  if (explicit) return explicit.toLowerCase();
  const title = (card.rawTitle || card.label || "").trim();
  const match = title.match(/^(?:tool result|tool event):\s*(.+)$/i);
  return normalizeTranscriptContractKey(match?.[1] || "");
}

function transcriptCommandAuditFromCard(card: ActivityTimelineCard): ChatTranscriptCommandAudit | null {
  const records = transcriptCardPayloadRecords(card);
  const artifacts = transcriptCardArtifacts(card);
  const toolName = transcriptToolNameFromCard(card);
  const stepType = (card.stepType || "").trim().toLowerCase();
  const argumentArtifacts = transcriptArtifactBodies(artifacts, [
    "tool_arguments",
    "arguments",
    "input",
  ]);
  const argumentRecords = argumentArtifacts
    .map((entry) => asRecord(tryParseActivityJson(entry.body)))
    .filter((record) => Object.keys(record).length > 0);
  const command =
    firstTranscriptString([...records, ...argumentRecords], [
      "command",
      "cmd",
      "shell_command",
      "code",
      "script",
      "input",
      "arguments",
      "args",
      "query",
      "path",
      "file",
      "source_path",
      "source_dir",
      "url",
      "app_url",
      "app_id",
    ]) ||
    argumentArtifacts[0]?.body ||
    "";

  const stdout = firstTranscriptString(records, ["stdout"]);
  const stderr = firstTranscriptString(records, ["stderr"]);
  const streamOutputParts = [
    stdout ? `stdout:\n${stdout}` : "",
    stderr ? `stderr:\n${stderr}` : "",
  ].filter(Boolean);
  const artifactOutputParts =
    stepType === "tool_start"
      ? []
      : transcriptArtifactBodies(artifacts, [
          "tool_output",
          "output",
          "tool_error",
          "error",
        ]).map((entry) => `${entry.label}:\n${entry.body}`);
  const structuredOutput =
    stepType === "tool_start" ? "" : transcriptReturnedOutputFromCard(card, records);
  const explicitOutput =
    streamOutputParts.join("\n\n") ||
    artifactOutputParts.join("\n\n") ||
    structuredOutput ||
    (stepType === "tool_start"
      ? ""
      : firstTranscriptString(records, [
          "output",
          "output_preview",
          "result_preview",
          "content",
          "result",
          "response",
          "summary",
          "message",
          "error",
          "error_text",
        ]));
  const output =
    explicitOutput ||
    (stepType === "tool_result" ? card.rawDetailFull || card.detailFull || "" : "");

  const commandLabel =
    toolName === "file_read" || toolName === "source_read"
      ? "Path"
      : toolName.startsWith("browser")
        ? "Browser action"
        : toolName === "app_deploy"
          ? "Deploy input"
          : toolName === "code_execute"
            ? "Code"
            : "Command";
  const outputLabel =
    card.kind === "Issue" || card.tone === "tone-error" ? "Error output" : "Output";

  const cleanedCommand = truncateTranscriptAuditBlock(
    redactTranscriptSensitiveText(command),
    1600,
  );
  const cleanedOutput = truncateTranscriptAuditBlock(
    redactTranscriptSensitiveText(output),
    3200,
  );
  if (!cleanedCommand && !cleanedOutput) return null;
  return {
    commandLabel,
    command: cleanedCommand,
    outputLabel,
    output: cleanedOutput,
  };
}

function transcriptReturnedOutputFromCard(
  card: ActivityTimelineCard,
  records: JsonRecord[],
): string {
  const candidates: unknown[] = [];
  const surface = surfaceFromCard(card);
  for (const payload of surface?.output || []) {
    if (payload.json != null) candidates.push(payload.json);
    if (payload.text) candidates.push(payload.text);
    if (payload.preview) candidates.push(payload.preview);
  }

  for (const record of records) {
    candidates.push(record);
  }

  for (const candidate of candidates) {
    const rendered = transcriptReturnedValueText(candidate);
    if (rendered) return rendered;
  }
  return "";
}

function transcriptReturnedValueText(value: unknown): string {
  const returnedValue = extractTranscriptReturnedValue(value);
  if (!transcriptValueHasContent(returnedValue)) return "";
  const sanitized = sanitizeTranscriptReturnedValue(returnedValue);
  const rendered =
    typeof sanitized === "string"
      ? sanitized.trim()
      : JSON.stringify(sanitized, null, 2);
  return rendered ? truncateTranscriptAuditBlock(rendered, 3200) : "";
}

function extractTranscriptReturnedValue(value: unknown, depth = 0): unknown {
  if (depth > 6 || value == null) return null;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed || isTranscriptOmittedPlaceholder(trimmed)) return null;
    const parsed = tryParseActivityJson(trimmed);
    if (parsed != null && parsed !== value) {
      const nested = extractTranscriptReturnedValue(parsed, depth + 1);
      if (transcriptValueHasContent(nested)) return nested;
    }
    return trimmed;
  }
  if (typeof value === "number" || typeof value === "boolean") return value;
  if (Array.isArray(value)) {
    return value.length > 0 ? value : null;
  }

  const record = asRecord(value);
  if (Object.keys(record).length === 0) return null;

  let visibleEntries = Object.entries(record).filter(
    ([key, entryValue]) =>
      !transcriptTransportMetadataFieldKey(key) &&
      !transcriptSensitiveFieldKey(key) &&
      transcriptValueHasContent(entryValue),
  );
  if (visibleEntries.length === 0) {
    visibleEntries = Object.entries(record).filter(
      ([key, entryValue]) =>
        !transcriptSensitiveFieldKey(key) && transcriptValueHasContent(entryValue),
    );
  }
  if (visibleEntries.length === 0) return null;

  const structuredEntries = visibleEntries.filter(([, entryValue]) => {
    if (Array.isArray(entryValue)) return entryValue.length > 0;
    const nested = asRecord(entryValue);
    return Object.keys(nested).length > 0;
  });
  if (structuredEntries.length === 1) {
    const nested = extractTranscriptReturnedValue(
      structuredEntries[0][1],
      depth + 1,
    );
    return transcriptValueHasContent(nested) ? nested : structuredEntries[0][1];
  }
  if (structuredEntries.length > 1) {
    return Object.fromEntries(structuredEntries);
  }
  return visibleEntries.length === 1
    ? visibleEntries[0][1]
    : Object.fromEntries(visibleEntries);
}

function transcriptValueHasContent(value: unknown): boolean {
  if (value == null) return false;
  if (typeof value === "string") {
    const trimmed = value.trim();
    return Boolean(trimmed && !isTranscriptOmittedPlaceholder(trimmed));
  }
  if (typeof value === "number" || typeof value === "boolean") return true;
  if (Array.isArray(value)) return value.length > 0;
  return Object.keys(asRecord(value)).length > 0;
}

function sanitizeTranscriptReturnedValue(
  value: unknown,
  key = "",
  depth = 0,
): unknown {
  if (transcriptSensitiveFieldKey(key)) return "[REDACTED]";
  if (value == null || typeof value === "number" || typeof value === "boolean") {
    return value;
  }
  if (typeof value === "string") {
    return redactTranscriptSensitiveText(value.trim());
  }
  if (Array.isArray(value)) {
    return value
      .slice(0, 25)
      .map((entry) => sanitizeTranscriptReturnedValue(entry, "", depth + 1));
  }
  if (depth >= 6) return "[TRUNCATED]";

  const out: JsonRecord = {};
  for (const [entryKey, entryValue] of Object.entries(asRecord(value))) {
    if (entryKey.startsWith("_")) continue;
    if (!transcriptValueHasContent(entryValue)) continue;
    out[entryKey] = sanitizeTranscriptReturnedValue(
      entryValue,
      entryKey,
      depth + 1,
    );
  }
  return out;
}

function transcriptSensitiveFieldKey(key: string): boolean {
  const normalized = normalizeTranscriptFieldKey(key);
  return (
    ACTIVITY_PAYLOAD_SECRET_KEY_PATTERN.test(key) ||
    /(?:^|_)(?:authorization|auth|cookie|cookies|set_cookie|session|credential|credentials|secret|token|password|passcode|private_key|api_key|apikey|refresh_token)(?:_|$)/i.test(
      normalized,
    )
  );
}

function transcriptTransportMetadataFieldKey(key: string): boolean {
  const normalized = normalizeTranscriptFieldKey(key);
  if (!normalized || normalized.startsWith("_")) return true;
  return /(?:^|_)(?:activity|call|cid|conversation|display|id|kind|label|name|ok|renderer|run|seq|sequence|state|status|stream|surface|task|time|timestamp|tool|trace|type|version)(?:_|$)/i.test(
    normalized,
  );
}

function redactTranscriptSensitiveText(value: string): string {
  let text = value;
  text = text.replace(
    /-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*?-----END [^-]*PRIVATE KEY-----/g,
    "[REDACTED PRIVATE KEY]",
  );
  text = text.replace(
    /\b(Bearer|Token)\s+[A-Za-z0-9._~+/=-]{16,}\b/gi,
    "$1 [REDACTED]",
  );
  text = text.replace(
    /\b(api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password|secret)\s*[:=]\s*["']?[^"'\s,;]{6,}/gi,
    "$1=[REDACTED]",
  );
  text = text.replace(
    /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi,
    "[EMAIL]",
  );
  text = text.replace(/\b\d{3}-\d{2}-\d{4}\b/g, "[SSN]");
  text = text.replace(
    /\b(?:\+?\d[\d .()-]{7,}\d)\b/g,
    (match) => (match.replace(/\D/g, "").length >= 10 ? "[PHONE]" : match),
  );
  return text;
}

function inlineToolActivityTitle(toolName: string, card: ActivityTimelineCard): string {
  const records = transcriptCardPayloadRecords(card);
  const surface = surfaceFromCard(card);
  const displayName =
    str(surface?.tool?.displayName, "").trim() ||
    firstTranscriptString(records, ["display_name", "displayName"]) ||
    humanizeToolIdentifier(toolName || "tool");
  if (displayName) return displayName;

  const surfaceTitle = str(surface?.title, "").trim();
  if (surfaceTitle) return surfaceTitle;
  return "Tool";
}

function inlineToolActivityLabel(card: ActivityTimelineCard): string {
  return firstTranscriptString(transcriptCardPayloadRecords(card), [
    "activity_label",
    "display_label",
    "activity_title",
    "display_title",
    "label",
  ]);
}

function humanizeToolIdentifier(value: string): string {
  const cleaned = (value || "")
    .trim()
    .split(/[_\-.]+/)
    .filter((part) => part.trim())
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
  return cleaned || "Tool";
}

function inlineToolActivityDetail(
  toolName: string,
  card: ActivityTimelineCard,
  fallbackDetail: string,
): string {
  void toolName;
  const records = transcriptCardPayloadRecords(card);
  const firstValue = (keys: string[]) => firstTranscriptString(records, keys);
  const structuredLabel = inlineToolActivityLabel(card);
  if (structuredLabel) {
    const title = inlineToolActivityTitle(toolName, card);
    if (
      normalizeToolStartIntentText(structuredLabel) !==
      normalizeToolStartIntentText(title)
    ) {
      return compactTranscriptDetail(structuredLabel);
    }
  }
  const fallback = compactTranscriptDetail(fallbackDetail || card.detail || card.summary);
  if (card.stepType.trim().toLowerCase() === "tool_result" && fallback) {
    return fallback;
  }
  const structuredDetail = firstValue([
    "activity_detail",
    "display_detail",
    "summary",
    "preview",
  ]);
  if (structuredDetail) return compactTranscriptDetail(structuredDetail);

  const detail = fallback;
  if (!detail) return "";
  return detail;
}

const ACTIVITY_PAYLOAD_PREVIEW_PRIORITY = [
  "kind",
  "status",
  "tool_name",
  "name",
  "agent_name",
  "agent_role",
  "task",
  "title",
  "file",
  "path",
  "url",
  "summary",
  "error",
];

const ACTIVITY_PAYLOAD_INTERNAL_KEYS = new Set([
  "__streamKey",
  "__omitted_keys",
  "agent_id",
  "chat_visible",
  "conversation_id",
  "conversationId",
  "cid",
  "delegation_id",
  "run_id",
  "runId",
  "task_id",
  "taskId",
  "trace_id",
  "traceId",
  "time",
  "timestamp",
  "ts",
  "plan_id",
  "plan_revision",
  "plan_step_id",
  "plan_step_title",
]);

const ACTIVITY_PAYLOAD_FORCE_SHOW_KEYS = new Set([
  "kind",
  "payload",
  "result",
  "degradation",
  "response",
  "steps",
  "files",
  "content",
  "raw_content",
  "text",
  "error",
]);

const ACTIVITY_PAYLOAD_SECRET_KEY_PATTERN =
  /(?:^|[_-])(?:access_password|password|passcode|secret|token|api_key|apikey|private_key|client_secret|refresh_token)(?:$|[_-])/i;
const ACTIVITY_PAYLOAD_FILE_BODY_KEYS = new Set([
  "body",
  "content",
  "content_delta",
  "content_snapshot",
  "file_content",
  "raw_content",
  "text",
]);
const ACTIVITY_PAYLOAD_FULL_TEXT_KEYS = new Set([
  ...ACTIVITY_PAYLOAD_FILE_BODY_KEYS,
  "detail",
]);

function isReasoningActivityRecord(value: unknown): boolean {
  const record = asRecord(value);
  const data = asRecord(record.data);
  const kind = str(record.kind, str(data.kind, ""))
    .trim()
    .toLowerCase();
  const stepType = str(record.step_type, str(record.type, ""))
    .trim()
    .toLowerCase();
  if (kind !== "reasoning_delta" && stepType !== "reasoning_delta") {
    return false;
  }
  return isVisibleReasoningPhase(str(record.phase, str(data.phase, "")));
}

function shouldPreserveFullActivityPayloadString(
  normalizedKey: string,
  parent: unknown,
): boolean {
  return (
    ACTIVITY_PAYLOAD_FULL_TEXT_KEYS.has(normalizedKey) &&
    isReasoningActivityRecord(parent)
  );
}

function isStreamLikeActivityRecord(value: unknown): boolean {
  const record = asRecord(value);
  const kind = str(record.kind, "").trim().toLowerCase();
  const stepType = str(record.step_type, str(record.type, ""))
    .trim()
    .toLowerCase();
  if (
    kind === "console_chunk" ||
    kind === "reasoning_delta" ||
    stepType === "reasoning_delta" ||
    kind === "argument_stream"
  ) {
    return true;
  }
  if (str(record.stream, "").trim()) return true;
  const streamKey = str(record.stream_key, str(record.__streamKey, ""))
    .trim()
    .toLowerCase();
  return streamKey.startsWith("console:");
}

function shouldOmitActivityPayloadString(
  normalizedKey: string,
  parent: unknown,
): boolean {
  if (!ACTIVITY_PAYLOAD_FILE_BODY_KEYS.has(normalizedKey)) return false;
  if (normalizedKey === "text") return false;
  return !isStreamLikeActivityRecord(parent);
}

function formatActivityToolName(name: string): string {
  const normalized = (name || "").trim().toLowerCase();
  if (!normalized) return "Tool";
  const direct: Record<string, string> = {
    app_deploy: "Deploy App",
    build_check: "Build check",
    code_execute: "Execute Code",
    shell: "Execute Terminal",
    run_tests: "Test run",
    lint_check: "Lint check",
    file_read: "Read",
    file_search: "Search files",
    file_write: "Write",
    file_patch: "Edit",
    file_delete: "Delete file",
    skill_manage: "Skill",
    source_read: "Read",
    source_write: "Write",
    source_edit: "Edit",
    source_list: "List",
    source_search: "Search",
    frontend_build: "Frontend build",
    schedule_task: "Schedule task",
    browser_auto: "Browser",
    browser_navigate: "Browser Navigate",
    browser_click: "Browser Click",
    browser_type: "Browser Type",
    browser_scroll: "Browser Scroll",
    browser_snapshot: "Browser Snapshot",
    browser_screenshot: "Browser Screenshot",
    browser_console: "Browser Console",
    browse: "Open web page",
    web_search: "Web search",
    agent_turn_loop: "Agent workflow",
  };
  if (direct[normalized]) return direct[normalized];
  return normalized
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (ch) => ch.toUpperCase());
}

function isStandaloneActivityStatusLabel(label: string): boolean {
  const normalized = (label || "").trim().replace(/\s+/g, " ");
  if (!normalized) return false;
  const words = normalized.split(" ");
  if (words.length > 3) return false;
  if (/\bin progress$/i.test(normalized)) return true;
  return words.every((word) => /ing$/i.test(word));
}

function runningActivityTitleForToolName(name: string): string {
  const toolLabel = formatActivityToolName(name || "tool");
  if (/^running\b/i.test(toolLabel)) return toolLabel;
  return isStandaloneActivityStatusLabel(toolLabel)
    ? toolLabel
    : `Running ${toolLabel}`;
}

function startingActivitySentenceForToolName(name: string): string {
  const toolLabel = formatActivityToolName(name || "tool");
  if (/^starting\b/i.test(toolLabel)) return `${toolLabel}.`;
  return isStandaloneActivityStatusLabel(toolLabel)
    ? `${toolLabel}.`
    : `Starting ${toolLabel}.`;
}

function tryParseActivityJson(raw: string): unknown | null {
  const trimmed = (raw || "").trim();
  if (!trimmed) return null;
  if (
    !(
      (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
      (trimmed.startsWith("[") && trimmed.endsWith("]"))
    )
  ) {
    return null;
  }
  try {
    return JSON.parse(trimmed);
  } catch {
    return null;
  }
}

function looksLikeStructuredActivityText(raw: string): boolean {
  const trimmed = (raw || "").trim();
  if (!trimmed) return false;
  if (!(trimmed.startsWith("{") || trimmed.startsWith("["))) return false;
  return /["}\]]\s*[:,]|^\{\s*"|^\[\s*(\{|"|\])/.test(trimmed);
}

function formatActivityPayloadValue(value: unknown): string {
  if (value == null) return "null";
  if (typeof value === "string") {
    const trimmed = value.trim().replace(/\s+/g, " ");
    if (/^\[omitted\s+\d+\s+chars?\]$/i.test(trimmed)) return "";
    if (!trimmed) return '""';
    return trimmed.length > 44
      ? `${trimmed.slice(0, 41).trimEnd()}...`
      : trimmed;
  }
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return "";
}

function summarizeActivityPayloadPreview(value: unknown): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return "Empty list";
    const first = value[0];
    const firstRecord = asRecord(first);
    if (Object.keys(firstRecord).length > 0) {
      const preview = summarizeActivityPayloadPreview(firstRecord);
      return `${value.length} item${value.length === 1 ? "" : "s"}${preview ? ` - ${preview}` : ""}`;
    }
    const firstValue = formatActivityPayloadValue(first);
    return `${value.length} item${value.length === 1 ? "" : "s"}${firstValue ? ` - ${firstValue}` : ""}`;
  }
  const record = asRecord(value);
  const keys = Object.keys(record);
  if (keys.length === 0) return formatActivityPayloadValue(value);
  const ordered = [
    ...ACTIVITY_PAYLOAD_PREVIEW_PRIORITY.filter((key) => key in record),
    ...keys.filter((key) => !ACTIVITY_PAYLOAD_PREVIEW_PRIORITY.includes(key)),
  ];
  const parts: string[] = [];
  for (const key of ordered) {
    if (parts.length >= 3) break;
    if (ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(key)) continue;
    const formatted = formatActivityPayloadValue(record[key]);
    if (!formatted) continue;
    parts.push(`${key}: ${formatted}`);
  }
  const remaining =
    keys.filter((key) => !ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(key)).length -
    parts.length;
  if (remaining > 0) {
    parts.push(`+${remaining} more`);
  }
  return parts.join(" | ");
}

function formatActivityPayloadFieldLabel(key: string): string {
  const normalized = (key || "")
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

function activityPayloadValueToCleanText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") {
    const trimmed = value.trim().replace(/\s+/g, " ");
    if (/^\[omitted\s+\d+\s+chars?\]$/i.test(trimmed)) return "";
    return trimmed;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return "None";
    const primitiveItems = value
      .slice(0, 4)
      .map((entry) => activityPayloadValueToCleanText(entry))
      .filter(Boolean);
    if (
      primitiveItems.length > 0 &&
      primitiveItems.length === Math.min(value.length, 4)
    ) {
      const suffix =
        value.length > primitiveItems.length
          ? ` +${value.length - primitiveItems.length} more`
          : "";
      return `${primitiveItems.join(", ")}${suffix}`;
    }
    return `${value.length} item${value.length === 1 ? "" : "s"}`;
  }
  const summary = summarizeActivityPayloadPreview(value);
  if (summary) return summary;
  const keys = Object.keys(asRecord(value)).filter(
    (key) => !ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(key),
  );
  return keys.length
    ? `${keys.slice(0, 4).map(formatActivityPayloadFieldLabel).join(", ")}${
        keys.length > 4 ? ` +${keys.length - 4} more` : ""
      }`
    : "";
}

function buildActivityPayloadItems(value: unknown): ActivityPayloadItem[] {
  const out: ActivityPayloadItem[] = [];
  const addItems = (source: unknown, prefix = "", depth = 0): void => {
    if (out.length >= 10) return;
    const record = asRecord(source);
    const entries = Object.entries(record).filter(
      ([key]) => !ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(key),
    );
    if (entries.length === 0) return;
    const ordered = [
      ...ACTIVITY_PAYLOAD_PREVIEW_PRIORITY
        .filter((key) => entries.some(([entryKey]) => entryKey === key))
        .map((key) => [key, record[key]] as [string, unknown]),
      ...entries.filter(
        ([key]) => !ACTIVITY_PAYLOAD_PREVIEW_PRIORITY.includes(key),
      ),
    ];
    for (const [key, entryValue] of ordered) {
      if (out.length >= 10) break;
      if (ACTIVITY_PAYLOAD_SECRET_KEY_PATTERN.test(key)) {
        out.push({
          label: formatActivityPayloadFieldLabel(
            prefix ? `${prefix} ${key}` : key,
          ),
          value: "[redacted]",
        });
        continue;
      }
      const entryRecord = asRecord(entryValue);
      const canFlatten =
        depth < 2 &&
        ["args", "arguments", "payload", "params", "input"].includes(
          key.trim().toLowerCase(),
        ) &&
        Object.keys(entryRecord).length > 0;
      if (canFlatten) {
        addItems(entryRecord, prefix, depth + 1);
        continue;
      }
      const label = formatActivityPayloadFieldLabel(
        prefix ? `${prefix} ${key}` : key,
      );
      const value = activityPayloadValueToCleanText(entryValue);
      if (!value) continue;
      out.push({ label, value: compactUiString(value, 260) });
    }
  };

  if (Array.isArray(value)) {
    value.slice(0, 6).forEach((entry, index) => {
      const text = activityPayloadValueToCleanText(entry);
      if (text) {
        out.push({
          label: `Item ${index + 1}`,
          value: compactUiString(text, 260),
        });
      }
    });
  } else {
    addItems(value);
  }
  return out;
}

function shouldTreatAsRawActivityText(text: string): boolean {
  const trimmed = (text || "").trim();
  if (!trimmed) return false;
  if (tryParseActivityJson(trimmed) != null) return true;
  if (looksLikeHtmlPayload(trimmed) || looksLikeSourcePayload(trimmed))
    return true;
  if (trimmed.length >= 220) return true;
  return (
    trimmed.length >= 140 &&
    (/[\r\n]/.test(trimmed) || /[{}[\]<>;]/.test(trimmed))
  );
}

function buildActivityPayloadView(value: unknown): ActivityPayloadView | null {
  if (value == null) return null;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const parsed = tryParseActivityJson(trimmed);
    if (parsed != null) {
      return buildActivityPayloadView(parsed);
    }
    if (!shouldTreatAsRawActivityText(trimmed)) return null;
    return {
      kind: "text",
      badgeLabel: "Output",
      headerLabel: "Detailed output",
      preview: summarizeActivityPayloadPreview(trimmed),
      body: trimmed,
      lineCount: trimmed.split(/\r?\n/).length,
      items: [],
    };
  }

  const asArray = Array.isArray(value) ? value : null;
  const asObject = asRecord(value);
  const hasObjectContent = Object.keys(asObject).length > 0;
  if (!asArray && !hasObjectContent) return null;

  const body = JSON.stringify(value, null, 2);
  if (!body) return null;
  const items = buildActivityPayloadItems(value);
  const keys = hasObjectContent ? Object.keys(asObject) : [];
  const visibleKeys = keys.filter(
    (key) => !ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(key),
  );
  const hasNested =
    (asArray?.length ?? 0) > 0 ||
    visibleKeys.some((key) => {
      const entry = asObject[key];
      return Array.isArray(entry) || (!!entry && typeof entry === "object");
    });
  const shouldShow =
    Boolean(asArray) ||
    hasNested ||
    visibleKeys.length > 4 ||
    body.length > 220 ||
    keys.some((key) => ACTIVITY_PAYLOAD_FORCE_SHOW_KEYS.has(key));
  if (!shouldShow) return null;

  return {
    kind: "json",
    badgeLabel: "Details",
    headerLabel: items.length > 0 ? "Action details" : "Additional details",
    preview: summarizeActivityPayloadPreview(value),
    body,
    lineCount: body.split(/\r?\n/).length,
    items,
  };
}

function buildActivityPayloadViewFromSources(
  ...values: unknown[]
): ActivityPayloadView | null {
  return buildRunPayloadViewFromSources(...values);
}

function compactUnknown(value: unknown, maxLen = 2200): string {
  if (value == null) return "";
  if (typeof value === "string") return value.trim().slice(0, maxLen);
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  try {
    const serialized = JSON.stringify(value, null, 2);
    if (!serialized) return "";
    if (serialized.length <= maxLen) return serialized;
    return `${serialized.slice(0, maxLen)}...`;
  } catch {
    return "";
  }
}

function fullTraceJson(
  value: unknown,
  maxLen = CHAT_ACTIVITY_TRACE_JSON_MAX_CHARS,
): string {
  return compactUnknown(value, maxLen);
}

function extractStepDetailText(step: JsonRecord, maxLen = 2200): string {
  const detail = str(step.detail, "").trim();
  if (detail) return detail.slice(0, maxLen);
  const dataText = compactUnknown(step.data, maxLen);
  if (dataText) return dataText;
  const titleData = compactUnknown(step.title, maxLen);
  return titleData;
}

function normalizeHeartbeatDetailText(detail: string): string {
  let text = (detail || "").trim();
  text = text.replace(/^still working:\s*/i, "");
  text = text.replace(/\bno new output yet\b\.?/gi, "");
  text = text.replace(/\(\s*\d+\s*s\s+idle\s*\)/gi, "");
  text = text.replace(/\s+\./g, ".");
  text = text.replace(/([.!?])\s*[.!?]+/g, "$1");
  text = text.replace(/\s+/g, " ").trim();
  if (!text) return "Working on the current step.";
  if (!/[.!?]$/.test(text)) text += ".";
  return text;
}

function looksLikeHtmlPayload(text: string): boolean {
  const trimmed = text.trim();
  return (
    /^<!doctype html/i.test(trimmed) ||
    /^<html\b/i.test(trimmed) ||
    (/<(html|head|body|title|div|script|main)\b/i.test(trimmed) &&
      /<\/(html|body|div|script|main)>/i.test(trimmed))
  );
}

function looksLikeSourcePayload(text: string): boolean {
  const sample = text.trim().split(/\r?\n/).slice(0, 10).join("\n");
  if (!sample) return false;
  return (
    /^(from\s+\w+\s+import|import\s+[\w.{},* ]+|def\s+\w+\(|class\s+\w+|async\s+def\s+\w+\()/m.test(
      sample,
    ) ||
    /^(const|let|var|function|export|import)\s/m.test(sample) ||
    /^\s*#include\s+[<"]/m.test(sample) ||
    /^package\s+[\w.]+;$/m.test(sample)
  );
}

function summarizeJsonActivityPayload(value: unknown): string {
  if (Array.isArray(value)) {
    return value.length === 0
      ? "No items were returned."
      : `Collected ${value.length} item${value.length === 1 ? "" : "s"}.`;
  }
  const obj = asRecord(value);
  const keys = Object.keys(obj);
  if (keys.length === 0) return "Received a status update.";

  const kind = str(obj.kind, "").trim().toLowerCase();
  const status = str(obj.status, "").trim();
  const summary = str(obj.summary, "").trim();
  const error = str(obj.error, "").trim();
  const toolName = str(obj.tool_name, str(obj.name, "")).trim();
  const flowKind = str(obj.flow_kind, "").trim();
  if (kind === "run_status") {
    const payload = asRecord(obj.payload);
    const userOutcome = asRecord(payload.user_outcome);
    const runStatus = humanizeMachineLabel(
      str(payload.run_status, str(payload.status, status)),
      "",
    );
    const outcomeMessage = str(userOutcome.message, "").trim();
    const totalTokens = num(payload.total_tokens, 0);
    const durationMs = num(payload.duration_ms, 0);
    const parts = [
      runStatus ? `Run ${runStatus}.` : "Run status updated.",
      outcomeMessage,
      totalTokens > 0 ? `${Math.round(totalTokens).toLocaleString()} tokens.` : "",
      durationMs > 0 ? `${Math.round(durationMs / 1000)}s elapsed.` : "",
    ].filter(Boolean);
    return parts.join(" ");
  }
  if (kind === "content") {
    return "Response content checkpoint saved.";
  }
  if (kind === "done") {
    return "Run completion checkpoint saved.";
  }
  if (flowKind && toolName && (obj.args != null || obj.arguments != null)) {
    return `Prepared ${formatActivityToolName(toolName)} input.`;
  }
  if (kind === "tool_dispatch") {
    const toolLabel = formatActivityToolName(toolName);
    const preview = summarizeActivityPayloadPreview(obj);
    return preview
      ? `Prepared ${toolLabel} input. ${preview}.`
      : `Prepared ${toolLabel} input.`;
  }
  if (kind === "phase_status") {
    const label = str(obj.label, "Working").trim() || "Working";
    const detail = str(obj.detail, "").trim();
    return detail ? `${label}. ${detail}` : `${label}.`;
  }
  if (kind === "console_chunk") {
    const stream = str(obj.stream, "console").trim() || "console";
    const stage = str(obj.stage, "").trim();
    return `${stage ? `${stage} ` : ""}${stream} output received.`;
  }
  if (kind === "argument_stream") {
    return "";
  }
  if (kind.startsWith("delegation_")) {
    const readable = readablePayloadFromValue(obj);
    if (readable) {
      return [readable.title, readable.detail]
        .filter(Boolean)
        .join(". ")
        .replace(/\.\s*\./g, ".")
        .trim();
    }
    const agentName = str(obj.agent_name, "Agent").trim() || "Agent";
    const agentRole = str(obj.agent_role, "").trim();
    const subject = agentRole ? `${agentName} / ${agentRole}` : agentName;
    if (summary) return `${subject}. ${summary}`;
    return `${subject} shared a progress update.`;
  }
  if (status && summary) {
    return `${status.charAt(0).toUpperCase()}${status.slice(1)}. ${summary}`;
  }
  if (error) {
    return summarizeActivityDetail(error) || "Returned error details.";
  }

  const apps = Array.isArray(obj.apps) ? asRecords(obj.apps) : [];
  const matchedApp = asRecord(obj.matched_app);
  if (apps.length > 0 || Object.keys(matchedApp).length > 0) {
    const matchedTitle = str(matchedApp.title, "").trim();
    if (matchedTitle) {
      return `Found ${Math.max(apps.length, 1)} app match${Math.max(apps.length, 1) === 1 ? "" : "es"} and selected ${matchedTitle}.`;
    }
    return `Found ${apps.length} app match${apps.length === 1 ? "" : "es"} and loaded the app details.`;
  }

  const title = str(obj.title, "").trim();
  const fileBytes = num(obj.file_bytes, -1);
  if (title && fileBytes >= 0) {
    return `Loaded ${title} details (${formatBytes(fileBytes)}).`;
  }

  const preview = summarizeActivityPayloadPreview(obj);
  if (preview) return `Collected details: ${preview}.`;
  const visibleKeys = keys.slice(0, 4).join(", ");
  const remaining = keys.length > 4 ? `, +${keys.length - 4} more` : "";
  return `Collected details: ${visibleKeys}${remaining}.`;
}

function summarizeActivityDetail(detail: string): string {
  const trimmed = (detail || "").trim();
  if (!trimmed) return "";

  if (
    /^still working:/i.test(trimmed) ||
    /\bno new output yet\b/i.test(trimmed)
  ) {
    return normalizeHeartbeatDetailText(trimmed);
  }

  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      return summarizeJsonActivityPayload(JSON.parse(trimmed));
    } catch {
      return compactTranscriptDetail(redactTranscriptSensitiveText(trimmed));
    }
  }

  if (looksLikeStructuredActivityText(trimmed)) {
    return compactTranscriptDetail(redactTranscriptSensitiveText(trimmed));
  }

  if (looksLikeHtmlPayload(trimmed)) {
    const titleMatch = trimmed.match(/<title[^>]*>([^<]+)<\/title>/i);
    const title = titleMatch?.[1]?.trim();
    return title
      ? `Reviewed HTML document: ${title}.`
      : "Reviewed HTML document.";
  }

  if (looksLikeSourcePayload(trimmed)) {
    const lineCount = trimmed.split(/\r?\n/).length;
    return `Reviewed source contents (${lineCount} line${lineCount === 1 ? "" : "s"}).`;
  }

  if (trimmed.length > 240 && /[{}[\]<>;]/.test(trimmed)) {
    return "Received detailed output.";
  }

  return trimmed;
}

function providerStreamIssueDetailFromStep(step: JsonRecord): string {
  const data = asRecord(step.data);
  if (str(data.kind, "").trim() !== "provider_stream_error") return "";
  const error = str(data.error, "").trim();
  const model = str(data.model, "").trim();
  const fallback = str(data.fallback, "").trim();
  const prefix = model ? `Provider stream issue on ${model}:` : "Provider stream issue:";
  const retry = fallback ? `Retrying with ${fallback}.` : "";
  return [prefix, error, retry].filter(Boolean).join(" ");
}

function interruptedRunDetailFromSteps(steps: JsonRecord[]): string {
  for (const step of [...steps].reverse()) {
    const providerDetail = providerStreamIssueDetailFromStep(step);
    if (providerDetail) return providerDetail;

    const stepType = str(step.step_type, str(step.type, ""))
      .trim()
      .toLowerCase();
    const title = str(step.title, "").trim();
    const detail = str(step.detail, "").trim();
    const data = asRecord(step.data);
    const reason = str(data.reason, str(data.error, "")).trim();
    if (stepType === "run_status" && (detail || reason || title)) {
      return detail || reason || title;
    }
  }
  return "";
}

function activityDataRecord(value: unknown): JsonRecord {
  if (isRecord(value)) return value;
  if (typeof value !== "string") return {};
  const trimmed = value.trim();
  if (!trimmed || !trimmed.startsWith("{")) return {};
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function normalizeToolStartIntentText(value: string): string {
  return (value || "")
    .toLowerCase()
    .replace(/[`"'.,:;!?()[\]{}<>/_\\-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function toolStartIntentText(payload: JsonRecord): string {
  const summary = str(payload.intent_summary, "").trim();
  const why = str(payload.why, str(payload.expected_outcome, "")).trim();
  if (!summary) return why;
  if (!why) return summary;
  const normalizedSummary = normalizeToolStartIntentText(summary);
  const normalizedWhy = normalizeToolStartIntentText(why);
  if (
    normalizedSummary &&
    normalizedWhy &&
    (normalizedSummary === normalizedWhy ||
      normalizedSummary.includes(normalizedWhy) ||
      normalizedWhy.includes(normalizedSummary))
  ) {
    return summary;
  }
  return `${summary} ${why}`;
}

function agentLoopProgressPresentation(
  payload: JsonRecord,
  fallbackDetail = "",
): ToolProgressPresentation | null {
  if (str(payload.kind, "").trim() !== "agent_loop_progress") return null;
  const phase = str(payload.phase, "").trim();
  const titleFromPayload = str(payload.title, "").trim();
  const titleByPhase: Record<string, string> = {
    context: "Preparing context",
    capability_state: "Checking capabilities",
    turn_plan: "Preparing turn plan",
    route_decision: "Routing request",
    intent_plan: "Preparing intent plan",
    action_scope: "Selecting actions",
    model_call: "Calling model",
    tool_execution: "Running actions",
    tool_result: "Processing action output",
  };
  const focus = str(payload.focus, "").trim();
  let detail =
    toolStartIntentText(payload) || fallbackDetail || str(payload.content, "").trim();
  let title = titleByPhase[phase] || titleFromPayload || "Working";
  if (phase === "model_call") {
    if (focus === "app_delivery") {
      title = "Generating app files";
      detail = "Generating the app file bundle.";
    } else if (focus === "app_inspection") {
      title = "Preparing app inspection";
    } else if (focus === "file_changes") {
      title = "Drafting file changes";
    }
  }
  return {
    title,
    detail,
    streamKey: phase ? `agent-loop:${phase}` : "agent-loop",
  };
}

function normalizeAgentProseKey(value: string): string {
  return (value || "")
    .toLowerCase()
    .replace(/[`"'.,:;!?()[\]{}<>/_\\-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function cleanAgentProseText(value: string): string {
  const text = stripAgentControlArtifacts(value || "")
    .replace(/\r\n/g, "\n")
    .trim();
  if (!text) return "";
  if (text.startsWith("{") || text.startsWith("[")) return "";
  if (/<\/?(function_calls|invoke|parameter)\b/i.test(text)) return "";
  if (/<<<AGENT_?SCOPE_?EXPAND>>>/i.test(text)) return "";
  return text.length > 900 ? `${text.slice(0, 897).trimEnd()}...` : text;
}

function redactSensitiveAssistantText(value: string): string {
  return (value || "")
    .replace(
      /("(?:access_password|password|api_key|apikey|secret|token)"\s*:\s*")([^"]*)(")/gi,
      "$1[redacted]$3",
    )
    .replace(
      /('(?:access_password|password|api_key|apikey|secret|token)'\s*:\s*')([^']*)(')/gi,
      "$1[redacted]$3",
    );
}

function stripAgentInternalReasoningLeaks(value: string): string {
  return redactSensitiveAssistantText(
    stripAgentControlArtifacts(value || "").replace(/\r\n/g, "\n").trim(),
  );
}

/**
 * A cancelled run can leave a bare status artifact as the assistant "reply":
 * the backend emits a "run cancelled" summary plus a "Chat run cancelled" error,
 * which concatenate into "run cancelled Chat run cancelled". Detect when the
 * whole message is just that artifact so we render a proper cancelled state
 * instead of dumping the raw text. Returns false for real replies that merely
 * mention cancellation.
 */
function isRunCancellationArtifact(value: string): boolean {
  const normalized = (value || "")
    .toLowerCase()
    .replace(/[^a-z\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized) return false;
  const stripped = normalized
    .replace(/chat run cancell?ed/g, " ")
    .replace(/run cancell?ed/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return stripped.length === 0;
}

/** Clean, on-palette "Run cancelled" state shown instead of the raw artifact text. */
function CancelledRunNotice({ detail }: { detail?: string }) {
  return (
    <Box
      sx={{
        display: "flex",
        alignItems: "center",
        gap: 1,
        mt: 0.5,
        px: 1.25,
        py: 1,
        borderRadius: 1.5,
        border: "1px solid rgba(255, 190, 99, 0.3)",
        background: "rgba(255, 190, 99, 0.06)",
      }}
    >
      <StopRoundedIcon
        fontSize="small"
        sx={{ color: "warning.main", flex: "0 0 auto" }}
      />
      <Box sx={{ minWidth: 0 }}>
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: "text.primary", lineHeight: 1.3 }}
        >
          Run cancelled
        </Typography>
        <Typography variant="caption" sx={{ color: "text.secondary" }}>
          {detail || "This run was stopped before completion."}
        </Typography>
      </Box>
    </Box>
  );
}

function streamedTextStartsNewSentence(text: string): boolean {
  const trimmed = text.trimStart();
  if (!trimmed) return false;
  return /^[A-Z]/.test(trimmed);
}

function streamedTextEndsSentence(text: string): boolean {
  return /[.!?]$/.test(text.trimEnd());
}

function shouldInsertStreamParagraphBoundary(
  current: string,
  incoming: string,
): boolean {
  if (!current || !incoming || /\s$/.test(current)) return false;
  const next = incoming.trimStart();
  if (next.length < 16) return false;
  return streamedTextEndsSentence(current) && streamedTextStartsNewSentence(next);
}

function streamingResponseAppendText(current: string, incoming: string): string {
  const normalized = (incoming || "").replace(/\r\n?/g, "\n");
  if (!normalized) return "";
  if (shouldInsertStreamParagraphBoundary(current, normalized)) {
    return `\n\n${normalized.trimStart()}`;
  }
  return normalized;
}

function normalizeStreamedProseParagraphs(text: string): string {
  return text
    .split(/(```[\s\S]*?```)/g)
    .map((part) =>
      part.startsWith("```")
        ? part
        : part.replace(/([.!?])(?=[A-Z][^\n]{15,})/g, "$1\n\n"),
    )
    .join("");
}

function visibleStreamingTranscriptText(value: string): string {
  const cleaned = stripAgentInternalReasoningLeaks(value);
  if (!cleaned) return "";
  return normalizeStreamedProseParagraphs(cleaned)
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function sanitizeChatMessageForUi(value: unknown): unknown {
  const message = asRecord(value);
  if (Object.keys(message).length === 0) return value;
  if (str(message.role, "").toLowerCase() !== "assistant") return message;
  const content = str(message.content, "");
  if (!content) return message;
  return {
    ...message,
    content: stripAgentInternalReasoningLeaks(content),
  };
}

function sanitizeChatMessagesPayloadForUi(payload: unknown): unknown {
  if (Array.isArray(payload)) return payload.map(sanitizeChatMessageForUi);
  const root = asRecord(payload);
  if (!Array.isArray(root.messages)) return payload;
  return {
    ...root,
    messages: root.messages.map(sanitizeChatMessageForUi),
  };
}

function modelInternalReasoningTextFromActivityStep(step: JsonRecord): string {
  void step;
  return "";
}

function modelProseTextFromActivityStep(step: JsonRecord): string {
  const data = activityDataRecord(step.data);
  const kind = str(data.kind, "").trim().toLowerCase();

  if (kind === "model_prose") {
    return cleanAgentProseText(
      str(
        data.content,
        str(data.content_snapshot, str(step.detail, "")),
      ),
    );
  }
  return "";
}

function agentProseTextFromActivityStep(step: JsonRecord): string {
  const modelProse = modelProseTextFromActivityStep(step);
  if (modelProse) return modelProse;

  const data = activityDataRecord(step.data);
  const stepType = str(step.step_type, str(step.type, ""))
    .trim()
    .toLowerCase();

  if (stepType === "tool_start") {
    return cleanAgentProseText(toolStartIntentText(data));
  }

  return "";
}

function agentProseMessagesFromActivitySteps(
  steps: JsonRecord[],
  maxItems = 4,
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const step of steps) {
    const text = agentProseTextFromActivityStep(step);
    if (!text) continue;
    const key = normalizeAgentProseKey(text);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push(text);
  }
  return out.slice(-Math.max(1, maxItems));
}

function activityStepType(step: JsonRecord): string {
  return str(step.step_type, str(step.type, "")).trim().toLowerCase();
}

function activityToolNameFromStep(step: JsonRecord): string {
  const data = activityDataRecord(step.data);
  const direct = str(data.tool_name, str(data.name, str(step.tool_name, ""))).trim();
  if (direct) return direct;
  const title = str(step.title, "").trim();
  const match = title.match(/^(?:tool result|tool event):\s*(.+)$/i);
  return normalizeTranscriptContractKey(match?.[1] || "");
}

function transcriptStatusFromCard(
  card: ActivityTimelineCard,
): ChatTranscriptActionStatus {
  const records = transcriptCardPayloadRecords(card);
  if (
    records.some((record) => {
      if (record.ok === false || record.success === false) return true;
      const status = str(record.status, str(record.state, "")).trim().toLowerCase();
      return Boolean(status && /^(error|failed|failure|blocked|invalid)$/.test(status));
    })
  ) {
    return "issue";
  }
  const combined = `${card.kind} ${card.tone} ${card.stepType}`.toLowerCase();
  if (/issue|error|fail|blocked/.test(combined)) return "issue";
  if (/done|complete|success|result/.test(combined)) return "done";
  return "running";
}

function compactTranscriptDetail(value: string): string {
  const text = (value || "").replace(/\s+/g, " ").trim();
  if (!text) return "";
  return text.length > 120 ? `${text.slice(0, 117).trimEnd()}...` : text;
}

function agentLoopProgressPhaseFromStep(step: JsonRecord): string {
  const data = activityDataRecord(step.data);
  if (str(data.kind, "").trim() !== "agent_loop_progress") return "";
  return str(data.phase, "").trim().toLowerCase();
}

function agentLoopProgressActionNamesFromStep(step: JsonRecord): string[] {
  const data = activityDataRecord(step.data);
  const out: string[] = [];
  const add = (value: unknown) => {
    const normalized = str(value, "")
      .trim()
      .replace(/[.ã€‚]+$/g, "")
      .replace(/^["'`]+|["'`]+$/g, "");
    if (!normalized || out.includes(normalized)) return;
    out.push(normalized);
  };
  for (const key of [
    "action_name",
    "tool_name",
    "name",
    "selected_action",
    "authorized_action",
  ]) {
    add(data[key]);
  }
  for (const key of ["actions", "action_names", "tool_names"]) {
    const values = data[key];
    if (Array.isArray(values)) values.forEach(add);
  }
  const text = [
    str(data.intent_summary, ""),
    str(data.expected_outcome, ""),
    str(data.content, ""),
    str(step.detail, ""),
    str(step.title, ""),
  ].join(" ");
  const callsMatch =
    text.match(/action call\(s\):\s*([a-zA-Z0-9_,\s.-]+)/i) ||
    text.match(/authorized action\(s\):\s*([a-zA-Z0-9_,\s.-]+)/i);
  if (callsMatch?.[1]) {
    callsMatch[1]
      .split(/[.;\n]/)[0]
      .split(/,|\band\b/i)
      .map((part) => part.trim())
      .filter(Boolean)
      .forEach(add);
  }
  return out;
}

function isMainChatReasoningStep(step: JsonRecord): boolean {
  const data = activityDataRecord(step.data);
  const stepType = activityStepType(step);
  const kind = str(data.kind, "").trim().toLowerCase();
  if (kind === "reasoning_delta" || stepType === "reasoning_delta") {
    return isVisibleReasoningPhase(str(data.phase, str(step.phase, "")));
  }
  return false;
}

function normalizeReasoningPhase(raw: unknown): string {
  const phase = str(raw, "reasoning")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return phase || "reasoning";
}

function isVisibleReasoningPhase(raw: unknown): boolean {
  const phase = normalizeReasoningPhase(raw);
  return (
    phase === "model" ||
    phase === "model_summary" ||
    phase === "reasoning" ||
    phase === "reasoning_summary"
  );
}

function reasoningStatusCopy(
  rawPhase: unknown,
  emittedProgress: string,
  payload?: JsonRecord,
): ToolProgressPresentation {
  const phase = normalizeReasoningPhase(rawPhase);
  const detail = emittedProgress.trim();
  const title =
    str(payload?.title, "").trim() ||
    str(payload?.label, "").trim() ||
    (phase === "model" || phase === "reasoning"
      ? "Thinking"
      : phase === "model_summary" || phase === "reasoning_summary"
        ? "Reasoning summary"
        : formatActivityToolName(phase));
  return {
    title,
    detail,
    streamKey: str(payload?.stream_key, `reasoning:${phase}`),
  };
}

function isReasoningProgressPayload(name: string, payloadObj: JsonRecord): boolean {
  return (
    str(payloadObj.kind, "").trim() === "reasoning_delta" ||
    str(name, "").trim().toLowerCase() === "reasoning"
  );
}

function buildToolProgressPresentation(
  name: string,
  content: string,
  payload: unknown,
  appDir = "",
): ToolProgressPresentation {
  const preview = (content || "").trim().slice(0, 1600);
  const detail = summarizeActivityDetail(preview);
  const payloadObj = asRecord(payload);
  const delegationKind = str(payloadObj.kind, "");
  const isFileWriteProgress =
    (name === "app_deploy" && str(payloadObj.kind, "") === "file_write") ||
    name === "file_write";
  const isToolEnvelope =
    !!str(payloadObj.flow_kind, "").trim() &&
    !!str(payloadObj.name, "").trim() &&
    (payloadObj.args != null || payloadObj.arguments != null);
  const isDraftFile = str(payloadObj.kind, "") === "draft_file";
  const isPhaseStatus = str(payloadObj.kind, "") === "phase_status";
  const isConsoleChunk = str(payloadObj.kind, "") === "console_chunk";

  if (isReasoningProgressPayload(name, payloadObj)) {
    return reasoningStatusCopy(
      payloadObj.phase,
      str(payloadObj.content, str(payloadObj.content_delta, content)),
      payloadObj,
    );
  }

  const agentLoopPresentation = agentLoopProgressPresentation(
    payloadObj,
    detail || preview,
  );
  if (agentLoopPresentation) return agentLoopPresentation;

  if (isToolEnvelope) {
    const toolName = str(payloadObj.name, name || "tool");
    const toolLabel = formatActivityToolName(toolName);
    return {
      title: runningActivityTitleForToolName(toolName),
      detail: "Preparing action input.",
      streamKey: str(payloadObj.run_id, `tool-envelope:${toolLabel}`),
    };
  }

  if (str(payloadObj.kind, "") === "provider_stream_error") {
    const error = str(payloadObj.error, "").trim();
    const model = str(payloadObj.model, "").trim();
    const fallback = str(payloadObj.fallback, "").trim();
    const retry = fallback ? `Retrying with ${fallback}.` : "";
    return {
      title: "Provider stream issue",
      detail:
        [error || detail || preview, retry].filter(Boolean).join(" ") ||
        "The provider stream stalled before usable output arrived.",
      streamKey: `provider-stream-error:${model || "model"}`,
    };
  }

  if (name === "delegation" && delegationKind.startsWith("delegation_")) {
    const agentName = str(payloadObj.agent_name, "").trim();
    const agentRole = str(payloadObj.agent_role, "").trim();
    const taskSummary = str(payloadObj.task, "").trim();
    const streamKey = agentName
      ? `delegation:${str(payloadObj.delegation_id, "run")}:${str(payloadObj.agent_id, agentName)}`
      : `delegation:${str(payloadObj.delegation_id, "run")}`;
    const roleLabel = agentRole
      ? `${agentName || "Agent"} / ${agentRole}`
      : agentName || "Delegation";
    if (delegationKind === "delegation_started") {
      const count = Math.max(0, num(payloadObj.agent_count, 0));
      return {
        title: "Launching agent swarm",
        detail:
          count > 0
            ? `Starting ${count} delegated agent${count === 1 ? "" : "s"}.`
            : detail || "Starting delegated work.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_assignment") {
      return {
        title: `Assigned ${roleLabel}`,
        detail: taskSummary || detail || "Prepared delegated assignment.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_agent_started") {
      return {
        title: `${roleLabel} is working`,
        detail:
          taskSummary || detail ||
          "Delegated work started.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_agent_progress") {
      return {
        title: `${roleLabel} is working`,
        detail: detail || taskSummary || "Delegated work is still running.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_agent_completed") {
      return {
        title: `${roleLabel} finished`,
        detail: detail || taskSummary || "Delegated work completed.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_agent_failed") {
      return {
        title: `${roleLabel} needs attention`,
        detail: detail || taskSummary || "Delegated work failed.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_synthesis_started") {
      return {
        title: "Synthesizing agent results",
        detail: detail || "Combining delegated results into one response.",
        streamKey,
      };
    }
    if (delegationKind === "delegation_completed") {
      return {
        title: "Agent swarm completed",
        detail: detail || "Delegated work completed.",
        streamKey,
      };
    }
  }

  if (isPhaseStatus) {
    const label = str(payloadObj.label, "").trim() || "Working";
    const phaseDetail = str(payloadObj.detail, preview).trim();
    const streamKey = str(
      payloadObj.stream_key,
      `phase-status:${name || "tool"}`,
    );
    return {
      title: label,
      detail: phaseDetail,
      streamKey,
    };
  }

  if (isDraftFile) {
    const fileName = normalizeWorkspaceFileName(
      payloadObj.file ?? payloadObj.path,
      appDir,
    );
    const snapshot = str(
      payloadObj.content_snapshot,
      str(payloadObj.content_delta, ""),
    ).trim();
    const lineNo = Math.max(
      0,
      num(payloadObj.line, snapshot ? snapshot.split(/\r?\n/).length : 0),
    );
    const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
    const lineLabel =
      totalLines > 0
        ? `Line ${Math.min(lineNo, totalLines)}/${totalLines}`
        : lineNo > 0
          ? `${lineNo} line${lineNo === 1 ? "" : "s"}`
          : "Draft ready";
    const targetPath = progressFileTargetPath(payloadObj, appDir, fileName);
    const currentLine = snapshot
      ? snapshot.split(/\r?\n/).slice(-1)[0]
      : "";
    const detailParts = [
      targetPath ? `Bundle file: ${targetPath}` : "",
      currentLine ? `${lineLabel}: ${currentLine}` : lineLabel,
    ].filter(Boolean);
    return {
      title: `Drafting ${fileName || "file"}`,
      detail: detailParts.join(" - "),
      streamKey: str(
        payloadObj.stream_key,
        fileName ? `draft-file:${fileName}` : "draft-file",
      ),
    };
  }

  if (isFileWriteProgress) {
    const fileName = normalizeWorkspaceFileName(
      payloadObj.file ?? payloadObj.path,
      appDir,
    );
    const lineNo = Math.max(0, num(payloadObj.line, 0));
    const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
    const text = str(payloadObj.text, "").trim();
    const lineLabel = progressLineLabel(lineNo, totalLines);
    const targetPath = progressFileTargetPath(payloadObj, appDir, fileName);
    const done = toBool(payloadObj.done);
    const detailParts = [
      targetPath ? `Target: ${targetPath}` : "",
      done && targetPath
        ? `Wrote ${targetPath}`
        : text && lineLabel
          ? `${lineLabel}: ${text}`
          : lineLabel || (done ? "Write complete" : "Preparing file"),
    ].filter(Boolean);
    return {
      title: `Writing ${fileName || "file"}`,
      detail: detailParts.join(" - "),
      streamKey: fileName ? `file-write:${fileName}` : "file-write",
    };
  }

  if (isConsoleChunk) {
    const stage = str(payloadObj.stage, "").trim();
    const stream = str(payloadObj.stream, "").trim();
    const text = str(payloadObj.text, content).trim();
    return {
      title: `${stage ? `${stage} ` : ""}${stream || "console"}`.trim(),
      detail: text || detail || "Console output",
      streamKey: str(
        payloadObj.stream_key,
        `console:${name || "tool"}:${stage || "stage"}:${stream || "stream"}`,
      ),
    };
  }

  return {
    title: runningActivityTitleForToolName(name || "tool"),
    detail: detail || preview,
  };
}

type SwarmChatAgent = {
  id: string;
  agentName: string;
  agentRole: string;
  modelName: string;
  task: string;
  status: string;
  summary: string;
  latestUpdate: string;
  isSpecialist: boolean;
  dependsOn: number[];
  elapsedMs?: number;
  sequence: number;
};

type SwarmChatRun = {
  id: string;
  request: string;
  status: string;
  summary: string;
  agentCount: number;
  updatedAtIndex: number;
  agents: SwarmChatAgent[];
};

function activityToneColor(kind: string, tone: string): string {
  if (kind === "Issue") return "var(--activity-tone-issue)";
  if (kind === "Done") return "var(--activity-tone-done)";
  if (kind === "Running") return "var(--activity-tone-running)";
  if (kind === "Planning") return "var(--activity-tone-planning)";
  if (tone === "tone-error") return "var(--activity-tone-issue)";
  if (tone === "tone-success") return "var(--activity-tone-done)";
  if (tone === "tone-action") return "var(--activity-tone-running)";
  if (tone === "tone-thinking") return "var(--activity-tone-planning)";
  return "var(--activity-tone-default)";
}

function activityKindDisplayLabel(kind: string): string {
  const normalized = (kind || "").trim().toLowerCase();
  if (normalized === "running") return "Working";
  if (normalized === "planning") return "Thinking";
  if (normalized === "issue") return "Attention";
  return kind || "Update";
}

function ActivityPayloadDisclosure({
  payload,
  expanded,
  onToggle,
  controlsId,
}: {
  payload: ActivityPayloadView;
  expanded: boolean;
  onToggle: () => void;
  controlsId: string;
}) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  async function handleCopyPayload() {
    if (!payload.body) return;
    try {
      await navigator.clipboard.writeText(payload.body);
      setCopied(true);
    } catch {
      // Clipboard access can be denied in insecure contexts.
    }
  }

  return (
    <Box className={`activity-payload-shell${expanded ? " is-expanded" : ""}`}>
      <Stack
        direction={{ xs: "column", sm: "row" }}
        className="activity-payload-head"
        sx={{
          alignItems: { xs: "stretch", sm: "center" },
          justifyContent: "space-between",
          gap: 0.75,
        }}
      >
        <Stack
          direction="row"
          spacing={0.75}
          className="activity-payload-preview"
          sx={{ alignItems: "center", minWidth: 0 }}
        >
          <span
            className={`activity-payload-chip activity-payload-chip-${payload.kind}`}
          >
            {payload.badgeLabel}
          </span>
          <Typography
            variant="caption"
            className="activity-payload-preview-text"
            title={payload.preview || payload.headerLabel}
          >
            {payload.preview || payload.headerLabel}
          </Typography>
        </Stack>
        <Stack
          direction="row"
          spacing={0.35}
          className="activity-payload-actions"
          sx={{ alignItems: "center" }}
        >
          <Tooltip title={copied ? "Copied" : "Copy details"} placement="top" arrow>
            <span>
              <IconButton
                size="small"
                className="activity-payload-copy"
                disabled={!payload.body}
                onClick={handleCopyPayload}
                aria-label="Copy payload details"
              >
                <ContentCopyRoundedIcon fontSize="inherit" />
              </IconButton>
            </span>
          </Tooltip>
          <Button
            size="small"
            variant="text"
            className="activity-payload-toggle"
            aria-expanded={expanded}
            aria-controls={controlsId}
            onClick={onToggle}
            endIcon={
              <ArrowDropDownRoundedIcon
                className={`activity-payload-toggle-icon${expanded ? " is-expanded" : ""}`}
              />
            }
          >
            {expanded ? "Hide raw payload" : "Raw payload"}
          </Button>
        </Stack>
      </Stack>
      {payload.items.length > 0 ? (
        <Box className="activity-payload-fields activity-payload-readable-fields">
          {payload.items.map((item, index) => (
            <Box
              key={`${controlsId}-field-${index}`}
              className="activity-payload-field"
            >
              <span className="activity-payload-field-label">
                {item.label}
              </span>
              <span className="activity-payload-field-value">
                {item.value}
              </span>
            </Box>
          ))}
        </Box>
      ) : payload.preview ? (
        <Typography variant="body2" className="activity-detail-copy activity-payload-readable-copy">
          {payload.preview}
        </Typography>
      ) : null}
      <Collapse in={expanded} mountOnEnter unmountOnExit>
        <Box id={controlsId} className="activity-payload-body activity-payload-raw-body">
          <Typography variant="caption" className="activity-payload-body-label">
            {payload.kind === "json" ? "Raw JSON payload" : "Raw text payload"}
          </Typography>
          <Box component="pre" className="activity-payload-pre">
            {payload.body}
          </Box>
        </Box>
      </Collapse>
    </Box>
  );
}

function activityPayloadViewsEqual(
  left: ActivityPayloadView | null,
  right: ActivityPayloadView | null,
): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  return (
    left.kind === right.kind &&
    left.badgeLabel === right.badgeLabel &&
    left.headerLabel === right.headerLabel &&
    left.preview === right.preview &&
    left.body === right.body &&
    left.lineCount === right.lineCount &&
    (left.items?.length ?? 0) === (right.items?.length ?? 0)
  );
}

function activityTimelineCardsRenderEqual(
  left: ActivityTimelineCard,
  right: ActivityTimelineCard,
): boolean {
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
    left.time === right.time &&
    activityPayloadViewsEqual(left.payloadView, right.payloadView)
  );
}

const ActivityTimelineRow = memo(function ActivityTimelineRow({
  row,
  isActive,
  onOpenDetails,
  detailed = false,
}: {
  row: ActivityTimelineCard;
  isActive: boolean;
  onOpenDetails?: () => void;
  detailed?: boolean;
}) {
  const lineTone = activityToneColor(row.kind, row.tone);
  const summary = row.summary || row.detailFull || row.detail || "";
  const stepTypeLabel = row.stepType.replace(/[_-]+/g, " ").trim();
  const rawTitle = row.rawTitle.trim();
  const lineDetail = summary || (rawTitle && rawTitle !== row.label ? rawTitle : "");
  return (
    <ButtonBase
      component="button"
      type="button"
      className={`term-line activity-timeline-row activity-log-line${isActive ? " term-line-active" : ""}${detailed ? " activity-timeline-row-detailed" : ""}`}
      onClick={onOpenDetails}
      aria-label={`Open activity details for ${row.label}`}
    >
      <span className="term-prompt" style={{ color: lineTone }}>
        -
      </span>
      <Box className="activity-log-main">
        <span
          className={`activity-kind-pill activity-kind-pill-${(row.kind || "update").toLowerCase()}`}
        >
          {activityKindDisplayLabel(row.kind)}
        </span>
        <span className="activity-log-label" title={row.label}>
          {row.label}
        </span>
        {lineDetail ? (
          <span className="activity-log-detail" title={lineDetail}>
            {lineDetail}
          </span>
        ) : null}
      </Box>
      <span className="activity-log-meta">
        {stepTypeLabel ? <span>{stepTypeLabel}</span> : null}
        {row.time ? <span>{formatTraceStepTime(row.time)}</span> : null}
        <span>#{row.index + 1}</span>
      </span>
    </ButtonBase>
  );
}, (prev, next) =>
  prev.isActive === next.isActive &&
  prev.detailed === next.detailed &&
  activityTimelineCardsRenderEqual(prev.row, next.row),
);

function InlineActivityCard({
  row,
  isActive,
  showPayload,
  payloadExpanded,
  onTogglePayload,
  payloadKey,
  onOpenConsole,
}: {
  row: ActivityTimelineCard;
  isActive: boolean;
  showPayload: boolean;
  payloadExpanded: boolean;
  onTogglePayload: () => void;
  payloadKey: string;
  onOpenConsole?: () => void;
}) {
  const summary = row.summary || row.detailFull || row.detail || "";
  const toneStyle = {
    "--inline-activity-tone": activityToneColor(row.kind, row.tone),
  } as CSSProperties;
  const payloadId = `inline-activity-payload-${payloadKey}`.replace(
    /[^a-zA-Z0-9_-]+/g,
    "-",
  );

  return (
    <Box
      className={`chat-inline-activity-card${isActive ? " is-active" : ""}`}
      style={toneStyle}
      role={onOpenConsole ? "button" : undefined}
      tabIndex={onOpenConsole ? 0 : undefined}
      onClick={onOpenConsole}
      onKeyDown={(event) => {
        if (!onOpenConsole) return;
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        onOpenConsole();
      }}
    >
      <Box className="chat-inline-activity-status" aria-hidden="true" />
      <Box className="chat-inline-activity-card-main">
        <Stack
          direction="row"
          spacing={0.75}
          className="chat-inline-activity-card-head"
          sx={{ alignItems: "center", minWidth: 0 }}
        >
          <Typography
            component="span"
            className="chat-inline-activity-kind"
          >
            {activityKindDisplayLabel(row.kind)}
          </Typography>
          <Typography
            component="span"
            className="chat-inline-activity-title"
            title={row.label}
          >
            {row.label}
          </Typography>
        </Stack>
        {summary ? (
          <Typography
            component="div"
            className="chat-inline-activity-detail"
            title={summary}
          >
            {summary}
          </Typography>
        ) : null}
        <Stack
          direction="row"
          spacing={0.75}
          className="chat-inline-activity-meta"
          sx={{ alignItems: "center", flexWrap: "wrap" }}
        >
          {row.time ? <span>{formatTraceStepTime(row.time)}</span> : null}
        </Stack>
        {showPayload && row.payloadView ? (
          <ActivityPayloadDisclosure
            payload={row.payloadView}
            expanded={payloadExpanded}
            onToggle={onTogglePayload}
            controlsId={payloadId}
          />
        ) : null}
      </Box>
    </Box>
  );
}

function buildInitialThinkingActivityCard(keyPrefix: string): ActivityTimelineCard {
  return {
    id: `${keyPrefix}:initial-thinking`,
    index: -1,
    stepType: "thinking",
    rawTitle: "Thinking",
    tone: "tone-thinking",
    kind: "Planning",
    label: "Thinking",
    detail: "Understanding the request and preparing the first action.",
    detailFull: "Understanding the request and preparing the first action.",
    summary: "Understanding the request and preparing the first action.",
    rawDetailFull: "",
    traceJson: "",
    payloadView: null,
    isHeartbeat: false,
    time: "",
  };
}

function activityCardIsPlanning(card: ActivityTimelineCard): boolean {
  return card.kind === "Planning" || card.tone === "tone-thinking";
}

function activityCardIsRunning(card: ActivityTimelineCard): boolean {
  return (
    card.kind === "Running" ||
    card.tone === "tone-action" ||
    card.tone === "tone-tool"
  );
}

function countPublicActivityCards(cards: ActivityTimelineCard[]): number {
  return cards.filter((card) => !card.isHeartbeat).length || cards.length;
}

function publicActivityKicker(cards: ActivityTimelineCard[], live: boolean): string {
  const count = countPublicActivityCards(cards);
  const latestMeaningful = [...cards].reverse().find((card) => !card.isHeartbeat);
  if (live || (latestMeaningful && activityCardIsRunning(latestMeaningful))) {
    return count > 1 ? "Current step" : "Starting";
  }
  return count > 1 ? "Run summary" : "Activity";
}

function withInitialThinkingActivityCard(
  cards: ActivityTimelineCard[],
  keyPrefix: string,
): ActivityTimelineCard[] {
  return cards.length > 0 ? cards : [buildInitialThinkingActivityCard(keyPrefix)];
}

function pickPublicActivityCard(
  cards: ActivityTimelineCard[],
  _live: boolean,
): ActivityTimelineCard {
  const meaningful = cards.filter((card) => !card.isHeartbeat);
  if (meaningful.length === 0) return buildInitialThinkingActivityCard("empty");
  return (
    [...meaningful].reverse().find((card) => activityCardIsRunning(card)) ||
    meaningful[meaningful.length - 1]
  );
}

function InlineActivityFeed({
  cards,
  live = false,
  expandedPayloads,
  onTogglePayload,
  keyPrefix,
  onOpenConsole,
}: {
  cards: ActivityTimelineCard[];
  live?: boolean;
  expandedPayloads: Set<string>;
  onTogglePayload: (id: string) => void;
  keyPrefix: string;
  onOpenConsole?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const displayCards = withInitialThinkingActivityCard(cards, keyPrefix);
  const activeId = live ? displayCards[displayCards.length - 1]?.id : "";
  const latestCards = displayCards.slice(-7);
  const visibleCards = displayCards.length > 8 ? latestCards : displayCards;
  const hiddenCount = Math.max(0, displayCards.length - visibleCards.length);
  const publicCard = pickPublicActivityCard(displayCards, live);
  const publicSummary =
    publicCard.summary || publicCard.detailFull || publicCard.detail || "";
  const progressKicker = publicActivityKicker(displayCards, live);

  if (cards.length === 0) return null;

  return (
    <Box
      className={`chat-inline-activity${live ? " is-live" : ""}${expanded ? " is-expanded" : " is-collapsed"}`}
    >
      <Button
        size="small"
        className={`chat-inline-activity-toggle chat-inline-run-card${live ? " is-live" : ""}`}
        role={live ? "status" : undefined}
        aria-live={live ? "polite" : undefined}
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
        endIcon={<ExpandMoreIcon className="chat-inline-activity-toggle-icon" />}
      >
        <Box className="chat-public-progress-dot" aria-hidden="true" />
        <Box className="chat-inline-run-copy">
          <span className="chat-inline-run-kicker">
            {progressKicker}
          </span>
          <Typography
            component="span"
            className="chat-public-progress-title"
            title={publicCard.label}
          >
            {publicCard.label}
          </Typography>
          {publicSummary ? (
            <Typography
              component="span"
              className="chat-public-progress-detail"
              title={publicSummary}
            >
              {publicSummary}
            </Typography>
          ) : null}
        </Box>
        <span className="chat-inline-task-toggle-action">
          {expanded ? "Hide" : "Expand"}
        </span>
      </Button>
      <Collapse in={expanded} mountOnEnter unmountOnExit>
        <Stack spacing={0.7} className="chat-inline-activity-list">
            {hiddenCount > 0 ? (
              <Typography variant="caption" className="chat-inline-activity-meta-note">
                Showing latest {visibleCards.length} of {displayCards.length}. Full details are in AgentArk Console.
              </Typography>
            ) : null}
          {visibleCards.map((row) => {
            const payloadKey = `${keyPrefix}:${row.id}`;
            return (
              <InlineActivityCard
                key={payloadKey}
                row={row}
                isActive={live && row.id === activeId}
                showPayload={false}
                payloadExpanded={expandedPayloads.has(payloadKey)}
                onTogglePayload={() => onTogglePayload(payloadKey)}
                payloadKey={payloadKey}
                onOpenConsole={onOpenConsole}
              />
            );
          })}
        </Stack>
      </Collapse>
    </Box>
  );
}

function normalizeSwarmStatus(status: unknown): string {
  const normalized = str(status, "").trim().toLowerCase();
  if (!normalized) return "running";
  if (normalized === "cancelled" || normalized === "canceled")
    return "interrupted";
  if (normalized === "timeout" || normalized === "timed out") return "timed_out";
  if (normalized === "degraded") return "partial";
  return normalized;
}

function swarmStatusChipColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  switch (normalizeSwarmStatus(status)) {
    case "completed":
    case "success":
    case "done":
      return "success";
    case "partial":
    case "running":
    case "assigned":
    case "synthesizing":
      return "warning";
    case "failed":
    case "timed_out":
    case "panicked":
    case "interrupted":
      return "error";
    default:
      return "default";
  }
}

function swarmStatusLabel(status: string): string {
  switch (normalizeSwarmStatus(status)) {
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
      return "Failed";
    case "interrupted":
      return "Stopped";
    default:
      return "Queued";
  }
}

function formatSwarmElapsedMs(value: unknown): string {
  const ms = Math.max(0, num(value, 0));
  if (!ms) return "";
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(secs >= 10 ? 0 : 1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return remSecs > 0 ? `${mins}m ${remSecs}s` : `${mins}m`;
}

function swarmAgentTypeLabel(agent: SwarmChatAgent): string {
  const role = agent.agentRole.trim();
  if (role) return agent.isSpecialist ? `${role} specialist` : role;
  return agent.isSpecialist ? "Specialist agent" : "Delegated agent";
}

function deriveSwarmRunStatus(
  agents: SwarmChatAgent[],
  fallback = "running",
): string {
  if (
    agents.some((agent) =>
      ["assigned", "running", "synthesizing"].includes(
        normalizeSwarmStatus(agent.status),
      ),
    )
  ) {
    return "running";
  }
  if (
    agents.length > 0 &&
    agents.every((agent) =>
      ["completed", "success", "done"].includes(normalizeSwarmStatus(agent.status)),
    )
  ) {
    return "completed";
  }
  if (
    agents.some((agent) => normalizeSwarmStatus(agent.status) === "interrupted")
  ) {
    return "interrupted";
  }
  if (
    agents.some((agent) =>
      ["completed", "success", "done"].includes(normalizeSwarmStatus(agent.status)),
    )
  ) {
    return "partial";
  }
  if (
    agents.some((agent) => normalizeSwarmStatus(agent.status) === "timed_out")
  ) {
    return "timed_out";
  }
  if (
    agents.some((agent) => normalizeSwarmStatus(agent.status) === "panicked")
  ) {
    return "panicked";
  }
  if (agents.some((agent) => normalizeSwarmStatus(agent.status) === "failed")) {
    return "failed";
  }
  return normalizeSwarmStatus(fallback);
}

function buildSwarmRunsFromStreamingSteps(
  steps: JsonRecord[],
  options?: { interrupted?: boolean },
): SwarmChatRun[] {
  const interrupted = Boolean(options?.interrupted);
  const runs = new Map<
    string,
    {
      id: string;
      request: string;
      status: string;
      summary: string;
      agentCount: number;
      updatedAtIndex: number;
      agents: Map<string, SwarmChatAgent>;
      order: string[];
    }
  >();

  steps.forEach((step, index) => {
    const payload = asRecord(step.data);
    const kind = str(payload.kind, "");
    if (!kind.startsWith("delegation_")) return;
    const runId = str(payload.delegation_id, "").trim();
    if (!runId) return;

    let run = runs.get(runId);
    if (!run) {
      run = {
        id: runId,
        request: "",
        status: "running",
        summary: "",
        agentCount: 0,
        updatedAtIndex: index,
        agents: new Map<string, SwarmChatAgent>(),
        order: [],
      };
      runs.set(runId, run);
    }

    run.updatedAtIndex = index;
    run.summary =
      str(payload.summary, str(step.detail, run.summary)).trim() || run.summary;
    run.request = str(payload.request, run.request).trim() || run.request;
    run.agentCount = Math.max(
      run.agentCount,
      Math.max(0, num(payload.agent_count, 0)),
    );

    if (kind === "delegation_started") {
      run.status = normalizeSwarmStatus(str(payload.status, "running"));
    } else if (kind === "delegation_synthesis_started") {
      run.status = normalizeSwarmStatus(str(payload.status, "synthesizing"));
    } else if (kind === "delegation_completed") {
      run.status = normalizeSwarmStatus(str(payload.status, "completed"));
    }

    const agentId = str(payload.agent_id, "").trim();
    if (!agentId) return;

    let agent = run.agents.get(agentId);
    if (!agent) {
      agent = {
        id: agentId,
        agentName: str(payload.agent_name, "Agent").trim() || "Agent",
        agentRole: str(payload.agent_role, "").trim(),
        modelName: str(payload.model_name, "").trim(),
        task: str(payload.task, "").trim(),
        status: "assigned",
        summary: str(payload.summary, "").trim(),
        latestUpdate: str(step.detail, "").trim(),
        isSpecialist: toBool(payload.is_specialist),
        dependsOn: Array.isArray(payload.depends_on)
          ? payload.depends_on
              .map((value) => num(value, -1))
              .filter((value) => value >= 0)
          : [],
        elapsedMs: undefined,
        sequence: Math.max(1, num(payload.sequence, run.order.length + 1)),
      };
      run.agents.set(agentId, agent);
      run.order.push(agentId);
    }

    agent.agentName =
      str(payload.agent_name, agent.agentName).trim() || agent.agentName;
    agent.agentRole =
      str(payload.agent_role, agent.agentRole).trim() || agent.agentRole;
    agent.modelName =
      str(payload.model_name, agent.modelName).trim() || agent.modelName;
    agent.task = str(payload.task, agent.task).trim() || agent.task;
    agent.summary = str(payload.summary, agent.summary).trim() || agent.summary;
    const latestUpdate = str(step.detail, str(payload.summary, agent.latestUpdate)).trim();
    agent.latestUpdate =
      /^\[omitted\s+\d+\s+chars?\]$/i.test(latestUpdate)
        ? agent.latestUpdate
        : latestUpdate || agent.latestUpdate;
    agent.isSpecialist = toBool(payload.is_specialist) || agent.isSpecialist;
    if (Array.isArray(payload.depends_on)) {
      agent.dependsOn = payload.depends_on
        .map((value) => num(value, -1))
        .filter((value) => value >= 0);
    }
    const elapsedMs = num(payload.elapsed_ms, 0);
    if (elapsedMs > 0) {
      agent.elapsedMs = elapsedMs;
    }

    if (kind === "delegation_assignment") {
      agent.status = "assigned";
    } else if (kind === "delegation_agent_started") {
      agent.status = normalizeSwarmStatus(str(payload.status, "running"));
    } else if (kind === "delegation_agent_progress") {
      agent.status = normalizeSwarmStatus(str(payload.status, "running"));
    } else if (kind === "delegation_agent_completed") {
      agent.status = normalizeSwarmStatus(str(payload.status, "completed"));
    } else if (kind === "delegation_agent_failed") {
      const reason = str(payload.reason, "").trim();
      agent.status = normalizeSwarmStatus(
        str(payload.status, /timeout/i.test(reason) ? "timed_out" : "failed"),
      );
    }
  });

  const out = Array.from(runs.values()).map((run) => {
    const agents = run.order
      .map((agentId) => run.agents.get(agentId))
      .filter((agent): agent is SwarmChatAgent => Boolean(agent))
      .sort((left, right) => left.sequence - right.sequence);

    let status = deriveSwarmRunStatus(agents, run.status);
    if (
      interrupted &&
      ["running", "assigned", "synthesizing"].includes(status)
    ) {
      status = "interrupted";
    }
    const normalizedAgents = agents.map((agent) => {
      const next = { ...agent };
      if (
        interrupted &&
        ["assigned", "running", "synthesizing"].includes(
          normalizeSwarmStatus(next.status),
        )
      ) {
        next.status = "interrupted";
        next.latestUpdate =
          next.latestUpdate || "Stopped before this delegated step finished.";
      }
      return next;
    });

    return {
      id: run.id,
      request: run.request,
      status,
      summary: run.summary,
      agentCount: Math.max(run.agentCount, normalizedAgents.length),
      updatedAtIndex: run.updatedAtIndex,
      agents: normalizedAgents,
    };
  });

  return out.sort((left, right) => right.updatedAtIndex - left.updatedAtIndex);
}

function SwarmActivityPanel({
  runs,
  interrupted = false,
  expandedPayloads,
  onTogglePayload,
}: {
  runs: SwarmChatRun[];
  interrupted?: boolean;
  expandedPayloads: Set<string>;
  onTogglePayload: (id: string) => void;
}) {
  if (runs.length === 0) return null;
  const totalAgents = runs.reduce(
    (sum, run) => sum + Math.max(run.agentCount, run.agents.length),
    0,
  );

  return (
    <Box
      sx={{
        mt: 1.5,
        p: 1.5,
        borderRadius: "var(--surface-radius-lg)",
        border: interrupted
          ? "1px solid var(--activity-panel-border-warning)"
          : "1px solid var(--activity-panel-border)",
        background: "var(--activity-panel-bg)",
        boxShadow: "var(--surface-shadow-soft)",
      }}
    >
      <Stack spacing={1.4}>
        <Stack
          direction={{ xs: "column", sm: "row" }}
          sx={{
            alignItems: { xs: "flex-start", sm: "center" },
            justifyContent: "space-between",
            gap: 1,
          }}
        >
          <Box>
            <Typography
              variant="overline"
              sx={{
                letterSpacing: 0,
                color: interrupted ? "warning.light" : "info.light",
              }}
            >
              Agent activity
            </Typography>
            <Typography variant="body2" sx={{ fontWeight: 700 }}>
              {interrupted
                ? "Delegated work was paused with live state preserved."
                : "Delegated specialists are working in parallel."}
            </Typography>
          </Box>
          <Stack
            direction="row"
            spacing={0.75}
            useFlexGap
            sx={{
              flexWrap: "wrap",
            }}
          >
            <span className={`chat-value-pill ${interrupted ? "tone-warning" : "tone-info"}`}>
              {runs.length} run{runs.length === 1 ? "" : "s"}
            </span>
            <span className="chat-value-pill">
              {totalAgents} agent{totalAgents === 1 ? "" : "s"}
            </span>
          </Stack>
        </Stack>

        {runs.map((run) => {
          const runSummary = summarizeActivityDetail(
            run.summary || `${run.agents.length} delegated agents tracked.`,
          );
          return (
            <Box
              key={run.id}
              sx={{
                p: 1.25,
                borderRadius: "var(--surface-radius)",
                background: "var(--activity-subpanel-bg)",
                border: "1px solid var(--surface-border)",
                boxShadow: "var(--micro-surface-shadow)",
              }}
            >
              <Stack spacing={1.2}>
                <Stack
                  direction={{ xs: "column", md: "row" }}
                  sx={{
                    alignItems: { xs: "flex-start", md: "center" },
                    justifyContent: "space-between",
                    gap: 1,
                  }}
                >
                  <Box sx={{ minWidth: 0 }}>
                    <Typography
                      variant="body2"
                      sx={{ fontWeight: 700 }}
                      className="swarm-run-request"
                    >
                      {run.request || "Delegated run"}
                    </Typography>
                    <Typography
                      variant="caption"
                      className="swarm-run-summary"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                        mt: 0.35,
                      }}
                    >
                      {runSummary ||
                        `${run.agents.length} delegated agents tracked.`}
                    </Typography>
                  </Box>
                  <Stack
                    direction="row"
                    spacing={0.75}
                    useFlexGap
                    sx={{
                      flexWrap: "wrap",
                    }}
                  >
                    <span className={`chat-value-pill tone-${swarmStatusChipColor(run.status)}`}>
                      {swarmStatusLabel(run.status)}
                    </span>
                    <span className="chat-value-pill">
                      {Math.max(run.agentCount, run.agents.length)} agent
                      {Math.max(run.agentCount, run.agents.length) === 1
                        ? ""
                        : "s"}
                    </span>
                  </Stack>
                </Stack>

                <Grid2 container spacing={1}>
                  {run.agents.map((agent) => {
                    const agentPayloadId = `swarm:${run.id}:${agent.id}`;
                    const agentPayloadView =
                      buildActivityPayloadViewFromSources(
                        agent.latestUpdate,
                        agent.summary,
                      );
                    const agentUpdate = summarizeActivityDetail(
                      agent.latestUpdate ||
                        agent.summary ||
                        "Waiting for the next update.",
                    );
                    const payloadControlsId =
                      `swarm-payload-${agentPayloadId}`.replace(
                        /[^a-zA-Z0-9_-]+/g,
                        "-",
                      );
                    return (
                      <Grid2 key={agent.id} size={{ xs: 12, xl: 6 }}>
                        <Box
                          sx={{
                            height: "100%",
                            p: 1.1,
                            borderRadius: "var(--surface-radius)",
                            border: "1px solid var(--surface-border)",
                            background: "var(--micro-surface-item-bg)",
                          }}
                        >
                          <Stack spacing={0.8}>
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              sx={{
                                alignItems: { xs: "flex-start", sm: "center" },
                                justifyContent: "space-between",
                                gap: 0.8,
                              }}
                            >
                              <Box sx={{ minWidth: 0 }}>
                                <Typography
                                  variant="body2"
                                  sx={{ fontWeight: 700 }}
                                >
                                  {agent.agentRole
                                    ? `${agent.agentName} / ${agent.agentRole}`
                                    : agent.agentName}
                                </Typography>
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  {swarmAgentTypeLabel(agent)}
                                </Typography>
                              </Box>
                              <Stack
                                direction="row"
                                spacing={0.75}
                                useFlexGap
                                sx={{
                                  flexWrap: "wrap",
                                }}
                              >
                                <span className={`chat-value-pill tone-${swarmStatusChipColor(agent.status)}`}>
                                  {swarmStatusLabel(agent.status)}
                                </span>
                                {agent.elapsedMs ? (
                                  <span className="chat-value-pill">
                                    {formatSwarmElapsedMs(agent.elapsedMs)}
                                  </span>
                                ) : null}
                              </Stack>
                            </Stack>
                            {agent.task ? (
                              <Typography
                                variant="body2"
                                className="swarm-agent-task"
                                sx={{ color: "var(--button-text)" }}
                              >
                                {agent.task}
                              </Typography>
                            ) : null}
                            <Typography
                              variant="caption"
                              className="swarm-agent-update"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {agentUpdate || "Waiting for the next update."}
                            </Typography>
                            {agentPayloadView ? (
                              <ActivityPayloadDisclosure
                                payload={agentPayloadView}
                                expanded={expandedPayloads.has(agentPayloadId)}
                                onToggle={() => onTogglePayload(agentPayloadId)}
                                controlsId={payloadControlsId}
                              />
                            ) : null}
                          </Stack>
                        </Box>
                      </Grid2>
                    );
                  })}
                </Grid2>
              </Stack>
            </Box>
          );
        })}
      </Stack>
    </Box>
  );
}

function extractPhaseStatusFromProgress(
  name: string,
  payload: unknown,
  fallbackDetail = "",
): StreamPhaseStatus | null {
  const payloadObj = asRecord(payload);
  if (str(payloadObj.kind, "") !== "phase_status") return null;
  const phase = str(payloadObj.phase, "").trim();
  const label = str(payloadObj.label, "").trim() || "Working";
  const detail = str(payloadObj.detail, fallbackDetail).trim();
  const rawStatus = str(payloadObj.status, "running").trim().toLowerCase();
  const status = phase === "completed" && rawStatus === "running"
    ? "completed"
    : rawStatus;
  const planStepId =
    typeof payloadObj.plan_step_id === "number"
      ? payloadObj.plan_step_id
      : num(payloadObj.plan_step_id, 0);
  return {
    toolName: name,
    phase,
    label,
    detail,
    status: ["completed", "failed", "skipped", "running"].includes(status)
      ? status
      : "running",
    elapsedSecs: Math.max(0, num(payloadObj.elapsed_secs, 0)),
    streamKey: str(
      payloadObj.stream_key,
      `phase-status:${name || "tool"}:${phase || "unknown"}`,
    ),
    planStepId: planStepId > 0 ? planStepId : null,
    planStepTitle: str(payloadObj.plan_step_title, "").trim(),
  };
}

function extractPhaseStatusFromActivityStep(
  step: JsonRecord,
): StreamPhaseStatus | null {
  if (str(step.step_type, "").trim().toLowerCase() !== "tool_progress")
    return null;
  const payloadObj = parseTraceDataRecord(step.data);
  const toolName = str(payloadObj.tool_name, "").trim();
  if (!toolName) return null;
  return extractPhaseStatusFromProgress(
    toolName,
    payloadObj,
    str(step.detail, ""),
  );
}

function summarizeRunStatusDegradation(notes: unknown): string {
  if (!Array.isArray(notes)) return "";
  const unique = Array.from(
    new Set(
      notes
        .map((entry) => asRecord(entry))
        .flatMap((entry) => [
          str(entry.summary, "").trim(),
          str(entry.detail, "").trim(),
        ])
        .filter(Boolean),
    ),
  );
  return summarizeActivityDetail(unique.slice(0, 2).join(" "));
}

function shouldSurfaceRunStatusStep(
  runStatus: string,
  requestState: string,
  outcomeStatus: string,
): boolean {
  return [runStatus, requestState, outcomeStatus].some((value) =>
    [
      "completed",
      "degraded",
      "blocked",
      "cancelled",
      "platform_failed",
      "service_unavailable",
      "hard_service_outage",
      "needs_input",
      "needs_stronger_model",
      "needs_credentials",
      "needs_permission",
      "needs_integration",
      "needs_clarification",
    ].includes(value),
  );
}

function buildRunStatusActivityStep(
  payloadValue: unknown,
  timestamp = "",
): JsonRecord | null {
  const payload = asRecord(payloadValue);
  const nested = asRecord(payload.payload);
  const userOutcome = asRecord(payload.user_outcome ?? nested.user_outcome);
  const runStatus = str(
    payload.run_status,
    str(payload.status, str(nested.status, str(payload.stage, ""))),
  )
    .trim()
    .toLowerCase();
  const requestState = str(
    userOutcome.request_state,
    str(nested.request_state, ""),
  )
    .trim()
    .toLowerCase();
  const outcomeStatus = str(
    userOutcome.status,
    str(nested.user_outcome_status, ""),
  )
    .trim()
    .toLowerCase();

  if (!shouldSurfaceRunStatusStep(runStatus, requestState, outcomeStatus)) {
    return null;
  }

  const effectiveStatus =
    outcomeStatus === "service_unavailable" ||
    requestState === "hard_service_outage"
      ? "service_unavailable"
      : requestState === "needs_credentials"
        ? requestState
        : runStatus || requestState || outcomeStatus;
  const detail =
    summarizeActivityDetail(
      str(userOutcome.message, "").trim() ||
        str(nested.error, "").trim() ||
        str(payload.error, "").trim() ||
        summarizeRunStatusDegradation(
          payload.degradation ?? nested.degradation ?? userOutcome.degradation,
        ) ||
        str(nested.response_preview, "").trim() ||
        str(payload.summary, "").trim() ||
        str(payload.detail, "").trim(),
    ) || "Run status updated.";

  return {
    step_type: "run_status",
    title: `Run status: ${humanizeMachineLabel(effectiveStatus || "updated")}`,
    detail,
    data: payload,
    timestamp,
  };
}

function persistedReasoningData(payload: JsonRecord): JsonRecord {
  const nested = activityDataRecord(payload.data);
  return Object.keys(nested).length > 0 ? nested : {};
}

function upsertPersistedReasoningStep(
  steps: JsonRecord[],
  indexesByStreamKey: Map<string, number>,
  payload: JsonRecord,
  timestamp: string,
  fallbackDetail = "",
): void {
  const nested = persistedReasoningData(payload);
  const phase = normalizeReasoningPhase(
    payload.phase ?? nested.phase ?? "reasoning",
  );
  const streamKey =
    str(payload.stream_key, str(nested.stream_key, str(payload.__streamKey, "")))
      .trim() || `reasoning:${phase || "active"}`;
  const existingIndex = indexesByStreamKey.get(streamKey);
  const existingStep =
    existingIndex == null ? null : asRecord(steps[existingIndex]);
  const existingData = asRecord(existingStep?.data);
  const currentText = str(
    existingData.content_snapshot,
    str(existingData.content, str(existingStep?.detail, "")),
  );
  const snapshot = str(payload.content_snapshot, str(nested.content_snapshot, ""));
  const delta = str(payload.content_delta, str(nested.content_delta, ""));
  const content = str(payload.content, str(nested.content, ""));
  const detail = str(payload.detail, str(nested.detail, fallbackDetail));
  const done = toBool(payload.done) || toBool(nested.done);
  const hasContentPayload = Boolean(
    snapshot.trim() || delta.trim() || content.trim(),
  );

  let nextText = currentText;
  if (snapshot.trim()) {
    nextText = snapshot;
  } else if (delta) {
    nextText = `${currentText}${delta}`;
  } else if (content) {
    nextText =
      currentText && !content.startsWith(currentText)
        ? `${currentText}${content}`
        : content;
  } else if (!(done && currentText.trim() && !hasContentPayload) && detail.trim()) {
    nextText = detail;
  }
  if (!nextText.trim()) return;

  const presentation = reasoningStatusCopy(phase, nextText, payload);
  const nextStep: JsonRecord = {
    ...(existingStep || {}),
    step_type: "reasoning_delta",
    title: presentation.title,
    detail: nextText,
    data: {
      ...nested,
      ...payload,
      kind: "reasoning_delta",
      phase,
      stream_key: streamKey,
      content: nextText,
      content_snapshot: nextText,
      done,
    },
    __streamKey: streamKey,
    timestamp: timestamp || str(existingStep?.timestamp, ""),
  };

  if (existingIndex == null) {
    indexesByStreamKey.set(streamKey, steps.length);
    steps.push(nextStep);
  } else {
    steps[existingIndex] = nextStep;
  }
}

function buildPersistedRunSteps(events: JsonRecord[]): JsonRecord[] {
  const steps: JsonRecord[] = [];
  const reasoningIndexesByStreamKey = new Map<string, number>();
  for (const rawEvent of events) {
    const event = asRecord(rawEvent);
    const kind = str(event.kind, "").trim().toLowerCase();
    const payload = asRecord(event.payload);
    const timestamp = str(event.ts, "");
    if (!kind) continue;

    if (kind === "thinking") {
      const stepType = str(payload.step_type, "thinking").trim() || "thinking";
      const nested = persistedReasoningData(payload);
      const payloadKind = str(payload.kind, str(nested.kind, ""))
        .trim()
        .toLowerCase();
      if (stepType === "reasoning_delta" || payloadKind === "reasoning_delta") {
        upsertPersistedReasoningStep(
          steps,
          reasoningIndexesByStreamKey,
          payload,
          timestamp,
          str(payload.detail, ""),
        );
        continue;
      }
      steps.push({
        step_type: stepType,
        title:
          str(payload.title, "").trim() ||
          (stepType === "heartbeat" ? "Working" : "Thinking"),
        detail: normalizeHeartbeatDetailText(str(payload.detail, "")),
        data: payload,
        timestamp,
      });
      continue;
    }

    if (kind === "reasoning_delta") {
      upsertPersistedReasoningStep(
        steps,
        reasoningIndexesByStreamKey,
        payload,
        timestamp,
      );
      continue;
    }

    if (kind === "tool_start") {
      const name = str(payload.name, "");
      const nestedPayload = asRecord(payload.payload);
      const inner =
        Object.keys(nestedPayload).length > 0 ? nestedPayload : payload;
      const intentText = toolStartIntentText(inner);
      steps.push({
        step_type: "tool_start",
        title: `Tool started: ${name || "tool"}`,
        detail:
          intentText || compactUnknown(inner, 240) || `Starting ${name || "tool"}.`,
        data:
          Object.keys(inner).length > 0
            ? { ...inner, tool_name: name }
            : { ...payload, tool_name: name },
        timestamp,
      });
      continue;
    }

    if (kind === "tool_progress") {
      const name = str(payload.name, "");
      const content = str(payload.content, "");
      const inner = asRecord(payload.payload);
      const presentation = buildToolProgressPresentation(
        name,
        content,
        inner,
        "",
      );
      steps.push({
        step_type: "tool_progress",
        title: presentation.title,
        detail: presentation.detail,
        data:
          Object.keys(inner).length > 0
            ? { ...inner, tool_name: name }
            : { ...payload, tool_name: name },
        ...(presentation.streamKey
          ? { __streamKey: presentation.streamKey }
          : {}),
        timestamp,
      });
      continue;
    }

    if (kind === "tool_result") {
      const name = str(payload.name, "");
      const content = str(payload.content, "");
      steps.push({
        step_type: "tool_result",
        title: `Tool finished: ${name || "tool"}`,
        detail: summarizeActivityDetail(content),
        data: { ...payload, tool_name: name },
        timestamp,
      });
      continue;
    }

    if (
      kind === "plan_generated" ||
      kind === "plan_revised" ||
      kind === "plan_ready_for_confirmation"
    ) {
      const normalizedPlan = normalizeExecutionPlanState(payload.plan);
      steps.push({
        step_type: kind,
        title:
          kind === "plan_generated"
            ? "Execution Plan"
            : kind === "plan_revised"
              ? "Execution Plan Revised"
              : "Plan Ready",
        detail:
          kind === "plan_generated"
            ? `${normalizedPlan?.steps.length || 0} steps planned`
            : kind === "plan_revised"
              ? str(payload.reason, "Execution plan revised.")
              : `${normalizedPlan?.steps.length || 0} steps ready`,
        plan: payload.plan,
        source: payload.source,
        task_id: payload.task_id,
        data: payload,
        timestamp,
      });
      continue;
    }

    if (kind === "plan_unavailable" || kind === "plan_step_update") {
      steps.push({
        step_type: kind,
        title:
          kind === "plan_unavailable"
            ? "Execution Plan Unavailable"
            : "Plan Step Update",
        detail:
          kind === "plan_unavailable"
            ? str(payload.reason, "Structured planning was unavailable.")
            : str(payload.detail, `Updated step ${num(payload.step_id, 0)}`),
        data: payload,
        timestamp,
      });
      continue;
    }

    if (kind === "run_status") {
      const runStatusStep = buildRunStatusActivityStep(payload, timestamp);
      if (runStatusStep) steps.push(runStatusStep);
    }
  }
  return sanitizeActivityStepsForUi(steps);
}

function isTraceCheckpointStep(step: JsonRecord): boolean {
  const stepType = str(step.step_type, str(step.type, ""))
    .trim()
    .toLowerCase();
  const source = str(step.source, "").trim().toLowerCase();
  const title = str(step.title, "").trim().toLowerCase();
  return (
    stepType === "checkpoint" ||
    source === "checkpoint" ||
    title.startsWith("checkpoint:")
  );
}

function parseTraceCheckpointRunEvent(step: JsonRecord): JsonRecord | null {
  if (!isTraceCheckpointStep(step)) return null;

  const artifacts = asRecords(step.artifacts);
  const candidates = [
    ...artifacts.map((artifact) => str(artifact.data, "")),
    str(step.data, "").replace(/^Checkpoint Payload\s*/i, ""),
    str(step.detail, ""),
  ]
    .map((value) => value.trim())
    .filter(Boolean);

  for (const candidate of candidates) {
    const parsed = tryParseActivityJson(candidate);
    const record = asRecord(parsed);
    if (
      str(record.kind, "").trim() &&
      Object.keys(asRecord(record.payload)).length > 0
    ) {
      return record;
    }
  }
  return null;
}

function buildTraceCheckpointRunSteps(rawSteps: JsonRecord[]): JsonRecord[] {
  const events = rawSteps
    .map(parseTraceCheckpointRunEvent)
    .filter((event): event is JsonRecord => Boolean(event));
  return events.length > 0 ? buildPersistedRunSteps(events) : [];
}

function isHumanReadableStatus(detail: string): boolean {
  const trimmed = (detail || "").trim();
  if (!trimmed || trimmed.length > 120) return false;
  if (looksLikeHtmlPayload(trimmed) || looksLikeSourcePayload(trimmed))
    return false;
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    return false;
  }
  return true;
}

function isSafetyPolicyBlockedText(text: string): boolean {
  return /blocked by safety policy/i.test(text || "");
}

function formatBytes(value: unknown): string {
  const bytes = num(value, -1);
  if (bytes < 0) return "-";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function sanitizeWorkspaceAppSnapshot(value: unknown): JsonRecord | null {
  const source = asRecord(value);
  const next: JsonRecord = {};
  for (const key of [
    "id",
    "app_id",
    "title",
    "url",
    "access_url",
    "local_url",
    "local_access_url",
    "app_dir",
    "runtime_mode",
    "created_at",
  ]) {
    const text = str(source[key], "").trim();
    if (text) next[key] = text;
  }
  for (const key of [
    "enabled",
    "running",
    "is_static",
    "access_guard_enabled",
    "expose_public",
  ]) {
    if (typeof source[key] === "boolean") next[key] = source[key];
  }
  if (typeof source.port === "number" && Number.isFinite(source.port)) {
    next.port = source.port;
  }
  return Object.keys(next).length > 0 ? next : null;
}

function compactWorkspaceFilesForSnapshot(
  files: WorkspaceFileEntry[],
  options?: { includeContent?: boolean },
): WorkspaceFileEntry[] {
  const out: WorkspaceFileEntry[] = [];
  let totalChars = 0;
  const includeContent = options?.includeContent !== false;
  for (const file of files) {
    const name = str(file?.name, "").trim();
    if (!name) continue;
    const remaining = CHAT_WORKSPACE_SNAPSHOT_MAX_TOTAL_CHARS - totalChars;
    if (remaining <= 0 || out.length >= CHAT_WORKSPACE_SNAPSHOT_MAX_FILES)
      break;
    const content = includeContent
      ? str(file?.content, "").slice(
          0,
          Math.min(CHAT_WORKSPACE_SNAPSHOT_MAX_FILE_CHARS, remaining),
        )
      : "";
    totalChars += content.length;
    out.push({ name, content });
  }
  return out;
}

function compactLiveFileWritesForSnapshot(
  liveFileWrites: Record<string, LiveFileWriteState>,
  options?: { includeContent?: boolean },
): Record<string, LiveFileWriteState> {
  const out: Record<string, LiveFileWriteState> = {};
  let totalChars = 0;
  const includeContent = options?.includeContent !== false;
  for (const [name, state] of Object.entries(liveFileWrites)) {
    const fileName = str(name, "").trim();
    if (!fileName) continue;
    const remaining = CHAT_WORKSPACE_SNAPSHOT_MAX_TOTAL_CHARS - totalChars;
    if (
      remaining <= 0 ||
      Object.keys(out).length >= CHAT_WORKSPACE_SNAPSHOT_MAX_FILES
    )
      break;
    const content = includeContent
      ? str(state?.content, "").slice(
          0,
          Math.min(CHAT_WORKSPACE_SNAPSHOT_MAX_FILE_CHARS, remaining),
        )
      : "";
    totalChars += content.length;
    out[fileName] = {
      content,
      line: Math.max(0, num(state?.line, 0)),
      totalLines: Math.max(0, num(state?.totalLines, 0)),
      done: toBool(state?.done),
    };
  }
  return out;
}

function loadStoredChatWorkspaceSnapshots(): Record<
  string,
  ChatWorkspaceSnapshot
> {
  if (typeof window === "undefined") return {};
  try {
    const raw =
      window.localStorage.getItem(CHAT_WORKSPACE_SNAPSHOTS_STORAGE_KEY) ??
      window.sessionStorage.getItem(CHAT_WORKSPACE_SNAPSHOTS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = asRecord(JSON.parse(raw));
    const now = Date.now();
    const next: Record<string, ChatWorkspaceSnapshot> = {};
    for (const [conversationId, value] of Object.entries(parsed)) {
      const entry = asRecord(value);
      const updatedAt = num(entry.updatedAt, 0);
      if (
        !conversationId.trim() ||
        updatedAt <= 0 ||
        now - updatedAt > CHAT_WORKSPACE_SNAPSHOT_TTL_MS
      ) {
        continue;
      }
      const deployedFiles = Array.isArray(entry.deployedFiles)
        ? compactWorkspaceFilesForSnapshot(
            entry.deployedFiles.map((row) => ({
              name: str(asRecord(row).name, ""),
              content: str(asRecord(row).content, ""),
            })),
          )
        : [];
      const liveFileWrites = compactLiveFileWritesForSnapshot(
        Object.fromEntries(
          Object.entries(asRecord(entry.liveFileWrites)).map(([name, row]) => {
            const state = asRecord(row);
            return [
              name,
              {
                content: str(state.content, ""),
                line: Math.max(0, num(state.line, 0)),
                totalLines: Math.max(0, num(state.totalLines, 0)),
                done: toBool(state.done),
              } satisfies LiveFileWriteState,
            ];
          }),
        ),
      );
      const streamedWorkspaceApp = sanitizeWorkspaceAppSnapshot(
        entry.streamedWorkspaceApp,
      );
      next[conversationId] = {
        conversationId,
        updatedAt,
        deployedFiles,
        liveFileWrites,
        streamedWorkspaceApp,
        codeViewerFileIdx: Math.max(0, num(entry.codeViewerFileIdx, 0)),
      };
    }
    return next;
  } catch {
    return {};
  }
}

function saveStoredChatWorkspaceSnapshots(
  snapshots: Record<string, ChatWorkspaceSnapshot>,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      CHAT_WORKSPACE_SNAPSHOTS_STORAGE_KEY,
      JSON.stringify(snapshots),
    );
    window.sessionStorage.removeItem(CHAT_WORKSPACE_SNAPSHOTS_STORAGE_KEY);
  } catch {
    // Ignore storage quota failures.
  }
}

function loadChatWorkspaceSnapshot(
  conversationId: string,
): ChatWorkspaceSnapshot | null {
  if (!conversationId) return null;
  return loadStoredChatWorkspaceSnapshots()[conversationId] || null;
}

function storeChatWorkspaceSnapshot(snapshot: ChatWorkspaceSnapshot): void {
  if (!snapshot.conversationId) return;
  const snapshots = loadStoredChatWorkspaceSnapshots();
  snapshots[snapshot.conversationId] = snapshot;
  const trimmedEntries = Object.entries(snapshots)
    .sort((a, b) => b[1].updatedAt - a[1].updatedAt)
    .slice(0, CHAT_WORKSPACE_SNAPSHOT_MAX_CONVERSATIONS);
  saveStoredChatWorkspaceSnapshots(Object.fromEntries(trimmedEntries));
}

function clearChatWorkspaceSnapshot(conversationId: string): void {
  if (!conversationId || typeof window === "undefined") return;
  const snapshots = loadStoredChatWorkspaceSnapshots();
  if (!snapshots[conversationId]) return;
  delete snapshots[conversationId];
  saveStoredChatWorkspaceSnapshots(snapshots);
}

function sanitizeChatTurnAttachments(raw: unknown): ChatTurnAttachment[] {
  const seen = new Set<string>();
  const out: ChatTurnAttachment[] = [];
  if (!Array.isArray(raw)) return out;
  for (const item of raw) {
    const record = asRecord(item);
    const name = str(record.name, str(record.filename, "")).trim();
    if (!name) continue;
    const kindRaw = str(record.kind, "").trim().toLowerCase();
    const kind =
      kindRaw === "document" || kindRaw === "visual" ? kindRaw : "file";
    const id = str(record.id, "").trim();
    const detail = str(record.detail, str(record.contentType, "")).trim();
    const key = `${kind}:${id || name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      name: name.slice(0, 220),
      kind,
      ...(id ? { id: id.slice(0, 160) } : {}),
      ...(detail ? { detail: detail.slice(0, 160) } : {}),
    });
  }
  return out.slice(0, 12);
}

function chatTurnAttachmentsFromFiles(files: File[]): ChatTurnAttachment[] {
  return sanitizeChatTurnAttachments(
    files.map((file) => ({
      name: file.name,
      kind: isVisualChatAttachment(file) ? "visual" : "file",
      detail: file.type || "",
    })),
  );
}

function compactExecutionPlanForPendingSnapshot(
  rawPlan: unknown,
): JsonRecord | null {
  const plan = executionPlanFromStructuredValue(rawPlan);
  if (!plan) return null;
  return {
    plan_id: plan.plan_id.slice(0, 160),
    revision: plan.revision,
    summary: plan.summary.slice(0, 2400),
    steps: plan.steps.slice(0, 64).map((step) => {
      const rawArguments = step.arguments;
      const compactedArguments =
        rawArguments && Object.keys(asRecord(rawArguments)).length > 0
          ? asRecord(sanitizeActivityPayloadForUi(rawArguments))
          : rawArguments ?? null;
      return {
        id: step.id,
        title: step.title.slice(0, 260),
        description: step.description.slice(0, 1800),
        status: step.status.slice(0, 80),
        action: step.action ? step.action.slice(0, 160) : null,
        arguments: compactedArguments,
        tool_hint: step.tool_hint ? step.tool_hint.slice(0, 180) : null,
        substeps: step.substeps.slice(0, 32).map((substep) => ({
          id: substep.id,
          title: substep.title.slice(0, 260),
          description: substep.description.slice(0, 1200),
          status: substep.status.slice(0, 80),
          tool_hint: substep.tool_hint
            ? substep.tool_hint.slice(0, 180)
            : null,
        })),
      };
    }),
  };
}

function compactPendingRunStepForSnapshot(rawStep: JsonRecord): JsonRecord {
  const raw = asRecord(rawStep);
  const data = asRecord(raw.data);
  const dataPreview = asRecord(data._plan_preview);
  const compacted: JsonRecord = {};
  const putString = (key: string, value: unknown, maxLen: number) => {
    const text = str(value, "").trim();
    if (text) compacted[key] = text.slice(0, maxLen);
  };
  const putNumber = (key: string, value: unknown) => {
    const numeric = typeof value === "number" ? value : num(value, Number.NaN);
    if (Number.isFinite(numeric)) compacted[key] = numeric;
  };

  putString("icon", raw.icon, 64);
  putString("title", raw.title, 220);
  putString("detail", raw.detail, 900);
  putString("step_type", raw.step_type, 80);
  putString("source", raw.source || data.source || dataPreview.source, 80);
  putString("__streamKey", raw.__streamKey, 180);
  putString("task_id", raw.task_id || data.task_id, 160);
  putString("conversation_id", raw.conversation_id || data.conversation_id, 160);
  putString("run_id", raw.run_id || data.run_id, 160);
  putString("plan_id", raw.plan_id || data.plan_id, 160);
  putString("step_title", raw.step_title || data.step_title, 260);
  putString("status", raw.status || data.status, 80);
  putNumber("revision", raw.revision ?? data.revision);
  putNumber("step_id", raw.step_id ?? data.step_id);

  if (isReasoningActivityRecord(raw)) {
    const phase = normalizeReasoningPhase(data.phase ?? raw.phase);
    const contentSnapshot = str(
      data.content_snapshot,
      str(data.content, str(raw.detail, "")),
    );
    const contentDelta = str(data.content_delta, "");
    const content = contentSnapshot || contentDelta;
    if (content) compacted.detail = content;
    compacted.data = {
      kind: "reasoning_delta",
      phase,
      stream_key:
        str(data.stream_key, str(raw.stream_key, str(raw.__streamKey, "")))
          .trim() || `reasoning:${phase}`,
      content,
      content_snapshot: content,
      done: toBool(data.done) || toBool(raw.done),
    };
    return compacted;
  }

  const plan =
    compactExecutionPlanForPendingSnapshot(raw.plan) ||
    compactExecutionPlanForPendingSnapshot(raw.plan_preview) ||
    compactExecutionPlanForPendingSnapshot(raw._plan_preview) ||
    compactExecutionPlanForPendingSnapshot(data.plan) ||
    compactExecutionPlanForPendingSnapshot(data.current_plan) ||
    compactExecutionPlanForPendingSnapshot(data.original_plan) ||
    compactExecutionPlanForPendingSnapshot(dataPreview.current_plan) ||
    compactExecutionPlanForPendingSnapshot(dataPreview.original_plan);
  if (plan) compacted.plan = plan;

  const compactedData = compactUnknown(raw.data, 800);
  if (compactedData) compacted.data = compactedData;
  return compacted;
}

function limitPendingRunStepsForSnapshot(steps: JsonRecord[]): JsonRecord[] {
  if (steps.length <= CHAT_PENDING_STREAM_STEPS_MAX) return steps;
  const keepIndexes = new Set<number>();
  steps.forEach((step, index) => {
    if (isMainChatReasoningStep(step)) keepIndexes.add(index);
  });
  let remaining = CHAT_PENDING_STREAM_STEPS_MAX;
  for (let index = steps.length - 1; index >= 0 && remaining > 0; index -= 1) {
    if (keepIndexes.has(index)) continue;
    keepIndexes.add(index);
    remaining -= 1;
  }
  return steps.filter((_, index) => keepIndexes.has(index));
}

function compactPendingRunStepsForSnapshot(steps: JsonRecord[]): JsonRecord[] {
  return limitPendingRunStepsForSnapshot(steps).map((step) =>
    compactPendingRunStepForSnapshot(asRecord(step)),
  );
}

function normalizeChatPendingRunSnapshot(
  raw: unknown,
): ChatPendingRunSnapshot | null {
  if (!raw || typeof raw !== "object") return null;
  const parsed = raw as Partial<ChatPendingRunSnapshot>;
  const conversationId =
    typeof parsed.conversationId === "string"
      ? parsed.conversationId.trim()
      : "";
  const startedAt = typeof parsed.startedAt === "number" ? parsed.startedAt : 0;
  if (!conversationId || startedAt <= 0) return null;
  if (Date.now() - startedAt > CHAT_PENDING_RUN_TTL_MS) return null;
  const streamingResponse =
    typeof parsed.streamingResponse === "string"
      ? parsed.streamingResponse.slice(
          0,
          CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS,
        )
      : "";
  const streamingSteps = Array.isArray(parsed.streamingSteps)
    ? compactPendingRunStepsForSnapshot(asRecords(parsed.streamingSteps))
    : [];
  const runId = typeof parsed.runId === "string" ? parsed.runId : "";
  const lastRunSeq =
    typeof parsed.lastRunSeq === "number" && Number.isFinite(parsed.lastRunSeq)
      ? Math.max(0, Math.floor(parsed.lastRunSeq))
      : 0;
  const initialMessageCount =
    typeof parsed.initialMessageCount === "number" &&
    Number.isFinite(parsed.initialMessageCount)
      ? Math.max(0, Math.floor(parsed.initialMessageCount))
      : undefined;
  const parsedPhase =
    parsed.phase === "interrupted"
      ? "interrupted"
      : parsed.phase === "awaiting_confirmation"
        ? "awaiting_confirmation"
        : "running";
  const phase =
    parsedPhase === "running" &&
    !runId.trim() &&
    Date.now() - startedAt > CHAT_PENDING_RUN_RECOVERY_GRACE_MS
      ? "interrupted"
      : parsedPhase;
  return {
    conversationId,
    message: typeof parsed.message === "string" ? parsed.message : "",
    startedAt,
    ...(initialMessageCount !== undefined ? { initialMessageCount } : {}),
    runId,
    mode: parsed.mode === "resume" ? "resume" : "fresh",
    phase,
    taskId: typeof parsed.taskId === "string" ? parsed.taskId : "",
    streamingResponse,
    streamingSteps,
    failedUserMessage:
      typeof parsed.failedUserMessage === "string"
        ? parsed.failedUserMessage
        : "",
    lastRunSeq,
    attachments: sanitizeChatTurnAttachments(parsed.attachments),
  };
}

function loadChatStoredRunSnapshot(
  storageKey: string,
): ChatPendingRunSnapshot | null {
  if (typeof window === "undefined") return null;
  try {
    const raw =
      window.localStorage.getItem(storageKey) ??
      window.sessionStorage.getItem(storageKey);
    if (!raw) return null;
    const normalized = normalizeChatPendingRunSnapshot(JSON.parse(raw));
    if (normalized) return normalized;
    window.localStorage.removeItem(storageKey);
    window.sessionStorage.removeItem(storageKey);
    return null;
  } catch {
    return null;
  }
}

function loadChatPendingLaunch(): ChatPendingLaunch | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(CHAT_PENDING_LAUNCH_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<ChatPendingLaunch>;
    const createdAt =
      typeof parsed.createdAt === "number" ? parsed.createdAt : 0;
    if (createdAt <= 0 || Date.now() - createdAt > CHAT_PENDING_RUN_TTL_MS) {
      window.sessionStorage.removeItem(CHAT_PENDING_LAUNCH_STORAGE_KEY);
      return null;
    }
    const launchMode =
      parsed.launchMode === "resume_task" ? "resume_task" : "message";
    const message = typeof parsed.message === "string" ? parsed.message : "";
    const taskId =
      typeof parsed.taskId === "string" ? parsed.taskId.trim() : "";
    if (launchMode === "resume_task" && !taskId) {
      window.sessionStorage.removeItem(CHAT_PENDING_LAUNCH_STORAGE_KEY);
      return null;
    }
    if (launchMode === "message" && !message.trim()) {
      window.sessionStorage.removeItem(CHAT_PENDING_LAUNCH_STORAGE_KEY);
      return null;
    }
    return {
      createdAt,
      launchMode,
      message,
      conversationId:
        typeof parsed.conversationId === "string" ? parsed.conversationId : "",
      newConversation: parsed.newConversation === true,
      taskId,
      source: typeof parsed.source === "string" ? parsed.source : "",
      acceptedSuggestionId:
        typeof parsed.acceptedSuggestionId === "string"
          ? parsed.acceptedSuggestionId
          : "",
      sentinelProposalId:
        typeof parsed.sentinelProposalId === "string"
          ? parsed.sentinelProposalId
          : "",
    };
  } catch {
    return null;
  }
}

function compactUiString(value: string, maxLen = CHAT_ACTIVITY_PAYLOAD_STRING_MAX_CHARS): string {
  if (value.length <= maxLen) return value;
  return `${value.slice(0, maxLen).trimEnd()}...`;
}

function omittedStringLabel(value: string): string {
  const chars = value.length;
  const lineCount = value ? value.split(/\r?\n/).length : 0;
  return lineCount > 1
    ? `[omitted ${chars.toLocaleString()} chars / ${lineCount.toLocaleString()} lines]`
    : `[omitted ${chars.toLocaleString()} chars]`;
}

function summarizeFilesPayloadForUi(value: unknown): JsonRecord[] {
  const summarizeEntry = (path: string, content: unknown): JsonRecord => {
    const body =
      typeof content === "string"
        ? content
        : str(asRecord(content).content, str(asRecord(content).raw_content, ""));
    const record = asRecord(content);
    const lineCount =
      body && typeof body === "string"
        ? body.split(/\r?\n/).length
        : Math.max(0, num(record.line_count, num(record.total_lines, 0)));
    const bytes =
      typeof record.bytes === "number"
        ? record.bytes
        : typeof record.size === "number"
          ? record.size
          : body.length;
    return {
      path,
      ...(bytes > 0 ? { bytes } : {}),
      ...(lineCount > 0 ? { line_count: lineCount } : {}),
    };
  };

  if (Array.isArray(value)) {
    return value
      .slice(0, CHAT_ACTIVITY_PAYLOAD_ARRAY_MAX_ITEMS)
      .map((entry) => {
        const record = asRecord(entry);
        const path = str(record.path, str(record.file, str(record.name, "")));
        return path ? summarizeEntry(path, record) : {};
      })
      .filter((entry) => !!str(entry.path, ""));
  }

  return Object.entries(asRecord(value))
    .slice(0, CHAT_ACTIVITY_PAYLOAD_ARRAY_MAX_ITEMS)
    .map(([path, content]) => summarizeEntry(path, content))
    .filter((entry) => !!str(entry.path, ""));
}

function summarizeNestedActivityPayloadForUi(
  value: unknown,
  key = "",
  depth = 0,
  parent?: unknown,
): unknown {
  const normalizedKey = key.trim().toLowerCase();
  if (ACTIVITY_PAYLOAD_SECRET_KEY_PATTERN.test(normalizedKey)) {
    return "[redacted]";
  }
  if (value == null || typeof value === "number" || typeof value === "boolean") {
    return value;
  }
  if (typeof value === "string") {
    if (shouldOmitActivityPayloadString(normalizedKey, parent)) {
      return value ? omittedStringLabel(value) : "";
    }
    return compactUiString(
      value,
      CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_CHARS,
    );
  }
  if (normalizedKey === "files" || normalizedKey === "sources") {
    return summarizeFilesPayloadForUi(value);
  }
  if (depth >= CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_DEPTH_MAX) {
    if (Array.isArray(value)) {
      return value.length === 0
        ? []
        : `[array: ${value.length.toLocaleString()} item${value.length === 1 ? "" : "s"}]`;
    }
    const visibleKeys = Object.keys(asRecord(value)).filter(
      (entryKey) => !ACTIVITY_PAYLOAD_INTERNAL_KEYS.has(entryKey),
    );
    if (visibleKeys.length === 0) return {};
    const shownKeys = visibleKeys.slice(
      0,
      CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_KEYS,
    );
    const suffix =
      visibleKeys.length > shownKeys.length
        ? `, +${visibleKeys.length - shownKeys.length} more`
        : "";
    return `[object: ${shownKeys.join(", ")}${suffix}]`;
  }
  if (Array.isArray(value)) {
    const items = value
      .slice(0, CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_ITEMS)
      .map((entry) =>
        summarizeNestedActivityPayloadForUi(entry, "", depth + 1, value),
      );
    if (value.length > items.length) {
      items.push(`[+${value.length - items.length} more items]`);
    }
    return items;
  }

  const source = asRecord(value);
  const out: JsonRecord = {};
  const entries = Object.entries(source);
  for (const [entryKey, entryValue] of entries.slice(
    0,
    CHAT_ACTIVITY_PAYLOAD_NESTED_SUMMARY_MAX_KEYS,
  )) {
    out[entryKey] = summarizeNestedActivityPayloadForUi(
      entryValue,
      entryKey,
      depth + 1,
      source,
    );
  }
  const omitted = entries.length - Object.keys(out).length;
  if (omitted > 0) out.__omitted_keys = omitted;
  return out;
}

function sanitizeActivityPayloadForUi(
  value: unknown,
  key = "",
  depth = 0,
  parent?: unknown,
): unknown {
  const normalizedKey = key.trim().toLowerCase();
  if (ACTIVITY_PAYLOAD_SECRET_KEY_PATTERN.test(normalizedKey)) {
    return "[redacted]";
  }
  if (value == null || typeof value === "number" || typeof value === "boolean") {
    return value;
  }
  if (typeof value === "string") {
    if (shouldOmitActivityPayloadString(normalizedKey, parent)) {
      return value ? omittedStringLabel(value) : "";
    }
    if (shouldPreserveFullActivityPayloadString(normalizedKey, parent)) {
      return value;
    }
    if (isStreamLikeActivityRecord(parent)) {
      return compactUiString(value, CHAT_ACTIVITY_STREAM_STRING_MAX_CHARS);
    }
    return compactUiString(value);
  }
  if (depth >= CHAT_ACTIVITY_PAYLOAD_DEPTH_MAX) {
    return summarizeNestedActivityPayloadForUi(value, key, 0, parent);
  }
  if (normalizedKey === "files" || normalizedKey === "sources") {
    return summarizeFilesPayloadForUi(value);
  }
  if (Array.isArray(value)) {
    const items = value
      .slice(0, CHAT_ACTIVITY_PAYLOAD_ARRAY_MAX_ITEMS)
      .map((entry) =>
        sanitizeActivityPayloadForUi(entry, "", depth + 1, value),
      );
    if (value.length > items.length) {
      items.push(`[omitted ${value.length - items.length} more items]`);
    }
    return items;
  }

  const source = asRecord(value);
  const out: JsonRecord = {};
  const entries = Object.entries(source);
  for (const [entryKey, entryValue] of entries.slice(0, CHAT_ACTIVITY_PAYLOAD_OBJECT_MAX_KEYS)) {
    out[entryKey] = sanitizeActivityPayloadForUi(
      entryValue,
      entryKey,
      depth + 1,
      source,
    );
  }
  const omitted = entries.length - Object.keys(out).length;
  if (omitted > 0) out.__omitted_keys = omitted;
  return out;
}

function sanitizeActivityStepForUi(step: JsonRecord): JsonRecord {
  return asRecord(sanitizeActivityPayloadForUi(step));
}

function sanitizeActivityStepsForUi(steps: JsonRecord[]): JsonRecord[] {
  return steps.map((step) => sanitizeActivityStepForUi(step));
}

function loadChatPendingRunSnapshot(): ChatPendingRunSnapshot | null {
  return loadChatStoredRunSnapshot(CHAT_PENDING_RUN_STORAGE_KEY);
}

function loadChatBackgroundRunSnapshots(): ChatPendingRunSnapshotMap {
  if (typeof window === "undefined") return {};
  try {
    const raw =
      window.localStorage.getItem(CHAT_BACKGROUND_RUN_STORAGE_KEY) ??
      window.sessionStorage.getItem(CHAT_BACKGROUND_RUN_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    const snapshots: ChatPendingRunSnapshotMap = {};
    const addSnapshot = (candidate: unknown, fallbackConversationId = "") => {
      const normalized = normalizeChatPendingRunSnapshot(candidate);
      const conversationId =
        normalized?.conversationId || fallbackConversationId.trim();
      if (!normalized || !conversationId) return;
      snapshots[conversationId] = {
        ...normalized,
        conversationId,
      };
    };
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const record = parsed as Record<string, unknown>;
      if (typeof record.conversationId === "string") {
        addSnapshot(parsed);
      } else {
        for (const [conversationId, value] of Object.entries(record)) {
          addSnapshot(value, conversationId);
        }
      }
    }
    return snapshots;
  } catch {
    return {};
  }
}

function storeChatPendingRunSnapshot(
  snapshot: ChatPendingRunSnapshot | null,
): void {
  storeChatStoredRunSnapshot(CHAT_PENDING_RUN_STORAGE_KEY, snapshot);
}

function storeChatBackgroundRunSnapshots(
  snapshots: ChatPendingRunSnapshotMap,
): void {
  if (typeof window === "undefined") return;
  try {
    const trimmedEntries = Object.entries(snapshots)
      .filter(([conversationId, snapshot]) => {
        const normalizedConversationId = conversationId.trim();
        return (
          !!normalizedConversationId &&
          !!snapshot &&
          snapshot.conversationId.trim() === normalizedConversationId
        );
      })
      .sort((a, b) => b[1].startedAt - a[1].startedAt)
      .slice(0, CHAT_BACKGROUND_RUN_SNAPSHOTS_MAX);
    if (trimmedEntries.length === 0) {
      window.localStorage.removeItem(CHAT_BACKGROUND_RUN_STORAGE_KEY);
      window.sessionStorage.removeItem(CHAT_BACKGROUND_RUN_STORAGE_KEY);
      return;
    }
    window.localStorage.setItem(
      CHAT_BACKGROUND_RUN_STORAGE_KEY,
      JSON.stringify(Object.fromEntries(trimmedEntries)),
    );
    window.sessionStorage.removeItem(CHAT_BACKGROUND_RUN_STORAGE_KEY);
  } catch {
    // Ignore storage failures.
  }
}

function storeChatStoredRunSnapshot(
  storageKey: string,
  snapshot: ChatPendingRunSnapshot | null,
): void {
  if (typeof window === "undefined") return;
  try {
    if (!snapshot) {
      window.localStorage.removeItem(storageKey);
      window.sessionStorage.removeItem(storageKey);
      return;
    }
    const serialized = JSON.stringify(snapshot);
    window.localStorage.setItem(storageKey, serialized);
    window.sessionStorage.removeItem(storageKey);
  } catch {
    // Ignore storage failures.
  }
}

function clearChatStoredRunSnapshotForConversation(
  storageKey: string,
  conversationId: string,
): void {
  if (!conversationId || typeof window === "undefined") return;
  const snapshot = loadChatStoredRunSnapshot(storageKey);
  if (!snapshot || snapshot.conversationId !== conversationId) return;
  try {
    window.localStorage.removeItem(storageKey);
    window.sessionStorage.removeItem(storageKey);
  } catch {
    // Ignore storage failures.
  }
}

function clearChatStoredBackgroundRunSnapshot(conversationId: string): void {
  if (!conversationId || typeof window === "undefined") return;
  const snapshots = loadChatBackgroundRunSnapshots();
  if (!snapshots[conversationId]) return;
  delete snapshots[conversationId];
  storeChatBackgroundRunSnapshots(snapshots);
}

function storeChatPendingLaunch(snapshot: ChatPendingLaunch | null): void {
  if (typeof window === "undefined") return;
  try {
    if (!snapshot) {
      window.sessionStorage.removeItem(CHAT_PENDING_LAUNCH_STORAGE_KEY);
      return;
    }
    window.sessionStorage.setItem(
      CHAT_PENDING_LAUNCH_STORAGE_KEY,
      JSON.stringify(snapshot),
    );
  } catch {
    // Ignore storage failures.
  }
}

function extractChatTurnAttachments(text: string): ChatTurnAttachment[] {
  const source = str(text, "");
  if (!source) return [];
  const out: ChatTurnAttachment[] = [];
  const docMatch = source.match(
    /\[Attached documents indexed for retrieval:\s*([\s\S]*?)\]/i,
  );
  if (docMatch) {
    const body = docMatch[1] || "";
    const filesPart = body.match(/(?:^|;)\s*files:\s*([\s\S]*)$/i)?.[1] || "";
    const refsPart = body.split(/;\s*files:/i)[0] || "";
    const ids = Array.from(refsPart.matchAll(/\bdoc:([A-Za-z0-9_-]+)/g)).map(
      (match) => match[1],
    );
    filesPart
      .split(/\s*,\s*/)
      .map((name) => name.trim())
      .filter(Boolean)
      .forEach((name, idx) => {
        out.push({
          name,
          kind: "document",
          ...(ids[idx] ? { id: ids[idx] } : {}),
        });
      });
  }
  const visualMatch = source.match(
    /\[Attached visual files available to vision\/OCR tools:\s*([\s\S]*?)\]/i,
  );
  if (visualMatch) {
    const body = visualMatch[1] || "";
    for (const match of body.matchAll(/\bupload_id:([^\s,]+)\s*\(([^)]*)\)/g)) {
      const id = match[1] || "";
      const rawMeta = match[2] || "";
      const splitAt = rawMeta.lastIndexOf(",");
      const name =
        splitAt >= 0 ? rawMeta.slice(0, splitAt).trim() : rawMeta.trim();
      const detail = splitAt >= 0 ? rawMeta.slice(splitAt + 1).trim() : "";
      if (!name) continue;
      out.push({
        name,
        kind: "visual",
        ...(id ? { id } : {}),
        ...(detail ? { detail } : {}),
      });
    }
  }
  return sanitizeChatTurnAttachments(out);
}

function stripAttachmentContextMarker(text: string): string {
  return text
    .replace(
      /\n\n\[Attached documents indexed for retrieval:[\s\S]*?\]/gi,
      "",
    )
    .replace(
      /\n\n\[Attached visual files available to vision\/OCR tools:[\s\S]*?\]/gi,
      "",
    )
    .trimEnd();
}

function looksLikeLeakedAgentPlanningTrace(text: string): boolean {
  const normalized = (text || "").replace(/\r\n/g, "\n").trim();
  if (!normalized) return false;
  const lines = normalized
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length < 4) return false;
  const lower = normalized.toLowerCase();
  const controlSignals = [
    "authorized actions",
    "available action",
    "turn plan",
    "tool history",
    "action scope",
    "scope expansion",
    "routing signal",
    "can_request_expansion",
    "parameters include",
    "call the action",
  ].filter((needle) => lower.includes(needle)).length;
  const firstPersonOperationalLines = lines.filter((line) =>
    /^(let me|i need to|i should|i can|i have|i don't have|wait\b|actually\b)/i.test(
      line,
    ),
  ).length;
  return controlSignals >= 2 && firstPersonOperationalLines >= 2;
}

// Defensive strip for assistant-message rendering. The agent loop uses an
// out-of-band scope-expansion sentinel and a legacy JSON envelope; if either
// slips into a streamed response (e.g. the iteration that produced it was
// shown briefly before the loop continued), we don't want them rendered as
// prose. This is belt-and-suspenders; the canonical fix is the prompt
// hygiene rule that tells the model not to emit these in user-visible text.
function stripAgentControlArtifacts(text: string): string {
  if (!text) return text;
  let out = text;
  if (looksLikeLeakedAgentPlanningTrace(out)) {
    return "This run exposed an internal planning trace instead of a final answer. Open Run Details for the trace, then retry after the install flow fix is running.";
  }
  // 1. Scope-expansion sentinels. Drop malformed historical spellings too,
  // because old turns may stream them before the loop continues.
  out = out.replace(/[ \t]*<<<AGENT_?SCOPE_?EXPAND>>>[^\n]*/gim, "");
  // 2. Non-native XML tool-call dialects. AgentArk parses native tool calls
  // or the JSON fallback protocol; these blocks are control artifacts, not
  // assistant prose.
  out = out.replace(
    /<function_calls\b[^>]*>[\s\S]*?<\/function_?calls>/gi,
    "",
  );
  out = out.replace(/<invoke\b[^>]*>[\s\S]*?<\/invoke>/gi, "");
  out = out.replace(/<parameter\b[^>]*>[\s\S]*?<\/parameter>/gi, "");
  out = out.replace(/<function_calls\b[\s\S]*$/gi, "");
  out = out.replace(/<invoke\b[\s\S]*$/gi, "");
  out = out.replace(/<parameter\b[\s\S]*$/gi, "");
  out = out.replace(
    /<agentark_internal_tool_context\b[^>]*>[\s\S]*?<\/agentark_internal_tool_context>/gi,
    "",
  );
  out = out.replace(/<agentark_internal_tool_context\b[\s\S]*$/gi, "");
  // 2. Legacy JSON envelope: `{"agent_action_scope":"expand", ...}`. Strip
  // wherever it appears so it can't render as prose. Tight pattern; only
  // matches the specific control-protocol shape, never legitimate JSON.
  out = out.replace(
    /\{\s*"agent_action_scope"\s*:\s*"expand"[\s\S]*?\}/g,
    "",
  );
  out = out.replace(/\{\s*"agent_tool_calls"\s*:\s*\[[\s\S]*?\]\s*\}/g, "");
  out = out.replace(/\{\s*"agent_action_scope"\s*:\s*"expand"[\s\S]*$/g, "");
  out = out.replace(/\{\s*"agent_tool_calls"\s*:\s*\[[\s\S]*$/g, "");
  // Collapse blank-line gaps left behind by removals.
  out = out.replace(/\n{3,}/g, "\n\n").trimEnd();
  return out;
}

type ChatMarkdownBlock =
  | { type: "heading"; level: number; text: string }
  | { type: "code"; language: string; content: string }
  | { type: "ul"; items: string[] }
  | { type: "ol"; items: string[] }
  | { type: "blockquote"; text: string }
  | { type: "hr" }
  | { type: "table"; rows: string[][] }
  | { type: "paragraph"; text: string };

type ChatMarkdownRenderOptions = {
  snippetNamespace?: string;
  onOpenSnippet?: (request: CodePreviewOpenRequest) => void;
};

function lineStartsMarkdownBlock(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return true;
  if (/^#{1,6}\s+/.test(trimmed)) return true;
  if (/^```/.test(trimmed)) return true;
  if (/^[-*]\s+/.test(trimmed)) return true;
  if (/^\d+\.\s+/.test(trimmed)) return true;
  return false;
}

function isIndentedMarkdownCodeCandidate(line: string): boolean {
  return /^(?: {4,}|\t)/.test(line);
}

function stripMarkdownCodeIndent(line: string): string {
  if (line.startsWith("\t")) return line.slice(1);
  return line.replace(/^ {4}/, "");
}

function looksLikeCodeLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  if (/^#{1,6}\s+/.test(trimmed)) return false;
  if (/^\s*(?:[-*+]|\d+[.)])\s+/.test(trimmed)) return false;
  if (/^(?:import|export|const|let|var|function|class|def|return|if|for|while|try|catch|async|await|SELECT|WITH|INSERT|UPDATE|CREATE)\b/.test(trimmed)) {
    return true;
  }
  if (/^(?:\$|>|#)\s+\S/.test(trimmed)) return true;
  if (/^(?:npm|pnpm|yarn|node|python|pip|cargo|git|docker|curl|cd|ls|mkdir|touch|cat|rg)\b/.test(trimmed)) {
    return true;
  }
  if (/^(?:<\/?[A-Za-z][\w:-]*|\{|\}|\[|\]|\/\/|\/\*|\*)/.test(trimmed)) {
    return true;
  }
  const codePunctuation = (trimmed.match(/[{}()[\];=<>]/g) || []).length;
  return codePunctuation >= 3 || /=>|;\s*$/.test(trimmed);
}

function looksLikeProseLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  if (/^#{1,6}\s+/.test(trimmed)) return true;
  if (/^\s*(?:[-*+]|\d+[.)])\s+/.test(trimmed)) return true;
  if (/^[>"'`]\S?/.test(trimmed)) return true;
  const words = trimmed.match(/[A-Za-z][A-Za-z'-]*/g) || [];
  if (words.length >= 4 && !looksLikeCodeLine(trimmed)) return true;
  if (words.length >= 2 && /[.!?:]$/.test(trimmed) && !looksLikeCodeLine(trimmed)) {
    return true;
  }
  return words.length >= 2 && /^[A-Z][A-Za-z0-9 /&,'()-]{2,90}$/.test(trimmed);
}

function looksLikeAccidentalIndentedProse(block: string[]): boolean {
  const contentLines = block
    .map(stripMarkdownCodeIndent)
    .map((line) => line.trim())
    .filter(Boolean);
  if (contentLines.length === 0) return false;
  const proseLines = contentLines.filter(looksLikeProseLine).length;
  const codeLines = contentLines.filter(looksLikeCodeLine).length;
  if (codeLines > 0 && codeLines >= proseLines) return false;
  return proseLines >= Math.max(1, Math.ceil(contentLines.length * 0.55));
}

function normalizeChatMarkdownForDisplay(text: string): string {
  const source = (text || "").replace(/\r\n/g, "\n");
  if (!source.includes("    ") && !source.includes("\t")) return source;

  const lines = source.split("\n");
  const out: string[] = [];
  let inFence = false;
  let index = 0;

  while (index < lines.length) {
    const line = lines[index] || "";
    const trimmed = line.trim();
    if (/^```/.test(trimmed)) {
      inFence = !inFence;
      out.push(line);
      index += 1;
      continue;
    }

    if (!inFence && trimmed && isIndentedMarkdownCodeCandidate(line)) {
      const block: string[] = [];
      while (index < lines.length) {
        const blockLine = lines[index] || "";
        const blockTrimmed = blockLine.trim();
        if (blockTrimmed && /^```/.test(blockTrimmed)) break;
        if (blockTrimmed && !isIndentedMarkdownCodeCandidate(blockLine)) break;
        block.push(blockLine);
        index += 1;
      }
      out.push(
        ...(looksLikeAccidentalIndentedProse(block)
          ? block.map(stripMarkdownCodeIndent)
          : block),
      );
      continue;
    }

    out.push(line);
    index += 1;
  }

  return out.join("\n");
}

function formatCompactValue(value: unknown): {
  text: string;
  tooltip?: string;
} {
  if (value == null) return { text: "-" };
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (looksLikeIsoTimestamp(trimmed)) {
      const meta = formatUiDateTimeMeta(trimmed, { fallback: "-" });
      return { text: meta.label, tooltip: meta.tip };
    }
    if (looksLikeIsoDateOnly(trimmed)) {
      const text = formatUiDateOnly(trimmed, { fallback: "-" });
      const tooltip = formatUiDateOnly(trimmed, {
        fallback: "-",
        includeYear: true,
      });
      return { text, tooltip };
    }
    return { text: value };
  }
  if (typeof value === "number")
    return { text: Number.isFinite(value) ? String(value) : "-" };
  if (typeof value === "boolean") return { text: value ? "true" : "false" };

  if (Array.isArray(value)) {
    const items = value
      .slice(0, 5)
      .map((v) =>
        typeof v === "string"
          ? v
          : typeof v === "number"
            ? String(v)
            : typeof v === "boolean"
              ? v
                ? "true"
                : "false"
              : "...",
      )
      .join(", ");
    const suffix = value.length > 5 ? ` +${value.length - 5} more` : "";
    return {
      text: items ? `${items}${suffix}` : `${value.length} items`,
      tooltip: items || undefined,
    };
  }

  if (typeof value === "object") {
    const rec = asRecord(value);
    const title =
      str(rec.title, "") ||
      str(rec.name, "") ||
      str(rec.label, "") ||
      str(rec.description, "");
    const id = str(rec.id, "");
    if (title) return { text: title, tooltip: id ? `ID: ${id}` : undefined };
    // Summarise scalar fields as readable text
    const scalars = Object.entries(rec)
      .filter(
        ([, v]) =>
          typeof v === "string" ||
          typeof v === "number" ||
          typeof v === "boolean",
      )
      .slice(0, 4)
      .map(
        ([k, v]) =>
          `${k}: ${typeof v === "string" && String(v).length > 30 ? `${String(v).slice(0, 30)}...` : String(v)}`,
      );
    if (scalars.length > 0) {
      const keys = Object.keys(rec);
      const more =
        keys.length > scalars.length
          ? ` (+${keys.length - scalars.length} fields)`
          : "";
      return {
        text: scalars.join(", ") + more,
        tooltip: `Fields: ${keys.join(", ")}`,
      };
    }
    const keys = Object.keys(rec);
    return {
      text: keys.length ? `${keys.length} fields` : "-",
      tooltip: keys.length ? `Fields: ${keys.join(", ")}` : undefined,
    };
  }

  return { text: String(value) };
}

function looksLikeUrl(value: string): boolean {
  const v = (value || "").trim();
  return v.startsWith("http://") || v.startsWith("https://");
}

function localAgentArkHref(value: string): string | null {
  try {
    const parsed = new URL(value);
    if (parsed.hostname !== "app.agentark.ai") return null;
    if (
      parsed.pathname.startsWith("/apps/") ||
      parsed.pathname.startsWith("/api/outputs/")
    ) {
      return `${parsed.pathname}${parsed.search}${parsed.hash}`;
    }
  } catch {
    return null;
  }
  return null;
}

function normalizeOutboundHref(value?: string): string | null {
  const trimmed = (value || "").trim();
  if (!trimmed) return null;
  const lower = trimmed.toLowerCase();
  if (
    lower.startsWith("javascript:") ||
    lower.startsWith("data:") ||
    lower.startsWith("vbscript:")
  ) {
    return null;
  }
  const localHref = localAgentArkHref(trimmed);
  if (localHref) return localHref;
  if (
    trimmed === "/apps" ||
    trimmed.startsWith("/apps/") ||
    trimmed === "/ui/documents" ||
    trimmed.startsWith("/ui/documents?") ||
    (trimmed.startsWith("/api/outputs/") &&
      !trimmed.includes("..") &&
      !trimmed.includes("\\"))
  ) {
    return trimmed;
  }
  if (looksLikeUrl(trimmed)) return trimmed;
  if (trimmed.startsWith("//")) return `https:${trimmed}`;
  if (/^[a-z0-9.-]+\.[a-z]{2,}(?:[/:?#]|$)/i.test(trimmed)) {
    return `https://${trimmed}`;
  }
  return null;
}

function renderInlineMarkdown(text: string): ReactNode[] {
  const source = text || "";
  if (!source) return [];
  const tokenRegex =
    /(`[^`\n]+`|\*\*[^*]+?\*\*|__[^_]+?__|\*[^*\n]+?\*|_[^_\n]+?_|(?:https?:\/\/[^\s<>()]+)|\[[^\]]+\]\(([^)\s]+)\))/g;
  const nodes: ReactNode[] = [];
  let index = 0;
  let lastIndex = 0;
  let match: RegExpExecArray | null = null;

  const pushText = (value: string) => {
    if (!value) return;
    nodes.push(<span key={`t-${index++}`}>{value}</span>);
  };

  while ((match = tokenRegex.exec(source)) !== null) {
    const token = match[0];
    const start = match.index;
    if (start > lastIndex) pushText(source.slice(lastIndex, start));

    if (token.startsWith("`") && token.endsWith("`")) {
      nodes.push(
        <code key={`c-${index++}`} className="chat-md-inline-code">
          {token.slice(1, -1)}
        </code>,
      );
    } else if (
      (token.startsWith("**") && token.endsWith("**")) ||
      (token.startsWith("__") && token.endsWith("__"))
    ) {
      nodes.push(<strong key={`b-${index++}`}>{token.slice(2, -2)}</strong>);
    } else if (
      (token.startsWith("*") && token.endsWith("*")) ||
      (token.startsWith("_") && token.endsWith("_"))
    ) {
      nodes.push(<em key={`i-${index++}`}>{token.slice(1, -1)}</em>);
    } else if (token.startsWith("[")) {
      const linkMatch = token.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
      if (linkMatch) {
        const rawHref = linkMatch[2].trim();
        const normalizedHref = normalizeOutboundHref(rawHref);
        if (!normalizedHref) {
          pushText(token);
          lastIndex = tokenRegex.lastIndex;
          continue;
        }
        nodes.push(
          <a
            key={`l-${index++}`}
            href={normalizedHref}
            target="_blank"
            rel="noopener noreferrer"
            className="chat-md-link"
            onClick={handleChatLinkClick}
          >
            {linkMatch[1]}
          </a>,
        );
      } else {
        pushText(token);
      }
    } else if (token.startsWith("http://") || token.startsWith("https://")) {
      const { href, trailing } = splitUrlTrailingPunctuation(token);
      const normalizedHref = normalizeOutboundHref(href);
      if (!normalizedHref) {
        pushText(token);
        lastIndex = tokenRegex.lastIndex;
        continue;
      }
      nodes.push(
        <a
          key={`u-${index++}`}
          href={normalizedHref}
          target="_blank"
          rel="noopener noreferrer"
          className="chat-md-link"
          onClick={handleChatLinkClick}
        >
          {normalizedHref}
        </a>,
      );
      if (trailing) pushText(trailing);
    } else {
      pushText(token);
    }

    lastIndex = tokenRegex.lastIndex;
  }

  if (lastIndex < source.length) pushText(source.slice(lastIndex));
  return nodes;
}

function renderMarkdownLineBreaks(text: string): ReactNode[] {
  const lines = (text || "").split("\n");
  const out: ReactNode[] = [];
  for (let i = 0; i < lines.length; i += 1) {
    out.push(
      <span key={`line-${i}`}>{renderInlineMarkdown(lines[i] || "")}</span>,
    );
    if (i < lines.length - 1) out.push(<br key={`br-${i}`} />);
  }
  return out;
}

function splitMarkdownTableRow(line: string): string[] {
  const trimmed = (line || "")
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "");
  return trimmed.split("|").map((cell) => cell.trim());
}

function isMarkdownTableSeparator(line: string): boolean {
  const cells = splitMarkdownTableRow(line);
  return (
    cells.length >= 2 &&
    cells.every((cell) => /^:?-{3,}:?$/.test(cell.replace(/\s+/g, "")))
  );
}

function parseMarkdownFallbackTable(
  lines: string[],
  startIndex: number,
): { rows: string[][]; nextIndex: number } | null {
  const headerLine = lines[startIndex] || "";
  const separatorLine = lines[startIndex + 1] || "";
  if (!headerLine.includes("|") || !isMarkdownTableSeparator(separatorLine)) {
    return null;
  }
  const rows = [splitMarkdownTableRow(headerLine)];
  let nextIndex = startIndex + 2;
  while (nextIndex < lines.length) {
    const line = lines[nextIndex] || "";
    if (!line.trim() || !line.includes("|")) break;
    rows.push(splitMarkdownTableRow(line));
    nextIndex += 1;
  }
  return rows[0]?.length ? { rows, nextIndex } : null;
}

function containsMarkdownTable(text: string): boolean {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  return lines.some((_, index) => Boolean(parseMarkdownFallbackTable(lines, index)));
}

function parseMarkdownFallbackBlocks(text: string): ChatMarkdownBlock[] {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  const blocks: ChatMarkdownBlock[] = [];
  let paragraphLines: string[] = [];
  let listType: "ul" | "ol" | null = null;
  let listItems: string[] = [];
  let quoteLines: string[] = [];
  let inCode = false;
  let codeLanguage = "";
  let codeLines: string[] = [];

  const flushParagraph = () => {
    const text = paragraphLines.join("\n").trim();
    if (text) blocks.push({ type: "paragraph", text });
    paragraphLines = [];
  };
  const flushList = () => {
    if (listType && listItems.length > 0) {
      blocks.push({ type: listType, items: listItems });
    }
    listType = null;
    listItems = [];
  };
  const flushQuote = () => {
    const text = quoteLines.join("\n").trim();
    if (text) blocks.push({ type: "blockquote", text });
    quoteLines = [];
  };
  const flushFlow = () => {
    flushParagraph();
    flushList();
    flushQuote();
  };

  for (let i = 0; i < lines.length; i += 1) {
    const rawLine = lines[i] || "";
    const line = rawLine.replace(/\s+$/, "");
    const trimmed = line.trim();

    if (inCode) {
      if (/^```/.test(trimmed)) {
        blocks.push({
          type: "code",
          language: codeLanguage,
          content: codeLines.join("\n"),
        });
        inCode = false;
        codeLanguage = "";
        codeLines = [];
      } else {
        codeLines.push(rawLine);
      }
      continue;
    }

    const codeFence = trimmed.match(/^```+\s*([^`]*)$/);
    if (codeFence) {
      flushFlow();
      inCode = true;
      codeLanguage = (codeFence[1] || "").trim().split(/\s+/)[0] || "";
      continue;
    }

    if (!trimmed) {
      flushFlow();
      continue;
    }

    const table = parseMarkdownFallbackTable(lines, i);
    if (table) {
      flushFlow();
      blocks.push({ type: "table", rows: table.rows });
      i = table.nextIndex - 1;
      continue;
    }

    if (/^([-*_])(?:\s*\1){2,}\s*$/.test(trimmed)) {
      flushFlow();
      blocks.push({ type: "hr" });
      continue;
    }

    const heading = trimmed.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      flushFlow();
      blocks.push({
        type: "heading",
        level: Math.min(6, Math.max(1, heading[1].length)),
        text: heading[2].trim(),
      });
      continue;
    }

    const quote = line.match(/^\s*>\s?(.*)$/);
    if (quote) {
      flushParagraph();
      flushList();
      quoteLines.push(quote[1] || "");
      continue;
    }
    flushQuote();

    const unordered = line.match(/^\s*[-*+]\s+(.+)$/);
    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
    if (unordered || ordered) {
      flushParagraph();
      const nextType = unordered ? "ul" : "ol";
      if (listType && listType !== nextType) flushList();
      listType = nextType;
      listItems.push((unordered?.[1] || ordered?.[1] || "").trim());
      continue;
    }

    if (lineStartsMarkdownBlock(line)) {
      flushFlow();
    }
    paragraphLines.push(line);
  }

  if (inCode) {
    blocks.push({
      type: "code",
      language: codeLanguage,
      content: codeLines.join("\n"),
    });
  }
  flushFlow();
  return blocks;
}

function splitUrlTrailingPunctuation(
  value: string,
): { href: string; trailing: string } {
  let href = value;
  let trailing = "";
  while (href.length > 0 && /[.,!?;:)]/.test(href[href.length - 1] || "")) {
    trailing = `${href[href.length - 1]}${trailing}`;
    href = href.slice(0, -1);
  }
  return { href, trailing };
}

function handleChatLinkClick(event: React.MouseEvent<HTMLElement>): void {
  event.stopPropagation();
  const anchor = event.currentTarget as HTMLAnchorElement;
  const outputPath = apiOutputPathFromHref(
    anchor.getAttribute("href") || anchor.href,
  );
  if (!outputPath) return;
  event.preventDefault();
  void downloadApiFile(outputPath).catch((error) => {
    console.error("Failed to download AgentArk output", error);
    const message =
      error instanceof Error ? error.message : "Could not download this file.";
    window.alert(`Download failed: ${message}`);
  });
}

// Hoisted so ReactMarkdown sees a referentially-stable plugin array.
const CHAT_REMARK_PLUGINS = [remarkGfm];

// React.memo + a memoized components map: the markdown re-parse (the single
// most expensive render-path operation in chat) now only runs when the text
// itself changes, not on every parent render. The snippet counter lives in a
// ref so the stable components map still numbers code fences from 0 on each
// (re)parse, matching the previous per-render `let blockIndex` behavior.
const MarkdownBody = memo(function MarkdownBody({
  text,
  snippetNamespace,
  onOpenSnippet,
}: { text: string } & ChatMarkdownRenderOptions) {
  const blockIndexRef = useRef(0);
  blockIndexRef.current = 0;
  const components = useMemo<Components>(() => {
  const componentMap: Components = {
    a({ href, children }) {
      const normalizedHref = normalizeOutboundHref(href);
      if (!normalizedHref) return <span>{children}</span>;
      return (
        <a
          className="chat-md-link"
          href={normalizedHref}
          target="_blank"
          rel="noopener noreferrer"
          onClick={handleChatLinkClick}
        >
          {children}
        </a>
      );
    },
    img({ src, alt }) {
      const normalizedSrc = normalizeOutboundHref(src);
      const label = str(alt, "").trim();
      if (!normalizedSrc) {
        return label ? <span className="chat-md-image-alt">{label}</span> : null;
      }
      return (
        <img
          className="chat-md-image"
          src={normalizedSrc}
          alt={label}
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={(event) => {
            event.currentTarget.style.display = "none";
          }}
        />
      );
    },
    pre({ children }) {
      const extracted = extractMarkdownCodeBlock(children);
      if (!extracted) {
        return <pre className="chat-md-code">{reactNodeToPlainText(children)}</pre>;
      }
      if (isAgentArkChartFence(extracted.className)) {
        return <InlineAgentArkChart code={extracted.code} />;
      }
      const snippetIndex = blockIndexRef.current++;
      const fileName = inferCodePreviewFileName(
        extracted.className,
        extracted.code,
      );
      const snippetId = snippetNamespace
        ? `${snippetNamespace}::snippet::${snippetIndex}`
        : undefined;
      return (
        <InlineCodePreview
          code={extracted.code}
          languageHint={extracted.className}
          fileName={fileName}
          snippetId={snippetId}
          onOpenInWorkspace={onOpenSnippet}
        />
      );
    },
    code({ children, className }) {
      const normalizedClassName = str(className, "").trim();
      return (
        <code className={normalizedClassName || "chat-md-inline-code"}>
          {children}
        </code>
      );
    },
    blockquote({ children }) {
      const raw = deepNodeText(children);
      const match = raw.match(
        /^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*\n?([\s\S]*)$/i,
      );
      if (match) {
        const kind = match[1].toLowerCase();
        const body = match[2].trim();
        const meta = CHAT_CALLOUT_META[kind] ?? CHAT_CALLOUT_META.note;
        return (
          <div className={`chat-md-callout chat-md-callout-${kind}`}>
            <span className="chat-md-callout-icon" aria-hidden="true">
              {meta.icon}
            </span>
            <div className="chat-md-callout-body">
              <span className="chat-md-callout-label">{meta.label}</span>
              <ReactMarkdown
                remarkPlugins={CHAT_REMARK_PLUGINS}
                components={componentMap}
              >
                {body}
              </ReactMarkdown>
            </div>
          </div>
        );
      }
      return <blockquote>{children}</blockquote>;
    },
  };
  return componentMap;
  }, [snippetNamespace, onOpenSnippet]);

  return (
    <ReactMarkdown remarkPlugins={CHAT_REMARK_PLUGINS} components={components}>
      {text}
    </ReactMarkdown>
  );
});

function extractCodeFences(
  text: string,
): Array<{ languageHint: string; code: string }> {
  const source = (text || "").trim();
  if (!source) return [];
  const out: Array<{ languageHint: string; code: string }> = [];
  const regex = /```([^\n`]*)\n([\s\S]*?)```/g;
  let match: RegExpExecArray | null = null;
  while ((match = regex.exec(source)) !== null) {
    const code = str(match[2], "").replace(/\r\n/g, "\n").replace(/\n$/, "");
    if (!code.trim()) continue;
    const languageHint = str(match[1], "").trim();
    if (isAgentArkChartFence(languageHint)) continue;
    out.push({
      languageHint,
      code,
    });
  }
  return out;
}

function extractFirstCodeFence(text: string): string {
  const first = extractCodeFences(text).find((snippet) =>
    isWorkspaceCodePreview(snippet.languageHint, snippet.code),
  );
  return first?.code.trim() || "";
}

function buildWorkspaceSnippetFiles(
  messages: unknown[],
): WorkspaceSnippetEntry[] {
  const out: WorkspaceSnippetEntry[] = [];
  let replyIndex = 0;
  let globalSnippetIndex = 0;
  messages.forEach((message, idx) => {
    const record = asRecord(message);
    if (str(record.role, "").toLowerCase() !== "assistant") return;
    const messageId = str(record.id, String(idx));
    const snippets = extractCodeFences(str(record.content, ""));
    if (snippets.length === 0) return;
    replyIndex += 1;
    snippets.forEach((snippet, snippetIndex) => {
      if (!isWorkspaceCodePreview(snippet.languageHint, snippet.code)) return;
      globalSnippetIndex += 1;
      const displayName = inferCodePreviewFileName(
        snippet.languageHint,
        snippet.code,
      );
      out.push({
        id: `${messageId}::snippet::${snippetIndex}`,
        name: `snippet-${globalSnippetIndex}-${displayName}`,
        displayName,
        content: snippet.code,
        languageHint: normalizeCodeFenceLanguage(snippet.languageHint),
        sourceMessageId: messageId,
        sourceLabel:
          snippets.length > 1
            ? `Reply ${replyIndex} / block ${snippetIndex + 1}`
            : `Reply ${replyIndex}`,
      });
    });
  });
  return out;
}

function renderChatMarkdown(
  text: string,
  options?: ChatMarkdownRenderOptions,
): ReactNode {
  const normalizedText = normalizeChatMarkdownForDisplay(text);
  if (!normalizedText.trim()) return null;
  return (
    <Box className="chat-markdown">
      <MarkdownBody
        text={normalizedText}
        snippetNamespace={options?.snippetNamespace}
        onOpenSnippet={options?.onOpenSnippet}
      />
    </Box>
  );
}

function renderStreamingChatMarkdown(
  text: string,
  options?: ChatMarkdownRenderOptions,
): ReactNode {
  const trimmed = normalizeChatMarkdownForDisplay(text).trimEnd();
  if (!trimmed.trim()) return null;
  return (
    <Box className="chat-markdown chat-markdown-streaming">
      <MarkdownBody
        text={trimmed}
        snippetNamespace={options?.snippetNamespace}
        onOpenSnippet={options?.onOpenSnippet}
      />
    </Box>
  );
}

function stripMarkdownDecorations(text: string): string {
  return (text || "")
    .replace(/\r\n/g, "\n")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^\s*#{1,6}\s+/gm, "")
    .replace(/^\s*[-+*]\s+/gm, "")
    .replace(/^\s*\d+\.\s+/gm, "")
    .replace(/[*_~]/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function extractMarkdownSection(text: string, heading: string): string {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  const target = `## ${heading}`.replace(/\s+/g, " ").trim().toLowerCase();
  let collecting = false;
  const collected: string[] = [];
  for (const line of lines) {
    const normalized = line.replace(/\s+/g, " ").trim().toLowerCase();
    if (normalized.startsWith("## ")) {
      if (collecting) break;
      collecting = normalized === target;
      continue;
    }
    if (collecting) {
      collected.push(line);
    }
  }
  return collected.join("\n").trim();
}

function normalizeMarkdownHeadingText(value: string): string {
  return stripMarkdownDecorations(value)
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

function extractMarkdownSectionMatching(
  text: string,
  matchesHeading: (heading: string) => boolean,
): string {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  let collecting = false;
  let headingLevel = 0;
  const collected: string[] = [];
  for (const line of lines) {
    const headingMatch = line.trim().match(/^(#{2,6})\s+(.+)$/);
    if (headingMatch) {
      const currentLevel = headingMatch[1].length;
      const heading = normalizeMarkdownHeadingText(headingMatch[2]);
      if (collecting && currentLevel <= headingLevel) break;
      if (!collecting && matchesHeading(heading)) {
        collecting = true;
        headingLevel = currentLevel;
        continue;
      }
    }
    if (collecting) collected.push(line);
  }
  return collected.join("\n").trim();
}

function extractMarkdownBulletSummaries(section: string, limit = 3): string[] {
  const lines = (section || "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const items = lines
    .filter((line) => !line.includes("|") && !line.startsWith("```"))
    .map((line) => line.replace(/^\s*(?:[-+*]|\d+[.)])\s+/, "").trim())
    .map((line) => stripMarkdownDecorations(line).replace(/\s+/g, " ").trim())
    .filter((line) => line.length > 20);
  if (items.length > 0) return items.slice(0, limit);
  return stripMarkdownDecorations(section)
    .split(/\n{2,}/)
    .map((block) => block.replace(/\s+/g, " ").trim())
    .filter((block) => block.length > 40)
    .slice(0, limit);
}

function countMarkdownItems(section: string): number {
  const lines = (section || "")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const itemCount = lines.filter((line) =>
    /^(\d+\.\s+|[-+*]\s+)/.test(line),
  ).length;
  if (itemCount > 0) return itemCount;
  return lines.length > 0 ? lines.filter((line) => line.length > 20).length : 0;
}

function metricCountFromMarkdown(text: string, label: string): number {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = (text || "").match(
    new RegExp(`\\*\\*\\s*${escaped}\\s*:\\s*\\*\\*\\s*(\\d+)`, "i"),
  );
  return match ? Math.max(0, Number.parseInt(match[1], 10) || 0) : 0;
}

const RESEARCH_EVIDENCE_BRIEF_HEADING = "## Evidence Brief Used For Synthesis";

function splitResearchEvidenceBrief(text: string): {
  mainContent: string;
  evidenceBrief: string;
} {
  const normalized = (text || "").replace(/\r\n/g, "\n").trim();
  if (!normalized) return { mainContent: "", evidenceBrief: "" };
  const markerPattern = new RegExp(
    `\\n\\s*-{3,}\\s*\\n\\s*${RESEARCH_EVIDENCE_BRIEF_HEADING.replace(
      /[.*+?^${}()|[\]\\]/g,
      "\\$&",
    )}`,
    "i",
  );
  const markerMatch = normalized.match(markerPattern);
  if (!markerMatch || markerMatch.index == null) {
    return { mainContent: normalized, evidenceBrief: "" };
  }
  const mainContent = normalized.slice(0, markerMatch.index).trim();
  const evidenceStart = markerMatch.index + markerMatch[0].lastIndexOf("##");
  const evidenceBrief = normalized.slice(evidenceStart).trim();
  return { mainContent, evidenceBrief };
}

function countAgentArkChartBlocks(text: string): number {
  const matches = (text || "").match(/```agentark-chart\s*[\s\S]*?```/gi);
  return matches?.length ?? 0;
}

function countMarkdownTables(text: string): number {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  let count = 0;
  for (let idx = 1; idx < lines.length; idx += 1) {
    const previous = lines[idx - 1] || "";
    const current = lines[idx] || "";
    if (
      previous.includes("|") &&
      /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(current)
    ) {
      count += 1;
    }
  }
  return count;
}

function isPlaceholderResearchReportContent(
  normalized: string,
  summary: string,
  keyFindingCount: number,
  sourceCount: number,
): boolean {
  const lowerSummary = (summary || "").trim().toLowerCase();
  const lowerNormalized = (normalized || "").trim().toLowerCase();
  const noEvidence = keyFindingCount === 0 && sourceCount === 0;
  if (!noEvidence) return false;
  return (
    lowerSummary.startsWith("no relevant information found for:") ||
    lowerNormalized.includes("no relevant information found for:") ||
    lowerNormalized.includes(
      "i gathered tool evidence, but the final response could not be formatted cleanly",
    )
  );
}

function looksLikeDiscardableResearchFailureMessage(content: string): boolean {
  const normalized = (content || "")
    .replace(/\r\n/g, "\n")
    .trim()
    .toLowerCase();
  if (!normalized.startsWith("# research")) return false;
  return normalized.includes("no relevant information found for:");
}

function trimResearchReportTail(text: string): string {
  const lines = (text || "").replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  for (const line of lines) {
    const trimmed = line.trim();
    if (
      out.length > 0 &&
      (trimmed.startsWith(
        "I completed the tool work, but the follow-up model",
      ) ||
        trimmed.startsWith(
          "I completed the tool work, but the follow-up models",
        ) ||
        trimmed.startsWith("I gathered tool evidence") ||
        trimmed.startsWith("Research report:") ||
        trimmed.startsWith("Web search gathered "))
    ) {
      break;
    }
    out.push(line);
  }
  return out.join("\n").trim();
}

function parseResearchReport(
  text: string,
  options: { allowEvidenceThin?: boolean } = {},
): ResearchReportPreview | null {
  const normalized = trimResearchReportTail(text);
  if (!normalized) return null;
  const lines = normalized.split("\n");
  const firstHeadingIndex = lines.findIndex((line) => line.trim().length > 0);
  if (firstHeadingIndex < 0) return null;
  const headingLine = lines[firstHeadingIndex].trim();
  const titleMatch = headingLine.match(
    /^#\s*(Deep Research|Research(?: Summary)?):\s*(.+)$/i,
  );
  if (!titleMatch) return null;
  const kind = /^deep/i.test(titleMatch[1] || "") ? "deep" : "research";
  const { mainContent, evidenceBrief } = splitResearchEvidenceBrief(normalized);
  const previewContent = mainContent || normalized;
  const previewLines = previewContent.split("\n");

  const firstSectionIndex = previewLines.findIndex(
    (line, idx) => idx > firstHeadingIndex && /^##\s+/.test(line.trim()),
  );
  const summaryBlock = previewLines
    .slice(
      firstHeadingIndex + 1,
      firstSectionIndex >= 0 ? firstSectionIndex : previewLines.length,
    )
    .join("\n")
    .trim();
  const executiveSummarySection = extractMarkdownSectionMatching(
    previewContent,
    (heading) => heading === "executive summary",
  );
  const fallbackSummaryBlock = summaryBlock
    .split("\n")
    .filter((line) => {
      const normalizedLine = stripMarkdownDecorations(line)
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      return (
        normalizedLine &&
        normalizedLine !== "---" &&
        !normalizedLine.startsWith("analyst research report")
      );
    })
    .join("\n")
    .trim();
  const summary = stripMarkdownDecorations(
    executiveSummarySection || fallbackSummaryBlock,
  );
  const summaryPreview =
    summary.length > 700 ? `${summary.slice(0, 697).trimEnd()}...` : summary;
  const keyFindingsSection =
    extractMarkdownSection(previewContent, "Key Findings") ||
    extractMarkdownSectionMatching(
      previewContent,
      (heading) => heading.includes("key finding") || heading.includes("finding"),
    );
  const keyFindingCount = countMarkdownItems(keyFindingsSection);
  const keyFindings = extractMarkdownBulletSummaries(keyFindingsSection, 3);
  const sourceSection = extractMarkdownSection(normalized, "Sources");
  const sourceCount = Math.max(
    countMarkdownItems(sourceSection),
    metricCountFromMarkdown(normalized, "Sources analyzed"),
  );
  const openQuestionsSection =
    extractMarkdownSection(normalized, "Open Questions") ||
    extractMarkdownSectionMatching(normalized, (heading) =>
      heading.includes("open question"),
    );
  const openQuestions = extractMarkdownBulletSummaries(openQuestionsSection, 3);
  const openQuestionCount = Math.max(
    countMarkdownItems(openQuestionsSection),
    openQuestions.length,
  );
  const contradictionsSection =
    extractMarkdownSection(normalized, "Contradictions To Verify") ||
    extractMarkdownSectionMatching(normalized, (heading) =>
      heading.includes("contradiction") || heading.includes("conflict"),
    );
  const contradictions = extractMarkdownBulletSummaries(
    contradictionsSection,
    3,
  );
  const contradictionCount = Math.max(
    countMarkdownItems(contradictionsSection),
    contradictions.length,
    normalizeMarkdownHeadingText(previewContent).includes("contradiction")
      ? 1
      : 0,
  );
  const highlights =
    keyFindings.length > 0
      ? keyFindings
      : extractMarkdownBulletSummaries(executiveSummarySection || summary, 3);
  if (
    isPlaceholderResearchReportContent(
      normalized,
      summary,
      keyFindingCount,
      sourceCount,
    ) &&
    !options.allowEvidenceThin
  ) {
    return null;
  }

  return {
    kind,
    title: str(titleMatch[2], "Research report").trim() || "Research report",
    summary,
    summaryPreview,
    keyFindings,
    keyFindingCount,
    openQuestions,
    contradictions,
    highlights,
    sourceCount,
    tableCount: countMarkdownTables(normalized),
    chartCount: countAgentArkChartBlocks(normalized),
    openQuestionCount,
    contradictionCount,
    mainContent: mainContent || normalized,
    evidenceBrief,
    content: normalized,
  };
}

function looksLikeResearchReportBody(text: string): boolean {
  const normalized = trimResearchReportTail(text);
  if (!normalized) return false;
  if (splitResearchEvidenceBrief(normalized).evidenceBrief.trim()) return true;
  const lines = normalized.split("\n").map((line) => line.trim());
  const sectionLikeLines = lines.filter((line) =>
    /^(#{2,6}\s+\S|\d+\.\s+\S)/.test(line),
  ).length;
  if (sectionLikeLines >= 2) return true;
  const paragraphCount = normalized
    .split(/\n{2,}/)
    .map((block) => stripMarkdownDecorations(block).trim())
    .filter((block) => block.length >= 120).length;
  return normalized.length >= 1200 && paragraphCount >= 4;
}

function parseResearchReportWithContext(
  text: string,
  options: {
    deepResearch?: boolean;
    previousUserPrompt?: string;
    conversationTitle?: string;
  } = {},
): ResearchReportPreview | null {
  const direct = parseResearchReport(text);
  if (direct || !options.deepResearch) return direct;
  const normalized = trimResearchReportTail(text);
  if (!normalized) return null;
  if (!looksLikeResearchReportBody(normalized)) return null;
  const title =
    shortenAssistantExportText(
      stripAttachmentContextMarker(
        options.previousUserPrompt || options.conversationTitle || "",
      ),
      180,
    ) || "Deep research report";
  return parseResearchReport(`# Deep Research: ${title}\n\n${normalized}`, {
    allowEvidenceThin: true,
  });
}

function isDeepResearchAssistantMessage(message: JsonRecord): boolean {
  return isDeepResearchPlanSource(str(message.model_used, ""));
}

function collectAssistantExportParagraphs(text: string): string[] {
  return stripMarkdownDecorations(text)
    .split(/\n{2,}/)
    .map((block) => block.replace(/\s+/g, " ").trim())
    .filter((block) => block.length > 0);
}

function shortenAssistantExportText(text: string, maxChars = 220): string {
  const normalized = (text || "").replace(/\s+/g, " ").trim();
  if (!normalized) return "";
  if (normalized.length <= maxChars) return normalized;
  return `${normalized.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function deriveAssistantExportHeading(
  content: string,
  report: ResearchReportPreview | null,
  headingHint: string,
  prompt: string,
  conversationTitle: string,
): string {
  const explicitHeading = str(report?.title, headingHint).trim();
  if (explicitHeading) return explicitHeading;

  const lines = (content || "").replace(/\r\n/g, "\n").split("\n");
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const markdownHeading = trimmed.match(/^#{1,6}\s+(.+)$/);
    if (markdownHeading) {
      const candidate = markdownHeading[1].trim();
      if (candidate.length >= 8 && candidate.length <= 140) {
        return candidate;
      }
      continue;
    }
    if (
      trimmed.length >= 8 &&
      trimmed.length <= 120 &&
      !/[.!?]$/.test(trimmed) &&
      !trimmed.includes("|")
    ) {
      return trimmed;
    }
  }

  const fallback = prompt || conversationTitle || "AgentArk report";
  return shortenAssistantExportText(fallback, 110) || "AgentArk report";
}

function buildAssistantExportSummaryBullets(
  content: string,
  report: ResearchReportPreview | null,
  plan: ExecutionPlanState | null,
  planFailure: string,
): string[] {
  const seen = new Set<string>();
  const bullets: string[] = [];
  const push = (value: string) => {
    const cleaned = shortenAssistantExportText(value, 240);
    if (!cleaned) return;
    const key = cleaned.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    bullets.push(cleaned);
  };

  if (report) {
    push(report.summaryPreview || report.summary);
    report.keyFindings.slice(0, 3).forEach(push);
  } else {
    const paragraphs = collectAssistantExportParagraphs(content);
    paragraphs.slice(0, 2).forEach(push);
    const recommendationParagraph = paragraphs.find((paragraph) =>
      /\brecommend/i.test(paragraph),
    );
    if (recommendationParagraph) {
      push(recommendationParagraph);
    }
  }

  if (plan?.steps.length) {
    push(
      `${plan.steps.length} execution step${plan.steps.length === 1 ? "" : "s"} captured before or during the run.`,
    );
  } else if (planFailure) {
    push(`Execution planning was unavailable: ${planFailure}`);
  }

  return bullets.slice(0, 4);
}

function stripDuplicateAssistantExportHeading(
  content: string,
  heading: string,
): string {
  const lines = (content || "").replace(/\r\n/g, "\n").split("\n");
  const headingKey = stripMarkdownDecorations(heading)
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  let firstNonEmptyIndex = -1;
  for (let idx = 0; idx < lines.length; idx += 1) {
    if (lines[idx].trim()) {
      firstNonEmptyIndex = idx;
      break;
    }
  }
  if (firstNonEmptyIndex < 0) return "";
  const firstLineKey = stripMarkdownDecorations(lines[firstNonEmptyIndex])
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  if (headingKey && headingKey === firstLineKey) {
    lines.splice(firstNonEmptyIndex, 1);
  }
  return lines.join("\n").trim();
}

function formatAssistantExportBody(content: string, heading: string): string {
  const deduped = stripDuplicateAssistantExportHeading(content, heading);
  const lines = deduped.replace(/\r\n/g, "\n").split("\n");
  const formatted = lines.map((line) => {
    const trimmed = line.trim();
    if (!trimmed) return "";
    const numberedSection = trimmed.match(/^(\d+)\.\s+(.{3,140})$/);
    if (numberedSection) {
      return `## ${numberedSection[2].trim()}`;
    }
    return line;
  });
  return formatted
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function escapeMarkdownTableCell(value: string): string {
  return (value || "").replace(/\|/g, "\\|").replace(/\s+/g, " ").trim();
}

function markdownTableFromRecords(records: JsonRecord[]): string {
  if (records.length === 0) return "";
  const columns = Array.from(
    records.reduce((set, record) => {
      Object.keys(record).forEach((key) => set.add(key));
      return set;
    }, new Set<string>()),
  ).slice(0, 8);
  if (columns.length === 0) return "";
  const lines = [
    `| ${columns.map(escapeMarkdownTableCell).join(" | ")} |`,
    `| ${columns.map(() => "---").join(" | ")} |`,
  ];
  records.slice(0, 12).forEach((record) => {
    lines.push(
      `| ${columns
        .map((column) => {
          const value = record[column];
          if (value == null) return "";
          if (
            typeof value === "string" ||
            typeof value === "number" ||
            typeof value === "boolean"
          ) {
            return escapeMarkdownTableCell(String(value));
          }
          return escapeMarkdownTableCell(JSON.stringify(value));
        })
        .join(" | ")} |`,
    );
  });
  return lines.join("\n");
}

function convertAgentArkChartFencesForExport(content: string): string {
  return (content || "").replace(
    /```agentark-chart\s*([\s\S]*?)```/gi,
    (_match, rawJson: string) => {
      const raw = (rawJson || "").trim();
      if (!raw) return "";
      try {
        const parsed = JSON.parse(raw);
        const record = asRecord(parsed);
        const title = str(record.title, "Chart").trim() || "Chart";
        const data = Array.isArray(record.data)
          ? asRecords(record.data)
          : [];
        const table = markdownTableFromRecords(data);
        return table ? `**${title}**\n\n${table}` : `**${title}**`;
      } catch {
        return "**Chart data omitted from Markdown export.**";
      }
    },
  );
}

function cleanResearchReportMarkdownForExport(
  report: ResearchReportPreview,
  options: { preserveChartFences?: boolean; includeEvidenceBrief?: boolean } = {},
): string {
  let body = (report.mainContent || report.content || "").trim();
  body = body.replace(
    /^#\s*(?:Deep Research|Research(?: Summary)?):\s*/i,
    "# ",
  );
  body = body.replace(
    /^##\s+\*\*(.+?)\*\*\s*(?:-|:|\u2013|\u2014)+\s*(.+)$/gm,
    "## $1\n\n$2",
  );
  if (!options.preserveChartFences) {
    body = convertAgentArkChartFencesForExport(body);
  }
  const evidenceBrief = (report.evidenceBrief || "").trim();
  if (options.includeEvidenceBrief && evidenceBrief) {
    body = `${body.trim()}\n\n---\n\n${evidenceBrief}`;
  }
  const sources = options.includeEvidenceBrief
    ? ""
    : extractMarkdownSection(report.evidenceBrief, "Sources");
  if (sources && !/^##\s+Sources\b/im.test(body)) {
    body = `${body.trim()}\n\n## Sources\n\n${sources.trim()}`;
  }
  return body.replace(/\n{3,}/g, "\n\n").trim();
}

function escapeDocumentHtml(value: string): string {
  return (value || "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function safeDocumentHref(value: string): string {
  const href = (value || "").trim();
  if (!/^(https?:|mailto:|#)/i.test(href)) return "";
  return escapeDocumentHtml(href);
}

function renderDocumentInlineMarkdown(value: string): string {
  let html = escapeDocumentHtml(value);
  html = html.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    (_match, label: string, href: string) => {
      const safeHref = safeDocumentHref(
        href.replace(/&amp;/g, "&").replace(/&quot;/g, '"'),
      );
      return safeHref ? `<a href="${safeHref}">${label}</a>` : label;
    },
  );
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  return html;
}

function markdownTableToDocumentHtml(rows: string[][]): string {
  if (rows.length === 0) return "";
  const [header = [], ...bodyRows] = rows;
  const headerHtml = header
    .map((cell) => `<th>${renderDocumentInlineMarkdown(cell)}</th>`)
    .join("");
  const bodyHtml = bodyRows
    .map(
      (row) =>
        `<tr>${row
          .map((cell) => `<td>${renderDocumentInlineMarkdown(cell)}</td>`)
          .join("")}</tr>`,
    )
    .join("");
  return `<table><thead><tr>${headerHtml}</tr></thead><tbody>${bodyHtml}</tbody></table>`;
}

function reportChartNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return null;
  const parsed = Number(value.replace(/[$,%\s,]/g, ""));
  return Number.isFinite(parsed) ? parsed : null;
}

function reportChartText(value: unknown, fallback = ""): string {
  if (typeof value === "string" && value.trim()) return value.trim();
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return fallback;
}

function reportChartRows(value: unknown): JsonRecord[] {
  return Array.isArray(value)
    ? value.filter((item): item is JsonRecord => {
        const record = asRecord(item);
        return Object.keys(record).length > 0;
      })
    : [];
}

function reportChartCategoryKey(spec: JsonRecord, rows: JsonRecord[]): string {
  const explicit = reportChartText(spec.x);
  if (explicit) return explicit;
  const keys = Array.from(new Set(rows.flatMap((row) => Object.keys(row))));
  return (
    keys.find((key) => rows.some((row) => reportChartNumber(row[key]) == null)) ||
    keys[0] ||
    ""
  );
}

function reportChartSeriesKeys(
  spec: JsonRecord,
  rows: JsonRecord[],
  categoryKey: string,
): string[] {
  if (Array.isArray(spec.series)) {
    const explicit = spec.series
      .map((item) =>
        typeof item === "string" ? item : reportChartText(asRecord(item).key),
      )
      .map((key) => key.trim())
      .filter(Boolean);
    if (explicit.length > 0) return explicit.slice(0, 4);
  }
  const keys = Array.from(new Set(rows.flatMap((row) => Object.keys(row))));
  return keys
    .filter(
      (key) =>
        key !== categoryKey &&
        rows.some((row) => reportChartNumber(row[key]) != null),
    )
    .slice(0, 4);
}

function reportChartKind(spec: JsonRecord): string {
  const kind = reportChartText(spec.type).toLowerCase();
  return ["line", "area", "scatter", "pie", "doughnut"].includes(kind)
    ? kind
    : "bar";
}

function reportChartColor(index: number): string {
  const colors = ["#78f2b0", "#d8ad78", "#b7a7ff", "#5f8f5f"];
  return colors[index % colors.length] || colors[0];
}

function reportChartValueLabel(value: number): string {
  if (Math.abs(value) >= 1000) return value.toLocaleString(undefined, { maximumFractionDigits: 0 });
  if (Math.abs(value) >= 10) return value.toLocaleString(undefined, { maximumFractionDigits: 1 });
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function reportChartStableId(value: string): string {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) | 0;
  }
  return Math.abs(hash).toString(36) || "0";
}

function reportChartBarSvg({
  label,
  value,
  percent,
  gradientId,
}: {
  label: string;
  value: number;
  percent: number;
  gradientId: string;
}): string {
  const safeLabel = escapeDocumentHtml(label);
  const safeValue = escapeDocumentHtml(reportChartValueLabel(value));
  const chartWidth = 300;
  const chartHeight = 16;
  const fillWidth =
    (Math.min(100, Math.max(0, percent)) / 100) * chartWidth;
  return `<div class="report-chart-row"><span>${safeLabel}</span><svg class="report-chart-bar" viewBox="0 0 ${chartWidth} ${chartHeight}" preserveAspectRatio="none" role="img" aria-label="${safeLabel}: ${safeValue}"><defs><linearGradient id="${gradientId}" x1="0" y1="0" x2="1" y2="0"><stop offset="0%" stop-color="#78f2b0"/><stop offset="100%" stop-color="#d8ad78"/></linearGradient></defs><rect x="0" y="1" width="${chartWidth}" height="14" rx="7" fill="#e5edf3"/><rect x="0" y="1" width="${Math.max(2, fillWidth).toFixed(1)}" height="14" rx="7" fill="url(#${gradientId})"/></svg><em>${safeValue}</em></div>`;
}

function reportChartPolarPoint(
  cx: number,
  cy: number,
  radius: number,
  angleDegrees: number,
): { x: number; y: number } {
  const angleRadians = ((angleDegrees - 90) * Math.PI) / 180;
  return {
    x: cx + radius * Math.cos(angleRadians),
    y: cy + radius * Math.sin(angleRadians),
  };
}

function reportChartPieSlicePath({
  cx,
  cy,
  outerRadius,
  innerRadius,
  startAngle,
  endAngle,
}: {
  cx: number;
  cy: number;
  outerRadius: number;
  innerRadius: number;
  startAngle: number;
  endAngle: number;
}): string {
  const startOuter = reportChartPolarPoint(cx, cy, outerRadius, endAngle);
  const endOuter = reportChartPolarPoint(cx, cy, outerRadius, startAngle);
  const largeArcFlag = endAngle - startAngle <= 180 ? "0" : "1";
  if (innerRadius <= 0) {
    return [
      `M ${cx} ${cy}`,
      `L ${startOuter.x.toFixed(1)} ${startOuter.y.toFixed(1)}`,
      `A ${outerRadius} ${outerRadius} 0 ${largeArcFlag} 0 ${endOuter.x.toFixed(1)} ${endOuter.y.toFixed(1)}`,
      "Z",
    ].join(" ");
  }
  const startInner = reportChartPolarPoint(cx, cy, innerRadius, endAngle);
  const endInner = reportChartPolarPoint(cx, cy, innerRadius, startAngle);
  return [
    `M ${startOuter.x.toFixed(1)} ${startOuter.y.toFixed(1)}`,
    `A ${outerRadius} ${outerRadius} 0 ${largeArcFlag} 0 ${endOuter.x.toFixed(1)} ${endOuter.y.toFixed(1)}`,
    `L ${endInner.x.toFixed(1)} ${endInner.y.toFixed(1)}`,
    `A ${innerRadius} ${innerRadius} 0 ${largeArcFlag} 1 ${startInner.x.toFixed(1)} ${startInner.y.toFixed(1)}`,
    "Z",
  ].join(" ");
}

function agentArkChartToReportHtml(rawJson: string): string {
  try {
    const spec = asRecord(JSON.parse(rawJson));
    const rows = reportChartRows(spec.data).slice(0, 28);
    if (rows.length === 0) return "";
    const categoryKey = reportChartCategoryKey(spec, rows);
    const seriesKeys = reportChartSeriesKeys(spec, rows, categoryKey);
    if (!categoryKey || seriesKeys.length === 0) return "";
    const title = reportChartText(spec.title, "Chart");
    const subtitle = reportChartText(spec.subtitle);
    const kind = reportChartKind(spec);
    const values = rows.flatMap((row) =>
      seriesKeys
        .map((key) => reportChartNumber(row[key]))
        .filter((value): value is number => value != null),
    );
    if (values.length === 0) return "";
    const safeTitle = escapeDocumentHtml(title);
    const safeSubtitle = subtitle
      ? `<div class="report-chart-subtitle">${escapeDocumentHtml(subtitle)}</div>`
      : "";
    const chartId = reportChartStableId(rawJson);

    if (kind === "line" || kind === "area" || kind === "scatter") {
      const width = 760;
      const height = 320;
      const left = 54;
      const right = 24;
      const top = 58;
      const bottom = 52;
      const plotWidth = width - left - right;
      const plotHeight = height - top - bottom;
      const min = Math.min(0, ...values);
      const max = Math.max(...values);
      const span = max === min ? 1 : max - min;
      const xFor = (index: number) =>
        left + (rows.length <= 1 ? plotWidth / 2 : (index / (rows.length - 1)) * plotWidth);
      const yFor = (value: number) => top + plotHeight - ((value - min) / span) * plotHeight;
      const seriesSvg = seriesKeys
        .map((key, seriesIndex) => {
          const points = rows
            .map((row, rowIndex) => {
              const value = reportChartNumber(row[key]);
              return value == null ? null : { x: xFor(rowIndex), y: yFor(value), value };
            })
            .filter((point): point is { x: number; y: number; value: number } => point != null);
          if (points.length === 0) return "";
          const color = reportChartColor(seriesIndex);
          const path = points
            .map((point, index) => `${index === 0 ? "M" : "L"}${point.x.toFixed(1)} ${point.y.toFixed(1)}`)
            .join(" ");
          const area =
            kind === "area"
              ? `<path d="${path} L ${points[points.length - 1].x.toFixed(1)} ${top + plotHeight} L ${points[0].x.toFixed(1)} ${top + plotHeight} Z" fill="${color}" opacity="0.12" />`
              : "";
          const markers = points
            .map(
              (point) =>
                `<circle cx="${point.x.toFixed(1)}" cy="${point.y.toFixed(1)}" r="${kind === "scatter" ? 4.5 : 3.2}" fill="${color}" />`,
            )
            .join("");
          return `${area}<path d="${path}" fill="none" stroke="${color}" stroke-width="2.6" />${markers}`;
        })
        .join("");
      const legend = seriesKeys
        .map(
          (key, index) =>
            `<span><svg viewBox="0 0 10 10" aria-hidden="true"><circle cx="5" cy="5" r="5" fill="${reportChartColor(index)}"/></svg>${escapeDocumentHtml(key)}</span>`,
        )
        .join("");
      const labelStep = Math.max(1, Math.ceil(rows.length / 6));
      const labels = rows
        .map((row, index) => {
          if (index % labelStep !== 0 && index !== rows.length - 1) return "";
          return `<text x="${xFor(index).toFixed(1)}" y="${height - 18}" text-anchor="middle">${escapeDocumentHtml(reportChartText(row[categoryKey], String(index + 1))).slice(0, 16)}</text>`;
        })
        .join("");
      return `<figure class="report-chart"><figcaption>${safeTitle}${safeSubtitle}</figcaption><div class="report-chart-legend">${legend}</div><svg viewBox="0 0 ${width} ${height}" role="img" aria-label="${safeTitle}"><rect x="0" y="0" width="${width}" height="${height}" rx="12" fill="#f8fbfd"/><line x1="${left}" y1="${top + plotHeight}" x2="${width - right}" y2="${top + plotHeight}" stroke="#b6c4cf"/><line x1="${left}" y1="${top}" x2="${left}" y2="${top + plotHeight}" stroke="#b6c4cf"/><text x="${left - 8}" y="${top + 6}" text-anchor="end">${escapeDocumentHtml(reportChartValueLabel(max))}</text><text x="${left - 8}" y="${top + plotHeight}" text-anchor="end">${escapeDocumentHtml(reportChartValueLabel(min))}</text>${seriesSvg}${labels}</svg></figure>`;
    }

    if (kind === "pie" || kind === "doughnut") {
      const valueKey = seriesKeys[0] || "";
      if (!valueKey) return "";
      const slices = rows
        .map((row, index) => ({
          label: reportChartText(row[categoryKey], String(index + 1)),
          value: Math.max(0, reportChartNumber(row[valueKey]) ?? 0),
          color: reportChartColor(index),
        }))
        .filter((slice) => slice.value > 0)
        .slice(0, 12);
      const total = slices.reduce((sum, slice) => sum + slice.value, 0);
      if (total <= 0) return "";
      const width = 760;
      const height = 300;
      const cx = 238;
      const cy = 154;
      const outerRadius = 92;
      const innerRadius = kind === "doughnut" ? 52 : 0;
      let cursor = 0;
      const paths = slices
        .map((slice) => {
          const startAngle = cursor;
          const endAngle = cursor + (slice.value / total) * 360;
          cursor = endAngle;
          return `<path d="${reportChartPieSlicePath({ cx, cy, outerRadius, innerRadius, startAngle, endAngle })}" fill="${slice.color}" stroke="#ffffff" stroke-width="2"/>`;
        })
        .join("");
      const centerLabel =
        kind === "doughnut"
          ? `<text x="${cx}" y="${cy - 4}" text-anchor="middle" font-size="22" font-weight="700" fill="#214636">${escapeDocumentHtml(reportChartValueLabel(total))}</text><text x="${cx}" y="${cy + 18}" text-anchor="middle" font-size="11" fill="#456273">total</text>`
          : "";
      const legendRows = slices
        .map((slice, index) => {
          const y = 66 + index * 18;
          const label = escapeDocumentHtml(slice.label.replace(/\s+/g, " ").slice(0, 30));
          const value = escapeDocumentHtml(reportChartValueLabel(slice.value));
          return `<g transform="translate(456 ${y})"><rect x="0" y="-9" width="11" height="11" rx="3" fill="${slice.color}"/><text x="18" y="0">${label}</text><text x="260" y="0" text-anchor="end">${value}</text></g>`;
        })
        .join("");
      return `<figure class="report-chart"><figcaption>${safeTitle}${safeSubtitle}</figcaption><svg viewBox="0 0 ${width} ${height}" role="img" aria-label="${safeTitle}"><rect x="0" y="0" width="${width}" height="${height}" rx="12" fill="#f8fbfd"/>${paths}${centerLabel}${legendRows}</svg></figure>`;
    }

    const firstSeries = seriesKeys[0];
    const chartRows = rows
      .map((row, index) => ({
        label: reportChartText(row[categoryKey], String(index + 1)),
        value: reportChartNumber(row[firstSeries]),
      }))
      .filter((row): row is { label: string; value: number } => row.value != null);
    const max = Math.max(...chartRows.map((row) => Math.abs(row.value)), 1);
    const bars = chartRows
      .map((row, index) => {
        const width = Math.max(2, Math.abs(row.value / max) * 100);
        return reportChartBarSvg({
          label: row.label,
          value: row.value,
          percent: width,
          gradientId: `report-chart-bar-${chartId}-${index}`,
        });
      })
      .join("");
    return `<figure class="report-chart"><figcaption>${safeTitle}${safeSubtitle}</figcaption><div class="report-chart-bars">${bars}</div></figure>`;
  } catch {
    return "";
  }
}

function markdownToDocumentHtml(markdown: string): string {
  const lines = (markdown || "").replace(/\r\n/g, "\n").split("\n");
  const html: string[] = [];
  let paragraphLines: string[] = [];
  let listMode: "ul" | "ol" | null = null;
  let codeLines: string[] | null = null;
  let codeLanguage = "";

  const closeParagraph = () => {
    if (paragraphLines.length === 0) return;
    html.push(`<p>${renderDocumentInlineMarkdown(paragraphLines.join(" "))}</p>`);
    paragraphLines = [];
  };
  const closeList = () => {
    if (!listMode) return;
    html.push(`</${listMode}>`);
    listMode = null;
  };
  const openList = (mode: "ul" | "ol") => {
    closeParagraph();
    if (listMode === mode) return;
    closeList();
    listMode = mode;
    html.push(`<${mode}>`);
  };
  const closeCode = () => {
    if (!codeLines) return;
    closeParagraph();
    closeList();
    const code = codeLines.join("\n");
    const language = codeLanguage.trim().toLowerCase();
    if (language === "agentark-chart") {
      html.push(
        agentArkChartToReportHtml(code) ||
          `<pre>${escapeDocumentHtml(code)}</pre>`,
      );
    } else {
      html.push(`<pre>${escapeDocumentHtml(code)}</pre>`);
    }
    codeLines = null;
    codeLanguage = "";
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] || "";
    const trimmed = line.trim();
    if (/^```/.test(trimmed)) {
      if (codeLines) {
        closeCode();
      } else {
        closeParagraph();
        closeList();
        codeLanguage = trimmed.replace(/^```/, "").trim();
        codeLines = [];
      }
      continue;
    }
    if (codeLines) {
      codeLines.push(line);
      continue;
    }
    const table = parseMarkdownFallbackTable(lines, index);
    if (table) {
      closeParagraph();
      closeList();
      html.push(markdownTableToDocumentHtml(table.rows));
      index = table.nextIndex - 1;
      continue;
    }
    if (!trimmed) {
      closeParagraph();
      closeList();
      continue;
    }
    const heading = trimmed.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      closeParagraph();
      closeList();
      const level = Math.min(6, Math.max(1, heading[1]?.length ?? 1));
      html.push(
        `<h${level}>${renderDocumentInlineMarkdown((heading[2] || "").trim())}</h${level}>`,
      );
      continue;
    }
    const unordered = trimmed.match(/^[-+*]\s+(.+)$/);
    if (unordered) {
      openList("ul");
      html.push(`<li>${renderDocumentInlineMarkdown(unordered[1])}</li>`);
      continue;
    }
    const ordered = trimmed.match(/^\d+\.\s+(.+)$/);
    if (ordered) {
      openList("ol");
      html.push(`<li>${renderDocumentInlineMarkdown(ordered[1])}</li>`);
      continue;
    }
    paragraphLines.push(trimmed);
  }
  closeCode();
  closeParagraph();
  closeList();
  return html.join("\n");
}

function reportPrintHtml(title: string, markdown: string): string {
  const safeTitle = escapeDocumentHtml(title || "AgentArk report");
  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>${safeTitle}</title>
  <style>
    @page { size: A4; margin: 18mm 16mm; }
    html { background: #f2f0eb; }
    body { color: #17202a; font-family: Aptos, Calibri, Arial, sans-serif; font-size: 11pt; line-height: 1.55; }
    h1, h2, h3, h4, h5, h6 { color: #214636; font-family: Georgia, "Times New Roman", serif; line-height: 1.25; margin: 1.1em 0 0.45em; }
    h1 { font-size: 22pt; border-bottom: 2px solid #214636; padding-bottom: 8pt; }
    h2 { font-size: 16pt; }
    h3 { font-size: 13pt; }
    p { margin: 0 0 0.75em; }
    ul, ol { margin: 0.35em 0 0.8em 1.4em; padding: 0; }
    li { margin: 0.2em 0; }
    table { width: 100%; border-collapse: collapse; margin: 0.8em 0 1em; font-size: 9.5pt; }
    th, td { border: 1px solid #b6c4cf; padding: 5pt 6pt; vertical-align: top; }
    th { background: #214636; color: #fff; font-weight: 700; }
    code, pre { font-family: Consolas, "Courier New", monospace; }
    code { background: #eff5ef; color: #214636; padding: 1pt 3pt; }
    pre { background: #252b36; color: #f8fafc; padding: 8pt; white-space: pre-wrap; }
    a { color: #2f8f68; }
    .report-chart { margin: 16pt 0; border: 1px solid #d2dde5; border-radius: 8pt; background: #fbfdff; padding: 10pt; break-inside: avoid; }
    .report-chart figcaption { color: #214636; font-weight: 700; margin-bottom: 5pt; }
    .report-chart-subtitle { color: #5d6b78; font-size: 9pt; font-weight: 400; margin-top: 2pt; }
    .report-chart svg { display: block; width: 100%; height: auto; }
    .report-chart text { fill: #456273; font-family: Aptos, Calibri, Arial, sans-serif; font-size: 10px; }
    .report-chart-legend { display: flex; flex-wrap: wrap; gap: 8pt; margin: 4pt 0 6pt; color: #385064; font-size: 9pt; }
    .report-chart-legend span { display: inline-flex; align-items: center; gap: 4pt; }
    .report-chart-legend svg { width: 8pt; height: 8pt; flex: 0 0 auto; }
    .report-chart-bars { display: grid; gap: 6pt; }
    .report-chart-row { display: grid; grid-template-columns: minmax(88pt, 1fr) minmax(120pt, 2fr) auto; align-items: center; gap: 8pt; font-size: 9pt; }
    .report-chart-row span { color: #2c465a; }
    .report-chart-bar { display: block; width: 100%; height: 12pt; }
    .report-chart-row em { color: #214636; font-style: normal; font-weight: 700; }
    .report-page { box-sizing: border-box; max-width: 184mm; min-height: 260mm; margin: 24px auto; padding: 22mm 18mm; background: #fffefb; box-shadow: 0 16px 54px rgba(21, 31, 44, 0.18); }
    @media print {
      * { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
      html { background: #fff; }
      body { margin: 0; }
      .report-page { max-width: none; min-height: 0; margin: 0; padding: 0; box-shadow: none; }
      table, pre, blockquote { break-inside: avoid; }
      h1, h2, h3 { break-after: avoid; }
    }
  </style>
</head>
<body>
<main class="report-page">
${markdownToDocumentHtml(markdown)}
</main>
</body>
</html>`;
}

function documentFileStem(value: string): string {
  return (
    (value || "research")
      .replace(/[^\w.-]+/g, "_")
      .replace(/^_+|_+$/g, "")
      .toLowerCase()
      .slice(0, 96) || "research"
  );
}

function formatExecutionPlanStatusLabel(status: string): string {
  const normalized = str(status, "").trim().toLowerCase();
  if (!normalized) return "pending";
  return humanizeMachineLabel(normalized);
}

function buildExecutionPlanExportSection(
  plan: ExecutionPlanState | null,
  planFailure: string,
  traceId: string,
): string[] {
  const lines: string[] = ["## Execution Plan", ""];
  if (plan?.steps.length) {
    if (plan.summary.trim()) {
      lines.push(plan.summary.trim(), "");
    }
    plan.steps.forEach((step, index) => {
      const title = step.title.trim() || `Step ${index + 1}`;
      const status = formatExecutionPlanStatusLabel(step.status);
      lines.push(`${index + 1}. **${title}** (${status})`);
      if (step.description.trim()) {
        lines.push(`   ${step.description.trim()}`);
      }
      step.substeps.forEach((substep) => {
        const subTitle = substep.title.trim() || `Substep ${substep.id}`;
        const subStatus = formatExecutionPlanStatusLabel(substep.status);
        const subDescription = substep.description.trim();
        lines.push(`   - ${subTitle}${subStatus ? ` (${subStatus})` : ""}`);
        if (subDescription) {
          lines.push(`     ${subDescription}`);
        }
      });
      lines.push("");
    });
    return lines;
  }
  if (planFailure) {
    lines.push(planFailure, "");
    return lines;
  }
  lines.push(
    traceId
      ? "No pre-execution plan was captured in the trace for this reply."
      : "No trace id was attached to this reply, so plan details were unavailable.",
    "",
  );
  return lines;
}

function researchReportMetaLabel(report: ResearchReportPreview): string {
  const parts = [report.kind === "deep" ? "Deep research report" : "Research report"];
  if (report.sourceCount > 0) {
    parts.push(
      `${report.sourceCount} source${report.sourceCount === 1 ? "" : "s"}`,
    );
  }
  if (report.tableCount > 0) {
    parts.push(
      `${report.tableCount} table${report.tableCount === 1 ? "" : "s"}`,
    );
  }
  if (report.chartCount > 0) {
    parts.push(
      `${report.chartCount} chart${report.chartCount === 1 ? "" : "s"}`,
    );
  }
  if (report.keyFindingCount > 0) {
    parts.push(
      `${report.keyFindingCount} key finding${report.keyFindingCount === 1 ? "" : "s"}`,
    );
  }
  if (report.openQuestionCount > 0) {
    parts.push(
      `${report.openQuestionCount} open question${report.openQuestionCount === 1 ? "" : "s"}`,
    );
  }
  if (report.contradictionCount > 0) {
    parts.push(
      `${report.contradictionCount} contradiction${report.contradictionCount === 1 ? "" : "s"}`,
    );
  }
  return parts.join(" | ");
}

const CHAT_ATTACHMENT_EXTENSIONS = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "csv",
  "tsv",
  "xml",
  "yaml",
  "yml",
  "pdf",
  "docx",
  "log",
  "html",
  "htm",
]);

const CHAT_VISUAL_ATTACHMENT_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "tif",
  "tiff",
  "svg",
]);

function chatAttachmentExtension(file: File): string {
  const name = (file.name || "").trim();
  const dotIdx = name.lastIndexOf(".");
  return dotIdx >= 0 ? name.slice(dotIdx + 1).toLowerCase() : "";
}

function isVisualChatAttachment(file: File): boolean {
  const contentType = (file.type || "").trim().toLowerCase();
  return (
    contentType.startsWith("image/") ||
    CHAT_VISUAL_ATTACHMENT_EXTENSIONS.has(chatAttachmentExtension(file))
  );
}

function isKnowledgeChatAttachment(file: File): boolean {
  return CHAT_ATTACHMENT_EXTENSIONS.has(chatAttachmentExtension(file));
}

function splitSupportedChatAttachments(files: File[]): {
  accepted: File[];
  rejected: string[];
} {
  const accepted: File[] = [];
  const rejected: string[] = [];
  for (const file of files) {
    const name = (file.name || "").trim();
    if (isKnowledgeChatAttachment(file) || isVisualChatAttachment(file)) {
      accepted.push(file);
    } else {
      rejected.push(name || "unnamed-file");
    }
  }
  return { accepted, rejected };
}

function formatDurationClock(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m ${secs}s`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

function errMessage(error: unknown): string {
  const normalize = (raw: string): string => {
    const msg = (raw || "").trim();
    if (!msg) return "Request failed";
    if (msg.startsWith("{") && msg.endsWith("}")) {
      try {
        const parsed = JSON.parse(msg) as Record<string, unknown>;
        const nested =
          str(parsed.error, "").trim() || str(parsed.message, "").trim();
        if (nested) return nested;
      } catch {
        // Fall through to raw message.
      }
    }
    return msg;
  };

  if (error instanceof Error) return normalize(error.message);
  if (typeof error === "string") return normalize(error);
  return "Request failed";
}

function extractPreviewImageUrl(text: string): string {
  const source = text || "";
  if (!source) return "";
  const appPreviewMatch = source.match(/!\[App Preview\]\(([^)\s]+)\)/i);
  if (appPreviewMatch?.[1]) return appPreviewMatch[1].trim();
  const genericMatch = source.match(/!\[[^\]]*\]\(([^)\s]+)\)/);
  if (genericMatch?.[1]) return genericMatch[1].trim();
  return "";
}

function toAbsoluteAppUrl(pathOrUrl: string, baseOrigin: string): string {
  const value = (pathOrUrl || "").trim();
  if (!value) return "";
  if (looksLikeUrl(value)) return value;
  const base = (baseOrigin || "").trim().replace(/\/+$/, "");
  if (!base) return value;
  if (value.startsWith("/")) return `${base}${value}`;
  return `${base}/${value}`;
}

function looksLikeUuid(value: string): boolean {
  const v = (value || "").trim();
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    v,
  );
}

function looksLikeIsoTimestamp(value: string): boolean {
  const v = (value || "").trim();
  if (!/^\d{4}-\d{2}-\d{2}T/.test(v)) return false;
  const dt = new Date(v);
  return !Number.isNaN(dt.getTime());
}

function looksLikeIsoDateOnly(value: string): boolean {
  const v = (value || "").trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(v)) return false;
  const dt = new Date(`${v}T00:00:00`);
  return !Number.isNaN(dt.getTime());
}

function formatTimestampForHumans(value: string): {
  label: string;
  tooltip: string;
} {
  const meta = formatUiDateTimeMeta(value, { fallback: value || "-" });
  return { label: meta.label, tooltip: meta.tip };
}

/** Format a trace step time string to local human-readable.
 *  Input: ISO timestamp like "2026-03-16T09:02:40Z" or "2026-03-16T09:02:40Z (1396ms)"
 *  Output: "12 Mar, 2:32 PM (1396ms)" - local time with optional duration
 */
function formatTraceStepTime(raw: string): string {
  if (!raw) return "";
  // Split off optional "(Xms)" suffix
  const match = raw.match(/^(.+?)(\s*\(\d+ms\))?$/);
  if (!match) return raw;
  const isopart = match[1].trim();
  const durationPart = match[2]?.trim() || "";
  const dt = new Date(isopart);
  if (Number.isNaN(dt.getTime())) return raw;
  const time = formatUiTime(dt, { fallback: raw, includeSeconds: true });
  return durationPart ? `${time} ${durationPart}` : time;
}

function formatChatTimestamp(value: string): {
  label: string;
  tooltip: string;
} {
  const absolute = formatUiDateTimeMeta(value, { fallback: value || "-" });
  const relative = formatUiRelativeDateTimeMeta(value, {
    fallback: value || "-",
  });
  return {
    label: `${absolute.label} | ${relative.label}`,
    tooltip: absolute.tip || relative.tip,
  };
}

/** Format any ISO timestamp string into a human-readable relative label with absolute tooltip. */
function humanTs(raw: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(raw, { fallback: "-" });
}

function formatDurationFromSeconds(value: unknown): string {
  const total = num(value, -1);
  if (total < 0) return "-";
  const sec = Math.floor(total);
  if (sec < 60) return `${sec}s`;
  const mins = Math.floor(sec / 60);
  const remSec = sec % 60;
  if (mins < 60) return remSec > 0 ? `${mins}m ${remSec}s` : `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  if (hours < 24) return remMins > 0 ? `${hours}h ${remMins}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `${days}d ${remHours}h` : `${days}d`;
}

function charsLabel(value: unknown): string {
  const amount = num(value, -1);
  if (amount < 0) return "-";
  return `${Math.round(amount).toLocaleString()} chars`;
}

function ChatRunMetricsCard({
  metrics,
  keyPrefix,
}: {
  metrics: ChatRunMetrics;
  keyPrefix: string;
}) {
  const [open, setOpen] = useState(false);
  const inputTokens = Math.max(0, Math.round(metrics.inputTokens ?? 0));
  const outputTokens = Math.max(0, Math.round(metrics.outputTokens ?? 0));
  const cachedTokens = Math.max(0, Math.round(metrics.cachedPromptTokens ?? 0));
  const cacheCreationTokens = Math.max(
    0,
    Math.round(metrics.cacheCreationPromptTokens ?? 0),
  );
  const explicitTotal = Math.max(0, Math.round(metrics.totalTokens ?? 0));
  const totalTokens = Math.max(explicitTotal, inputTokens + outputTokens);
  const durationMs = metrics.durationMs ?? null;
  const ttftMs = metrics.timeToFirstTokenMs ?? null;
  // Model/LLM latency awareness: tier the provider response time so slowness is
  // visible at a glance. Thresholds are deliberately simple and tunable.
  const modelLatencyMs = metrics.modelLatencyMs ?? null;
  const latencyTier =
    modelLatencyMs == null
      ? null
      : modelLatencyMs <= 8000
        ? "good"
        : modelLatencyMs <= 20000
          ? "fair"
          : "slow";
  const latencyLabel =
    latencyTier === "good" ? "Good" : latencyTier === "fair" ? "Fair" : "Slow";
  if (totalTokens <= 0) return null;
  const inputPct = totalTokens > 0 ? (inputTokens / totalTokens) * 100 : 0;
  const outputPct = totalTokens > 0 ? (outputTokens / totalTokens) * 100 : 0;
  const cachedPctOfInput =
    inputTokens > 0 ? Math.min(100, (cachedTokens / inputTokens) * 100) : 0;
  const cacheBadge =
    cachedTokens > 0 ? `${cachedPctOfInput.toFixed(0)}% cached` : null;
  const formatMs = (ms: number) =>
    ms >= 1000 ? `${(ms / 1000).toFixed(2)}s` : `${Math.round(ms)}ms`;
  return (
    <>
      <button
        type="button"
        className="chat-run-metrics-trigger"
        onClick={() => setOpen(true)}
        aria-label={`Show token usage: ${totalTokens.toLocaleString()} tokens`}
        data-key={keyPrefix}
      >
        <span className="chat-run-metrics-trigger-number">
          {totalTokens.toLocaleString()}
        </span>
        <span className="chat-run-metrics-trigger-label">tokens</span>
        {cacheBadge ? (
          <span className="chat-run-metrics-trigger-badge">{cacheBadge}</span>
        ) : null}
        {latencyTier && modelLatencyMs != null ? (
          <span
            className={`chat-run-latency-chip tone-${latencyTier}`}
            aria-label={`Model latency: ${latencyLabel}, ${formatMs(modelLatencyMs)}`}
          >
            <span className="chat-run-latency-dot" aria-hidden="true" />
            {latencyLabel} · {formatMs(modelLatencyMs)}
          </span>
        ) : null}
        <ChevronRightRoundedIcon
          className="chat-run-metrics-trigger-chevron"
          fontSize="inherit"
          aria-hidden="true"
        />
      </button>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        maxWidth="xs"
        fullWidth
        slotProps={{ paper: { className: "chat-run-metrics-dialog-paper" } }}
      >
        <DialogTitle className="chat-run-metrics-dialog-title">
          <span className="chat-run-metrics-dialog-total">
            {totalTokens.toLocaleString()}
          </span>
          <span className="chat-run-metrics-dialog-total-suffix">
            tokens · this turn
          </span>
        </DialogTitle>
        <DialogContent className="chat-run-metrics-dialog-content">
          <div className="chat-run-metrics-stack-bar" aria-hidden="true">
            <div
              className="chat-run-metrics-stack-input"
              style={{ width: `${inputPct}%` }}
            />
            <div
              className="chat-run-metrics-stack-output"
              style={{ width: `${outputPct}%` }}
            />
          </div>
          <div className="chat-run-metrics-row">
            <span className="chat-run-metrics-swatch chat-run-metrics-swatch-input" />
            <span className="chat-run-metrics-row-label">Input</span>
            <span className="chat-run-metrics-row-value">
              {inputTokens.toLocaleString()}
            </span>
          </div>
          <div className="chat-run-metrics-row">
            <span className="chat-run-metrics-swatch chat-run-metrics-swatch-output" />
            <span className="chat-run-metrics-row-label">Output</span>
            <span className="chat-run-metrics-row-value">
              {outputTokens.toLocaleString()}
            </span>
          </div>
          {cachedTokens > 0 ? (
            <>
              <div className="chat-run-metrics-section-divider" />
              <div className="chat-run-metrics-cache-header">
                <span className="chat-run-metrics-cache-label">Cache hit</span>
                <span className="chat-run-metrics-cache-percent">
                  {cachedPctOfInput.toFixed(0)}%
                </span>
              </div>
              <div className="chat-run-metrics-cache-bar" aria-hidden="true">
                <div
                  className="chat-run-metrics-cache-fill"
                  style={{ width: `${cachedPctOfInput}%` }}
                />
              </div>
              <div className="chat-run-metrics-row chat-run-metrics-row-sub">
                <span className="chat-run-metrics-row-label">Cached prompt</span>
                <span className="chat-run-metrics-row-value">
                  {cachedTokens.toLocaleString()}
                </span>
              </div>
              {cacheCreationTokens > 0 ? (
                <div className="chat-run-metrics-row chat-run-metrics-row-sub">
                  <span className="chat-run-metrics-row-label">Cache write</span>
                  <span className="chat-run-metrics-row-value">
                    {cacheCreationTokens.toLocaleString()}
                  </span>
                </div>
              ) : null}
            </>
          ) : null}
          {ttftMs != null || durationMs != null || modelLatencyMs != null ? (
            <>
              <div className="chat-run-metrics-section-divider" />
              {latencyTier && modelLatencyMs != null ? (
                <div className="chat-run-metrics-row chat-run-metrics-row-sub">
                  <span className="chat-run-metrics-row-label">Model latency</span>
                  <span
                    className={`chat-run-metrics-row-value chat-run-latency-value tone-${latencyTier}`}
                  >
                    {latencyLabel} · {formatMs(modelLatencyMs)}
                  </span>
                </div>
              ) : null}
              {ttftMs != null ? (
                <div className="chat-run-metrics-row chat-run-metrics-row-sub">
                  <span className="chat-run-metrics-row-label">
                    Time to first token
                  </span>
                  <span className="chat-run-metrics-row-value">
                    {formatMs(ttftMs)}
                  </span>
                </div>
              ) : null}
              {durationMs != null ? (
                <div className="chat-run-metrics-row chat-run-metrics-row-sub">
                  <span className="chat-run-metrics-row-label">Total time</span>
                  <span className="chat-run-metrics-row-value">
                    {formatMs(durationMs)}
                  </span>
                </div>
              ) : null}
            </>
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  );
}

function promptProposalStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved") return "success";
  if (normalized === "rejected") return "error";
  return "warning";
}

function promptCanarySafetyStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "auto_reverted" || normalized === "disabled_by_user")
    return "success";
  if (normalized === "kept_active") return "default";
  return "warning";
}

function humanizeStatusLabel(value: string): string {
  return humanizeMachineLabel(value, "-");
}

function promptProposalRiskColor(
  risk: string,
): "default" | "success" | "warning" | "error" {
  const normalized = risk.trim().toLowerCase();
  if (normalized === "high") return "error";
  if (normalized === "medium") return "warning";
  if (normalized === "low") return "success";
  return "default";
}

function boolLabelForKey(
  key: string,
  value: boolean,
): { label: string; color: "success" | "warning" | "default" } {
  const k = (key || "").toLowerCase();
  if (k.includes("enabled"))
    return {
      label: value ? "Enabled" : "Disabled",
      color: value ? "success" : "warning",
    };
  if (k.includes("active"))
    return {
      label: value ? "Active" : "Inactive",
      color: value ? "success" : "warning",
    };
  if (k.includes("connected"))
    return {
      label: value ? "Connected" : "Not connected",
      color: value ? "success" : "warning",
    };
  return { label: value ? "Yes" : "No", color: value ? "success" : "default" };
}

function DataTable({
  rows,
  columns,
}: {
  rows: JsonRecord[];
  columns: string[];
}) {
  return (
    <TableContainer className="table-shell">
      <Table size="small">
        <TableHead>
          <TableRow>
            {columns.map((column) => (
              <TableCell key={column}>{column}</TableCell>
            ))}
          </TableRow>
        </TableHead>
        <TableBody>
          {rows.map((row, index) => (
            <TableRow key={str(row.id, String(index))}>
              {columns.map((column) => (
                <TableCell key={`${index}-${column}`}>
                  <Typography variant="caption" sx={{ whiteSpace: "pre-wrap" }}>
                    {(() => {
                      const v = row[column];
                      const out = formatCompactValue(v);
                      return <span title={out.tooltip || ""}>{out.text}</span>;
                    })()}
                  </Typography>
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

function KeyValuePanel({
  title,
  data,
  emptyLabel,
  maxRows,
}: {
  title: string;
  data: JsonRecord;
  emptyLabel?: string;
  maxRows?: number;
}) {
  const entries = Object.entries(data || {});
  const shown = entries.slice(0, maxRows ?? 14);
  return (
    <Box
      sx={{
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-255-255-255-080)",
        background: "var(--ui-rgba-255-255-255-025)",
        p: 1.25,
      }}
    >
      <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
        {title}
      </Typography>
      <Stack spacing={0} sx={{ mt: 0.9 }}>
        {shown.length === 0 ? (
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
            }}
          >
            {emptyLabel || "No details available."}
          </Typography>
        ) : (
          shown.map(([k, v], index) => {
            const out = formatCompactValue(v);
            const keyLower = (k || "").toLowerCase();
            const renderValue = () => {
              if (typeof v === "string" && looksLikeUrl(v)) {
                const trimmed = v.trim();
                const label =
                  trimmed.length > 54 ? `${trimmed.slice(0, 54)}...` : trimmed;
                return (
                  <Typography
                    variant="body2"
                    sx={{ wordBreak: "break-all" }}
                    title={trimmed}
                  >
                    <a
                      href={trimmed}
                      target="_blank"
                      rel="noreferrer"
                      style={{ color: "inherit", textDecoration: "underline" }}
                    >
                      {label}
                    </a>
                  </Typography>
                );
              }
              if (
                typeof v === "string" &&
                (looksLikeIsoTimestamp(v) ||
                  looksLikeIsoDateOnly(v) ||
                  keyLower.endsWith("_at") ||
                  keyLower.endsWith("_date") ||
                  keyLower.includes("timestamp"))
              ) {
                const t =
                  looksLikeIsoDateOnly(v) || keyLower.endsWith("_date")
                    ? {
                        label: formatUiDateOnly(v, { fallback: "-" }),
                        tooltip: formatUiDateOnly(v, {
                          fallback: "-",
                          includeYear: true,
                        }),
                      }
                    : formatTimestampForHumans(v);
                return (
                  <span className="chat-value-pill" title={t.tooltip}>
                    {t.label}
                  </span>
                );
              }
              if (typeof v === "boolean") {
                const b = boolLabelForKey(k, v);
                return (
                  <span className={`chat-value-pill tone-${b.color}`}>
                    {b.label}
                  </span>
                );
              }
              if (typeof v === "number" && Number.isFinite(v)) {
                if (keyLower.includes("ms") || keyLower.includes("duration")) {
                  return (
                    <span className="chat-value-pill">
                      {Math.round(v)} ms
                    </span>
                  );
                }
                if (
                  keyLower.includes("count") ||
                  keyLower.includes("total") ||
                  keyLower.includes("remaining")
                ) {
                  return <span className="chat-value-pill">{String(v)}</span>;
                }
              }
              if (
                typeof v === "string" &&
                (looksLikeUuid(v) ||
                  keyLower.endsWith("_id") ||
                  keyLower === "id")
              ) {
                const trimmed = v.trim();
                const label =
                  trimmed.length > 22
                    ? `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`
                    : trimmed;
                return (
                  <button
                    type="button"
                    className="chat-value-pill is-clickable"
                    title={trimmed}
                    onClick={async () => {
                      try {
                        await navigator.clipboard.writeText(trimmed);
                      } catch {
                        // ignore
                      }
                    }}
                  >
                    {label}
                  </button>
                );
              }
              return (
                <Typography
                  variant="body2"
                  sx={{
                    minWidth: 0,
                    flex: "1 1 auto",
                    wordBreak: "break-word",
                  }}
                  title={out.tooltip || ""}
                >
                  {out.text}
                </Typography>
              );
            };
            return (
              <Box
                key={k}
                sx={{
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    md: "160px minmax(0, 1fr)",
                  },
                  gap: { xs: 0.35, md: 1.1 },
                  py: 0.9,
                  borderTop:
                    index === 0 ? "none" : "1px solid var(--ui-rgba-255-255-255-060)",
                }}
              >
                <Typography
                  variant="caption"
                  sx={{
                    color: "var(--ui-rgba-188-198-212-680)",
                    minWidth: 0,
                  }}
                >
                  {k}
                </Typography>
                {renderValue()}
              </Box>
            );
          })
        )}
        {entries.length > shown.length ? (
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              pt: 0.9,
            }}
          >
            {entries.length - shown.length} more field(s) not shown.
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
}

type ChatStarterCategoryId =
  | "build"
  | "watch"
  | "background"
  | "research"
  | "swarm"
  | "browser"
  | "security"
  | "advanced";

type ChatStarterTabId = "all" | Exclude<ChatStarterCategoryId, "advanced">;

type ChatStarterExample = {
  id: string;
  title: string;
  summary: string;
  prompt: string;
  category: ChatStarterCategoryId;
  defaultVisible?: boolean;
  deepResearch?: boolean;
};

type ChatComposerPrefillRequest = {
  text: string;
  seq: number;
  browserProfileContext?: JsonRecord | null;
};

type ChatComposerPrefillPayload = {
  text: string;
  browser_profile_context?: JsonRecord | null;
};

function normalizeChatComposerPrefillPayload(
  stored: string,
): ChatComposerPrefillPayload | null {
  const raw = stored.trim();
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    const record = asRecord(parsed);
    const text = str(record.text, "").trimEnd();
    if (text) {
      const context = asRecord(record.browser_profile_context);
      return {
        text,
        browser_profile_context:
          Object.keys(context).length > 0 ? context : null,
      };
    }
  } catch {
    // Legacy prefill values were plain strings.
  }

  const legacyProfile = raw.match(/Use the saved browser login profile "([^"]+)"/);
  if (legacyProfile?.[1]) {
    const taskMatch = raw.match(/\nTask:\s*([\s\S]*)$/);
    const task = taskMatch?.[1]?.trimStart() || "";
    return {
      text: `Browser profile: ${legacyProfile[1]}\n\nTask: ${task}`,
      browser_profile_context: null,
    };
  }
  return { text: stored, browser_profile_context: null };
}

const CHAT_STARTER_CATEGORY_META: Record<
  ChatStarterCategoryId,
  { label: string; description: string }
> = {
  build: {
    label: "Build & deploy",
    description:
      "Ship apps, dashboards, pages, and local deployments from chat.",
  },
  watch: {
    label: "Watchers & dashboards",
    description:
      "Monitor live sources, refresh feeds, and publish useful views.",
  },
  background: {
    label: "Background sessions",
    description:
      "Keep reminders, follow-ups, and monitoring in one durable session.",
  },
  research: {
    label: "Deep research",
    description:
      "Run slower, source-backed analysis with a reviewable research plan.",
  },
  swarm: {
    label: "Swarm",
    description:
      "Split structured work across multiple agents and return one answer.",
  },
  browser: {
    label: "Playwright browser automation",
    description:
      "Drive the browser, pause for user choices, and inspect pages step by step.",
  },
  security: {
    label: "Access & permissions",
    description:
      "Check granted tools, connected access, and fallback behavior without overreaching.",
  },
  advanced: {
    label: "Advanced",
    description:
      "Internal operator prompts for Pulse, Sentinel, Evolve, trace diagnostics, and system inspection.",
  },
};

const CHAT_STARTER_CATEGORY_ICON: Record<ChatStarterCategoryId, LucideIcon> = {
  build: CubeIcon,
  watch: Eye,
  background: ImageIcon,
  research: Search,
  swarm: Network,
  browser: Globe,
  security: Lock,
  advanced: Sparkles,
};

const starterTabIcon = (id: ChatStarterTabId): LucideIcon =>
  id === "all" ? Sparkles : CHAT_STARTER_CATEGORY_ICON[id];

const CHAT_STARTER_CATEGORY_ORDER: ChatStarterCategoryId[] = [
  "build",
  "watch",
  "background",
  "research",
  "swarm",
  "browser",
  "security",
];

const CHAT_STARTER_TAB_ORDER: ChatStarterTabId[] = [
  "all",
  "build",
  "watch",
  "background",
  "research",
  "swarm",
  "browser",
  "security",
];

const CHAT_STARTER_TAB_LABELS: Record<ChatStarterTabId, string> = {
  all: "Suggested",
  build: "Build",
  watch: "Watchers",
  background: "Background",
  research: "Research",
  swarm: "Swarm",
  browser: "Browser",
  security: "Access",
};

const CHAT_STARTER_EXAMPLES: ChatStarterExample[] = [
  {
    id: "finance-dashboard",
    title: "Build a finance tracker",
    summary:
      "Create a personal finance dashboard with budgets, charts, CSV import/export, SQLite, and a local link.",
    prompt:
      "Build me a personal finance tracker dashboard with budgets, charts, CSV import/export, dark mode, mobile support and deploy it locally with a link.",
    category: "build",
    defaultVisible: true,
  },
  {
    id: "deploy-github-repo",
    title: "Deploy a GitHub repo locally",
    summary:
      "Clone a public repo, run it, and make it available from this machine.",
    prompt:
      "Deploy this GitHub repo locally and make it available with a working link: https://github.com/mdn/beginner-html-site-styled",
    category: "build",
  },
  {
    id: "hn-ai-dashboard",
    title: "Monitor Hacker News AI stories",
    summary:
      "Check top stories every few minutes, summarize strong AI items, and keep a live dashboard updated.",
    prompt:
      "Monitor Hacker News top stories every 5 minutes. For stories with 100+ points that mention AI, LLM, or agents, summarize them in 2 sentences, save them to a feed page, and deploy a live dashboard with links.",
    category: "watch",
  },
  {
    id: "arxiv-rl-feed",
    title: "Track arXiv ML papers",
    summary:
      "Watch recent ML, reinforcement learning, and time-series papers and keep a public feed fresh.",
    prompt:
      "Build a static page that refreshes every 10 seconds, pulls the latest arXiv papers, and highlights machine learning, reinforcement learning, time series modeling, and novel approaches.",
    category: "watch",
  },
  {
    id: "pricing-monitor",
    title: "Track model pricing changes",
    summary:
      "Keep an ongoing session that checks major AI pricing and notifies me only if something changes.",
    prompt:
      "Keep monitoring OpenAI, Anthropic, Google AI, and Perplexity pricing in the background. Check twice a day and notify me in app only if pricing or plan tiers change.",
    category: "background",
    defaultVisible: true,
  },
  {
    id: "reply-tracker",
    title: "Track replies in one session",
    summary:
      "Follow a thread, keep reminders together, and summarize status in the same background session.",
    prompt:
      "Track replies from Acme about the partnership proposal in the background. Keep all follow-ups, reminders, and status in one session.",
    category: "background",
  },
  {
    id: "ai-environmental-impact-research",
    title: "Research AI's environmental footprint",
    summary:
      "Produce a source-backed view on AI infrastructure's energy, water, carbon, and policy tradeoffs.",
    prompt:
      "Research the strategic question of whether aggressively expanding AI research investment, frontier-model infrastructure, and compute capacity over the next 5–10 years is environmentally sustainable. Cover electricity demand, cooling water use, carbon emissions, grid and siting impact, supply-chain footprint, mitigation options (efficiency, renewable colocation, model right-sizing), regional comparisons, and realistic policy responses.",
    category: "research",
    defaultVisible: true,
    deepResearch: true,
  },
  {
    id: "next-feature-research",
    title: "Compare what to build next",
    summary:
      "Evaluate product directions with market reasoning, delivery risk, customer impact, and one recommendation.",
    prompt:
      "Compare whether we should build invoice OCR, approval workflows, or audit trails next. I want market reasoning, delivery risk, customer impact, and a final recommendation.",
    category: "research",
    deepResearch: true,
  },
  {
    id: "approval-workflows-launch",
    title: "Plan a launch with multiple agents",
    summary:
      "Break buyer pain points, positioning, rollout, risks, and metrics into a single operator-ready plan.",
    prompt:
      "Use multiple agents for this. We run a B2B SaaS for finance teams and need a launch plan for approval workflows, including buyer pain points, positioning, rollout, risks, and success metrics.",
    category: "swarm",
  },
  {
    id: "trial-to-paid-swarm",
    title: "Improve trial-to-paid conversion",
    summary:
      "Use swarm to find likely causes, propose experiments, define metrics, and call out implementation risks.",
    prompt:
      "Use swarm. We need a plan to improve trial-to-paid conversion for our SaaS. Analyze likely causes, propose experiments, define metrics, and identify implementation risks.",
    category: "swarm",
  },
  {
    id: "wikipedia-pause",
    title: "Drive the browser and pause",
    summary:
      "Navigate a site, stop at the right point, and wait for my choice before continuing.",
    prompt:
      "Open https://www.wikipedia.org, search for OpenAI, go to the article, and when you get there stop and ask me whether I should inspect the History section or the Products section.",
    category: "browser",
  },
  {
    id: "hn-login",
    title: "Open a login flow",
    summary:
      "Go to a login page and handle the browser steps directly inside the task.",
    prompt: "Go to https://news.ycombinator.com/login and log in for me.",
    category: "browser",
  },
  {
    id: "browser-research",
    title: "Research with browser evidence",
    summary:
      "Open sources in the browser, collect evidence, and return a cited summary.",
    prompt:
      "Use the browser to research three reliable sources about the current state of local-first AI agents, capture the useful evidence, and summarize the tradeoffs.",
    category: "browser",
  },
  {
    id: "browser-form-fill",
    title: "Fill a form",
    summary:
      "Navigate to a form, pause for private fields, and submit only after review.",
    prompt:
      "Open this form URL: <paste-url>. Fill the non-sensitive fields from the details I provide, pause for anything private, and ask me before submitting.",
    category: "browser",
  },
  {
    id: "browser-scrape-table",
    title: "Extract page data",
    summary:
      "Open a page, inspect its structure, and return clean tabular data.",
    prompt:
      "Open this page: <paste-url>. Extract the main table or repeated list into structured rows with source links.",
    category: "browser",
  },
  {
    id: "browser-login-needed",
    title: "Login-needed workflow",
    summary:
      "Start a browser task and hand control back when authentication is needed.",
    prompt:
      "Open this app: <paste-url>. If login is required, pause for browser handoff, then continue the workflow after I finish signing in.",
    category: "browser",
  },
  {
    id: "browser-verify-deploy",
    title: "Verify a deployed app",
    summary:
      "Open a deployed app, check console errors, screenshots, and key flows.",
    prompt:
      "Open this deployed app URL: <paste-url>. Verify the main user flow, check for console errors, capture a screenshot, and report anything broken.",
    category: "browser",
  },
  {
    id: "google-workspace-tools",
    title: "Check Google Workspace access",
    summary:
      "Inspect which Google Workspace tools are actually available right now.",
    prompt: "What tools do you currently have for Google Workspace?",
    category: "security",
  },
  {
    id: "drive-roadmap",
    title: "Find a Drive file if granted",
    summary:
      "Search Drive for a file by name and only succeed if access is really available.",
    prompt: "Find my Drive file named roadmap.",
    category: "security",
  },
  {
    id: "places-fallback",
    title: "Check fallback routing",
    summary:
      "Use public search when a connected Places integration is not available.",
    prompt: "List flight schools near Madhyamgram, Kolkata.",
    category: "security",
  },
  {
    id: "arkpulse-latest-run",
    title: "What was Pulse latest run?",
    summary: "Inspect the latest Pulse run and summarize the main result.",
    prompt: "What was Pulse latest run?",
    category: "advanced",
  },
  {
    id: "recent-evolution",
    title: "What does recent evolution say?",
    summary:
      "Summarize recent evolution activity and how the current state looks.",
    prompt: "What does recent evolution say and how does it look?",
    category: "advanced",
  },
  {
    id: "sentinel-observations",
    title: "Show recent sentinel observations",
    summary:
      "Inspect recent sentinel observations and call out anything worth attention.",
    prompt:
      "Show me recent sentinel observations and anything worth attention.",
    category: "advanced",
  },
  {
    id: "latest-trace",
    title: "Inspect the latest trace",
    summary: "Read the newest trace and tell me what failed or looks odd.",
    prompt: "Show me the latest trace and tell me what failed or looks odd.",
    category: "advanced",
  },
  {
    id: "last-5-traces",
    title: "Compare the last 5 traces",
    summary:
      "Check recent execution traces and identify which tool is failing most.",
    prompt:
      "Look at the last 5 execution traces and tell me which tool is failing most.",
    category: "advanced",
  },
  {
    id: "duplicate-reminders",
    title: "Find duplicate reminder tasks",
    summary: "Inspect recent reminder tasks and tell me if duplicates exist.",
    prompt: "Find recent reminder tasks and tell me if there are duplicates.",
    category: "advanced",
  },
  {
    id: "arkpulse-running",
    title: "Check whether Pulse is running",
    summary:
      "Inspect AgentArk directly and determine whether Pulse is running right now.",
    prompt:
      "Without guessing, inspect AgentArk and tell me whether Pulse is running right now.",
    category: "advanced",
  },
  {
    id: "trace-by-id",
    title: "Inspect a trace by ID",
    summary:
      "Take a specific trace ID and summarize the failure path precisely.",
    prompt:
      "Inspect the trace with id <paste-id-from-trace-page> and summarize the failure path.",
    category: "advanced",
  },
  {
    id: "schedule-task-logs",
    title: "Find failed schedule_task logs",
    summary:
      "Use the live database if needed and avoid guessing table names while tracing failures.",
    prompt:
      "Use the live database if needed, but do not guess table names: find the newest failed operational logs related to schedule_task.",
    category: "advanced",
  },
  {
    id: "evolution-awaiting-review",
    title: "Find learning awaiting review",
    summary: "Inspect what evolution learned recently that still needs review.",
    prompt:
      "What has evolution learned recently that is still awaiting review?",
    category: "advanced",
  },
  {
    id: "arkpulse-vs-traces",
    title: "Compare Pulse warnings with traces",
    summary:
      "Check whether recent Pulse warnings line up with recent trace failures.",
    prompt:
      "Compare recent Pulse warnings with recent trace failures and tell me if they line up.",
    category: "advanced",
  },
];

const CHAT_STARTER_DEFAULT_EXAMPLES = CHAT_STARTER_EXAMPLES.filter(
  (example) => example.defaultVisible,
);

const CHAT_STARTER_ADVANCED_EXAMPLES = CHAT_STARTER_EXAMPLES.filter(
  (example) => example.category === "advanced",
);
const CHAT_SECRET_WARNING =
  "Never paste secrets, API keys, passwords, or sensitive data into normal chat. Use the secure credential form shown in this conversation.";

const ChatComposerInput = memo(function ChatComposerInput({
  attachedFilesCount,
  composerLocked,
  deepResearchDisabled,
  deepResearchEnabled,
  isStoppingStream,
  isStreaming,
  onAttachFiles,
  onStopStreaming,
  onSubmit,
  onToggleDeepResearch,
  placeholder,
  prefillRequest,
}: {
  attachedFilesCount: number;
  composerLocked: boolean;
  deepResearchDisabled: boolean;
  deepResearchEnabled: boolean;
  isStoppingStream: boolean;
  isStreaming: boolean;
  onAttachFiles: () => void;
  onStopStreaming: () => void;
  onSubmit: (draft: string) => Promise<boolean>;
  onToggleDeepResearch: () => void;
  placeholder: string;
  prefillRequest: ChatComposerPrefillRequest | null;
}) {
  const [draft, setDraft] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const lastAppliedPrefillSeqRef = useRef<number | null>(null);

  const resizeComposerTextarea = (el: HTMLTextAreaElement | null) => {
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 150)}px`;
  };

  useLayoutEffect(() => {
    resizeComposerTextarea(textareaRef.current);
  }, [draft]);

  useEffect(() => {
    if (!prefillRequest) return;
    if (prefillRequest.seq === lastAppliedPrefillSeqRef.current) return;
    setDraft(prefillRequest.text);
    lastAppliedPrefillSeqRef.current = prefillRequest.seq;
    if (typeof window === "undefined") return;
    window.requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.focus();
      const end = prefillRequest.text.length;
      try {
        el.setSelectionRange(end, end);
      } catch {
        // Ignore selection errors on browsers that do not support it here.
      }
      resizeComposerTextarea(el);
    });
  }, [prefillRequest]);

  const submitCurrentDraft = async () => {
    if (
      composerLocked ||
      isStreaming ||
      (!draft.trim() && attachedFilesCount === 0)
    ) {
      return;
    }
    const accepted = await onSubmit(draft);
    if (!accepted) return;
    setDraft("");
    const el = textareaRef.current;
    if (el) {
      el.style.height = "auto";
    }
  };
  return (
    <>
      <textarea
        ref={textareaRef}
        className="chat-composer-textarea"
        placeholder={placeholder}
        aria-label="Message"
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          resizeComposerTextarea(e.target);
        }}
        onKeyDown={(e) => {
          if (
            e.key === "Enter" &&
            !e.shiftKey &&
            !e.nativeEvent.isComposing
          ) {
            e.preventDefault();
            void submitCurrentDraft();
          }
        }}
        rows={1}
        disabled={composerLocked}
      />
      <div className="chat-composer-actions">
        <div className="chat-composer-inline-tools" aria-label="Composer tools">
          <Tooltip
            title={
              attachedFilesCount > 0
                ? `${attachedFilesCount} file${attachedFilesCount === 1 ? "" : "s"} attached`
                : "Upload files"
            }
          >
            <span>
              <IconButton
                type="button"
                size="small"
                className={`chat-composer-tool-btn${attachedFilesCount > 0 ? " is-active" : ""}`}
                aria-label={
                  attachedFilesCount > 0
                    ? `${attachedFilesCount} file${attachedFilesCount === 1 ? "" : "s"} attached`
                    : "Upload files"
                }
                onClick={onAttachFiles}
                disabled={isStreaming || composerLocked}
              >
                <AttachFileRoundedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
          <Tooltip
            title={
              deepResearchDisabled
                ? "Deep research is unavailable for this state"
                : "Deep research: cited multi-source research"
            }
          >
            <span>
              <IconButton
                type="button"
                size="small"
                className={`chat-composer-tool-btn chat-composer-research-btn${
                  deepResearchEnabled && !deepResearchDisabled ? " is-active" : ""
                }`}
                onClick={() => {
                  if (isStreaming || composerLocked || deepResearchDisabled) return;
                  onToggleDeepResearch();
                }}
                disabled={isStreaming || composerLocked || deepResearchDisabled}
                aria-label={
                  deepResearchEnabled && !deepResearchDisabled
                    ? "Disable deep research"
                    : "Enable deep research"
                }
                aria-pressed={deepResearchEnabled && !deepResearchDisabled}
              >
                <TravelExploreRoundedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
        </div>
        {isStreaming && TASK_CANCEL_CONTROLS_ENABLED ? (
          <IconButton
            size="small"
            className="chat-composer-stop-btn"
            aria-label={isStoppingStream ? "Stopping run" : "Stop run"}
            disabled={isStoppingStream}
            onClick={onStopStreaming}
          >
            <StopRoundedIcon fontSize="small" />
          </IconButton>
        ) : (
          <IconButton
            id="chat-send-btn"
            size="small"
            className="chat-composer-send-btn"
            disabled={
              isStreaming ||
              composerLocked ||
              (!draft.trim() && attachedFilesCount === 0)
            }
            onClick={() => {
              void submitCurrentDraft();
            }}
          >
            <ArrowUpwardRoundedIcon fontSize="small" />
          </IconButton>
        )}
      </div>
    </>
  );
});

function ChatPageInner({
  autoRefresh,
  isActive,
  onNavigateToView,
}: {
  autoRefresh: boolean;
  isActive: boolean;
  onNavigateToView?: (view: string, replace?: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const chatAutoRefresh = autoRefresh && isActive;
  const [conversationId, setConversationId] = useState<string | null>(
    () => readChatRouteConversationId(),
  );
  const [draftChatActive, setDraftChatActive] = useState(() =>
    readChatDraftMode(),
  );
  const [deepResearchEnabled, setDeepResearchEnabled] = useState(false);
  const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatNotice, setChatNotice] = useState<string | null>(null);
  const [chatCredentialValues, setChatCredentialValues] = useState<
    Record<string, string>
  >({});
  const [chatCredentialError, setChatCredentialError] = useState<string | null>(
    null,
  );
  const [chatCredentialDialogOpen, setChatCredentialDialogOpen] =
    useState(false);
  const autoOpenedCredentialPromptFingerprintRef = useRef("");
  const [
    dismissedCredentialPromptConversationIds,
    setDismissedCredentialPromptConversationIds,
  ] = useState<Set<string>>(() => new Set());
  const [isStreaming, setIsStreaming] = useState(false);
  const [liveRunStreamOpen, setLiveRunStreamOpen] = useState(false);
  const [isStoppingStream, setIsStoppingStream] = useState(false);
  const [pendingUserMessage, setPendingUserMessage] = useState<string | null>(
    null,
  );
  const [failedUserMessage, setFailedUserMessage] = useState<string | null>(
    null,
  );
  const [streamingResponse, setStreamingResponse] = useState("");
  const [streamingResponseChoices, setStreamingResponseChoices] = useState<
    ChatClarificationChoice[]
  >([]);
  const [streamingRunMetrics, setStreamingRunMetrics] =
    useState<ChatRunMetrics | null>(null);
  const [streamingSteps, setStreamingSteps] = useState<JsonRecord[]>([]);
  const [executionPlan, setExecutionPlan] = useState<ExecutionPlanState | null>(
    null,
  );
  const [planConfirmation, setPlanConfirmation] =
    useState<PlanConfirmationState | null>(null);
  const [expandedPlanSteps, setExpandedPlanSteps] = useState<Set<string>>(
    () => new Set(),
  );
  const togglePlanStepExpansion = (key: string) => {
    if (!key) return;
    setExpandedPlanSteps((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };
  const [executionPlanFailure, setExecutionPlanFailure] = useState("");
  const [executionPlanExpanded, setExecutionPlanExpanded] = useState(false);
  const [streamingProgressMessages, setStreamingProgressMessages] = useState<
    string[]
  >([]);
  const [
    completedProgressMessagesByConversation,
    setCompletedProgressMessagesByConversation,
  ] = useState<Record<string, { messages: string[]; beforeMessageId: string }>>(
    {},
  );
  const [streamTraceOpen, setStreamTraceOpen] = useState(false);
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window !== "undefined"
      ? window.innerWidth
      : CHAT_INLINE_ACTIVITY_MIN_WIDTH,
  );
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  // True once the user explicitly closes the console mid-run; suppresses the
  // stream-driven auto-open (revealLiveFilesConsole / delegation progress) for
  // the rest of the current run. Reset at run start so the next run can auto-open.
  const workspaceUserClosedRef = useRef(false);
  const [activeStepId, setActiveStepId] = useState<string | null>(null);
  const handleActivateStep = useCallback((id: string | null) => {
    setActiveStepId(id);
    if (id) setWorkspaceOpen(true);
  }, []);
  const closeWorkspacePanel = useCallback(() => {
    workspaceUserClosedRef.current = true;
    setWorkspaceOpen(false);
  }, []);
  const [conversationSidebarOpen, setConversationSidebarOpen] = useState(false);
  const [starterActiveTab, setStarterActiveTab] =
    useState<ChatStarterTabId>("all");
  const [starterAdvancedExpanded, setStarterAdvancedExpanded] = useState(false);
  const [emptyEarlyAccessNoticeDismissed, setEmptyEarlyAccessNoticeDismissed] =
    useState(() => readEarlyAccessNoticeDismissed());
  const [conversationPage, setConversationPage] = useState(0);
  const [activityAutoFollow, setActivityAutoFollow] = useState(true);
  const [activityDetailRow, setActivityDetailRow] =
    useState<ActivityTimelineCard | null>(null);
  const [expandedActivityPayloads, setExpandedActivityPayloads] = useState<
    Set<string>
  >(new Set());
  const [expandedTranscriptActions, setExpandedTranscriptActions] = useState<
    Set<string>
  >(new Set());
  const [secretHelperMode, setSecretHelperMode] = useState<"reuse" | "manual">(
    "reuse",
  );
  const [secretHelperKey, setSecretHelperKey] = useState("OPENAI_API_KEY");
  const [secretHelperValue, setSecretHelperValue] = useState("");
  const [secretHelperBusy, setSecretHelperBusy] = useState(false);
  const [isDragOverChat, setIsDragOverChat] = useState(false);
  const [deployedFiles, setDeployedFiles] = useState<WorkspaceFileEntry[]>([]);
  const [streamedWorkspaceApp, setStreamedWorkspaceApp] =
    useState<JsonRecord | null>(null);
  const [liveFileWrites, setLiveFileWrites] = useState<
    Record<string, LiveFileWriteState>
  >({});
  const [streamPhaseStatus, setStreamPhaseStatus] =
    useState<StreamPhaseStatus | null>(null);
  // Live reasoning preview. Aggregated from structural
  // `reasoning_delta` events on the SSE step pipeline (see
  // `handleStreamToolProgress`). Updates are buffered so high-frequency model
  // reasoning does not re-render the whole chat pane on every SSE delta.
  const [reasoningStream, setReasoningStream] = useState<{
    phase: string;
    content: string;
  } | null>(null);
  const [codeViewerOpen, setCodeViewerOpen] = useState(false);
  const [codeViewerFileIdx, setCodeViewerFileIdx] = useState(0);
  const [selectedSnippetId, setSelectedSnippetId] = useState<string | null>(
    null,
  );
  const [selectedSnippetOverride, setSelectedSnippetOverride] =
    useState<WorkspaceSnippetEntry | null>(null);
  const [previewDialogOpen, setPreviewDialogOpen] = useState(false);
  const [researchReportDialog, setResearchReportDialog] =
    useState<ResearchReportDialogState | null>(null);
  const [submittedClarificationChoices, setSubmittedClarificationChoices] =
    useState<Record<string, boolean>>({});
  const [traceStepsById, setTraceStepsById] = useState<
    Record<string, JsonRecord[]>
  >({});
  const [traceLoadingById, setTraceLoadingById] = useState<
    Record<string, boolean>
  >({});
  const [traceErrorById, setTraceErrorById] = useState<Record<string, string>>(
    {},
  );
  const [lastRunSteps, setLastRunSteps] = useState<JsonRecord[]>([]);
  const [conversationMenuAnchor, setConversationMenuAnchor] =
    useState<HTMLElement | null>(null);
  const [conversationMenuTarget, setConversationMenuTarget] =
    useState<JsonRecord | null>(null);
  const [postDeleteConversationFallback, setPostDeleteConversationFallback] =
    useState<{
      deletedId: string;
      preferredId: string | null;
    } | null>(null);
  const [pendingRunSnapshot, setPendingRunSnapshot] =
    useState<ChatPendingRunSnapshot | null>(() => loadChatPendingRunSnapshot());
  const [backgroundRunSnapshots, setBackgroundRunSnapshots] =
    useState<ChatPendingRunSnapshotMap>(() => loadChatBackgroundRunSnapshots());
  const [composerPrefillRequest, setComposerPrefillRequest] =
    useState<ChatComposerPrefillRequest | null>(null);
  const [composerBrowserProfileContext, setComposerBrowserProfileContext] =
    useState<JsonRecord | null>(null);
  const activeRunUsesLiveStream =
    liveRunStreamOpen && (isStreaming || pendingRunSnapshot !== null);
  const chatBackgroundRefresh =
    (!activeRunUsesLiveStream && (isStreaming || pendingRunSnapshot !== null)) ||
    Object.keys(backgroundRunSnapshots).length > 0;
  const chatPassiveRefresh = chatAutoRefresh && !activeRunUsesLiveStream;
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const dragDepthRef = useRef(0);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const streamLockRef = useRef(false);
  const streamAbortRef = useRef<AbortController | null>(null);
  const streamGenerationRef = useRef(0);
  const backgroundDetachGenerationsRef = useRef<Set<number>>(new Set());
  const draftChatActiveRef = useRef(draftChatActive);
  const activeChatTaskIdRef = useRef<string | null>(null);
  const stopRequestedRef = useRef(false);
  const recentSendRef = useRef<{ fingerprint: string; at: number } | null>(
    null,
  );
  const streamingStepsRef = useRef<JsonRecord[]>([]);
  const streamingStepKeySeqRef = useRef(1);
  const queuedStreamingStepsRef = useRef<JsonRecord[] | null>(null);
  const streamingStepsFlushTimerRef = useRef<number | null>(null);
  // Holds either a ready snapshot or a lazy producer; producers let the
  // per-tick effect skip step compaction entirely — it only runs once, at
  // debounced flush time, instead of ~12x/sec during streaming.
  const pendingRunSnapshotStoreRef = useRef<
    ChatPendingRunSnapshot | (() => ChatPendingRunSnapshot | null) | null
  >(pendingRunSnapshot);
  const pendingRunSnapshotStoreTimerRef = useRef<number | null>(null);
  const workspaceSnapshotStoreRef = useRef<ChatWorkspaceSnapshot | null>(null);
  const workspaceSnapshotStoreTimerRef = useRef<number | null>(null);
  const workspaceActivityRef = useRef<HTMLDivElement | null>(null);
  const conversationIdRef = useRef<string | null>(conversationId);
  const pendingRunSnapshotRef = useRef<ChatPendingRunSnapshot | null>(
    pendingRunSnapshot,
  );
  const streamingResponseRef = useRef(streamingResponse);
  const latestRunEventSeqRef = useRef(pendingRunSnapshot?.lastRunSeq ?? 0);
  const previousInlineSidebarsRef = useRef({
    conversations: viewportWidth >= CHAT_INLINE_CONVERSATIONS_MIN_WIDTH,
    activity: viewportWidth >= CHAT_INLINE_ACTIVITY_MIN_WIDTH,
  });
  const pendingFileReadPathRef = useRef("");
  const pendingFileWritePathRef = useRef("");
  const lastProgressBubbleCategoryRef = useRef("");
  const lastProgressBubbleAtRef = useRef(0);
  const reasoningProgressByPhaseRef = useRef<Record<string, string>>({});
  const reasoningActivityEmitRef = useRef<Record<string, number>>({});
  const reasoningPreviewDraftRef = useRef<{
    phase: string;
    content: string;
  } | null>(null);
  const reasoningPreviewFlushTimerRef = useRef<number | null>(null);
  const streamedWorkspaceAppRef = useRef<JsonRecord | null>(null);
  const streamingTokenBufferRef = useRef("");
  const streamingTokenFlushTimerRef = useRef<number | null>(null);
  const lastWorkspaceRestoreSeedRef = useRef("");
  const lastWorkspaceActivityRestoreSeedRef = useRef("");
  const reattachedRunIdRef = useRef("");
  const reconciledInterruptedRunIdRef = useRef("");
  const conversationOffset = conversationPage * CHAT_CONVERSATIONS_PAGE_SIZE;
  const queueComposerPrefill = useCallback((prefill: ChatComposerPrefillPayload) => {
    const browserProfileContext = prefill.browser_profile_context ?? null;
    setComposerBrowserProfileContext(browserProfileContext);
    setComposerPrefillRequest((prev) => ({
      text: prefill.text,
      browserProfileContext,
      seq: (prev?.seq ?? 0) + 1,
    }));
  }, []);
  const setDraftChatMode = (active: boolean) => {
    draftChatActiveRef.current = active;
    setDraftChatActive(active);
    writeChatDraftMode(active);
  };

  useEffect(() => {
    if (typeof window === "undefined") return;
    let stored: string | null = null;
    try {
      stored = window.sessionStorage.getItem(ARKREFLECT_COMPOSER_PREFILL_STORAGE_KEY);
    } catch {
      stored = null;
    }
    if (!stored) return;
    try {
      window.sessionStorage.removeItem(ARKREFLECT_COMPOSER_PREFILL_STORAGE_KEY);
    } catch {
      // ignore â€” best-effort cleanup
    }
    queueComposerPrefill({ text: stored, browser_profile_context: null });
    // queueComposerPrefill is stable (uses setState updater); intentional one-shot mount effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const consumeComposerPrefill = () => {
      let stored: string | null = null;
      try {
        stored = window.sessionStorage.getItem(CHAT_COMPOSER_PREFILL_STORAGE_KEY);
      } catch {
        stored = null;
      }
      if (!stored) return;
      try {
        window.sessionStorage.removeItem(CHAT_COMPOSER_PREFILL_STORAGE_KEY);
      } catch {
        // Best-effort cleanup.
      }
      const prefill = normalizeChatComposerPrefillPayload(stored);
      if (!prefill) return;
      queueComposerPrefill(prefill);
    };
    consumeComposerPrefill();
    window.addEventListener(CHAT_COMPOSER_PREFILL_EVENT, consumeComposerPrefill);
    return () => {
      window.removeEventListener(
        CHAT_COMPOSER_PREFILL_EVENT,
        consumeComposerPrefill,
      );
    };
  }, [queueComposerPrefill]);

  const cancelStreamingTokenFlush = () => {
    if (
      typeof window !== "undefined" &&
      streamingTokenFlushTimerRef.current !== null
    ) {
      window.clearTimeout(streamingTokenFlushTimerRef.current);
    }
    streamingTokenFlushTimerRef.current = null;
  };
  const flushStreamingTokenBuffer = () => {
    const buffered = streamingTokenBufferRef.current;
    if (!buffered) return;
    streamingTokenBufferRef.current = "";
    setStreamingResponse((prev) => {
      const next = `${prev}${buffered}`;
      streamingResponseRef.current = next;
      return next;
    });
  };
  const scheduleStreamingTokenFlush = () => {
    if (typeof window === "undefined") {
      flushStreamingTokenBuffer();
      return;
    }
    if (streamingTokenFlushTimerRef.current !== null) return;
    streamingTokenFlushTimerRef.current = window.setTimeout(() => {
      streamingTokenFlushTimerRef.current = null;
      flushStreamingTokenBuffer();
    }, 80);
  };
  const appendStreamingToken = (token: string): string => {
    const appendText = streamingResponseAppendText(
      streamingResponseRef.current,
      token,
    );
    if (!appendText) return "";
    streamingTokenBufferRef.current += appendText;
    streamingResponseRef.current += appendText;
    scheduleStreamingTokenFlush();
    return appendText;
  };
  const recordRunEventSeq = (payload: unknown) => {
    const seq = num(asRecord(payload).seq, 0);
    if (seq > latestRunEventSeqRef.current) {
      latestRunEventSeqRef.current = Math.floor(seq);
    }
  };
  const setStreamingResponseNow = (next: string) => {
    cancelStreamingTokenFlush();
    streamingTokenBufferRef.current = "";
    streamingResponseRef.current = next;
    setStreamingResponse(next);
  };
  const setLiveRunStreamOpenNow = (open: boolean) => {
    setLiveRunStreamOpen((prev) => (prev === open ? prev : open));
  };
  const flushReasoningPreview = () => {
    reasoningPreviewFlushTimerRef.current = null;
    const next = reasoningPreviewDraftRef.current;
    if (!next) return;
    setReasoningStream((prev) =>
      prev?.phase === next.phase && prev.content === next.content
        ? prev
        : next,
    );
  };
  const scheduleReasoningPreviewFlush = (immediate = false) => {
    if (typeof window === "undefined" || immediate) {
      if (
        typeof window !== "undefined" &&
        reasoningPreviewFlushTimerRef.current !== null
      ) {
        window.clearTimeout(reasoningPreviewFlushTimerRef.current);
      }
      flushReasoningPreview();
      return;
    }
    if (reasoningPreviewFlushTimerRef.current !== null) return;
    const pendingLength = reasoningPreviewDraftRef.current?.content.length ?? 0;
    const flushDelay =
      pendingLength > 120_000
        ? 750
        : pendingLength > 60_000
          ? 400
          : CHAT_REASONING_PREVIEW_FLUSH_MS;
    reasoningPreviewFlushTimerRef.current = window.setTimeout(
      flushReasoningPreview,
      flushDelay,
    );
  };
  const setReasoningPreviewBuffered = (
    phase: string,
    content: string,
    immediate = false,
  ) => {
    reasoningPreviewDraftRef.current = {
      phase,
      content,
    };
    scheduleReasoningPreviewFlush(immediate);
  };
  const clearReasoningPreview = () => {
    if (
      typeof window !== "undefined" &&
      reasoningPreviewFlushTimerRef.current !== null
    ) {
      window.clearTimeout(reasoningPreviewFlushTimerRef.current);
    }
    reasoningPreviewFlushTimerRef.current = null;
    reasoningPreviewDraftRef.current = null;
    setReasoningStream(null);
  };
  const flushQueuedStreamingSteps = () => {
    if (
      typeof window !== "undefined" &&
      streamingStepsFlushTimerRef.current !== null
    ) {
      window.clearTimeout(streamingStepsFlushTimerRef.current);
    }
    streamingStepsFlushTimerRef.current = null;
    const queued = queuedStreamingStepsRef.current;
    queuedStreamingStepsRef.current = null;
    if (!queued) return;
    setStreamingSteps(queued);
  };
  const scheduleStreamingStepsFlush = () => {
    if (typeof window === "undefined") {
      flushQueuedStreamingSteps();
      return;
    }
    if (streamingStepsFlushTimerRef.current !== null) return;
    streamingStepsFlushTimerRef.current = window.setTimeout(() => {
      streamingStepsFlushTimerRef.current = null;
      const queued = queuedStreamingStepsRef.current;
      queuedStreamingStepsRef.current = null;
      if (queued) setStreamingSteps(queued);
    }, CHAT_STREAMING_STEP_FLUSH_MS);
  };
  const setStreamingStepsNow = (next: JsonRecord[]) => {
    if (
      typeof window !== "undefined" &&
      streamingStepsFlushTimerRef.current !== null
    ) {
      window.clearTimeout(streamingStepsFlushTimerRef.current);
    }
    streamingStepsFlushTimerRef.current = null;
    queuedStreamingStepsRef.current = null;
    const sanitized = sanitizeActivityStepsForUi(next);
    streamingStepsRef.current = sanitized;
    setStreamingSteps(sanitized);
  };
  const resolvePendingRunSnapshotSource = (
    value: ChatPendingRunSnapshot | (() => ChatPendingRunSnapshot | null) | null,
  ): ChatPendingRunSnapshot | null =>
    typeof value === "function" ? value() : value;
  const scheduleChatPendingRunSnapshotStore = (
    snapshot: ChatPendingRunSnapshot | (() => ChatPendingRunSnapshot | null) | null,
  ) => {
    pendingRunSnapshotStoreRef.current = snapshot;
    if (typeof window === "undefined") {
      storeChatPendingRunSnapshot(resolvePendingRunSnapshotSource(snapshot));
      return;
    }
    if (!snapshot) {
      if (pendingRunSnapshotStoreTimerRef.current !== null) {
        window.clearTimeout(pendingRunSnapshotStoreTimerRef.current);
      }
      pendingRunSnapshotStoreTimerRef.current = null;
      storeChatPendingRunSnapshot(null);
      return;
    }
    if (pendingRunSnapshotStoreTimerRef.current !== null) return;
    pendingRunSnapshotStoreTimerRef.current = window.setTimeout(() => {
      pendingRunSnapshotStoreTimerRef.current = null;
      storeChatPendingRunSnapshot(
        resolvePendingRunSnapshotSource(pendingRunSnapshotStoreRef.current),
      );
    }, CHAT_PENDING_RUN_SNAPSHOT_FLUSH_MS);
  };
  const storeChatPendingRunSnapshotNow = (
    snapshot: ChatPendingRunSnapshot | null,
  ) => {
    pendingRunSnapshotStoreRef.current = snapshot;
    if (
      typeof window !== "undefined" &&
      pendingRunSnapshotStoreTimerRef.current !== null
    ) {
      window.clearTimeout(pendingRunSnapshotStoreTimerRef.current);
    }
    pendingRunSnapshotStoreTimerRef.current = null;
    storeChatPendingRunSnapshot(snapshot);
  };
  const flushChatPendingRunSnapshotStore = () => {
    if (
      typeof window !== "undefined" &&
      pendingRunSnapshotStoreTimerRef.current !== null
    ) {
      window.clearTimeout(pendingRunSnapshotStoreTimerRef.current);
    }
    pendingRunSnapshotStoreTimerRef.current = null;
    storeChatPendingRunSnapshot(
      resolvePendingRunSnapshotSource(pendingRunSnapshotStoreRef.current),
    );
  };
  const scheduleChatWorkspaceSnapshotStore = (
    snapshot: ChatWorkspaceSnapshot,
  ) => {
    workspaceSnapshotStoreRef.current = snapshot;
    if (typeof window === "undefined") {
      storeChatWorkspaceSnapshot(snapshot);
      return;
    }
    if (workspaceSnapshotStoreTimerRef.current !== null) return;
    workspaceSnapshotStoreTimerRef.current = window.setTimeout(() => {
      workspaceSnapshotStoreTimerRef.current = null;
      const queued = workspaceSnapshotStoreRef.current;
      if (queued) storeChatWorkspaceSnapshot(queued);
    }, CHAT_WORKSPACE_SNAPSHOT_FLUSH_MS);
  };
  const flushChatWorkspaceSnapshotStore = () => {
    if (
      typeof window !== "undefined" &&
      workspaceSnapshotStoreTimerRef.current !== null
    ) {
      window.clearTimeout(workspaceSnapshotStoreTimerRef.current);
    }
    workspaceSnapshotStoreTimerRef.current = null;
    const queued = workspaceSnapshotStoreRef.current;
    if (queued) storeChatWorkspaceSnapshot(queued);
  };
  const storeChatWorkspaceSnapshotNow = (snapshot: ChatWorkspaceSnapshot) => {
    if (
      typeof window !== "undefined" &&
      workspaceSnapshotStoreTimerRef.current !== null
    ) {
      window.clearTimeout(workspaceSnapshotStoreTimerRef.current);
    }
    workspaceSnapshotStoreTimerRef.current = null;
    workspaceSnapshotStoreRef.current = snapshot;
    storeChatWorkspaceSnapshot(snapshot);
  };
  const canInlineConversationSidebar =
    viewportWidth >= CHAT_INLINE_CONVERSATIONS_MIN_WIDTH;
  const canInlineWorkspacePanel =
    viewportWidth >= CHAT_INLINE_ACTIVITY_MIN_WIDTH;
  const scopedConversationPath = useMemo(
    () =>
      `/conversations?sidebar=1&limit=${CHAT_CONVERSATIONS_PAGE_SIZE}&offset=${conversationOffset}`,
    [conversationOffset],
  );

  const convQ = useQuery({
    queryKey: ["chat-conversations", conversationPage],
    queryFn: () => api.rawGet(scopedConversationPath),
    refetchInterval:
      chatPassiveRefresh || chatBackgroundRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: chatBackgroundRefresh,
  });

  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    let frame = 0;
    const syncWidth = () => {
      frame = 0;
      setViewportWidth(window.innerWidth);
    };
    const handleResize = () => {
      if (frame !== 0) return;
      frame = window.requestAnimationFrame(syncWidth);
    };
    syncWidth();
    window.addEventListener("resize", handleResize, { passive: true });
    return () => {
      if (frame !== 0) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", handleResize);
    };
  }, []);

  useEffect(() => {
    const previous = previousInlineSidebarsRef.current;
    if (
      previous.conversations &&
      !canInlineConversationSidebar &&
      conversationSidebarOpen
    ) {
      setConversationSidebarOpen(false);
    }
    if (previous.activity && !canInlineWorkspacePanel && workspaceOpen) {
      setWorkspaceOpen(false);
    }
    previousInlineSidebarsRef.current = {
      conversations: canInlineConversationSidebar,
      activity: canInlineWorkspacePanel,
    };
  }, [
    canInlineConversationSidebar,
    canInlineWorkspacePanel,
    conversationSidebarOpen,
    workspaceOpen,
  ]);

  const conversationsPayload = asRecord(convQ.data);
  const conversations = pickRecords(conversationsPayload, "conversations");
  const starredConversations = pickRecords(
    conversationsPayload,
    "starred_conversations",
  ).slice(0, CHAT_STARRED_LIMIT);
  const orderedSidebarConversationIds = useMemo(
    () =>
      [...starredConversations, ...conversations]
        .map((conv) => str(conv.id, "").trim())
        .filter(Boolean),
    [starredConversations, conversations],
  );
  const sidebarConversationIds = useMemo(
    () => new Set(orderedSidebarConversationIds),
    [orderedSidebarConversationIds],
  );
  const conversationListTotal = Math.max(
    0,
    num(conversationsPayload.total, conversations.length),
  );
  const conversationListLimit = Math.max(
    1,
    num(conversationsPayload.limit, CHAT_CONVERSATIONS_PAGE_SIZE),
  );
  const conversationPageCount = Math.max(
    1,
    Math.ceil(conversationListTotal / conversationListLimit),
  );
  const conversationPageLabel = `${Math.min(conversationPage + 1, conversationPageCount)}/${conversationPageCount}`;
  const selectedConversationQ = useQuery({
    queryKey: ["chat-conversation", conversationId],
    queryFn: () =>
      api.rawGet(`/conversations/${encodeURIComponent(conversationId || "")}`),
    enabled: !!conversationId,
    refetchInterval: chatPassiveRefresh ? REFRESH_MS : false,
  });
  const selectedConversation = useMemo(() => {
    const sidebarMatch =
      [...starredConversations, ...conversations].find(
        (conv) => str(conv.id, "") === conversationId,
      ) ?? null;
    const fetched = asRecord(selectedConversationQ.data);
    if (str(fetched.id, "").trim()) {
      return sidebarMatch ? { ...sidebarMatch, ...fetched } : fetched;
    }
    return sidebarMatch;
  }, [
    starredConversations,
    conversations,
    conversationId,
    selectedConversationQ.data,
  ]);
  const backgroundSessionsQ = useQuery({
    queryKey: ["chat-background-sessions"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: chatPassiveRefresh ? REFRESH_MS : false,
  });
  const backgroundSessions = useMemo(
    () =>
      pickRecords(backgroundSessionsQ.data, "sessions").filter((session) =>
        isBackgroundSessionVisibleInUi(
          session as unknown as BackgroundSessionSummary,
        ),
    ),
    [backgroundSessionsQ.data],
  );
  const activeConversationSession = useMemo(
    () =>
      backgroundSessions.find((session) => {
        const sessionConversationId = str(session.conversation_id, "").trim();
        const status = str(session.status, "").trim().toLowerCase();
        return (
          !!conversationId &&
          sessionConversationId === conversationId &&
          !["completed", "failed", "cancelled"].includes(status)
        );
      }) || null,
    [backgroundSessions, conversationId],
  );
  const workingConversationIds = useMemo(() => {
    const ids = new Set<string>();
    const addWorkingId = (id: string, fallback: string) => {
      const normalized = id.trim();
      ids.add(normalized || fallback);
    };
    if (
      pendingRunSnapshot &&
      (pendingRunSnapshot.phase ?? "running") === "running"
    ) {
      addWorkingId(
        str(pendingRunSnapshot?.conversationId, ""),
        "__active_pending_chat__",
      );
    }
    Object.values(backgroundRunSnapshots).forEach((snapshot, idx) => {
      if ((snapshot.phase ?? "running") !== "running") return;
      addWorkingId(
        str(snapshot.conversationId, ""),
        `__background_pending_chat_${idx}__`,
      );
    });
    backgroundSessions.forEach((session, idx) => {
      const status = str(session.status, "").trim().toLowerCase();
      if (!["active", "working", "running"].includes(status)) return;
      const sessionConversationId = str(session.conversation_id, "").trim();
      if (!sessionConversationId) return;
      addWorkingId(sessionConversationId, `__background_session_${idx}__`);
    });
    return ids;
  }, [backgroundRunSnapshots, backgroundSessions, pendingRunSnapshot]);
  const workingChatCount = workingConversationIds.size;
  const selectedMessageCount = num(selectedConversation?.message_count, 0);
  const selectedConversationUpdatedAtMs = Date.parse(
    str(selectedConversation?.updated_at, ""),
  );
  const recentlyTouchedEmptyConversation =
    selectedMessageCount === 0 &&
    Number.isFinite(selectedConversationUpdatedAtMs) &&
    Date.now() - selectedConversationUpdatedAtMs < 10 * 60 * 1000;
  const hasPendingSnapshotForConversation =
    !!conversationId && pendingRunSnapshot?.conversationId === conversationId;
  const currentBackgroundRunSnapshot = conversationId
    ? backgroundRunSnapshots[conversationId]
    : null;
  const currentConversationHasActiveRun =
    (hasPendingSnapshotForConversation &&
      (pendingRunSnapshot?.phase ?? "running") === "running") ||
    (Boolean(currentBackgroundRunSnapshot) &&
      (currentBackgroundRunSnapshot?.phase ?? "running") === "running");
  const isStreamingForCurrentConversation =
    (isStreaming || liveRunStreamOpen) && hasPendingSnapshotForConversation;
  const liveStreamOpenForCurrentConversation =
    liveRunStreamOpen && hasPendingSnapshotForConversation;
  const shouldPollMessages =
    !!conversationId &&
    ((!liveStreamOpenForCurrentConversation &&
      (isStreamingForCurrentConversation || hasPendingSnapshotForConversation)) ||
      recentlyTouchedEmptyConversation);
  const shouldPreparePersistedThread =
    !!conversationId && (isActive || shouldPollMessages);
  const messagesQ = useQuery({
    queryKey: ["chat-messages", conversationId],
    queryFn: async () =>
      sanitizeChatMessagesPayloadForUi(await api.rawGet(
        `/conversations/${encodeURIComponent(conversationId || "")}/messages?limit=100`,
      )),
    enabled: !!conversationId && (isActive || shouldPollMessages),
    refetchInterval: shouldPollMessages
      ? 2000
      : chatPassiveRefresh
        ? REFRESH_MS
        : false,
    refetchIntervalInBackground: shouldPollMessages,
  });
  const chatCredentialPromptQ = useQuery({
    queryKey: ["chat-credential-prompt", conversationId],
    queryFn: () =>
      api.rawGet(
        `/chat/credential-prompt?conversation_id=${encodeURIComponent(conversationId || "")}`,
      ),
    enabled:
      !!conversationId &&
      (isActive || shouldPollMessages || isStreamingForCurrentConversation),
    refetchInterval: shouldPollMessages
      ? 2000
      : chatPassiveRefresh
        ? REFRESH_MS
        : false,
    refetchIntervalInBackground: shouldPollMessages,
  });

  const messages = useMemo(
    () =>
      shouldPreparePersistedThread
        ? pickRecords(messagesQ.data, "messages")
        : [],
    [shouldPreparePersistedThread, messagesQ.data],
  );
  const activeConversationMessageCount = Math.max(
    selectedMessageCount,
    messages.length,
  );
  const activeConversationActivityLoading = Boolean(
    conversationId && (messagesQ.isLoading || selectedConversationQ.isLoading),
  );
  const selectedConversationHasUnloadedMessages = Boolean(
    conversationId && selectedMessageCount > 0 && messages.length === 0,
  );
  const selectedConversationAwaitingPersistedMessages = Boolean(
    conversationId &&
      messages.length === 0 &&
      (activeConversationActivityLoading ||
        selectedConversationHasUnloadedMessages ||
        recentlyTouchedEmptyConversation),
  );
  const deepResearchDisabled = Boolean(
    conversationId &&
      (activeConversationActivityLoading || activeConversationMessageCount > 0),
  );
  useEffect(() => {
    if (deepResearchDisabled && deepResearchEnabled) {
      setDeepResearchEnabled(false);
    }
  }, [deepResearchDisabled, deepResearchEnabled]);

  useEffect(() => {
    const visibleTraceIds = messages
      .map((message) => str(message.trace_id, "").trim())
      .filter(Boolean);
    const recentTraceIds = visibleTraceIds.slice(-CHAT_TRACE_STATE_CACHE_MAX);
    const traceIdsToKeep = new Set(recentTraceIds);
    setTraceStepsById((prev) =>
      pruneRecordToAllowedKeys(prev, traceIdsToKeep),
    );
    setTraceLoadingById((prev) =>
      pruneRecordToAllowedKeys(prev, traceIdsToKeep),
    );
    setTraceErrorById((prev) =>
      pruneRecordToAllowedKeys(prev, traceIdsToKeep),
    );
  }, [messages]);
  const previousUserPromptByIndex = useMemo(() => {
    const map = new Map<number, string>();
    let lastUserPrompt = "";
    for (let i = 0; i < messages.length; i += 1) {
      const m = asRecord(messages[i]);
      const role = str(m.role, "").toLowerCase();
      if (role === "user") {
        map.set(i, "");
        lastUserPrompt = stripAttachmentContextMarker(str(m.content, ""));
      } else {
        map.set(i, lastUserPrompt);
      }
    }
    return map;
  }, [messages]);
  const chatCredentialPromptPayload = asRecord(chatCredentialPromptQ.data);
  const chatCredentialPrompt = asRecord(chatCredentialPromptPayload.prompt);
  const chatCredentialPromptFields = pickRecords(chatCredentialPrompt, "fields");
  const chatCredentialPromptModeKind = str(
    chatCredentialPrompt.mode_kind,
    "",
  )
    .trim()
    .toLowerCase();
  const chatCredentialPromptIsOAuthShape =
    chatCredentialPromptModeKind === "oauth2_authorization_code" ||
    chatCredentialPromptModeKind === "oauth2_device_code" ||
    chatCredentialPromptModeKind === "hybrid";
  const chatCredentialPromptDocsUrl = str(
    chatCredentialPrompt.docs_url,
    "",
  ).trim();
  const chatCredentialPromptSettingsPath = str(
    chatCredentialPrompt.settings_path,
    "",
  ).trim();
  const chatCredentialPromptVisible =
    toBool(chatCredentialPromptPayload.present) &&
    chatCredentialPromptFields.length > 0;
  const chatCredentialPromptFingerprint = chatCredentialPromptVisible
    ? `${conversationId || ""}:${str(chatCredentialPrompt.kind, "")}:${str(chatCredentialPrompt.title, "")}:${chatCredentialPromptFields.map((field) => `${str(field.key, "")}:${toBool(field.required)}`).join("|")}`
    : "";
  const submitChatCredentialPromptMutation = useMutation({
    mutationFn: (values: Record<string, string>) =>
      api.rawPost("/chat/credential-prompt/submit", {
        conversation_id: conversationId,
        values,
      }),
    onSuccess: async (data) => {
      const payload = asRecord(data);
      const followup = str(payload.followup, "").trim();
      setChatCredentialError(null);
      setChatCredentialValues({});
      setChatCredentialDialogOpen(false);
      if (conversationId) {
        setDismissedCredentialPromptConversationIds((prev) => {
          if (!prev.has(conversationId)) return prev;
          const next = new Set(prev);
          next.delete(conversationId);
          return next;
        });
      }
      if (followup) {
        setChatNotice(followup);
      }
      await queryClient.invalidateQueries({
        queryKey: ["chat-credential-prompt", conversationId],
      });
      await queryClient.invalidateQueries({
        queryKey: ["chat-messages", conversationId],
      });
    },
    onError: (err) => {
      setChatCredentialError(normalizeChatError(errMessage(err)));
    },
  });
  const dismissChatCredentialPromptMutation = useMutation({
    mutationFn: () =>
      api.rawDelete(
        `/chat/credential-prompt?conversation_id=${encodeURIComponent(conversationId || "")}`,
      ),
    onSuccess: async () => {
      setChatCredentialError(null);
      setChatCredentialValues({});
      setChatCredentialDialogOpen(false);
      if (conversationId) {
        setDismissedCredentialPromptConversationIds((prev) => {
          const next = new Set(prev);
          next.add(conversationId);
          return next;
        });
      }
      setChatNotice(
        "Secure credential request dismissed. You can add credentials later in Settings.",
      );
      await queryClient.invalidateQueries({
        queryKey: ["chat-credential-prompt", conversationId],
      });
    },
    onError: (err) => {
      setChatCredentialError(normalizeChatError(errMessage(err)));
    },
  });
  useEffect(() => {
    // Auto-open the secure credential dialog once per distinct prompt.
    // Closing it (X / backdrop / Esc) is a local cancel: the compact
    // in-chat card stays available to reopen, and the same prompt never
    // force-reopens against the user's choice.
    if (!chatCredentialPromptVisible) {
      autoOpenedCredentialPromptFingerprintRef.current = "";
      setChatCredentialDialogOpen(false);
      return;
    }
    if (
      autoOpenedCredentialPromptFingerprintRef.current ===
      chatCredentialPromptFingerprint
    ) {
      return;
    }
    autoOpenedCredentialPromptFingerprintRef.current =
      chatCredentialPromptFingerprint;
    setChatCredentialDialogOpen(true);
  }, [chatCredentialPromptVisible, chatCredentialPromptFingerprint]);
  const selectedConversationErrorText = errMessage(selectedConversationQ.error)
    .replace(/^error:\s*/i, "")
    .trim()
    .toLowerCase();
  const selectedConversationNotFound =
    !!conversationId &&
    !sidebarConversationIds.has(conversationId) &&
    selectedConversationErrorText === "conversation not found";
  const latestAssistantTraceId = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const candidate = messages[i];
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      const traceId = str(candidate.trace_id, "").trim();
      if (traceId) return traceId;
    }
    return "";
  }, [messages]);
  const selectedConversationWorkspace = asRecord(
    selectedConversation?.workspace,
  );
  const restoredConversationWorkspaceApp = useMemo(
    () =>
      extractWorkspaceAppFromStreamPayload("app_inspect", {
        matched_app: selectedConversationWorkspace,
      }),
    [selectedConversationWorkspace],
  );
  const restoredConversationWorkspaceFiles = useMemo(
    () =>
      extractWorkspaceFilesFromStreamPayload("app_inspect", {
        matched_app: selectedConversationWorkspace,
      }),
    [selectedConversationWorkspace],
  );

  useEffect(() => {
    if (chatCredentialPromptVisible && conversationId) {
      setDismissedCredentialPromptConversationIds((prev) => {
        if (!prev.has(conversationId)) return prev;
        const next = new Set(prev);
        next.delete(conversationId);
        return next;
      });
    }
    if (!chatCredentialPromptVisible) {
      setChatCredentialValues({});
      setChatCredentialError(null);
      return;
    }
    setChatCredentialValues((prev) => {
      const next: Record<string, string> = {};
      for (const field of chatCredentialPromptFields) {
        const key = str(field.key, "").trim();
        if (!key) continue;
        next[key] = prev[key] || "";
      }
      const prevKeys = Object.keys(prev);
      const nextKeys = Object.keys(next);
      if (
        prevKeys.length === nextKeys.length &&
        nextKeys.every((key) => prev[key] === next[key])
      ) {
        return prev;
      }
      return next;
    });
    setChatCredentialError(null);
  }, [
    chatCredentialPromptFields,
    chatCredentialPromptFingerprint,
    chatCredentialPromptVisible,
    conversationId,
  ]);

  useEffect(() => {
    if (!pendingRunSnapshot) {
      scheduleChatPendingRunSnapshotStore(null);
      return;
    }
    // Lazy producer: step compaction + response slicing run once at the
    // debounced flush, not on every 80ms token / 180ms step tick where the
    // result was overwritten before ever being persisted.
    scheduleChatPendingRunSnapshotStore(() => ({
      ...pendingRunSnapshot,
      message: pendingUserMessage ?? pendingRunSnapshot.message,
      streamingResponse: streamingResponse.slice(
        0,
        CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS,
      ),
      streamingSteps: compactPendingRunStepsForSnapshot(
        asRecords(streamingSteps),
      ),
      failedUserMessage: failedUserMessage ?? "",
      lastRunSeq: Math.max(
        pendingRunSnapshot.lastRunSeq ?? 0,
        latestRunEventSeqRef.current,
      ),
    }));
  }, [
    pendingRunSnapshot,
    pendingUserMessage,
    failedUserMessage,
    streamingResponse,
    streamingSteps,
  ]);

  useEffect(() => {
    conversationIdRef.current = conversationId;
    if (conversationId && draftChatActiveRef.current) {
      setDraftChatMode(false);
    }
  }, [conversationId]);

  useEffect(() => {
    draftChatActiveRef.current = draftChatActive;
  }, [draftChatActive]);

  useEffect(() => {
    writeChatRouteConversationId(conversationId);
  }, [conversationId]);

  useEffect(() => {
    pendingRunSnapshotRef.current = pendingRunSnapshot;
    latestRunEventSeqRef.current = Math.max(
      latestRunEventSeqRef.current,
      pendingRunSnapshot?.lastRunSeq ?? 0,
    );
  }, [pendingRunSnapshot]);

  useEffect(() => {
    streamingResponseRef.current = streamingResponse;
  }, [streamingResponse]);
  useEffect(
    () => () => {
      if (
        typeof window !== "undefined" &&
        streamingTokenFlushTimerRef.current !== null
      ) {
        window.clearTimeout(streamingTokenFlushTimerRef.current);
      }
      streamingTokenFlushTimerRef.current = null;
      streamingTokenBufferRef.current = "";
      if (
        typeof window !== "undefined" &&
        streamingStepsFlushTimerRef.current !== null
      ) {
        window.clearTimeout(streamingStepsFlushTimerRef.current);
      }
      streamingStepsFlushTimerRef.current = null;
      if (
        typeof window !== "undefined" &&
        reasoningPreviewFlushTimerRef.current !== null
      ) {
        window.clearTimeout(reasoningPreviewFlushTimerRef.current);
      }
      reasoningPreviewFlushTimerRef.current = null;
      if (
        typeof window !== "undefined" &&
        pendingRunSnapshotStoreTimerRef.current !== null
      ) {
        window.clearTimeout(pendingRunSnapshotStoreTimerRef.current);
      }
      pendingRunSnapshotStoreTimerRef.current = null;
      storeChatPendingRunSnapshot(
        resolvePendingRunSnapshotSource(pendingRunSnapshotStoreRef.current),
      );
      if (
        typeof window !== "undefined" &&
        workspaceSnapshotStoreTimerRef.current !== null
      ) {
        window.clearTimeout(workspaceSnapshotStoreTimerRef.current);
      }
      workspaceSnapshotStoreTimerRef.current = null;
      if (workspaceSnapshotStoreRef.current) {
        storeChatWorkspaceSnapshot(workspaceSnapshotStoreRef.current);
      }
    },
    [],
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof document === "undefined") {
      return undefined;
    }
    const flushBufferedChatState = () => {
      flushStreamingTokenBuffer();
      flushQueuedStreamingSteps();
      flushReasoningPreview();
      flushChatPendingRunSnapshotStore();
      flushChatWorkspaceSnapshotStore();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        flushBufferedChatState();
      }
    };
    window.addEventListener("pagehide", flushBufferedChatState);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("pagehide", flushBufferedChatState);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, []);
  // Keep stream choices scoped to active assistant text.
  useEffect(() => {
    if (!streamingResponse.trim()) {
      setStreamingResponseChoices([]);
    }
  }, [streamingResponse]);

  useEffect(() => {
    if (typeof window === "undefined" || !conversationId) return;
    try {
      window.sessionStorage.setItem(
        CHAT_LAST_CONVERSATION_STORAGE_KEY,
        conversationId,
      );
    } catch {
      // Ignore storage failures.
    }
  }, [conversationId]);

  useEffect(() => {
    if (!conversationId) {
      lastWorkspaceRestoreSeedRef.current = "";
      return;
    }
    if (isStreamingForCurrentConversation) return;
    const cachedSnapshot = loadChatWorkspaceSnapshot(conversationId);
    const mergedWorkspaceApp = sanitizeWorkspaceAppSnapshot({
      ...(cachedSnapshot?.streamedWorkspaceApp || {}),
      ...(restoredConversationWorkspaceApp || {}),
    });
    const appDir = str(
      mergedWorkspaceApp?.app_dir,
      str(cachedSnapshot?.streamedWorkspaceApp?.app_dir, ""),
    ).trim();
    const mergedFiles = mergeWorkspaceFiles(
      restoredConversationWorkspaceFiles,
      cachedSnapshot?.deployedFiles || [],
      appDir,
    );
    const mergedLiveWrites = canonicalizeLiveFileWrites(
      cachedSnapshot?.liveFileWrites || {},
      appDir,
    );
    const restoreSeed = JSON.stringify({
      conversationId,
      updatedAt: str(selectedConversation?.updated_at, ""),
      appId: str(mergedWorkspaceApp?.id, str(mergedWorkspaceApp?.app_id, "")),
      files: mergedFiles.map((file) => file.name),
      snapshotUpdatedAt: cachedSnapshot?.updatedAt || 0,
      liveWrites: Object.keys(mergedLiveWrites),
    });
    if (lastWorkspaceRestoreSeedRef.current === restoreSeed) return;
    lastWorkspaceRestoreSeedRef.current = restoreSeed;
    if (
      !mergedWorkspaceApp &&
      mergedFiles.length === 0 &&
      Object.keys(mergedLiveWrites).length === 0
    ) {
      return;
    }
    if (mergedWorkspaceApp) {
      streamedWorkspaceAppRef.current = mergedWorkspaceApp;
      setStreamedWorkspaceApp(mergedWorkspaceApp);
    }
    if (mergedFiles.length > 0) {
      setDeployedFiles(mergedFiles);
      const requestedIndex = Math.max(
        0,
        num(cachedSnapshot?.codeViewerFileIdx, 0),
      );
      setCodeViewerFileIdx(
        Math.min(requestedIndex, Math.max(0, mergedFiles.length - 1)),
      );
    }
    if (Object.keys(mergedLiveWrites).length > 0) {
      setLiveFileWrites(mergedLiveWrites);
    }
  }, [
    conversationId,
    isStreamingForCurrentConversation,
    restoredConversationWorkspaceApp,
    restoredConversationWorkspaceFiles,
    selectedConversation?.updated_at,
  ]);

  useEffect(() => {
    if (!conversationId) return;
    const compactedFiles = compactWorkspaceFilesForSnapshot(deployedFiles, {
      includeContent: true,
    });
    const compactedLiveWrites = compactLiveFileWritesForSnapshot(
      liveFileWrites,
      { includeContent: true },
    );
    const compactedApp = sanitizeWorkspaceAppSnapshot(streamedWorkspaceApp);
    if (
      !compactedApp &&
      compactedFiles.length === 0 &&
      Object.keys(compactedLiveWrites).length === 0
    ) {
      return;
    }
    scheduleChatWorkspaceSnapshotStore({
      conversationId,
      updatedAt: Date.now(),
      deployedFiles: compactedFiles,
      liveFileWrites: compactedLiveWrites,
      streamedWorkspaceApp: compactedApp,
      codeViewerFileIdx: Math.max(0, num(codeViewerFileIdx, 0)),
    });
  }, [
    conversationId,
    deployedFiles,
    liveFileWrites,
    streamedWorkspaceApp,
    codeViewerFileIdx,
  ]);

  useEffect(() => {
    const maxPage = Math.max(0, conversationPageCount - 1);
    if (conversationPage > maxPage) {
      setConversationPage(maxPage);
    }
  }, [conversationPage, conversationPageCount]);

  useEffect(() => {
    const pending = pendingRunSnapshot ?? loadChatPendingRunSnapshot();
    const suppressPendingAutoSelect =
      draftChatActive || draftChatActiveRef.current;
    if (pending) {
      const shouldSelectPendingConversation =
        !conversationId && !suppressPendingAutoSelect;
      const viewingPendingConversation =
        conversationId === pending.conversationId;
      if (shouldSelectPendingConversation) {
        setConversationPage(0);
        setConversationId(pending.conversationId);
      }
      if (shouldSelectPendingConversation || viewingPendingConversation) {
        if (pending.message && !pendingUserMessage) {
          setPendingUserMessage(pending.message);
        }
        if (pending.failedUserMessage && !failedUserMessage) {
          setFailedUserMessage(pending.failedUserMessage);
        }
        if (pending.streamingResponse && !streamingResponse) {
          setStreamingResponseNow(pending.streamingResponse);
        }
        if (
          Array.isArray(pending.streamingSteps) &&
          pending.streamingSteps.length > 0 &&
          streamingStepsRef.current.length === 0
        ) {
          const restoredSteps = pending.streamingSteps.map((step) =>
            ensureActivityStepTime(asRecord(step)),
          );
          setStreamingStepsNow(restoredSteps);
        }
        return;
      }
    }

    if (
      conversationId ||
      suppressPendingAutoSelect ||
      (starredConversations.length === 0 && conversations.length === 0) ||
      typeof window === "undefined"
    ) {
      return;
    }
    try {
      const lastSelected = window.sessionStorage
        .getItem(CHAT_LAST_CONVERSATION_STORAGE_KEY)
        ?.trim();
      if (lastSelected && sidebarConversationIds.has(lastSelected)) {
        setConversationId(lastSelected);
      }
    } catch {
      // Ignore storage failures.
    }
  }, [
    conversationId,
    conversations,
    starredConversations,
    sidebarConversationIds,
    pendingRunSnapshot,
    pendingUserMessage,
    failedUserMessage,
    streamingResponse,
    streamingSteps.length,
    draftChatActive,
  ]);

  useEffect(() => {
    if (!postDeleteConversationFallback) return;
    if (
      conversationId &&
      conversationId !== postDeleteConversationFallback.deletedId
    ) {
      setPostDeleteConversationFallback(null);
      return;
    }
    if (conversationId) return;
    const availableIds = orderedSidebarConversationIds.filter(
      (id) => id !== postDeleteConversationFallback.deletedId,
    );
    if (availableIds.length > 0) {
      const nextConversationId =
        postDeleteConversationFallback.preferredId &&
        availableIds.includes(postDeleteConversationFallback.preferredId)
          ? postDeleteConversationFallback.preferredId
          : availableIds[0];
      setPostDeleteConversationFallback(null);
      openConversationById(nextConversationId);
      return;
    }
    if (!convQ.isFetching && conversationListTotal === 0) {
      setPostDeleteConversationFallback(null);
    }
  }, [
    postDeleteConversationFallback,
    conversationId,
    orderedSidebarConversationIds,
    convQ.isFetching,
    conversationListTotal,
  ]);

  useEffect(() => {
    if (!conversationId || pendingRunSnapshot) return;
    const restoredSnapshot = backgroundRunSnapshots[conversationId];
    if (!restoredSnapshot) return;
    const nextBackgroundSnapshots = { ...backgroundRunSnapshots };
    delete nextBackgroundSnapshots[conversationId];
    setBackgroundRunSnapshots(nextBackgroundSnapshots);
    storeChatBackgroundRunSnapshots(nextBackgroundSnapshots);
    setPendingRunSnapshot(restoredSnapshot);
    storeChatPendingRunSnapshotNow(restoredSnapshot);
  }, [conversationId, pendingRunSnapshot, backgroundRunSnapshots]);

  useEffect(() => {
    if (!pendingRunSnapshot) return;
    if (conversationId !== pendingRunSnapshot.conversationId) return;
    if (
      pendingRunSnapshot.phase === "awaiting_confirmation" ||
      pendingRunSnapshot.phase === "interrupted"
    )
      return;
    const latestMessage = messages[messages.length - 1];
    if (str(latestMessage?.role, "").toLowerCase() !== "assistant") return;
    const latestTimestampMs = Date.parse(str(latestMessage?.timestamp, ""));
    if (
      Number.isFinite(latestTimestampMs) &&
      latestTimestampMs + 1000 < pendingRunSnapshot.startedAt
    ) {
      return;
    }
    const preservedSteps =
      streamingStepsRef.current.length > 0
        ? trimTrailingHeartbeatSteps(streamingStepsRef.current)
        : trimTrailingHeartbeatSteps(streamingSteps);
    const restoredPlan =
      executionPlan ?? extractExecutionPlanFromTraceSteps(preservedSteps);
    if (
      shouldKeepPlanInApprovalState(
        restoredPlan,
        preservedSteps,
        pendingRunSnapshot.mode === "resume" ? "resume" : "fresh",
      )
    ) {
      return;
    }
    if (preservedSteps.length > 0) {
      setLastRunSteps(preservedSteps);
    }
    if (streamingProgressMessages.length > 0) {
      setCompletedProgressMessagesByConversation((prev) => {
        const next = {
          ...prev,
          [pendingRunSnapshot.conversationId]: {
            messages: streamingProgressMessages.slice(-5),
            beforeMessageId: str(latestMessage?.id, ""),
          },
        };
        const entries = Object.entries(next);
        if (entries.length <= CHAT_PROGRESS_MEMORY_MAX_CONVERSATIONS) {
          return next;
        }
        return Object.fromEntries(
          entries.slice(-CHAT_PROGRESS_MEMORY_MAX_CONVERSATIONS),
        ) as Record<string, { messages: string[]; beforeMessageId: string }>;
      });
    }
    clearPendingRunPresentation(preservedSteps);
  }, [
    pendingRunSnapshot,
    conversationId,
    messages,
    executionPlan,
    streamingSteps,
    streamingProgressMessages,
  ]);

  useEffect(() => {
    setCompletedProgressMessagesByConversation((prev) => {
      const entries = Object.entries(prev);
      if (entries.length <= CHAT_PROGRESS_MEMORY_MAX_CONVERSATIONS) return prev;
      const keepIds = new Set(
        [
          ...entries
            .slice(-(CHAT_PROGRESS_MEMORY_MAX_CONVERSATIONS - 1))
            .map(([id]) => id),
          conversationId,
        ].filter((id): id is string => Boolean(id)),
      );
      return Object.fromEntries(
        entries.filter(([id]) => keepIds.has(id)),
      ) as Record<string, { messages: string[]; beforeMessageId: string }>;
    });
  }, [conversationId]);

  useEffect(() => {
    const snapshot = pendingRunSnapshot;
    if (!snapshot) return;
    if (conversationId !== snapshot.conversationId) return;
    if (
      snapshot.phase === "awaiting_confirmation" ||
      snapshot.phase === "interrupted"
    ) {
      return;
    }
    if (isStreaming || streamLockRef.current) return;
    if (str(snapshot.runId, "").trim()) return;

    let cancelled = false;
    let retryHandle: number | null = null;

    const pollLatestRun = async () => {
      try {
        const outcome = await syncPendingRunFromLatestRun(
          snapshot.conversationId,
          snapshot,
          { allowTerminalClear: true },
        );
        if (cancelled || outcome !== "none") return;
      } catch {
        if (cancelled) return;
      }
      if (Date.now() - snapshot.startedAt > CHAT_PENDING_RUN_RECOVERY_GRACE_MS) {
        pushStreamingStep({
          step_type: "run_status",
          title: "Run status: interrupted",
          detail:
            "The browser had a saved pending run, but the backend no longer has an active matching run after reconnect.",
          data: {
            interruption_kind: "lost_backend_run",
            reason: "backend_run_missing_after_reconnect",
          },
        });
        markPendingRunInterrupted(
          snapshot.taskId,
          snapshot.streamingResponse || streamingResponseRef.current,
        );
        setPendingUserMessage(null);
        return;
      }
      retryHandle = window.setTimeout(() => {
        void pollLatestRun();
      }, 1500);
    };

    void pollLatestRun();
    return () => {
      cancelled = true;
      if (retryHandle !== null) {
        window.clearTimeout(retryHandle);
      }
    };
  }, [
    conversationId,
    pendingRunSnapshot,
    isStreaming,
  ]);

  useEffect(() => {
    if (
      !latestAssistantTraceId ||
      isStreaming ||
      hasPendingSnapshotForConversation
    )
      return;
    if (
      traceStepsById[latestAssistantTraceId] ||
      traceLoadingById[latestAssistantTraceId] ||
      traceErrorById[latestAssistantTraceId]
    )
      return;
    void loadTraceForId(latestAssistantTraceId);
  }, [
    latestAssistantTraceId,
    isStreaming,
    hasPendingSnapshotForConversation,
    traceStepsById,
    traceLoadingById,
    traceErrorById,
  ]);

  // Keep a small warm cache for recent assistant traces. Loading every prior
  // trace in a long chat makes the console sluggish after a few turns.
  useEffect(() => {
    if (isStreaming || hasPendingSnapshotForConversation) return;
    const traceIds: string[] = [];
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (str(message.role, "").toLowerCase() !== "assistant") continue;
      const traceId = str(message.trace_id, "").trim();
      if (!traceId) continue;
      if (traceId === latestAssistantTraceId) continue;
      if (
        traceStepsById[traceId] ||
        traceLoadingById[traceId] ||
        traceErrorById[traceId]
      )
        continue;
      traceIds.push(traceId);
      if (traceIds.length >= Math.max(0, CHAT_TRACE_EAGER_LOAD_MAX - 1)) break;
    }
    if (traceIds.length === 0) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      traceIds.forEach((traceId) => {
        if (!cancelled) void loadTraceForId(traceId);
      });
    }, CHAT_TRACE_EAGER_LOAD_IDLE_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [
    messages,
    latestAssistantTraceId,
    isStreaming,
    hasPendingSnapshotForConversation,
    traceStepsById,
    traceLoadingById,
    traceErrorById,
  ]);

  useEffect(() => {
    if (!conversationId || isStreaming || hasPendingSnapshotForConversation)
      return;
    let cancelled = false;
    void (async () => {
      try {
        const payload = asRecord(
          await api.rawGet(
            `/conversations/${encodeURIComponent(conversationId)}/latest-run`,
          ),
        );
        if (cancelled) return;
        const events = asRecords(payload.events);
        setLastRunSteps(buildPersistedRunSteps(events));
      } catch {
        if (!cancelled) {
          setLastRunSteps([]);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [conversationId, isStreaming, hasPendingSnapshotForConversation]);

  const toHumanToolName = (name: string): string => {
    const normalized = (name || "").trim().toLowerCase();
    if (!normalized) return "Tool";
    const direct: Record<string, string> = {
      app_deploy: "App deploy",
      build_check: "Build check",
      run_tests: "Test run",
      lint_check: "Lint check",
      source_read: "Read files",
      source_write: "Write files",
      source_edit: "Edit files",
      source_list: "List files",
      source_search: "Search files",
      frontend_build: "Frontend build",
      schedule_task: "Schedule task",
      browse: "Open web page",
      web_search: "Web search",
    };
    if (direct[normalized]) return direct[normalized];
    return normalized
      .replace(/[_-]+/g, " ")
      .replace(/\b\w/g, (ch) => ch.toUpperCase());
  };

  const toggleExpandedActivityPayload = useCallback((id: string) => {
    setExpandedActivityPayloads((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const toggleExpandedTranscriptAction = (id: string) => {
    setExpandedTranscriptActions((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const followActivityConsole = () => {
    setActivityAutoFollow(true);
  };

  const revealLiveFilesConsole = () => {
    // Respect an explicit mid-run user dismissal for the rest of this run.
    if (workspaceUserClosedRef.current) return;
    setWorkspaceOpen(true);
    setActiveStepId(null);
    setSelectedSnippetId(null);
    setActivityAutoFollow(true);
  };

  const openActivityConsole = () => {
    if (canInlineWorkspacePanel) {
      setWorkspaceOpen(true);
    } else {
      setWorkspaceOpen(true);
    }
    setActivityAutoFollow(true);
  };

  const resetStreamingProgressBubbleState = () => {
    lastProgressBubbleCategoryRef.current = "";
    lastProgressBubbleAtRef.current = 0;
    reasoningProgressByPhaseRef.current = {};
    reasoningActivityEmitRef.current = {};
  };

  const normalizeStreamingProgressBubbleText = (value: string): string =>
    (value || "")
      .replace(/\r\n/g, "\n")
      .split("\n")
      .map((line) => line.replace(/[ \t]+/g, " ").trim())
      .join("\n")
      .replace(/\n{3,}/g, "\n\n")
      .trim();

  const pushStreamingProgressBubble = (
    message: string,
    options?: { category?: string; replace?: boolean; minIntervalMs?: number },
  ) => {
    const text = normalizeStreamingProgressBubbleText(message);
    if (!text) return;
    const category = str(options?.category, "").trim();
    const replace = Boolean(options?.replace && category);
    const minIntervalMs = Math.max(0, num(options?.minIntervalMs, 0));
    const now = Date.now();
    setStreamingProgressMessages((prev) => {
      if (replace && category) {
        const sameCategory = lastProgressBubbleCategoryRef.current === category;
        if (
          sameCategory &&
          now - lastProgressBubbleAtRef.current < minIntervalMs
        ) {
          return prev;
        }
        if (sameCategory && prev.length > 0) {
          if (prev[prev.length - 1] === text) return prev;
          lastProgressBubbleAtRef.current = now;
          return [...prev.slice(0, -1), text];
        }
      }
      if (prev.some((entry) => entry === text)) {
        if (replace && category) {
          lastProgressBubbleCategoryRef.current = category;
          lastProgressBubbleAtRef.current = now;
        }
        return prev;
      }
      if (replace && category) {
        lastProgressBubbleCategoryRef.current = category;
        lastProgressBubbleAtRef.current = now;
      }
      return [...prev.slice(-4), text];
    });
  };

  const maybeSurfaceThinkingProgressBubble = (step: JsonRecord) => {
    if (isHeartbeatStreamingStep(step)) return;
    const detail = simplifyConsoleDetail(
      summarizeActivityDetail(extractStepDetailText(step, 900)),
    );
    if (!detail) return;
    const presentation = humanizeStep(
      str(step.title, ""),
      detail,
      str(step.step_type, str(step.type, "thinking")),
    );
    const label = str(presentation.label, "").trim();
    const message =
      label && detail && label.toLowerCase() !== detail.toLowerCase()
        ? `${label}: ${detail}`
        : detail || label;
    if (!message) return;
    pushStreamingProgressBubble(message, {
      category: "thinking",
      replace: true,
      minIntervalMs: 2500,
    });
  };

  const maybeSurfaceToolStartProgressBubble = (
    name: string,
    payloadObj: JsonRecord,
  ) => {
    const message = toolStartIntentText(payloadObj).trim();
    if (!message) return;
    pushStreamingProgressBubble(message, {
      category: `tool-start:${str(name, "").trim().toLowerCase() || "tool"}`,
      replace: true,
      minIntervalMs: 4000,
    });
  };

  const maybeSurfaceToolProgressBubble = (
    name: string,
    content: string,
    payloadObj: JsonRecord,
    progressPresentation: ToolProgressPresentation,
  ) => {
    const kind = str(payloadObj.kind, "");
    const workspaceAppDir = str(streamedWorkspaceAppRef.current?.app_dir, "");

    if (kind === "draft_file") {
      const fileName = normalizeWorkspaceFileName(
        payloadObj.file ?? payloadObj.path,
        workspaceAppDir,
      );
      const lineNo = Math.max(0, num(payloadObj.line, 0));
      const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
      const targetPath = progressFileTargetPath(
        payloadObj,
        workspaceAppDir,
        fileName,
      );
      const lineLabel = progressLineLabel(lineNo, totalLines);
      const message = [
        fileName ? `Drafting ${fileName}` : "Drafting file",
        targetPath ? `bundle path: ${targetPath}` : "",
        lineLabel,
      ]
        .filter(Boolean)
        .join(" - ");
      pushStreamingProgressBubble(message, {
        category: str(
          payloadObj.stream_key,
          fileName ? `draft-file:${fileName}` : "draft-file",
        ),
        replace: true,
        minIntervalMs: 1200,
      });
      return;
    }

    if (kind === "file_write" || name === "file_write") {
      const fileName = normalizeWorkspaceFileName(
        payloadObj.file ?? payloadObj.path,
        workspaceAppDir,
      );
      const lineNo = Math.max(0, num(payloadObj.line, 0));
      const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
      const targetPath = progressFileTargetPath(
        payloadObj,
        workspaceAppDir,
        fileName,
      );
      const lineLabel = progressLineLabel(lineNo, totalLines);
      const done = toBool(payloadObj.done);
      const message = [
        done ? "Wrote file" : "Writing file",
        fileName,
        targetPath && targetPath !== fileName ? `to ${targetPath}` : "",
        lineLabel,
      ]
        .filter(Boolean)
        .join(" - ");
      pushStreamingProgressBubble(message, {
        category: fileName ? `file-write:${fileName}` : "file-write",
        replace: true,
        minIntervalMs: 1200,
      });
      return;
    }

    if (kind === "argument_stream") {
      return;
    }

    const runningTitle = runningActivityTitleForToolName(name).toLowerCase();
    const message = (
      progressPresentation.detail ||
      simplifyConsoleDetail(
        summarizeActivityDetail(content.trim().slice(0, 1600)),
      ) ||
      `I'm still ${runningTitle}.`
    ).trim();
    if (!message || /^working\.{0,3}$/i.test(message)) return;
    pushStreamingProgressBubble(message, {
      category: `tool-progress:${str(name, "").trim().toLowerCase() || "tool"}`,
      replace: true,
      minIntervalMs: 8000,
    });
  };

  const simplifyConsoleDetail = (detail: string): string => {
    let text = (detail || "").replace(/\s+/g, " ").trim();
    if (!text) return "";

    if (/^loaded \d+ messages?, packed \d+/i.test(text))
      return "Collected recent chat context.";
    if (/channel:\s*\w+\s*\|\s*length:\s*\d+\s*chars/i.test(text))
      return "Reading your request.";
    if (/found \d+ relevant memories/i.test(text)) {
      const m = text.match(/found\s+(\d+)\s+relevant memories/i);
      const count = m?.[1] || "0";
      return `Found ${count} related memory item${count === "1" ? "" : "s"}.`;
    }
    if (/complex\s*[-=]?>\s*direct llm/i.test(text))
      return "Using a direct execution strategy.";
    if (/using primary model/i.test(text))
      return "Selected the best available model.";
    if (
      /response length:\s*\d+\s*chars/i.test(text) ||
      /tool calls:\s*\d+/i.test(text)
    ) {
      return "Prepared the next response.";
    }
    if (/proof id:|verification id:/i.test(text))
      return "Saved a verifiable execution record.";
    if (/running in sandboxed environment/i.test(text))
      return "Running this action in a safe workspace.";
    if (
      /install(ing)? dependencies|npm install|pnpm install|yarn install|cargo fetch/i.test(
        text,
      )
    ) {
      return "Installing dependencies.";
    }
    if (isSafetyPolicyBlockedText(text)) {
      return "Blocked by safety policy. The agent needs a different approach.";
    }
    if (
      /approval required|needs approval|awaiting approval|requires approval/i.test(
        text,
      )
    ) {
      return "Waiting for your approval/input.";
    }
    if (/browse failed; used search fallback/i.test(text)) {
      return "Could not open the page directly, switched to web search.";
    }
    if (/http error 404/i.test(text) || /\b404\b.*not found/i.test(text)) {
      return "Page not found (404). Trying alternate sources.";
    }
    if (/search results for:/i.test(text))
      return "Found search results and selected relevant sources.";
    if (/^\{\s*"name"\s*:\s*"[^"]+"\s*\}$/i.test(text)) {
      const toolMatch = text.match(/"name"\s*:\s*"([^"]+)"/i);
      if (toolMatch?.[1]) return `${toHumanToolName(toolMatch[1])} started.`;
    }

    if (text.length > 170) text = `${text.slice(0, 167).trimEnd()}...`;
    return text;
  };

  const normalizeStatusText = (value: string): string =>
    (value || "")
      .toLowerCase()
      .replace(/[`"'.,:;!?()[\]{}<>/_\\-]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();

  const isRedundantStatusDetail = (label: string, detail: string): boolean => {
    const a = normalizeStatusText(label);
    const b = normalizeStatusText(detail);
    if (!a || !b) return false;
    if (a === b) return true;
    if (a.length >= 16 && b.includes(a)) return true;
    if (b.length >= 16 && a.includes(b)) return true;
    return false;
  };

  const planStepUpdateTitle = (status: string): string => {
    switch ((status || "").trim().toLowerCase()) {
      case "completed":
        return "Plan Step Completed";
      case "failed":
        return "Plan Step Failed";
      case "skipped":
        return "Plan Step Skipped";
      case "running":
        return "Plan Step Started";
      default:
        return "Plan Step Updated";
    }
  };

  const planStepUpdateDetail = (
    status: string,
    stepId: number,
    stepTitle: string,
  ): string => {
    const subject = stepTitle || (stepId > 0 ? `step ${stepId}` : "plan step");
    switch ((status || "").trim().toLowerCase()) {
      case "completed":
        return `Completed ${subject}.`;
      case "failed":
        return `Failed ${subject}.`;
      case "skipped":
        return `Skipped ${subject}.`;
      case "running":
        return `Started ${subject}.`;
      default:
        return `Updated ${subject}.`;
    }
  };

  const normalizePlanStepUpdateStep = (step: JsonRecord): JsonRecord => {
    const stepType = str(step.step_type, "");
    if (stepType !== "plan_step_update") return step;
    const status = str(step.status, "pending");
    const stepId =
      typeof step.step_id === "number" ? step.step_id : num(step.step_id, 0);
    const stepTitle = str(step.step_title, "").trim();
    const title = str(step.title, "").trim() || planStepUpdateTitle(status);
    const detail =
      str(step.detail, "").trim() ||
      planStepUpdateDetail(status, stepId, stepTitle);
    return {
      ...step,
      title,
      detail,
      step_title: stepTitle,
    };
  };

  const maybeSurfacePlanStepProgressBubble = (step: JsonRecord) => {
    const stepType = str(step.step_type, "");
    if (stepType !== "plan_step_update") return;
    if (
      !!str(planConfirmation?.source, "").trim() ||
      pendingRunSnapshot?.phase === "running" ||
      pendingRunSnapshot?.phase === "awaiting_confirmation"
    ) {
      return;
    }
    const status = str(step.status, "").trim().toLowerCase();
    if (!["running", "completed", "failed", "skipped"].includes(status)) return;
    const stepId =
      typeof step.step_id === "number" ? step.step_id : num(step.step_id, 0);
    const stepTitle = str(step.step_title, "").trim();
    const message = planStepUpdateDetail(status, stepId, stepTitle);
    if (!message) return;
    pushStreamingProgressBubble(message, {
      category: `plan-step:${str(step.plan_id, "")}:${num(step.revision, 0)}:${stepId}:${status}`,
      minIntervalMs: 0,
    });
  };

  const currentExecutionPlanStepMeta = () => {
    const planForMeta =
      executionPlan ??
      extractExecutionPlanFromTraceSteps(
        trimTrailingHeartbeatSteps(streamingStepsRef.current),
      );
    if (!planForMeta) return null;
    const activeStep =
      planForMeta.steps.find((step) => step.status === "running") ||
      planForMeta.steps.find((step) => step.status === "pending");
    if (!activeStep) return null;
    return {
      planId: planForMeta.plan_id,
      revision: planForMeta.revision,
      stepId: activeStep.id,
      stepTitle: activeStep.title,
    };
  };

  const attachCurrentPlanStepPayload = (payload: JsonRecord): JsonRecord => {
    const meta = currentExecutionPlanStepMeta();
    if (!meta) return payload;
    return {
      ...payload,
      plan_id: str(payload.plan_id, "") || meta.planId,
      plan_revision: num(payload.plan_revision, 0) || meta.revision,
      plan_step_id:
        typeof payload.plan_step_id === "number"
          ? payload.plan_step_id
          : meta.stepId,
      plan_step_title:
        str(payload.plan_step_title, "").trim() || meta.stepTitle,
    };
  };

  const decorateActivityDetailWithPlanStep = (
    detail: string,
    payloadObj: JsonRecord,
  ): string => {
    const planStepTitle = str(payloadObj.plan_step_title, "").trim();
    if (!planStepTitle) return detail;
    const prefix = `Plan step: ${planStepTitle}.`;
    if (!detail) return prefix;
    return isRedundantStatusDetail(prefix, detail)
      ? detail
      : `${prefix} ${detail}`;
  };

  const markPendingRunAwaitingPlanConfirmation = (taskId = "") => {
    setPendingRunSnapshot((prev) => {
      if (!prev) return prev;
      const next = {
        ...prev,
        taskId: taskId || prev.taskId || "",
        phase: "awaiting_confirmation" as ChatPendingRunPhase,
      };
      storeChatPendingRunSnapshotNow(next);
      return next;
    });
  };

  const markPendingRunInterrupted = (
    taskId = "",
    streamingResponseOverride = "",
  ) => {
    setPendingRunSnapshot((prev) => {
      if (!prev) return prev;
      const interruptedSteps = limitPendingRunStepsForSnapshot(
        trimTrailingHeartbeatSteps(streamingStepsRef.current),
      );
      const next = {
        ...prev,
        taskId: taskId || prev.taskId || "",
        phase: "interrupted" as ChatPendingRunPhase,
        streamingResponse: (
          streamingResponseOverride ||
          prev.streamingResponse ||
          ""
        ).slice(0, CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS),
        streamingSteps: interruptedSteps,
      };
      storeChatPendingRunSnapshotNow(next);
      return next;
    });
  };

  function clearPendingRunPresentation(completedSteps: JsonRecord[] = []) {
    if (completedSteps.length > 0) {
      setLastRunSteps(completedSteps);
    }
    storeChatPendingRunSnapshotNow(null);
    setPendingRunSnapshot(null);
    setPendingUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
  }

  function isActiveExecutionRunStatus(raw: string): boolean {
    switch ((raw || "").trim().toLowerCase()) {
      case "accepted":
      case "routing":
      case "model_selection":
      case "planning":
      case "tool_dispatch":
      case "synthesis":
        return true;
      default:
        return false;
    }
  }

  function isTerminalExecutionRunStatus(raw: string): boolean {
    const normalized = (raw || "").trim().toLowerCase();
    return Boolean(normalized) && !isActiveExecutionRunStatus(normalized);
  }

  const fetchConversationMessagesIntoCache = useCallback(
    async (targetConversationId: string): Promise<JsonRecord[]> => {
      const id = targetConversationId.trim();
      if (!id) return [];
      const payload = sanitizeChatMessagesPayloadForUi(await api.rawGet(
        `/conversations/${encodeURIComponent(id)}/messages?limit=100`,
      ));
      const previousRecords = pickRecords(
        queryClient.getQueryData(["chat-messages", id]),
        "messages",
      );
      const choicesByTrace = new Map<string, ChatClarificationChoice[]>();
      const choicesByContent = new Map<string, ChatClarificationChoice[]>();
      for (const record of previousRecords) {
        const choices = clarificationChoices(record.choices).filter(
          (choice) => !isDirectChatApprovalChoice(choice),
        );
        if (choices.length === 0) continue;
        const traceId = str(record.trace_id, str(record.traceId, "")).trim();
        const content = stripAgentInternalReasoningLeaks(
          str(record.content, ""),
        ).trim();
        if (traceId) choicesByTrace.set(traceId, choices);
        if (content) choicesByContent.set(content, choices);
      }
      const payloadRecords = pickRecords(payload, "messages");
      const mergedRecords = payloadRecords.map((record) => {
        if (clarificationChoices(record.choices).length > 0) return record;
        const traceId = str(record.trace_id, str(record.traceId, "")).trim();
        const content = stripAgentInternalReasoningLeaks(
          str(record.content, ""),
        ).trim();
        const preservedChoices =
          (traceId ? choicesByTrace.get(traceId) : undefined) ||
          (content ? choicesByContent.get(content) : undefined);
        return preservedChoices && preservedChoices.length > 0
          ? { ...record, choices: preservedChoices }
          : record;
      });
      const persistedAssistantKeys = new Set(
        mergedRecords
          .filter(
            (record) =>
              str(record.role, "").trim().toLowerCase() === "assistant",
          )
          .flatMap((record) => {
            const content = stripAgentInternalReasoningLeaks(
              str(record.content, ""),
            ).trim();
            const traceId = str(record.trace_id, str(record.traceId, "")).trim();
            return [content ? `content:${content}` : "", traceId ? `trace:${traceId}` : ""].filter(Boolean);
          }),
      );
      const optimisticAssistantRecords = previousRecords.filter((record) => {
        if (!toBool(record.optimistic)) return false;
        if (str(record.role, "").trim().toLowerCase() !== "assistant") {
          return false;
        }
        const content = stripAgentInternalReasoningLeaks(
          str(record.content, ""),
        ).trim();
        if (!content) return false;
        const traceId = str(record.trace_id, str(record.traceId, "")).trim();
        return !(
          persistedAssistantKeys.has(`content:${content}`) ||
          (traceId && persistedAssistantKeys.has(`trace:${traceId}`))
        );
      });
      const visibleRecords =
        optimisticAssistantRecords.length > 0
          ? [...mergedRecords, ...optimisticAssistantRecords]
          : mergedRecords;
      const nextPayload = Array.isArray(payload)
        ? visibleRecords
        : { ...asRecord(payload), messages: visibleRecords };
      queryClient.setQueryData(["chat-messages", id], nextPayload);
      return visibleRecords;
    },
    [queryClient],
  );

  const refreshConversationMessagesAfterStream = useCallback(
    async (
      targetConversationId: string,
      minAssistantMessages?: number,
      options?: { settle?: boolean; minLatestAssistantCreatedAtMs?: number },
    ): Promise<void> => {
      const id = targetConversationId.trim();
      if (!id) return;
      let lastError: unknown = null;
      let fetched = false;
      const shouldSettle = options?.settle === true;
      const expectsFreshAssistant =
        options?.minLatestAssistantCreatedAtMs != null;
      const hasExpectedAssistantCount = (records: JsonRecord[]) =>
        minAssistantMessages == null ||
        records.filter(
          (message) => str(message.role, "").trim().toLowerCase() === "assistant",
        ).length >= minAssistantMessages;
      const hasFreshAssistant = (records: JsonRecord[]) => {
        const minCreatedAtMs = options?.minLatestAssistantCreatedAtMs;
        if (minCreatedAtMs == null) return true;
        return records.some((message) => {
          if (str(message.role, "").trim().toLowerCase() !== "assistant") {
            return false;
          }
          const timestampMs = Date.parse(str(message.timestamp, ""));
          return Number.isFinite(timestampMs) && timestampMs + 1000 >= minCreatedAtMs;
        });
      };
      for (const settleDelayMs of [0, 250, 750, 1500, 3000, 6000]) {
        if (settleDelayMs > 0) {
          await delay(settleDelayMs);
        }
        try {
          const records = await fetchConversationMessagesIntoCache(id);
          fetched = true;
          lastError = null;
          const waitedForPersistence =
            !shouldSettle || settleDelayMs >= 1500;
          if (
            waitedForPersistence &&
            hasExpectedAssistantCount(records) &&
            hasFreshAssistant(records)
          ) {
            return;
          }
        } catch (error) {
          if (!fetched) {
            lastError = error;
          }
        }
      }
      if (!fetched && lastError) {
        throw lastError;
      }
      if (minAssistantMessages != null || expectsFreshAssistant) {
        throw new Error("The final assistant message has not been persisted yet.");
      }
    },
    [fetchConversationMessagesIntoCache],
  );

  const appendAssistantContentToConversationCache = useCallback(
    (targetConversationId: string, payload: unknown) => {
      const id = targetConversationId.trim();
      if (!id) return;
      const obj = asRecord(payload);
      const content = stripAgentInternalReasoningLeaks(
        str(obj.content, ""),
      ).trim();
      if (!content) return;
      const runId = str(obj.run_id, str(obj.runId, "")).trim();
      const traceId = str(obj.trace_id, str(obj.traceId, "")).trim();
      const modelUsed = str(obj.model_used, str(obj.model, "")).trim();
      const choices = clarificationChoices(obj.choices);
      const metricFields = chatRunMetricMessageFieldsFromPayload(payload);
      const timestamp = new Date().toISOString();
      queryClient.setQueryData(["chat-messages", id], (previous: unknown) => {
        const previousObj = asRecord(previous);
        const records = pickRecords(previous, "messages");
        const alreadyPresentIndex = records.findIndex((message) => {
          const role = str(message.role, "").trim().toLowerCase();
          if (role !== "assistant") return false;
          if (
            stripAgentInternalReasoningLeaks(
              str(message.content, ""),
            ).trim() === content
          ) {
            return true;
          }
          const messageTraceId = str(message.trace_id, str(message.traceId, "")).trim();
          return Boolean(traceId && messageTraceId === traceId);
        });
        if (alreadyPresentIndex >= 0) {
          const existing = asRecord(records[alreadyPresentIndex]);
          const shouldAddChoices =
            choices.length > 0 &&
            clarificationChoices(existing.choices).length === 0;
          const shouldAddTraceId =
            Boolean(traceId) &&
            !str(existing.trace_id, str(existing.traceId, "")).trim();
          const shouldAddMetrics = Object.keys(metricFields).length > 0;
          if (!shouldAddChoices && !shouldAddTraceId && !shouldAddMetrics) {
            return previous;
          }
          const nextRecords = records.map((record, idx) =>
            idx === alreadyPresentIndex
              ? {
                  ...record,
                  ...(shouldAddTraceId ? { trace_id: traceId } : {}),
                  ...(shouldAddMetrics ? metricFields : {}),
                  ...(shouldAddChoices ? { choices } : {}),
                }
              : record,
          );
          return Array.isArray(previous)
            ? nextRecords
            : {
                ...previousObj,
                messages: nextRecords,
              };
        }
        const nextMessage: JsonRecord = {
          id: `stream:${runId || traceId || timestamp}`,
          conversation_id: id,
          role: "assistant",
          content,
          timestamp,
          model_used: modelUsed || "stream",
          trace_id: traceId,
          ...metricFields,
          ...(choices.length > 0 ? { choices } : {}),
          optimistic: true,
        };
        if (Array.isArray(previous)) {
          return [...records, nextMessage];
        }
        return {
          ...previousObj,
          messages: [...records, nextMessage],
        };
      });
    },
    [queryClient],
  );

  const recoverAssistantMessageFromLatestRun = useCallback(
    async (
      targetConversationId: string,
      options?: { expectedRunId?: string; minRunUpdatedAtMs?: number },
    ): Promise<boolean> => {
      const id = targetConversationId.trim();
      if (!id) return false;
      const payload = asRecord(
        await api.rawGet(`/conversations/${encodeURIComponent(id)}/latest-run`),
      );
      const run = asRecord(payload.run);
      if (Object.keys(run).length === 0) return false;
      const runConversationId = str(
        run.conversation_id,
        str(run.conversationId, id),
      ).trim();
      if (runConversationId && runConversationId !== id) return false;
      const expectedRunId = str(options?.expectedRunId, "").trim();
      const runId = str(run.id, "").trim();
      if (expectedRunId && runId && runId !== expectedRunId) return false;
      const assistantPayload = extractLatestRunAssistantContentPayload(
        payload,
        id,
        expectedRunId || runId,
      );
      if (!assistantPayload) return false;
      appendAssistantContentToConversationCache(id, assistantPayload);
      return true;
    },
    [appendAssistantContentToConversationCache],
  );

  const removeApprovalChoicesFromConversationCache = useCallback(
    (targetConversationId: string, approvalId: string) => {
      const id = targetConversationId.trim();
      const normalizedApprovalId = approvalId.trim();
      if (!id || !normalizedApprovalId) return;
      queryClient.setQueryData(["chat-messages", id], (previous: unknown) => {
        const previousObj = asRecord(previous);
        const records = pickRecords(previous, "messages");
        if (records.length === 0) return previous;
        let changed = false;
        const nextRecords = records.map((record) => {
          const choices = clarificationChoices(record.choices);
          if (choices.length === 0) return record;
          const nextChoices = choices.filter((choice) => {
            const approval =
              choice.approval ?? parseInternalApprovalSubmitToken(choice.submitText);
            return approval?.id !== normalizedApprovalId;
          });
          if (nextChoices.length === choices.length) return record;
          changed = true;
          return nextChoices.length > 0
            ? { ...record, choices: nextChoices }
            : Object.fromEntries(
                Object.entries(record).filter(([key]) => key !== "choices"),
              );
        });
        if (!changed) return previous;
        return Array.isArray(previous)
          ? nextRecords
          : {
              ...previousObj,
              messages: nextRecords,
            };
      });
    },
    [queryClient],
  );

  const refreshConversationMessagesFromStreamPayload = useCallback(
    (
      payload: unknown,
      fallbackConversationId = "",
      minAssistantMessages?: number,
      options?: { settle?: boolean; minLatestAssistantCreatedAtMs?: number },
    ) => {
      const id = streamPayloadConversationId(payload, fallbackConversationId);
      if (!id) return;
      appendAssistantContentToConversationCache(id, payload);
      void refreshConversationMessagesAfterStream(
        id,
        minAssistantMessages,
        options,
      ).catch(async () => {
        const recovered = await recoverAssistantMessageFromLatestRun(id, {
          expectedRunId: streamPayloadRunId(payload),
          minRunUpdatedAtMs: options?.minLatestAssistantCreatedAtMs,
        }).catch(() => false);
        if (recovered) return;
        void queryClient.invalidateQueries({ queryKey: ["chat-messages", id] });
      });
    },
    [
      appendAssistantContentToConversationCache,
      queryClient,
      recoverAssistantMessageFromLatestRun,
      refreshConversationMessagesAfterStream,
    ],
  );

  async function syncPendingRunFromLatestRun(
    pendingConversationId: string,
    snapshot: ChatPendingRunSnapshot,
    options?: { expectedRunId?: string; allowTerminalClear?: boolean },
  ): Promise<"none" | "active" | "terminal"> {
    if (!pendingConversationId) return "none";
    const payload = asRecord(
      await api.rawGet(
        `/conversations/${encodeURIComponent(pendingConversationId)}/latest-run`,
      ),
    );
    const run = asRecord(payload.run);
    if (Object.keys(run).length === 0) return "none";
    const runConversationId = str(
      run.conversation_id,
      str(run.conversationId, pendingConversationId),
    ).trim();
    if (runConversationId && runConversationId !== pendingConversationId) {
      return "none";
    }
    const expectedRunId = str(options?.expectedRunId, "").trim();
    const runId = str(run.id, "").trim();
    if (expectedRunId && runId && runId !== expectedRunId) {
      return "none";
    }
    const runTimestampMs = Date.parse(
      str(run.updated_at, str(run.created_at, "")),
    );
    if (
      Number.isFinite(runTimestampMs) &&
      runTimestampMs + 1000 < snapshot.startedAt
    ) {
      return "none";
    }
    const persistedSteps = trimTrailingHeartbeatSteps(
      buildPersistedRunSteps(asRecords(payload.events)),
    );
    if (persistedSteps.length > 0) {
      setStreamingStepsNow(persistedSteps);
    }
    const latestRunAssistantPayload = extractLatestRunAssistantContentPayload(
      payload,
      pendingConversationId,
      expectedRunId || runId,
    );
    if (latestRunAssistantPayload) {
      appendAssistantContentToConversationCache(
        pendingConversationId,
        latestRunAssistantPayload,
      );
    }
    if (runId) {
      setPendingRunSnapshot((prev) => {
        const base = prev ?? pendingRunSnapshotRef.current ?? snapshot;
        if (
          base.conversationId &&
          base.conversationId !== pendingConversationId
        ) {
          return base;
        }
        const next: ChatPendingRunSnapshot = {
          ...base,
          conversationId: pendingConversationId,
          runId,
          ...(persistedSteps.length > 0 ? { streamingSteps: persistedSteps } : {}),
        };
        storeChatPendingRunSnapshotNow(next);
        return next;
      });
    }
    const status = str(run.status, str(run.run_status, "")).trim().toLowerCase();
    const terminal =
      !!str(run.completed_at, "").trim() ||
      (!!status && !isActiveExecutionRunStatus(status));
    if (!terminal) {
      return runId ? "active" : "none";
    }
    if (!options?.allowTerminalClear) {
      return "terminal";
    }
    clearPendingRunPresentation(persistedSteps);
    await queryClient.invalidateQueries({
      queryKey: ["chat-conversations"],
    });
    try {
      await refreshConversationMessagesAfterStream(
        pendingConversationId,
        undefined,
        {
          settle: true,
          minLatestAssistantCreatedAtMs: snapshot.startedAt,
        },
      );
    } catch {
      if (!latestRunAssistantPayload) {
        const recovered = await recoverAssistantMessageFromLatestRun(
          pendingConversationId,
          {
            expectedRunId: expectedRunId || runId,
            minRunUpdatedAtMs: snapshot.startedAt,
          },
        ).catch(() => false);
        if (recovered) return "terminal";
        await queryClient.invalidateQueries({
          queryKey: ["chat-messages", pendingConversationId],
        });
      }
    }
    return "terminal";
  }

  const humanizeStep = (
    title: string,
    detail: string,
    stepType: string,
  ): { label: string; detail: string; kind?: string; tone?: string } => {
    const t = title.toLowerCase();
    if (stepType === "run_status" || t.startsWith("run status:")) {
      const runStatus = title
        .split(":")
        .slice(1)
        .join(":")
        .trim()
        .toLowerCase()
        .replace(/\s+/g, "_");
      if (runStatus === "completed") {
        return {
          label: "Run completed",
          detail: detail || "Delivered the final response.",
          kind: "Done",
          tone: "tone-success",
        };
      }
      if (runStatus === "degraded") {
        return {
          label: "Run degraded",
          detail: detail || "Some model attempts or execution paths failed.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "cancelled") {
        return {
          label: "Run cancelled",
          detail: detail || "This run was cancelled before completion.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "blocked") {
        return {
          label: "Run blocked",
          detail:
            detail || "This request is blocked and needs operator attention.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "platform_failed") {
        return {
          label: "Run failed",
          detail:
            detail ||
            "The framework hit an internal failure before completion.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (
        runStatus === "service_unavailable" ||
        runStatus === "hard_service_outage"
      ) {
        return {
          label: "Run failed",
          detail:
            detail ||
            "The model service failed before this run could continue.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "needs_input" || runStatus === "needs_clarification") {
        return {
          label: "Needs input",
          detail:
            detail || "The agent needs more information before continuing.",
          kind: "Issue",
          tone: "tone-thinking",
        };
      }
      if (runStatus === "needs_credentials") {
        return {
          label: "Needs credentials",
          detail:
            detail ||
            "Valid credentials are required before this run can continue.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "needs_permission") {
        return {
          label: "Needs permission",
          detail: detail || "Operator approval is required before continuing.",
          kind: "Issue",
          tone: "tone-thinking",
        };
      }
      if (runStatus === "needs_integration") {
        return {
          label: "Needs integration",
          detail: detail || "A missing integration is blocking this run.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (runStatus === "needs_stronger_model") {
        return {
          label: "Needs stronger model",
          detail:
            detail ||
            "The configured model pool could not handle this request.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
    }
    if (stepType === "plan_step_update" || t.startsWith("plan step ")) {
      if (t.includes("failed")) {
        return {
          label: title || "Plan step failed",
          detail: detail || "",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      if (t.includes("completed") || t.includes("skipped")) {
        return {
          label: title || "Plan step completed",
          detail: detail || "",
          kind: "Done",
          tone: "tone-success",
        };
      }
      if (
        t.includes("started") ||
        t.includes("running") ||
        t.includes("queued") ||
        t.includes("updated")
      ) {
        return {
          label: title || "Plan step updated",
          detail: detail || "",
          kind: "Running",
          tone: "tone-action",
        };
      }
    }
    // Log-style: short typed label + actual detail from the step data
    if (
      t === "message received" ||
      t.startsWith("message received") ||
      t === "request received" ||
      t.startsWith("request received")
    ) {
      return { label: "Reading your request", detail: detail || "" };
    }
    if (t === "memory layer" || t.startsWith("memory layer")) {
      return {
        label: "Loading memory",
        detail: detail || "Checking saved context and preferences",
      };
    }
    if (t === "memory retrieval" || t.startsWith("memory retrieval")) {
      return {
        label: "Searching memory",
        detail: detail || "Looking for related past conversations",
      };
    }
    if (t === "context packing" || t.startsWith("context packing")) {
      return {
        label: "Building context",
        detail: detail || "Assembling conversation history",
      };
    }
    if (t === "llm routing decision" || t.startsWith("llm routing decision")) {
      return {
        label: "Choosing strategy",
        detail: detail || "Deciding the best execution approach",
      };
    }
    if (t === "model selection" || t.startsWith("model selection")) {
      return {
        label: "Selecting model",
        detail: detail || "Picking the best available model",
      };
    }
    if (t === "llm request" || t.startsWith("llm request")) {
      return {
        label: "Thinking",
        detail: detail || "Sending request to AI model",
      };
    }
    if (t === "autopilot proceed" || t.startsWith("autopilot proceed")) {
      return {
        label: "Running autonomously",
        detail: detail || "Proceeding without user input",
      };
    }
    if (t.startsWith("tool started:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      return {
        label: runningActivityTitleForToolName(rawName),
        detail: detail || "",
        kind: "Running",
        tone: "tone-action",
      };
    }
    if (t.startsWith("tool finished:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      const summarized = summarizeActivityDetail(detail);
      if (
        isSafetyPolicyBlockedText(detail) ||
        isSafetyPolicyBlockedText(summarized)
      ) {
        return {
          label: `${toHumanToolName(rawName)} blocked`,
          detail:
            "Blocked by safety policy. The agent needs a different approach.",
          kind: "Issue",
          tone: "tone-error",
        };
      }
      return {
        label: `${toHumanToolName(rawName)} completed`,
        detail:
          summarized && !isHumanReadableStatus(summarized)
            ? summarized
            : summarized &&
                summarized !== `${toHumanToolName(rawName)} completed`
              ? summarized
              : "",
        kind: "Done",
        tone: "tone-success",
      };
    }
    if (t.startsWith("tool progress:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      const statusOnly = isStandaloneActivityStatusLabel(
        formatActivityToolName(rawName),
      );
      return {
        label: runningActivityTitleForToolName(rawName),
        detail: detail || (statusOnly ? "" : "Working..."),
        kind: "Running",
        tone: "tone-action",
      };
    }
    if (stepType.includes("tool_progress") && title.trim()) {
      return {
        label: title.trim(),
        detail: detail || "Working...",
        kind: "Running",
        tone: "tone-action",
      };
    }
    if (stepType.includes("tool_start")) {
      return {
        label: "Executing action",
        detail: detail || "",
        kind: "Running",
        tone: "tone-action",
      };
    }
    if (stepType.includes("tool_progress")) {
      return {
        label: "Action in progress",
        detail: detail || "",
        kind: "Running",
        tone: "tone-action",
      };
    }
    if (stepType.includes("tool_result")) {
      return {
        label: "Action completed",
        detail: detail || "",
        kind: "Done",
        tone: "tone-success",
      };
    }
    if (t.includes("approval")) {
      return {
        label: "Waiting for approval",
        detail: detail || "",
        tone: "tone-thinking",
      };
    }
    if (t === "response complete" || t.startsWith("response complete")) {
      return {
        label: "Response delivered",
        detail: detail || "",
        kind: "Done",
        tone: "tone-success",
      };
    }
    if (
      t === "llm response received" ||
      t.startsWith("llm response received")
    ) {
      return {
        label: "Response received",
        detail: detail || "Model finished generating",
        kind: "Update",
      };
    }
    if (
      t === "self evolve" ||
      t.startsWith("self evolve") ||
      t.startsWith("running self evolve")
    ) {
      return {
        label: "Self-evolving",
        detail: detail || "Autonomous code modification started",
        kind: "Running",
        tone: "tone-action",
      };
    }
    // Fallback: use raw title as-is
    const fallbackLabel = title || stepType.replace(/[_-]+/g, " ").trim();
    return { label: fallbackLabel, detail };
  };

  const toolProgressPresentationFromStep = (
    step: JsonRecord,
    stepType: string,
    fallbackDetail: string,
  ): ToolProgressPresentation | null => {
    if (!stepType.includes("tool_progress")) return null;
    const data = activityDataRecord(step.data);
    if (Object.keys(data).length === 0) return null;

    if (str(data.kind, "").trim() === "agent_loop_progress") {
      return agentLoopProgressPresentation(data, fallbackDetail);
    }

    const name = str(data.tool_name, str(data.name, "")).trim();
    if (!name) return null;
    const content = str(data.content, fallbackDetail).trim();
    return buildToolProgressPresentation(name, content, data, "");
  };

  const isHeartbeatStreamingStep = (value: JsonRecord): boolean => {
    const icon = normalizeStatusText(str(value.icon, ""));
    const stepType = normalizeStatusText(
      str(value.step_type, str(value.type, "")),
    );
    return (
      icon === "wait" ||
      stepType.includes("heartbeat") ||
      toBool(value.is_heartbeat)
    );
  };

  const streamingStepDedupKey = (value: JsonRecord): string => {
    const stepType = normalizeStatusText(
      str(value.step_type, str(value.type, "step")),
    );
    const title = normalizeStatusText(str(value.title, ""));
    const detail = normalizeStatusText(str(value.detail, ""));
    return `${stepType}|${title}|${detail}`;
  };

  const streamingStepDisplayKey = (value: JsonRecord): string => {
    const title = normalizeStatusText(str(value.title, ""));
    const detail = normalizeStatusText(str(value.detail, ""));
    return `${title}|${detail}`;
  };

  const getStreamingStepStableKey = (value: JsonRecord): string =>
    str(value.__streamKey, str(value.id, ""));

  const isCompleteModelProseStep = (value: JsonRecord): boolean => {
    const data = asRecord(value.data);
    return (
      normalizeStatusText(str(data.kind, "")) === "model_prose" &&
      !str(data.content_delta, "").trim() &&
      !str(data.content_snapshot, "").trim()
    );
  };

  const streamingStepStructuralStableKey = (value: JsonRecord): string => {
    const explicitKey = getStreamingStepStableKey(value);
    const data = asRecord(value.data);
    const stepType = normalizeStatusText(
      str(value.step_type, str(value.type, "")),
    );
    const dataKind = normalizeStatusText(str(data.kind, ""));
    if ((
      stepType === "reasoning_delta" ||
      dataKind === "reasoning_delta" ||
      dataKind === "model_prose"
    ) && !isCompleteModelProseStep(value)) {
      const eventStreamKey = str(data.stream_key, str(value.stream_key, "")).trim();
      if (eventStreamKey) return eventStreamKey;
      const phase = normalizeReasoningPhase(
        str(data.phase, str(value.phase, "")),
      );
      const toolName =
        normalizeStatusText(str(data.tool_name, str(value.tool_name, ""))) ||
        "reasoning";
      const runId = str(data.run_id, str(value.run_id, "")).trim();
      return ["reasoning", runId, phase, toolName]
        .filter((part) => part.trim())
        .join(":");
    }

    if (stepType === "plan_step_update") {
      const planId = str(value.plan_id, str(data.plan_id, "")).trim();
      const revision = str(value.revision, str(data.revision, "")).trim();
      const stepId = str(value.step_id, str(data.step_id, "")).trim();
      if (planId || stepId) {
        return ["plan_step_update", planId, revision, stepId]
          .filter((part) => part.trim())
          .join(":");
      }
    }

    if (stepType === "tool_progress") {
      const streamKey = str(data.stream_key, str(value.stream_key, ""))
        .trim();
      if (streamKey) return streamKey;
      const dataKind = normalizeStatusText(str(data.kind, ""));
      if (dataKind === "phase_status") {
        const toolName =
          normalizeStatusText(str(data.tool_name, str(data.name, ""))) ||
          "tool";
        const phase =
          normalizeStatusText(str(data.phase, str(value.phase, ""))) ||
          "active";
        return `phase-status:${toolName}:${phase}`;
      }
    }

    return explicitKey;
  };

  const isStreamedModelTextStep = (value: JsonRecord): boolean => {
    const data = asRecord(value.data);
    const stepType = normalizeStatusText(
      str(value.step_type, str(value.type, "")),
    );
    const dataKind = normalizeStatusText(str(data.kind, ""));
    return (
      !isCompleteModelProseStep(value) &&
      (stepType === "reasoning_delta" ||
        dataKind === "reasoning_delta" ||
        dataKind === "model_prose")
    );
  };

  const streamedStepText = (value: JsonRecord): string => {
    const data = asRecord(value.data);
    const stepType = normalizeStatusText(
      str(value.step_type, str(value.type, "")),
    );
    const dataKind = normalizeStatusText(str(data.kind, ""));
    const done = toBool(data.done) || toBool(value.done);
    const contentText = str(
      data.content_snapshot,
      str(data.content, str(data.content_delta, "")),
    );
    if (
      done &&
      !contentText.trim() &&
      (stepType === "reasoning_delta" || dataKind === "reasoning_delta")
    ) {
      return "";
    }
    return str(
      data.content_snapshot,
      str(data.content, str(data.content_delta, str(value.detail, ""))),
    );
  };

  const mergeStreamedModelTextStep = (
    previous: JsonRecord,
    incoming: JsonRecord,
  ): JsonRecord => {
    const prevText = streamedStepText(previous);
    const incomingText = streamedStepText(incoming);
    const mergedText =
      incomingText && prevText && !incomingText.startsWith(prevText)
        ? `${prevText}${incomingText}`
        : incomingText || prevText;
    const incomingData = asRecord(incoming.data);
    return {
      ...incoming,
      detail: stripAgentControlArtifacts(mergedText),
      data: {
        ...incomingData,
        content: stripAgentControlArtifacts(mergedText),
        content_snapshot: stripAgentControlArtifacts(mergedText),
      },
    };
  };

  const attachStreamingStepStableKey = (
    value: JsonRecord,
    preferredKey?: string,
  ): JsonRecord => {
    const existing = getStreamingStepStableKey(value);
    if (existing) return value;
    return {
      ...value,
      __streamKey:
        preferredKey || `stream-step-${streamingStepKeySeqRef.current++}`,
    };
  };

  // Stable identities matter here: these builders feed the dep arrays of the
  // transcript memos below — a per-render arrow silently turns those memos
  // into every-render recomputes across the whole message list.
  const buildStepCard = useCallback((
    step: JsonRecord,
    index: number,
  ): ActivityTimelineCard => {
    const stepType = str(step.step_type, str(step.type, "step")).toLowerCase();
    const title = str(step.title, "").trim();
    const reasoningStep = isMainChatReasoningStep(step);
    const fullDetail = reasoningStep
      ? streamedStepText(step) || extractStepDetailText(step, Number.MAX_SAFE_INTEGER)
      : extractStepDetailText(step, 2800);
    const rawDetail = fullDetail.slice(0, 900);
    const progressPresentation = toolProgressPresentationFromStep(
      step,
      stepType,
      rawDetail,
    );
    const human = humanizeStep(
      progressPresentation?.title || title,
      progressPresentation?.detail || rawDetail,
      stepType,
    );
    const humanDetailRaw = str(human.detail, "").trim();
    const summarizedDetail = humanDetailRaw
      ? summarizeActivityDetail(humanDetailRaw)
      : "";
    let detail = summarizedDetail
      ? simplifyConsoleDetail(summarizedDetail)
      : "";
    const time = str(step.time, "");
    const baseLabel = stepType.replace(/[_-]+/g, " ").trim() || "step";
    const rawLabel = human.label || title || baseLabel;
    // Only capitalize if label doesn't contain file paths/extensions
    const label = /\.\w{1,5}\b|\//.test(rawLabel)
      ? rawLabel
      : rawLabel.replace(/\b\w/g, (ch) => ch.toUpperCase());
    let tone = "tone-neutral";
    let kind = "Update";
    const labelLower = label.toLowerCase();
    if (stepType.includes("tool_start")) {
      tone = "tone-tool";
      kind = "Running";
    } else if (stepType.includes("tool_progress")) {
      tone = "tone-action";
      kind = "Running";
    } else if (
      stepType.includes("tool_result") ||
      stepType.includes("result") ||
      stepType.includes("complete") ||
      stepType.includes("success")
    ) {
      tone = "tone-success";
      kind = "Done";
    } else if (stepType === "info") {
      tone = "tone-neutral";
      kind = "Done";
    } else if (stepType.includes("error") || stepType.includes("fail")) {
      tone = "tone-error";
      kind = "Issue";
    } else if (
      stepType.includes("think") ||
      stepType.includes("plan") ||
      stepType.includes("reason")
    ) {
      tone = "tone-thinking";
      kind = "Planning";
    } else if (stepType.includes("action") || stepType.includes("execute")) {
      tone = "tone-action";
      kind = "Running";
    } else if (
      stepType.includes("response") ||
      stepType.includes("final") ||
      stepType.includes("summary")
    ) {
      tone = "tone-synthesis";
      kind = "Done";
    } else if (
      /start|running|loading|checking|choosing|selecting|generating/.test(
        labelLower,
      )
    ) {
      tone = "tone-action";
      kind = "Running";
    } else if (/finished|complete|done|recorded|generated/.test(labelLower)) {
      tone = "tone-success";
      kind = "Done";
    } else if (/issue|error|failed|blocked/.test(labelLower)) {
      tone = "tone-error";
      kind = "Issue";
    }
    if (isSafetyPolicyBlockedText(`${label} ${detail}`)) {
      tone = "tone-error";
      kind = "Issue";
    }
    if (human.tone) tone = human.tone;
    if (human.kind) kind = human.kind;
    let detailFull = summarizedDetail;
    if (isRedundantStatusDetail(label, detail)) {
      detail = "";
    }
    if (
      detailFull &&
      (isRedundantStatusDetail(label, detailFull) ||
        (detail && isRedundantStatusDetail(detail, detailFull)))
    ) {
      detailFull = "";
    }
    const stableId = getStreamingStepStableKey(step);
    const hideStructuredPayload =
      stepType === "checkpoint" || stepType === "run_status";
    const hidePayloadView =
      hideStructuredPayload || stepType === "reasoning_delta";
    const rawDetailFull = hideStructuredPayload
      ? ""
      : humanDetailRaw
        ? fullDetail || rawDetail
        : "";
    const payloadView = hidePayloadView
      ? null
      : buildActivityPayloadViewFromSources(step.data, rawDetailFull);
    const surface =
      surfaceFromValue(step, stableId || `${time || "live"}-${index}`) ||
      surfaceFromValue(step.data, stableId || `${time || "live"}-${index}`) ||
      null;
    const summary = detailFull || detail;
    return {
      id: stableId || `${time || "live"}-${index}-${label}`,
      index,
      stepType,
      rawTitle: title || baseLabel,
      tone,
      kind,
      label,
      detail,
      detailFull,
      summary,
      rawDetailFull,
      traceJson: hideStructuredPayload ? "" : fullTraceJson(step),
      payloadView,
      isHeartbeat: isHeartbeatStreamingStep(step),
      time,
      surface,
    };
  }, []);

  const safeBuildStepCard = useCallback((step: unknown, index: number) => {
    const record = asRecord(step);
    try {
      return buildStepCard(record, index);
    } catch {
      const stepType = str(
        record.step_type,
        str(record.type, "step"),
      ).toLowerCase();
      const title = str(record.title, "").trim();
      const rawDetail = extractStepDetailText(record, 600);
      const label =
        title || stepType.replace(/[_-]+/g, " ").trim() || "Activity update";
      const stableId = getStreamingStepStableKey(record);
      const safeDetail = rawDetail
        ? simplifyConsoleDetail(summarizeActivityDetail(rawDetail))
        : "";
      const hideStructuredPayload =
        stepType === "checkpoint" || stepType === "run_status";
      const hidePayloadView =
        hideStructuredPayload || stepType === "reasoning_delta";
      return {
        id:
          stableId || `${str(record.time, "live") || "live"}-${index}-${label}`,
        index,
        stepType,
        rawTitle: title || label,
        tone: "tone-neutral",
        kind: "Update",
        label,
        detail: safeDetail,
        detailFull: "",
        summary: safeDetail,
        rawDetailFull: hideStructuredPayload ? "" : rawDetail,
        traceJson: hideStructuredPayload ? "" : fullTraceJson(record),
        payloadView: hidePayloadView
          ? null
          : buildActivityPayloadViewFromSources(rawDetail, record.data),
        isHeartbeat: false,
        time: str(record.time, ""),
        surface:
          surfaceFromValue(record, stableId || `${str(record.time, "live") || "live"}-${index}`) ||
          surfaceFromValue(record.data, stableId || `${str(record.time, "live") || "live"}-${index}`) ||
          null,
      };
    }
  }, [buildStepCard]);

  const buildChatTranscriptItemsFromSteps = useCallback((
    sourceSteps: JsonRecord[],
    keyPrefix: string,
    maxItems = 28,
    options?: { complete?: boolean },
  ): ChatTranscriptItem[] => {
    const steps = compressActivitySteps(
      trimTrailingHeartbeatSteps(sourceSteps),
    );
    const items: ChatTranscriptItem[] = [];
    const proseSeen = new Set<string>();
    const actionIndicesByTool = new Map<string, number[]>();
    let reasoningItemIndex = -1;
    let latestAgentLoopActionKey = "";
    let pendingProse: Extract<ChatTranscriptItem, { kind: "prose" }> | null =
      null;

    const toolKey = (toolName: string, card: ActivityTimelineCard) => {
      const normalizedTool = toolName.trim().toLowerCase();
      if (normalizedTool) return normalizedTool;
      return (card.rawTitle || card.label || card.stepType).trim().toLowerCase();
    };

    const actionTitle = (toolName: string, card: ActivityTimelineCard) =>
      inlineToolActivityTitle(toolName, card);

    const detailLabel = (type: string, card: ActivityTimelineCard) => {
      if (type === "tool_start") return "Input";
      if (type === "tool_result") return "Result";
      if (type === "tool_progress") return card.label || "Step";
      return card.kind || "Step";
    };

    const buildActionDetail = (
      type: string,
      card: ActivityTimelineCard,
      fallbackDetail: string,
    ): ChatTranscriptActionDetail | null => {
      const audit = transcriptCommandAuditFromCard(card);
      const auditDetail =
        type === "tool_start"
          ? audit?.command || ""
          : type === "tool_result"
            ? audit?.output || ""
            : "";
      const detail = compactTranscriptDetail(
        auditDetail || card.summary || card.detail || fallbackDetail,
      );
      if (!detail && type === "tool_progress") return null;
      return {
        id: `${card.id}:${type}`,
        label: detailLabel(type, card),
        detail: detail || card.label || "Updated.",
        status:
          type === "tool_result"
            ? transcriptStatusFromCard(card) === "running"
              ? "done"
              : transcriptStatusFromCard(card)
            : transcriptStatusFromCard(card),
        card,
      };
    };

    const appendActionDetail = (
      action: Extract<ChatTranscriptItem, { kind: "action" }>,
      detail: ChatTranscriptActionDetail | null,
    ) => {
      if (!detail) return;
      const duplicate = action.details.some(
        (entry) =>
          entry.label === detail.label &&
          entry.detail === detail.detail &&
          entry.status === detail.status,
      );
      if (duplicate) return;
      action.details = [...action.details.slice(-7), detail];
    };

    const buildReasoningDetail = (
      type: string,
      card: ActivityTimelineCard,
      fallbackDetail: string,
    ): ChatTranscriptActionDetail | null => {
      const detail = compactTranscriptDetail(
        card.summary || card.detail || fallbackDetail,
      );
      if (!detail) return null;
      return {
        id: `${card.id}:${type}:reasoning`,
        label: card.label || detailLabel(type, card),
        detail,
        status: transcriptStatusFromCard(card),
        card,
      };
    };

    const appendReasoningDetail = (
      detail: ChatTranscriptActionDetail | null,
    ) => {
      if (!detail) return;
      let item =
        reasoningItemIndex >= 0 ? items[reasoningItemIndex] : undefined;
      if (!item || item.kind !== "reasoning") {
        const nextItem: ChatTranscriptItem = {
          kind: "reasoning",
          id: `${keyPrefix}:reasoning`,
          title: "Reasoning",
          detail: detail.detail,
          status: detail.status,
          details: [detail],
        };
        reasoningItemIndex = items.length;
        items.push(nextItem);
        return;
      }
      const duplicate = item.details.some(
        (entry) =>
          entry.label === detail.label &&
          entry.detail === detail.detail &&
          entry.status === detail.status,
      );
      item = {
        ...item,
        detail: detail.detail || item.detail,
        status:
          detail.status === "issue"
            ? "issue"
            : item.status === "running"
              ? detail.status
              : item.status,
        details: duplicate
          ? item.details
          : [...item.details.slice(-14), detail],
      };
      items[reasoningItemIndex] = item;
    };

    const pushPendingProse = () => {
      if (!pendingProse) return;
      const key = normalizeAgentProseKey(pendingProse.text);
      if (key && !proseSeen.has(key)) {
        proseSeen.add(key);
        items.push(pendingProse);
      }
      pendingProse = null;
    };

    const rememberActionIndex = (key: string, index: number) => {
      const existing = actionIndicesByTool.get(key) || [];
      actionIndicesByTool.set(key, [...existing, index]);
    };

    const latestMatchingAction = (key: string) => {
      const indices = actionIndicesByTool.get(key) || [];
      for (let idx = indices.length - 1; idx >= 0; idx -= 1) {
        const item = items[indices[idx]];
        if (item?.kind === "action" && item.status === "running") {
          return indices[idx];
        }
      }
      return indices.length > 0 ? indices[indices.length - 1] : -1;
    };

    const runLooksComplete = () =>
      Boolean(options?.complete) ||
      steps.some((step) => {
        const data = activityDataRecord(step.data);
        const combined = [
          activityStepType(step),
          str(step.title, ""),
          str(step.detail, ""),
          str(data.status, str(data.run_status, "")),
        ]
          .join(" ")
          .toLowerCase();
        return /\b(completed|complete|succeeded|success|done)\b/.test(combined);
      });

    const completeTranscriptItems = (value: ChatTranscriptItem[]) =>
      value.map((item) => {
        if (item.kind !== "action" && item.kind !== "reasoning") return item;
        if (item.status === "issue") return item;
        return {
          ...item,
          status: "done" as ChatTranscriptActionStatus,
          details: item.details.map((detail) =>
            detail.status === "issue"
              ? detail
              : { ...detail, status: "done" as ChatTranscriptActionStatus },
          ),
        };
      });

    for (let idx = 0; idx < steps.length; idx += 1) {
      const step = steps[idx];
      if (isHeartbeatStreamingStep(step)) continue;
      if (isInternalChatTranscriptStep(step)) continue;
      const stepType = activityStepType(step);
      const card = safeBuildStepCard(step, idx);
      const internalReasoningText = modelInternalReasoningTextFromActivityStep(step);
      if (internalReasoningText) {
        // Internal reasoning belongs in Run Details / Computer, not as a
        // transient chat transcript row that disappears when streaming ends.
        continue;
      }
      const proseText = modelProseTextFromActivityStep(step);
      if (proseText) {
        pendingProse = {
          kind: "prose",
          id: `${keyPrefix}:prose:${card.id}`,
          text: proseText,
        };
        continue;
      }
      const agentLoopPhase = agentLoopProgressPhaseFromStep(step);
      if (isMainChatReasoningStep(step)) {
        // Planning/classifier/model reasoning is useful diagnostic context,
        // but the chat lane should stay stable and response-focused.
        continue;
      }
      if (agentLoopPhase === "tool_execution") {
        pushPendingProse();
        const actionNames = agentLoopProgressActionNamesFromStep(step);
        const toolName = actionNames[0] || activityToolNameFromStep(step);
        const syntheticKey = toolName
          ? toolName.trim().toLowerCase()
          : "agent-loop-actions";
        latestAgentLoopActionKey = syntheticKey;
        const title =
          actionNames.length > 1
            ? `${actionNames.length} Actions`
            : actionTitle(toolName, card);
        const audit = transcriptCommandAuditFromCard(card);
        const detail = inlineToolActivityDetail(
          toolName,
          card,
          audit?.command || card.summary || card.detail || str(step.detail, ""),
        );
        const childDetail = buildActionDetail(
          "tool_start",
          card,
          str(step.detail, ""),
        );
        const action: ChatTranscriptItem = {
          kind: "action",
          id: `${keyPrefix}:action:${card.id}`,
          card,
          toolName,
          title,
          detail,
          status: "running",
          details: childDetail ? [childDetail] : [],
        };
        const actionIndex = items.length;
        items.push(action);
        rememberActionIndex(syntheticKey, actionIndex);
        continue;
      }
      if (agentLoopPhase === "tool_result") {
        const actionNames = agentLoopProgressActionNamesFromStep(step);
        const toolName = actionNames[0] || activityToolNameFromStep(step);
        const syntheticKey =
          latestAgentLoopActionKey ||
          (toolName ? toolName.trim().toLowerCase() : "agent-loop-actions");
        const status = transcriptStatusFromCard(card);
        const audit = transcriptCommandAuditFromCard(card);
        const detail = inlineToolActivityDetail(
          toolName,
          card,
          audit?.output || card.summary || card.detail || str(step.detail, ""),
        );
        const matchIndex = latestMatchingAction(syntheticKey);
        if (matchIndex >= 0) {
          const item = items[matchIndex];
          if (item.kind === "action") {
            item.card = card;
            item.status = status === "running" ? "done" : status;
            item.detail = detail || item.detail;
            appendActionDetail(
              item,
              buildActionDetail("tool_result", card, str(step.detail, "")),
            );
          }
        } else {
          pushPendingProse();
          const childDetail = buildActionDetail(
            "tool_result",
            card,
            str(step.detail, ""),
          );
          const action: ChatTranscriptItem = {
            kind: "action",
            id: `${keyPrefix}:action:${card.id}`,
            card,
            toolName,
            title: actionTitle(toolName, card),
            detail,
            status: status === "running" ? "done" : status,
            details: childDetail ? [childDetail] : [],
          };
          const actionIndex = items.length;
          items.push(action);
          rememberActionIndex(syntheticKey, actionIndex);
        }
        continue;
      }
      if (stepType === "tool_start") {
        const data = activityDataRecord(step.data);
        if (!pendingProse && items.length === 0) {
          const fallbackProse = cleanAgentProseText(toolStartIntentText(data));
          if (fallbackProse) {
            pendingProse = {
              kind: "prose",
              id: `${keyPrefix}:prose:${card.id}:tool-intent`,
              text: fallbackProse,
            };
          }
        }
        pushPendingProse();
        const toolName = activityToolNameFromStep(step);
        const title = actionTitle(toolName, card);
        const audit = transcriptCommandAuditFromCard(card);
        const detail = inlineToolActivityDetail(
          toolName,
          card,
          audit?.command ||
            toolStartIntentText(data) ||
            card.summary ||
            card.detail ||
            str(step.detail, ""),
        );
        const childDetail = buildActionDetail(
          stepType,
          card,
          toolStartIntentText(data) || str(step.detail, ""),
        );
        const action: ChatTranscriptItem = {
          kind: "action",
          id: `${keyPrefix}:action:${card.id}`,
          card,
          toolName,
          title,
          detail,
          status: "running",
          details: childDetail ? [childDetail] : [],
        };
        const actionIndex = items.length;
        items.push(action);
        rememberActionIndex(toolKey(toolName, card), actionIndex);
        continue;
      }
      if (stepType === "tool_progress") {
        // Progress events update an existing action's detail and substeps.
        // Orphan progress events (no matching parent tool_start) are dropped
        // from the chat surface — they remain visible in Run Details / Computer.
        const toolName = activityToolNameFromStep(step);
        const key = toolKey(toolName, card);
        const matchIndex = latestMatchingAction(key);
        if (matchIndex >= 0) {
          const detail = inlineToolActivityDetail(
            toolName,
            card,
            card.summary || card.detail || str(step.detail, ""),
          );
          const item = items[matchIndex];
          if (item.kind === "action") {
            item.card = card;
            item.status = transcriptStatusFromCard(card);
            if (detail) item.detail = detail;
            appendActionDetail(
              item,
              buildActionDetail(stepType, card, str(step.detail, "")),
            );
          }
        }
        continue;
      }
      if (stepType === "tool_result") {
        const toolName = activityToolNameFromStep(step);
        const key = toolKey(toolName, card);
        const status = transcriptStatusFromCard(card);
        const audit = transcriptCommandAuditFromCard(card);
        const detail = inlineToolActivityDetail(
          toolName,
          card,
          audit?.output || card.summary || card.detail || str(step.detail, ""),
        );
        const matchIndex = latestMatchingAction(key);
        if (matchIndex >= 0) {
          const item = items[matchIndex];
          if (item.kind === "action") {
            item.card = card;
            item.status = status === "running" ? "done" : status;
            item.detail = detail || item.detail;
            appendActionDetail(
              item,
              buildActionDetail(stepType, card, str(step.detail, "")),
            );
          }
        } else {
          pushPendingProse();
          const childDetail = buildActionDetail(
            stepType,
            card,
            str(step.detail, ""),
          );
          const action: ChatTranscriptItem = {
            kind: "action",
            id: `${keyPrefix}:action:${card.id}`,
            card,
            toolName,
            title: actionTitle(toolName, card),
            detail,
            status: status === "running" ? "done" : status,
            details: childDetail ? [childDetail] : [],
          };
          const actionIndex = items.length;
          items.push(action);
          rememberActionIndex(key, actionIndex);
        }
        continue;
      }
    }

    const finalItems = runLooksComplete() ? completeTranscriptItems(items) : items;
    // Keep each tool action as its own sequential row. Collapsing consecutive
    // same-tool actions into a single counted row (×N) hid the individual
    // calls — expanding the group only ever surfaced one — so we render them
    // unmerged in execution order.
    return finalItems.slice(-Math.max(1, maxItems));
  }, [safeBuildStepCard]);

  const streamingTraceCards = useMemo(
    () =>
      streamingSteps
        .map((step, idx) => safeBuildStepCard(step, idx))
        .slice(-24),
    [streamingSteps],
  );
  const pickPrimaryActivityCard = (
    cards: ActivityTimelineCard[],
  ): ActivityTimelineCard | null => {
    if (cards.length === 0) return null;
    const last = cards[cards.length - 1];
    if (!last.isHeartbeat) return last;
    return [...cards].reverse().find((card) => !card.isHeartbeat) || last;
  };
  const streamingActivity = useMemo(() => {
    const last = pickPrimaryActivityCard(streamingTraceCards);
    if (!last) return "Thinking...";
    const kind = (last.kind || "").toLowerCase();
    if (kind.includes("planning") || kind.includes("thinking"))
      return "Thinking...";
    if (kind.includes("memory") || kind.includes("loading"))
      return "Recalling context...";
    if (kind.includes("done") || kind.includes("update"))
      return "Writing response...";
    return "Working...";
  }, [streamingTraceCards]);

  const traceSummaryText = (
    cards: ActivityTimelineCard[],
    opts?: { loading?: boolean; streaming?: boolean; error?: string },
  ) => {
    if (opts?.error) return "Activity details unavailable.";
    if (opts?.loading && cards.length === 0) return "View activity";
    if (cards.length === 0)
      return opts?.streaming
        ? "Waiting for first activity update..."
        : "View activity";
    const last = pickPrimaryActivityCard(cards) || cards[cards.length - 1];
    const count = countMeaningfulActivityCards(cards);
    return `${count} update${count === 1 ? "" : "s"} | Now: ${last.label}`;
  };

  const traceSummaryFromSteps = (
    steps: JsonRecord[],
    opts?: { loading?: boolean; streaming?: boolean; error?: string },
  ) => {
    if (opts?.error) return "Activity details unavailable.";
    if (opts?.loading && steps.length === 0) return "View activity";
    if (steps.length === 0)
      return opts?.streaming
        ? "Waiting for first activity update..."
        : "View activity";
    const normalizedSteps = compressActivitySteps(steps);
    let normalizedPrimaryIndex = normalizedSteps.length - 1;
    for (let i = normalizedSteps.length - 1; i >= 0; i -= 1) {
      if (!isHeartbeatStreamingStep(normalizedSteps[i])) {
        normalizedPrimaryIndex = i;
        break;
      }
    }
    const normalizedPrimaryCard = safeBuildStepCard(
      normalizedSteps[normalizedPrimaryIndex],
      normalizedPrimaryIndex,
    );
    const normalizedCount =
      normalizedSteps.filter((step) => !isHeartbeatStreamingStep(step))
        .length || normalizedSteps.length;
    return `${normalizedCount} update${normalizedCount === 1 ? "" : "s"} | Now: ${normalizedPrimaryCard.label}`;
    let primaryIndex = steps.length - 1;
    for (let i = steps.length - 1; i >= 0; i -= 1) {
      if (!isHeartbeatStreamingStep(steps[i])) {
        primaryIndex = i;
        break;
      }
    }
    const primaryCard = safeBuildStepCard(steps[primaryIndex], primaryIndex);
    return `${steps.length} update${steps.length === 1 ? "" : "s"} - Now: ${primaryCard.label}`;
  };

  const parseTraceSteps = (payload: unknown): JsonRecord[] => {
    const rec = asRecord(payload);
    const raw = Array.isArray(rec.steps)
      ? rec.steps
      : Array.isArray(rec.trace)
        ? rec.trace
        : [];
    const rawSteps = raw
      .filter((x) => x && typeof x === "object")
      .map((x) => normalizeActivityStepTime(asRecord(x)));
    const checkpointRunSteps = buildTraceCheckpointRunSteps(rawSteps);
    const sourceSteps =
      checkpointRunSteps.length > 0
        ? [
            ...rawSteps.filter(
              (step) =>
                !isTraceCheckpointStep(step) && isMainChatReasoningStep(step),
            ),
            ...checkpointRunSteps,
          ]
        : rawSteps;
    const steps = sanitizeActivityStepsForUi(
      compressActivitySteps(
        sourceSteps,
      ),
    );
    return steps.length > CHAT_STREAMING_STEPS_UI_MAX
      ? steps.slice(-CHAT_STREAMING_STEPS_UI_MAX)
      : steps;
  };

  const loadTraceForId = async (traceId: string) => {
    if (!traceId) return;
    if (
      traceStepsById[traceId] ||
      traceLoadingById[traceId] ||
      traceErrorById[traceId]
    )
      return;
    setTraceLoadingById((prev) => ({ ...prev, [traceId]: true }));
    setTraceErrorById((prev) => ({ ...prev, [traceId]: "" }));
    try {
      const payload = await api.rawGet(`/trace/${encodeURIComponent(traceId)}`);
      const steps = parseTraceSteps(payload);
      setTraceStepsById((prev) => ({ ...prev, [traceId]: steps }));
    } catch (err) {
      const raw = errMessage(err);
      let normalized = raw;
      try {
        const parsed = JSON.parse(raw) as { error?: string; message?: string };
        normalized = parsed.error || parsed.message || raw;
      } catch {
        // keep raw
      }
      if (/trace/i.test(normalized) && /not found/i.test(normalized)) {
        normalized = "Detailed activity is not available for this response.";
      }
      setTraceErrorById((prev) => ({ ...prev, [traceId]: normalized }));
    } finally {
      setTraceLoadingById((prev) => ({ ...prev, [traceId]: false }));
    }
  };

  const getTraceStepsForExport = async (
    traceId: string,
  ): Promise<JsonRecord[]> => {
    if (!traceId) return [];
    if (traceStepsById[traceId]) {
      return traceStepsById[traceId];
    }
    try {
      const payload = await api.rawGet(`/trace/${encodeURIComponent(traceId)}`);
      const steps = parseTraceSteps(payload);
      setTraceStepsById((prev) => ({ ...prev, [traceId]: steps }));
      return steps;
    } catch {
      return [];
    }
  };

  const detachStreamingRunToBackground = () => {
    if (!isStreaming && !streamLockRef.current) return false;
    const activeSnapshot = pendingRunSnapshotRef.current ?? pendingRunSnapshot;
    const backgroundSnapshot = activeSnapshot
      ? movePendingRunSnapshotToBackground()
      : null;
    const streamGeneration = streamGenerationRef.current;
    if (streamGeneration > 0) {
      backgroundDetachGenerationsRef.current.add(streamGeneration);
    }
    streamAbortRef.current?.abort();
    setPendingRunSnapshot(null);
    storeChatPendingRunSnapshotNow(null);
    if (backgroundSnapshot) {
      setChatNotice(
        "Moved the active run to the background. You can keep chatting elsewhere.",
      );
    }
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setExecutionPlan(null);
    setExecutionPlanFailure("");
    setExecutionPlanExpanded(false);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
    setStreamPhaseStatus(null);
    clearReasoningPreview();
    setLiveFileWrites({});
    setDeployedFiles([]);
    setIsStreaming(false);
    setLiveRunStreamOpenNow(false);
    setIsStoppingStream(false);
    streamLockRef.current = false;
    activeChatTaskIdRef.current = null;
    return true;
  };

  const startNewConversation = (options?: { preserveCurrentRun?: boolean }) => {
    setDraftChatMode(true);
    const preserveCurrentRun = options?.preserveCurrentRun ?? true;
    let detachedCurrentRun = false;
    if (preserveCurrentRun && (isStreaming || streamLockRef.current)) {
      detachStreamingRunToBackground();
      detachedCurrentRun = true;
    } else if (
      preserveCurrentRun &&
      (pendingRunSnapshotRef.current ?? pendingRunSnapshot)?.conversationId
    ) {
      movePendingRunSnapshotToBackground();
    }
    if (!detachedCurrentRun) {
      streamAbortRef.current?.abort();
    }
    streamAbortRef.current = null;
    stopRequestedRef.current = false;
    activeChatTaskIdRef.current = null;
    reattachedRunIdRef.current = "";
    writeChatRouteConversationId(null);
    streamLockRef.current = false;
    setIsStreaming(false);
    setLiveRunStreamOpenNow(false);
    setIsStoppingStream(false);
    setPendingRunSnapshot(null);
    storeChatPendingRunSnapshotNow(null);
    if (typeof window !== "undefined") {
      try {
        window.sessionStorage.removeItem(CHAT_LAST_CONVERSATION_STORAGE_KEY);
      } catch {
        // Ignore storage failures.
      }
    }
    dragDepthRef.current = 0;
    setIsDragOverChat(false);
    setConversationId(null);
    setConversationPage(0);
    queueComposerPrefill({ text: "", browser_profile_context: null });
    setDeepResearchEnabled(false);
    setAttachedFiles([]);
    setChatError(null);
    setChatNotice(null);
    setPlanConfirmation(null);
    setResearchReportDialog(null);
    lastWorkspaceRestoreSeedRef.current = "";
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setExecutionPlan(null);
    setExecutionPlanFailure("");
    setExecutionPlanExpanded(false);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
    setTraceStepsById({});
    setTraceLoadingById({});
    setTraceErrorById({});
    setLastRunSteps([]);
    setCompletedProgressMessagesByConversation({});
    setLiveFileWrites({});
    setDeployedFiles([]);
    setSelectedSnippetId(null);
    setStreamPhaseStatus(null);
    clearReasoningPreview();
    setStreamedWorkspaceApp(null);
    streamedWorkspaceAppRef.current = null;
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    pendingFileReadPathRef.current = "";
    pendingFileWritePathRef.current = "";
    if (
      typeof window !== "undefined" &&
      window.innerWidth < CHAT_INLINE_CONVERSATIONS_MIN_WIDTH
    ) {
      setConversationSidebarOpen(false);
    }
  };

  const openConversationById = (id: string) => {
    if (!id) return;
    setDraftChatMode(false);
    setChatError(null);
    if (conversationId === id) {
      writeChatRouteConversationId(id);
      return;
    }
    if (isStreaming || streamLockRef.current) {
      detachStreamingRunToBackground();
    } else if (
      (pendingRunSnapshotRef.current ?? pendingRunSnapshot)?.conversationId &&
      (pendingRunSnapshotRef.current ?? pendingRunSnapshot)?.conversationId !==
        id
    ) {
      movePendingRunSnapshotToBackground();
    }
    lastWorkspaceRestoreSeedRef.current = "";
    setPlanConfirmation(null);
    setResearchReportDialog(null);
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setExecutionPlan(null);
    setExecutionPlanFailure("");
    setExecutionPlanExpanded(false);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
    setTraceStepsById({});
    setTraceLoadingById({});
    setTraceErrorById({});
    setLastRunSteps([]);
    setLiveFileWrites({});
    setDeployedFiles([]);
    setSelectedSnippetId(null);
    setStreamPhaseStatus(null);
    clearReasoningPreview();
    setStreamedWorkspaceApp(null);
    streamedWorkspaceAppRef.current = null;
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    pendingFileReadPathRef.current = "";
    pendingFileWritePathRef.current = "";
    setConversationId(id);
    if (
      typeof window !== "undefined" &&
      window.innerWidth < CHAT_INLINE_CONVERSATIONS_MIN_WIDTH
    ) {
      setConversationSidebarOpen(false);
    }
    const restoredSnapshot =
      pendingRunSnapshot?.conversationId === id
        ? pendingRunSnapshot
        : backgroundRunSnapshots[id]
          ? backgroundRunSnapshots[id]
          : null;
    if (restoredSnapshot && backgroundRunSnapshots[id]) {
      const nextBackgroundSnapshots = { ...backgroundRunSnapshots };
      delete nextBackgroundSnapshots[id];
      setBackgroundRunSnapshots(nextBackgroundSnapshots);
      storeChatBackgroundRunSnapshots(nextBackgroundSnapshots);
      setPendingRunSnapshot(restoredSnapshot);
      storeChatPendingRunSnapshotNow(restoredSnapshot);
    }
    if (restoredSnapshot?.conversationId === id) {
      if (restoredSnapshot.message) {
        setPendingUserMessage(restoredSnapshot.message);
      }
      if (restoredSnapshot.failedUserMessage) {
        setFailedUserMessage(restoredSnapshot.failedUserMessage);
      }
      if (restoredSnapshot.streamingResponse) {
        setStreamingResponseNow(restoredSnapshot.streamingResponse);
      }
      if (
        Array.isArray(restoredSnapshot.streamingSteps) &&
        restoredSnapshot.streamingSteps.length > 0
      ) {
        setStreamingStepsNow(restoredSnapshot.streamingSteps);
      }
    }
  };

  const getConversationFallbackAfterDelete = (
    deletedId: string,
  ): string | null => {
    const orderedIds = orderedSidebarConversationIds.filter(Boolean);
    if (orderedIds.length === 0) return null;
    const deletedIndex = orderedIds.findIndex((id) => id === deletedId);
    const remainingIds = orderedIds.filter((id) => id !== deletedId);
    if (remainingIds.length === 0) return null;
    if (deletedIndex < 0) return remainingIds[0];
    return (
      remainingIds[Math.min(deletedIndex, remainingIds.length - 1)] ||
      remainingIds[0]
    );
  };

  useEffect(() => {
    if (
      !selectedConversationNotFound ||
      !conversationId ||
      postDeleteConversationFallback
    )
      return;
    if (
      pendingRunSnapshot ||
      isStreaming ||
      pendingUserMessage ||
      failedUserMessage ||
      streamingResponse ||
      messages.length > 0
    ) {
      return;
    }
    const preferredFallbackId =
      getConversationFallbackAfterDelete(conversationId);
    setPostDeleteConversationFallback({
      deletedId: conversationId,
      preferredId: preferredFallbackId,
    });
    startNewConversation({ preserveCurrentRun: false });
    setChatNotice(
      preferredFallbackId
        ? "That chat is no longer available. Loaded another conversation."
        : "That chat is no longer available.",
    );
  }, [
    selectedConversationNotFound,
    conversationId,
    postDeleteConversationFallback,
    pendingRunSnapshot,
    isStreaming,
    pendingUserMessage,
    failedUserMessage,
    streamingResponse,
    messages.length,
  ]);

  const queueAttachedFiles = (files: FileList | File[] | null) => {
    if (!files || files.length === 0) return;
    const incoming = Array.from(files as ArrayLike<File>);
    const { accepted, rejected } = splitSupportedChatAttachments(incoming);
    if (rejected.length > 0) {
      const preview = rejected.slice(0, 3).join(", ");
      const extra =
        rejected.length > 3 ? ` (+${rejected.length - 3} more)` : "";
      setChatNotice(`Skipped unsupported files: ${preview}${extra}`);
    }
    if (accepted.length === 0) return;
    setAttachedFiles((prev) => {
      const merged = [...prev];
      for (const file of accepted) {
        const exists = merged.some(
          (f) =>
            f.name === file.name &&
            f.size === file.size &&
            f.lastModified === file.lastModified,
        );
        if (!exists) merged.push(file);
      }
      return merged.slice(0, 8);
    });
  };

  const handleChatDragEnter = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current += 1;
    if (!isStreaming) setIsDragOverChat(true);
  };

  const handleChatDragOver = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    if (!isStreaming && !isDragOverChat) setIsDragOverChat(true);
  };

  const handleChatDragLeave = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) setIsDragOverChat(false);
  };

  const handleChatDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current = 0;
    setIsDragOverChat(false);
    if (isStreaming) return;
    queueAttachedFiles(event.dataTransfer?.files ?? null);
  };

  // Capture pasted screenshots / files from anywhere inside the chat shell.
  // Plain-text paste is left alone (no preventDefault) so typing still works
  // normally; we only intercept when the clipboard actually carries file
  // items. A pasted image from the OS shortcut (e.g. Win+Shift+S) arrives
  // as a `kind === "file"` item with `type === "image/png"` and an empty or
  // generic name, which `splitSupportedChatAttachments` already accepts.
  const handleChatPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    if (isStreaming) return;
    const clipboard = event.clipboardData;
    if (!clipboard || clipboard.items.length === 0) return;
    const files: File[] = [];
    for (const item of Array.from(clipboard.items)) {
      if (item.kind !== "file") continue;
      const file = item.getAsFile();
      if (file) files.push(file);
    }
    if (files.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    queueAttachedFiles(files);
  };

  const removeAttachedFile = (idx: number) => {
    setAttachedFiles((prev) => prev.filter((_, i) => i !== idx));
  };

  type IndexedKnowledgeAttachment = {
    id: string;
    filename: string;
    chunks: number;
  };
  type UploadedVisualAttachment = {
    id: string;
    filename: string;
    path: string;
    size: number;
    contentType: string;
  };
  const uploadAttachmentsForKnowledge = async (files: File[]) => {
    if (files.length === 0)
      return {
        documents: [] as IndexedKnowledgeAttachment[],
        visuals: [] as UploadedVisualAttachment[],
      };
    const documents: IndexedKnowledgeAttachment[] = [];
    const visuals: UploadedVisualAttachment[] = [];
    for (const file of files) {
      const formData = new FormData();
      formData.append("file", file, file.name);
      if (isVisualChatAttachment(file) && !isKnowledgeChatAttachment(file)) {
        const out = asRecord(await api.rawPostForm("/api/upload", formData));
        const rows = pickRecords(out, "files");
        const uploadedFile = rows[0] ?? {};
        const id = str(uploadedFile.id, "");
        if (!id) {
          throw new Error(`Failed to upload '${file.name}'.`);
        }
        visuals.push({
          id,
          filename: str(uploadedFile.name, file.name),
          path: str(uploadedFile.path, ""),
          size: num(uploadedFile.size, file.size),
          contentType: file.type || "image",
        });
        continue;
      }

      const out = asRecord(
        await api.rawPostForm("/documents/upload-file", formData),
      );
      const id = str(out.id, "");
      if (!id) {
        throw new Error(`Failed to index '${file.name}'.`);
      }
      documents.push({
        id,
        filename: str(out.filename, file.name),
        chunks: num(out.chunks, 0),
      });
    }
    return { documents, visuals };
  };

  const copyText = async (value: string) => {
    const text = value.trim();
    if (!text) throw new Error("Nothing to copy.");
    const nav = typeof navigator !== "undefined" ? navigator : null;
    if (nav && nav.clipboard?.writeText) {
      await nav.clipboard.writeText(text);
      return;
    }
    const doc = typeof document !== "undefined" ? document : null;
    if (!doc) throw new Error("Clipboard is not available.");
    const ta = doc.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.left = "-9999px";
    doc.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = doc.execCommand("copy");
    doc.body.removeChild(ta);
    if (!ok) throw new Error("Copy failed.");
  };

  const normalizeChatError = (raw: string): string => {
    let message = (raw || "").trim();
    if (!message) return "Something went wrong while running this request.";

    for (let i = 0; i < 3; i += 1) {
      const withoutPrefix = message.replace(/^error:\s*/i, "").trim();
      if (withoutPrefix !== message) {
        message = withoutPrefix;
        continue;
      }
      try {
        const parsed = JSON.parse(message) as unknown;
        if (typeof parsed === "string") {
          const next = parsed.trim();
          if (next && next !== message) {
            message = next;
            continue;
          }
          break;
        }
        const obj = asRecord(parsed);
        const extracted = [
          str(obj.error, ""),
          str(obj.message, ""),
          str(obj.detail, ""),
          str(obj.reason, ""),
        ]
          .map((v) => v.trim())
          .find(Boolean);
        if (extracted && extracted !== message) {
          message = extracted;
          continue;
        }
      } catch {
        // keep raw text when it is not JSON
      }
      break;
    }

    if (
      (/localhost:11434/i.test(message) ||
        /127\.0\.0\.1:11434/i.test(message)) &&
      (/error sending request for url/i.test(message) ||
        /connection refused/i.test(message) ||
        /provider instability/i.test(message))
    ) {
      return "No working local model is available. If you want Ollama, start it and load a model. Otherwise configure a model in Settings > Models.";
    }
    if (
      /missing ['"`]?files['"`]?/i.test(message) ||
      /object mapping filename to content/i.test(message)
    ) {
      return "Deploy payload was malformed - the LLM did not provide a valid `files` object. The agent has been given details to self-correct on retry.";
    }
    if (
      /error decoding response body/i.test(message) ||
      /error deserializing response body/i.test(message)
    ) {
      return "Model/provider response format mismatch. For GLM, use OpenAI-compatible Chat Completions in Settings > Models (correct base URL + model), or switch to a known-compatible model.";
    }
    if (
      /openai-compatible response schema mismatch/i.test(message) ||
      /openai-compatible response was not valid json/i.test(message) ||
      /openai-compatible api returned an error payload/i.test(message) ||
      /no response from openai/i.test(message)
    ) {
      return "Model/provider response format mismatch. For GLM, confirm OpenAI-compatible Chat Completions support in Settings > Models, or switch to a known-compatible model.";
    }
    if (/syntaxerror/i.test(message) && /(app\.py|python)/i.test(message)) {
      const lineMatch = message.match(/line\s+(\d+)/i);
      const lineHint = lineMatch?.[1] ? ` at line ${lineMatch[1]}` : "";
      return `Generated app code has a Python syntax error${lineHint}. Ask AgentArk to fix the generated file and redeploy.`;
    }
    if (
      /stopped shortly after launch/i.test(message) ||
      /runtime is not active/i.test(message)
    ) {
      return "The deployed app process crashed after startup. Check the validation/error details for runtime logs, then retry deploy with a corrected entry/install command.";
    }
    if (
      /missing authorization:\s*bearer/i.test(message) ||
      /bearer\s*<api_key>/i.test(message) ||
      /api authentication is not configured/i.test(message) ||
      /^unauthorized\b/i.test(message)
    ) {
      return "HTTP API auth is missing or expired. Open Settings > Advanced > API Key (HTTP), regenerate/copy the key, then retry.";
    }
    if (
      /openai api error/i.test(message) ||
      /anthropic api error/i.test(message)
    ) {
      const lower = message.toLowerCase();
      if (
        /missing.*api[_\s-]?key/.test(lower) ||
        /invalid.*api[_\s-]?key/.test(lower) ||
        /authentication/.test(lower) ||
        /unauthorized/.test(lower)
      ) {
        return "A provider API key is missing. Set it in Settings > Models (LLM) or Settings > Integrations (tool-specific keys), then retry.";
      }
      return message;
    }
    if (
      /missing.*api[_\s-]?key/i.test(message) ||
      /no api key available/i.test(message) ||
      /api key.*not configured/i.test(message)
    ) {
      return "A provider API key is missing. Set it in Settings > Models (LLM) or Settings > Integrations (tool-specific keys), then retry.";
    }
    if (/missing.*auth/i.test(message)) {
      return "Authentication is missing for this action. Check Settings > Models and Settings > Advanced > API Key (HTTP).";
    }
    if (
      /http error 404/i.test(message) ||
      /\b404\b.*not found/i.test(message)
    ) {
      return "A page request returned 404 (not found). The agent should switch to another source automatically.";
    }
    if (
      /error executing 'browse'/i.test(message) ||
      /failed to fetch url/i.test(message) ||
      /\bbrowse\b.*\bfailed\b/i.test(message)
    ) {
      return "The web page lookup failed. The agent can continue by searching alternative sources.";
    }
    return message;
  };

  const exportConversationById = async (
    targetId: string,
    titleHint?: string,
  ) => {
    if (!targetId) return;
    setChatError(null);
    try {
      let exportMessages = messages;
      if (conversationId !== targetId || exportMessages.length === 0) {
        const payload = await api.rawGet(
          `/conversations/${encodeURIComponent(targetId)}/messages?limit=200`,
        );
        exportMessages = pickRecords(payload, "messages");
      }
      const title =
        (titleHint || str(selectedConversation?.title, "chat")).trim() ||
        "chat";
      const safe =
        title
          .replace(/[^\w.-]+/g, "_")
          .replace(/^_+|_+$/g, "")
          .toLowerCase() || "chat";
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      const lines: string[] = [];
      lines.push(`# ${title}`);
      lines.push(`conversation_id: ${targetId}`);
      lines.push(`exported_at: ${new Date().toISOString()}`);
      lines.push("");
      for (const message of exportMessages) {
        const role = str(message.role, "assistant");
        const ts = str(message.timestamp, "");
        const content = str(message.content, "");
        lines.push(`${role.toUpperCase()}${ts ? ` (${ts})` : ""}`);
        lines.push(content);
        lines.push("");
      }
      downloadTextFile(
        `${safe}-${stamp}.txt`,
        lines.join("\n"),
        "text/plain;charset=utf-8",
      );
      setChatNotice("Chat exported.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const downloadTextFile = (
    filename: string,
    content: string,
    mimeType = "text/plain;charset=utf-8",
  ) => {
    const blob = new Blob([content], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const exportAssistantMarkdown = async ({
    content,
    headingHint,
    previousUserPrompt,
    timestamp,
    traceId,
    deepResearchHint = false,
  }: {
    content: string;
    headingHint?: string;
    previousUserPrompt?: string;
    timestamp?: string;
    traceId?: string;
    deepResearchHint?: boolean;
  }) => {
    try {
      const normalizedContent = str(content, "").trim();
      if (!normalizedContent) throw new Error("Nothing to export.");
      const prompt = (previousUserPrompt || "").trim();
      const conversationTitle =
        str(selectedConversation?.title, "").trim() || "research";
      const report = parseResearchReportWithContext(normalizedContent, {
        deepResearch: deepResearchHint,
        previousUserPrompt: prompt,
        conversationTitle,
      });
      const cleanTraceId = str(traceId, "").trim();
      const traceSteps = cleanTraceId
        ? await getTraceStepsForExport(cleanTraceId)
        : [];
      const exportPlan = extractExecutionPlanFromTraceSteps(traceSteps);
      const exportPlanFailure =
        extractExecutionPlanFailureFromTraceSteps(traceSteps).trim();
      const heading = deriveAssistantExportHeading(
        normalizedContent,
        report,
        str(headingHint, "").trim(),
        prompt,
        conversationTitle,
      );
      if (report) {
        const cleanReportBody = researchReportExportMarkdown({
          report,
          previousUserPrompt: prompt,
          timestamp,
          traceId,
          preserveChartFences: true,
        });
        const safe =
          heading
            .replace(/[^\w.-]+/g, "_")
            .replace(/^_+|_+$/g, "")
            .toLowerCase()
            .slice(0, 96) || "research";
        const stamp = new Date().toISOString().replace(/[:.]/g, "-");
        downloadTextFile(
          `${safe}-${stamp}.md`,
          cleanReportBody || normalizedContent,
          "text/markdown;charset=utf-8",
        );
        setChatNotice("Report exported.");
        return;
      }
      const summaryBullets = buildAssistantExportSummaryBullets(
        normalizedContent,
        report,
        exportPlan,
        exportPlanFailure,
      );
      const detailedResponse = formatAssistantExportBody(
        normalizedContent,
        heading,
      );
      const safe =
        heading
          .replace(/[^\w.-]+/g, "_")
          .replace(/^_+|_+$/g, "")
          .toLowerCase()
          .slice(0, 96) || "research";
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      const cleanTimestamp = str(timestamp, "").trim();
      const lines: string[] = [];
      lines.push(`# ${heading}`);
      lines.push("");
      lines.push("> Exported from AgentArk.");
      lines.push("");
      if (summaryBullets.length > 0) {
        lines.push("## Executive Summary");
        lines.push("");
        summaryBullets.forEach((bullet) => lines.push(`- ${bullet}`));
        lines.push("");
      }
      lines.push(
        ...buildExecutionPlanExportSection(
          exportPlan,
          exportPlanFailure,
          cleanTraceId,
        ),
      );
      if (prompt) {
        lines.push("## Request");
        lines.push(prompt);
        lines.push("");
      }
      lines.push("## Report Details");
      lines.push("");
      lines.push("| Field | Value |");
      lines.push("| --- | --- |");
      lines.push(`| Conversation | ${conversationId || "Not available"} |`);
      lines.push(`| Assistant time | ${cleanTimestamp || "Not available"} |`);
      lines.push(`| Trace id | ${cleanTraceId || "Not available"} |`);
      lines.push(`| Exported at | ${new Date().toISOString()} |`);
      lines.push("");
      lines.push("## Detailed Response");
      lines.push("");
      lines.push(detailedResponse || normalizedContent);
      lines.push("");
      downloadTextFile(
        `${safe}-${stamp}.md`,
        lines.join("\n"),
        "text/markdown;charset=utf-8",
      );
      setChatNotice("Report exported.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const researchReportExportMarkdown = ({
    report,
    previousUserPrompt,
    timestamp,
    traceId,
    preserveChartFences,
  }: {
    report: ResearchReportPreview;
    previousUserPrompt?: string;
    timestamp?: string;
    traceId?: string;
    preserveChartFences: boolean;
  }): string => {
    const body = cleanResearchReportMarkdownForExport(report, {
      preserveChartFences,
      includeEvidenceBrief: true,
    });
    const prompt = str(previousUserPrompt, "").trim();
    const details: string[] = [];
    if (prompt) {
      details.push("## Request", "", prompt, "");
    }
    details.push("## Report Details", "");
    details.push("| Field | Value |");
    details.push("| --- | --- |");
    details.push(`| Conversation | ${conversationId || "Not available"} |`);
    details.push(`| Assistant time | ${str(timestamp, "").trim() || "Not available"} |`);
    details.push(`| Trace id | ${str(traceId, "").trim() || "Not available"} |`);
    details.push(`| Exported at | ${new Date().toISOString()} |`);
    return [body, details.join("\n")].filter(Boolean).join("\n\n");
  };

  const researchReportExportHtml = ({
    report,
    headingHint,
    previousUserPrompt,
    timestamp,
    traceId,
  }: {
    report: ResearchReportPreview;
    headingHint?: string;
    previousUserPrompt?: string;
    timestamp?: string;
    traceId?: string;
  }): { filenameStem: string; heading: string; html: string } => {
    const heading =
      str(headingHint, "").trim() ||
      report.title ||
      "Research report";
    const markdown = researchReportExportMarkdown({
      report,
      previousUserPrompt,
      timestamp,
      traceId,
      preserveChartFences: true,
    });
    return {
      filenameStem: documentFileStem(heading),
      heading,
      html: reportPrintHtml(heading, markdown),
    };
  };

  const downloadResearchReportHtml = (request: {
    report: ResearchReportPreview;
    headingHint?: string;
    previousUserPrompt?: string;
    timestamp?: string;
    traceId?: string;
  }) => {
    try {
      const prepared = researchReportExportHtml(request);
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      downloadTextFile(
        `${prepared.filenameStem}-${stamp}.html`,
        prepared.html,
        "text/html;charset=utf-8",
      );
      setChatNotice("HTML report downloaded.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const exportResearchReportPdf = (request: {
    report: ResearchReportPreview;
    headingHint?: string;
    previousUserPrompt?: string;
    timestamp?: string;
    traceId?: string;
  }) => {
    try {
      const prepared = researchReportExportHtml(request);
      const printWindow = window.open("", "_blank", "width=980,height=1100");
      if (!printWindow) {
        setChatError("PDF export window was blocked. Allow pop-ups for AgentArk and try again.");
        return;
      }
      printWindow.document.open();
      printWindow.document.write(prepared.html);
      printWindow.document.close();
      printWindow.document.title = prepared.heading;
      printWindow.focus();
      printWindow.setTimeout(() => {
        printWindow.focus();
        printWindow.print();
      }, 300);
      setChatNotice("PDF export opened.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const exportAssistantMessage = async (
    message: JsonRecord,
    previousUserPrompt?: string,
  ) => {
    const content = str(message.content, "").trim();
    const report = parseResearchReportWithContext(content, {
      deepResearch: isDeepResearchAssistantMessage(message),
      previousUserPrompt,
      conversationTitle: str(selectedConversation?.title, ""),
    });
    if (report) {
      downloadResearchReportHtml({
        report,
        headingHint: report.title,
        previousUserPrompt,
        timestamp: str(message.timestamp, "").trim(),
        traceId: str(message.trace_id, "").trim(),
      });
      return;
    }
    await exportAssistantMarkdown({
      content,
      previousUserPrompt,
      timestamp: str(message.timestamp, "").trim(),
      traceId: str(message.trace_id, "").trim(),
      deepResearchHint: isDeepResearchAssistantMessage(message),
    });
  };

  const openResearchReportPreview = ({
    report,
    previousUserPrompt,
    messageId,
    timestamp,
    traceId,
  }: {
    report: ResearchReportPreview;
    previousUserPrompt: string;
    messageId: string;
    timestamp?: string;
    traceId?: string;
  }) => {
    setResearchReportDialog({
      report,
      messageId,
      previousUserPrompt,
      timestamp,
      traceId,
    });
  };

  const copyMessage = async (message: JsonRecord) => {
    try {
      const role = str(message.role, "").toLowerCase();
      const content = str(message.content, "");
      await copyText(
        role === "user" ? stripAttachmentContextMarker(content) : content,
      );
      setChatNotice("Message copied.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const renderResearchReportCard = ({
    report,
    previousUserPrompt,
    messageId,
    timestamp,
    traceId,
    isStreaming = false,
  }: {
    report: ResearchReportPreview;
    previousUserPrompt: string;
    messageId: string;
    timestamp?: string;
    traceId?: string;
    isStreaming?: boolean;
  }) => {
    const metaLabel = isStreaming
      ? `Streaming report preview | ${researchReportMetaLabel(report)}`
      : researchReportMetaLabel(report);
    const summaryText =
      report.summaryPreview ||
      report.highlights[0] ||
      "Open the report to review the full research write-up.";
    const visibleHighlights = report.highlights.slice(0, 3);
    const visibleOpenQuestions = report.openQuestions.slice(0, 2);
    const visibleContradictions = report.contradictions.slice(0, 2);
    return (
      <Box className="chat-research-report-shell">
        <Typography variant="caption" className="chat-research-report-meta">
          {metaLabel}
        </Typography>
        <Box
          className="chat-research-report-card"
        >
          <Stack
            direction="row"
            spacing={1.5}
            sx={{
              justifyContent: "space-between",
              alignItems: "flex-start",
            }}
          >
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <Typography
                variant="body1"
                className="chat-research-report-title"
              >
                {report.title}
              </Typography>
            </Box>
            <Stack
              direction="row"
              spacing={0.5}
              sx={{
                alignItems: "center",
              }}
            >
              <Tooltip
                title={
                  isStreaming
                    ? "Export current draft as PDF"
                    : "Export as PDF"
                }
              >
                <IconButton
                  size="small"
                  className="chat-research-report-action"
                  onClick={(event) => {
                    event.stopPropagation();
                    exportResearchReportPdf({
                      report,
                      headingHint: report.title,
                      previousUserPrompt,
                      timestamp,
                      traceId,
                    });
                  }}
                >
                  <PictureAsPdfRoundedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
              <Tooltip
                title={
                  isStreaming
                    ? "Download current draft as HTML"
                    : "Download HTML report"
                }
              >
                <IconButton
                  size="small"
                  className="chat-research-report-action"
                  onClick={(event) => {
                    event.stopPropagation();
                    downloadResearchReportHtml({
                      report,
                      headingHint: report.title,
                      previousUserPrompt,
                      timestamp,
                      traceId,
                    });
                  }}
                >
                  <ArticleRoundedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
              <Tooltip
                title={
                  isStreaming ? "Open current report draft" : "Open report"
                }
              >
                <IconButton
                  size="small"
                  className="chat-research-report-action"
                  onClick={(event) => {
                    event.stopPropagation();
                    openResearchReportPreview({
                      report,
                      previousUserPrompt,
                      messageId,
                      timestamp,
                      traceId,
                    });
                  }}
                >
                  <OpenInFullRoundedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            </Stack>
          </Stack>
          <Typography
            variant="caption"
            className="chat-research-report-eyebrow"
          >
            Executive summary
          </Typography>
          <Typography variant="body2" className="chat-research-report-summary">
            {summaryText}
          </Typography>
          <Box className="chat-research-report-statbar" aria-label="Report coverage">
            {report.sourceCount > 0 ? (
              <span className="chat-research-report-pill strong">
                {report.sourceCount} source{report.sourceCount === 1 ? "" : "s"}
              </span>
            ) : null}
            {report.tableCount > 0 ? (
              <span className="chat-research-report-pill">
                {report.tableCount} table{report.tableCount === 1 ? "" : "s"}
              </span>
            ) : null}
            {report.chartCount > 0 ? (
              <span className="chat-research-report-pill">
                {report.chartCount} chart{report.chartCount === 1 ? "" : "s"}
              </span>
            ) : null}
            {report.openQuestionCount > 0 ? (
              <span className="chat-research-report-pill">
                {report.openQuestionCount} open question
                {report.openQuestionCount === 1 ? "" : "s"}
              </span>
            ) : null}
            {report.contradictionCount > 0 ? (
              <span className="chat-research-report-pill">
                {report.contradictionCount} contradiction
                {report.contradictionCount === 1 ? "" : "s"}
              </span>
            ) : null}
          </Box>
          {visibleHighlights.length > 0 ? (
            <Box className="chat-research-report-section">
              <Typography
                variant="caption"
                className="chat-research-report-section-title"
              >
                Key findings
              </Typography>
              <Stack component="ol" spacing={0.35} className="chat-research-report-list">
                {visibleHighlights.map((finding, index) => (
                  <Typography
                    component="li"
                    key={`${report.title}-finding-${index}`}
                    variant="body2"
                    className="chat-research-report-finding"
                  >
                    {finding}
                  </Typography>
                ))}
              </Stack>
            </Box>
          ) : null}
          {visibleOpenQuestions.length > 0 || visibleContradictions.length > 0 ? (
            <Box className="chat-research-report-callouts">
              {visibleOpenQuestions.length > 0 ? (
                <Box className="chat-research-report-callout">
                  <Typography
                    variant="caption"
                    className="chat-research-report-section-title"
                  >
                    Open questions
                  </Typography>
                  {visibleOpenQuestions.map((item, index) => (
                    <Typography
                      key={`${report.title}-question-${index}`}
                      variant="body2"
                      className="chat-research-report-finding"
                    >
                      {item}
                    </Typography>
                  ))}
                </Box>
              ) : null}
              {visibleContradictions.length > 0 ? (
                <Box className="chat-research-report-callout warning">
                  <Typography
                    variant="caption"
                    className="chat-research-report-section-title"
                  >
                    Contradictions
                  </Typography>
                  {visibleContradictions.map((item, index) => (
                    <Typography
                      key={`${report.title}-contradiction-${index}`}
                      variant="body2"
                      className="chat-research-report-finding"
                    >
                      {item}
                    </Typography>
                  ))}
                </Box>
              ) : null}
            </Box>
          ) : null}
          <Box className="chat-research-report-footer">
            <Button
              size="small"
              variant="contained"
              className="chat-research-report-primary-action"
              onClick={(event) => {
                event.stopPropagation();
                openResearchReportPreview({
                  report,
                  previousUserPrompt,
                  messageId,
                  timestamp,
                  traceId,
                });
              }}
            >
              Open full report
            </Button>
            <Button
              size="small"
              variant="outlined"
              className="chat-research-report-secondary-action"
              startIcon={<PictureAsPdfRoundedIcon fontSize="small" />}
              onClick={(event) => {
                event.stopPropagation();
                exportResearchReportPdf({
                  report,
                  headingHint: report.title,
                  previousUserPrompt,
                  timestamp,
                  traceId,
                });
              }}
            >
              Export PDF
            </Button>
            <Button
              size="small"
              variant="outlined"
              className="chat-research-report-secondary-action"
              startIcon={<ArticleRoundedIcon fontSize="small" />}
              onClick={(event) => {
                event.stopPropagation();
                downloadResearchReportHtml({
                  report,
                  headingHint: report.title,
                  previousUserPrompt,
                  timestamp,
                  traceId,
                });
              }}
            >
              Download HTML
            </Button>
          </Box>
        </Box>
      </Box>
    );
  };

  const renderPlanConfirmationCard = ({
    threadMode = false,
  }: { threadMode?: boolean } = {}) => (
    <Box
      className={`chat-plan-confirmation-card${threadMode ? " thread" : ""}${planConfirmation?.editing ? " editing" : ""}`}
    >
      {isAwaitingPlanConfirmation ? (
        <Stack spacing={1.5}>
          <Box sx={{ minWidth: 0 }}>
            {planConfirmation?.editing ? (
              <TextField
                size="small"
                multiline
                minRows={2}
                value={planConfirmationSummaryText}
                onChange={(e) =>
                  updatePlanConfirmationDraft((draft) => ({
                    ...draft,
                    summary: e.target.value,
                  }))
                }
                placeholder="Add a short plan title"
                fullWidth
              />
            ) : (
              <Typography
                variant="body1"
                className="chat-plan-confirmation-headline"
              >
                {planConfirmationSummaryText ||
                  planConfirmationOutlineLabel(planConfirmation?.source)}
              </Typography>
            )}
          </Box>

          <Stack spacing={1} className="chat-plan-confirmation-step-list">
            {planConfirmationVisibleSteps.map((step, index) => {
              const stepCopy = describeExecutionPlanStep(
                step,
                `Step ${index + 1}`,
              );
              return (
                <Box
                  key={step.draft_id}
                  className={`chat-plan-confirmation-step${step.enabled ? "" : " disabled"}${planConfirmation?.editing ? " editing" : ""}${expandedPlanSteps.has(step.draft_id) ? " expanded" : ""}`}
                >
                  <Box
                    className={`chat-plan-confirmation-step-marker pending${step.enabled ? "" : " disabled"}${planConfirmation?.editing ? " editing" : ""}`}
                  >
                    {!planConfirmation?.editing && step.enabled ? (
                      <span className="chat-plan-confirmation-step-dot" />
                    ) : step.enabled ? (
                      index + 1
                    ) : (
                      "-"
                    )}
                  </Box>
                  <Box sx={{ minWidth: 0, flex: 1 }}>
                    {planConfirmation?.editing ? (
                      <Stack spacing={0.9}>
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          spacing={1}
                          sx={{
                            alignItems: { sm: "center" },
                          }}
                        >
                          <Checkbox
                            checked={step.enabled}
                            onChange={(e) =>
                              updatePlanConfirmationDraft((draft) => ({
                                ...draft,
                                steps: draft.steps.map((candidate) =>
                                  candidate.draft_id === step.draft_id
                                    ? {
                                        ...candidate,
                                        enabled: e.target.checked,
                                      }
                                    : candidate,
                                ),
                              }))
                            }
                            size="small"
                            sx={{ p: 0.25 }}
                          />
                          <TextField
                            size="small"
                            value={step.title}
                            onChange={(e) =>
                              updatePlanConfirmationDraft((draft) => ({
                                ...draft,
                                steps: draft.steps.map((candidate) =>
                                  candidate.draft_id === step.draft_id
                                    ? { ...candidate, title: e.target.value }
                                    : candidate,
                                ),
                              }))
                            }
                            fullWidth
                            placeholder={`Step ${index + 1}`}
                          />
                        </Stack>
                        <TextField
                          size="small"
                          multiline
                          minRows={2}
                          value={step.description}
                          onChange={(e) =>
                            updatePlanConfirmationDraft((draft) => ({
                              ...draft,
                              steps: draft.steps.map((candidate) =>
                                candidate.draft_id === step.draft_id
                                  ? {
                                      ...candidate,
                                      description: e.target.value,
                                    }
                                  : candidate,
                              ),
                            }))
                          }
                          fullWidth
                          placeholder="Describe what this step should verify or produce"
                        />
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                          }}
                        >
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={index === 0}
                            onClick={() =>
                              updatePlanConfirmationDraft((draft) => {
                                if (index === 0) return draft;
                                const steps = [...draft.steps];
                                [steps[index - 1], steps[index]] = [
                                  steps[index],
                                  steps[index - 1],
                                ];
                                return { ...draft, steps };
                              })
                            }
                          >
                            Move up
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={
                              index === planConfirmationVisibleSteps.length - 1
                            }
                            onClick={() =>
                              updatePlanConfirmationDraft((draft) => {
                                if (index >= draft.steps.length - 1)
                                  return draft;
                                const steps = [...draft.steps];
                                [steps[index], steps[index + 1]] = [
                                  steps[index + 1],
                                  steps[index],
                                ];
                                return { ...draft, steps };
                              })
                            }
                          >
                            Move down
                          </Button>
                        </Stack>
                      </Stack>
                    ) : (
                      <>
                        <Typography
                          variant="body2"
                          className="chat-plan-confirmation-step-title"
                        >
                          {stepCopy.title}
                        </Typography>
                        {stepCopy.description ? (
                          <>
                            <button
                              type="button"
                              className="chat-plan-confirmation-step-toggle"
                              aria-expanded={expandedPlanSteps.has(
                                step.draft_id,
                              )}
                              onClick={() =>
                                togglePlanStepExpansion(step.draft_id)
                              }
                            >
                              <span className="chat-plan-confirmation-step-toggle-icon" />
                              {expandedPlanSteps.has(step.draft_id)
                                ? "Hide details"
                                : "Show details"}
                            </button>
                            <Typography
                              variant="caption"
                              className="chat-plan-confirmation-step-detail"
                            >
                              {stepCopy.description}
                            </Typography>
                          </>
                        ) : null}
                      </>
                    )}
                  </Box>
                </Box>
              );
            })}
          </Stack>

          {!planConfirmation?.editing ? (
            <Typography
              variant="caption"
              className="chat-plan-confirmation-inline-note"
            >
              {planConfirmationEnabledCount} step
              {planConfirmationEnabledCount === 1 ? "" : "s"} selected
              {planConfirmationDisabledCount > 0
                ? `, ${planConfirmationDisabledCount} hidden`
                : ""}
            </Typography>
          ) : null}

          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            className="chat-plan-confirmation-actions"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              flexWrap: "wrap",
            }}
          >
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
              }}
            >
              <Button
                size="small"
                variant="outlined"
                className="chat-plan-confirmation-action ghost"
                onClick={() =>
                  setPlanConfirmation((prev) =>
                    prev
                      ? {
                          ...prev,
                          editing: !prev.editing,
                        }
                      : prev,
                  )
                }
              >
                {planConfirmation?.editing ? "Done editing" : "Edit"}
              </Button>
              {planConfirmation?.editing ? (
                <Button
                  size="small"
                  variant="text"
                  className="chat-plan-confirmation-action text"
                  onClick={resetPlanConfirmationDraft}
                >
                  Reset
                </Button>
              ) : null}
            </Stack>
            <Stack
              direction="row"
              spacing={1}
              useFlexGap
              sx={{
                flexWrap: "wrap",
              }}
            >
              {TASK_CANCEL_CONTROLS_ENABLED ? (
                <Button
                  size="small"
                  variant="outlined"
                  className="chat-plan-confirmation-action ghost"
                  onClick={() => void handlePlanConfirmationCancel()}
                >
                  Cancel
                </Button>
              ) : null}
              <Button
                size="small"
                variant="contained"
                className="chat-plan-confirmation-action primary"
                disabled={
                  !planConfirmationDraftPlan ||
                  planConfirmationEnabledCount === 0 ||
                  isStreaming
                }
                onClick={() => void handlePlanConfirmationStart()}
              >
                Start
              </Button>
            </Stack>
          </Stack>
        </Stack>
      ) : (
        (() => {
          const livePlan = activePlanConfirmationState;
          const liveSteps = livePlan?.steps || [];
          const completedCount = liveSteps.filter(
            (step) => step.status === "completed",
          ).length;
          const runningCount = liveSteps.filter(
            (step) => step.status === "running",
          ).length;
          const failedCount = liveSteps.filter(
            (step) => step.status === "failed",
          ).length;
          const pendingCount = Math.max(
            0,
            liveSteps.length - completedCount - runningCount - failedCount,
          );
          const badgeLabel = isInterruptedPlanConfirmation
            ? "Interrupted"
            : isFailedPlanConfirmation
              ? "Failed"
              : isCompletedPlanConfirmation
                ? "Completed"
                : "Working";
          const failureDetail =
            isInterruptedPlanConfirmation || isFailedPlanConfirmation
              ? latestRunStatusSummary?.detail ||
                chatError ||
                "This research run cannot be resumed. The last saved plan and progress are preserved here."
              : "";
          return (
            <Stack spacing={1.4}>
              <Stack
                direction="row"
                spacing={1}
                useFlexGap
                sx={{
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                  flexWrap: "wrap",
                }}
              >
                <Box sx={{ minWidth: 0, flex: 1 }}>
                  <Typography
                    variant="body1"
                    className="chat-plan-confirmation-headline"
                  >
                    {str(
                      livePlan?.summary,
                      planConfirmationSummaryText ||
                        planConfirmationOutlineLabel(planConfirmation?.source),
                    )}
                  </Typography>
                </Box>
                <Typography
                  variant="caption"
                  className={`chat-plan-confirmation-inline-status ${
                    isInterruptedPlanConfirmation || isFailedPlanConfirmation
                      ? "failed"
                      : isCompletedPlanConfirmation
                        ? "completed"
                        : "running"
                  }`}
                >
                  {badgeLabel}
                </Typography>
              </Stack>
              {runningCount > 0 ? (
                <Box
                  className="chat-plan-confirmation-progress"
                  aria-hidden="true"
                >
                  <Box className="chat-plan-confirmation-progress-fill" />
                </Box>
              ) : null}
              <Typography
                variant="caption"
                className="chat-plan-confirmation-inline-note"
              >
                {runningCount > 0
                  ? `${completedCount} of ${liveSteps.length} complete · working…`
                  : `${completedCount} of ${liveSteps.length} complete${failedCount > 0 ? `, ${failedCount} failed` : ""}`}
              </Typography>
              {failureDetail ? (
                <Stack spacing={0.75}>
                  <Typography
                    variant="caption"
                    className="chat-plan-confirmation-inline-note"
                  >
                    {failureDetail}
                  </Typography>
                  {isSearchBackendSetupIssue(failureDetail) ? (
                    <Box>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={() => onNavigateToView?.("search")}
                      >
                        Open Search Settings
                      </Button>
                    </Box>
                  ) : null}
                </Stack>
              ) : null}
              <Stack
                spacing={0.9}
                className="chat-plan-confirmation-step-list live"
              >
                {liveSteps.map((step, index) => {
                  const stepStatus =
                    str(step.status, "pending").trim() || "pending";
                  const stepCopy = describeExecutionPlanStep(
                    step,
                    `Step ${index + 1}`,
                  );
                  const phaseDrivenSubsteps =
                    stepStatus === "running"
                      ? livePlanPhaseStatuses.map(
                          (phaseStatus, phaseIndex) => ({
                            id: phaseIndex + 1,
                            title: phaseStatus.label,
                            description: phaseStatus.detail,
                            status: phaseStatus.status,
                            tool_hint: phaseStatus.toolName,
                          }),
                        )
                      : [];
                  const displayedSubsteps =
                    phaseDrivenSubsteps.length > 0
                      ? phaseDrivenSubsteps
                      : step.substeps;
                  return (
                    <Box
                      key={`${step.id}-${step.title}`}
                      className={`chat-plan-confirmation-step live ${stepStatus}${expandedPlanSteps.has(step.title) ? " expanded" : ""}`}
                    >
                      <Box
                        className={`chat-plan-confirmation-step-marker live ${stepStatus}`}
                      >
                        {renderExecutionPlanStatusIcon(
                          stepStatus,
                          `chat-plan-confirmation-status-icon${stepStatus === "running" ? " spin" : ""}`,
                        )}
                      </Box>
                      <Box sx={{ minWidth: 0, flex: 1 }}>
                        <Typography
                          variant="body2"
                          className="chat-plan-confirmation-step-title"
                        >
                          {stepCopy.title}
                        </Typography>
                        {stepCopy.description ? (
                          <>
                            <button
                              type="button"
                              className="chat-plan-confirmation-step-toggle"
                              aria-expanded={expandedPlanSteps.has(step.title)}
                              onClick={() =>
                                togglePlanStepExpansion(step.title)
                              }
                            >
                              <span className="chat-plan-confirmation-step-toggle-icon" />
                              {expandedPlanSteps.has(step.title)
                                ? "Hide details"
                                : "Show details"}
                            </button>
                            <Typography
                              variant="caption"
                              className="chat-plan-confirmation-step-detail"
                            >
                              {stepCopy.description}
                            </Typography>
                          </>
                        ) : null}
                        {displayedSubsteps.length > 0 ? (
                          <Stack
                            spacing={0.55}
                            className="chat-plan-confirmation-substeps"
                          >
                            {displayedSubsteps.map((substep, substepIndex) => {
                              const substepStatus =
                                str(substep.status, "pending").trim() ||
                                "pending";
                              const substepCopy = describeExecutionPlanSubstep(
                                substep,
                                `Substep ${substepIndex + 1}`,
                              );
                              return (
                                <Box
                                  key={`${substep.id || substepIndex}-${substep.title}`}
                                  className={`chat-plan-confirmation-substep ${substepStatus}`}
                                >
                                  <Box
                                    className={`chat-plan-confirmation-substep-marker ${substepStatus}`}
                                  >
                                    {renderExecutionPlanStatusIcon(
                                      substepStatus,
                                      `chat-plan-confirmation-status-icon${substepStatus === "running" ? " spin" : ""}`,
                                    )}
                                  </Box>
                                  <Box sx={{ minWidth: 0, flex: 1 }}>
                                    <Typography
                                      variant="caption"
                                      className="chat-plan-confirmation-substep-title"
                                    >
                                      {substepCopy.title}
                                    </Typography>
                                    {substepCopy.description ? (
                                      <Typography
                                        variant="caption"
                                        className="chat-plan-confirmation-substep-detail"
                                      >
                                        {substepCopy.description}
                                      </Typography>
                                    ) : null}
                                  </Box>
                                </Box>
                              );
                            })}
                          </Stack>
                        ) : null}
                      </Box>
                    </Box>
                  );
                })}
              </Stack>
            </Stack>
          );
        })()
      )}
    </Box>
  );

  const toggleConversationStarMutation = useMutation({
    mutationFn: ({ id, starred }: { id: string; starred: boolean }) =>
      api.rawPatch(`/conversations/${encodeURIComponent(id)}`, { starred }),
    onSuccess: async (_data, vars) => {
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({
        queryKey: ["chat-conversation", vars.id],
      });
      setChatNotice(vars.starred ? "Chat starred." : "Chat unstarred.");
    },
    onError: (err) => {
      setChatError(normalizeChatError(errMessage(err)));
    },
  });

  const toggleConversationStar = async (id: string, starred: boolean) => {
    if (!id || toggleConversationStarMutation.isPending) return;
    setChatError(null);
    await toggleConversationStarMutation.mutateAsync({ id, starred });
  };

  const deleteConversationMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/conversations/${encodeURIComponent(id)}`),
    onSuccess: async (_data, id) => {
      const deletedActiveConversation = conversationId === id;
      const preferredFallbackId = deletedActiveConversation
        ? getConversationFallbackAfterDelete(id)
        : null;
      clearChatWorkspaceSnapshot(id);
      clearChatStoredRunSnapshotForConversation(
        CHAT_PENDING_RUN_STORAGE_KEY,
        id,
      );
      clearChatStoredBackgroundRunSnapshot(id);
      setPendingRunSnapshot((prev) =>
        prev?.conversationId === id ? null : prev,
      );
      setBackgroundRunSnapshots((prev) => {
        if (!prev[id]) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      });
      if (deletedActiveConversation) {
        setPostDeleteConversationFallback({
          deletedId: id,
          preferredId: preferredFallbackId,
        });
        startNewConversation({ preserveCurrentRun: false });
      }
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-messages", id] });
      setChatNotice("Chat deleted.");
    },
    onError: (err) => {
      setChatError(normalizeChatError(errMessage(err)));
    },
  });

  const deleteConversation = async (id: string) => {
    if (!id || isStreaming || deleteConversationMutation.isPending) return;
    const shouldDelete =
      typeof window === "undefined"
        ? true
        : window.confirm("Delete this chat and all its messages?");
    if (!shouldDelete) return;
    setChatError(null);
    const taskIds = [pendingRunSnapshot, backgroundRunSnapshots[id] || null]
      .flatMap((snapshot) =>
        snapshot && snapshot.conversationId === id
          ? [str(snapshot.taskId, "").trim()]
          : [],
      )
      .filter(Boolean);
    for (const taskId of [...new Set(taskIds)]) {
      try {
        await api.cancelTask(taskId);
      } catch {
        // Ignore cancellation failures and continue with the destructive delete.
      }
    }
    await deleteConversationMutation.mutateAsync(id);
  };

  const openConversationMenu = (
    event: MouseEvent<HTMLElement>,
    conv: JsonRecord,
  ) => {
    event.stopPropagation();
    setConversationMenuAnchor(event.currentTarget);
    setConversationMenuTarget(conv);
  };

  const closeConversationMenu = () => {
    setConversationMenuAnchor(null);
    setConversationMenuTarget(null);
  };

  const renderConversationCard = (conv: JsonRecord) => {
    const id = str(conv.id, "");
    const active = conversationId === id;
    const starred = toBool(conv.starred);
    const running = active && isStreaming;
    const title =
      str(conv.title, "Untitled").replace(/\s+/g, " ").trim() || "Untitled";
    const updatedAt = str(conv.updated_at, "");
    const updatedAtTooltip = updatedAt
      ? formatChatTimestamp(updatedAt).tooltip
      : "";
    const cardClassName = [
      "conversation-card",
      active ? "active" : "",
      starred ? "conversation-card-starred" : "",
      running ? "is-running" : "",
    ]
      .filter(Boolean)
      .join(" ");
    return (
      <Box
        key={id}
        className={cardClassName}
        onClick={() => {
          openConversationById(id);
        }}
        role="button"
        tabIndex={0}
        title={updatedAtTooltip || undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            openConversationById(id);
          }
        }}
      >
        <Stack
          direction="row"
          spacing={1}
          sx={{
            alignItems: "center",
          }}
        >
          {/* Left-aligned status slot. Running = spinner in AgentArk green,
              otherwise a low-contrast dot that brightens on the active row.
              Reserved width keeps every title aligned regardless of state. */}
          <Box
            className="conversation-card-status"
            aria-label={
              running ? "Conversation is running" : active ? "Active conversation" : ""
            }
            aria-hidden={!running && !active}
          >
            {running ? (
              <CircularProgress size={12} thickness={5} color="inherit" />
            ) : (
              <span className="conversation-card-status-dot" />
            )}
          </Box>
          <Tooltip title={title} placement="top-start" enterDelay={250}>
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <div className="conversation-card-title">{title}</div>
            </Box>
          </Tooltip>
          <Stack
            direction="row"
            spacing={0.25}
            sx={{
              alignItems: "center",
            }}
          >
            <Tooltip title={starred ? "Unstar chat" : "Star chat"}>
              <span>
                <IconButton
                  size="small"
                  className={`conversation-card-star-btn${starred ? " active" : ""}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    void toggleConversationStar(id, !starred);
                  }}
                  disabled={toggleConversationStarMutation.isPending}
                >
                  {starred ? (
                    <StarRoundedIcon fontSize="small" />
                  ) : (
                    <StarBorderRoundedIcon fontSize="small" />
                  )}
                </IconButton>
              </span>
            </Tooltip>
            <Tooltip title="Chat options">
              <span>
                <IconButton
                  size="small"
                  className="conversation-card-menu"
                  onClick={(e) => {
                    openConversationMenu(e, conv);
                  }}
                  disabled={
                    deleteConversationMutation.isPending ||
                    toggleConversationStarMutation.isPending
                  }
                >
                  <MoreVertIcon fontSize="small" />
                </IconButton>
              </span>
            </Tooltip>
          </Stack>
        </Stack>
      </Box>
    );
  };

  const normalizeActivityStepTime = (step: JsonRecord): JsonRecord => {
    const directTime =
      str(step.time, "").trim() ||
      str(step.timestamp, "").trim() ||
      str(step.created_at, "").trim() ||
      str(step.createdAt, "").trim() ||
      str(step.at, "").trim();
    if (!directTime) return step;
    if (str(step.time, "").trim() === directTime) return step;
    return {
      ...step,
      time: directTime,
    };
  };

  const ensureActivityStepTime = (
    step: JsonRecord,
    fallbackTime?: string,
  ): JsonRecord => {
    const normalized = normalizeActivityStepTime(step);
    if (str(normalized.time, "").trim()) return normalized;
    const stamp = (fallbackTime || new Date().toISOString()).trim();
    if (!stamp) return normalized;
    return {
      ...normalized,
      time: stamp,
    };
  };

  const normalizeActivityStepForDisplay = (step: JsonRecord): JsonRecord => {
    const timedStep = normalizeActivityStepTime(
      normalizePlanStepUpdateStep(step),
    );
    if (isHeartbeatStreamingStep(timedStep)) {
      return {
        ...timedStep,
        step_type: "heartbeat",
        title: "Working",
        detail: normalizeHeartbeatDetailText(str(timedStep.detail, "")),
      };
    }

    const title = str(timedStep.title, "");
    const stepType = normalizeStatusText(
      str(timedStep.step_type, str(timedStep.type, "")),
    );
    const detail = str(timedStep.detail, "").trim();
    const lowerTitle = title.toLowerCase();
    const isToolActivity =
      stepType.includes("tool_progress") ||
      stepType.includes("tool_result") ||
      lowerTitle.startsWith("tool progress:") ||
      lowerTitle.startsWith("tool finished:");

    if (!isToolActivity || !detail) return timedStep;
    const summarized = simplifyConsoleDetail(summarizeActivityDetail(detail));
    if (!summarized || summarized === detail) return timedStep;
    return {
      ...timedStep,
      detail: summarized,
    };
  };

  const compressActivitySteps = (steps: JsonRecord[]): JsonRecord[] => {
    const out: JsonRecord[] = [];
    for (const raw of steps) {
      const step = normalizeActivityStepForDisplay(raw);
      const incomingHeartbeat = isHeartbeatStreamingStep(step);
      if (incomingHeartbeat) {
        const lastIdx = out.length - 1;
        if (lastIdx >= 0 && isHeartbeatStreamingStep(out[lastIdx])) {
          if (
            streamingStepDisplayKey(out[lastIdx]) ===
            streamingStepDisplayKey(step)
          ) {
            continue;
          }
          out[lastIdx] = step;
        } else {
          out.push(step);
        }
        continue;
      }
      if (out.length > 0 && isHeartbeatStreamingStep(out[out.length - 1])) {
        out.pop();
      }
      const stableKey = streamingStepStructuralStableKey(step);
      const stableStep = stableKey
        ? attachStreamingStepStableKey(step, stableKey)
        : step;
      if (stableKey) {
        const existingIndex = out.findIndex(
          (row) => streamingStepStructuralStableKey(row) === stableKey,
        );
        if (existingIndex >= 0) {
          const mergedStep =
            isStreamedModelTextStep(stableStep) &&
            isStreamedModelTextStep(out[existingIndex])
              ? attachStreamingStepStableKey(
                  mergeStreamedModelTextStep(out[existingIndex], stableStep),
                  stableKey,
                )
              : stableStep;
          out.splice(existingIndex, 1);
          out.push(mergedStep);
          continue;
        }
      }
      out.push(stableStep);
    }
    return out;
  };

  const trimTrailingHeartbeatSteps = (steps: JsonRecord[]): JsonRecord[] => {
    const next = [...steps];
    while (
      next.length > 0 &&
      isHeartbeatStreamingStep(asRecord(next[next.length - 1]))
    ) {
      next.pop();
    }
    return next;
  };

  const limitStepsPreservingReasoning = (
    steps: JsonRecord[],
    maxSteps: number,
  ): JsonRecord[] => {
    if (steps.length <= maxSteps) return steps;
    const keepIndexes = new Set<number>();
    let heartbeatIndex = -1;
    steps.forEach((step, index) => {
      if (isMainChatReasoningStep(step)) keepIndexes.add(index);
      if (heartbeatIndex < 0 && isHeartbeatStreamingStep(step)) {
        heartbeatIndex = index;
      }
    });
    const heartbeatReserved =
      heartbeatIndex >= 0 && !keepIndexes.has(heartbeatIndex) ? 1 : 0;
    let remaining = Math.max(0, maxSteps - keepIndexes.size - heartbeatReserved);
    for (let index = steps.length - 1; index >= 0 && remaining > 0; index -= 1) {
      if (keepIndexes.has(index) || index === heartbeatIndex) continue;
      keepIndexes.add(index);
      remaining -= 1;
    }
    if (heartbeatIndex >= 0) keepIndexes.add(heartbeatIndex);
    return steps.filter((_, index) => keepIndexes.has(index));
  };

  const limitStreamingStepsForUi = (steps: JsonRecord[]): JsonRecord[] => {
    if (steps.length <= CHAT_STREAMING_STEPS_UI_MAX) return steps;
    return limitStepsPreservingReasoning(
      steps,
      CHAT_STREAMING_STEPS_UI_MAX,
    );
  };

  const limitActivityStepsForRender = (steps: JsonRecord[]): JsonRecord[] => {
    if (steps.length <= CHAT_WORKSPACE_ACTIVITY_RENDER_MAX) return steps;
    return limitStepsPreservingReasoning(
      steps,
      CHAT_WORKSPACE_ACTIVITY_RENDER_MAX,
    );
  };

  const countMeaningfulActivityCards = (
    cards: ActivityTimelineCard[],
  ): number => {
    const nonHeartbeat = cards.filter((card) => !card.isHeartbeat).length;
    return nonHeartbeat > 0 ? nonHeartbeat : cards.length;
  };

  const summarizeToolStartPayload = (
    name: string,
    payload: unknown,
  ): string => {
    const root = asRecord(payload);
    const intentText = toolStartIntentText(root);
    if (intentText) return intentText;
    const normalizedName = (name || "").trim().toLowerCase();
    if (normalizedName === "app_deploy") {
      const nested = asRecord(root.payload);
      // The backend summary uses file_names (array) + file_count (number);
      // fall back to counting keys in a files object when present.
      const rootFiles = asRecord(root.files);
      const nestedFiles = asRecord(nested.files);
      const filesObj =
        Object.keys(rootFiles).length > 0 ? rootFiles : nestedFiles;
      const summaryNames: string[] = Array.isArray(root.file_names)
        ? (root.file_names as string[])
        : [];
      const fileCount =
        typeof root.file_count === "number"
          ? (root.file_count as number)
          : summaryNames.length > 0
            ? summaryNames.length
            : Object.keys(filesObj).length;
      const fileNames =
        summaryNames.length > 0 ? summaryNames : Object.keys(filesObj);
      const entryCommand = str(
        root.entry_command,
        str(nested.entry_command, ""),
      ).trim();
      if (fileCount > 0) {
        const namePreview = fileNames.slice(0, 6).join(", ");
        const overflow = fileCount > 6 ? ` +${fileCount - 6} more` : "";
        return `Deploying ${fileCount} file${fileCount === 1 ? "" : "s"}: ${namePreview}${overflow}${entryCommand ? " (dynamic runtime)" : " (static)"}.`;
      }
      return "Preparing deployment package.";
    }
    if (normalizedName === "file_write") {
      const fileName = normalizeWorkspaceFileName(
        root.path ?? root.file,
        str(streamedWorkspaceAppRef.current?.app_dir, ""),
      );
      const lineCount = Math.max(0, num(root.line_count, 0));
      if (fileName) {
        return lineCount > 0
          ? `Preparing ${fileName} (${lineCount} line${lineCount === 1 ? "" : "s"}).`
          : `Preparing ${fileName}.`;
      }
    }
    const compact = compactUnknown(payload, 320);
    if (!compact) return startingActivitySentenceForToolName(name);
    return simplifyConsoleDetail(compact);
  };

  const pushStreamingStep = (step: JsonRecord) => {
    const normalizedIncomingStep = normalizePlanStepUpdateStep(step);
    // Handle execution plan events
    const stepType = str(normalizedIncomingStep.step_type, "");
    if (stepType === "plan_generated") {
      const nextPlan = normalizeExecutionPlanState(normalizedIncomingStep.plan);
      const pendingPlan = resetExecutionPlanProgress(nextPlan);
      setExecutionPlan(pendingPlan);
      setExecutionPlanFailure("");
      setExecutionPlanExpanded(
        (prev) => prev || planConfirmation?.stage === "running",
      );
    }
    if (stepType === "plan_revised") {
      const nextPlan = normalizeExecutionPlanState(normalizedIncomingStep.plan);
      if (nextPlan) {
        const pendingPlan = resetExecutionPlanProgress(nextPlan);
        setExecutionPlan(pendingPlan);
        setExecutionPlanFailure("");
        setPlanConfirmation((prev) =>
          prev?.stage === "awaiting_confirmation"
            ? {
                ...prev,
                originalPlan: pendingPlan,
                draft: createPlanConfirmationDraft(pendingPlan),
              }
            : prev,
        );
      }
      setExecutionPlanExpanded(
        (prev) => prev || planConfirmation?.stage === "running",
      );
    }
    if (stepType === "plan_ready_for_confirmation") {
      const nextPlan = resetExecutionPlanProgress(
        normalizeExecutionPlanState(normalizedIncomingStep.plan),
      );
      const taskId = str(normalizedIncomingStep.task_id, "").trim();
      const planSource = planConfirmationSourceValue(
        str(normalizedIncomingStep.source, "").trim(),
      );
      setExecutionPlan(nextPlan);
      setExecutionPlanFailure("");
      setExecutionPlanExpanded(false);
      if (isDeepResearchPlanSource(planSource)) {
        setPlanConfirmation({
          stage: "awaiting_confirmation",
          taskId: taskId || null,
          source: planSource,
          originalPlan: nextPlan,
          draft: createPlanConfirmationDraft(nextPlan),
          editing: false,
          messageId: null,
        });
        markPendingRunAwaitingPlanConfirmation(taskId);
        setChatNotice(
          `${planConfirmationDisplayLabel(planSource)} ready. Review it, then Start.`,
        );
      }
    }
    if (stepType === "plan_unavailable") {
      setExecutionPlan(null);
      setExecutionPlanFailure(
        str(
          normalizedIncomingStep.detail,
          "Structured planning was unavailable.",
        ),
      );
      setPlanConfirmation(null);
    }
    if (
      stepType === "plan_step_update" &&
      typeof normalizedIncomingStep.step_id === "number"
    ) {
      const sid = normalizedIncomingStep.step_id as number;
      const newStatus =
        typeof normalizedIncomingStep.status === "string"
          ? normalizedIncomingStep.status
          : "running";
      const planId = str(normalizedIncomingStep.plan_id, "");
      const revision = num(normalizedIncomingStep.revision, 0);
      const nextSubsteps = Array.isArray(normalizedIncomingStep.substeps)
        ? normalizeExecutionPlanSubsteps(normalizedIncomingStep.substeps)
        : null;
      // Don't flip the plan into "running" while the server snapshot still
      // says we're awaiting user confirmation. Stale `plan_step_update`
      // events replayed from the stream history would otherwise fight the
      // approval-repair effect (line 16605), causing the composer placeholder
      // to alternate between "Ask for changes" and "Message" on every render.
      if (pendingSnapshotPhase !== "awaiting_confirmation") {
        setPlanConfirmation((prev) =>
          prev && isDeepResearchPlanSource(prev.source)
            ? {
                ...prev,
                stage: "running",
                editing: false,
              }
            : prev,
        );
      }
      setExecutionPlanExpanded(true);
      setExecutionPlan((prev) => {
        if (!prev) return prev;
        if (planId && prev.plan_id && planId !== prev.plan_id) return prev;
        if (revision > 0 && prev.revision > 0 && revision !== prev.revision)
          return prev;
        return {
          ...prev,
          steps: prev.steps.map((s) =>
            s.id === sid
              ? (() => {
                  const nextStepSubsteps = nextSubsteps ?? s.substeps;
                  return {
                    ...s,
                    status: deriveExecutionPlanStepStatus(
                      newStatus,
                      nextStepSubsteps,
                    ),
                    substeps: nextStepSubsteps,
                  };
                })()
              : s,
          ),
        };
      });
      maybeSurfacePlanStepProgressBubble(normalizedIncomingStep);
    }

    const prevSteps = streamingStepsRef.current;
    const normalizedStep = sanitizeActivityStepForUi(
      ensureActivityStepTime(normalizeActivityStepForDisplay(normalizedIncomingStep)),
    );
    const incomingHeartbeat = isHeartbeatStreamingStep(normalizedStep);
    const incomingStableKey = streamingStepStructuralStableKey(normalizedStep);
    let next: JsonRecord[];
    if (incomingHeartbeat) {
      const existingIndex = prevSteps.findIndex((row) =>
        isHeartbeatStreamingStep(row),
      );
      if (existingIndex >= 0) {
        if (
          streamingStepDisplayKey(prevSteps[existingIndex]) ===
          streamingStepDisplayKey(normalizedStep)
        ) {
          return;
        }
        next = [...prevSteps];
        next[existingIndex] = attachStreamingStepStableKey(
          normalizedStep,
          getStreamingStepStableKey(prevSteps[existingIndex]),
        );
      } else {
        next = [...prevSteps, attachStreamingStepStableKey(normalizedStep)];
      }
    } else {
      next = [...prevSteps];
      const heartbeatIndex = next.findIndex((row) =>
        isHeartbeatStreamingStep(row),
      );
      if (heartbeatIndex >= 0) {
        next.splice(heartbeatIndex, 1);
      }
      if (incomingStableKey) {
        const existingIndex = next.findIndex(
          (row) => getStreamingStepStableKey(row) === incomingStableKey,
        );
        if (existingIndex >= 0) {
          const replacementStep =
            isStreamedModelTextStep(normalizedStep) &&
            isStreamedModelTextStep(next[existingIndex])
              ? mergeStreamedModelTextStep(next[existingIndex], normalizedStep)
              : normalizedStep;
          next.splice(existingIndex, 1);
          next.push(
            attachStreamingStepStableKey(replacementStep, incomingStableKey),
          );
          next = limitStreamingStepsForUi(next);
          streamingStepsRef.current = next;
          queuedStreamingStepsRef.current = next;
          scheduleStreamingStepsFlush();
          return;
        }
      }
      const lastIdx = next.length - 1;
      const incomingKey = streamingStepDedupKey(normalizedStep);
      if (
        lastIdx >= 0 &&
        streamingStepDedupKey(next[lastIdx]) === incomingKey
      ) {
        next[lastIdx] = attachStreamingStepStableKey(
          normalizedStep,
          getStreamingStepStableKey(next[lastIdx]),
        );
      } else {
        next.push(
          attachStreamingStepStableKey(
            normalizedStep,
            incomingStableKey || undefined,
          ),
        );
      }
    }
    next = limitStreamingStepsForUi(next);
    streamingStepsRef.current = next;
    queuedStreamingStepsRef.current = next;
    scheduleStreamingStepsFlush();
  };

  const rememberStreamedWorkspaceApp = (nextApp: JsonRecord | null) => {
    if (!nextApp) return;
    const merged = { ...(streamedWorkspaceAppRef.current || {}), ...nextApp };
    const appDir = str(merged.app_dir, "");
    streamedWorkspaceAppRef.current = merged;
    setStreamedWorkspaceApp(merged);
    if (appDir) {
      setDeployedFiles((prev) => mergeWorkspaceFiles(prev, [], appDir));
      setLiveFileWrites((prev) => canonicalizeLiveFileWrites(prev, appDir));
    }
  };

  const handleStreamThinking = (step: JsonRecord) => {
    maybeSurfaceThinkingProgressBubble(step);
    pushStreamingStep(step);
  };

  const handleStreamToolStart = (
    name: string,
    payload?: Record<string, unknown>,
  ) => {
    followActivityConsole();
    const payloadObj = attachCurrentPlanStepPayload(asRecord(payload));
    maybeSurfaceToolStartProgressBubble(name, payloadObj);
    const payloadSummary = decorateActivityDetailWithPlanStep(
      summarizeToolStartPayload(name, payloadObj),
      payloadObj,
    );
    pushStreamingStep({
      step_type: "tool_start",
      title: `Tool started: ${name}`,
      detail: payloadSummary || startingActivitySentenceForToolName(name),
      data:
        Object.keys(payloadObj).length > 0
          ? { ...payloadObj, tool_name: name }
          : { tool_name: name },
    });
    if (name === "file_read") {
      pendingFileReadPathRef.current = normalizeWorkspaceFileName(
        payloadObj.path ?? payloadObj.file,
        str(streamedWorkspaceAppRef.current?.app_dir, ""),
      );
    }
    if (name === "file_write") {
      pendingFileWritePathRef.current = normalizeWorkspaceFileName(
        payloadObj.path ?? payloadObj.file,
        str(streamedWorkspaceAppRef.current?.app_dir, ""),
      );
    }
    const capturedApp = extractWorkspaceAppFromStreamPayload(name, payload);
    rememberStreamedWorkspaceApp(capturedApp);
    const workspaceAppDir = str(
      capturedApp?.app_dir,
      str(streamedWorkspaceAppRef.current?.app_dir, ""),
    );
    const capturedFiles = extractWorkspaceFilesFromStreamPayload(name, payload);
    if (capturedFiles.length > 0) {
      revealLiveFilesConsole();
      setDeployedFiles((prev) =>
        mergeWorkspaceFiles(prev, capturedFiles, workspaceAppDir),
      );
      setLiveFileWrites((prev) => {
        const next = { ...prev };
        for (const file of capturedFiles) {
          const normalizedName = normalizeWorkspaceFileName(
            file.name,
            workspaceAppDir,
          );
          if (!normalizedName) continue;
          if (!next[normalizedName]) {
            const totalLines =
              file.content.length > 0 ? file.content.split(/\r?\n/).length : 0;
            next[normalizedName] = {
              content: choosePreferredWorkspaceFileContent("", file.content),
              line: 0,
              totalLines,
              done: false,
            };
          } else if (file.content && !next[normalizedName].content) {
            const totalLines = file.content.split(/\r?\n/).length;
            next[normalizedName] = {
              ...next[normalizedName],
              content: choosePreferredWorkspaceFileContent(
                next[normalizedName].content,
                file.content,
              ),
              totalLines: next[normalizedName].totalLines || totalLines,
            };
          }
        }
        return canonicalizeLiveFileWrites(next, workspaceAppDir);
      });
      setCodeViewerFileIdx(0);
    }
  };

  const handleStreamToolResult = (
    name: string,
    content: string,
    payload?: Record<string, unknown>,
  ) => {
    followActivityConsole();
    const preview = content.trim().slice(0, 1600);
    const payloadObj = attachCurrentPlanStepPayload(asRecord(payload));
    const detail = decorateActivityDetailWithPlanStep(
      simplifyConsoleDetail(summarizeActivityDetail(preview)),
      payloadObj,
    );
    const capturedApp = extractWorkspaceAppFromStreamPayload(name, payloadObj);
    rememberStreamedWorkspaceApp(capturedApp);
    const workspaceAppDir = str(
      capturedApp?.app_dir,
      str(streamedWorkspaceAppRef.current?.app_dir, ""),
    );
    const capturedFiles = extractWorkspaceFilesFromStreamPayload(
      name,
      payloadObj,
    );
    if (capturedFiles.length > 0) {
      revealLiveFilesConsole();
      setDeployedFiles((prev) =>
        mergeWorkspaceFiles(prev, capturedFiles, workspaceAppDir),
      );
    }
    if (name === "file_read") {
      const rawContent = str(payloadObj.raw_content, "").trim();
      const fileName = normalizeWorkspaceFileName(
        payloadObj.path ?? payloadObj.file ?? pendingFileReadPathRef.current,
        workspaceAppDir,
      );
      if (fileName && rawContent) {
        revealLiveFilesConsole();
        setDeployedFiles((prev) =>
          mergeWorkspaceFiles(
            prev,
            [{ name: fileName, content: rawContent }],
            workspaceAppDir,
          ),
        );
      }
      pendingFileReadPathRef.current = "";
    }
    if (name === "file_write") {
      const resultPathMatch = content.match(/written to\s+(.+)$/i);
      const fileName = normalizeWorkspaceFileName(
        payloadObj.path ??
          payloadObj.file ??
          resultPathMatch?.[1] ??
          pendingFileWritePathRef.current,
        workspaceAppDir,
      );
      const resultContent = choosePreferredWorkspaceFileContent(
        str(payloadObj.file_content, str(payloadObj.raw_content, "")),
        str(payloadObj.content, ""),
      );
      if (fileName) {
        revealLiveFilesConsole();
        setDeployedFiles((prev) => {
          const existing = prev.find((file) => file.name === fileName);
          const fallbackContent = existing?.content || "";
          return mergeWorkspaceFiles(
            prev,
            [
              {
                name: fileName,
                content: resultContent || fallbackContent,
              },
            ],
            workspaceAppDir,
          );
        });
        setLiveFileWrites((prev) => {
          const existing = prev[fileName];
          const mergedContent = choosePreferredWorkspaceFileContent(
            existing?.content || "",
            resultContent,
          );
          const totalLines = mergedContent
            ? mergedContent.split(/\r?\n/).length
            : Math.max(existing?.totalLines ?? 0, 0);
          return canonicalizeLiveFileWrites(
            {
              ...prev,
              [fileName]: {
                content: mergedContent,
                line:
                  totalLines > 0
                    ? totalLines
                    : Math.max(existing?.line ?? 0, 0),
                totalLines,
                done: true,
              },
            },
            workspaceAppDir,
          );
        });
      }
      pendingFileWritePathRef.current = "";
    }
    if (name === "app_deploy" || name === "app_restart") {
      setLiveFileWrites({});
    }
    pushStreamingStep({
      step_type: "tool_result",
      title: `Tool finished: ${name || "tool"}`,
      detail: detail || preview,
      data: Object.keys(payloadObj).length > 0
        ? { ...payloadObj, tool_name: name }
        : { tool_name: name, content: detail || preview },
    });
  };

  const handleStreamReasoningDeltaPayload = (
    payload: Record<string, unknown>,
    sourceName = "reasoning",
    fallbackContent = "",
  ) => {
    const payloadObj = asRecord(payload);
    const phase = normalizeReasoningPhase(payloadObj.phase);
    const streamKey = str(payloadObj.stream_key, `reasoning:${phase || "active"}`);
    const snapshot = str(payloadObj.content_snapshot, "");
    const delta = str(
      payloadObj.content_delta,
      str(payloadObj.content, fallbackContent),
    );
    const done = toBool(payloadObj.done);
    if (!phase) return;
    if (!isVisibleReasoningPhase(phase)) return;

    const current = reasoningProgressByPhaseRef.current[phase] || "";
    const nextContent = snapshot || (done ? current : `${current}${delta}`);
    reasoningProgressByPhaseRef.current[phase] = nextContent;
    setReasoningPreviewBuffered(phase, nextContent, done);

    const detail = nextContent.trim();
    if (!detail) return;
    const presentation = reasoningStatusCopy(phase, detail, payloadObj);
    const now = Date.now();
    const lastEmit = reasoningActivityEmitRef.current[streamKey] || 0;
    const activityEmitInterval =
      detail.length > 120_000 ? 2500 : detail.length > 60_000 ? 1500 : 750;
    const shouldEmit = done || now - lastEmit >= activityEmitInterval;
    if (shouldEmit) {
      reasoningActivityEmitRef.current[streamKey] = now;
      pushStreamingStep({
        step_type: "reasoning_delta",
        title: presentation.title,
        detail: decorateActivityDetailWithPlanStep(
          presentation.detail,
          payloadObj,
        ),
        data: {
          ...payloadObj,
          tool_name: sourceName,
          content_snapshot: detail,
        },
        __streamKey: presentation.streamKey || streamKey,
      });
    }
  };

  const handleStreamToolProgress = (
    name: string,
    content: string,
    payload?: Record<string, unknown>,
  ) => {
    followActivityConsole();
    const payloadObj = attachCurrentPlanStepPayload(asRecord(payload));
    const progressKind = str(payloadObj.kind, "").trim().toLowerCase();
    if (progressKind === "turn_completed") {
      return;
    }
    if (
      progressKind.startsWith("delegation_") &&
      !workspaceUserClosedRef.current
    ) {
      setWorkspaceOpen(true);
    }
    // Backward compatibility for older streams that carried reasoning deltas
    // as tool progress. Current streams use dedicated `reasoning_delta` events.
    // Identification is by event kind only, never by phrasing.
    if (isReasoningProgressPayload(name, payloadObj)) {
      handleStreamReasoningDeltaPayload(payloadObj, name, content);
      return;
    }
    const workspaceAppDir = str(streamedWorkspaceAppRef.current?.app_dir, "");
    const progressPresentation = buildToolProgressPresentation(
      name,
      content,
      payloadObj,
      workspaceAppDir,
    );
    const phaseStatus = extractPhaseStatusFromProgress(
      name,
      payloadObj,
      progressPresentation.detail,
    );
    if (phaseStatus) {
      setStreamPhaseStatus(phaseStatus);
      setExecutionPlan((prev) =>
        applyCapabilitySetupPhaseStatusToExecutionPlan(
          applyAppDeliveryPhaseStatusToExecutionPlan(prev, phaseStatus),
          phaseStatus,
        ),
      );
    }
    maybeSurfaceToolProgressBubble(
      name,
      content,
      payloadObj,
      progressPresentation,
    );

    const isDraftFile = progressKind === "draft_file";
    if (isDraftFile) {
      const fileName = normalizeWorkspaceFileName(
        payloadObj.file ?? payloadObj.path,
        workspaceAppDir,
      );
      if (fileName) {
        revealLiveFilesConsole();
        const snapshot = str(payloadObj.content_snapshot, "");
        const delta = str(payloadObj.content_delta, "");
        const done = toBool(payloadObj.done);
        const derivedLines = snapshot
          ? countContentLines(snapshot)
          : delta
            ? countContentLines(delta)
            : 0;
        const lineNo = Math.max(0, num(payloadObj.line, derivedLines));
        const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
        // Write to the canonical key directly: re-canonicalizing the WHOLE
        // map per delta re-compacted every file's content on every progress
        // event (O(files x size) per delta). Invalid names are dropped here,
        // exactly as canonicalizeLiveFileWrites would have dropped them.
        const canonicalFileName = normalizeWorkspaceFileName(
          fileName,
          workspaceAppDir,
        );
        const canonicalFileNameValid =
          !!canonicalFileName && isLikelyWorkspaceFileName(canonicalFileName);
        const currentContentHint =
          (canonicalFileNameValid
            ? liveFileWrites[canonicalFileName]?.content
            : liveFileWrites[fileName]?.content) || "";
        if (canonicalFileNameValid) {
          setLiveFileWrites((prev) => {
            const current = prev[canonicalFileName];
            let nextContent = current?.content ?? "";
            if (snapshot) {
              nextContent = choosePreferredWorkspaceFileContent(
                nextContent,
                snapshot,
              );
            } else if (delta) {
              nextContent = `${nextContent}${delta}`;
            }
            nextContent = compactWorkspacePreviewContent(
              nextContent,
              CHAT_WORKSPACE_UI_MAX_FILE_CHARS,
            );
            // Counting lines over the whole accumulated file is only needed
            // when the payload lacks authoritative line/total numbers.
            const nextDerivedLines =
              lineNo > 0 && totalLines > 0
                ? 0
                : nextContent
                  ? countContentLines(nextContent)
                  : derivedLines;
            const nextLine = Math.max(
              current?.line ?? 0,
              lineNo || nextDerivedLines,
            );
            const nextTotalLines =
              totalLines > 0
                ? totalLines
                : Math.max(current?.totalLines ?? 0, nextDerivedLines);
            const nextDone =
              done || (nextTotalLines > 0 && nextLine >= nextTotalLines);
            if (
              current &&
              current.content === nextContent &&
              current.line === nextLine &&
              current.totalLines === nextTotalLines &&
              current.done === nextDone
            ) {
              return prev;
            }
            return {
              ...prev,
              [canonicalFileName]: {
                content: nextContent,
                line: nextLine,
                totalLines: nextTotalLines,
                done: nextDone,
              },
            };
          });
        }
        setDeployedFiles((prev) => {
          const existing = prev.find((file) => file.name === fileName);
          const baseContent = currentContentHint || existing?.content || "";
          const mergedContent = snapshot
            ? choosePreferredWorkspaceFileContent(baseContent, snapshot)
            : delta
              ? `${baseContent}${delta}`
              : baseContent;
          if (existing && existing.content === mergedContent) return prev;
          return mergeWorkspaceFiles(
            prev,
            [{ name: fileName, content: mergedContent }],
            workspaceAppDir,
          );
        });
      }
    }

    const isFileWriteProgress =
      (name === "app_deploy" && str(payloadObj.kind, "") === "file_write") ||
      name === "file_write";
    if (isFileWriteProgress) {
      const fileName = normalizeWorkspaceFileName(
        payloadObj.file ?? payloadObj.path,
        workspaceAppDir,
      );
      if (fileName) {
        revealLiveFilesConsole();
        const lineNo = Math.max(0, num(payloadObj.line, 0));
        const totalLines = Math.max(0, num(payloadObj.total_lines, 0));
        const done =
          toBool(payloadObj.done) || (totalLines > 0 && lineNo >= totalLines);
        const text = str(payloadObj.text, "");
        setLiveFileWrites((prev) => {
          const current = prev[fileName];
          const currentLine = current?.line ?? 0;
          let nextContent = current?.content ?? "";
          if (lineNo > currentLine) {
            if (lineNo > 0) nextContent += `${text}\n`;
          } else if (!current && text) {
            nextContent = `${text}\n`;
          }
          const nextLine = Math.max(currentLine, lineNo);
          const nextTotalLines =
            totalLines > 0 ? totalLines : (current?.totalLines ?? 0);
          if (
            current &&
            current.content === nextContent &&
            current.line === nextLine &&
            current.totalLines === nextTotalLines &&
            current.done === done
          ) {
            return prev;
          }
          return canonicalizeLiveFileWrites(
            {
              ...prev,
              [fileName]: {
                content: nextContent,
                line: nextLine,
                totalLines: nextTotalLines,
                done,
              },
            },
            workspaceAppDir,
          );
        });
        setDeployedFiles((prev) => {
          if (prev.some((file) => file.name === fileName)) return prev;
          return mergeWorkspaceFiles(
            prev,
            [{ name: fileName, content: "" }],
            workspaceAppDir,
          );
        });
      }
    }

    const progressData = {
      ...payloadObj,
      tool_name: name,
    };
    pushStreamingStep({
      step_type: "tool_progress",
      title: progressPresentation.title,
      detail: decorateActivityDetailWithPlanStep(
        progressPresentation.detail,
        payloadObj,
      ),
      data: progressData,
      ...(progressPresentation.streamKey
        ? { __streamKey: progressPresentation.streamKey }
        : {}),
    });
  };

  const reattachToolHandlersRef = useRef({
    onToolStart: handleStreamToolStart,
    onToolResult: handleStreamToolResult,
    onToolProgress: handleStreamToolProgress,
    onReasoningDelta: handleStreamReasoningDeltaPayload,
  });

  useEffect(() => {
    reattachToolHandlersRef.current = {
      onToolStart: handleStreamToolStart,
      onToolResult: handleStreamToolResult,
      onToolProgress: handleStreamToolProgress,
      onReasoningDelta: handleStreamReasoningDeltaPayload,
    };
  }, [
    handleStreamToolStart,
    handleStreamToolResult,
    handleStreamToolProgress,
    handleStreamReasoningDeltaPayload,
  ]);

  const buildLiveRunArchive = ():
    | {
        pendingSnapshot: ChatPendingRunSnapshot;
        workspaceSnapshot: ChatWorkspaceSnapshot | null;
      }
    | null => {
    const activeSnapshot = pendingRunSnapshotRef.current ?? pendingRunSnapshot;
    const activeConversationId =
      activeSnapshot?.conversationId || conversationIdRef.current || conversationId || "";
    if (!activeConversationId) return null;
    const currentSteps = limitPendingRunStepsForSnapshot(
      trimTrailingHeartbeatSteps(
        streamingStepsRef.current.length > 0
          ? streamingStepsRef.current
          : streamingSteps,
      ),
    );
    const archive = buildChatLiveRunArchive({
      pendingSnapshot: activeSnapshot,
      conversationId: activeConversationId,
      taskId: activeChatTaskIdRef.current || activeSnapshot?.taskId || "",
      streamingResponse:
        streamingResponseRef.current ||
        streamingResponse ||
        activeSnapshot?.streamingResponse ||
        "",
      streamingSteps: currentSteps,
      deployedFiles: compactWorkspaceFilesForSnapshot(deployedFiles, {
        includeContent: true,
      }),
      liveFileWrites: compactLiveFileWritesForSnapshot(liveFileWrites, {
        includeContent: true,
      }),
      streamedWorkspaceApp: sanitizeWorkspaceAppSnapshot(
        streamedWorkspaceAppRef.current || streamedWorkspaceApp,
      ),
      codeViewerFileIdx,
      nowMs: Date.now(),
      maxResponseChars: CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS,
    });
    if (!archive.pendingSnapshot) return null;
    return {
      pendingSnapshot: {
        ...(archive.pendingSnapshot as ChatPendingRunSnapshot),
        conversationId: activeConversationId,
        message: str(archive.pendingSnapshot.message, activeSnapshot?.message || ""),
        startedAt: num(
          archive.pendingSnapshot.startedAt,
          activeSnapshot?.startedAt || Date.now(),
        ),
        streamingSteps: limitPendingRunStepsForSnapshot(
          (archive.pendingSnapshot.streamingSteps || []) as JsonRecord[],
        ),
      },
      workspaceSnapshot: archive.workspaceSnapshot
        ? ({
            ...(archive.workspaceSnapshot as ChatWorkspaceSnapshot),
            conversationId: activeConversationId,
          } satisfies ChatWorkspaceSnapshot)
        : null,
    };
  };

  const movePendingRunSnapshotToBackground =
    (): ChatPendingRunSnapshot | null => {
      const archivedRun = buildLiveRunArchive();
      if (!archivedRun) return null;
      const archivedSnapshot = archivedRun.pendingSnapshot;
      if (archivedRun.workspaceSnapshot) {
        storeChatWorkspaceSnapshotNow(archivedRun.workspaceSnapshot);
      }
      const nextBackgroundSnapshots = {
        ...backgroundRunSnapshots,
        [archivedSnapshot.conversationId]: archivedSnapshot,
      };
      setBackgroundRunSnapshots(nextBackgroundSnapshots);
      storeChatBackgroundRunSnapshots(nextBackgroundSnapshots);
      return archivedSnapshot;
    };

  const runStreamingChat = async (
    message: string,
    files: File[] = [],
    opts?: {
      sensitive?: boolean;
      conversationIdOverride?: string;
      newConversation?: boolean;
      statusSource?: string;
      deepResearch?: boolean;
      resumeTaskId?: string;
      planOverride?: ExecutionPlanState | null;
      acceptedSuggestionId?: string;
      sentinelProposalId?: string;
      browserProfileContext?: JsonRecord | null;
    },
  ): Promise<boolean> => {
    const resumeTaskId = (opts?.resumeTaskId || "").trim();
    const isResumeMode = !!resumeTaskId;
    const requestedConversationOverride = (
      opts?.conversationIdOverride || ""
    ).trim();
    const shouldStartNewConversation =
      Boolean(opts?.newConversation) && !requestedConversationOverride && !isResumeMode;
    let targetConversationId =
      requestedConversationOverride ||
      (shouldStartNewConversation ? "" : conversationId || "");
    const preservedResumeSnapshot =
      isResumeMode && targetConversationId
        ? (() => {
            const activeSnapshot = pendingRunSnapshotRef.current;
            if (
              activeSnapshot &&
              activeSnapshot.conversationId === targetConversationId &&
              (!resumeTaskId ||
                !activeSnapshot.taskId ||
                activeSnapshot.taskId === resumeTaskId)
            ) {
              return activeSnapshot;
            }
            const backgroundSnapshot =
              backgroundRunSnapshots[targetConversationId];
            if (
              backgroundSnapshot &&
              backgroundSnapshot.conversationId === targetConversationId &&
              (!resumeTaskId ||
                !backgroundSnapshot.taskId ||
                backgroundSnapshot.taskId === resumeTaskId)
            ) {
              return backgroundSnapshot;
            }
            return null;
          })()
        : null;
    const preservedResumeResponse = preservedResumeSnapshot
      ? str(preservedResumeSnapshot.streamingResponse, "").slice(
          0,
          CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS,
        )
      : "";
    const preservedResumeSteps = preservedResumeSnapshot
      ? limitPendingRunStepsForSnapshot(
          trimTrailingHeartbeatSteps(
            asRecords(preservedResumeSnapshot.streamingSteps).map((step) =>
              ensureActivityStepTime(step),
            ),
          ),
        )
      : [];
    let activeMessage = isResumeMode ? "" : message.trim();
    const activeMessagePreview = maskSensitiveChatPreview(activeMessage);
    if (
      (!activeMessage && files.length === 0 && !isResumeMode) ||
      isStreaming ||
      streamLockRef.current
    )
      return false;
    if (
      !isResumeMode &&
      targetConversationId &&
      workingConversationIds.has(targetConversationId)
    ) {
      setChatNotice("This conversation already has a run in progress.");
      return false;
    }
    if (isResumeMode && !targetConversationId) {
      setChatError(
        "This stopped task is missing its conversation, so it cannot be resumed.",
      );
      return false;
    }
    const now = Date.now();
    const deepResearch = Boolean(opts?.deepResearch);
    const executionProfile = deepResearch
      ? {
          capability_tags: ["research", "source_synthesis", "decision_grade"],
          depth_hint: "deep",
          deliverables: ["answer", "markdown", "document"],
          plan_first: true,
          long_running: true,
          confidence: 1,
          source: "ui_override",
        }
      : undefined;
    const planOverride = opts?.planOverride ?? null;
    const acceptedSuggestionId = str(opts?.acceptedSuggestionId, "").trim();
    const sentinelProposalId = str(opts?.sentinelProposalId, "").trim();
    const browserProfileContext =
      opts?.browserProfileContext &&
      Object.keys(opts.browserProfileContext).length > 0
        ? opts.browserProfileContext
        : null;
    const browserProfileFingerprint = browserProfileContext
      ? `${str(browserProfileContext.profile_id, "")}::${str(
          browserProfileContext.profile_name,
          "",
        )}`
      : "__no_browser_profile__";
    const executionMode: ChatExecutionMode = "auto";
    if (!isResumeMode && workingChatCount >= CHAT_WORKING_CHATS_MAX) {
      setChatError(
        `You already have ${CHAT_WORKING_CHATS_MAX} working chats. Wait for one to finish or stop one before starting another.`,
      );
      return false;
    }
    streamLockRef.current = true;
    if (!isResumeMode && !targetConversationId) {
      try {
        const created = asRecord(
          await api.rawPost("/conversations", {
            title: activeMessagePreview || "New Chat",
            channel: "web",
          }),
        );
        const createdId = str(
          created.id,
          str(created.conversation_id, str(created.conversationId, "")),
        ).trim();
        if (!createdId) {
          streamLockRef.current = false;
          setChatError("Could not attach this chat before starting.");
          return false;
        }
        targetConversationId = createdId;
        setConversationPage(0);
        setConversationId(createdId);
        setDraftChatMode(false);
      } catch (err) {
        streamLockRef.current = false;
        setChatError(
          `Could not attach this chat before starting: ${normalizeChatError(errMessage(err))}`,
        );
        return false;
      }
    }
    if (targetConversationId) {
      setDraftChatMode(false);
    }
    const attachmentFingerprint = files
      .map((file) => `${file.name}:${file.size}:${file.lastModified}`)
      .join("|");
    const fingerprint = isResumeMode
      ? `resume::${resumeTaskId}::${targetConversationId || "__no_conversation__"}`
      : `${targetConversationId || "__new__"}::${activeMessagePreview
          .toLowerCase()
          .replace(/\s+/g, " ")
          .trim()}::${attachmentFingerprint}::${deepResearch ? "research" : "chat"}::${executionMode}::${acceptedSuggestionId || "__no_suggestion__"}::${sentinelProposalId || "__no_sentinel__"}::${browserProfileFingerprint}`;
    const lastSend = recentSendRef.current;
    if (
      lastSend &&
      lastSend.fingerprint === fingerprint &&
      now - lastSend.at < 1500
    ) {
      streamLockRef.current = false;
      setChatNotice("Duplicate send ignored.");
      return false;
    }
    recentSendRef.current = { fingerprint, at: now };
    stopRequestedRef.current = false;
    const streamGeneration = streamGenerationRef.current + 1;
    streamGenerationRef.current = streamGeneration;
    activeChatTaskIdRef.current = isResumeMode ? resumeTaskId : null;
    const abortController = new AbortController();
    streamAbortRef.current = abortController;
    setLiveRunStreamOpenNow(false);

    setChatError(null);
    // Deep research no longer goes through a plan-confirmation step: the backend
    // planner that consumed `plan_first` was retired in the spine-substrate
    // migration, so the run executes end-to-end. Setting an optimistic
    // `stage: "planning"` here only produced an orphaned "Preparing deep research
    // plan" card that no backend event ever cleared. Deep research now falls
    // through to the `null` branch and runs immediately like any other chat turn.
    if (isResumeMode && planOverride) {
      setPlanConfirmation((prev) =>
        prev
          ? {
              ...prev,
              stage: "running",
              editing: false,
            }
          : prev,
      );
    } else if (!isResumeMode) {
      setPlanConfirmation(null);
    }
    const sensitiveMessage = !isResumeMode && Boolean(opts?.sensitive);
    setPendingUserMessage(
      !isResumeMode && !sensitiveMessage ? activeMessagePreview : null,
    );
    setFailedUserMessage(null);
    setStreamingResponseNow(preservedResumeResponse);
    setStreamingResponseChoices([]);
    setStreamingRunMetrics(null);
    setStreamingStepsNow(preservedResumeSteps);
    // Scope the Console to the new run: on a fresh send, clear the previous run's
    // latched steps so the console doesn't fall back to the prior run during the
    // window before the new run's first step arrives. Resume keeps its steps.
    if (!isResumeMode) {
      setLastRunSteps([]);
    }
    if (isResumeMode && planOverride) {
      setExecutionPlan(planOverride);
      setExecutionPlanFailure("");
      setExecutionPlanExpanded(true);
    } else {
      setExecutionPlan(null);
      setExecutionPlanFailure("");
      setExecutionPlanExpanded(false);
    }
    setStreamingProgressMessages(
      isResumeMode ? [] : ["Thinking."],
    );
    resetStreamingProgressBubbleState();
    setCompletedProgressMessagesByConversation((prev) => {
      if (!targetConversationId || !prev[targetConversationId]) return prev;
      const next = { ...prev };
      delete next[targetConversationId];
      return next;
    });
    setLiveFileWrites({});
    setDeployedFiles([]);
    setStreamPhaseStatus(null);
    setStreamedWorkspaceApp(null);
    streamedWorkspaceAppRef.current = null;
    latestRunEventSeqRef.current = 0;
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    setIsStreaming(true);
    // New run: clear any prior user dismissal so first activity can auto-open.
    workspaceUserClosedRef.current = false;
    pendingFileReadPathRef.current = "";
    pendingFileWritePathRef.current = "";

    if (targetConversationId && conversationId !== targetConversationId) {
      setConversationId(targetConversationId);
    }
    const initialPendingSnapshot: ChatPendingRunSnapshot = {
      conversationId: targetConversationId,
      message:
        !isResumeMode && !sensitiveMessage
          ? activeMessagePreview
          : str(preservedResumeSnapshot?.message, ""),
      startedAt: Date.now(),
      initialMessageCount: shouldStartNewConversation ? 0 : messages.length,
      runId: "",
      mode: isResumeMode ? "resume" : "fresh",
      phase: "running",
      taskId: resumeTaskId,
      streamingResponse: preservedResumeResponse,
      streamingSteps: preservedResumeSteps,
      failedUserMessage: "",
      lastRunSeq: 0,
      attachments: isResumeMode
        ? sanitizeChatTurnAttachments(preservedResumeSnapshot?.attachments)
        : chatTurnAttachmentsFromFiles(files),
    };
    storeChatPendingRunSnapshotNow(initialPendingSnapshot);
    setPendingRunSnapshot(initialPendingSnapshot);

    let resolvedConversationId = targetConversationId;
    let payloadMessage = activeMessage;
    let attachmentPayloads: Array<{
      upload_id?: string;
      document_id?: string;
      kind: string;
      content_type?: string | null;
    }> = [];
    let streamError: string | null = null;
    let latestStreamingResponse = preservedResumeResponse;
    const streamStartedAt = Date.now();
    // Lightweight first-render latency instrumentation. Run-scoped closures only:
    // no state, no refs, no re-renders, zero behavior change. Each "first" flag
    // logs exactly once per run via console.debug.
    const timingSubmittedAt = performance.now();
    let loggedFirstStreamEvent = false;
    let loggedFirstReplyText = false;
    let loggedRunComplete = false;
    let receivedTokenDeltas = 0;
    let receivedTokenChars = 0;
    let appendedTokenChars = 0;
    const markFirstStreamEventTiming = () => {
      if (loggedFirstStreamEvent) return;
      loggedFirstStreamEvent = true;
      console.debug(
        `[chat-timing] first stream event: ${Math.round(
          performance.now() - timingSubmittedAt,
        )}ms`,
      );
    };
    const markFirstReplyTextTiming = () => {
      if (loggedFirstReplyText) return;
      loggedFirstReplyText = true;
      console.debug(
        `[chat-timing] first reply text: ${Math.round(
          performance.now() - timingSubmittedAt,
        )}ms`,
      );
    };
    const markRunCompleteTiming = () => {
      if (loggedRunComplete) return;
      loggedRunComplete = true;
      console.debug(
        `[chat-timing] run complete: ${Math.round(
          performance.now() - timingSubmittedAt,
        )}ms (token deltas: ${receivedTokenDeltas}, received chars: ${receivedTokenChars}, appended chars: ${appendedTokenChars})`,
      );
    };
    const streamDetachedToBackground = () =>
      backgroundDetachGenerationsRef.current.has(streamGeneration);
    const updateDetachedBackgroundSnapshot = (
      patch: Partial<ChatPendingRunSnapshot>,
    ) => {
      const detachedConversationId = str(
        patch.conversationId,
        targetConversationId || initialPendingSnapshot.conversationId || "",
      ).trim();
      if (!detachedConversationId) return;
      setBackgroundRunSnapshots((prev) => {
        const existing =
          prev[detachedConversationId] || {
            ...initialPendingSnapshot,
            conversationId: detachedConversationId,
          };
        const nextSnapshot: ChatPendingRunSnapshot = {
          ...existing,
          ...patch,
          conversationId: detachedConversationId,
        };
        const next = {
          ...prev,
          [detachedConversationId]: nextSnapshot,
        };
        storeChatBackgroundRunSnapshots(next);
        return next;
      });
    };
    const initialAssistantMessageCount =
      targetConversationId && targetConversationId === conversationId
        ? messages.filter(
            (message) =>
              str(message.role, "").trim().toLowerCase() === "assistant",
          ).length
        : 0;
    let firstTokenMs: number | null = null;
    const absorbRunMetrics = (payload: unknown) => {
      const obj = asRecord(payload);
      const next: ChatRunMetrics = {};
      const observedElapsedMs = Math.max(1, Date.now() - streamStartedAt);
      const parsedMetrics = chatRunMetricsFromPayload(payload);
      if (parsedMetrics.inputTokens != null) next.inputTokens = parsedMetrics.inputTokens;
      if (parsedMetrics.outputTokens != null) next.outputTokens = parsedMetrics.outputTokens;
      if (parsedMetrics.totalTokens != null) next.totalTokens = parsedMetrics.totalTokens;
      const durationMs = parsedMetrics.durationMs;
      const timeToFirstStreamActivityMs =
        parsedMetrics.timeToFirstStreamActivityMs;
      const timeToFirstTokenMs = parsedMetrics.timeToFirstTokenMs;
      if (durationMs != null) {
        next.durationMs = Math.max(durationMs, observedElapsedMs);
      }
      if (timeToFirstStreamActivityMs != null) {
        next.timeToFirstStreamActivityMs = timeToFirstStreamActivityMs;
      }
      if (timeToFirstTokenMs != null) {
        const serverFirstTokenMs = timeToFirstTokenMs;
        firstTokenMs =
          firstTokenMs != null
            ? Math.min(firstTokenMs, serverFirstTokenMs)
            : serverFirstTokenMs;
        next.timeToFirstTokenMs = firstTokenMs;
      } else if (firstTokenMs != null) {
        next.timeToFirstTokenMs = firstTokenMs;
      }
      if (Object.keys(next).length === 0) return;
      setStreamingRunMetrics((prev) => ({ ...(prev ?? {}), ...next }));
    };
    const markFirstStreamingToken = () => {
      if (firstTokenMs != null) return;
      firstTokenMs = Math.max(1, Date.now() - streamStartedAt);
      setStreamingRunMetrics((prev) => ({
        ...(prev ?? {}),
        timeToFirstTokenMs: firstTokenMs,
      }));
    };
    const absorbConversationId = (payload: unknown) => {
      const obj = asRecord(payload);
      const cid = str(
        obj.conversation_id,
        str(obj.cid, str(obj.conversationId, "")),
      );
      const runId = str(obj.run_id, "");
      if (streamDetachedToBackground()) {
        if (cid || runId) {
          updateDetachedBackgroundSnapshot({
            ...(cid ? { conversationId: cid } : {}),
            ...(runId ? { runId } : {}),
          });
        }
        return;
      }
      if (cid) {
        resolvedConversationId = cid;
        setPendingRunSnapshot((prev) => {
          const base = prev ?? initialPendingSnapshot;
          const next = {
            ...base,
            conversationId: cid,
            ...(runId ? { runId } : {}),
          };
          if (
            prev &&
            next.conversationId === base.conversationId &&
            next.runId === base.runId
          ) {
            return prev;
          }
          scheduleChatPendingRunSnapshotStore(next);
          return next;
        });
        if (!conversationIdRef.current) {
          setConversationPage(0);
          setConversationId(cid);
        }
        return;
      }
      if (runId) {
        setPendingRunSnapshot((prev) => {
          const base = prev ?? initialPendingSnapshot;
          const next = { ...base, runId };
          if (prev && next.runId === base.runId) {
            return prev;
          }
          scheduleChatPendingRunSnapshotStore(next);
          return next;
        });
      }
    };
    const handlePlanStreamEvent = (eventName: string, payload: unknown) => {
      markFirstStreamEventTiming();
      recordRunEventSeq(payload);
      absorbConversationId(payload);
      if (streamDetachedToBackground()) return;
      if (eventName === "run_status") {
        absorbRunMetrics(payload);
        const runStatusStep = buildRunStatusActivityStep(payload);
        if (runStatusStep) pushStreamingStep(runStatusStep);
        if (isTerminalExecutionRunStatus(streamPayloadRunStatus(payload))) {
          refreshConversationMessagesFromStreamPayload(
            payload,
            resolvedConversationId || targetConversationId,
            initialAssistantMessageCount + 1,
            { settle: true, minLatestAssistantCreatedAtMs: streamStartedAt },
          );
        }
        return;
      }
      if (
        eventName === "plan_generated" ||
        eventName === "plan_revised" ||
        eventName === "plan_step_update" ||
        eventName === "plan_unavailable" ||
        eventName === "plan_ready_for_confirmation"
      ) {
        const planPayload = asRecord(payload);
        if (Object.keys(planPayload).length > 0) {
          pushStreamingStep(planPayload);
        }
      }
    };

    try {
      if (!isResumeMode && files.length > 0) {
        setChatNotice(
          `Indexing ${files.length} attachment${files.length === 1 ? "" : "s"}...`,
        );
        const uploaded = await uploadAttachmentsForKnowledge(files);
        attachmentPayloads = [
          ...uploaded.documents.map((item) => ({
            document_id: item.id,
            kind: "document",
            content_type: "application/octet-stream",
          })),
          ...uploaded.visuals.map((item) => ({
            upload_id: item.id,
            kind: "visual",
            content_type: item.contentType || "image",
          })),
        ];
        if (attachmentPayloads.length > 0) {
          const totalUploaded =
            uploaded.documents.length + uploaded.visuals.length;
          setChatNotice(
            `Prepared ${totalUploaded} attachment${totalUploaded === 1 ? "" : "s"} for this request.`,
          );
        }
      }
      await (isResumeMode
        ? api.resumeChatTaskStream(
            resumeTaskId,
            planOverride
              ? { plan_override: planOverride as Record<string, unknown> }
              : undefined,
            {
              signal: abortController.signal,
              onOpen: () => {
                if (!streamDetachedToBackground()) setLiveRunStreamOpenNow(true);
              },
              onEvent: handlePlanStreamEvent,
              onToken: (token, payload) => {
                if (streamDetachedToBackground()) return;
                if (!isSyntheticStreamTokenPayload(payload)) {
                  markFirstStreamingToken();
                }
                markFirstReplyTextTiming();
                receivedTokenDeltas += 1;
                receivedTokenChars += token.length;
                const appended = appendStreamingToken(token);
                appendedTokenChars += appended.length;
                latestStreamingResponse += appended;
              },
              onThinking: (step) => {
                if (streamDetachedToBackground()) return;
                absorbConversationId(step);
                handleStreamThinking(step);
              },
              onReasoningDelta: (payload) => {
                if (streamDetachedToBackground()) return;
                absorbConversationId(payload);
                handleStreamReasoningDeltaPayload(payload);
              },
              onToolStart: (name, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolStart(name, payload);
              },
              onToolResult: (name, content, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolResult(name, content, payload);
              },
              onToolProgress: (name, content, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolProgress(name, content, payload);
              },
              onTaskStarted: (payload) => {
                if (streamDetachedToBackground()) return;
                const taskId = str(payload.task_id, "");
                if (!taskId) return;
                activeChatTaskIdRef.current = taskId;
                setPendingRunSnapshot((prev) => {
                  const next = {
                    ...(prev ?? initialPendingSnapshot),
                    taskId,
                    mode: (isResumeMode
                      ? "resume"
                      : "fresh") as ChatPendingRunMode,
                    phase:
                      prev?.phase === "awaiting_confirmation"
                        ? ("awaiting_confirmation" as ChatPendingRunPhase)
                        : ("running" as ChatPendingRunPhase),
                  };
                  storeChatPendingRunSnapshotNow(next);
                  return next;
                });
                void queryClient.invalidateQueries({ queryKey: ["tasks"] });
                void queryClient.invalidateQueries({
                  queryKey: ["tasks-manager"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-status"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-agents"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-delegations"],
                });
              },
              onTaskStatus: (payload) => {
                if (streamDetachedToBackground()) return;
                const taskId = str(payload.task_id, "");
                const status = str(payload.status, "");
                if (isTerminalChatTaskStatus(status)) {
                  refreshConversationMessagesFromStreamPayload(
                    payload,
                    resolvedConversationId || targetConversationId,
                    initialAssistantMessageCount + 1,
                    { settle: true, minLatestAssistantCreatedAtMs: streamStartedAt },
                  );
                }
                if (!taskId || !status) return;
                const pauseKind = str(
                  payload.pause_kind,
                  str(payload._pause_kind, ""),
                )
                  .trim()
                  .toLowerCase();
                if (
                  (status === "paused" || status === "awaiting_approval") &&
                  (pauseKind === "plan_confirmation" ||
                    deepResearch ||
                    Boolean(planOverride) ||
                    !!str(planConfirmation?.source, "").trim())
                ) {
                  markPendingRunAwaitingPlanConfirmation(taskId);
                }
                void queryClient.invalidateQueries({ queryKey: ["tasks"] });
                void queryClient.invalidateQueries({
                  queryKey: ["tasks-manager"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-status"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-agents"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-delegations"],
                });
              },
              onContent: (payload) => {
                if (streamDetachedToBackground()) return;
                const text = stripAgentInternalReasoningLeaks(
                  str(payload.content, ""),
                );
                if (text.trim()) {
                  markFirstReplyTextTiming();
                  latestStreamingResponse = text;
                  setStreamingResponseNow(text);
                  refreshConversationMessagesFromStreamPayload(
                    payload,
                    resolvedConversationId || targetConversationId,
                    initialAssistantMessageCount + 1,
                  );
                }
                setStreamingResponseChoices(clarificationChoices(payload.choices));
                absorbRunMetrics(payload);
                absorbConversationId(payload);
              },
              onDone: (payload) => {
                if (streamDetachedToBackground()) return;
                markRunCompleteTiming();
                setIsStreaming(false);
                setLiveRunStreamOpenNow(false);
                absorbConversationId(payload);
                refreshConversationMessagesFromStreamPayload(
                  payload,
                  resolvedConversationId || targetConversationId,
                  initialAssistantMessageCount + 1,
                  { settle: true, minLatestAssistantCreatedAtMs: streamStartedAt },
                );
              },
              onError: (messageText) => {
                if (streamDetachedToBackground()) return;
                streamError = normalizeChatError(messageText);
              },
            },
          )
        : api.chatStream(
            {
              message: payloadMessage,
              channel: "web",
              conversation_id: targetConversationId || undefined,
              execution_profile: executionProfile,
              // Plan confirmation is retired for deep research — the backend no
              // longer consumes `plan_first`, so requesting `before_execution`
              // only set a dead profile flag. Send no plan-confirmation mode.
              execution_mode: executionMode,
              attachments_present: attachmentPayloads.length > 0,
              attachments: attachmentPayloads,
              client_timezone: getRequestUiTimeZone(),
              client_timezone_offset_minutes: Number.isFinite(
                new Date().getTimezoneOffset(),
              )
                ? -new Date().getTimezoneOffset()
                : undefined,
              accepted_suggestion_id: acceptedSuggestionId || undefined,
              sentinel_proposal_id: sentinelProposalId || undefined,
              browser_profile_context: browserProfileContext || undefined,
            },
            {
              signal: abortController.signal,
              onOpen: () => {
                if (!streamDetachedToBackground()) setLiveRunStreamOpenNow(true);
              },
              onEvent: handlePlanStreamEvent,
              onToken: (token, payload) => {
                if (streamDetachedToBackground()) return;
                if (!isSyntheticStreamTokenPayload(payload)) {
                  markFirstStreamingToken();
                }
                markFirstReplyTextTiming();
                receivedTokenDeltas += 1;
                receivedTokenChars += token.length;
                const appended = appendStreamingToken(token);
                appendedTokenChars += appended.length;
                latestStreamingResponse += appended;
              },
              onThinking: (step) => {
                if (streamDetachedToBackground()) return;
                absorbConversationId(step);
                handleStreamThinking(step);
              },
              onReasoningDelta: (payload) => {
                if (streamDetachedToBackground()) return;
                absorbConversationId(payload);
                handleStreamReasoningDeltaPayload(payload);
              },
              onToolStart: (name, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolStart(name, payload);
              },
              onToolResult: (name, content, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolResult(name, content, payload);
              },
              onToolProgress: (name, content, payload) => {
                if (streamDetachedToBackground()) return;
                handleStreamToolProgress(name, content, payload);
              },
              onTaskStarted: (payload) => {
                if (streamDetachedToBackground()) return;
                const taskId = str(payload.task_id, "");
                if (!taskId) return;
                activeChatTaskIdRef.current = taskId;
                setPendingRunSnapshot((prev) => {
                  const next = {
                    ...(prev ?? initialPendingSnapshot),
                    taskId,
                    mode: "fresh" as ChatPendingRunMode,
                    phase:
                      prev?.phase === "awaiting_confirmation"
                        ? ("awaiting_confirmation" as ChatPendingRunPhase)
                        : ("running" as ChatPendingRunPhase),
                  };
                  storeChatPendingRunSnapshotNow(next);
                  return next;
                });
                void queryClient.invalidateQueries({ queryKey: ["tasks"] });
                void queryClient.invalidateQueries({
                  queryKey: ["tasks-manager"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-status"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-agents"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-delegations"],
                });
              },
              onTaskStatus: (payload) => {
                if (streamDetachedToBackground()) return;
                const taskId = str(payload.task_id, "");
                const status = str(payload.status, "");
                if (isTerminalChatTaskStatus(status)) {
                  refreshConversationMessagesFromStreamPayload(
                    payload,
                    resolvedConversationId || targetConversationId,
                    initialAssistantMessageCount + 1,
                    { settle: true, minLatestAssistantCreatedAtMs: streamStartedAt },
                  );
                }
                if (!taskId || !status) return;
                const pauseKind = str(
                  payload.pause_kind,
                  str(payload._pause_kind, ""),
                )
                  .trim()
                  .toLowerCase();
                if (
                  (status === "paused" || status === "awaiting_approval") &&
                  (pauseKind === "plan_confirmation" ||
                    deepResearch ||
                    Boolean(planOverride) ||
                    !!str(planConfirmation?.source, "").trim())
                ) {
                  markPendingRunAwaitingPlanConfirmation(taskId);
                }
                void queryClient.invalidateQueries({ queryKey: ["tasks"] });
                void queryClient.invalidateQueries({
                  queryKey: ["tasks-manager"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-status"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-agents"],
                });
                void queryClient.invalidateQueries({
                  queryKey: ["swarm-delegations"],
                });
              },
              onContent: (payload) => {
                if (streamDetachedToBackground()) return;
                const text = stripAgentInternalReasoningLeaks(
                  str(payload.content, ""),
                );
                if (text.trim()) {
                  markFirstReplyTextTiming();
                  latestStreamingResponse = text;
                  setStreamingResponseNow(text);
                  refreshConversationMessagesFromStreamPayload(
                    payload,
                    resolvedConversationId || targetConversationId,
                    initialAssistantMessageCount + 1,
                  );
                }
                setStreamingResponseChoices(clarificationChoices(payload.choices));
                absorbRunMetrics(payload);
                absorbConversationId(payload);
              },
              onDone: (payload) => {
                if (streamDetachedToBackground()) return;
                markRunCompleteTiming();
                setIsStreaming(false);
                setLiveRunStreamOpenNow(false);
                absorbConversationId(payload);
                refreshConversationMessagesFromStreamPayload(
                  payload,
                  resolvedConversationId || targetConversationId,
                  initialAssistantMessageCount + 1,
                  { settle: true, minLatestAssistantCreatedAtMs: streamStartedAt },
                );
              },
              onError: (messageText) => {
                if (streamDetachedToBackground()) return;
                streamError = normalizeChatError(messageText);
              },
            },
          ));
    } catch (err) {
      const detachedAbort =
        backgroundDetachGenerationsRef.current.has(streamGeneration);
      const aborted =
        (stopRequestedRef.current || detachedAbort) &&
        ((err instanceof DOMException && err.name === "AbortError") ||
          errMessage(err).toLowerCase().includes("abort"));
      if (!aborted) {
        streamError = normalizeChatError(errMessage(err));
      }
    } finally {
      const detachedToBackground =
        backgroundDetachGenerationsRef.current.has(streamGeneration);
      if (detachedToBackground) {
        backgroundDetachGenerationsRef.current.delete(streamGeneration);
      }
      const isCurrentStreamGeneration =
        streamGenerationRef.current === streamGeneration;
      const wasStopped = stopRequestedRef.current && !streamError;
      const finalTaskId = activeChatTaskIdRef.current || resumeTaskId || "";
      const completedStreamingSteps = trimTrailingHeartbeatSteps(
        streamingStepsRef.current,
      );
      const completedStreamingPlan =
        extractExecutionPlanFromTraceSteps(completedStreamingSteps);
      const planConfirmationTaskIdFromStream = (() => {
        for (let idx = completedStreamingSteps.length - 1; idx >= 0; idx -= 1) {
          const step = asRecord(completedStreamingSteps[idx]);
          const stepType = str(step.step_type, "").trim().toLowerCase();
          if (
            stepType === "plan_ready_for_confirmation" ||
            stepType === "plan_generated" ||
            stepType === "plan_revised"
          ) {
            const taskId = str(step.task_id, "").trim();
            if (taskId) return taskId;
          }
        }
        return "";
      })();
      const finalPlanTaskId = finalTaskId || planConfirmationTaskIdFromStream;
      const latestRunStatusFromStream = extractLatestRunStatusSummary(
        completedStreamingSteps,
      );
      const hadUsablePlanInStream =
        completedStreamingSteps.some((step) => {
          const stepType = str(step.step_type, "").trim().toLowerCase();
          return (
            stepType === "plan_generated" ||
            stepType === "plan_revised" ||
            stepType === "plan_ready_for_confirmation" ||
            stepType === "plan_step_update"
          );
        }) || !!completedStreamingPlan;
      const awaitingPlanConfirmationFromStream =
        !isResumeMode &&
        shouldKeepPlanInApprovalState(
          completedStreamingPlan,
          completedStreamingSteps,
          "fresh",
        );
      const explicitPlanningFailure = extractExecutionPlanFailureFromTraceSteps(
        completedStreamingSteps,
      ).trim();
      const terminalPlanningFailure =
        !detachedToBackground &&
        !wasStopped &&
        deepResearch &&
        !isResumeMode &&
        !planOverride &&
        !hadUsablePlanInStream &&
        (Boolean(streamError) ||
          Boolean(explicitPlanningFailure) ||
          isTerminalDeepResearchFailureStatus(
            str(latestRunStatusFromStream?.status, ""),
          ));
      const terminalPlanningFailureDetail =
        explicitPlanningFailure ||
        str(latestRunStatusFromStream?.detail, "").trim() ||
        streamError ||
        "Deep research could not prepare a usable plan.";
      if (streamError) {
        setChatError(streamError);
        setPlanConfirmation((prev) => {
          if (!prev) return prev;
          if (terminalPlanningFailure) {
            return null;
          }
          if (isResumeMode && planOverride) {
            return {
              ...prev,
              stage: "awaiting_confirmation",
              editing: false,
            };
          }
          return {
            ...prev,
            stage: "failed",
            editing: false,
          };
        });
        if (!sensitiveMessage && !isResumeMode) {
          setFailedUserMessage(activeMessagePreview);
        }
      } else if (terminalPlanningFailure) {
        setExecutionPlanFailure(terminalPlanningFailureDetail);
        setPlanConfirmation(null);
      } else if (awaitingPlanConfirmationFromStream) {
        const planSource =
          extractPlanConfirmationSourceFromSteps(completedStreamingSteps) ||
          (deepResearch ? PLAN_CONFIRMATION_SOURCE_DEEP_RESEARCH : "execution");
        const nextPlan = resetExecutionPlanProgress(completedStreamingPlan);
        if (isDeepResearchPlanSource(planSource)) {
          if (nextPlan) {
            setExecutionPlan(nextPlan);
            setExecutionPlanFailure("");
          }
          markPendingRunAwaitingPlanConfirmation(finalPlanTaskId);
          setPlanConfirmation((prev) => {
            if (
              prev &&
              prev.stage === "awaiting_confirmation" &&
              prev.originalPlan &&
              prev.draft
            ) {
              return {
                ...prev,
                taskId: finalPlanTaskId || prev.taskId,
                editing: false,
              };
            }
            const originalPlan = prev?.originalPlan ?? nextPlan;
            return {
              stage: "awaiting_confirmation",
              taskId: finalPlanTaskId || prev?.taskId || null,
              source: planSource,
              originalPlan,
              draft: createPlanConfirmationDraft(originalPlan),
              editing: false,
              messageId: prev?.messageId ?? null,
            };
          });
        }
      }
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({
        queryKey: ["chat-credential-prompt"],
      });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["swarm-status"] });
      await queryClient.invalidateQueries({ queryKey: ["swarm-agents"] });
      await queryClient.invalidateQueries({ queryKey: ["swarm-delegations"] });
      if (!detachedToBackground && !streamError && !wasStopped) {
        setFailedUserMessage(null);
        const currentPendingSnapshotPhase = str(
          pendingRunSnapshotRef.current?.phase,
          "",
        ).trim();
        if (
          !!str(planConfirmation?.source, "").trim() &&
          !awaitingPlanConfirmationFromStream &&
          currentPendingSnapshotPhase !== "awaiting_confirmation" &&
          currentPendingSnapshotPhase !== "interrupted"
        ) {
          setPlanConfirmation(null);
        }
        const candidateConversationId =
          resolvedConversationId || targetConversationId;
        if (candidateConversationId) {
          try {
            await api.rawGet(
              `/conversations/${encodeURIComponent(candidateConversationId)}`,
            );
            resolvedConversationId = candidateConversationId;
          } catch {
            resolvedConversationId = "";
          }
        }
        if (!resolvedConversationId) {
          try {
            const latest = await api.rawGet(
              "/conversations?limit=1",
            );
            const newest = pickRecords(latest, "conversations")[0];
            const newestId = str(newest?.id, "");
            if (newestId) resolvedConversationId = newestId;
          } catch {
            // Ignore fallback lookup failures; chat can still be selected manually.
          }
        }
        if (resolvedConversationId) {
          setConversationPage(0);
          setConversationId(resolvedConversationId);
          try {
            await refreshConversationMessagesAfterStream(
              resolvedConversationId,
              initialAssistantMessageCount + 1,
              {
                settle: true,
                minLatestAssistantCreatedAtMs: streamStartedAt,
              },
            );
          } catch {
            const recovered = await recoverAssistantMessageFromLatestRun(
              resolvedConversationId,
              {
                expectedRunId: str(
                  pendingRunSnapshotRef.current?.runId,
                  "",
                ).trim(),
                minRunUpdatedAtMs: streamStartedAt,
              },
            ).catch(() => false);
            if (!recovered) {
              await queryClient.invalidateQueries({
                queryKey: ["chat-messages", resolvedConversationId],
              });
            }
          }
        }
      }
      if (!detachedToBackground && !streamError && !wasStopped)
        setAttachedFiles([]);
      if (
        !detachedToBackground &&
        !streamError &&
        !wasStopped &&
        streamingStepsRef.current.length > 0
      ) {
        setLastRunSteps(trimTrailingHeartbeatSteps(streamingStepsRef.current));
      }
      if (
        typeof window !== "undefined" &&
        opts?.statusSource &&
        !wasStopped &&
        !detachedToBackground
      ) {
        const completedStatusMessage =
          opts.statusSource === "sentinel"
            ? "Sentinel launch completed. Review Chat for the result."
            : "Pulse fix completed. Review Chat for the result.";
        window.dispatchEvent(
          new CustomEvent<ChatRunStatusDetail>(CHAT_RUN_STATUS_EVENT, {
            detail: {
              conversationId: resolvedConversationId || targetConversationId,
              source: opts.statusSource,
              status: streamError ? "error" : "completed",
              message: streamError ? streamError : completedStatusMessage,
            },
          }),
        );
      }
      if (detachedToBackground) {
        setFailedUserMessage(null);
      } else if (wasStopped) {
        setFailedUserMessage(null);
        const activeSnapshot = pendingRunSnapshotRef.current;
        const interruptedSteps = limitPendingRunStepsForSnapshot(
          trimTrailingHeartbeatSteps(streamingStepsRef.current),
        );
        const fallbackInterruptedSteps =
          interruptedSteps.length > 0
            ? interruptedSteps
            : limitPendingRunStepsForSnapshot(
                asRecords(activeSnapshot?.streamingSteps),
              );
        const interruptedConversationId =
          resolvedConversationId ||
          targetConversationId ||
          str(activeSnapshot?.conversationId, "");
        const interruptedResponse = (
          latestStreamingResponse ||
          streamingResponseRef.current ||
          str(activeSnapshot?.streamingResponse, "")
        ).slice(0, CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS);
        const interruptedMessage =
          str(activeSnapshot?.message, "") ||
          initialPendingSnapshot.message ||
          activeMessagePreview;
        const shouldPreserveInterruptedStop = Boolean(
          interruptedConversationId &&
            (finalTaskId ||
              str(activeSnapshot?.runId, "").trim() ||
              interruptedResponse.trim() ||
              fallbackInterruptedSteps.length > 0 ||
              interruptedMessage.trim()),
        );
        if (shouldPreserveInterruptedStop) {
          const interruptedSnapshot: ChatPendingRunSnapshot = {
            ...initialPendingSnapshot,
            ...(activeSnapshot ?? {}),
            conversationId: interruptedConversationId,
            runId: str(
              activeSnapshot?.runId,
              initialPendingSnapshot.runId || "",
            ),
            taskId: finalTaskId || str(activeSnapshot?.taskId, ""),
            mode: isResumeMode ? "resume" : "fresh",
            phase: "interrupted",
            message: interruptedMessage,
            streamingResponse: interruptedResponse,
            streamingSteps: fallbackInterruptedSteps,
            failedUserMessage: "",
          };
          storeChatPendingRunSnapshotNow(interruptedSnapshot);
          setPendingRunSnapshot(interruptedSnapshot);
          setPendingUserMessage(null);
          setStreamingResponseNow(interruptedSnapshot.streamingResponse || "");
          setStreamingStepsNow(fallbackInterruptedSteps);
        } else {
          storeChatPendingRunSnapshotNow(null);
          setPendingRunSnapshot(null);
          setPendingUserMessage(null);
          setStreamingStepsNow([]);
          setExecutionPlan(null);
          setExecutionPlanFailure("");
          setExecutionPlanExpanded(false);
          setStreamingResponseNow("");
        }
      } else if (streamError || terminalPlanningFailure) {
        if (streamError && isResumeMode && preservedResumeSnapshot) {
          const restoredInterruptedSnapshot: ChatPendingRunSnapshot = {
            ...preservedResumeSnapshot,
            conversationId: resolvedConversationId || targetConversationId,
            taskId: finalTaskId || preservedResumeSnapshot.taskId || "",
            phase: "interrupted",
            streamingResponse: preservedResumeResponse,
            streamingSteps: preservedResumeSteps,
          };
          storeChatPendingRunSnapshotNow(restoredInterruptedSnapshot);
          setPendingRunSnapshot(restoredInterruptedSnapshot);
          setPendingUserMessage(null);
          setStreamingStepsNow(preservedResumeSteps);
          setExecutionPlan(null);
          setExecutionPlanFailure("");
          setExecutionPlanExpanded(false);
          setStreamingResponseNow(preservedResumeResponse);
        } else if (streamError && !isResumeMode && !sensitiveMessage) {
          const interruptedSteps = limitPendingRunStepsForSnapshot(
            trimTrailingHeartbeatSteps(streamingStepsRef.current),
          );
          const activeSnapshot = pendingRunSnapshotRef.current;
          const interruptedSnapshot: ChatPendingRunSnapshot = {
            ...initialPendingSnapshot,
            conversationId:
              resolvedConversationId ||
              targetConversationId ||
              activeSnapshot?.conversationId ||
              "",
            runId: str(
              activeSnapshot?.runId,
              initialPendingSnapshot.runId || "",
            ),
            taskId: finalTaskId || activeSnapshot?.taskId || "",
            mode: "fresh",
            phase: "interrupted",
            message: activeMessagePreview,
            failedUserMessage: activeMessagePreview,
            streamingResponse: latestStreamingResponse.slice(
              0,
              CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS,
            ),
            streamingSteps: interruptedSteps,
            ...(typeof activeSnapshot?.lastRunSeq === "number"
              ? { lastRunSeq: activeSnapshot.lastRunSeq }
              : {}),
          };
          storeChatPendingRunSnapshotNow(interruptedSnapshot);
          setPendingRunSnapshot(interruptedSnapshot);
          setPendingUserMessage(activeMessagePreview);
          setStreamingStepsNow(interruptedSteps);
          setExecutionPlan(null);
          setExecutionPlanFailure("");
          setExecutionPlanExpanded(false);
          setStreamingResponseNow(interruptedSnapshot.streamingResponse || "");
        } else {
          storeChatPendingRunSnapshotNow(null);
          setPendingRunSnapshot(null);
          setPendingUserMessage(null);
          setStreamingStepsNow([]);
          setExecutionPlan(null);
          if (terminalPlanningFailure) {
            setExecutionPlanFailure(terminalPlanningFailureDetail);
          } else {
            setExecutionPlanFailure("");
          }
          setExecutionPlanExpanded(false);
          setStreamingResponseNow("");
        }
      } else if (
        !detachedToBackground &&
        !wasStopped &&
        !awaitingPlanConfirmationFromStream
      ) {
        clearPendingRunPresentation(
          trimTrailingHeartbeatSteps(streamingStepsRef.current),
        );
      }
      if (isCurrentStreamGeneration) {
        streamAbortRef.current = null;
        activeChatTaskIdRef.current = null;
        stopRequestedRef.current = false;
        setIsStreaming(false);
        setLiveRunStreamOpenNow(false);
        setIsStoppingStream(false);
        streamLockRef.current = false;
      }
    }
    return !streamError;
  };

  const reattachRunId = str(pendingRunSnapshot?.runId, "").trim();
  const reattachConversationId = str(
    pendingRunSnapshot?.conversationId,
    "",
  ).trim();
  const reattachPhase = str(pendingRunSnapshot?.phase, "").trim().toLowerCase();
  const reattachTaskId = str(pendingRunSnapshot?.taskId, "").trim();

  useEffect(() => {
    const runId = reattachRunId;
    const pendingConversationId = reattachConversationId;
    if (!runId || !pendingConversationId) return;
    if (reattachPhase === "awaiting_confirmation") return;
    if (!conversationId || conversationId !== pendingConversationId) return;
    if (isStreaming || streamLockRef.current) return;
    if (reattachPhase === "interrupted") {
      // "interrupted" is this tab's claim, not the backend's: the stream
      // detached (page crash, navigation, network drop) but the run may have
      // finished server-side. Reconcile once per run against the backend's
      // latest-run record and let the truth win — terminal clears the label
      // and refreshes messages, a still-active run lifts the label so the
      // normal reattach path below takes over, and an unknown run keeps the
      // interrupted presentation untouched.
      if (reconciledInterruptedRunIdRef.current === runId) return;
      reconciledInterruptedRunIdRef.current = runId;
      const snapshot = pendingRunSnapshotRef.current;
      if (!snapshot || snapshot.conversationId !== pendingConversationId) {
        return;
      }
      void syncPendingRunFromLatestRun(pendingConversationId, snapshot, {
        expectedRunId: runId,
        allowTerminalClear: true,
      })
        .then((outcome) => {
          if (outcome !== "active") return;
          setPendingRunSnapshot((prev) => {
            if (!prev || str(prev.runId, "").trim() !== runId) return prev;
            const next: ChatPendingRunSnapshot = {
              ...prev,
              phase: "running" as ChatPendingRunPhase,
            };
            storeChatPendingRunSnapshotNow(next);
            return next;
          });
        })
        .catch(() => {
          // Backend unreachable: keep the interrupted presentation and allow
          // a retry on a later effect pass.
          reconciledInterruptedRunIdRef.current = "";
        });
      return;
    }
    if (reattachedRunIdRef.current === runId) return;

    reattachedRunIdRef.current = runId;
    const abortController = new AbortController();
    streamAbortRef.current = abortController;
    let latestStreamingResponse = streamingResponseRef.current;
    let runCompleted = false;
    let runInterrupted = false;
    let recoveredFromLatestRun = false;
    setLiveRunStreamOpenNow(false);
    const reattachSinceSeq = Math.max(
      0,
      Math.floor(
        num(
          pendingRunSnapshotRef.current?.lastRunSeq ??
            pendingRunSnapshot?.lastRunSeq,
          0,
        ),
      ),
    );
    const reattachStartedAtMs = Math.max(
      0,
      num(pendingRunSnapshot?.startedAt, 0),
    );

    const absorbRunPayload = (payload: unknown) => {
      const obj = asRecord(payload);
      const cid = str(
        obj.conversation_id,
        str(obj.cid, str(obj.conversationId, pendingConversationId)),
      );
      const nextRunId = str(obj.run_id, runId);
      const metrics = chatRunMetricsFromPayload(payload);
      if (Object.keys(metrics).length > 0) {
        setStreamingRunMetrics((prev) => ({ ...(prev ?? {}), ...metrics }));
      }
      setPendingRunSnapshot((prev) => {
        const base = prev ??
          pendingRunSnapshotRef.current ?? {
            conversationId: cid,
            message: "",
            startedAt: Date.now(),
          };
        const next = {
          ...base,
          conversationId: cid || base.conversationId,
          runId: nextRunId || base.runId || runId,
        };
        if (
          prev &&
          next.conversationId === base.conversationId &&
          next.runId === base.runId
        ) {
          return prev;
        }
        scheduleChatPendingRunSnapshotStore(next);
        return next;
      });
    };

    const recoverFromLatestRun = async () => {
      const snapshot =
        pendingRunSnapshotRef.current &&
        pendingRunSnapshotRef.current.conversationId === pendingConversationId
          ? pendingRunSnapshotRef.current
          : {
              conversationId: pendingConversationId,
              message: "",
              startedAt: Date.now(),
              runId,
              phase: "running" as ChatPendingRunPhase,
              taskId: reattachTaskId,
              streamingResponse: latestStreamingResponse,
              streamingSteps: [],
              failedUserMessage: "",
            };
      try {
        const outcome = await syncPendingRunFromLatestRun(
          pendingConversationId,
          snapshot,
          {
            expectedRunId: runId,
            allowTerminalClear: true,
          },
        );
        return outcome !== "none";
      } catch {
        return false;
      }
    };

    void api
      .runStream(runId, reattachSinceSeq, {
        signal: abortController.signal,
        onOpen: () => setLiveRunStreamOpenNow(true),
        onEvent: (eventName, payload) => {
          recordRunEventSeq(payload);
          absorbRunPayload(payload);
          if (eventName === "run_status") {
            const runStatusStep = buildRunStatusActivityStep(payload);
            if (runStatusStep) pushStreamingStep(runStatusStep);
            if (isTerminalExecutionRunStatus(streamPayloadRunStatus(payload))) {
              refreshConversationMessagesFromStreamPayload(
                payload,
                pendingConversationId,
                undefined,
                {
                  settle: true,
                  minLatestAssistantCreatedAtMs: reattachStartedAtMs,
                },
              );
            }
            return;
          }
          if (
            eventName === "plan_generated" ||
            eventName === "plan_revised" ||
            eventName === "plan_step_update" ||
            eventName === "plan_unavailable" ||
            eventName === "plan_ready_for_confirmation"
          ) {
            const planPayload = asRecord(payload);
            if (Object.keys(planPayload).length > 0) {
              pushStreamingStep(planPayload);
            }
          }
        },
        onToken: (token) => {
          latestStreamingResponse += appendStreamingToken(token);
        },
        onThinking: (step) => {
          absorbRunPayload(step);
          handleStreamThinking(step);
        },
        onReasoningDelta: (payload) => {
          absorbRunPayload(payload);
          reattachToolHandlersRef.current.onReasoningDelta(payload);
        },
        onToolStart: (name, payload) =>
          reattachToolHandlersRef.current.onToolStart(name, payload),
        onToolResult: (name, content, payload) =>
          reattachToolHandlersRef.current.onToolResult(name, content, payload),
        onToolProgress: (name, content, payload) =>
          reattachToolHandlersRef.current.onToolProgress(
            name,
            content,
            payload,
          ),
        onContent: (payload) => {
          const text = stripAgentInternalReasoningLeaks(
            str(payload.content, ""),
          );
          if (text.trim()) {
            latestStreamingResponse = text;
            setStreamingResponseNow(text);
            refreshConversationMessagesFromStreamPayload(
              payload,
              pendingConversationId,
              undefined,
              {
                settle: true,
                minLatestAssistantCreatedAtMs: reattachStartedAtMs,
              },
            );
          }
          setStreamingResponseChoices(clarificationChoices(payload.choices));
          absorbRunPayload(payload);
        },
        onError: (messageText) => {
          if (!abortController.signal.aborted) {
            runInterrupted = true;
            const normalizedError = normalizeChatError(messageText);
            setChatError(normalizedError);
            const detail = `Chat run could not be reattached: ${normalizedError}`;
            pushStreamingStep({
              step_type: "run_status",
              title: "Run status: interrupted",
              detail,
              data: {
                reason: normalizedError,
              },
            });
            markPendingRunInterrupted(
              reattachTaskId || str(pendingRunSnapshotRef.current?.taskId, ""),
              latestStreamingResponse,
            );
            setPlanConfirmation((prev) =>
              prev
                ? {
                    ...prev,
                    stage: "interrupted",
                    editing: false,
                  }
                : prev,
            );
          }
        },
        onDone: (payload) => {
          absorbRunPayload(payload);
          runCompleted = true;
          clearPendingRunPresentation(
            trimTrailingHeartbeatSteps(streamingStepsRef.current),
          );
          void queryClient.invalidateQueries({
            queryKey: ["chat-conversations"],
          });
          void queryClient.invalidateQueries({ queryKey: ["tasks"] });
          void refreshConversationMessagesAfterStream(
            pendingConversationId,
            undefined,
            {
              settle: true,
              minLatestAssistantCreatedAtMs: reattachStartedAtMs,
            },
          ).catch(async () => {
            const recovered = await recoverAssistantMessageFromLatestRun(
              pendingConversationId,
              {
                expectedRunId: runId,
                minRunUpdatedAtMs: reattachStartedAtMs,
              },
            ).catch(() => false);
            if (recovered) return;
            void queryClient.invalidateQueries({
              queryKey: ["chat-messages", pendingConversationId],
            });
          });
        },
      })
      .catch(async (error) => {
        if (abortController.signal.aborted) return;
        recoveredFromLatestRun = await recoverFromLatestRun();
        if (recoveredFromLatestRun) return;
        runInterrupted = true;
        const normalizedError = normalizeChatError(errMessage(error));
        setChatError(normalizedError);
        pushStreamingStep({
          step_type: "run_status",
          title: "Run status: interrupted",
          detail: `Chat run could not be reattached: ${normalizedError}`,
          data: {
            reason: normalizedError,
          },
        });
        markPendingRunInterrupted(
          reattachTaskId || str(pendingRunSnapshotRef.current?.taskId, ""),
          latestStreamingResponse,
        );
        setPlanConfirmation((prev) =>
          prev
            ? {
                ...prev,
                stage: "interrupted",
                editing: false,
              }
            : prev,
        );
      })
      .finally(() => {
        if (streamAbortRef.current === abortController) {
          streamAbortRef.current = null;
        }
        if (!abortController.signal.aborted) {
          setLiveRunStreamOpenNow(false);
        }
        if (!abortController.signal.aborted) {
          if (!runCompleted && !runInterrupted && !recoveredFromLatestRun) {
            void recoverFromLatestRun().then((recovered) => {
              if (recovered) return;
              const preservedResponse = str(
                pendingRunSnapshotRef.current?.streamingResponse,
                "",
              );
              if (!latestStreamingResponse && preservedResponse) {
                setStreamingResponseNow(preservedResponse);
              }
            });
          } else {
            const preservedResponse = str(
              pendingRunSnapshotRef.current?.streamingResponse,
              "",
            );
            if (!latestStreamingResponse && preservedResponse) {
              setStreamingResponseNow(preservedResponse);
            }
          }
        }
      });

    return () => {
      abortController.abort();
      setLiveRunStreamOpenNow(false);
      if (streamAbortRef.current === abortController) {
        streamAbortRef.current = null;
      }
      if (reattachedRunIdRef.current === runId) {
        reattachedRunIdRef.current = "";
      }
    };
  }, [
    reattachRunId,
    reattachConversationId,
    reattachPhase,
    reattachTaskId,
    conversationId,
    isStreaming,
    pendingRunSnapshot?.startedAt,
    queryClient,
    recoverAssistantMessageFromLatestRun,
    refreshConversationMessagesAfterStream,
    refreshConversationMessagesFromStreamPayload,
  ]);

  const handleStopStreaming = async () => {
    const activeSnapshot = pendingRunSnapshotRef.current ?? pendingRunSnapshot;
    const activeRunId = str(
      activeSnapshot?.runId || pendingRunSnapshot?.runId,
      "",
    ).trim();
    const activeTaskId =
      activeChatTaskIdRef.current ||
      str(activeSnapshot?.taskId || pendingRunSnapshot?.taskId, "").trim();
    const activeSnapshotPhase = str(activeSnapshot?.phase, "running")
      .trim()
      .toLowerCase();
    const canStopPendingRun =
      activeSnapshotPhase === "running" &&
      Boolean(activeRunId || activeTaskId || liveRunStreamOpen);
    if (!isStreaming && !streamLockRef.current && !canStopPendingRun) return;
    const stoppingAttachedRun = !isStreaming && !streamLockRef.current;
    stopRequestedRef.current = true;
    setIsStoppingStream(true);
    setChatError(null);
    setChatNotice("Stopping...");
    let stopError = "";
    if (activeRunId) {
      try {
        await api.rawPost(`/runs/${encodeURIComponent(activeRunId)}/cancel`, {});
      } catch (err) {
        stopError = `run cancel failed: ${errMessage(err)}`;
      }
    }
    streamAbortRef.current?.abort();
    if (stoppingAttachedRun) {
      setLiveRunStreamOpenNow(false);
    }
    if (activeTaskId) {
      try {
        await api.rawPost(
          `/tasks/${encodeURIComponent(activeTaskId)}/cancel`,
          {},
        );
        setChatNotice("Stopped.");
      } catch (err) {
        const taskError = `task cancel failed: ${errMessage(err)}`;
        setChatNotice(`Stop requested, but ${stopError || taskError}`);
      }
    } else if (stopError) {
      setChatNotice(`Stop requested, but ${stopError}`);
    } else {
      setChatNotice("Stopped.");
    }
    void queryClient.invalidateQueries({ queryKey: ["tasks"] });
    void queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    void queryClient.invalidateQueries({ queryKey: ["swarm-status"] });
    void queryClient.invalidateQueries({ queryKey: ["swarm-agents"] });
    void queryClient.invalidateQueries({ queryKey: ["swarm-delegations"] });
    if (stoppingAttachedRun) {
      markPendingRunInterrupted(
        activeTaskId,
        streamingResponseRef.current || str(activeSnapshot?.streamingResponse, ""),
      );
      stopRequestedRef.current = false;
      setIsStoppingStream(false);
    }
  };

  const updatePlanConfirmationDraft = (
    updater: (draft: PlanConfirmationDraft) => PlanConfirmationDraft,
  ) => {
    setPlanConfirmation((prev) => {
      if (!prev?.draft) return prev;
      return {
        ...prev,
        draft: updater(prev.draft),
      };
    });
  };

  const resetPlanConfirmationDraft = () => {
    setPlanConfirmation((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        draft: createPlanConfirmationDraft(prev.originalPlan),
        editing: false,
      };
    });
  };

  const clearPlanConfirmationPreviewState = () => {
    setPlanConfirmation(null);
    setExecutionPlan(null);
    setExecutionPlanFailure("");
    setExecutionPlanExpanded(false);
    storeChatPendingRunSnapshotNow(null);
    setPendingRunSnapshot(null);
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
  };

  const handlePlanConfirmationCancel = async () => {
    const taskId = str(planConfirmation?.taskId, "").trim();
    if (!taskId) {
      clearPlanConfirmationPreviewState();
      return;
    }
    try {
      await api.cancelTask(taskId);
      setChatNotice(
        `${planConfirmationDisplayLabel(planConfirmation?.source)} canceled.`,
      );
      clearPlanConfirmationPreviewState();
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const handlePlanConfirmationStart = async () => {
    const taskId = str(planConfirmation?.taskId, "").trim();
    const overridePlan = buildExecutionPlanFromDraft(
      planConfirmation?.draft ?? null,
      planConfirmation?.originalPlan ?? null,
    );
    const anchorCandidate =
      planConfirmationMessageIndex >= 0
        ? str(
            asRecord(messages[planConfirmationMessageIndex]).id,
            String(planConfirmationMessageIndex),
          ).trim()
        : str(planConfirmation?.messageId, "").trim();
    if (!taskId || !overridePlan) {
      setChatError("Select at least one research step before starting.");
      return;
    }

    setPlanConfirmation((prev) =>
      prev
        ? {
            ...prev,
            stage: "running",
            editing: false,
            messageId:
              str(prev.messageId, "").trim() || anchorCandidate || null,
          }
        : prev,
    );

    const ok = await runStreamingChat("", [], {
      conversationIdOverride: conversationId || undefined,
      resumeTaskId: taskId,
      planOverride: overridePlan,
    });

    if (!ok) {
      setPlanConfirmation((prev) =>
        prev
          ? {
              ...prev,
              stage: "awaiting_confirmation",
            }
          : prev,
      );
    }
  };

  const submitComposerMessage = async (
    messageText: string,
    files: File[] = [],
    browserProfileContext: JsonRecord | null = composerBrowserProfileContext,
  ): Promise<boolean> => {
    const trimmed = messageText.trim();
    const useDeepResearchForSubmit =
      deepResearchEnabled && !deepResearchDisabled;
    if (
      isStreaming ||
      currentConversationHasActiveRun ||
      (!trimmed && files.length === 0)
    )
      return false;

    if (composerAwaitingPlanConfirmation) {
      const pausedTaskId = str(planConfirmation?.taskId, "").trim();
      if (pausedTaskId) {
        try {
          await api.cancelTask(pausedTaskId);
        } catch {
          setChatNotice(
            "Starting a new message, but the earlier paused plan may still remain in Tasks.",
          );
        }
      }
      clearPlanConfirmationPreviewState();
      setChatNotice(
        useDeepResearchForSubmit
          ? `Updating the ${planConfirmationDisplayLabel(planConfirmation?.source).toLowerCase()}...`
          : `Discarding the paused ${planConfirmationDisplayLabel(planConfirmation?.source).toLowerCase()} and sending your message...`,
      );
    }

    setChatError(null);
    // Clear any consumed template prefill so it can't re-hydrate the composer
    // when it remounts (e.g. the starter -> active-conversation view switch on
    // the first message). Otherwise the chosen template text reappears after send.
    setComposerPrefillRequest(null);
    void runStreamingChat(trimmed, files, {
      deepResearch: useDeepResearchForSubmit,
      browserProfileContext,
    }).then((accepted) => {
      if (accepted && browserProfileContext === composerBrowserProfileContext) {
        setComposerBrowserProfileContext(null);
      }
    });
    return true;
  };

  useEffect(() => {
    setSubmittedClarificationChoices({});
  }, [conversationId]);

  const submitClarificationChoice = async (
    messageKey: string,
    choice: ChatClarificationChoice,
    choiceKey: string,
  ): Promise<void> => {
    const trimmed = choice.submitText.trim();
    if (!trimmed || submittedClarificationChoices[choiceKey]) return;
    setSubmittedClarificationChoices((prev) => ({
      ...prev,
      [choiceKey]: true,
    }));
    const approval =
      choice.approval ?? parseInternalApprovalSubmitToken(trimmed);
    if (
      (choice.kind === "direct_chat_approval" ||
        choice.kind === "direct_chat_chain_approval" ||
        approval) &&
      approval
    ) {
      try {
        const result = asRecord(
          await api.rawPost(
            `/chat/tool-approvals/${encodeURIComponent(approval.id)}/decision`,
            { decision: approval.decision },
          ),
        );
        const response = str(result.response, "").trim();
        if (response && conversationId) {
          removeApprovalChoicesFromConversationCache(conversationId, approval.id);
          appendAssistantContentToConversationCache(conversationId, {
            content: response,
            model_used: "approval",
          });
          void queryClient.invalidateQueries({
            queryKey: ["chat-messages", conversationId],
          });
        }
      } catch (error) {
        setSubmittedClarificationChoices((prev) => {
          const next = { ...prev };
          delete next[choiceKey];
          return next;
        });
        setChatError(
          error instanceof Error ? error.message : "Failed to update approval.",
        );
      }
      return;
    }
    await submitComposerMessage(trimmed, []);
  };

  const renderClarificationChoiceGroup = (
    messageKey: string,
    choices: ChatClarificationChoice[],
    forceDisabled = false,
  ) => {
    if (choices.length === 0) return null;
    const disabled = forceDisabled || isStreaming;
    const approvalChoice = choices.find(
      (choice) =>
        (choice.kind === "direct_chat_approval" ||
          choice.kind === "direct_chat_chain_approval") &&
        choice.approval,
    );
    const isApprovalGroup = Boolean(approvalChoice?.approval);
    const approvalSteps =
      choices.find(
        (choice) =>
          choice.kind === "direct_chat_chain_approval" &&
          (choice.approval?.steps?.length ?? 0) > 0,
      )?.approval?.steps ?? [];
    return (
      <Stack
        spacing={isApprovalGroup ? 1.1 : 0.85}
        sx={
          isApprovalGroup
            ? {
                mt: 1.4,
                p: 1.35,
                borderRadius: 1.25,
                border: "1px solid rgba(245, 158, 11, 0.58)",
                background:
                  "linear-gradient(135deg, rgba(245, 158, 11, 0.17), rgba(15, 23, 42, 0.42))",
                boxShadow:
                  "0 16px 38px rgba(0, 0, 0, 0.26), inset 0 1px 0 rgba(255, 255, 255, 0.08)",
              }
            : { mt: 1.25 }
        }
      >
        {isApprovalGroup ? (
          <Stack spacing={0.35}>
            <Typography
              variant="overline"
              sx={{
                color: "warning.light",
                fontWeight: 800,
                letterSpacing: 0,
                lineHeight: 1.2,
              }}
            >
              Approval required
            </Typography>
            <Typography
              variant="body2"
              sx={{ color: "text.primary", fontWeight: 700, lineHeight: 1.35 }}
            >
              {approvalSteps.length > 1
                ? `${approvalSteps.length} actions are waiting to run`
                : approvalChoice?.approval?.actionName
                  ? `Run ${approvalChoice.approval.actionName}`
                  : "Run this action"}
            </Typography>
          </Stack>
        ) : null}
        {approvalSteps.length > 0 ? (
          <Box
            sx={{
              border: isApprovalGroup
                ? "1px solid rgba(245, 158, 11, 0.28)"
                : "1px solid rgba(148, 163, 184, 0.24)",
              borderRadius: 1,
              p: 1,
              background: isApprovalGroup
                ? "rgba(2, 6, 23, 0.34)"
                : "rgba(15, 23, 42, 0.18)",
            }}
          >
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", display: "block", mb: 0.5 }}
            >
              {approvalSteps.length > 1
                ? `Approval covers ${approvalSteps.length} actions`
                : "Approval action"}
            </Typography>
            <Stack spacing={0.4}>
              {approvalSteps.map((step, idx) => (
                <Typography
                  key={`${messageKey}-approval-step-${idx}-${step.actionName}`}
                  variant="caption"
                  sx={{ color: "text.secondary" }}
                >
                  {idx + 1}. {step.actionName}
                </Typography>
              ))}
            </Stack>
          </Box>
        ) : null}
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{
            flexWrap: "wrap",
          }}
        >
          {choices.map((choice, idx) => {
            const choiceKey = [
              messageKey,
              choice.kind || "choice",
              choice.approval?.id || choice.submitText,
            ].join(":");
            return (
              <Button
                key={`${messageKey}-${choice.submitText}-${idx}`}
                size={isApprovalGroup ? "medium" : "small"}
                variant={
                  isApprovalGroup && choice.approval?.decision === "approve"
                    ? "contained"
                    : "outlined"
                }
                color={
                  isApprovalGroup && choice.approval?.decision === "reject"
                    ? "error"
                    : isApprovalGroup
                      ? "warning"
                      : "primary"
                }
                disabled={disabled || Boolean(submittedClarificationChoices[choiceKey])}
                onClick={() => {
                  void submitClarificationChoice(messageKey, choice, choiceKey);
                }}
                sx={{
                  borderRadius: 1,
                  textTransform: "none",
                  fontWeight: isApprovalGroup ? 800 : 500,
                  minHeight: isApprovalGroup ? 38 : undefined,
                  px: isApprovalGroup ? 1.65 : undefined,
                }}
              >
                {choice.label}
              </Button>
            );
          })}
        </Stack>
      </Stack>
    );
  };

  useEffect(() => {
    if (typeof window === "undefined") return;
    const handleLaunchRun = (event: Event) => {
      const detail = (event as CustomEvent<ChatLaunchRunDetail>).detail;
      if (isStreaming || streamLockRef.current) {
        detail?.reject?.(
          "Chat is already busy with another run. Wait for it to finish, then retry this fix.",
        );
        return;
      }
      const resumeTaskId = str(detail?.taskId, "").trim();
      const launchMode =
        !!resumeTaskId || detail?.launchMode === "resume_task"
          ? "resume_task"
          : "message";
      const message = str(detail?.message, "").trim();
      if (launchMode === "message" && !message) {
        detail?.reject?.("No message provided.");
        return;
      }
      if (launchMode === "resume_task" && !resumeTaskId) {
        detail?.reject?.("No resumable task was provided.");
        return;
      }
      detail?.resolve?.(true);
      void runStreamingChat(launchMode === "resume_task" ? "" : message, [], {
        conversationIdOverride:
          str(detail?.conversationId, "").trim() || undefined,
        newConversation: detail?.newConversation === true,
        statusSource: str(detail?.source, "").trim() || undefined,
        resumeTaskId: launchMode === "resume_task" ? resumeTaskId : undefined,
      }).catch((err) => {
        if (typeof window !== "undefined" && detail?.source) {
          window.dispatchEvent(
            new CustomEvent<ChatRunStatusDetail>(CHAT_RUN_STATUS_EVENT, {
              detail: {
                conversationId: str(detail?.conversationId, "").trim(),
                source: detail.source,
                status: "error",
                message: errMessage(err),
              },
            }),
          );
        }
      });
    };
    window.addEventListener(
      CHAT_LAUNCH_RUN_EVENT,
      handleLaunchRun as EventListener,
    );
    return () => {
      window.removeEventListener(
        CHAT_LAUNCH_RUN_EVENT,
        handleLaunchRun as EventListener,
      );
    };
  }, [isStreaming, runStreamingChat]);

  useEffect(() => {
    if (!isActive || isStreaming || streamLockRef.current) return;
    const pendingLaunch = loadChatPendingLaunch();
    if (!pendingLaunch) return;
    storeChatPendingLaunch(null);
    void runStreamingChat(
      pendingLaunch.launchMode === "resume_task"
        ? ""
        : str(pendingLaunch.message, ""),
      [],
      {
        conversationIdOverride:
          str(pendingLaunch.conversationId, "").trim() || undefined,
        newConversation: pendingLaunch.newConversation === true,
        statusSource: str(pendingLaunch.source, "").trim() || undefined,
        resumeTaskId:
          pendingLaunch.launchMode === "resume_task"
            ? str(pendingLaunch.taskId, "").trim() || undefined
            : undefined,
        acceptedSuggestionId:
          str(pendingLaunch.acceptedSuggestionId, "").trim() || undefined,
        sentinelProposalId:
          str(pendingLaunch.sentinelProposalId, "").trim() || undefined,
      },
    ).catch(() => {
      // The normal chat error UI will surface the failure.
    });
  }, [isActive, isStreaming, runStreamingChat]);

  // Pin scroll to bottom during streaming - useLayoutEffect runs before paint
  // so the user never sees the intermediate jank position.
  const stickToBottom = useRef(true);
  // Track whether user is near bottom to decide if we should auto-stick
  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    const onScroll = () => {
      const nearBottom =
        thread.scrollHeight - thread.scrollTop - thread.clientHeight < 80;
      stickToBottom.current = nearBottom;
    };
    thread.addEventListener("scroll", onScroll, { passive: true });
    return () => thread.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    if (!pendingUserMessage) return;
    if (isStreaming) return;
    const pendingNormalized =
      stripAttachmentContextMarker(pendingUserMessage).trim();
    if (!pendingNormalized) {
      setPendingUserMessage(null);
    }
  }, [pendingUserMessage, isStreaming]);

  useEffect(() => {
    if (!chatNotice) return;
    const timer = window.setTimeout(() => setChatNotice(null), 2200);
    return () => window.clearTimeout(timer);
  }, [chatNotice]);

  const pendingSnapshotPhase =
    pendingRunSnapshot?.phase === "interrupted"
      ? "interrupted"
      : pendingRunSnapshot?.phase === "awaiting_confirmation"
        ? "awaiting_confirmation"
        : "running";
  const pendingSnapshotMode =
    pendingRunSnapshot?.mode === "resume" ? "resume" : "fresh";
  const hasFocusedDraftStream =
    !conversationId &&
    pendingSnapshotMode === "fresh" &&
    pendingSnapshotPhase === "running" &&
    Boolean(pendingRunSnapshot) &&
    (isStreaming ||
      !!pendingUserMessage ||
      !!streamingResponse.trim() ||
      streamingProgressMessages.length > 0 ||
      streamingSteps.length > 0);
  const hasFocusedDraftInterruptedRun =
    !conversationId &&
    pendingSnapshotMode === "fresh" &&
    pendingSnapshotPhase === "interrupted" &&
    Boolean(pendingRunSnapshot) &&
    (!!pendingUserMessage ||
      !!failedUserMessage ||
      !!str(pendingRunSnapshot?.message, "").trim() ||
      !!str(pendingRunSnapshot?.failedUserMessage, "").trim() ||
      !!streamingResponse.trim() ||
      streamingSteps.length > 0);
  const hasLivePendingThread =
    hasPendingSnapshotForConversation ||
    hasFocusedDraftStream ||
    hasFocusedDraftInterruptedRun;
  const hasRecoveredStream =
    !isStreamingForCurrentConversation &&
    hasPendingSnapshotForConversation &&
    pendingSnapshotPhase === "running";
  const showInterruptedRunCard =
    (hasPendingSnapshotForConversation || hasFocusedDraftInterruptedRun) &&
    pendingSnapshotPhase === "interrupted" &&
    !isStreamingForCurrentConversation;
  // Track the messages count when streaming started so we can detect when the
  // final assistant message has actually landed in the messages list.
  const streamStartMsgCount = useRef(messages.length);
  const prevIsStreaming = useRef(false);
  if (isStreamingForCurrentConversation && !prevIsStreaming.current) {
    streamStartMsgCount.current = messages.length;
  }
  prevIsStreaming.current = isStreamingForCurrentConversation;
  // The final message has arrived if messages grew AND the latest is from assistant
  const lastMsg = messages[messages.length - 1];
  const lastAssistantMessageHasRenderableContent =
    str(lastMsg?.role, "").toLowerCase() === "assistant" &&
    (!!str(lastMsg?.content, "").trim() ||
      clarificationChoices(asRecord(lastMsg).choices).length > 0);
  const finalMessageLanded =
    !isStreamingForCurrentConversation &&
    messages.length > streamStartMsgCount.current &&
    lastAssistantMessageHasRenderableContent;
  // Show streaming bubble while streaming OR while waiting for final message to land
  const showStreamingAssistant =
    isStreamingForCurrentConversation ||
    hasFocusedDraftStream ||
    (hasRecoveredStream && !finalMessageLanded);
  const canStopCurrentRun =
    isStreaming ||
    (pendingSnapshotPhase === "running" &&
      (hasPendingSnapshotForConversation || hasFocusedDraftStream) &&
      Boolean(
        str(pendingRunSnapshot?.runId, "").trim() ||
          str(pendingRunSnapshot?.taskId, "").trim() ||
          liveRunStreamOpen,
      ));
  useEffect(() => {
    if (
      !hasPendingSnapshotForConversation ||
      pendingSnapshotPhase !== "running" ||
      !finalMessageLanded
    ) {
      return;
    }
    const pendingSteps =
      streamingStepsRef.current.length > 0
        ? trimTrailingHeartbeatSteps(streamingStepsRef.current)
        : trimTrailingHeartbeatSteps(streamingSteps);
    const restoredPlan =
      executionPlan ?? extractExecutionPlanFromTraceSteps(pendingSteps);
    if (
      shouldKeepPlanInApprovalState(
        restoredPlan,
        pendingSteps,
        pendingSnapshotMode,
      )
    ) {
      markPendingRunAwaitingPlanConfirmation(
        str(pendingRunSnapshot?.taskId, "").trim(),
      );
      return;
    }
    storeChatPendingRunSnapshotNow(null);
    setPendingRunSnapshot(null);
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponseNow("");
    setStreamingStepsNow([]);
    setStreamingProgressMessages([]);
    resetStreamingProgressBubbleState();
    setPlanConfirmation((prev) =>
      prev?.stage === "running"
        ? {
            ...prev,
            stage: "completed",
            editing: false,
          }
        : prev,
    );
  }, [
    executionPlan,
    finalMessageLanded,
    hasPendingSnapshotForConversation,
    lastMsg,
    pendingRunSnapshot?.taskId,
    pendingSnapshotMode,
    pendingSnapshotPhase,
    streamingSteps,
    resetStreamingProgressBubbleState,
  ]);
    const visiblePendingUserMessage =
      hasLivePendingThread &&
      pendingSnapshotMode === "fresh" &&
      (pendingSnapshotPhase === "running" ||
        pendingSnapshotPhase === "interrupted")
      ? pendingUserMessage ||
        str(pendingRunSnapshot?.message, "").trim() ||
          str(pendingRunSnapshot?.failedUserMessage, "").trim() ||
          null
        : null;
    const interruptedTaskId = str(pendingRunSnapshot?.taskId, "").trim();
    const interruptedRetryMessage = (
      visiblePendingUserMessage ||
      pendingUserMessage ||
      str(pendingRunSnapshot?.message, "").trim() ||
      str(pendingRunSnapshot?.failedUserMessage, "").trim() ||
      failedUserMessage ||
      ""
    ).trim();
    const canRecoverInterruptedRun = Boolean(
      interruptedTaskId || interruptedRetryMessage,
    );
  const pendingUserMessageAccepted =
    pendingSnapshotPhase === "interrupted" ||
    Boolean(str(pendingRunSnapshot?.runId, "").trim()) ||
    streamingSteps.length > 0 ||
    streamingProgressMessages.length > 0 ||
    streamingResponse.trim().length > 0;
  const pendingUserMessageLabel = pendingUserMessageAccepted
    ? "You"
    : "You | sending...";
  const pendingSnapshotStartedAt = pendingRunSnapshot?.startedAt ?? 0;
  const pendingSnapshotInitialMessageCount =
    typeof pendingRunSnapshot?.initialMessageCount === "number" &&
    Number.isFinite(pendingRunSnapshot.initialMessageCount)
      ? Math.max(0, Math.floor(pendingRunSnapshot.initialMessageCount))
      : null;
  const latestPendingUserMessageIndex = useMemo(() => {
    if (!hasLivePendingThread) return -1;
    if (
      pendingSnapshotInitialMessageCount !== null &&
      pendingSnapshotInitialMessageCount < messages.length
    ) {
      for (
        let idx = messages.length - 1;
        idx >= pendingSnapshotInitialMessageCount;
        idx -= 1
      ) {
        const candidate = asRecord(messages[idx]);
        if (str(candidate.role, "").toLowerCase() === "user") {
          return idx;
        }
      }
    }
    if (pendingSnapshotStartedAt <= 0) return -1;
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const candidate = asRecord(messages[idx]);
      if (str(candidate.role, "").toLowerCase() !== "user") continue;
      const tsMs = Date.parse(str(candidate.timestamp, ""));
      if (!Number.isFinite(tsMs)) continue;
      if (tsMs + 1000 < pendingSnapshotStartedAt) continue;
      return idx;
    }
    return -1;
  }, [
    hasLivePendingThread,
    messages,
    pendingSnapshotInitialMessageCount,
    pendingSnapshotStartedAt,
  ]);
  const pendingUserMessageAlreadyPersisted = latestPendingUserMessageIndex !== -1;
  const visibleFailedUserMessage =
    !pendingUserMessageAlreadyPersisted &&
    !hasPendingSnapshotForConversation &&
    !conversationId &&
    !isStreaming &&
    !!failedUserMessage &&
    messages.length === 0
      ? failedUserMessage
      : null;
  useEffect(() => {
    if (!pendingUserMessageAlreadyPersisted) return;
    if (pendingUserMessage) {
      setPendingUserMessage(null);
    }
    if (failedUserMessage) {
      setFailedUserMessage(null);
    }
    if (pendingRunSnapshot?.message || pendingRunSnapshot?.failedUserMessage) {
      setPendingRunSnapshot((prev) => {
        if (!prev) return prev;
        const next = {
          ...prev,
          message: "",
          failedUserMessage: "",
        };
        storeChatPendingRunSnapshotNow(next);
        return next;
      });
    }
  }, [
    failedUserMessage,
    pendingRunSnapshot?.failedUserMessage,
    pendingRunSnapshot?.message,
    pendingUserMessage,
    pendingUserMessageAlreadyPersisted,
  ]);
  // visibleStreamingTranscriptText runs ~18 full-text scans; memoized so it
  // costs once per token flush instead of once per render of any state.
  const visibleStreamingResponse = useMemo(
    () =>
      hasLivePendingThread
        ? visibleStreamingTranscriptText(streamingResponse)
        : "",
    [hasLivePendingThread, streamingResponse],
  );
  // Low-priority copy for the streaming markdown parse: React can interrupt
  // and coalesce the expensive ReactMarkdown re-parse under load instead of
  // blocking input/scroll on every 80ms token flush (mirrors
  // deferredComputerTokenPreview below).
  const deferredStreamingMarkdownText = useDeferredValue(
    visibleStreamingResponse,
  );
  const visibleLiveModelEmit = useMemo(() => {
    if (!hasLivePendingThread || visibleStreamingResponse.trim()) return "";
    const content = reasoningStream?.content || "";
    const normalized = content.replace(/\r\n/g, "\n").trim();
    if (!normalized) return "";
    const blocks = normalized
      .split(/\n{2,}/)
      .map((block) => block.trim())
      .filter(Boolean);
    const latestBlock = blocks[blocks.length - 1] || normalized;
    return latestBlock.length > 1400
      ? latestBlock.slice(-1400).trim()
      : latestBlock;
  }, [hasLivePendingThread, reasoningStream?.content, visibleStreamingResponse]);
  // Latest interleaved model narration (model_prose ToolProgress) for live
  // display in the work panel during a tool-using run. Rendered directly in the
  // panel branch (which is proven to mount during the run) rather than via the
  // reply-bubble path, which doesn't flip for tool turns.
  const liveModelProseText = useMemo(() => {
    if (!hasLivePendingThread) return "";
    for (let i = streamingSteps.length - 1; i >= 0; i -= 1) {
      const t = modelProseTextFromActivityStep(streamingSteps[i] as JsonRecord);
      if (t && t.trim()) return t.trim();
    }
    return "";
  }, [hasLivePendingThread, streamingSteps]);
  const deferredLiveModelEmitText = useDeferredValue(visibleLiveModelEmit);
  const visibleStreamingMarkdownText = visibleStreamingResponse.trim()
    ? deferredStreamingMarkdownText
    : deferredLiveModelEmitText;
  const computerTokenPreview = useMemo(() => {
    if (!visibleStreamingResponse) return "";
    return visibleStreamingResponse.length > CHAT_COMPUTER_TOKEN_PREVIEW_MAX_CHARS
      ? visibleStreamingResponse.slice(-CHAT_COMPUTER_TOKEN_PREVIEW_MAX_CHARS)
      : visibleStreamingResponse;
  }, [visibleStreamingResponse]);
  const deferredComputerTokenPreview = useDeferredValue(computerTokenPreview);
  const interruptedRunDetail = useMemo(
    () =>
      interruptedRunDetailFromSteps(
        trimTrailingHeartbeatSteps(
          streamingSteps.length > 0
            ? streamingSteps
            : (pendingRunSnapshot?.streamingSteps ?? []),
        ),
      ),
    [pendingRunSnapshot?.streamingSteps, streamingSteps],
  );
  const streamingResearchPrompt =
    visiblePendingUserMessage ||
    (() => {
      for (let cursor = messages.length - 1; cursor >= 0; cursor -= 1) {
        const candidate = asRecord(messages[cursor]);
        if (str(candidate.role, "").toLowerCase() !== "user") continue;
        return stripAttachmentContextMarker(str(candidate.content, ""));
      }
      return "";
    })();
  const streamingResearchReport = useMemo(
    () =>
      showStreamingAssistant
        ? parseResearchReportWithContext(visibleStreamingResponse, {
            deepResearch: isDeepResearchPlanSource(planConfirmation?.source),
            previousUserPrompt: streamingResearchPrompt,
            conversationTitle: str(selectedConversation?.title, ""),
          })
        : null,
    [
      showStreamingAssistant,
      visibleStreamingResponse,
      planConfirmation?.source,
      streamingResearchPrompt,
      selectedConversation?.title,
    ],
  );
  const completedProgressSnapshot =
    !hasPendingSnapshotForConversation && conversationId
      ? completedProgressMessagesByConversation[conversationId] || null
      : null;
  const suppressInChatPlanInterimUpdates =
    planConfirmation?.stage === "awaiting_confirmation" ||
    planConfirmation?.stage === "running" ||
    planConfirmation?.stage === "completed" ||
    planConfirmation?.stage === "interrupted";
  const visibleStreamingProgressMessages = hasLivePendingThread
    ? suppressInChatPlanInterimUpdates
      ? []
      : streamingProgressMessages
    : completedProgressSnapshot?.messages || [];
  const completedProgressBeforeMessageId =
    completedProgressSnapshot?.beforeMessageId || "";
  const latestStreamingAssistantIndex = useMemo(() => {
    if (!showStreamingAssistant || pendingSnapshotStartedAt <= 0) return -1;
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const candidate = asRecord(messages[idx]);
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      const candidateContent = str(candidate.content, "").trim();
      const candidateChoices = clarificationChoices(candidate.choices);
      if (!candidateContent && candidateChoices.length === 0) continue;
      const tsMs = Date.parse(str(candidate.timestamp, ""));
      if (Number.isFinite(tsMs) && tsMs + 1000 < pendingSnapshotStartedAt) {
        continue;
      }
      return idx;
    }
    return -1;
  }, [messages, pendingSnapshotStartedAt, showStreamingAssistant]);
  const planConfirmationMessageIndex = useMemo(() => {
    if (!planConfirmation) return -1;
    const anchoredMessageId = str(planConfirmation.messageId, "").trim();
    if (anchoredMessageId) {
      for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
        const candidate = asRecord(messages[idx]);
        const candidateId = str(candidate.id, String(idx));
        if (candidateId === anchoredMessageId) return idx;
      }
    }
    const shouldAnchorBeforeRunStart =
      planConfirmation.stage === "running" ||
      planConfirmation.stage === "completed" ||
      planConfirmation.stage === "failed";
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const candidate = asRecord(messages[idx]);
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      const tsMs = Date.parse(str(candidate.timestamp, ""));
      if (Number.isFinite(tsMs) && pendingSnapshotStartedAt > 0) {
        if (shouldAnchorBeforeRunStart) {
          if (tsMs > pendingSnapshotStartedAt + 1000) {
            continue;
          }
        } else if (tsMs + 1000 < pendingSnapshotStartedAt) {
          continue;
        }
      }
      return idx;
    }
    return -1;
  }, [messages, pendingSnapshotStartedAt, planConfirmation]);
  const hasLiveThreadActivity = Boolean(
    visiblePendingUserMessage ||
    visibleFailedUserMessage ||
    isStreamingForCurrentConversation ||
    hasLivePendingThread ||
    visibleStreamingResponse.trim() ||
    selectedConversationAwaitingPersistedMessages,
  );
  const hasRenderableThread = messages.length > 0 || hasLiveThreadActivity;
  const showEmptyHero =
    !hasRenderableThread &&
    !showStreamingAssistant &&
    !visiblePendingUserMessage &&
    !visibleFailedUserMessage;

  useLayoutEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    if (showEmptyHero) {
      thread.scrollTop = 0;
      return;
    }
    if (stickToBottom.current) {
      thread.scrollTop = thread.scrollHeight;
    }
  }, [
    showEmptyHero,
    messages.length,
    pendingUserMessage,
    failedUserMessage,
    Math.floor(streamingResponse.length / 36),
    streamingProgressMessages.length,
    isStreaming,
  ]);

  useEffect(() => {
    setEmptyEarlyAccessNoticeDismissed(readEarlyAccessNoticeDismissed());
  }, [showEmptyHero]);

  const shouldInlineCompletedProgressBeforeAssistant =
    !showStreamingAssistant &&
    visibleStreamingProgressMessages.length > 0 &&
    !!completedProgressBeforeMessageId;
  const renderAgentAvatar = (extraClassName = "") => (
    <Avatar
      variant="rounded"
      className={`chat-avatar chat-avatar-agent${extraClassName ? ` ${extraClassName}` : ""}`}
      sx={{ width: 44, height: 44 }}
    >
      <Box
        component="img"
        src={AgentLogo}
        alt="AgentArk"
        className="chat-avatar-agent-logo"
      />
    </Avatar>
  );
  const renderUserAvatar = (extraClassName = "") => (
    <Avatar
      className={`chat-avatar chat-avatar-user${extraClassName ? ` ${extraClassName}` : ""}`}
      sx={{ width: 38, height: 38 }}
    >
      <Box className="chat-avatar-user-shell">
        <UserRound className="chat-avatar-user-icon" aria-hidden="true" />
        <Sparkles className="chat-avatar-user-accent" aria-hidden="true" />
      </Box>
    </Avatar>
  );
  const renderProgressRows = (_keyPrefix: string) => null;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const latestAssistantMessageText = useMemo(() => {
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const message = asRecord(messages[idx]);
      if (str(message.role, "").toLowerCase() === "assistant") {
        return str(message.content, "");
      }
    }
    return "";
  }, [messages]);
  const workspaceSnippetFiles = useMemo(
    () => buildWorkspaceSnippetFiles(messages),
    [messages],
  );
  useEffect(() => {
    if (!selectedSnippetId) return;
    if (
      workspaceSnippetFiles.some((snippet) => snippet.id === selectedSnippetId)
    ) {
      if (selectedSnippetOverride?.id === selectedSnippetId) {
        setSelectedSnippetOverride(null);
      }
      return;
    }
    if (selectedSnippetOverride?.id === selectedSnippetId) {
      return;
    }
    setSelectedSnippetId(null);
  }, [selectedSnippetId, selectedSnippetOverride, workspaceSnippetFiles]);
  // Keep this callback stable so messageRenderBundle memoization only busts
  // when message data changes.
  const openCodePreviewInWorkspace = useCallback(
    (request: CodePreviewOpenRequest) => {
      const normalizedCode = str(request.code, "")
        .replace(/\r\n/g, "\n")
        .replace(/\n$/, "");
      const displayName =
        str(request.fileName, "").trim() ||
        inferCodePreviewFileName(request.languageHint, normalizedCode);
      const snippetId =
        request.snippetId ||
        `preview::${displayName}::${normalizedCode.length}`;
      setWorkspaceOpen(true);
      setActiveStepId(null);
      setSelectedSnippetId(snippetId);
      if (normalizedCode) {
        setSelectedSnippetOverride({
          id: snippetId,
          name: displayName,
          displayName,
          content: normalizedCode,
          languageHint: normalizeCodeFenceLanguage(request.languageHint),
          sourceMessageId: "",
          sourceLabel: "Current reply",
        });
      }
    },
    [],
  );
  const messageRenderBundle = useMemo(() => {
    let latestChoiceMessageIndex = -1;
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const candidate = asRecord(messages[idx]);
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      if (clarificationChoices(candidate.choices).length > 0) {
        latestChoiceMessageIndex = idx;
        break;
      }
    }
    return messages.map((raw, idx) => {
      const message = asRecord(raw);
      const role = str(message.role, "").toLowerCase();
      const isUser = role === "user";
      const isAssistant = role === "assistant";
      const messageId = str(message.id, String(idx));
      const tsRaw = str(message.timestamp, "");
      const ts = tsRaw ? formatChatTimestamp(tsRaw) : null;
      const content = str(message.content);
      const renderedContent = isUser
        ? stripAttachmentContextMarker(content)
        : stripAgentInternalReasoningLeaks(content);
      const attachments = isUser ? extractChatTurnAttachments(content) : [];
      const rawMessageChoices = isAssistant
        ? clarificationChoices(message.choices)
        : [];
      // Only render choice buttons on the FIRST assistant message in the
      // latest assistant message that currently carries choices. Older
      // approvals can remain in history, but only the current pending choice
      // should be actionable in the chat lane.
      let messageChoices: ChatClarificationChoice[] = [];
      if (rawMessageChoices.length > 0 && idx === latestChoiceMessageIndex) {
        messageChoices = rawMessageChoices;
      }
      const previousUserPrompt = previousUserPromptByIndex.get(idx) || "";
      const researchReport = isAssistant
        ? parseResearchReportWithContext(renderedContent, {
            deepResearch: isDeepResearchAssistantMessage(message),
            previousUserPrompt,
            conversationTitle: str(selectedConversation?.title, ""),
          })
        : null;
      const traceId = str(message.trace_id, "").trim();
      const hasTrace = !isUser && !!traceId;
      const markdownNode =
        !isUser && !researchReport
          ? renderChatMarkdown(renderedContent, {
              snippetNamespace: messageId,
              onOpenSnippet: openCodePreviewInWorkspace,
            })
          : null;
      const runMetrics = isAssistant
        ? chatRunMetricsFromPayload(message)
        : null;
      const runMetricItems = isAssistant && runMetrics
        ? buildChatRunMetricItems(runMetrics)
        : [];
      return {
        message,
        idx,
        messageId,
        role,
        isUser,
        isAssistant,
        tsRaw,
        ts,
        renderedContent,
        attachments,
        messageChoices,
        researchReport,
        previousUserPrompt,
        traceId,
        hasTrace,
        markdownNode,
        runMetricItems,
        runMetrics,
      };
    });
  }, [
    messages,
    previousUserPromptByIndex,
    openCodePreviewInWorkspace,
    selectedConversation?.title,
  ]);
  const latestAssistantTraceSteps = useMemo(
    () => (latestAssistantTraceId ? traceStepsById[latestAssistantTraceId] || [] : []),
    [latestAssistantTraceId, traceStepsById],
  );
  const completedLastRunSteps = useMemo(
    () => trimTrailingHeartbeatSteps(lastRunSteps),
    [lastRunSteps],
  );
  const completedPersistedTraceSteps = useMemo(
    () => trimTrailingHeartbeatSteps(latestAssistantTraceSteps),
    [latestAssistantTraceSteps],
  );
  const mergePersistedReasoningIntoSteps = (
    primarySteps: JsonRecord[],
    persistedSteps: JsonRecord[],
  ): JsonRecord[] => {
    if (persistedSteps.length === 0) return primarySteps;
    const persistedReasoningSteps = persistedSteps.filter(isMainChatReasoningStep);
    if (persistedReasoningSteps.length === 0) return primarySteps;
    if (primarySteps.length === 0) return persistedSteps;
    return trimTrailingHeartbeatSteps(
      compressActivitySteps([...persistedReasoningSteps, ...primarySteps]),
    );
  };
  const persistedExecutionPlan = useMemo(
    () =>
      showStreamingAssistant || hasPendingSnapshotForConversation
        ? null
        : extractExecutionPlanFromTraceSteps(completedPersistedTraceSteps),
    [
      showStreamingAssistant,
      hasPendingSnapshotForConversation,
      completedPersistedTraceSteps,
    ],
  );
  const persistedExecutionPlanFailure = useMemo(
    () =>
      showStreamingAssistant || hasPendingSnapshotForConversation
        ? ""
        : extractExecutionPlanFailureFromTraceSteps(
            completedPersistedTraceSteps,
          ),
    [
      showStreamingAssistant,
      hasPendingSnapshotForConversation,
      completedPersistedTraceSteps,
    ],
  );
  const displayedExecutionPlanState = executionPlan ?? persistedExecutionPlan;
  const displayedExecutionPlan = displayedExecutionPlanState?.steps || [];
  const displayedExecutionPlanSummary = str(
    displayedExecutionPlanState?.summary,
    "",
  );
  const displayedExecutionPlanFailure =
    executionPlanFailure || persistedExecutionPlanFailure;
  const hasVisibleExecutionPlanContext =
    displayedExecutionPlan.length > 0 ||
    !!displayedExecutionPlanFailure ||
    (planConfirmation?.originalPlan?.steps?.length ?? 0) > 0 ||
    (persistedExecutionPlan?.steps?.length ?? 0) > 0;
  const visibleExecutionPlanFailure = hasVisibleExecutionPlanContext
    ? displayedExecutionPlanFailure
    : "";
  const planConfirmationDraftPlan = useMemo(
    () =>
      buildExecutionPlanFromDraft(
        planConfirmation?.draft ?? null,
        planConfirmation?.originalPlan ?? null,
      ),
    [planConfirmation],
  );
  const planConfirmationIsDeepResearch = isDeepResearchPlanSource(
    planConfirmation?.source,
  );
  const isAwaitingPlanConfirmation =
    planConfirmationIsDeepResearch &&
    planConfirmation?.stage === "awaiting_confirmation";
  const composerAwaitingPlanConfirmation =
    isAwaitingPlanConfirmation &&
    !isStreamingForCurrentConversation &&
    !showStreamingAssistant;
  const isRunningPlanConfirmation =
    planConfirmationIsDeepResearch && planConfirmation?.stage === "running";
  const isCompletedPlanConfirmation =
    planConfirmationIsDeepResearch && planConfirmation?.stage === "completed";
  const isFailedPlanConfirmation =
    planConfirmationIsDeepResearch && planConfirmation?.stage === "failed";
  const isInterruptedPlanConfirmation =
    planConfirmationIsDeepResearch &&
    planConfirmation?.stage === "interrupted";
  const isLivePlanConfirmation =
    isRunningPlanConfirmation ||
    isCompletedPlanConfirmation ||
    isFailedPlanConfirmation ||
    isInterruptedPlanConfirmation;
  const activePlanConfirmationState = useMemo(
    () =>
      isLivePlanConfirmation
        ? mergeExecutionPlanProgress(
            planConfirmationDraftPlan ?? planConfirmation?.originalPlan ?? null,
            displayedExecutionPlanState,
          )
        : null,
    [
      displayedExecutionPlanState,
      isLivePlanConfirmation,
      planConfirmation,
      planConfirmationDraftPlan,
    ],
  );
  const showPlanConfirmationCard =
    planConfirmationIsDeepResearch &&
    (isAwaitingPlanConfirmation ||
      composerAwaitingPlanConfirmation ||
      isRunningPlanConfirmation ||
      isCompletedPlanConfirmation ||
      isInterruptedPlanConfirmation);
  const planConfirmationEnabledCount =
    planConfirmation?.draft?.steps.filter((step) => step.enabled).length ?? 0;
  const planConfirmationDisabledCount =
    (planConfirmation?.draft?.steps.length ?? 0) - planConfirmationEnabledCount;
  const planConfirmationVisibleSteps = planConfirmation?.draft?.steps || [];
  const planConfirmationSummaryText = str(
    planConfirmation?.draft?.summary,
    str(planConfirmation?.originalPlan?.summary, ""),
  );
  const planConfirmationDepthCue = composerAwaitingPlanConfirmation
    ? planConfirmationEnabledCount > 0
      ? `Will expand these ${planConfirmationEnabledCount} approved step${planConfirmationEnabledCount === 1 ? "" : "s"} into live execution after you press Start.`
      : "Select at least one step to continue."
    : isAwaitingPlanConfirmation
      ? planConfirmationEnabledCount > 0
        ? `Will expand these ${planConfirmationEnabledCount} approved step${planConfirmationEnabledCount === 1 ? "" : "s"} into live execution after you press Start.`
        : "Select at least one step to continue."
    : isRunningPlanConfirmation
      ? "This is the approved plan. Live substeps and status updates should stay attached here while the run executes."
      : isCompletedPlanConfirmation
        ? "This is the approved plan with the final execution progress merged into it."
        : isFailedPlanConfirmation
          ? "This is the approved plan with the latest execution state preserved from the failed run."
          : isInterruptedPlanConfirmation
            ? "This run was interrupted. The approved plan and the last saved progress are preserved here."
            : "Preparing a structured execution outline.";

  const applyStarterExample = (example: ChatStarterExample) => {
    queueComposerPrefill({
      text: example.prompt,
      browser_profile_context: null,
    });
    setDeepResearchEnabled(Boolean(example.deepResearch) && !deepResearchDisabled);
    setChatError(null);
    setChatNotice(null);
  };

  const starterVisibleExamples =
    starterActiveTab === "all"
      ? CHAT_STARTER_DEFAULT_EXAMPLES
      : CHAT_STARTER_EXAMPLES.filter(
          (example) => example.category === starterActiveTab,
        );

  const renderStarterExampleCard = (item: ChatStarterExample) => {
    const categoryMeta = CHAT_STARTER_CATEGORY_META[item.category];
    const CategoryIcon = CHAT_STARTER_CATEGORY_ICON[item.category];
    return (
      <Button
        key={item.id}
        variant="outlined"
        className="chat-starter-card"
        data-category={item.category}
        onClick={() => applyStarterExample(item)}
      >
        <Box className="chat-starter-card-body">
          <span className="chat-starter-card-icon" aria-hidden="true">
            <CategoryIcon size={22} strokeWidth={1.75} />
          </span>
          <Box className="chat-starter-card-main">
            <Box className="chat-starter-card-meta">
              <span className="chat-starter-tag">{categoryMeta.label}</span>
              {item.deepResearch && item.category !== "research" ? (
                <span className="chat-starter-tag research">Deep research</span>
              ) : null}
            </Box>
            <Typography component="span" className="chat-starter-card-title">
              {item.title}
            </Typography>
            <Typography component="span" className="chat-starter-card-copy">
              {item.summary}
            </Typography>
          </Box>
          <span className="chat-starter-card-go" aria-hidden="true">
            <ArrowRight size={16} strokeWidth={2} />
          </span>
        </Box>
      </Button>
    );
  };

  useEffect(() => {
    if (!planConfirmation) return;
    if (str(planConfirmation.messageId, "").trim()) return;
    if (planConfirmationMessageIndex < 0) return;
    const candidate = asRecord(messages[planConfirmationMessageIndex]);
    const candidateId = str(
      candidate.id,
      String(planConfirmationMessageIndex),
    ).trim();
    if (!candidateId) return;
    setPlanConfirmation((prev) =>
      prev && !str(prev.messageId, "").trim()
        ? {
            ...prev,
            messageId: candidateId,
          }
        : prev,
    );
  }, [messages, planConfirmation, planConfirmationMessageIndex]);

  useEffect(() => {
    if (executionPlan || streamingSteps.length === 0) return;
    const restoredPlan = extractExecutionPlanFromTraceSteps(
      trimTrailingHeartbeatSteps(streamingSteps),
    );
    if (!restoredPlan) return;
    setExecutionPlan(restoredPlan);
    setExecutionPlanFailure("");
  }, [executionPlan, streamingSteps]);

  const approvalRepairSourceSteps = useMemo(
    () =>
      trimTrailingHeartbeatSteps(
        streamingSteps.length > 0
          ? streamingSteps
          : (pendingRunSnapshot?.streamingSteps ?? []),
      ),
    [pendingRunSnapshot?.streamingSteps, streamingSteps],
  );
  const shouldRepairApprovalState = useMemo(
    () =>
      hasPendingSnapshotForConversation &&
      !isStreamingForCurrentConversation &&
      !showStreamingAssistant &&
      pendingSnapshotPhase === "awaiting_confirmation" &&
      pendingSnapshotMode !== "resume" &&
      shouldKeepPlanInApprovalState(
        executionPlan,
        approvalRepairSourceSteps,
        pendingSnapshotMode,
      ),
    [
      approvalRepairSourceSteps,
      executionPlan,
      hasPendingSnapshotForConversation,
      isStreamingForCurrentConversation,
      pendingSnapshotMode,
      pendingSnapshotPhase,
      showStreamingAssistant,
    ],
  );

  useEffect(() => {
    if (!shouldRepairApprovalState || !executionPlan) return;
    const resolvedTaskId = str(pendingRunSnapshot?.taskId, "").trim() || null;
    if (pendingSnapshotPhase !== "awaiting_confirmation") {
      markPendingRunAwaitingPlanConfirmation(str(resolvedTaskId, ""));
    }
    const needsRepair =
      !planConfirmation ||
      planConfirmation.stage !== "awaiting_confirmation" ||
      !planConfirmation.originalPlan ||
      !planConfirmation.draft ||
      (!!resolvedTaskId && planConfirmation.taskId !== resolvedTaskId);
    if (!needsRepair) return;
    setPlanConfirmation((prev) => {
      const nextOriginalPlan = prev?.originalPlan ?? executionPlan;
      const nextDraft =
        prev?.draft ?? createPlanConfirmationDraft(nextOriginalPlan);
      return {
        stage: "awaiting_confirmation",
        taskId: str(prev?.taskId, resolvedTaskId || "").trim() || null,
        source:
          prev?.source ||
          extractPlanConfirmationSourceFromSteps(approvalRepairSourceSteps) ||
          "execution",
        originalPlan: nextOriginalPlan,
        draft: nextDraft,
        editing: false,
        messageId: prev?.messageId ?? null,
      };
    });
  }, [
    approvalRepairSourceSteps,
    executionPlan,
    pendingRunSnapshot?.taskId,
    pendingSnapshotPhase,
    planConfirmation,
    shouldRepairApprovalState,
  ]);

  useEffect(() => {
    if (
      !executionPlan ||
      pendingSnapshotPhase !== "awaiting_confirmation" ||
      isStreamingForCurrentConversation ||
      showStreamingAssistant
    )
      return;
    const resolvedTaskId = str(pendingRunSnapshot?.taskId, "").trim() || null;
    const inferredSource =
      planConfirmation?.source ||
      extractPlanConfirmationSourceFromSteps(approvalRepairSourceSteps) ||
      "execution";
    const needsRepair =
      !planConfirmation ||
      !planConfirmation.originalPlan ||
      !planConfirmation.draft ||
      (!!resolvedTaskId && planConfirmation.taskId !== resolvedTaskId);
    if (!needsRepair) return;
    setPlanConfirmation((prev) => {
      const nextOriginalPlan = prev?.originalPlan ?? executionPlan;
      const nextDraft =
        prev?.draft ?? createPlanConfirmationDraft(nextOriginalPlan);
      return {
        stage: "awaiting_confirmation",
        taskId: str(prev?.taskId, resolvedTaskId || "").trim() || null,
        source: prev?.source || inferredSource,
        originalPlan: nextOriginalPlan,
        draft: nextDraft,
        editing: false,
        messageId: prev?.messageId ?? null,
      };
    });
    if (planConfirmation?.stage !== "awaiting_confirmation") {
      setChatNotice(
        `${planConfirmationDisplayLabel(inferredSource)} ready. Review it, edit it, or ask for changes below.`,
      );
    }
  }, [
    approvalRepairSourceSteps,
    executionPlan,
    isStreamingForCurrentConversation,
    pendingRunSnapshot?.taskId,
    pendingSnapshotPhase,
    planConfirmation,
    showStreamingAssistant,
  ]);

  useEffect(() => {
    if (
      !executionPlan ||
      pendingSnapshotPhase !== "interrupted" ||
      !hasPendingSnapshotForConversation
    ) {
      return;
    }
    const resolvedTaskId = str(pendingRunSnapshot?.taskId, "").trim() || null;
    const needsRepair =
      !planConfirmation ||
      planConfirmation.stage !== "interrupted" ||
      !planConfirmation.originalPlan ||
      !planConfirmation.draft ||
      (!!resolvedTaskId && planConfirmation.taskId !== resolvedTaskId);
    if (!needsRepair) return;
    setPlanConfirmation((prev) => {
      const nextOriginalPlan = prev?.originalPlan ?? executionPlan;
      const nextDraft =
        prev?.draft ?? createPlanConfirmationDraft(nextOriginalPlan);
      return {
        stage: "interrupted",
        taskId: str(prev?.taskId, resolvedTaskId || "").trim() || null,
        source:
          prev?.source ||
          extractPlanConfirmationSourceFromSteps(approvalRepairSourceSteps) ||
          "execution",
        originalPlan: nextOriginalPlan,
        draft: nextDraft,
        editing: false,
        messageId: prev?.messageId ?? null,
      };
    });
  }, [
    executionPlan,
    hasPendingSnapshotForConversation,
    pendingRunSnapshot?.taskId,
    pendingSnapshotPhase,
    planConfirmation,
  ]);

  useEffect(() => {
    if (
      !executionPlan ||
      pendingSnapshotPhase !== "running" ||
      !hasPendingSnapshotForConversation ||
      shouldRepairApprovalState
    ) {
      return;
    }
    const resolvedTaskId = str(pendingRunSnapshot?.taskId, "").trim() || null;
    const needsRepair =
      !planConfirmation ||
      !["running", "completed", "failed"].includes(planConfirmation.stage) ||
      !planConfirmation.originalPlan ||
      !planConfirmation.draft ||
      (!!resolvedTaskId && planConfirmation.taskId !== resolvedTaskId);
    if (!needsRepair) return;
    setPlanConfirmation((prev) => {
      const nextOriginalPlan = prev?.originalPlan ?? executionPlan;
      return {
        stage: "running",
        taskId: str(prev?.taskId, resolvedTaskId || "").trim() || null,
        source:
          prev?.source ||
          extractPlanConfirmationSourceFromSteps(approvalRepairSourceSteps) ||
          "execution",
        originalPlan: nextOriginalPlan,
        draft: createPlanConfirmationDraft(nextOriginalPlan),
        editing: false,
        messageId: prev?.messageId ?? null,
      };
    });
  }, [
    approvalRepairSourceSteps,
    executionPlan,
    hasPendingSnapshotForConversation,
    pendingRunSnapshot?.taskId,
    pendingSnapshotPhase,
    planConfirmation,
    shouldRepairApprovalState,
  ]);

  const completedWorkspaceSteps = useMemo(
    () =>
      completedLastRunSteps.length > 0
        ? mergePersistedReasoningIntoSteps(
            completedLastRunSteps,
            completedPersistedTraceSteps,
          )
        : completedPersistedTraceSteps,
    [completedLastRunSteps, completedPersistedTraceSteps],
  );

  const workspaceStepsUseActiveRun =
    (showStreamingAssistant || hasPendingSnapshotForConversation) &&
    streamingSteps.length > 0;
  const workspaceStepsSource = useMemo(
    () =>
      workspaceStepsUseActiveRun
        ? trimTrailingHeartbeatSteps(streamingSteps)
        : completedWorkspaceSteps,
    [
      completedWorkspaceSteps,
      streamingSteps,
      workspaceStepsUseActiveRun,
    ],
  );
  const workspaceActivityRestoreState = useMemo(
    () => workspaceStateFromActivitySteps(workspaceStepsSource),
    [workspaceStepsSource],
  );
  useEffect(() => {
    if (!conversationId) {
      lastWorkspaceActivityRestoreSeedRef.current = "";
      return;
    }
    const restoredApp = sanitizeWorkspaceAppSnapshot(
      workspaceActivityRestoreState.app,
    );
    const appDir = str(restoredApp?.app_dir, "");
    const restoredFiles = workspaceActivityRestoreState.deployedFiles;
    const restoredLiveWrites = workspaceActivityRestoreState.liveFileWrites;
    if (
      !restoredApp &&
      restoredFiles.length === 0 &&
      Object.keys(restoredLiveWrites).length === 0
    ) {
      if (workspaceStepsUseActiveRun) {
        lastWorkspaceActivityRestoreSeedRef.current = JSON.stringify({
          conversationId,
          active: true,
          emptyWorkspace: true,
        });
        streamedWorkspaceAppRef.current = null;
        setStreamedWorkspaceApp(null);
        setDeployedFiles((prev) => (prev.length > 0 ? [] : prev));
        setLiveFileWrites((prev) => (Object.keys(prev).length > 0 ? {} : prev));
      }
      return;
    }
    const restoreSeed = JSON.stringify({
      conversationId,
      appId: str(restoredApp?.id, str(restoredApp?.app_id, "")),
      files: restoredFiles.map((file) => [file.name, file.content.length]),
      liveWrites: Object.entries(restoredLiveWrites).map(([name, state]) => [
        name,
        state.content.length,
        state.line,
        state.totalLines,
        state.done,
      ]),
    });
    if (lastWorkspaceActivityRestoreSeedRef.current === restoreSeed) return;
    lastWorkspaceActivityRestoreSeedRef.current = restoreSeed;
    if (restoredApp) {
      streamedWorkspaceAppRef.current = {
        ...(streamedWorkspaceAppRef.current || {}),
        ...restoredApp,
      };
      setStreamedWorkspaceApp(streamedWorkspaceAppRef.current);
    }
    if (restoredFiles.length > 0) {
      setDeployedFiles((prev) =>
        mergeWorkspaceFiles(prev, restoredFiles, appDir),
      );
      setCodeViewerFileIdx((prev) =>
        Math.min(Math.max(0, prev), Math.max(0, restoredFiles.length - 1)),
      );
    }
    if (Object.keys(restoredLiveWrites).length > 0) {
      setLiveFileWrites((prev) =>
        mergeLiveFileWriteStates(prev, restoredLiveWrites, appDir),
      );
    }
  }, [conversationId, workspaceActivityRestoreState, workspaceStepsUseActiveRun]);
  const liveChatTranscriptItems = useMemo(
    () =>
      buildChatTranscriptItemsFromSteps(
        trimTrailingHeartbeatSteps(
          streamingSteps.length > 0 ? streamingSteps : workspaceStepsSource,
        ),
        "live-transcript",
        8,
      ).filter((item) => item.kind === "action" || item.kind === "prose"),
    [buildChatTranscriptItemsFromSteps, streamingSteps, workspaceStepsSource],
  );
  const livePlanPhaseStatuses = useMemo(() => {
    const plan =
      activePlanConfirmationState ??
      executionPlan ??
      persistedExecutionPlan ??
      null;
    const activeStep =
      plan?.steps.find((step) => step.status === "running") || null;
    if (!activeStep) return [] as StreamPhaseStatus[];
    const latestByKey = new Map<string, StreamPhaseStatus>();
    const sourceSteps = trimTrailingHeartbeatSteps(
      streamingSteps.length > 0 ? streamingSteps : workspaceStepsSource,
    );
    for (const step of sourceSteps) {
      const phaseStatus = extractPhaseStatusFromActivityStep(step);
      if (!phaseStatus) continue;
      if (phaseStatus.planStepId && phaseStatus.planStepId !== activeStep.id)
        continue;
      if (
        !phaseStatus.planStepId &&
        phaseStatus.planStepTitle &&
        phaseStatus.planStepTitle.trim().toLowerCase() !==
          activeStep.title.trim().toLowerCase()
      ) {
        continue;
      }
      latestByKey.set(phaseStatus.streamKey, phaseStatus);
    }
    const phaseOrder = [
      "planning",
      "searching",
      "ranking",
      "reading",
      "synthesis",
    ];
    return Array.from(latestByKey.values()).sort((left, right) => {
      const leftIndex = phaseOrder.indexOf(left.phase.toLowerCase());
      const rightIndex = phaseOrder.indexOf(right.phase.toLowerCase());
      if (leftIndex === rightIndex)
        return left.label.localeCompare(right.label);
      if (leftIndex === -1) return 1;
      if (rightIndex === -1) return -1;
      return leftIndex - rightIndex;
    });
  }, [
    activePlanConfirmationState,
    executionPlan,
    persistedExecutionPlan,
    streamingSteps,
    workspaceStepsSource,
  ]);
  const restoredPhaseStatus = useMemo(() => {
    const sourceSteps = trimTrailingHeartbeatSteps(
      streamingSteps.length > 0 ? streamingSteps : workspaceStepsSource,
    );
    const statuses = sourceSteps
      .map((step) => extractPhaseStatusFromActivityStep(step))
      .filter((status): status is StreamPhaseStatus => Boolean(status));
    if (statuses.length === 0) return null;
    const latestRunning = [...statuses]
      .reverse()
      .find((status) => status.status === "running");
    return latestRunning || statuses[statuses.length - 1];
  }, [streamingSteps, workspaceStepsSource]);
  const latestRunStatusSummary = useMemo(
    () => extractLatestRunStatusSummary(workspaceStepsSource),
    [workspaceStepsSource],
  );
  const workspacePlanConfirmationSource = useMemo(
    () => extractPlanConfirmationSourceFromSteps(workspaceStepsSource),
    [workspaceStepsSource],
  );
  const deepResearchPlanPreviewMessageId = useMemo(() => {
    for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
      const candidate = asRecord(messages[idx]);
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      const traceId = str(candidate.trace_id, "").trim();
      const traceSteps = traceId ? traceStepsById[traceId] || [] : [];
      if (traceSteps.length === 0) continue;
      if (
        !isDeepResearchPlanSource(
          extractPlanConfirmationSourceFromSteps(traceSteps),
        )
      ) {
        continue;
      }
      if (!activityStepsRepresentAwaitingPlanConfirmation(traceSteps)) continue;
      return str(candidate.id, String(idx)).trim() || null;
    }
    return null;
  }, [messages, traceStepsById]);
  const workspaceSteps = useMemo(() => {
    const compressed = compressActivitySteps(workspaceStepsSource);
    if (
      pendingSnapshotPhase === "awaiting_confirmation" ||
      planConfirmation?.stage === "awaiting_confirmation"
    ) {
      return compressed.filter((step) => !isHeartbeatStreamingStep(step));
    }
    return compressed;
  }, [workspaceStepsSource, pendingSnapshotPhase, planConfirmation?.stage]);
  const workspaceConsoleSteps = useMemo(() => {
    const raw = compressActivitySteps(
      trimTrailingHeartbeatSteps(workspaceStepsSource).map((step) =>
        normalizeActivityStepTime(normalizePlanStepUpdateStep(step)),
      ),
    );
    if (
      pendingSnapshotPhase === "awaiting_confirmation" ||
      planConfirmation?.stage === "awaiting_confirmation"
    ) {
      return raw.filter((step) => !isHeartbeatStreamingStep(step));
    }
    return raw;
  }, [workspaceStepsSource, pendingSnapshotPhase, planConfirmation?.stage]);
  const hasDeepResearchPlanContext = Boolean(
    isDeepResearchPlanSource(planConfirmation?.source) ||
    isDeepResearchPlanSource(workspacePlanConfirmationSource) ||
    deepResearchPlanPreviewMessageId,
  );
  const swarmActivityRuns = useMemo(
    () =>
      buildSwarmRunsFromStreamingSteps(workspaceSteps, {
        interrupted: showInterruptedRunCard,
      }),
    [workspaceSteps, showInterruptedRunCard],
  );
  const workspaceCards = useMemo(() => {
    return limitActivityStepsForRender(workspaceSteps).map((step, idx) =>
      safeBuildStepCard(step, idx),
    );
  }, [workspaceSteps]);
  const workspaceConsoleCards = useMemo(() => {
    return limitActivityStepsForRender(workspaceConsoleSteps).map((step, idx) =>
      safeBuildStepCard(step, idx),
    );
  }, [workspaceConsoleSteps]);
  const computerTaskProgress = useMemo(
    () =>
      latestTaskProgressFromSteps(workspaceConsoleSteps) ||
      latestTaskProgressFromSteps(workspaceSteps) ||
      taskProgressFromExecutionPlan(displayedExecutionPlanState),
    [displayedExecutionPlanState, workspaceConsoleSteps, workspaceSteps],
  );
  const inlineWorkspaceCards = useMemo(() => {
    if (!showStreamingAssistant && !hasPendingSnapshotForConversation) {
      return [];
    }
    const meaningful = workspaceCards.filter((card) => !card.isHeartbeat);
    return meaningful.length > 0 ? meaningful : workspaceCards;
  }, [hasPendingSnapshotForConversation, showStreamingAssistant, workspaceCards]);
  // Build compact per-assistant-message transcript rows from persisted traces.
  // This keeps prior tool actions visible after a new turn begins. Rows stay
  // collapsed in chat, and expanded rows reveal exact structured input/output.
  const perMessageTraceTranscriptById = useMemo(() => {
    const byMessageId: Record<string, ChatTranscriptItem[]> = {};
    for (const bundle of messageRenderBundle) {
      if (!bundle.isAssistant || !bundle.traceId) continue;
      const steps = traceStepsById[bundle.traceId] || [];
      if (steps.length === 0) continue;
      const items = buildChatTranscriptItemsFromSteps(
        steps,
        `message-trace:${bundle.messageId}:${bundle.traceId}`,
        28,
        { complete: true },
      ).filter((item) => item.kind !== "prose");
      if (items.length > 0) {
        byMessageId[bundle.messageId] = items;
      }
    }
    return byMessageId;
  }, [buildChatTranscriptItemsFromSteps, messageRenderBundle, traceStepsById]);
  const completedRunTranscriptItems = useMemo(
    () =>
      buildChatTranscriptItemsFromSteps(
        completedLastRunSteps,
        "completed-run-transcript",
        8,
        { complete: true },
      ).filter((item) => item.kind === "action"),
    [buildChatTranscriptItemsFromSteps, completedLastRunSteps],
  );
  const latestWorkspaceCard = pickPrimaryActivityCard(workspaceCards);
  const progressRows = useMemo(() => {
    const seen = new Set<string>();
    const rows: Array<{
      label: string;
      status: "done" | "running" | "update";
      detail: string;
      time: string;
      tone: string;
    }> = [];
    for (const row of workspaceCards) {
      const key = row.label.trim().toLowerCase();
      if (!key || seen.has(key)) continue;
      seen.add(key);
      const status =
        row.kind === "Done"
          ? "done"
          : row.kind === "Running"
            ? "running"
            : "update";
      rows.push({
        label: row.label,
        status,
        detail: row.summary || row.detail || "",
        time: row.time || "",
        tone: row.tone,
      });
    }
    return rows.slice(-16);
  }, [workspaceCards]);

  const codeFromCards = useMemo(() => {
    for (let i = workspaceCards.length - 1; i >= 0; i -= 1) {
      const detail = str(
        workspaceCards[i]?.rawDetailFull,
        workspaceCards[i]?.detail || "",
      ).trim();
      const fenced = extractFirstCodeFence(detail);
      if (fenced) return fenced;
      if (
        detail.length > 80 &&
        /(import |const |function |class |=>|<div|SELECT |INSERT |CREATE )/i.test(
          detail,
        )
      ) {
        return detail;
      }
    }
    return "";
  }, [workspaceCards]);
  const codeSnapshot = useMemo(
    () =>
      codeFromCards ||
      extractFirstCodeFence(streamingResponse) ||
      extractFirstCodeFence(latestAssistantMessageText),
    [codeFromCards, latestAssistantMessageText, streamingResponse],
  );
  const activeCodeFile = deployedFiles[codeViewerFileIdx] ?? null;
  const activeSnippetFile = useMemo(() => {
    if (workspaceSnippetFiles.length === 0 && !selectedSnippetOverride) {
      return null;
    }
    if (selectedSnippetId) {
      return (
        (selectedSnippetOverride?.id === selectedSnippetId
          ? selectedSnippetOverride
          : null) ||
        workspaceSnippetFiles.find(
          (snippet) => snippet.id === selectedSnippetId,
        ) || null
      );
    }
    return deployedFiles.length === 0
      ? workspaceSnippetFiles[workspaceSnippetFiles.length - 1] ||
          selectedSnippetOverride ||
          null
      : null;
  }, [
    workspaceSnippetFiles,
    selectedSnippetId,
    selectedSnippetOverride,
    deployedFiles.length,
  ]);
  const activePhaseStatus =
    isStreamingForCurrentConversation || pendingSnapshotPhase === "running"
      ? (streamPhaseStatus ?? restoredPhaseStatus)
      : null;
  const currentRunStillActiveForMetrics =
    pendingSnapshotPhase === "running" &&
    (isStreamingForCurrentConversation ||
      liveStreamOpenForCurrentConversation ||
      hasFocusedDraftStream ||
      hasPendingSnapshotForConversation);
  const computerRunMetricItems = useMemo(
    () => (streamingRunMetrics ? buildChatRunMetricItems(streamingRunMetrics) : []),
    [streamingRunMetrics],
  );
  const streamingRunMetricItems =
    !currentRunStillActiveForMetrics ? computerRunMetricItems : [];
  const hasVisibleStreamingReply = Boolean(
    visibleStreamingResponse.trim() || visibleLiveModelEmit.trim(),
  );
  const showLiveExecutionPanel = liveChatTranscriptItems.length > 0;
  const resolvedActiveFileContent = choosePreferredWorkspaceFileContent(
    activeCodeFile ? liveFileWrites[activeCodeFile.name]?.content || "" : "",
    activeCodeFile?.content || "",
  );
  const codeViewerContent = activeCodeFile
    ? resolvedActiveFileContent ||
      `Preview unavailable for ${activeCodeFile.name} until file contents are captured.`
    : "";
  const activeWorkspaceCodeEntry = activeSnippetFile ?? activeCodeFile;
  const activeWorkspaceCodePath =
    activeSnippetFile?.displayName || activeCodeFile?.name || "";
  const activeWorkspaceCodeContent =
    activeSnippetFile?.content || codeViewerContent;
  const activeWorkspaceCodeSourceLabel = activeSnippetFile?.sourceLabel || "";
  const isShowingSnippetPreview = Boolean(activeSnippetFile);
  const activeWorkspaceCodeLines = useMemo(
    () =>
      renderCodeBlockLines(activeWorkspaceCodeContent || "", {
        fileName: activeWorkspaceCodePath,
      }),
    [activeWorkspaceCodeContent, activeWorkspaceCodePath],
  );
  const codeSnapshotLines = useMemo(
    () => renderCodeBlockLines(codeSnapshot),
    [codeSnapshot],
  );

  const appsWorkspaceQ = useQuery({
    queryKey: ["chat-workspace-apps"],
    queryFn: () => api.rawGet("/api/apps"),
    enabled: workspaceOpen,
    refetchInterval:
      workspaceOpen && autoRefresh && !activeRunUsesLiveStream
        ? REFRESH_MS
        : false,
  });
  const tunnelWorkspaceQ = useQuery({
    queryKey: ["chat-workspace-tunnel"],
    queryFn: () => api.rawGet("/tunnel/status"),
    enabled: workspaceOpen,
    refetchInterval:
      workspaceOpen && autoRefresh && !activeRunUsesLiveStream
        ? REFRESH_MS
        : false,
  });
  const workspaceApps = pickRecords(appsWorkspaceQ.data, "apps");
  const workspaceTunnel = asRecord(tunnelWorkspaceQ.data);
  const workspaceTunnelMeta = getTunnelAccessMeta(workspaceTunnel);
  const workspaceTunnelBaseUrl = str(workspaceTunnel.url, "")
    .trim()
    .replace(/\/+$/, "");
  const workspaceSelectedPublicAppId = str(
    workspaceTunnel.selected_app_id,
    "",
  ).trim();
  const workspaceExposedPublicAppIds = new Set(
    stringList(workspaceTunnel.exposed_app_ids),
  );
  if (workspaceSelectedPublicAppId) {
    workspaceExposedPublicAppIds.add(workspaceSelectedPublicAppId);
  }
  const activeWorkspaceApp = useMemo(() => {
    const workspaceAppSeed =
      streamedWorkspaceApp || restoredConversationWorkspaceApp;
    const hintedAppId = str(
      workspaceAppSeed?.id,
      str(workspaceAppSeed?.app_id, ""),
    ).trim();
    if (hintedAppId) {
      const matched = workspaceApps.find(
        (app) => str(app.id, "").trim() === hintedAppId,
      );
      if (matched) {
        return { ...matched, ...(workspaceAppSeed || {}) };
      }
    }
    if (workspaceAppSeed) {
      return workspaceAppSeed;
    }
    // No app deployed in this conversation - don't show stale preview from previous ones.
    return null;
  }, [workspaceApps, streamedWorkspaceApp, restoredConversationWorkspaceApp]);
  const activeWorkspaceAppId = str(
    activeWorkspaceApp?.id,
    str(activeWorkspaceApp?.app_id, ""),
  ).trim();
  const activeWorkspaceAppDir = str(activeWorkspaceApp?.app_dir, "").trim();
  const previewPath = str(
    activeWorkspaceApp?.local_access_url,
    str(
      activeWorkspaceApp?.access_url,
      str(activeWorkspaceApp?.local_url, str(activeWorkspaceApp?.url, "")),
    ),
  ).trim();
  const publicAccessPath = str(
    activeWorkspaceApp?.access_url,
    str(activeWorkspaceApp?.url, ""),
  ).trim();
  const previewUrl = toAbsoluteAppUrl(previewPath, origin);
  const previewImagePath = useMemo(() => {
    const streamImage = extractPreviewImageUrl(streamingResponse);
    if (streamImage) return streamImage;
    return extractPreviewImageUrl(latestAssistantMessageText);
  }, [streamingResponse, latestAssistantMessageText]);
  const previewImageUrl = toAbsoluteAppUrl(previewImagePath, origin);
  const publicPreviewUrl =
    workspaceTunnelBaseUrl &&
    workspaceTunnelBaseUrl !== origin &&
    activeWorkspaceAppId &&
    workspaceExposedPublicAppIds.has(activeWorkspaceAppId) &&
    publicAccessPath
      ? toAbsoluteAppUrl(publicAccessPath, workspaceTunnelBaseUrl)
      : "";
  const showWorkspacePanel = workspaceOpen;
  const showConversationSidebar = conversationSidebarOpen;
  const showWorkspacePanelInline =
    showWorkspacePanel && canInlineWorkspacePanel;
  const showConversationSidebarInline =
    showConversationSidebar && canInlineConversationSidebar;
  const showWorkspacePanelDrawer =
    showWorkspacePanel && !canInlineWorkspacePanel;
  const showConversationSidebarDrawer =
    showConversationSidebar && !canInlineConversationSidebar;
  const conversationListError = convQ.error;
  const messagesError = messagesQ.error;
  const messagesErrorText = errMessage(messagesError);
  const draftConversationMissing =
    !!conversationId &&
    !messages.length &&
    !sidebarConversationIds.has(conversationId) &&
    normalizeChatError(messagesErrorText).toLowerCase() ===
      "conversation not found";
  const visibleConversationListError = conversationListError;
  const visibleMessagesError = draftConversationMissing ? null : messagesError;
  const latestRunStatus = str(latestRunStatusSummary?.status, "")
    .trim()
    .toLowerCase();
  const chatCredentialPromptDismissed =
    !!conversationId &&
    dismissedCredentialPromptConversationIds.has(conversationId) &&
    !chatCredentialPromptVisible;
  const credentialActionNeeded =
    latestRunStatus === "needs_credentials" &&
    !chatCredentialPromptVisible &&
    !chatCredentialPromptDismissed;
  const credentialUiActive =
    chatCredentialPromptVisible || credentialActionNeeded;
  const searchIssueText = [
    chatError || "",
    executionPlanFailure || "",
    latestRunStatusSummary?.detail || "",
  ]
    .filter(Boolean)
    .join("\n");
  const searchSetupActionNeeded = isSearchBackendSetupIssue(searchIssueText);
  const visibleConversationError =
    visibleConversationListError ||
    visibleMessagesError ||
    (chatError && !credentialUiActive);
  const suggestedSecretKey =
    str(chatCredentialPromptFields[0]?.key, "").trim().toUpperCase() ||
    secretHelperKey ||
    "OPENAI_API_KEY";
  const latestRunningCard = useMemo(
    () =>
      [...workspaceCards]
        .reverse()
        .find((row) => row.kind === "Running" || row.kind === "Planning") ||
      null,
    [workspaceCards],
  );
  const latestCompletedCard = useMemo(
    () =>
      [...workspaceCards].reverse().find((row) => row.kind === "Done") || null,
    [workspaceCards],
  );
  const currentStatusCard = useMemo(
    () => latestRunningCard || latestWorkspaceCard || null,
    [latestRunningCard, latestWorkspaceCard],
  );
  const currentWorkspaceIssue = currentStatusCard?.kind === "Issue";
  const safetyPolicyBlocked =
    currentWorkspaceIssue &&
    isSafetyPolicyBlockedText(
      `${currentStatusCard?.label || ""} ${currentStatusCard?.detail || ""} ${currentStatusCard?.detailFull || ""}`,
    );
  const hasCompletedWorkspaceRun =
    !showStreamingAssistant &&
    !currentWorkspaceIssue &&
    workspaceCards.length > 0 &&
    latestWorkspaceCard?.kind === "Done";
  const activityUpdateCountLabel = `${workspaceCards.length} update${workspaceCards.length === 1 ? "" : "s"}`;
  const consoleEventCountLabel = `${workspaceConsoleCards.length} event${workspaceConsoleCards.length === 1 ? "" : "s"}`;
  const progressSummary = !progressRows.length
    ? "No activity yet"
    : showStreamingAssistant
      ? activityUpdateCountLabel
      : hasCompletedWorkspaceRun
        ? `Run completed - ${activityUpdateCountLabel}`
        : activityUpdateCountLabel;
  const consoleProgressSummary = workspaceConsoleCards.length
    ? consoleEventCountLabel
    : progressSummary;
  const activityDetailPayloadKey = activityDetailRow
    ? `activity-detail:${activityDetailRow.id}`
    : "";
  const activityDetailReadableDetail =
    activityDetailRow?.rawDetailFull &&
    looksLikeStructuredActivityText(activityDetailRow.rawDetailFull)
      ? summarizeActivityDetail(activityDetailRow.rawDetailFull)
      : "";
  const executionPlanCompletedCount = displayedExecutionPlan.filter(
    (step) => step.status === "completed",
  ).length;
  const executionPlanActiveCount = displayedExecutionPlan.filter(
    (step) => step.status === "running",
  ).length;
  const executionPlanFailedCount = displayedExecutionPlan.filter(
    (step) => step.status === "failed",
  ).length;
  const executionPlanPendingCount = Math.max(
    0,
    displayedExecutionPlan.length -
      executionPlanCompletedCount -
      executionPlanActiveCount -
      executionPlanFailedCount,
  );
  const executionPlanNeedsAttention = credentialUiActive || safetyPolicyBlocked;
  const hasLiveSwarmRun = swarmActivityRuns.some((run) =>
    ["assigned", "running", "synthesizing"].includes(
      normalizeSwarmStatus(run.status),
    ),
  );
  const isExecutionPlanFinalizing =
    displayedExecutionPlan.length > 0 &&
    executionPlanCompletedCount === displayedExecutionPlan.length &&
    (showStreamingAssistant || hasPendingSnapshotForConversation) &&
    !finalMessageLanded;
  const isExecutionPlanTransitioning =
    displayedExecutionPlan.length > 0 &&
    executionPlanCompletedCount > 0 &&
    executionPlanPendingCount > 0 &&
    executionPlanActiveCount === 0 &&
    (showStreamingAssistant || hasPendingSnapshotForConversation) &&
    !isExecutionPlanFinalizing &&
    !hasLiveSwarmRun;
  const workspaceStatusCopy = useMemo(() => {
    if (credentialUiActive) {
      return {
        line1: "Status: Waiting for secure input",
        line2:
          str(chatCredentialPrompt.title, "").trim() ||
          "A secure credential is needed before this run can continue.",
        tone: "warning",
      };
    }
    if (safetyPolicyBlocked) {
      return {
        line1: "Status: Blocked by safety policy",
        line2:
          "The agent tried a disallowed tool and needs a different approach.",
        tone: "warning",
      };
    }
    if (isExecutionPlanFinalizing) {
      return {
        line1: "Status: Finalizing answer",
        line2:
          "All approved research steps are complete. AgentArk is composing the final response.",
        tone: "info",
      };
    }
    if (isExecutionPlanTransitioning) {
      return {
        line1: "Status: Starting next branch",
        line2: `${executionPlanCompletedCount} of ${displayedExecutionPlan.length} plan steps are complete. Waiting for the next branch to start.`,
        tone: "info",
      };
    }
    if (isStreamingForCurrentConversation) {
      const active = latestRunningCard || latestWorkspaceCard;
      const liveWriteEntry =
        Object.entries(liveFileWrites).find(([, state]) => !state.done) ||
        null;
      if (liveWriteEntry) {
        const [fileName, state] = liveWriteEntry;
        const line =
          state.totalLines > 0
            ? `line ${Math.min(state.line, state.totalLines)} of ${state.totalLines}`
            : "capturing generated code";
        return {
          line1: `Status: Writing ${fileName}`,
          line2: line,
          tone: "info",
        };
      }
      return {
        line1: `Status: ${activePhaseStatus?.label || "Running"}`,
        line2:
          activePhaseStatus?.detail ||
          active?.detail ||
          "Agent is actively running actions.",
        tone: "info",
      };
    }
    if (latestWorkspaceCard?.kind === "Done") {
      return {
        line1: "Status: Completed",
        line2: latestWorkspaceCard.detail || latestWorkspaceCard.label,
        tone: "default",
      };
    }
    if (latestWorkspaceCard) {
      return {
        line1: "Status: Stopped",
        line2: latestWorkspaceCard.detail || latestWorkspaceCard.label,
        tone: "default",
      };
    }
    return {
      line1: "Status: Stopped",
      line2: "Send a request to start a run.",
      tone: "default",
    };
  }, [
    activePhaseStatus?.detail,
    activePhaseStatus?.label,
    chatCredentialPrompt.title,
    credentialUiActive,
    displayedExecutionPlan.length,
    executionPlanCompletedCount,
    isExecutionPlanFinalizing,
    isExecutionPlanTransitioning,
    isStreamingForCurrentConversation,
    latestRunningCard,
    latestWorkspaceCard,
    liveFileWrites,
    safetyPolicyBlocked,
  ]);
  const nowDoingLabel = useMemo(() => {
    if (credentialUiActive) return "Waiting for secure input";
    if (safetyPolicyBlocked) return "Blocked by safety policy";
    if (isExecutionPlanFinalizing) return "Finalizing answer";
    if (isExecutionPlanTransitioning) return "Starting next branch";
    if (isStreamingForCurrentConversation) {
      const liveWriteEntry =
        Object.entries(liveFileWrites).find(([, state]) => !state.done) ||
        null;
      if (liveWriteEntry?.[0]) return `Writing ${liveWriteEntry[0]}`;
    }
    if (activePhaseStatus?.label) return activePhaseStatus.label;
    const active = latestRunningCard || latestWorkspaceCard;
    return active?.label || "Waiting for next step";
  }, [
    activePhaseStatus?.label,
    credentialUiActive,
    isExecutionPlanFinalizing,
    isExecutionPlanTransitioning,
    isStreamingForCurrentConversation,
    latestRunningCard,
    latestWorkspaceCard,
    liveFileWrites,
    safetyPolicyBlocked,
  ]);
  const liveWriteEntries = useMemo(
    () =>
      Object.entries(liveFileWrites).sort((a, b) => {
        const aDone = a[1].done ? 1 : 0;
        const bDone = b[1].done ? 1 : 0;
        return aDone - bDone;
      }),
    [liveFileWrites],
  );
  const activeLiveWriteEntry = useMemo(
    () =>
      liveWriteEntries.find(([, state]) => !state.done) ||
      liveWriteEntries[0] ||
      null,
    [liveWriteEntries],
  );
  const computerWorkspaceFiles = useMemo(() => {
    // Build the union of files the agent has fully deployed AND files it is
    // currently streaming. The first deploy of a turn has empty deployedFiles
    // until the action returns, so without folding live-writes in here the
    // Computer pane would render "Files: 0" while the model is mid-draft â€”
    // exactly the Lovable/Bolt hole the user flagged. Live-write content takes
    // precedence over any captured snapshot for the same path.
    const merged = new Map<
      string,
      { path: string; displayPath?: string; content: string }
    >();
    for (const file of deployedFiles) {
      merged.set(file.name, {
        path: file.name,
        displayPath: workspaceFileDisplayPath(
          file.name,
          activeWorkspaceAppId,
          activeWorkspaceAppDir,
        ),
        content: choosePreferredWorkspaceFileContent(
          liveFileWrites[file.name]?.content || "",
          file.content || "",
        ),
      });
    }
    for (const [name, state] of Object.entries(liveFileWrites)) {
      if (merged.has(name)) continue;
      const content = choosePreferredWorkspaceFileContent("", state.content || "");
      if (state.done && !content) continue;
      merged.set(name, {
        path: name,
        displayPath: workspaceFileDisplayPath(
          name,
          activeWorkspaceAppId,
          activeWorkspaceAppDir,
        ),
        content,
      });
    }
    return Array.from(merged.values());
  }, [activeWorkspaceAppDir, activeWorkspaceAppId, deployedFiles, liveFileWrites]);
  const executionPlanStatusLabel = executionPlanNeedsAttention
    ? "Needs attention"
    : visibleExecutionPlanFailure
      ? "Planner offline"
      : isExecutionPlanFinalizing
        ? "Finalizing"
        : isExecutionPlanTransitioning
          ? "Starting next branch"
          : executionPlanActiveCount > 0
            ? "Working"
            : displayedExecutionPlan.length > 0 &&
                executionPlanCompletedCount === displayedExecutionPlan.length
              ? "Completed"
              : executionPlanFailedCount > 0 && executionPlanCompletedCount > 0
                ? "Completed"
                : "Ready";
  const executionPlanSummaryText =
    displayedExecutionPlan.length > 0
      ? [
          displayedExecutionPlanSummary ||
            `${displayedExecutionPlan.length} step${displayedExecutionPlan.length === 1 ? "" : "s"}`,
          `${executionPlanCompletedCount} done`,
          isExecutionPlanFinalizing ? "final answer in progress" : null,
          isExecutionPlanTransitioning ? "next branch starting" : null,
          executionPlanActiveCount > 0
            ? `${executionPlanActiveCount} running`
            : null,
          executionPlanPendingCount > 0
            ? `${executionPlanPendingCount} pending`
            : null,
        ]
          .filter(Boolean)
          .join(" - ")
      : visibleExecutionPlanFailure;
  const executionPlanTone = executionPlanNeedsAttention
    ? "failed"
    : visibleExecutionPlanFailure
      ? "failed"
      : isExecutionPlanFinalizing || isExecutionPlanTransitioning
        ? "running"
        : executionPlanActiveCount > 0
          ? "running"
          : displayedExecutionPlan.length > 0 &&
              executionPlanCompletedCount === displayedExecutionPlan.length
            ? "done"
            : "pending";
  const restoredDeepResearchStage =
    useMemo<PlanConfirmationStage | null>(() => {
      if (
        !hasDeepResearchPlanContext ||
        !displayedExecutionPlanState ||
        displayedExecutionPlan.length === 0
      ) {
        return null;
      }
      const normalizedRunStatus = str(latestRunStatusSummary?.status, "")
        .trim()
        .toLowerCase();
      if (pendingSnapshotPhase === "awaiting_confirmation")
        return "awaiting_confirmation";
      if (
        normalizedRunStatus === "interrupted" ||
        normalizedRunStatus === "cancelled" ||
        normalizedRunStatus === "canceled"
      ) {
        return "interrupted";
      }
      if (
        visibleExecutionPlanFailure ||
        normalizedRunStatus === "platform_failed" ||
        normalizedRunStatus === "failed" ||
        executionPlanFailedCount > 0
      ) {
        return "failed";
      }
      if (executionPlanActiveCount > 0) return "running";
      if (
        displayedExecutionPlan.length > 0 &&
        executionPlanCompletedCount === displayedExecutionPlan.length
      ) {
        return "completed";
      }
      if (executionPlanPendingCount === displayedExecutionPlan.length) {
        return "awaiting_confirmation";
      }
      return null;
    }, [
      displayedExecutionPlan,
      visibleExecutionPlanFailure,
      displayedExecutionPlanState,
      executionPlanActiveCount,
      executionPlanCompletedCount,
      executionPlanFailedCount,
      executionPlanPendingCount,
      hasDeepResearchPlanContext,
      latestRunStatusSummary?.status,
      pendingSnapshotPhase,
    ]);
  const shouldPreferDeepResearchPlanCard =
    hasDeepResearchPlanContext && displayedExecutionPlan.length > 0;
  const shouldSuppressCompactExecutionPlan =
    (hasDeepResearchPlanContext &&
      hasPendingSnapshotForConversation &&
      ["running", "awaiting_confirmation", "interrupted"].includes(
        pendingSnapshotPhase,
      ) &&
      displayedExecutionPlan.length > 0) ||
    shouldPreferDeepResearchPlanCard;
  const shouldShowCompactExecutionPlan =
    hasDeepResearchPlanContext &&
    !showPlanConfirmationCard &&
    displayedExecutionPlan.length > 0 &&
    !shouldSuppressCompactExecutionPlan;
  const shouldShowExecutionPlanWarning =
    hasDeepResearchPlanContext &&
    !showPlanConfirmationCard &&
    !!visibleExecutionPlanFailure;

  useEffect(() => {
    if (
      !restoredDeepResearchStage ||
      !displayedExecutionPlanState ||
      !hasDeepResearchPlanContext
    ) {
      return;
    }
    const anchoredMessageId =
      str(deepResearchPlanPreviewMessageId, "").trim() || null;
    const needsRepair =
      !planConfirmation ||
      planConfirmation.stage !== restoredDeepResearchStage ||
      !planConfirmation.originalPlan ||
      !planConfirmation.draft ||
      (anchoredMessageId &&
        str(planConfirmation.messageId, "").trim() !== anchoredMessageId);
    if (!needsRepair) return;
    setPlanConfirmation((prev) => {
      const nextOriginalPlan =
        prev?.originalPlan ?? displayedExecutionPlanState;
      const restoredSource = isDeepResearchPlanSource(prev?.source)
        ? str(prev?.source, PLAN_CONFIRMATION_SOURCE_DEEP_RESEARCH)
        : isDeepResearchPlanSource(workspacePlanConfirmationSource)
          ? workspacePlanConfirmationSource
          : PLAN_CONFIRMATION_SOURCE_DEEP_RESEARCH;
      return {
        stage: restoredDeepResearchStage,
        taskId: str(prev?.taskId, "").trim() || null,
        source: restoredSource,
        originalPlan: nextOriginalPlan,
        draft: createPlanConfirmationDraft(nextOriginalPlan),
        editing: false,
        messageId: anchoredMessageId || prev?.messageId || null,
      };
    });
  }, [
    deepResearchPlanPreviewMessageId,
    displayedExecutionPlanState,
    hasDeepResearchPlanContext,
    planConfirmation,
    restoredDeepResearchStage,
    workspacePlanConfirmationSource,
  ]);

  const renderExecutionPlanStatusIcon = (status: string, className = "") => {
    if (status === "completed") {
      return <CheckCircleRoundedIcon className={className} />;
    }
    if (status === "failed") {
      return <ErrorOutlineRoundedIcon className={className} />;
    }
    if (status === "running") {
      return <AutorenewRoundedIcon className={className} />;
    }
    return <RadioButtonUncheckedRoundedIcon className={className} />;
  };
  const executionPlanStepStatusLabel = (status: string) => {
    const normalized = str(status, "").trim().toLowerCase();
    if (normalized === "completed") return "Done";
    if (normalized === "running") return "Running";
    if (normalized === "failed") return "Failed";
    if (normalized === "skipped") return "Skipped";
    return "Pending";
  };

  const renderConversationSidebarContent = (drawer = false) => (
    <Box
      className="list-shell chat-sidebar"
      sx={{
        minHeight: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        maxHeight: drawer ? "none" : { xs: 260, lg: "none" },
      }}
    >
      <Stack
        direction="row"
        sx={{
          justifyContent: "space-between",
          alignItems: "center",
          mb: 1.5,
        }}
      >
        <Typography variant="h6">Conversations</Typography>
        <Stack
          direction="row"
          spacing={0.75}
          sx={{
            alignItems: "center",
          }}
        >
          <Button
            size="small"
            variant="outlined"
            className="chat-toolbar-btn"
            onClick={() => startNewConversation()}
          >
            New chat
          </Button>
          {drawer ? (
            <IconButton
              size="small"
              onClick={() => setConversationSidebarOpen(false)}
            >
              <CloseIcon fontSize="small" />
            </IconButton>
          ) : null}
        </Stack>
      </Stack>

      <Box sx={{ flex: 1, minHeight: 0, overflow: "auto", pr: 0.5 }}>
        <Stack spacing={0.9} className="conversation-list">
          {starredConversations.length === 0 && conversations.length === 0 ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              No conversations yet.
            </Typography>
          ) : (
            <>
              {starredConversations.length > 0 ? (
                <Box className="conversation-group">
                  <Typography
                    variant="caption"
                    className="conversation-group-label"
                    sx={{ px: 0.25 }}
                  >
                    Starred
                  </Typography>
                  <Stack spacing={0.4}>
                    {starredConversations.map((conv) =>
                      renderConversationCard(conv),
                    )}
                  </Stack>
                </Box>
              ) : null}
              {conversations.length > 0 ? (
                <Box className="conversation-group">
                  <Typography
                    variant="caption"
                    className="conversation-group-label"
                    sx={{ px: 0.25 }}
                  >
                    Recents
                  </Typography>
                  <Stack spacing={0.4}>
                    {conversations.map((conv) => renderConversationCard(conv))}
                  </Stack>
                </Box>
              ) : null}
            </>
          )}
        </Stack>
      </Box>
      <Stack
        direction="row"
        spacing={0.5}
        sx={{
          alignItems: "center",
          justifyContent: "space-between",
          mt: 0.75,
          px: 0.25,
        }}
      >
        <Typography variant="caption" className="conversation-pagination-copy">
          {conversationListTotal} chat{conversationListTotal === 1 ? "" : "s"}
        </Typography>
        <Stack
          direction="row"
          spacing={0.5}
          sx={{
            alignItems: "center",
          }}
        >
          <IconButton
            size="small"
            onClick={() => setConversationPage((prev) => Math.max(0, prev - 1))}
            disabled={conversationPage <= 0}
            sx={{ p: 0.35 }}
          >
            <ChevronLeftRoundedIcon sx={{ fontSize: 16 }} />
          </IconButton>
          <Typography variant="caption" className="conversation-page-indicator">
            {conversationPageLabel}
          </Typography>
          <IconButton
            size="small"
            onClick={() =>
              setConversationPage((prev) =>
                Math.min(Math.max(0, conversationPageCount - 1), prev + 1),
              )
            }
            disabled={conversationPage >= conversationPageCount - 1}
            sx={{ p: 0.35 }}
          >
            <ChevronRightRoundedIcon sx={{ fontSize: 16 }} />
          </IconButton>
        </Stack>
      </Stack>
      <Menu
        anchorEl={conversationMenuAnchor}
        open={Boolean(conversationMenuAnchor)}
        onClose={closeConversationMenu}
      >
        <MenuItem
          disabled={toggleConversationStarMutation.isPending}
          onClick={() => {
            const id = str(conversationMenuTarget?.id, "");
            const starred = toBool(conversationMenuTarget?.starred);
            closeConversationMenu();
            if (id) void toggleConversationStar(id, !starred);
          }}
        >
          {toBool(conversationMenuTarget?.starred)
            ? "Unstar chat"
            : "Star chat"}
        </MenuItem>
        <MenuItem
          onClick={() => {
            const id = str(conversationMenuTarget?.id, "");
            const title = str(conversationMenuTarget?.title, "chat");
            closeConversationMenu();
            if (id) void exportConversationById(id, title);
          }}
        >
          Export chat
        </MenuItem>
        <MenuItem
          disabled={isStreaming || deleteConversationMutation.isPending}
          onClick={() => {
            const id = str(conversationMenuTarget?.id, "");
            closeConversationMenu();
            if (id) void deleteConversation(id);
          }}
        >
          Delete chat
        </MenuItem>
      </Menu>
    </Box>
  );

  const renderComputerPaneContent = (_drawer = false) => (
    <Box
      className="computer-pane-shell"
      sx={{
        minHeight: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignSelf: "start",
      }}
    >
      <ComputerPane
        liveCards={inlineWorkspaceCards}
        allCards={workspaceConsoleCards}
        activeStepId={activeStepId}
        onActivate={handleActivateStep}
        onClose={closeWorkspacePanel}
        nowDoingLabel={nowDoingLabel || streamingActivity}
        snippetPath={activeWorkspaceCodePath}
        snippetContent={activeWorkspaceCodeContent}
        isStreaming={isStreamingForCurrentConversation}
        startedAt={pendingSnapshotStartedAt || null}
        tokenPreview={deferredComputerTokenPreview}
        runMetrics={computerRunMetricItems}
        reasoningPreview={reasoningStream?.content || ""}
        reasoningPhase={reasoningStream?.phase || ""}
        taskProgress={computerTaskProgress}
        showSnippet={Boolean(selectedSnippetId) && !activeStepId}
        workspaceFiles={computerWorkspaceFiles}
        liveWritePath={activeLiveWriteEntry?.[0] || null}
        liveWriteContent={activeLiveWriteEntry?.[1]?.content || ""}
        liveWriteActive={
          Boolean(activeLiveWriteEntry) && !activeLiveWriteEntry?.[1]?.done
        }
      />
    </Box>
  );

  const renderActivityPanelContent = (_drawer = false) => (
    <Box
      className="list-shell chat-workspace-shell"
      sx={{
        minHeight: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        p: 0.7,
        alignSelf: "start",
      }}
    >
      <Stack
        direction="row"
        className="chat-console-toolbar"
        sx={{
          justifyContent: "space-between",
          alignItems: "center",
          px: 0.5,
          pt: 0.25,
          pb: 0.75,
        }}
      >
        <Typography variant="subtitle2">AgentArk Console</Typography>
        <Tooltip title="Close console">
          <IconButton
            size="small"
            aria-label="Close AgentArk Console"
            onClick={() => {
              workspaceUserClosedRef.current = true;
              setWorkspaceOpen(false);
            }}
          >
            <CloseIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>
      <Box
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", overflowX: "hidden" }}
        className="chat-workspace-sections"
      >
        <Box className="chat-activity-intro">
          <Typography variant="caption" className="chat-activity-intro-kicker">
            Live console
          </Typography>
          <Typography variant="subtitle2" className="chat-activity-intro-title">
            AgentArk Console
          </Typography>
          <Typography variant="caption" className="chat-activity-intro-copy">
            Current focus: {nowDoingLabel}. Tool output and runtime activity stay here.
          </Typography>
        </Box>
        <SwarmActivityPanel
          runs={swarmActivityRuns}
          interrupted={showInterruptedRunCard}
          expandedPayloads={expandedActivityPayloads}
          onTogglePayload={toggleExpandedActivityPayload}
        />
        <Box className="term-shell">
          <Box className="term-titlebar">
            <Typography variant="caption" className="term-titlebar-text">
              Activity
            </Typography>
            <Box sx={{ flex: 1 }} />
            <Typography variant="caption" className="term-titlebar-stats">
              {consoleProgressSummary}
            </Typography>
          </Box>
          <Box
            className="term-body"
            ref={workspaceActivityRef}
            onScroll={() => {
              const node = workspaceActivityRef.current;
              if (!node) return;
              const nearBottom =
                node.scrollHeight - node.scrollTop - node.clientHeight < 22;
              if (nearBottom && !activityAutoFollow)
                setActivityAutoFollow(true);
              if (!nearBottom && activityAutoFollow)
                setActivityAutoFollow(false);
            }}
          >
            {workspaceConsoleCards.length === 0 ? (
              <Box className="term-empty-state">
                <Typography variant="overline" className="term-empty-kicker">
                  Quiet for now
                </Typography>
                <Typography variant="body2" className="term-empty-copy">
                  Activity updates appear here when AgentArk starts a run,
                  emits a preview, or records a runtime step.
                </Typography>
              </Box>
            ) : (
              workspaceConsoleCards.map((row, idx) => {
                const isLast = idx === workspaceConsoleCards.length - 1;
                const isActive = isLast && isStreamingForCurrentConversation;
                return (
                  <ActivityTimelineRow
                    key={`activity-${row.id}`}
                    row={row}
                    isActive={isActive}
                    onOpenDetails={() => setActivityDetailRow(row)}
                    detailed
                  />
                );
              })
            )}
          </Box>
        </Box>
        {workspaceSnippetFiles.length > 0 ? (
          <Accordion className="chat-workspace-section" disableGutters>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              sx={{ minHeight: 34 }}
            >
              <Typography variant="subtitle2">
                Snippets ({workspaceSnippetFiles.length})
              </Typography>
            </AccordionSummary>
            <AccordionDetails sx={{ p: "4px 8px 8px" }}>
              <Stack spacing={0.5}>
                {workspaceSnippetFiles.map((snippet) => (
                  <Box
                    key={snippet.id}
                    className={`deployed-file-row${activeSnippetFile?.id === snippet.id ? " is-selected" : ""}`}
                    onClick={() => {
                      setWorkspaceOpen(true);
                      setSelectedSnippetId(snippet.id);
                    }}
                  >
                    <span className="deployed-file-icon">&lt;/&gt;</span>
                    <span
                      className="deployed-file-name"
                      title={snippet.displayName}
                    >
                      {snippet.displayName}
                    </span>
                    <span className="deployed-file-size">
                      {snippet.sourceLabel}
                    </span>
                  </Box>
                ))}
              </Stack>
            </AccordionDetails>
          </Accordion>
        ) : null}

        {isShowingSnippetPreview && activeWorkspaceCodeEntry ? (
          <Accordion
            className="chat-workspace-section"
            disableGutters
            defaultExpanded
          >
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              sx={{ minHeight: 34 }}
            >
              <Stack
                direction="row"
                spacing={1}
                className="chat-workspace-code-summary"
                sx={{
                  alignItems: "center",
                }}
              >
                <Stack
                  direction="row"
                  spacing={1}
                  className="chat-workspace-code-heading"
                  sx={{
                    alignItems: "center",
                  }}
                >
                  <Typography variant="subtitle2">Snippet preview</Typography>
                  <Typography
                    variant="caption"
                    className="chat-workspace-code-path"
                    title={activeWorkspaceCodePath}
                  >
                    {activeWorkspaceCodePath}
                  </Typography>
                </Stack>
                <Box sx={{ flex: 1, minWidth: 0 }} />
                <Stack
                  direction="row"
                  spacing={1}
                  className="chat-workspace-code-actions"
                  sx={{
                    alignItems: "center",
                  }}
                >
                  <Typography
                    variant="caption"
                    className="chat-workspace-code-meta"
                    title={activeWorkspaceCodeSourceLabel}
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    {activeWorkspaceCodeSourceLabel}
                  </Typography>
                  <Button
                    size="small"
                    variant="text"
                    className="chat-workspace-code-open"
                    onClick={(event) => {
                      event.stopPropagation();
                      setCodeViewerOpen(true);
                    }}
                  >
                    Open full screen
                  </Button>
                </Stack>
              </Stack>
            </AccordionSummary>
            <AccordionDetails>
              {workspaceSnippetFiles.length > 1 ? (
                <Box className="code-file-tabs chat-workspace-file-tabs">
                  {workspaceSnippetFiles.map((snippet) => (
                    <button
                      key={snippet.id}
                      className={`code-file-tab${activeSnippetFile?.id === snippet.id ? " code-file-tab-active" : ""}`}
                      onClick={() => setSelectedSnippetId(snippet.id)}
                    >
                      {snippet.displayName}
                    </button>
                  ))}
                </Box>
              ) : null}
              <pre className="code-viewer-pre chat-workspace-code-inline">
                <code>{activeWorkspaceCodeLines}</code>
              </pre>
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                  display: "block",
                  mt: 0.75,
                }}
              >
                Referenced from {activeWorkspaceCodeSourceLabel}.
              </Typography>
            </AccordionDetails>
          </Accordion>
        ) : codeSnapshot ? (
          <Accordion className="chat-workspace-section" disableGutters>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              sx={{ minHeight: 34 }}
            >
              <Typography variant="subtitle2">Code</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <pre className="code-viewer-pre chat-workspace-pre">
                <code>{codeSnapshotLines}</code>
              </pre>
            </AccordionDetails>
          </Accordion>
        ) : null}

        {previewUrl ? (
          <Accordion
            className="chat-workspace-section chat-workspace-section-preview"
            disableGutters
          >
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              sx={{ minHeight: 34 }}
            >
              <Typography variant="subtitle2">Preview</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <Box className="chat-workspace-preview">
                <Typography
                  variant="caption"
                  noWrap
                  title={previewUrl}
                  sx={{
                    color: "text.secondary",
                    display: "block",
                    mb: 0.7,
                  }}
                >
                  Local:{" "}
                  <Link
                    href={previewUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    underline="hover"
                  >
                    {previewUrl}
                  </Link>
                </Typography>
                {publicPreviewUrl ? (
                  <Typography
                    variant="caption"
                    noWrap
                    title={publicPreviewUrl}
                    sx={{
                      color: "info.main",
                      display: "block",
                      mb: 0.7,
                    }}
                  >
                    {workspaceTunnelMeta.isPrivate
                      ? "Private access:"
                      : "Public:"}{" "}
                    <Link
                      href={publicPreviewUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      underline="hover"
                    >
                      {publicPreviewUrl}
                    </Link>
                  </Typography>
                ) : null}
                <Stack direction="row" spacing={0.8} sx={{ mt: 0.7 }}>
                  <Button
                    size="small"
                    variant="outlined"
                    onClick={() =>
                      window.open(previewUrl, "_blank", "noopener,noreferrer")
                    }
                  >
                    Open live app
                  </Button>
                  <Button
                    size="small"
                    variant="contained"
                    onClick={() => setPreviewDialogOpen(true)}
                    disabled={!previewImageUrl}
                  >
                    Open preview popup
                  </Button>
                </Stack>
                {!previewImageUrl ? (
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      display: "block",
                      mt: 0.7,
                    }}
                  >
                    Screenshot preview will appear after deployment validation
                    captures it.
                  </Typography>
                ) : null}
              </Box>
            </AccordionDetails>
          </Accordion>
        ) : null}
      </Box>
    </Box>
  );

  const submitSecretHelper = async (modeOverride?: "reuse" | "manual") => {
    if (secretHelperBusy || isStreaming) return;
    const key = (secretHelperKey || "").trim().toUpperCase();
    const mode = modeOverride || secretHelperMode;
    if (!key) {
      setChatError(
        "Enter which key name to set first (example: OPENAI_API_KEY).",
      );
      return;
    }
    setSecretHelperBusy(true);
    setChatError(null);
    setChatNotice(null);
    try {
      if (mode === "reuse") {
        const payload = asRecord(
          await api.rawPost(
            "/chat/credential/raw-secret/reuse-model-credential",
            {
              conversation_id: conversationId,
              key,
            },
          ),
        );
        const followup = str(payload.followup, "").trim();
        setChatNotice(followup || `${key} saved securely.`);
        await queryClient.invalidateQueries({
          queryKey: ["chat-credential-prompt", conversationId],
        });
        await queryClient.invalidateQueries({
          queryKey: ["chat-messages", conversationId],
        });
        await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
        return;
      }
      if (!secretHelperValue.trim()) {
        setChatError("Enter the key value first.");
        return;
      }
      const payload = asRecord(
        await api.rawPost("/chat/credential/raw-secret/submit", {
          conversation_id: conversationId,
          key,
          value: secretHelperValue,
        }),
      );
      const followup = str(payload.followup, "").trim();
      setSecretHelperValue("");
      setChatNotice(followup || `${key} saved securely.`);
      await queryClient.invalidateQueries({
        queryKey: ["chat-credential-prompt", conversationId],
      });
      await queryClient.invalidateQueries({
        queryKey: ["chat-messages", conversationId],
      });
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
    } catch (error) {
      setChatError(normalizeChatError(errMessage(error)));
    } finally {
      setSecretHelperBusy(false);
    }
  };

  const submitChatCredentialPrompt = async () => {
    if (
      !conversationId ||
      !chatCredentialPromptVisible ||
      submitChatCredentialPromptMutation.isPending
    ) {
      return;
    }
    const values: Record<string, string> = {};
    const missingLabels: string[] = [];
    for (const field of chatCredentialPromptFields) {
      const key = str(field.key, "").trim();
      if (!key) continue;
      const value = (chatCredentialValues[key] || "").trim();
      const label = str(field.label, key).trim() || key;
      if (!value) {
        if (toBool(field.required)) {
          missingLabels.push(label);
        }
        continue;
      }
      values[key] = value;
    }
    if (missingLabels.length > 0) {
      setChatCredentialError(
        `Enter the required value${missingLabels.length > 1 ? "s" : ""}: ${missingLabels.join(", ")}`,
      );
      return;
    }
    if (Object.keys(values).length === 0) {
      setChatCredentialError("Enter at least one credential value.");
      return;
    }
    setChatCredentialError(null);
    setChatError(null);
    await submitChatCredentialPromptMutation.mutateAsync(values);
  };

  useEffect(() => {
    if (!credentialUiActive) return;
    if (!secretHelperKey || secretHelperKey === "OPENAI_API_KEY") {
      setSecretHelperKey(suggestedSecretKey);
    }
  }, [credentialUiActive, suggestedSecretKey, secretHelperKey]);

  useEffect(() => {
    if (!activityAutoFollow) return;
    const node = workspaceActivityRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
  }, [workspaceConsoleCards.length, activityAutoFollow, isStreaming]);

  const composerPlaceholder =
    credentialUiActive
      ? "Use the secure credential form above"
      : composerAwaitingPlanConfirmation
        ? "Ask for changes to the plan, or press Start."
        : "Message (Enter to send, Shift+Enter for newline)";

  const emptyComposerPlaceholder =
    credentialUiActive || composerAwaitingPlanConfirmation
      ? composerPlaceholder
      : "How can I help you today?";

  const pendingTurnAttachments = useMemo(() => {
    const fromSnapshot = sanitizeChatTurnAttachments(
      pendingRunSnapshot?.attachments,
    );
    if (fromSnapshot.length > 0) return fromSnapshot;
    return isStreaming ? chatTurnAttachmentsFromFiles(attachedFiles) : [];
  }, [attachedFiles, isStreaming, pendingRunSnapshot?.attachments]);

  const renderAttachedFilePills = (className = "") =>
    !isStreaming && attachedFiles.length > 0 ? (
      <Box className={`chat-attached-file-pills ${className}`.trim()}>
        {attachedFiles.map((file, idx) => (
          <span
            key={`${file.name}-${file.size}-${file.lastModified}-${idx}`}
            className="chat-attached-file-pill"
          >
            <AttachFileRoundedIcon fontSize="inherit" aria-hidden="true" />
            <span className="chat-attached-file-pill-label">{file.name}</span>
            <button
              type="button"
              className="chat-attached-file-pill-remove"
              aria-label={`Remove ${file.name}`}
              onClick={() => removeAttachedFile(idx)}
            >
              <CloseIcon fontSize="inherit" />
            </button>
          </span>
        ))}
      </Box>
    ) : null;

  const renderTurnAttachmentPills = (
    attachments: ChatTurnAttachment[],
    keyPrefix: string,
  ) =>
    attachments.length > 0 ? (
      <Box className="chat-turn-attachments" aria-label="Attached files">
        {attachments.map((attachment, idx) => {
          const detail =
            attachment.kind === "document"
              ? "Indexed document"
              : attachment.kind === "visual"
                ? "Attached visual"
                : "Attached file";
          return (
            <Tooltip
              key={`${keyPrefix}:${attachment.kind}:${attachment.id || attachment.name}:${idx}`}
              title={attachment.detail ? `${detail} - ${attachment.detail}` : detail}
            >
              <span className="chat-turn-attachment-pill">
                <AttachFileRoundedIcon fontSize="inherit" aria-hidden="true" />
                <span className="chat-turn-attachment-pill-label">
                  {attachment.name}
                </span>
              </span>
            </Tooltip>
          );
        })}
      </Box>
    ) : null;

  const renderRunMetricPills = (
    items: ChatRunMetricItem[],
    keyPrefix: string,
    ariaLabel: string,
    extraClassName = "",
  ) =>
    items.length > 0 ? (
      <Box
        className={`chat-run-metrics${extraClassName ? ` ${extraClassName}` : ""}`}
        aria-label={ariaLabel}
      >
        {items.map((item) => (
          <span className="chat-run-metric" key={`${keyPrefix}:${item.label}`}>
            <span className="chat-run-metric-label">{item.label}</span>
            <span className="chat-run-metric-value">{item.value}</span>
          </span>
        ))}
      </Box>
    ) : null;

  const renderTranscriptStatusIcon = (
    status: ChatTranscriptActionStatus,
  ): ReactNode => {
    if (status === "done") return <CheckCircleRoundedIcon fontSize="inherit" />;
    if (status === "issue") return <ErrorOutlineRoundedIcon fontSize="inherit" />;
    return <span className="chat-transcript-running-indicator" />;
  };

  const renderTranscriptActionDetail = (
    parentId: string,
    entry: ChatTranscriptActionDetail,
    entryIdx: number,
  ) => {
    const audit = transcriptCommandAuditFromCard(entry.card);
    return (
      <Box
        key={`${parentId}:detail:${entry.id}:${entryIdx}`}
        className={`chat-transcript-action-detail-shell${audit ? " has-audit" : ""}`}
      >
        <button
          type="button"
          className={`chat-transcript-action-detail-row status-${entry.status}`}
          onClick={() => handleActivateStep(entry.card.id)}
          aria-label={`${entry.label}${entry.detail ? `: ${entry.detail}` : ""}`}
        >
          <span
            className="chat-transcript-action-detail-dot"
            aria-hidden="true"
          />
          <span className="chat-transcript-action-detail-copy">
            <span className="chat-transcript-action-detail-label">
              {entry.label}
            </span>
            {entry.detail ? (
              <span className="chat-transcript-action-detail-text">
                {entry.detail}
              </span>
            ) : null}
          </span>
        </button>
        {audit ? (
          <Box
            className="chat-transcript-command-audit"
            aria-label={`${entry.label} command and output`}
          >
            {audit.command ? (
              <Box className="chat-transcript-command-block">
                <span className="chat-transcript-command-label">
                  {audit.commandLabel}
                </span>
                <Box component="pre" className="chat-transcript-command-pre">
                  {audit.command}
                </Box>
              </Box>
            ) : null}
            {audit.output ? (
              <Box className="chat-transcript-command-block">
                <span className="chat-transcript-command-label">
                  {audit.outputLabel}
                </span>
                <Box component="pre" className="chat-transcript-command-pre">
                  {audit.output}
                </Box>
              </Box>
            ) : null}
          </Box>
        ) : null}
      </Box>
    );
  };

  const renderChatTranscriptItems = (
    items: ChatTranscriptItem[],
    keyPrefix: string,
    isLiveTranscript = false,
  ): ReactNode => {
    if (items.length === 0) return null;

    const renderActionRow = (
      item: Extract<ChatTranscriptItem, { kind: "action" }>,
      key: string,
    ): ReactNode => {
      const statusIcon = renderTranscriptStatusIcon(item.status);
      const expanded = expandedTranscriptActions.has(item.id);
      const stepCount = item.details.length;
      return (
        <Box
          key={key}
          className={`chat-transcript-action-shell status-${item.status}${expanded ? " is-expanded" : ""}`}
        >
          <button
            type="button"
            className={`chat-transcript-action-row status-${item.status}`}
            onClick={() => {
              handleActivateStep(item.card.id);
              toggleExpandedTranscriptAction(item.id);
            }}
            aria-expanded={expanded}
            aria-label={`${expanded ? "Collapse" : "Expand"} ${item.title}${item.detail ? `: ${item.detail}` : ""}`}
          >
            <span className="chat-transcript-action-icon" aria-hidden="true">
              {statusIcon}
            </span>
            <span className="chat-transcript-action-main">
              <span className="chat-transcript-action-title">{item.title}</span>
              {item.count && item.count > 1 ? (
                <span
                  className="chat-transcript-action-count-chip"
                  aria-label={`${item.count} calls`}
                >
                  ×{item.count}
                </span>
              ) : null}
              {item.detail ? (
                <>
                  <span
                    className="chat-transcript-action-separator"
                    aria-hidden="true"
                  >
                    |
                  </span>
                  <span className="chat-transcript-action-detail">
                    {item.detail}
                  </span>
                </>
              ) : null}
            </span>
            <ExpandMoreIcon
              className="chat-transcript-action-chevron"
              fontSize="inherit"
              aria-hidden="true"
            />
          </button>
          <Collapse in={expanded && stepCount > 0} timeout="auto" unmountOnExit>
            <Box className="chat-transcript-action-details">
              {item.details.map((entry, entryIdx) =>
                renderTranscriptActionDetail(item.id, entry, entryIdx)
              )}
            </Box>
          </Collapse>
        </Box>
      );
    };

    const renderActionGroup = (
      actions: Extract<ChatTranscriptItem, { kind: "action" }>[],
      groupId: string,
    ): ReactNode => {
      const totalCount = actions.reduce(
        (sum, a) => sum + (a.count ?? 1),
        0,
      );
      const hasIssue = actions.some((a) => a.status === "issue");
      const hasRunning = actions.some((a) => a.status === "running");
      // Force-expand the live step group only while a step is actually in
      // flight, then auto-collapse once all steps finish. Manual expand
      // (expandedTranscriptActions) still wins. Gate on hasRunning — NOT
      // streaming-reply presence — so the card stays open through interleaved
      // prose-then-more-tools runs.
      const expanded =
        (isLiveTranscript && hasRunning) ||
        expandedTranscriptActions.has(groupId);
      const aggregateStatus: ChatTranscriptActionStatus = hasIssue
        ? "issue"
        : hasRunning
          ? "running"
          : "done";
      // Collapsed header carries a one-line tour of the tools used so the row
      // reads as activity, not a bare count.
      const distinctTitles: string[] = [];
      for (const action of actions) {
        if (action.title && !distinctTitles.includes(action.title)) {
          distinctTitles.push(action.title);
        }
      }
      const shownTitles = distinctTitles.slice(0, 3);
      const groupSummary =
        shownTitles.join(", ") +
        (distinctTitles.length > shownTitles.length
          ? ` +${distinctTitles.length - shownTitles.length} more`
          : "");
      return (
        <Box
          key={groupId}
          className={`chat-transcript-action-group status-${aggregateStatus}${expanded ? " is-expanded" : ""}`}
        >
          <button
            type="button"
            className={`chat-transcript-action-group-header status-${aggregateStatus}`}
            onClick={() => toggleExpandedTranscriptAction(groupId)}
            aria-expanded={expanded}
            aria-label={`${expanded ? "Collapse" : "Expand"} ${totalCount} ${totalCount === 1 ? "step" : "steps"}${groupSummary ? `: ${groupSummary}` : ""}`}
          >
            {aggregateStatus !== "done" ? (
              <span className="chat-transcript-action-icon" aria-hidden="true">
                {renderTranscriptStatusIcon(aggregateStatus)}
              </span>
            ) : null}
            <span className="chat-transcript-action-group-title">
              {totalCount} {totalCount === 1 ? "step" : "steps"}
            </span>
            {groupSummary ? (
              <span className="chat-transcript-action-group-summary">
                {groupSummary}
              </span>
            ) : null}
          </button>
          <Collapse in={expanded} timeout="auto" unmountOnExit>
            <Box className="chat-transcript-action-group-body">
              {actions.map((action, i) =>
                renderActionRow(action, `${groupId}:${action.id}:${i}`)
              )}
            </Box>
          </Collapse>
        </Box>
      );
    };

    const renderProse = (
      item: Extract<ChatTranscriptItem, { kind: "prose" }>,
      key: string,
    ): ReactNode => (
      <Typography
        key={key}
        variant="body2"
        className={`chat-transcript-prose-line${isLiveTranscript ? " is-live" : ""}`}
      >
        {item.text}
      </Typography>
    );

    const renderReasoning = (
      item: Extract<ChatTranscriptItem, { kind: "reasoning" }>,
      key: string,
    ): ReactNode => {
      const expanded = expandedTranscriptActions.has(item.id);
      const stepCount = item.details.length;
      return (
        <Box
          key={key}
          className={`chat-transcript-action-shell chat-transcript-reasoning-shell status-${item.status}${expanded ? " is-expanded" : ""}`}
        >
          <button
            type="button"
            className={`chat-transcript-action-row chat-transcript-reasoning-row status-${item.status}`}
            onClick={() => toggleExpandedTranscriptAction(item.id)}
            aria-expanded={expanded}
            aria-label={`${expanded ? "Collapse" : "Expand"} ${item.title}`}
          >
            <span className="chat-transcript-action-icon" aria-hidden="true">
              {renderTranscriptStatusIcon(item.status)}
            </span>
            <span className="chat-transcript-action-main">
              <span className="chat-transcript-action-title">{item.title}</span>
              {item.detail ? (
                <>
                  <span
                    className="chat-transcript-action-separator"
                    aria-hidden="true"
                  >
                    |
                  </span>
                  <span className="chat-transcript-action-detail">
                    {item.detail}
                  </span>
                </>
              ) : null}
            </span>
            <ExpandMoreIcon
              className="chat-transcript-action-chevron"
              fontSize="inherit"
              aria-hidden="true"
            />
          </button>
          <Collapse in={expanded && stepCount > 0} timeout="auto" unmountOnExit>
            <Box className="chat-transcript-action-details chat-transcript-reasoning-details">
              {item.details.map((entry, entryIdx) =>
                renderTranscriptActionDetail(item.id, entry, entryIdx)
              )}
            </Box>
          </Collapse>
        </Box>
      );
    };

    // Walk items, grouping consecutive action items into runs.
    // Every run (1+) sits inside a "N step(s)" parent so the layout shape
    // stays consistent whether the turn used one tool or twenty.
    const nodes: ReactNode[] = [];
    let actionRun: Extract<ChatTranscriptItem, { kind: "action" }>[] = [];
    let groupCounter = 0;
    const flushActionRun = () => {
      if (actionRun.length === 0) return;
      const groupId = `${keyPrefix}:group:${groupCounter++}`;
      nodes.push(renderActionGroup(actionRun.slice(), groupId));
      actionRun = [];
    };

    items.forEach((item, idx) => {
      if (item.kind === "action") {
        actionRun.push(item);
      } else {
        flushActionRun();
        const key = `${keyPrefix}:${item.id}:${idx}`;
        if (item.kind === "prose") {
          nodes.push(renderProse(item, key));
        } else {
          nodes.push(renderReasoning(item, key));
        }
      }
    });
    flushActionRun();

    return (
      <Box
        className={`chat-agent-transcript${isLiveTranscript ? " is-live" : ""}`}
        aria-label="Agent activity transcript"
      >
        {nodes}
      </Box>
    );
  };

  const renderComposerInput = (placeholder: string) => (
    <ChatComposerInput
      attachedFilesCount={attachedFiles.length}
      composerLocked={credentialUiActive || currentConversationHasActiveRun}
      deepResearchDisabled={deepResearchDisabled}
      deepResearchEnabled={deepResearchEnabled}
      isStoppingStream={isStoppingStream}
      isStreaming={canStopCurrentRun}
      onAttachFiles={() => fileInputRef.current?.click()}
      onStopStreaming={() => {
        void handleStopStreaming();
      }}
      onSubmit={(draft) =>
        submitComposerMessage(draft, attachedFiles, composerBrowserProfileContext)
      }
      onToggleDeepResearch={() => setDeepResearchEnabled((prev) => !prev)}
      placeholder={placeholder}
      prefillRequest={composerPrefillRequest}
    />
  );

  return (
    <Box
      sx={{
        flex: 1,
        width: "100%",
        height: "100%",
        minHeight: 0,
        minWidth: 0,
        display: "grid",
        gridTemplateColumns: {
          xs: "1fr",
          md: showConversationSidebarInline
            ? "clamp(288px, 24vw, 340px) minmax(0,1fr)"
            : "1fr",
          lg: showWorkspacePanelInline
            ? showConversationSidebarInline
              ? "clamp(288px, 17vw, 336px) minmax(0,1fr) clamp(420px, 32vw, 640px)"
              : "minmax(0,1fr) clamp(460px, 38vw, 720px)"
            : showConversationSidebarInline
              ? "clamp(296px, 20vw, 344px) minmax(0,1fr)"
              : "minmax(0,1fr)",
          xl: showWorkspacePanelInline
            ? showConversationSidebarInline
              ? "clamp(300px, 17.5vw, 348px) minmax(0,1fr) clamp(480px, 34vw, 700px)"
              : "minmax(0,1fr) clamp(520px, 40vw, 780px)"
            : showConversationSidebarInline
              ? "clamp(304px, 18vw, 360px) minmax(0,1fr)"
              : "minmax(0,1fr)",
        },
        gap: { xs: 1, md: 1.15 },
      }}
    >
      {showConversationSidebarInline
        ? renderConversationSidebarContent()
        : null}
      <Box
        data-tour-target="chat-workspace"
        className={`list-shell chat-shell chat-density-immersive${showWorkspacePanelInline ? " chat-shell-console-open" : ""}${isDragOverChat ? " chat-shell-drop-active" : ""}`}
        sx={{
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          position: "relative",
        }}
        onDragEnter={handleChatDragEnter}
        onDragOver={handleChatDragOver}
        onDragLeave={handleChatDragLeave}
        onDrop={handleChatDrop}
        onPaste={handleChatPaste}
      >
        <Stack
          direction={{ xs: "column", sm: "row" }}
          spacing={0.75}
          sx={{
            justifyContent: "space-between",
            alignItems: { xs: "stretch", sm: "center" },
            mb: 0.75,
          }}
        >
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{
              alignItems: "center",
              minWidth: 0,
              flexWrap: "wrap",
            }}
          >
            <Button
              size="small"
              variant="outlined"
              className={`chat-toolbar-btn${showConversationSidebarInline || showConversationSidebarDrawer ? " active" : ""}`}
              startIcon={
                showConversationSidebarInline ? (
                  <ChevronLeftRoundedIcon fontSize="small" />
                ) : (
                  <ChevronRightRoundedIcon fontSize="small" />
                )
              }
              onClick={() => {
                if (canInlineConversationSidebar) {
                  setConversationSidebarOpen((prev) => !prev);
                } else {
                  setConversationSidebarOpen(true);
                }
              }}
            >
              {showConversationSidebarInline
                ? "Hide conversations"
                : "Conversations"}
            </Button>
            {!showConversationSidebarInline ? (
              <Button
                size="small"
                variant="outlined"
                className="chat-toolbar-btn"
                onClick={() => startNewConversation()}
              >
                New chat
              </Button>
            ) : null}
            <Avatar
              src={AgentLogo}
              variant="rounded"
              sx={{ width: 18, height: 18, bgcolor: "var(--ui-rgba-12-22-40-850)" }}
            />
            <Typography variant="caption" className="chat-toolbar-context">
              Workspace
            </Typography>
          </Stack>
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{
              alignItems: "center",
              minWidth: 0,
              flexWrap: "wrap",
              justifyContent: { xs: "flex-start", sm: "flex-end" },
            }}
          >
            {activeConversationSession ? (
              <button
                type="button"
                className="chat-session-pill"
                disabled={!onNavigateToView}
                onClick={() => onNavigateToView?.("background-work")}
                title={`Session: ${str(activeConversationSession.title, "Background session")}`}
              >
                <span className="chat-session-pill-dot" aria-hidden="true" />
                <span className="chat-session-pill-label">
                  Session: {str(activeConversationSession.title, "Background session")}
                </span>
              </button>
            ) : null}
            <Tooltip
              title={
                showWorkspacePanelInline || showWorkspacePanelDrawer
                  ? "Hide Run Details"
                  : "Show Run Details"
              }
            >
              <span
                className={`activity-toggle-pill${showWorkspacePanelInline || showWorkspacePanelDrawer ? " active" : ""}${isStreamingForCurrentConversation ? " streaming" : ""}`}
                onClick={() => {
                  if (canInlineWorkspacePanel) {
                    setWorkspaceOpen((prev) => {
                      const next = !prev;
                      // Opening via the pill re-enables stream auto-open;
                      // closing suppresses it for the rest of the run.
                      workspaceUserClosedRef.current = !next;
                      return next;
                    });
                  } else {
                    workspaceUserClosedRef.current = false;
                    setWorkspaceOpen(true);
                  }
                }}
                style={{ display: "inline-flex" }}
              >
                <span className="toggle-dot" />
                Run Details
              </span>
            </Tooltip>
          </Stack>
        </Stack>
        <Box
          className="chat-main-column"
          sx={{
            flex: 1,
            minHeight: 0,
            width: "100%",
            display: "flex",
            flexDirection: "column",
          }}
        >
          <Box className="chat-reading-column">
            <Box
              ref={threadRef}
              sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
              className={`chat-thread chat-thread-immersive${showEmptyHero ? " chat-thread-empty" : ""}`}
            >
              {showEmptyHero ? (
                <Box className="chat-empty-state">
                  <Box className="chat-empty-brand">
                    <Box
                      component="img"
                      src={AgentLogo}
                      alt=""
                      className="chat-empty-logo"
                    />
                    <Typography variant="h4" className="chat-empty-title">
                      {conversationId ? "Conversation" : "AgentArk"}
                    </Typography>
                    {!conversationId ? (
                      <Typography
                        component="p"
                        className="chat-empty-subtitle"
                      >
                        Your local AI command center.
                      </Typography>
                    ) : null}
                  </Box>
                  {!emptyEarlyAccessNoticeDismissed ? (
                    <Box
                      className="chat-empty-early-access"
                      role="status"
                      aria-live="polite"
                    >
                      <InfoOutlinedIcon
                        className="chat-empty-early-access-icon"
                        fontSize="small"
                        aria-hidden="true"
                      />
                      <Typography
                        component="p"
                        className="chat-empty-early-access-copy"
                      >
                        <strong>AgentArk is in beta and can make mistakes.</strong>{" "}
                        Review browser actions, connected-tool results, and
                        long-running work before relying on them.
                      </Typography>
                      <IconButton
                        size="small"
                        className="chat-empty-early-access-close"
                        aria-label="Dismiss early access notice"
                        onClick={() => {
                          dismissEarlyAccessNoticeForSevenDays();
                          setEmptyEarlyAccessNoticeDismissed(true);
                        }}
                      >
                        <CloseIcon fontSize="small" />
                      </IconButton>
                    </Box>
                  ) : null}
                  <Box className="chat-empty-composer-wrap">
                    {renderAttachedFilePills("chat-empty-attachments")}
                    <Box
                      className={`chat-composer-shell chat-composer-shell-centered${shouldShowExecutionPlanWarning ? " has-plan" : ""}`}
                    >
                      {renderComposerInput(emptyComposerPlaceholder)}
                    </Box>
                  </Box>
                  <Box className="chat-starter-shell">
                    <Box className="chat-starter-tabs-head">
                      <Typography
                        variant="caption"
                        className="chat-starter-caption"
                      >
                        Common starts
                      </Typography>
                      <Tabs
                        value={starterActiveTab}
                        onChange={(_, value) =>
                          setStarterActiveTab(value as ChatStarterTabId)
                        }
                        variant="scrollable"
                        scrollButtons="auto"
                        className="chat-starter-tabs"
                      >
                        {CHAT_STARTER_TAB_ORDER.map((categoryId) => {
                          const TabIcon = starterTabIcon(categoryId);
                          return (
                            <Tab
                              key={categoryId}
                              value={categoryId}
                              iconPosition="start"
                              icon={
                                <TabIcon
                                  size={16}
                                  strokeWidth={1.85}
                                  className="chat-starter-tab-icon"
                                />
                              }
                              label={CHAT_STARTER_TAB_LABELS[categoryId]}
                            />
                          );
                        })}
                      </Tabs>
                    </Box>
                    <Box
                      className={`chat-starter-grid${starterActiveTab === "all" ? "" : " chat-starter-grid-expanded"}`}
                    >
                      {starterVisibleExamples.map(renderStarterExampleCard)}
                    </Box>
                    {CHAT_STARTER_ADVANCED_EXAMPLES.length > 0 ? (
                      <Box className="chat-starter-advanced">
                        <Button
                          size="small"
                          variant="text"
                          className="chat-starter-toggle"
                          endIcon={
                            <ExpandMoreIcon
                              sx={{
                                transform: starterAdvancedExpanded
                                  ? "rotate(180deg)"
                                  : "rotate(0deg)",
                                transition: "transform 0.16s ease",
                              }}
                            />
                          }
                          onClick={() =>
                            setStarterAdvancedExpanded((prev) => !prev)
                          }
                        >
                          {starterAdvancedExpanded ? "Hide advanced" : "Advanced"}
                        </Button>
                        {starterAdvancedExpanded ? (
                          <Box className="chat-starter-grid chat-starter-grid-expanded">
                            {CHAT_STARTER_ADVANCED_EXAMPLES.map(
                              renderStarterExampleCard,
                            )}
                          </Box>
                        ) : null}
                      </Box>
                    ) : null}
                  </Box>
                </Box>
              ) : (
                <Stack spacing={1.5}>
                  {messageRenderBundle.map((bundle) => {
                    const {
                      message,
                      idx,
                      messageId,
                      isUser,
                      isAssistant,
                      tsRaw,
                      ts,
                      renderedContent,
                      attachments,
                      messageChoices,
                      researchReport,
                      previousUserPrompt,
                      traceId,
                      hasTrace,
                      markdownNode,
                      runMetricItems,
                      runMetrics,
                    } = bundle;
                    const traceTranscriptItems = isAssistant
                      ? perMessageTraceTranscriptById[messageId] || []
                      : [];
                    const messageTranscriptItems =
                      isAssistant &&
                      traceTranscriptItems.length === 0 &&
                      !showStreamingAssistant &&
                      idx === messages.length - 1
                        ? completedRunTranscriptItems
                        : traceTranscriptItems;
                    const isPlanConfirmationMessage =
                      isAssistant &&
                      showPlanConfirmationCard &&
                      idx === planConfirmationMessageIndex;
                    const rawTraceSteps = hasTrace
                      ? traceStepsById[traceId] || []
                      : [];
                    const traceShowsAwaitingPlanConfirmation =
                      rawTraceSteps.length > 0 &&
                      activityStepsRepresentAwaitingPlanConfirmation(
                        rawTraceSteps,
                      );
                    const shouldInsertCompletedProgressBeforeMessage =
                      shouldInlineCompletedProgressBeforeAssistant &&
                      messageId === completedProgressBeforeMessageId;
                    if (
                      showPlanConfirmationCard &&
                      !isPlanConfirmationMessage &&
                      isAssistant &&
                      traceShowsAwaitingPlanConfirmation
                    ) {
                      return null;
                    }
                    if (
                      isAssistant &&
                      (showPlanConfirmationCard ||
                        shouldPreferDeepResearchPlanCard) &&
                      looksLikeDiscardableResearchFailureMessage(
                        renderedContent,
                      )
                    ) {
                      return null;
                    }
                    return (
                      <Fragment key={messageId}>
                        {shouldInsertCompletedProgressBeforeMessage
                          ? renderProgressRows("completed-progress")
                          : null}
                        <Box
                          className={
                            isUser
                              ? "chat-row chat-row-user"
                              : `chat-row${isPlanConfirmationMessage ? " chat-row-plan-confirmation" : ""}`
                          }
                        >
                          {!isUser ? renderAgentAvatar() : null}
                          <Box
                            className={
                              isUser
                                ? "chat-bubble chat-bubble-user"
                                : `chat-bubble chat-bubble-assistant${isPlanConfirmationMessage ? " chat-bubble-plan-confirmation" : ""}`
                            }
                          >
                            {isPlanConfirmationMessage ? (
                              renderPlanConfirmationCard({ threadMode: true })
                            ) : (
                              <>
                                <Stack
                                  direction="row"
                                  spacing={0.5}
                                  sx={{
                                    justifyContent: "space-between",
                                    alignItems: "center",
                                  }}
                                >
                                  <Typography
                                    variant="caption"
                                    title={ts?.tooltip || undefined}
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {isUser ? "You" : "AgentArk"}
                                    {ts ? ` | ${ts.label}` : ""}
                                  </Typography>
                                  <Stack
                                    direction="row"
                                    spacing={0.25}
                                    sx={{
                                      alignItems: "center",
                                    }}
                                  >
                                    {!isUser ? (
                                      <Tooltip title="Download reply">
                                        <IconButton
                                          size="small"
                                          onClick={() => {
                                            void exportAssistantMessage(
                                              message,
                                              previousUserPrompt,
                                            );
                                          }}
                                          sx={{
                                            color: "var(--ui-rgba-189-216-249-900)",
                                          }}
                                        >
                                          <FileDownloadRoundedIcon fontSize="small" />
                                        </IconButton>
                                      </Tooltip>
                                    ) : null}
                                    <Tooltip title="Copy message">
                                      <IconButton
                                        size="small"
                                        onClick={() => {
                                          void copyMessage(message);
                                        }}
                                        sx={{
                                          color: "var(--ui-rgba-189-216-249-900)",
                                        }}
                                      >
                                        <ContentCopyRoundedIcon fontSize="small" />
                                      </IconButton>
                                    </Tooltip>
                                  </Stack>
                                </Stack>
                                {/* Detailed tool/action outputs stay in Run Details; chat shows compact transcript rows. */}
                                {isUser ? (
                                  <>
                                    <Typography
                                      variant="body2"
                                      sx={{ whiteSpace: "pre-wrap" }}
                                    >
                                      {renderedContent}
                                    </Typography>
                                    {renderTurnAttachmentPills(
                                      attachments,
                                      messageId,
                                    )}
                                  </>
                                ) : (
                                  <>
                                    {messageTranscriptItems.length > 0
                                      ? renderChatTranscriptItems(
                                          messageTranscriptItems,
                                          `message-transcript:${messageId}`,
                                        )
                                      : null}
                                    {researchReport
                                      ? renderResearchReportCard({
                                          report: researchReport,
                                          previousUserPrompt,
                                          messageId,
                                          timestamp: tsRaw,
                                          traceId,
                                        })
                                      : isRunCancellationArtifact(
                                            renderedContent,
                                          )
                                        ? <CancelledRunNotice />
                                        : markdownNode}
                                    {runMetrics ? (
                                      <ChatRunMetricsCard
                                        metrics={runMetrics}
                                        keyPrefix={`response-metrics:${messageId}`}
                                      />
                                    ) : null}
                                    {!runMetrics && runMetricItems.length > 0
                                      ? renderRunMetricPills(
                                          runMetricItems,
                                          `response-metrics:${messageId}`,
                                          "Response token usage",
                                          "chat-run-metrics-response",
                                        )
                                      : null}
                                  </>
                                )}
                                {!isUser
                                  ? renderClarificationChoiceGroup(
                                      messageId,
                                      messageChoices,
                                      idx < messages.length - 1,
                                    )
                                  : null}
                              </>
                            )}
                          </Box>
                          {isUser ? renderUserAvatar() : null}
                        </Box>
                      </Fragment>
                    );
                  })}

                  {visiblePendingUserMessage &&
                  (showStreamingAssistant || showInterruptedRunCard) &&
                  latestPendingUserMessageIndex === -1 ? (
                    <Box className="chat-row chat-row-user">
                      <Box className="chat-bubble chat-bubble-user">
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {pendingUserMessageLabel}
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{ whiteSpace: "pre-wrap" }}
                        >
                          {visiblePendingUserMessage}
                        </Typography>
                        {renderTurnAttachmentPills(
                          pendingTurnAttachments,
                          "pending-user-message",
                        )}
                      </Box>
                      {renderUserAvatar("chat-avatar-pending chat-avatar-user-live")}
                    </Box>
                  ) : null}

                  {selectedConversationAwaitingPersistedMessages &&
                  messages.length === 0 ? (
                    <Box className="chat-row chat-thinking-inline">
                      {renderAgentAvatar("chat-avatar-working")}
                      <span className="chat-thinking-text">
                        {activeConversationActivityLoading
                          ? "Loading conversation..."
                          : "Waiting for the first saved message..."}
                      </span>
                    </Box>
                  ) : null}

                  {visibleFailedUserMessage &&
                  !isStreamingForCurrentConversation ? (
                    <Box className="chat-row chat-row-user">
                      <Box className="chat-bubble chat-bubble-user">
                        <Typography
                          variant="caption"
                          sx={{
                            color: "warning.main",
                          }}
                        >
                          You | not sent
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{ whiteSpace: "pre-wrap" }}
                        >
                          {visibleFailedUserMessage}
                        </Typography>
                        {renderTurnAttachmentPills(
                          pendingTurnAttachments,
                          "failed-user-message",
                        )}
                      </Box>
                      {renderUserAvatar()}
                    </Box>
                  ) : null}

                  {visibleStreamingProgressMessages.length > 0 &&
                  inlineWorkspaceCards.length === 0 &&
                  !showStreamingAssistant &&
                  (showStreamingAssistant || !completedProgressBeforeMessageId)
                    ? renderProgressRows(
                        showStreamingAssistant
                          ? "stream-progress-live"
                          : "stream-progress-fallback",
                      )
                    : null}

                  {showPlanConfirmationCard &&
                  !showStreamingAssistant &&
                  planConfirmationMessageIndex === -1 ? (
                    <Box className="chat-row chat-row-plan-confirmation">
                      {renderAgentAvatar()}
                      <Box className="chat-bubble chat-bubble-assistant chat-bubble-plan-confirmation">
                        {renderPlanConfirmationCard({ threadMode: true })}
                      </Box>
                    </Box>
                  ) : null}

                    {showInterruptedRunCard ? (
                      <Box className="chat-row chat-row-interrupted-work">
                        {renderAgentAvatar()}
                        <Box className="chat-bubble chat-bubble-assistant chat-bubble-interrupted-work">
                          <Stack spacing={1}>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "warning.main",
                            }}
                          >
                            AgentArk | interrupted
                          </Typography>
                          {visibleStreamingResponse.trim() &&
                          !isRunCancellationArtifact(visibleStreamingResponse) ? (
                            streamingResearchReport ? (
                              renderResearchReportCard({
                                report: streamingResearchReport,
                                previousUserPrompt: streamingResearchPrompt,
                                messageId: "streaming-interrupted-report",
                                isStreaming: true,
                              })
                            ) : (
                              renderChatMarkdown(visibleStreamingResponse, {
                                snippetNamespace: "streaming-interrupted",
                                onOpenSnippet: openCodePreviewInWorkspace,
                              })
                            )
                          ) : (
                            <Typography
                              variant="body2"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {interruptedRunDetail ||
                                "This run was interrupted before a full reply was sent."}
                            </Typography>
                          )}
                          {renderChatTranscriptItems(
                            liveChatTranscriptItems,
                            "interrupted-transcript",
                          )}
                          {TASK_RETRY_CONTROLS_ENABLED ? (
                            <Box>
                              <Button
                                size="small"
                                variant="outlined"
                                disabled={isStreaming || !canRecoverInterruptedRun}
                                onClick={() => {
                                  const taskId = str(
                                    pendingRunSnapshot?.taskId,
                                    "",
                                  ).trim();
                                  if (taskId) {
                                    void runStreamingChat("", [], {
                                      conversationIdOverride:
                                        conversationId || undefined,
                                      resumeTaskId: taskId,
                                    });
                                    return;
                                  }
                                  if (!interruptedRetryMessage) return;
                                  void runStreamingChat(interruptedRetryMessage, [], {
                                    conversationIdOverride:
                                      conversationId ||
                                      str(
                                        pendingRunSnapshot?.conversationId,
                                        "",
                                      ).trim() ||
                                      undefined,
                                  });
                                }}
                              >
                                {interruptedTaskId ? "Resume" : "Retry"}
                              </Button>
                            </Box>
                          ) : null}
                        </Stack>
                      </Box>
                    </Box>
                  ) : null}

                  {showStreamingAssistant &&
                  latestStreamingAssistantIndex === -1 ? (
                    !hasVisibleStreamingReply &&
                    !isRunningPlanConfirmation &&
                    visibleStreamingProgressMessages.length === 0 &&
                    liveChatTranscriptItems.length === 0 ? (
                    <Box className="chat-row chat-thinking-inline">
                      {renderAgentAvatar("chat-avatar-working")}
                      <span className="chat-thinking-text">
                        {streamingActivity}
                      </span>
                    </Box>
                  ) : (
                    <>
                      {(isAwaitingPlanConfirmation ||
                        isRunningPlanConfirmation) &&
                      !hasVisibleStreamingReply ? (
                        <Box className="chat-row chat-row-plan-confirmation">
                          {renderAgentAvatar("chat-avatar-working")}
                          <Box className="chat-bubble chat-bubble-assistant chat-bubble-plan-confirmation">
                            {renderPlanConfirmationCard({ threadMode: true })}
                          </Box>
                        </Box>
                      ) : (
                        <>
                          {/* Live activity renders ABOVE the streaming reply,
                              mirroring the persisted-message layout (transcript
                              before markdown) so nothing dangles under the
                              caret and the layout doesn't jump on completion. */}
                          {showLiveExecutionPanel ? (
                            <Box className="chat-row chat-row-live-work">
                              {renderAgentAvatar("chat-avatar-working")}
                              <Box
                                className={`chat-live-work-panel${
                                  liveChatTranscriptItems.length > 0
                                    ? " has-transcript"
                                    : " has-status-copy"
                                }${CHAT_LAYOUT_MODE === "split" ? " is-split" : ""}`}
                              >
                                {liveModelProseText ? (
                                  <Typography
                                    variant="body2"
                                    className="chat-live-model-prose"
                                    sx={{
                                      color: "#e9e2d6",
                                      fontSize: "0.9rem",
                                      lineHeight: 1.55,
                                      whiteSpace: "pre-wrap",
                                      mb:
                                        liveChatTranscriptItems.length > 0
                                          ? 1
                                          : 0.5,
                                    }}
                                  >
                                    {liveModelProseText}
                                  </Typography>
                                ) : null}
                                {liveChatTranscriptItems.length > 0 ? (
                                  renderChatTranscriptItems(
                                    liveChatTranscriptItems,
                                    "live-transcript",
                                    true,
                                  )
                                ) : (
                                  <Typography
                                    variant="body2"
                                    className="chat-live-ack-copy"
                                  >
                                    {nowDoingLabel || streamingActivity}
                                  </Typography>
                                )}
                                {streamingRunMetricItems.length > 0 ? (
                                  <Box
                                    className="chat-run-metrics chat-run-metrics-live"
                                    aria-label="Live run metrics"
                                  >
                                    {streamingRunMetricItems.map((item) => (
                                      <span
                                        className="chat-run-metric"
                                        key={`streaming:${item.label}`}
                                      >
                                        <span className="chat-run-metric-label">
                                          {item.label}
                                        </span>
                                        <span className="chat-run-metric-value">
                                          {item.value}
                                        </span>
                                      </span>
                                    ))}
                                  </Box>
                                ) : null}
                              </Box>
                            </Box>
                          ) : null}
                          {hasVisibleStreamingReply ? (
                            <Box className="chat-row">
                              {showLiveExecutionPanel ? (
                                <Box className="chat-avatar-spacer" aria-hidden="true" />
                              ) : (
                                renderAgentAvatar("chat-avatar-working")
                              )}
                              <Box className="chat-bubble chat-bubble-assistant chat-bubble-streaming chat-bubble-streaming-reply">
                                <Typography
                                  variant="caption"
                                  className="chat-streaming-status"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  {streamingResearchReport
                                    ? "Deep research report is streaming..."
                                    : nowDoingLabel || streamingActivity}
                                </Typography>
                                <Box className="chat-stream-section-reply">
                                  {streamingResearchReport ? (
                                    <Box sx={{ position: "relative" }}>
                                      {renderResearchReportCard({
                                        report: streamingResearchReport,
                                        previousUserPrompt: streamingResearchPrompt,
                                        messageId: "streaming-report",
                                        isStreaming: true,
                                      })}
                                      <span className="stream-caret" />
                                    </Box>
                                  ) : (
                                    renderStreamingChatMarkdown(
                                      visibleStreamingMarkdownText,
                                      {
                                        snippetNamespace: "streaming-reply",
                                        onOpenSnippet: openCodePreviewInWorkspace,
                                      },
                                    )
                                  )}
                                </Box>
                                {renderClarificationChoiceGroup(
                                  `streaming-clarification:${str(
                                    pendingRunSnapshot?.runId,
                                    (visibleStreamingResponse || visibleLiveModelEmit).slice(0, 80),
                                  )}`,
                                  streamingResponseChoices,
                                )}
                              </Box>
                            </Box>
                          ) : null}
                          {!hasVisibleStreamingReply && !showLiveExecutionPanel ? (
                            <Box className="chat-row">
                              {renderAgentAvatar("chat-avatar-working")}
                              <Box className="chat-bubble chat-bubble-assistant chat-bubble-streaming">
                                <Typography
                                  variant="caption"
                                  className="chat-streaming-status"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  {nowDoingLabel || streamingActivity}
                                </Typography>
                                <div className="typing-dots" aria-label="typing">
                                  <span />
                                  <span />
                                  <span />
                                </div>
                              </Box>
                            </Box>
                          ) : null}
                        </>
                      )}
                    </>
                  )
                  ) : null}
                </Stack>
              )}
            </Box>

            {visibleConversationError ? (
              <Alert severity="error" sx={{ mt: 1 }}>
                <Stack spacing={1}>
                  <Typography variant="body2">
                    {normalizeChatError(
                      chatError ||
                        errMessage(
                          visibleConversationListError || visibleMessagesError,
                        ),
                    )}
                  </Typography>
                  {searchSetupActionNeeded ? (
                    <Box>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={() => onNavigateToView?.("search")}
                      >
                        Open Search Settings
                      </Button>
                    </Box>
                  ) : null}
                </Stack>
              </Alert>
            ) : null}
            {credentialActionNeeded ? (
              <Box className="chat-action-required" sx={{ mt: 1 }}>
                <Stack spacing={1}>
                  <Typography variant="subtitle2">
                    Waiting for your input
                  </Typography>
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    A secure credential is required before I can continue. If
                    this should reuse the current model key, use that option.
                    Otherwise enter the key name and save the value here.
                  </Typography>
                  <Stack
                    direction={{ xs: "column", md: "row" }}
                    spacing={1}
                    className="chat-action-options"
                  >
                    <Button
                      size="small"
                      variant={
                        secretHelperMode === "reuse" ? "contained" : "outlined"
                      }
                      onClick={async () => {
                        setSecretHelperMode("reuse");
                        await submitSecretHelper("reuse");
                      }}
                    >
                      Use current model key
                    </Button>
                    <Button
                      size="small"
                      variant={
                        secretHelperMode === "manual" ? "contained" : "outlined"
                      }
                      onClick={() => setSecretHelperMode("manual")}
                    >
                      Add secret manually
                    </Button>
                  </Stack>
                  <Stack
                    direction={{ xs: "column", md: "row" }}
                    spacing={1}
                    sx={{
                      alignItems: { md: "center" },
                    }}
                  >
                    <TextField
                      size="small"
                      label="Key name"
                      value={secretHelperKey}
                      onChange={(e) =>
                        setSecretHelperKey(e.target.value.toUpperCase())
                      }
                      placeholder="OPENAI_API_KEY"
                      sx={{ minWidth: 210 }}
                    />
                    {secretHelperMode === "manual" ? (
                      <TextField
                        size="small"
                        label="Key value"
                        type="password"
                        value={secretHelperValue}
                        onChange={(e) => setSecretHelperValue(e.target.value)}
                        placeholder="Paste key"
                        sx={{ flex: 1 }}
                      />
                    ) : (
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          flex: 1,
                        }}
                      >
                        Reuses your current model key and stores it encrypted
                        for this app.
                      </Typography>
                    )}
                    <Button
                      size="small"
                      variant="contained"
                      disabled={secretHelperBusy || isStreaming}
                      onClick={() => {
                        void submitSecretHelper();
                      }}
                    >
                      {secretHelperBusy ? "Saving..." : "Save and continue"}
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            ) : null}
            {chatCredentialPromptVisible ? (
              <Box
                className="list-shell"
                sx={{
                  mt: 1,
                  width: "min(100%, 720px)",
                  maxWidth: "100%",
                  alignSelf: "flex-start",
                  borderRadius: 1.5,
                  px: { xs: 1.25, sm: 1.5 },
                  py: 1,
                }}
              >
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={1}
                  sx={{
                    alignItems: { sm: "center" },
                    justifyContent: "space-between",
                  }}
                >
                  <Box sx={{ minWidth: 0 }}>
                    <Typography variant="subtitle2">
                      {str(
                        chatCredentialPrompt.title,
                        "Secure credentials required",
                      )}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", display: "block" }}
                    >
                      Credentials go through a secure form and are stored
                      encrypted — never through normal chat.
                    </Typography>
                  </Box>
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{ flexShrink: 0, alignItems: "center" }}
                  >
                    <Button
                      size="small"
                      variant="contained"
                      disabled={dismissChatCredentialPromptMutation.isPending}
                      onClick={() => setChatCredentialDialogOpen(true)}
                    >
                      Open secure form
                    </Button>
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={
                        submitChatCredentialPromptMutation.isPending ||
                        dismissChatCredentialPromptMutation.isPending
                      }
                      onClick={() => {
                        dismissChatCredentialPromptMutation.mutate();
                      }}
                    >
                      {dismissChatCredentialPromptMutation.isPending
                        ? "Dismissing..."
                        : "Later"}
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            ) : null}
            <Dialog
              open={chatCredentialPromptVisible && chatCredentialDialogOpen}
              onClose={() => setChatCredentialDialogOpen(false)}
              fullWidth
              maxWidth="sm"
              slotProps={{
                paper: {
                  sx: {
                    borderRadius: 2.25,
                    border: "1px solid var(--ui-rgba-255-255-255-080)",
                    background:
                      "linear-gradient(160deg, var(--ui-rgba-24-24-28-980), var(--ui-rgba-15-15-18-950))",
                    backdropFilter: "blur(18px)",
                    WebkitBackdropFilter: "blur(18px)",
                  },
                },
              }}
            >
              <DialogTitle
                sx={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  py: 1.25,
                  px: 2,
                  minHeight: 48,
                  borderBottom: "1px solid var(--ui-rgba-255-255-255-080)",
                }}
              >
                <Typography variant="h6" sx={{ lineHeight: 1 }}>
                  {str(
                    chatCredentialPrompt.title,
                    "Secure credentials required",
                  )}
                </Typography>
                <IconButton
                  size="small"
                  onClick={() => setChatCredentialDialogOpen(false)}
                  aria-label="Close secure credential form"
                >
                  <CloseIcon fontSize="small" />
                </IconButton>
              </DialogTitle>
              <DialogContent sx={{ pt: 2 }}>
                <Stack spacing={1.25} sx={{ mt: 0.5 }}>
                  <Typography
                    variant="body2"
                    sx={{ color: "text.secondary" }}
                  >
                    {str(chatCredentialPrompt.description, "").trim() ||
                      "Provide the requested credential values here so AgentArk can store them encrypted and continue."}
                  </Typography>
                  <Alert
                    severity="warning"
                    variant="outlined"
                    sx={{
                      py: 0.25,
                      px: 1,
                      "& .MuiAlert-icon": { py: 0.25 },
                      "& .MuiAlert-message": { py: 0.25 },
                    }}
                  >
                    {str(chatCredentialPrompt.warning, "").trim() ||
                      CHAT_SECRET_WARNING}
                  </Alert>
                  {chatCredentialPromptIsOAuthShape ? (
                    <Alert severity="info" sx={{ py: 0.5 }}>
                      This integration uses an OAuth browser handoff. Save any
                      client values below, then click the connect button to
                      finish signing in.
                    </Alert>
                  ) : null}
                  <Grid2 container spacing={1}>
                    {chatCredentialPromptFields.map((field, index) => {
                      const key = str(field.key, "").trim();
                      if (!key) return null;
                      const required = toBool(field.required);
                      const inputType = str(field.input_type, "password")
                        .trim()
                        .toLowerCase();
                      const isTextarea = inputType === "textarea";
                      const isPlainText = inputType === "text";
                      const placeholder = str(field.placeholder, "");
                      const help = str(field.help, "").trim();
                      const helperText = help
                        ? help
                        : required
                          ? "Required"
                          : "Optional";
                      return (
                        <Grid2
                          key={`${key}-${index}`}
                          size={{
                            xs: 12,
                            md:
                              chatCredentialPromptFields.length > 1 &&
                              !isTextarea
                                ? 6
                                : 12,
                          }}
                        >
                          <TextField
                            fullWidth
                            size="small"
                            autoFocus={index === 0}
                            type={isPlainText || isTextarea ? "text" : "password"}
                            multiline={isTextarea}
                            minRows={isTextarea ? 3 : undefined}
                            autoComplete="off"
                            label={str(field.label, key)}
                            placeholder={placeholder || undefined}
                            value={chatCredentialValues[key] || ""}
                            onChange={(e) => {
                              const value = e.target.value;
                              setChatCredentialValues((prev) => ({
                                ...prev,
                                [key]: value,
                              }));
                              if (chatCredentialError) {
                                setChatCredentialError(null);
                              }
                            }}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" && !isTextarea) {
                                e.preventDefault();
                                void submitChatCredentialPrompt();
                              }
                            }}
                            helperText={helperText}
                          />
                        </Grid2>
                      );
                    })}
                  </Grid2>
                  {chatCredentialError ? (
                    <Alert severity="error" sx={{ py: 0.5 }}>
                      {chatCredentialError}
                    </Alert>
                  ) : null}
                  {chatCredentialPromptDocsUrl ? (
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary" }}
                    >
                      Where do I get this?{" "}
                      <a
                        href={chatCredentialPromptDocsUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        onClick={handleChatLinkClick}
                      >
                        Open integration docs
                      </a>
                    </Typography>
                  ) : null}
                  {chatCredentialPromptSettingsPath ? (
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                        display: "block",
                      }}
                    >
                      Prefer Settings? Open{" "}
                      <Box
                        component="span"
                        sx={{
                          color: "text.primary",
                          fontWeight: 600,
                        }}
                      >
                        {chatCredentialPromptSettingsPath}
                      </Box>
                      .
                    </Typography>
                  ) : null}
                  {str(chatCredentialPrompt.fallback_command, "").trim() ? (
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      <code>
                        {str(chatCredentialPrompt.fallback_command, "")}
                      </code>
                    </Typography>
                  ) : null}
                </Stack>
              </DialogContent>
              <DialogActions sx={{ px: 2, pb: 1.75, pt: 0.5 }}>
                <Button
                  size="small"
                  variant="outlined"
                  disabled={
                    submitChatCredentialPromptMutation.isPending ||
                    dismissChatCredentialPromptMutation.isPending
                  }
                  onClick={() => {
                    dismissChatCredentialPromptMutation.mutate();
                  }}
                >
                  {dismissChatCredentialPromptMutation.isPending
                    ? "Dismissing..."
                    : "Fill in Settings later"}
                </Button>
                <Button
                  size="small"
                  variant="contained"
                  disabled={
                    submitChatCredentialPromptMutation.isPending ||
                    dismissChatCredentialPromptMutation.isPending ||
                    isStreaming
                  }
                  onClick={() => {
                    void submitChatCredentialPrompt();
                  }}
                >
                  {submitChatCredentialPromptMutation.isPending
                    ? "Saving..."
                    : str(
                        chatCredentialPrompt.submit_label,
                        "Save securely",
                      )}
                </Button>
              </DialogActions>
            </Dialog>
            {chatNotice &&
            !visibleConversationListError &&
            !visibleMessagesError &&
            !chatError ? (
              <Alert severity="info" sx={{ mt: 1 }}>
                {chatNotice}
              </Alert>
            ) : null}
            {isDragOverChat ? (
              <Box className="chat-drop-overlay">
                <Typography variant="subtitle2">
                  Drop files to attach
                </Typography>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Supported: TXT, MD, JSON, CSV, XML, YAML, PDF, DOCX, LOG,
                  HTML, PNG, JPG, WEBP, GIF
                </Typography>
              </Box>
            ) : null}

            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept=".txt,.md,.markdown,.json,.csv,.tsv,.xml,.yaml,.yml,.pdf,.docx,.log,.html,.htm,.png,.jpg,.jpeg,.gif,.webp,.bmp,.tif,.tiff,.svg,image/*"
              style={{ display: "none" }}
              onChange={(e) => {
                queueAttachedFiles(e.target.files);
                e.currentTarget.value = "";
              }}
            />
            {!showEmptyHero ? renderAttachedFilePills() : null}
            {shouldShowCompactExecutionPlan ? (
              <Accordion
                expanded={executionPlanExpanded}
                onChange={(_, expanded) => setExecutionPlanExpanded(expanded)}
                disableGutters
                elevation={0}
                className={`chat-plan-strip${executionPlanExpanded ? " expanded" : ""}`}
              >
                <AccordionSummary
                  expandIcon={
                    <ExpandMoreIcon
                      sx={{ color: "var(--ui-rgba-196-223-255-820)" }}
                    />
                  }
                  className="chat-plan-summary"
                >
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "center",
                      width: "100%",
                      minWidth: 0,
                    }}
                  >
                    <Box sx={{ minWidth: 0 }}>
                      <Stack
                        direction="row"
                        spacing={0.8}
                        useFlexGap
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap",
                        }}
                      >
                        <Typography
                          variant="caption"
                          className="chat-plan-kicker"
                        >
                          Planner
                        </Typography>
                        {executionPlanActiveCount > 0 ||
                        isExecutionPlanFinalizing ||
                        isExecutionPlanTransitioning ? (
                          <CircularProgress
                            size={12}
                            thickness={6}
                            className="chat-plan-spinner"
                          />
                        ) : null}
                        <Typography
                          variant="body2"
                          className="chat-plan-status"
                        >
                          {executionPlanStatusLabel}
                        </Typography>
                      </Stack>
                      <Typography
                        variant="caption"
                        className="chat-plan-summary-copy"
                      >
                        {executionPlanSummaryText}
                      </Typography>
                    </Box>
                  </Stack>
                </AccordionSummary>
                <AccordionDetails className="chat-plan-details">
                  <Stack spacing={0.75}>
                    {displayedExecutionPlan.map((step) => {
                      const isCompleted = step.status === "completed";
                      const isRunning = step.status === "running";
                      const isFailed = step.status === "failed";
                      const stepCopy = describeExecutionPlanStep(
                        step,
                        `Step ${step.id}`,
                      );
                      const stepStatusLabel = executionPlanStepStatusLabel(
                        step.status,
                      );
                      return (
                        <Box
                          key={step.id}
                          className={`chat-plan-step${isCompleted ? " done" : ""}${isRunning ? " running" : ""}${isFailed ? " failed" : ""}`}
                        >
                          <Box className="chat-plan-step-marker">
                            {renderExecutionPlanStatusIcon(
                              step.status,
                              isRunning ? "spin" : "",
                            )}
                          </Box>
                          <Box sx={{ minWidth: 0, flex: 1 }}>
                            <Box className="chat-plan-step-title-row">
                              <Typography
                                variant="body2"
                                className="chat-plan-step-title"
                              >
                                {step.id}. {stepCopy.title}
                              </Typography>
                              <span
                                className={`chat-plan-step-state status-${step.status || "pending"}`}
                              >
                                {stepStatusLabel}
                              </span>
                            </Box>
                            {stepCopy.description ? (
                              <Typography
                                variant="caption"
                                className="chat-plan-step-detail"
                              >
                                {stepCopy.description}
                              </Typography>
                            ) : null}
                            {step.substeps.length > 0 ? (
                              <Stack
                                spacing={0.55}
                                className="chat-plan-substeps"
                              >
                                {step.substeps.map((substep) => {
                                  const isSubCompleted =
                                    substep.status === "completed";
                                  const isSubRunning =
                                    substep.status === "running";
                                  const isSubFailed =
                                    substep.status === "failed";
                                  const substepStatusLabel =
                                    executionPlanStepStatusLabel(
                                      substep.status,
                                    );
                                  return (
                                    <Box
                                      key={`${step.id}:${substep.id}`}
                                      className={`chat-plan-substep${isSubCompleted ? " done" : ""}${isSubRunning ? " running" : ""}${isSubFailed ? " failed" : ""}`}
                                    >
                                      <Box className="chat-plan-substep-marker">
                                        {renderExecutionPlanStatusIcon(
                                          substep.status,
                                          isSubRunning ? "spin" : "",
                                        )}
                                      </Box>
                                      <Box className="chat-plan-substep-copy">
                                        <Typography
                                          variant="caption"
                                          className="chat-plan-substep-title"
                                        >
                                          {step.id}.{substep.id} {substep.title}
                                        </Typography>
                                        <span
                                          className={`chat-plan-step-state chat-plan-substep-state status-${substep.status || "pending"}`}
                                        >
                                          {substepStatusLabel}
                                        </span>
                                      </Box>
                                    </Box>
                                  );
                                })}
                              </Stack>
                            ) : null}
                          </Box>
                        </Box>
                      );
                    })}
                  </Stack>
                </AccordionDetails>
              </Accordion>
            ) : null}
            {!showEmptyHero ? (
              <Box
                className={`chat-composer-shell${shouldShowExecutionPlanWarning ? " has-plan" : ""}`}
              >
                {shouldShowExecutionPlanWarning ? (
                  <Box className="chat-composer-plan-warning">
                    <Stack spacing={1}>
                      <Stack
                        direction="row"
                        spacing={1.1}
                        sx={{
                          alignItems: "flex-start",
                        }}
                      >
                        <Box className="chat-composer-plan-icon status-failed">
                          {renderExecutionPlanStatusIcon("failed")}
                        </Box>
                        <Box sx={{ minWidth: 0, flex: 1 }}>
                          <Stack
                            direction="row"
                            spacing={0.8}
                            useFlexGap
                            sx={{
                              alignItems: "center",
                              flexWrap: "wrap",
                            }}
                          >
                            <Typography
                              variant="caption"
                              className="chat-composer-plan-kicker"
                            >
                              Planner
                            </Typography>
                            <Typography
                              variant="caption"
                              className="chat-composer-plan-status"
                            >
                              Planner offline
                            </Typography>
                          </Stack>
                          <Typography
                            variant="caption"
                            className="chat-composer-plan-summary"
                          >
                            {visibleExecutionPlanFailure}
                          </Typography>
                        </Box>
                      </Stack>
                      {searchSetupActionNeeded ? (
                        <Box>
                          <Button
                            size="small"
                            variant="outlined"
                            onClick={() => onNavigateToView?.("search")}
                          >
                            Open Search Settings
                          </Button>
                        </Box>
                      ) : null}
                    </Stack>
                  </Box>
                ) : null}
                {renderComposerInput(composerPlaceholder)}
              </Box>
            ) : null}
          </Box>
        </Box>
      </Box>
      {showWorkspacePanelInline ? (
        <Box sx={{ minHeight: 0, display: { xs: "none", lg: "contents" } }}>
          {CHAT_LAYOUT_MODE === "split"
            ? renderComputerPaneContent()
            : renderActivityPanelContent()}
        </Box>
      ) : null}
      <Drawer
        anchor="left"
        open={showConversationSidebarDrawer}
        onClose={() => setConversationSidebarOpen(false)}
        ModalProps={{ keepMounted: true }}
        slotProps={{
          paper: { className: "chat-mobile-drawer chat-mobile-drawer-left" },
        }}
      >
        {renderConversationSidebarContent(true)}
      </Drawer>
      <Drawer
        anchor="right"
        open={showWorkspacePanelDrawer}
        onClose={() => setWorkspaceOpen(false)}
        ModalProps={{ keepMounted: true }}
        slotProps={{
          paper: { className: "chat-mobile-drawer chat-mobile-drawer-right" },
        }}
      >
        {CHAT_LAYOUT_MODE === "split"
          ? renderComputerPaneContent(true)
          : renderActivityPanelContent(true)}
      </Drawer>
      <Dialog
        open={!!activityDetailRow}
        onClose={() => setActivityDetailRow(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: { className: "activity-detail-dialog" },
        }}
      >
        <DialogTitle className="activity-detail-title">
          <Stack
            direction="row"
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: "flex-start",
            }}
          >
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="subtitle1" className="activity-detail-heading">
                {activityDetailRow?.label || "Activity details"}
              </Typography>
              <Typography variant="caption" className="activity-detail-meta">
                {[
                  activityDetailRow?.kind
                    ? activityKindDisplayLabel(activityDetailRow.kind)
                    : "",
                  activityDetailRow?.stepType
                    ? activityDetailRow.stepType.replace(/[_-]+/g, " ")
                    : "",
                  activityDetailRow?.time
                    ? formatTraceStepTime(activityDetailRow.time)
                    : "",
                ]
                  .filter(Boolean)
                  .join(" | ")}
              </Typography>
            </Box>
            <IconButton size="small" onClick={() => setActivityDetailRow(null)}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Stack>
        </DialogTitle>
        <DialogContent dividers className="activity-detail-content">
          {activityDetailRow ? (
            <Stack spacing={1.25}>
              {activityDetailRow.summary ? (
                <Box className="activity-detail-section">
                  <Typography variant="caption" className="activity-detail-label">
                    Summary
                  </Typography>
                  <Typography variant="body2" className="activity-detail-copy">
                    {activityDetailRow.summary}
                  </Typography>
                </Box>
              ) : null}
              {activityDetailRow.rawTitle &&
              activityDetailRow.rawTitle !== activityDetailRow.label ? (
                <Box className="activity-detail-section">
                  <Typography variant="caption" className="activity-detail-label">
                    Source title
                  </Typography>
                  <Typography variant="body2" className="activity-detail-copy">
                    {activityDetailRow.rawTitle}
                  </Typography>
                </Box>
              ) : null}
              {activityDetailRow.rawDetailFull ? (
                <Box className="activity-detail-section">
                  <Typography variant="caption" className="activity-detail-label">
                    Detail
                  </Typography>
                  {activityDetailReadableDetail ? (
                    <Typography variant="body2" className="activity-detail-copy">
                      {activityDetailReadableDetail}
                    </Typography>
                  ) : (
                    <Box component="pre" className="activity-detail-pre">
                      {activityDetailRow.rawDetailFull}
                    </Box>
                  )}
                </Box>
              ) : null}
              {activityDetailRow.payloadView ? (
                <ActivityPayloadDisclosure
                  payload={activityDetailRow.payloadView}
                  expanded={expandedActivityPayloads.has(
                    activityDetailPayloadKey,
                  )}
                  onToggle={() =>
                    toggleExpandedActivityPayload(activityDetailPayloadKey)
                  }
                  controlsId={activityDetailPayloadKey || "activity-detail"}
                />
              ) : null}
            </Stack>
          ) : null}
        </DialogContent>
      </Dialog>
      {/* Code Viewer Dialog */}
      <Dialog
        open={codeViewerOpen && isShowingSnippetPreview}
        onClose={() => setCodeViewerOpen(false)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: { className: "code-viewer-dialog" },
        }}
      >
        <DialogTitle
          sx={{
            p: "10px 16px",
            borderBottom: "1px solid var(--ui-rgba-100-160-230-180)",
          }}
        >
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
            }}
          >
            <Box>
              <Typography variant="subtitle1" sx={{ fontWeight: 600 }}>
                Assistant Snippets
              </Typography>
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                }}
              >
                {activeWorkspaceCodeSourceLabel}
              </Typography>
            </Box>
            <IconButton size="small" onClick={() => setCodeViewerOpen(false)}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Stack>
          {workspaceSnippetFiles.length > 1 ? (
            <Box className="code-file-tabs" sx={{ mt: 0.5 }}>
              {workspaceSnippetFiles.map((snippet) => (
                <button
                  key={snippet.id}
                  className={`code-file-tab${activeSnippetFile?.id === snippet.id ? " code-file-tab-active" : ""}`}
                  onClick={() => setSelectedSnippetId(snippet.id)}
                >
                  {snippet.displayName}
                </button>
              ))}
            </Box>
          ) : null}
        </DialogTitle>
        <DialogContent sx={{ p: 0 }}>
          {activeWorkspaceCodeEntry && (
            <>
              <pre className="code-viewer-pre">
                <code>{activeWorkspaceCodeLines}</code>
              </pre>
              <Box sx={{ px: 1.5, pb: 1 }}>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Referenced from {activeWorkspaceCodeSourceLabel}.
                </Typography>
              </Box>
            </>
          )}
        </DialogContent>
      </Dialog>
      <Dialog
        open={previewDialogOpen}
        onClose={() => setPreviewDialogOpen(false)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle
          sx={{
            p: "10px 16px",
            borderBottom: "1px solid var(--ui-rgba-100-160-230-180)",
          }}
        >
          Deployment Preview
        </DialogTitle>
        <DialogContent sx={{ p: 2 }}>
          <Stack spacing={1}>
            <Typography variant="body2">
              Local:{" "}
              <Link
                href={previewUrl}
                target="_blank"
                rel="noopener noreferrer"
                underline="hover"
              >
                {previewUrl}
              </Link>
            </Typography>
            {publicPreviewUrl ? (
              <Typography variant="body2">
                {workspaceTunnelMeta.isPrivate ? "Private access:" : "Public:"}{" "}
                <Link
                  href={publicPreviewUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  underline="hover"
                >
                  {publicPreviewUrl}
                </Link>
              </Typography>
            ) : null}
            {previewImageUrl ? (
              <Box
                component="img"
                src={previewImageUrl}
                alt="Deployed app screenshot"
                sx={{
                  width: "100%",
                  borderRadius: 1,
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "var(--ui-rgba-20-20-24-880)",
                }}
              />
            ) : (
              <Alert severity="info">
                No screenshot is available yet for this deployment.
              </Alert>
            )}
          </Stack>
        </DialogContent>
      </Dialog>
      <Dialog
        open={!!researchReportDialog}
        onClose={() => setResearchReportDialog(null)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: { className: "chat-research-report-dialog" },
        }}
      >
        <DialogTitle className="chat-research-report-dialog-title">
          <Stack
            direction="row"
            spacing={1.5}
            sx={{
              justifyContent: "space-between",
              alignItems: "flex-start",
            }}
          >
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <Typography
                variant="subtitle1"
                className="chat-research-report-dialog-heading"
              >
                {researchReportDialog?.report.title || "Research report"}
              </Typography>
              <Typography
                variant="caption"
                className="chat-research-report-dialog-meta"
              >
                {researchReportDialog
                  ? researchReportMetaLabel(researchReportDialog.report)
                  : "Research report"}
              </Typography>
            </Box>
            <Stack direction="row" spacing={0.5}>
              <Tooltip title="Export as PDF">
                <span>
                  <IconButton
                    size="small"
                    onClick={() => {
                      if (!researchReportDialog) return;
                      exportResearchReportPdf({
                        report: researchReportDialog.report,
                        headingHint: researchReportDialog.report.title,
                        previousUserPrompt:
                          researchReportDialog.previousUserPrompt,
                        timestamp: researchReportDialog.timestamp,
                        traceId: researchReportDialog.traceId,
                      });
                    }}
                  >
                    <PictureAsPdfRoundedIcon fontSize="small" />
                  </IconButton>
                </span>
              </Tooltip>
              <Tooltip title="Download HTML report">
                <span>
                  <IconButton
                    size="small"
                    onClick={() => {
                      if (!researchReportDialog) return;
                      downloadResearchReportHtml({
                        report: researchReportDialog.report,
                        headingHint: researchReportDialog.report.title,
                        previousUserPrompt:
                          researchReportDialog.previousUserPrompt,
                        timestamp: researchReportDialog.timestamp,
                        traceId: researchReportDialog.traceId,
                      });
                    }}
                  >
                    <ArticleRoundedIcon fontSize="small" />
                  </IconButton>
                </span>
              </Tooltip>
              <IconButton
                size="small"
                onClick={() => setResearchReportDialog(null)}
              >
                <CloseIcon fontSize="small" />
              </IconButton>
            </Stack>
          </Stack>
        </DialogTitle>
        <DialogContent dividers className="chat-research-report-dialog-content">
          {researchReportDialog ? (
            <Box className="chat-research-report-paper">
              {renderChatMarkdown(
                researchReportExportMarkdown({
                  report: researchReportDialog.report,
                  previousUserPrompt: researchReportDialog.previousUserPrompt,
                  timestamp: researchReportDialog.timestamp,
                  traceId: researchReportDialog.traceId,
                  preserveChartFences: true,
                }),
                {
                  snippetNamespace: `research-report-${researchReportDialog.messageId || "dialog"}-full`,
                  onOpenSnippet: openCodePreviewInWorkspace,
                },
              )}
            </Box>
          ) : null}
        </DialogContent>
        <DialogActions className="chat-research-report-dialog-actions">
          <Button
            variant="outlined"
            onClick={() => {
              if (!researchReportDialog) return;
              exportResearchReportPdf({
                report: researchReportDialog.report,
                headingHint: researchReportDialog.report.title,
                previousUserPrompt: researchReportDialog.previousUserPrompt,
                timestamp: researchReportDialog.timestamp,
                traceId: researchReportDialog.traceId,
              });
            }}
          >
            Export PDF
          </Button>
          <Button
            variant="outlined"
            onClick={() => {
              if (!researchReportDialog) return;
              downloadResearchReportHtml({
                report: researchReportDialog.report,
                headingHint: researchReportDialog.report.title,
                previousUserPrompt: researchReportDialog.previousUserPrompt,
                timestamp: researchReportDialog.timestamp,
                traceId: researchReportDialog.traceId,
              });
            }}
          >
            Download HTML
          </Button>
          <Button
            variant="contained"
            onClick={() => setResearchReportDialog(null)}
          >
            Close
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}

const ChatPage = memo(ChatPageInner);
ChatPage.displayName = "ChatPage";

export default ChatPage;
function formatTraceDuration(durationMs: unknown): string {
  const ms = num(durationMs, -1);
  if (ms < 0) return "pending";
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60)
    return `${totalSeconds >= 10 ? totalSeconds.toFixed(0) : totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  return `${minutes}m ${seconds}s`;
}

function buildEvolutionFocusCaseLabel(row: JsonRecord): string {
  const surface = str(row.surface, "case").trim();
  const delta = num(row.score_delta, Number.NaN);
  const preview = str(row.prompt_preview, "").trim();
  const invalidBefore = toBool(row.invalid_json_before);
  const invalidAfter = toBool(row.invalid_json_after);
  const parts = [surface];
  if (Number.isFinite(delta))
    parts.push(`${delta >= 0 ? "+" : ""}${(delta * 100).toFixed(0)} pts`);
  if (invalidBefore !== invalidAfter)
    parts.push(invalidAfter ? "JSON regressed" : "JSON stabilized");
  if (preview) parts.push(preview);
  return parts.join(" / ");
}

function traceStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" {
  switch (status.trim().toLowerCase()) {
    case "completed":
      return "success";
    case "warning":
      return "warning";
    case "failed":
      return "error";
    default:
      return "default";
  }
}

function traceStepColor(
  stepType: string,
): "default" | "success" | "warning" | "error" {
  switch (stepType.trim().toLowerCase()) {
    case "success":
      return "success";
    case "warning":
      return "warning";
    case "error":
      return "error";
    default:
      return "default";
  }
}

function formatTraceData(value: unknown): string {
  if (typeof value !== "string") return str(value, "");
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

type TraceEvidenceItem = {
  title: string;
  detail: string;
  type: string;
};

type TraceStepConsoleView = {
  detail: string;
  dataText: string;
};

function pickTraceStepArtifacts(step: JsonRecord): JsonRecord[] {
  return pickRecords(step, "artifacts");
}

function traceArtifactLabel(artifact: JsonRecord): string {
  const explicit = str(artifact.label, "").trim();
  if (explicit) return explicit;
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

function traceArtifactKindLabel(artifact: JsonRecord): string {
  const kind = str(artifact.kind, "").trim();
  return kind ? titleCaseLabel(kind) : "Artifact";
}

function traceArtifactFormat(artifact: JsonRecord): string {
  return str(artifact.format, "").trim().toUpperCase();
}

function traceArtifactBody(artifact: JsonRecord): string {
  const raw = artifact.data;
  if (typeof raw === "string") return formatTraceData(raw);
  if (raw == null) return "";
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return str(raw, "");
  }
}

function traceArtifactSummary(artifact: JsonRecord): string {
  const explicit = str(artifact.summary, "").trim();
  if (explicit) return explicit;
  const body = collapseInlineWhitespace(traceArtifactBody(artifact));
  return body ? truncateTraceEvidence(body, 180) : "";
}

function traceArtifactChipLabel(artifact: JsonRecord): string {
  const label = traceArtifactLabel(artifact);
  const summary = traceArtifactSummary(artifact);
  if (summary && summary.length <= 56) {
    return `${label}: ${summary}`;
  }
  return label;
}

function summarizeTraceArtifactsInline(artifacts: JsonRecord[]): string {
  return uniqueNonEmptyStrings(
    artifacts.map(
      (artifact) =>
        traceArtifactSummary(artifact) || traceArtifactLabel(artifact),
    ),
  )
    .slice(0, 2)
    .map((value) => truncateTraceEvidence(value, 120))
    .join(" | ");
}

function buildTraceArtifactBlocks(artifacts: JsonRecord[]): string {
  return artifacts
    .map((artifact) => {
      const label = traceArtifactLabel(artifact);
      const format = traceArtifactFormat(artifact);
      const summary = traceArtifactSummary(artifact);
      const body = traceArtifactBody(artifact);
      const lines = [
        format ? `${label} (${format})` : label,
        summary ? `Summary: ${summary}` : "",
        body,
      ].filter(Boolean);
      return lines.join("\n");
    })
    .filter(Boolean)
    .join("\n\n");
}

function truncateTraceEvidence(value: string, max = 240): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, Math.max(0, max - 3)).trimEnd()}...`;
}

function summarizeTraceOutcome(trace: JsonRecord): string {
  const status = str(trace.status, "").trim().toLowerCase();
  if (status === "completed") {
    return `Outcome: completed successfully in ${formatTraceDuration(trace.duration_ms)}`;
  }
  if (status === "failed" || status === "error" || status === "warning") {
    return `Outcome: failed after ${formatTraceDuration(trace.duration_ms)}`;
  }
  if (!status || status === "running") {
    return "Outcome: still running";
  }
  return `Outcome: ${status}`;
}

function isExecutionProofStep(step: JsonRecord): boolean {
  const combined =
    `${str(step.title, "")}\n${str(step.detail, "")}\n${formatTraceData(step.data)}`.toLowerCase();
  return /execution record saved|execution proof generated|verification id:|proof id:/.test(
    combined,
  );
}

function buildTraceEvidenceItems(steps: JsonRecord[]): TraceEvidenceItem[] {
  return steps
    .map((step) => {
      const title = str(step.title, "").trim();
      const detail = str(step.detail, "").trim();
      const dataText = formatTraceData(step.data);
      const type = str(step.type, str(step.step_type, "step")).trim() || "step";
      const combined = `${title}\n${detail}\n${dataText}`.toLowerCase();
      if (!title && !detail && !dataText) return null;
      if (
        /execution record saved|execution proof generated|verification id:|proof id:/.test(
          combined,
        )
      )
        return null;
      if (
        /memory available|context packing|selected the best available model|using a direct execution strategy|prepared the next response/.test(
          combined,
        ) &&
        !dataText
      ) {
        return null;
      }
      const summary = detail || dataText || title;
      return {
        title: title || "Step",
        detail: truncateTraceEvidence(summary),
        type,
      };
    })
    .filter((item): item is TraceEvidenceItem => Boolean(item))
    .slice(-4);
}

function extractTraceArtifacts(
  trace: JsonRecord,
  steps: JsonRecord[],
): string[] {
  const artifactLabels = uniqueNonEmptyStrings(
    steps.flatMap((step) =>
      pickTraceStepArtifacts(step).map((artifact) =>
        traceArtifactChipLabel(artifact),
      ),
    ),
  );
  if (artifactLabels.length > 0) {
    return artifactLabels.slice(0, 6);
  }
  const sources = [
    str(trace.response, ""),
    ...steps.flatMap((step) => [
      str(step.detail, ""),
      formatTraceData(step.data),
    ]),
  ];
  const found = new Set<string>();
  for (const source of sources) {
    const text = source.trim();
    if (!text) continue;
    for (const match of text.match(/https?:\/\/[^\s"'<>]+/g) || []) {
      found.add(match);
    }
    for (const match of text.match(/[A-Za-z]:\\[^\s"'<>]+/g) || []) {
      found.add(match);
    }
  }
  return Array.from(found).slice(0, 6);
}

function buildExecutionProofConsoleEvidence(
  trace: JsonRecord,
  steps: JsonRecord[],
): string {
  const lines: string[] = [];
  const action = truncateTraceEvidence(str(trace.message, ""), 180);
  const outcome = summarizeTraceOutcome(trace);
  const finalResult = truncateTraceEvidence(str(trace.response, ""), 220);
  const artifacts = extractTraceArtifacts(trace, steps).slice(0, 3);
  const keyEvidence = buildTraceEvidenceItems(steps).slice(-3);

  if (action) lines.push(`Action attempted: ${action}`);
  lines.push(outcome);
  if (finalResult) lines.push(`Final result: ${finalResult}`);
  if (artifacts.length > 0) {
    lines.push(`Artifacts or outputs: ${artifacts.join(" | ")}`);
  }
  for (const item of keyEvidence) {
    lines.push(`${item.title}: ${item.detail}`);
  }
  lines.push(
    "Open Trace Detail for the verification record and the full evidence.",
  );

  return lines.join("\n");
}

function buildTraceStepConsoleView(
  trace: JsonRecord,
  steps: JsonRecord[],
  step: JsonRecord,
): TraceStepConsoleView {
  const detail = str(step.detail, "").trim();
  const rawDataText = formatTraceData(step.data);
  const artifactText = buildTraceArtifactBlocks(pickTraceStepArtifacts(step));
  const dataText =
    artifactText && rawDataText.trim() === artifactText.trim()
      ? artifactText
      : [rawDataText, artifactText].filter(Boolean).join("\n\n");

  if (!isExecutionProofStep(step)) {
    return { detail, dataText };
  }

  return {
    detail:
      "Verifiable execution record saved. The evidence for this run is summarized below.",
    dataText: buildExecutionProofConsoleEvidence(trace, steps),
  };
}

function parseTraceDataRecord(value: unknown): JsonRecord {
  if (isRecord(value)) return value;
  if (typeof value !== "string") return {};
  const trimmed = value.trim();
  if (!trimmed) return {};
  try {
    return asRecord(JSON.parse(trimmed));
  } catch {
    return {};
  }
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => str(item, "").trim()).filter(Boolean);
}

function promotionGateSummary(data: JsonRecord): string {
  const report = asRecord(data.promotion_gate_report);
  return (
    str(report.summary, "").trim() ||
    str(data.promotion_gate_summary, "").trim() ||
    str(data.promotion_gate, "").trim()
  );
}

function percentageLabel(value: unknown, digits = 1): string {
  const parsed = num(value, Number.NaN);
  if (!Number.isFinite(parsed)) return "";
  return `${(parsed * 100).toFixed(digits)}%`;
}

type EvolutionReviewCard = {
  key: string;
  title: string;
  status: string;
  detail: string;
  chips: string[];
  rationale?: string;
  example?: string;
  evidence?: string;
};

type EvolutionPatternCard = EvolutionReviewCard & {
  runs: JsonRecord[];
  latestSeen?: string;
  toolSummary?: string;
  completedCount: number;
  failedCount: number;
  acceptedCount: number;
};

function collapseInlineWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function truncateUiText(value: string, maxChars = 120): string {
  const normalized = collapseInlineWhitespace(value);
  if (normalized.length <= maxChars) return normalized;
  return `${normalized.slice(0, Math.max(0, maxChars - 1)).trimEnd()}...`;
}

function titleCaseLabel(value: string): string {
  return value
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(" ");
}

function learningEvidenceStatusColor(
  status: string,
): "default" | "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (
    normalized === "completed" ||
    normalized === "success" ||
    normalized === "succeeded"
  )
    return "success";
  if (normalized === "failed" || normalized === "error") return "error";
  if (normalized === "accepted") return "info";
  if (normalized === "repeating successfully") return "success";
  if (normalized === "mixed results") return "warning";
  if (normalized === "needs review") return "error";
  if (normalized === "user preference captured" || normalized === "seen once")
    return "info";
  return "default";
}

function learningEvidenceTimestampMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function latestLearningEvidenceTimestamp(runs: JsonRecord[]): number {
  return runs.reduce(
    (latest, run) =>
      Math.max(latest, learningEvidenceTimestampMs(str(run.created_at, ""))),
    0,
  );
}

function learningEvidenceToolLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "schedule_task") return "Scheduled task";
  if (normalized === "calendar_create") return "Calendar event";
  return titleCaseLabel(normalized);
}

function summarizeLearningEvidenceTools(values: string[]): string {
  if (values.length === 0) return "";
  const counts = new Map<string, number>();
  values.forEach((value) => {
    const key = value.trim().toLowerCase();
    if (!key) return;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  });
  return Array.from(counts.entries())
    .sort(
      (left, right) => right[1] - left[1] || left[0].localeCompare(right[0]),
    )
    .slice(0, 3)
    .map(
      ([name, count]) =>
        `${learningEvidenceToolLabel(name)}${count > 1 ? ` x${count}` : ""}`,
    )
    .join(", ");
}

function uniqueNonEmptyStrings(values: Array<unknown>): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  values.forEach((value) => {
    const normalized = collapseInlineWhitespace(str(value, "").trim());
    if (!normalized || seen.has(normalized)) return;
    seen.add(normalized);
    out.push(normalized);
  });
  return out;
}

function summarizeEvolutionPatternRun(run: JsonRecord): string {
  const decision = asRecord(run.decision_summary);
  const executionStatus = asRecord(run.execution_status);
  const summary = collapseInlineWhitespace(
    str(
      run.failure_reason,
      str(
        run.outcome_summary,
        str(
          decision.summary,
          str(decision.completion_status, str(executionStatus.status, "")),
        ),
      ),
    ),
  );
  return summary || "No summary recorded.";
}

function evolutionPatternStatusExplanation(card: EvolutionPatternCard): string {
  const normalized = card.status.trim().toLowerCase();
  if (normalized === "user preference captured") {
    return "A direct user correction was recorded here. That kind of signal is strong and can steer similar requests in the future.";
  }
  if (normalized === "repeating successfully") {
    return "Similar requests have been finishing through the same path. AgentArk is checking whether that path is stable enough to reuse by default.";
  }
  if (normalized === "mixed results") {
    return "Similar requests have both worked and failed. AgentArk is comparing the difference before changing behavior.";
  }
  if (normalized === "needs review") {
    return "Similar requests are running into issues often enough that AgentArk may need a guardrail or a different route.";
  }
  if (normalized === "seen once") {
    return "This has only been seen once so far. One example is useful context, but not enough to change behavior yet.";
  }
  return "AgentArk is collecting a few more examples before it decides whether this pattern should change how future requests are handled.";
}

function normalizeLearningEvidenceState(run: JsonRecord): string {
  const decision = asRecord(run.decision_summary);
  return collapseInlineWhitespace(
    str(
      run.correction_state,
      str(decision.completion_status, str(run.success_state, "observed")),
    ),
  ).toLowerCase();
}

function inferLearningEvidenceTitle(
  runs: JsonRecord[],
  requestPreview: string,
  taskType: string,
): string {
  const combinedRequest = requestPreview.toLowerCase();
  const allTools = new Set<string>();
  runs.forEach((run) => {
    stringList(run.tool_names).forEach((toolName) => {
      const normalized = toolName.trim().toLowerCase();
      if (normalized) allTools.add(normalized);
    });
  });

  if (
    /dont use|don't use|do not use/.test(combinedRequest) &&
    /(calendar|calender)/.test(combinedRequest)
  ) {
    return "Operator correction: avoid Calendar";
  }
  if (
    /meeting/.test(combinedRequest) &&
    /(notify|remind|arrive)/.test(combinedRequest)
  ) {
    return "Meeting-date reminder requests";
  }
  if (
    /(notify|remind)/.test(combinedRequest) &&
    /(date|today|tomorrow|january|february|march|april|may|june|july|august|september|october|november|december|\d{4})/.test(
      combinedRequest,
    )
  ) {
    return "Date-based reminder requests";
  }
  if (
    allTools.has("calendar_create") ||
    /(calendar|event)/.test(combinedRequest)
  ) {
    return "Calendar event creation requests";
  }
  if (allTools.has("schedule_task")) {
    return "Scheduled follow-up requests";
  }
  if (taskType) return `${titleCaseLabel(taskType)} requests`;
  if (requestPreview) return truncateUiText(requestPreview, 80);
  return "Observed request pattern";
}

function buildEvolutionReviewCards(steps: JsonRecord[]): EvolutionReviewCard[] {
  const cards: EvolutionReviewCard[] = [];
  steps.forEach((step, idx) => {
    const data = parseTraceDataRecord(step.data);
    const traceKind = str(data.trace_kind, "").trim().toLowerCase();
    if (!traceKind.startsWith("self_evolve.")) return;

    const status = str(step.type, str(step.step_type, "info")).trim() || "info";
    const title = str(step.title, "Evolve").trim();
    const detail = str(step.detail, "").trim();
    const chips: string[] = [];
    const evidence: string[] = [];
    let rationale = "";

    if (traceKind === "self_evolve.request") {
      const mode = str(data.mode, "policy");
      const request = str(data.request, "").trim();
      chips.push(`Mode ${mode}`);
      if (toBool(data.apply_promotion)) chips.push("Promotion enabled");
      const canaryRollout = num(data.canary_rollout_percent, -1);
      if (canaryRollout > 0) chips.push(`Canary ${canaryRollout}%`);
      rationale = request;
    } else if (traceKind === "self_evolve.policy.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineAccuracy = percentageLabel(data.baseline_accuracy, 0);
      const candidateAccuracy = percentageLabel(
        data.best_candidate_accuracy,
        0,
      );
      const gain = num(data.accuracy_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const changedFields = stringList(data.changed_fields);
      const notes = stringList(data.notes);
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineAccuracy || candidateAccuracy) {
        chips.push(`${baselineAccuracy || "?"} -> ${candidateAccuracy || "?"}`);
      }
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      if (changedFields.length)
        evidence.push(`Changed fields: ${changedFields.join(", ")}`);
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.policy.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      chips.push(`Promotion ${promotionMode}`);
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Replay samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Replay gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (traceKind === "self_evolve.prompt.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineScore = percentageLabel(data.baseline_score, 0);
      const candidateScore = percentageLabel(data.best_candidate_score, 0);
      const gain = num(data.score_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      const notes = stringList(data.notes);
      const diffSummary = asRecord(data.diff_summary);
      const routerChanged = stringList(diffSummary.router_changed_fields);
      const primaryResponseChanged = stringList(
        diffSummary.primary_response_changed_fields,
      );
      const synthesisChanged = stringList(
        diffSummary.delegation_synthesis_changed_fields,
      );
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineScore || candidateScore) {
        chips.push(`${baselineScore || "?"} -> ${candidateScore || "?"}`);
      }
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (routerChanged.length)
        evidence.push(`Router changes: ${routerChanged.join(", ")}`);
      if (primaryResponseChanged.length)
        evidence.push(
          `Primary response changes: ${primaryResponseChanged.join(", ")}`,
        );
      if (synthesisChanged.length)
        evidence.push(`Synthesis changes: ${synthesisChanged.join(", ")}`);
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      const invalidRateBefore = num(
        data.baseline_router_invalid_json_rate,
        Number.NaN,
      );
      const invalidRateAfter = num(
        data.candidate_router_invalid_json_rate,
        Number.NaN,
      );
      if (
        Number.isFinite(invalidRateBefore) &&
        Number.isFinite(invalidRateAfter)
      ) {
        evidence.push(
          `Router invalid JSON: ${(invalidRateBefore * 100).toFixed(1)}% -> ${(invalidRateAfter * 100).toFixed(1)}%`,
        );
      }
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.prompt.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      chips.push(`Promotion ${promotionMode}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Experience samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Experience gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (traceKind === "self_evolve.specialist_prompt.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineScore = percentageLabel(data.baseline_score, 0);
      const candidateScore = percentageLabel(data.best_candidate_score, 0);
      const gain = num(data.score_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      const notes = stringList(data.notes);
      const diffSummary = asRecord(data.diff_summary);
      const changedItems = stringList(diffSummary.changed_surfaces).concat(
        stringList(diffSummary.changed_roles),
      );
      const changePreview = stringList(diffSummary.change_preview);
      const focusCases = pickRecords(data, "focus_cases");
      chips.push(
        `${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`,
      );
      if (baselineScore || candidateScore)
        chips.push(`${baselineScore || "?"} -> ${candidateScore || "?"}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (Number.isFinite(gain))
        chips.push(
          `Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`,
        );
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${promotionGateSummary(data) || "unknown"}`;
      if (changedItems.length)
        evidence.push(`Changed: ${changedItems.join(", ")}`);
      if (changePreview.length)
        evidence.push(`Preview: ${changePreview.join(" | ")}`);
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(
          `Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`,
        );
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue))
        evidence.push(`P-value: ${pValue.toFixed(4)}`);
      if (focusCases.length) {
        evidence.push(
          `Focus cases: ${focusCases.slice(0, 3).map(buildEvolutionFocusCaseLabel).join(" | ")}`,
        );
      }
      if (notes.length) evidence.push(`Why: ${notes.join(" | ")}`);
      const lineageId = str(data.lineage_entry_id, "").trim();
      if (lineageId) evidence.push(`Lineage: ${lineageId}`);
    } else if (traceKind === "self_evolve.specialist_prompt.promotion") {
      const promotionMode = str(data.promotion_mode, "none").trim();
      const canaryState = asRecord(data.canary_state);
      const replay = asRecord(data.replay_evaluation);
      const optimizedSurfaces = stringList(data.optimized_surfaces);
      chips.push(`Promotion ${promotionMode}`);
      if (optimizedSurfaces.length) chips.push(optimizedSurfaces.join(" + "));
      if (toBool(data.promotion_applied)) chips.push("Applied");
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) chips.push(`Rollout ${rollout}%`);
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(
          `Experience samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`,
        );
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain))
        evidence.push(`Experience gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (
      traceKind === "self_evolve.manual_action.result" ||
      traceKind === "self_evolve.manual_action.request"
    ) {
      const action = humanizeMachineLabel(str(data.action, ""), "");
      const canaryState = asRecord(data.canary_state);
      chips.push(action || "Manual action");
      if (Object.keys(canaryState).length > 0) {
        chips.push(
          toBool(canaryState.enabled) ? "Canary enabled" : "Canary disabled",
        );
      }
      rationale = str(data.message, detail).trim();
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(
          `Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`,
        );
      }
      const rollout = num(canaryState.rollout_percent, -1);
      if (rollout > 0) evidence.push(`Rollout: ${rollout}%`);
    }

    cards.push({
      key: `${traceKind}-${idx}`,
      title,
      status,
      detail,
      chips,
      rationale: rationale || undefined,
      evidence: evidence.join("\n") || undefined,
    });
  });
  return cards;
}

function evolutionTraceIdHint(payload: unknown): string {
  const traceId = str(asRecord(payload).trace_id, "").trim();
  return traceId ? ` Trace ${traceId.slice(0, 8)} recorded.` : "";
}

function syncRunStatusColor(
  status: string,
): "success" | "warning" | "error" | "default" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "completed") return "success";
  if (normalized === "failed") return "error";
  if (normalized === "blocked") return "warning";
  return "default";
}

function syncRunTriggerLabel(trigger: string): string {
  const normalized = trigger.trim().toLowerCase();
  if (normalized === "manual") return "Manual";
  if (normalized === "background") return "Background";
  return humanizeMachineLabel(normalized, "Unknown");
}

type TraceRange = "1h" | "6h" | "24h" | "7d" | "14d" | "30d";
const TRACE_RANGE_PRESETS: {
  value: TraceRange;
  label: string;
  hours: number;
}[] = [
  { value: "1h", label: "1 hour", hours: 1 },
  { value: "6h", label: "6 hours", hours: 6 },
  { value: "24h", label: "24 hours", hours: 24 },
  { value: "7d", label: "7 days", hours: 168 },
  { value: "14d", label: "14 days", hours: 336 },
  { value: "30d", label: "30 days", hours: 720 },
];

function traceRangeHours(range: TraceRange): number {
  return TRACE_RANGE_PRESETS.find((p) => p.value === range)?.hours || 168;
}

function traceRangeSinceISO(range: TraceRange): string {
  const ms = traceRangeHours(range) * 3600 * 1000;
  return new Date(Date.now() - ms).toISOString();
}

type TraceBucket = { label: string; ts: number };

function buildTraceTrendBuckets(range: TraceRange): TraceBucket[] {
  const hours = traceRangeHours(range);
  const bucketCount = Math.min(
    hours <= 6 ? hours : hours <= 24 ? 12 : hours <= 168 ? 7 : 14,
    14,
  );
  const spanMs = hours * 3600 * 1000;
  const bucketMs = spanMs / bucketCount;
  const now = Date.now();
  const buckets: TraceBucket[] = [];
  for (let i = 0; i < bucketCount; i++) {
    const ts = now - spanMs + (i + 1) * bucketMs;
    const label =
      hours <= 24
        ? formatUiTime(ts, { fallback: "-", hour12: false })
        : formatUiDateOnly(ts, { fallback: "-" });
    buckets.push({ label, ts });
  }
  return buckets;
}

function bucketizeTraceItems<T>(
  items: T[],
  getTs: (item: T) => string,
  buckets: TraceBucket[],
): number[] {
  const counts = new Array(buckets.length).fill(0) as number[];
  for (const item of items) {
    const ts = new Date(getTs(item)).getTime();
    if (!ts || isNaN(ts)) continue;
    for (let i = buckets.length - 1; i >= 0; i--) {
      const lo = i === 0 ? 0 : buckets[i - 1].ts;
      if (ts > lo && ts <= buckets[i].ts) {
        counts[i]++;
        break;
      }
    }
  }
  return counts;
}

function traceSecurityEventTypeLabel(eventType: string): string {
  const normalized = (eventType || "").trim().toLowerCase();
  return humanizeMachineLabel(normalized, "Unknown");
}


function buildEvolutionEvidenceCards(
  runs: JsonRecord[],
): EvolutionPatternCard[] {
  const groupedRuns = new Map<string, JsonRecord[]>();
  runs.forEach((run, idx) => {
    const requestText = collapseInlineWhitespace(str(run.request_text, ""));
    const intentKey = collapseInlineWhitespace(str(run.intent_key, ""));
    const key = requestText
      ? `request:${requestText.toLowerCase()}`
      : intentKey
        ? `intent:${intentKey.toLowerCase()}`
        : `run:${str(run.id, String(idx))}`;
    const existing = groupedRuns.get(key);
    if (existing) {
      existing.push(run);
    } else {
      groupedRuns.set(key, [run]);
    }
  });

  return Array.from(groupedRuns.values())
    .sort(
      (left, right) =>
        latestLearningEvidenceTimestamp(right) -
        latestLearningEvidenceTimestamp(left),
    )
    .map((groupRuns, idx): EvolutionPatternCard | null => {
      const latestRun = groupRuns.reduce((best, current) => {
        return learningEvidenceTimestampMs(str(current.created_at, "")) >=
          learningEvidenceTimestampMs(str(best.created_at, ""))
          ? current
          : best;
      }, groupRuns[0]);
      const taskType = str(latestRun.task_type, "").trim();
      const requestPreview = collapseInlineWhitespace(
        str(latestRun.request_text, ""),
      );
      const title = inferLearningEvidenceTitle(
        groupRuns,
        requestPreview,
        taskType,
      );
      const latestSeen = humanTs(str(latestRun.created_at, "")).label;

      const allToolNames: string[] = [];
      const successfulToolNames: string[] = [];
      const failedToolNames: string[] = [];
      const failureReasons: string[] = [];
      let completedCount = 0;
      let failedCount = 0;
      let acceptedCount = 0;

      groupRuns.forEach((run) => {
        const state = normalizeLearningEvidenceState(run);
        const toolNames = stringList(run.tool_names);
        const failureReason = collapseInlineWhitespace(
          str(run.failure_reason, str(run.outcome_summary, "")),
        );
        toolNames.forEach((toolName) => {
          allToolNames.push(toolName);
          if (
            state === "completed" ||
            state === "success" ||
            state === "succeeded"
          )
            successfulToolNames.push(toolName);
          if (state === "failed" || state === "error")
            failedToolNames.push(toolName);
        });
        if (
          state === "completed" ||
          state === "success" ||
          state === "succeeded"
        )
          completedCount += 1;
        else if (state === "failed" || state === "error") {
          failedCount += 1;
          if (failureReason) failureReasons.push(failureReason);
        } else if (state === "accepted") acceptedCount += 1;
      });

      if (!title && !requestPreview && allToolNames.length === 0) return null;

      const runCount = groupRuns.length;
      const allToolSet = new Set(
        allToolNames.map((name) => name.trim().toLowerCase()).filter(Boolean),
      );
      const successfulToolSet = new Set(
        successfulToolNames
          .map((name) => name.trim().toLowerCase())
          .filter(Boolean),
      );
      const failedToolSet = new Set(
        failedToolNames
          .map((name) => name.trim().toLowerCase())
          .filter(Boolean),
      );
      const dominantBlocker = failureReasons[0]
        ? truncateUiText(failureReasons[0], 110)
        : "";
      const isPreferencePattern =
        acceptedCount > 0 && completedCount === 0 && failedCount === 0;
      const recoveredReminderPath =
        completedCount > 0 &&
        failedCount > 0 &&
        (allToolSet.has("schedule_task") ||
          successfulToolSet.has("schedule_task")) &&
        (allToolSet.has("calendar_create") ||
          failedToolSet.has("calendar_create"));

      let status = "Collecting examples";
      if (isPreferencePattern) status = "User preference captured";
      else if (completedCount > 0 && failedCount > 0) status = "Mixed results";
      else if (completedCount > 1) status = "Repeating successfully";
      else if (completedCount === 1) status = "Seen once";
      else if (failedCount > 0) status = "Needs review";

      let detail = `Captured ${runCount} related run${runCount === 1 ? "" : "s"} for comparison.`;
      let rationale =
        "Evolve is still collecting enough examples to decide whether a product change is warranted.";

      if (isPreferencePattern) {
        detail = `Captured an explicit user correction that should steer similar requests from the start.`;
        rationale =
          "Direct user constraints are stronger evidence than a default integration guess or a broad tool match.";
      } else if (recoveredReminderPath) {
        detail = `Observed ${runCount} similar reminder requests. Earlier attempts went through Calendar and failed. Later runs completed by creating scheduled in-app reminders instead.`;
        rationale =
          "This is concrete evidence that date-based 'notify me' requests do not need Calendar when a local reminder already satisfies the request.";
      } else if (completedCount > 1 && failedCount === 0) {
        detail = `Observed ${runCount} similar runs completing with a repeatable path.`;
        rationale =
          "Repeated success is strong enough to keep the current routing and tool-selection choice under watch.";
      } else if (completedCount > 0 && failedCount > 0) {
        detail = `Observed ${runCount} related runs with mixed outcomes. The latest path completed, but earlier attempts show the routing still needs refinement.`;
        rationale =
          "Evolve can use the failures to narrow when the alternate path should be avoided.";
      } else if (failedCount > 0) {
        detail = `Observed ${runCount} failed run${runCount === 1 ? "" : "s"}${dominantBlocker ? `, mostly blocked by ${dominantBlocker}` : ""}.`;
        rationale =
          "This is evidence for a guardrail or tighter trigger before retrying the same path.";
      } else if (completedCount === 1) {
        detail =
          "Observed one completed run. Evolve usually waits for repetition before treating it as a stable lesson.";
        rationale =
          "A single success is useful context, but not enough to claim that behavior improved.";
      }

      const chips = [`${runCount} run${runCount === 1 ? "" : "s"}`];
      if (completedCount > 0) chips.push(`${completedCount} completed`);
      if (failedCount > 0) chips.push(`${failedCount} failed`);
      if (acceptedCount > 0)
        chips.push(
          `${acceptedCount} correction${acceptedCount === 1 ? "" : "s"}`,
        );

      const evidence: string[] = [];
      const toolSummary = summarizeLearningEvidenceTools(allToolNames);
      if (toolSummary) evidence.push(`Tools used: ${toolSummary}`);
      if (failedCount > 0 && dominantBlocker)
        evidence.push(`Main blocker: ${dominantBlocker}`);

      return {
        key: `${str(latestRun.id, "run")}-${idx}`,
        title,
        status,
        detail,
        chips,
        rationale,
        example: requestPreview
          ? truncateUiText(requestPreview, 120)
          : undefined,
        evidence: evidence.join(" | "),
        runs: groupRuns,
        latestSeen: latestSeen !== "-" ? latestSeen : undefined,
        toolSummary,
        completedCount,
        failedCount,
        acceptedCount,
      };
    })
    .filter((card): card is EvolutionPatternCard => card !== null)
    .slice(0, 4);
}

function skillEvolutionChipColor(
  status: string,
): "default" | "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved" || normalized === "improved") return "success";
  if (
    normalized === "draft" ||
    normalized === "pending" ||
    normalized === "inconclusive"
  )
    return "warning";
  if (normalized === "regressed" || normalized === "rejected") return "error";
  if (normalized === "unchanged") return "info";
  return "default";
}

function skillEvolutionAlertSeverity(
  status: string,
): "success" | "warning" | "error" | "info" {
  const normalized = status.trim().toLowerCase();
  if (normalized === "approved" || normalized === "improved") return "success";
  if (normalized === "regressed" || normalized === "rejected") return "error";
  if (normalized === "unchanged") return "info";
  return "warning";
}

function skillEvolutionActionLabel(action: string): string {
  const normalized = action.trim().toLowerCase();
  if (normalized === "create_skill") return "Create skill";
  if (normalized === "optimize_description") return "Tune trigger";
  if (normalized === "improve_skill") return "Improve skill";
  return action || "Skill change";
}

function canonicalSkillIdentifier(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const compact = trimmed.toLowerCase().replace(/[^a-z0-9]+/g, "");
  if (compact === "trendprophet") return "trend-prophet";
  return trimmed;
}

function skillEvolutionMetricRows(
  row: JsonRecord,
): Array<{
  label: string;
  before: string;
  after: string;
  delta: string;
  positive: boolean | null;
}> {
  const baseline = asRecord(row.impact_baseline);
  const observed = asRecord(row.impact_observed);
  const successDelta =
    num(observed.success_rate, 0) - num(baseline.success_rate, 0);
  const failureDelta =
    num(baseline.failure_rate, 0) - num(observed.failure_rate, 0);
  const toolErrorDelta =
    num(baseline.tool_error_rate, 0) - num(observed.tool_error_rate, 0);
  return [
    {
      label: "Success",
      before: percentageLabel(baseline.success_rate, 1) || "-",
      after: percentageLabel(observed.success_rate, 1) || "-",
      delta: evolutionGainLabel(successDelta),
      positive: Number.isFinite(successDelta) ? successDelta >= 0 : null,
    },
    {
      label: "Failure",
      before: percentageLabel(baseline.failure_rate, 1) || "-",
      after: percentageLabel(observed.failure_rate, 1) || "-",
      delta: evolutionGainLabel(failureDelta),
      positive: Number.isFinite(failureDelta) ? failureDelta >= 0 : null,
    },
    {
      label: "Tool errors",
      before: percentageLabel(baseline.tool_error_rate, 1) || "-",
      after: percentageLabel(observed.tool_error_rate, 1) || "-",
      delta: evolutionGainLabel(toolErrorDelta),
      positive: Number.isFinite(toolErrorDelta) ? toolErrorDelta >= 0 : null,
    },
  ];
}

function evolutionSurfaceAudienceLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy") return "Reply routing";
  if (normalized === "main prompt bundle") return "Main replies";
  if (normalized === "request classifier") return "Request understanding";
  if (normalized === "specialist prompts") return "Specialist helpers";
  return value || "Experiment";
}

function evolutionSurfaceSummary(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Tests how often the assistant should answer directly versus handing work off.";
  if (normalized === "main prompt bundle")
    return "Tests a different set of reply instructions for the main assistant response.";
  if (normalized === "request classifier")
    return "Tests a different way to classify incoming requests before choosing a path.";
  if (normalized === "specialist prompts")
    return "Tests different instructions for helper specialists used during delegated work.";
  return "Tests a candidate behavior against the current stable setup.";
}

function evolutionSurfaceBenefit(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Can reduce unnecessary handoffs and keep simple requests faster.";
  if (normalized === "main prompt bundle")
    return "Can make replies clearer, more reliable, or less repetitive.";
  if (normalized === "request classifier")
    return "Can improve how quickly the assistant recognizes the right handling path.";
  if (normalized === "specialist prompts")
    return "Can make delegated specialist work more accurate and consistent.";
  return "Can improve how future requests are handled if the candidate keeps performing well.";
}

function evolutionSurfaceStableSummary(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "routing policy")
    return "Reply routing is using the current stable logic.";
  if (normalized === "main prompt bundle")
    return "Main replies are using the current stable prompt bundle.";
  if (normalized === "request classifier")
    return "Request understanding is using the current stable classifier.";
  if (normalized === "specialist prompts")
    return "Specialist helpers are using the current stable prompt bundle.";
  return "This area is currently on the stable baseline.";
}

function evolutionExperimentStatusText(item: {
  gate: string;
  last: string;
  enabled: boolean;
}): string {
  const gate = str(item.gate, "").trim();
  const last = str(item.last, "").trim();
  if (!item.enabled) return "No active experiment is running here.";
  if (gate && gate !== "-") return `Current gate signal: ${gate}.`;
  if (last && !/^no .* runs yet$/i.test(last)) return last;
  return "This experiment is running against the current stable behavior.";
}

function promptProposalScopeLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "prompt_profile") return "Main replies";
  if (normalized === "specialist_prompt_profile") return "Specialist helpers";
  return humanizeStatusLabel(value || "prompt profile");
}

function promptCanaryActionSummary(row: JsonRecord): string {
  const baselineSuccessRate = num(row.baseline_success_rate, 0) * 100;
  const candidateSuccessRate = num(row.candidate_success_rate, 0) * 100;
  const baselineSamples = num(row.baseline_samples, 0);
  const candidateSamples = num(row.candidate_samples, 0);
  return `Stable behavior is at ${baselineSuccessRate.toFixed(1)}% over ${baselineSamples.toLocaleString()} recent runs. The experiment is at ${candidateSuccessRate.toFixed(1)}% over ${candidateSamples.toLocaleString()} runs.`;
}

type EvolutionReviewEvidence = {
  metrics: Array<{ label: string; value: string }>;
  current: string[];
  proposed: string[];
  impact: string[];
};

function formatSignedPoints(value: number): string {
  if (!Number.isFinite(value)) return "-";
  return `${value >= 0 ? "+" : ""}${value.toFixed(1)} pts`;
}

function cleanEvidenceLines(lines: unknown[], limit = 3): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of lines) {
    const line = str(raw, "").trim();
    if (!line || seen.has(line)) continue;
    seen.add(line);
    out.push(line);
    if (out.length >= limit) break;
  }
  return out;
}

function EvolutionReviewEvidenceStrip({
  evidence,
}: {
  evidence: EvolutionReviewEvidence;
}): JSX.Element | null {
  const metrics = evidence.metrics
    .filter((item) => str(item.label, "").trim() && str(item.value, "").trim())
    .slice(0, 4);
  const sections = [
    { label: "Current", lines: cleanEvidenceLines(evidence.current, 3) },
    { label: "Proposed", lines: cleanEvidenceLines(evidence.proposed, 3) },
    { label: "Expected effect", lines: cleanEvidenceLines(evidence.impact, 3) },
  ].filter((section) => section.lines.length > 0);
  if (metrics.length === 0 && sections.length === 0) return null;
  return (
    <Box
      sx={{
        mt: 1,
        pt: 1,
        borderTop: "1px solid var(--ui-rgba-145-170-205-120)",
      }}
    >
      {metrics.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: {
              xs: "1fr 1fr",
              md: "repeat(4, minmax(0,1fr))",
            },
            gap: 1,
            mb: sections.length > 0 ? 1 : 0,
          }}
        >
          {metrics.map((item) => (
            <Box key={`${item.label}-${item.value}`} sx={{ minWidth: 0 }}>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                {item.label}
              </Typography>
              <Typography variant="body2">{item.value}</Typography>
            </Box>
          ))}
        </Box>
      ) : null}
      {sections.length > 0 ? (
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: {
              xs: "1fr",
              md: "repeat(auto-fit, minmax(0,1fr))",
            },
            gap: 1,
          }}
        >
          {sections.map((section) => (
            <Box key={section.label} sx={{ minWidth: 0 }}>
              <Typography
                variant="caption"
                sx={{ color: "text.secondary", display: "block", mb: 0.35 }}
              >
                {section.label}
              </Typography>
              <Stack spacing={0.35}>
                {section.lines.map((line, idx) => (
                  <Typography
                    key={`${section.label}-${idx}`}
                    variant="caption"
                    sx={{ color: "#fff8ed", display: "block" }}
                  >
                    - {line}
                  </Typography>
                ))}
              </Stack>
            </Box>
          ))}
        </Box>
      ) : null}
    </Box>
  );
}

function promptCanaryReviewEvidence(row: JsonRecord): EvolutionReviewEvidence {
  const baselineSuccessRate = num(row.baseline_success_rate, 0) * 100;
  const candidateSuccessRate = num(row.candidate_success_rate, 0) * 100;
  const baselineSamples = num(row.baseline_samples, 0);
  const candidateSamples = num(row.candidate_samples, 0);
  const successDelta = candidateSuccessRate - baselineSuccessRate;
  return {
    metrics: [
      { label: "Stable", value: `${baselineSuccessRate.toFixed(1)}%` },
      { label: "Experiment", value: `${candidateSuccessRate.toFixed(1)}%` },
      { label: "Gap", value: formatSignedPoints(successDelta) },
      {
        label: "Samples",
        value: `${baselineSamples.toLocaleString()} / ${candidateSamples.toLocaleString()}`,
      },
    ],
    current: [
      `Stable version ${str(row.baseline_version, "-")} is carrying ${baselineSamples.toLocaleString()} recent runs.`,
    ],
    proposed: [
      `Experiment version ${str(row.candidate_version, "-")} is still active on ${candidateSamples.toLocaleString()} runs.`,
    ],
    impact: [
      successDelta < 0
        ? `The experiment is currently down ${Math.abs(successDelta).toFixed(1)} points versus stable.`
        : `The experiment is currently up ${successDelta.toFixed(1)} points versus stable.`,
      `Wins versus losses: ${num(row.wins, 0).toLocaleString()} / ${num(row.losses, 0).toLocaleString()}.`,
      num(row.regression_p_value, Number.NaN) >= 0
        ? `Regression check p-value: ${num(row.regression_p_value, 0).toFixed(4)}.`
        : "",
    ],
  };
}

function promptOptimizationReviewEvidence(
  row: JsonRecord,
): EvolutionReviewEvidence {
  const preview = asRecord(row.change_preview);
  const current = stringList(preview.before);
  const proposed = stringList(preview.after);
  const impact = stringList(preview.impact_estimate);
  const targetArea = promptProposalScopeLabel(str(row.target_scope, "prompt_profile"));
  const riskLevel = str(row.risk_level, "default");
  const reviewStatus = str(row.review_status, "open");
  return {
    metrics: [
      { label: "Area", value: targetArea },
      { label: "Risk", value: `${riskLevel || "unknown"} risk` },
      {
        label: "Decision",
        value:
          reviewStatus === "open"
            ? "Needs decision"
            : humanizeStatusLabel(reviewStatus),
      },
    ],
    current:
      current.length > 0
        ? current
        : stringList(row.evidence).slice(0, 2),
    proposed:
      proposed.length > 0
        ? proposed
        : stringList(row.expected_benefit).slice(0, 2),
    impact:
      impact.length > 0
        ? impact
        : [
            ...stringList(row.expected_benefit).slice(0, 2),
            ...stringList(row.caveats).slice(0, 1),
          ],
  };
}

function skillReviewEvidence(row: JsonRecord): EvolutionReviewEvidence {
  const diffPreview = asRecord(row.diff_preview);
  const baseline = asRecord(row.impact_baseline);
  const evidence = asRecord(row.evidence);
  const added = stringList(diffPreview.added);
  const removed = stringList(diffPreview.removed);
  const failureReasons = stringList(evidence.recent_failure_reasons);
  const selectedExamples = stringList(evidence.selected_failure_examples);
  return {
    metrics: [
      {
        label: "Confidence",
        value: `${ratioPercent(row.confidence).toFixed(0)}%`,
      },
      {
        label: "Matched runs",
        value: num(baseline.matched_runs, 0).toLocaleString(),
      },
      {
        label: "Baseline success",
        value: percentageLabel(baseline.success_rate, 1) || "-",
      },
      {
        label: "Baseline failure",
        value: percentageLabel(baseline.failure_rate, 1) || "-",
      },
    ],
    current:
      removed.length > 0
        ? removed.slice(0, 2).map((line) => `Replace: ${line}`)
        : failureReasons.slice(0, 2).map((line) => `Current failure: ${line}`),
    proposed:
      added.length > 0
        ? added.slice(0, 2)
        : cleanEvidenceLines([
            str(row.diff_summary, ""),
            str(row.summary, ""),
          ]),
    impact: cleanEvidenceLines([
      `Targets ${num(baseline.matched_runs, 0).toLocaleString()} matched runs with ${percentageLabel(baseline.failure_rate, 1) || "-"} failure and ${percentageLabel(baseline.tool_error_rate, 1) || "-"} tool errors.`,
      ...failureReasons.slice(0, 1).map((line) => `Focus area: ${line}`),
      ...selectedExamples.slice(0, 1).map((line) => `Recent mismatch: ${line}`),
    ]),
  };
}

function learningCandidateReviewEvidence(
  row: JsonRecord,
  context: {
    strategyBaselineVersion: string;
    patternById: Map<string, JsonRecord>;
    itemById: Map<string, JsonRecord>;
  },
): EvolutionReviewEvidence {
  const type = str(row.candidate_type, "");
  const preview = asRecord(row.proposed_content_preview);
  const confidence = `${ratioPercent(row.confidence).toFixed(0)}%`;
  if (type === "strategy") {
    const pattern = context.patternById.get(str(row.pattern_id, ""));
    const defaultGuidance = stringList(preview.default_guidance);
    const taskGuidance = stringList(preview.task_guidance);
    const toolSequence = stringList(asRecord(pattern).tool_sequence);
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        {
          label: "Pattern runs",
          value: num(asRecord(pattern).sample_count, 0).toLocaleString(),
        },
        {
          label: "Pattern success",
          value: percentageLabel(asRecord(pattern).success_rate, 1) || "-",
        },
        {
          label: "Candidate",
          value: str(preview.strategy_version, "-"),
        },
      ],
      current: cleanEvidenceLines([
        context.strategyBaselineVersion
          ? `Matching requests still use stable strategy ${context.strategyBaselineVersion}.`
          : "Matching requests still use the current stable strategy.",
        pattern
          ? `Observed pattern ${str(pattern.title, "pattern")} succeeded on ${num(asRecord(pattern).sample_count, 0).toLocaleString()} runs.`
          : "",
      ]),
      proposed: cleanEvidenceLines([
        preview.strategy_version
          ? `Approve strategy version ${str(preview.strategy_version, "-")}.`
          : "",
        ...defaultGuidance.slice(0, 1),
        ...taskGuidance.slice(0, 2),
      ]),
      impact: cleanEvidenceLines([
        toolSequence.length > 0
          ? `Would steer matching work toward ${toolSequence.join(" -> ")}.`
          : "",
        `Confidence ${confidence} from repeated pattern evidence.`,
      ]),
    };
  }
  if (
    type === "memory_add" ||
    type === "memory_update" ||
    type === "memory_retract"
  ) {
    const operationType = str(preview.operation_type, type);
    const semanticKey = str(preview.semantic_key, str(row.subject_key, "memory"));
    const valuePreview = str(preview.value_preview, "");
    const scope = humanizeStatusLabel(str(preview.scope, "global"));
    const durability = humanizeStatusLabel(str(preview.durability, ""));
    const sensitive = toBool(preview.looks_sensitive);
    const sensitiveReason = str(preview.sensitive_reason, "");
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        { label: "Kind", value: humanizeStatusLabel(str(preview.memory_kind, "memory")) },
        { label: "Scope", value: scope },
        {
          label: "Duration",
          value: durability || "-",
        },
      ],
      current: [
        operationType === "memory_add"
          ? `Future turns do not yet store ${semanticKey} as reusable memory.`
          : operationType === "memory_update"
            ? `Future turns still use the current saved value for ${semanticKey}.`
            : `Future turns still treat ${semanticKey} as active memory.`,
      ],
      proposed: cleanEvidenceLines([
        operationType === "memory_retract"
          ? `Retract saved memory ${semanticKey}.`
          : valuePreview
            ? `${humanizeStatusLabel(operationType)} ${semanticKey}: ${valuePreview}`
            : `${humanizeStatusLabel(operationType)} ${semanticKey}.`,
        `Apply at ${scope}${durability ? ` with ${durability} duration` : ""}.`,
      ]),
      impact: cleanEvidenceLines([
        operationType === "memory_add"
          ? "Future replies can use this fact automatically."
          : operationType === "memory_update"
            ? "Future replies will rely on the updated value instead of the older one."
            : "Future replies will stop leaning on the retracted memory.",
        sensitive
          ? `Value stays masked because it looks sensitive${sensitiveReason ? `: ${sensitiveReason}` : "."}`
          : `Confidence ${confidence}.`,
      ]),
    };
  }
  if (type === "memory_deprecate") {
    const item = context.itemById.get(str(preview.item_id, ""));
    const nextStatus = humanizeStatusLabel(str(preview.next_status, "deprecated"));
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        {
          label: "Support",
          value: num(asRecord(item).support_count, 0).toLocaleString(),
        },
        {
          label: "Contradictions",
          value: num(asRecord(item).contradiction_count, 0).toLocaleString(),
        },
      ],
      current: cleanEvidenceLines([
        item
          ? `${str(item.title, "This learned item")} is still active.`
          : "This learned item is still active.",
        item
          ? `Support versus contradictions: ${num(asRecord(item).support_count, 0).toLocaleString()} / ${num(asRecord(item).contradiction_count, 0).toLocaleString()}.`
          : "",
      ]),
      proposed: [
        `Set this learned item to ${nextStatus}.`,
      ],
      impact: [
        "Stops stale guidance from influencing future turns.",
      ],
    };
  }
  if (type === "memory_merge") {
    const target = context.itemById.get(str(preview.target_item_id, ""));
    const source = context.itemById.get(str(preview.source_item_id, ""));
    return {
      metrics: [
        { label: "Confidence", value: confidence },
        { label: "Target", value: str(asRecord(target).title, "Kept item") },
        { label: "Duplicate", value: str(asRecord(source).title, "Merged item") },
      ],
      current: cleanEvidenceLines([
        target && source
          ? `${str(asRecord(target).title, "Target item")} and ${str(asRecord(source).title, "source item")} are both still active.`
          : "Two overlapping memories are still active separately.",
      ]),
      proposed: cleanEvidenceLines([
        target
          ? `Keep ${str(asRecord(target).title, "the stronger memory")} as the surviving record.`
          : "Keep the stronger memory as the surviving record.",
        source
          ? `Deprecate duplicate memory ${str(asRecord(source).title, "the duplicate")}.`
          : "Deprecate the duplicate memory.",
      ]),
      impact: cleanEvidenceLines([
        "Reduces duplicate recall and conflicting memory retrieval.",
        str(preview.reason, "") === "duplicate_content"
          ? "The merge is based on overlapping content, not surface wording."
          : "",
      ]),
    };
  }
  return {
    metrics: [{ label: "Confidence", value: confidence }],
    current: cleanEvidenceLines([str(row.summary, ""), str(row.preview, "")]),
    proposed: cleanEvidenceLines([str(row.preview, ""), str(row.title, "")]),
    impact: cleanEvidenceLines([
      `Confidence ${confidence}.`,
      "Evolve will measure impact after approval if this change goes live.",
    ]),
  };
}

type EvolutionPageTab = "what" | "helped" | "tests" | "review";

const EVOLUTION_PAGE_TABS: Array<{ value: EvolutionPageTab; label: string }> = [
  { value: "what", label: "Recent changes" },
  { value: "helped", label: "What improved" },
  { value: "tests", label: "Experiments" },
  { value: "review", label: "Needs approval" },
];

function clampPercent(value: unknown): number {
  const parsed = num(value, 0);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(100, parsed));
}

function ratioPercent(value: unknown): number {
  const parsed = num(value, 0);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(100, parsed * 100));
}

function evolutionGainLabel(value: unknown): string {
  const parsed = num(value, Number.NaN);
  if (!Number.isFinite(parsed)) return "-";
  return `${parsed >= 0 ? "+" : ""}${(parsed * 100).toFixed(1)} pts`;
}

function EvolutionStatStrip({
  items,
}: {
  items: Array<{
    label: string;
    value: ReactNode;
    helper: ReactNode;
    tone?: "default" | "good" | "warn" | "info";
  }>;
}) {
  return (
    <Box className="list-shell stat-strip">
      {items.map((item) => (
        <div
          key={String(item.label)}
          className="stat-strip-item"
          data-tone={item.tone !== "default" ? item.tone : undefined}
        >
          <span className="stat-strip-label">{item.label}</span>
          <span className="stat-strip-value">{item.value}</span>
          <span className="stat-strip-helper">{item.helper}</span>
        </div>
      ))}
    </Box>
  );
}

function EvolutionRolloutBar({
  label,
  percent,
}: {
  label: string;
  percent: number;
}) {
  const pct = clampPercent(percent);
  return (
    <Box>
      <Stack
        direction="row"
        spacing={1}
        sx={{
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Typography variant="body2">{label}</Typography>
        <Typography
          variant="caption"
          sx={{
            color: "text.secondary",
          }}
        >
          {pct.toFixed(0)}%
        </Typography>
      </Stack>
      <Box
        role="meter"
        aria-label={`${label} rollout ${pct.toFixed(0)} percent`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        sx={{
          mt: 0.5,
          height: 8,
          borderRadius: 1,
          bgcolor: "var(--ui-rgba-145-170-205-140)",
          overflow: "hidden",
        }}
      >
        <Box
          sx={{
            width: `${pct}%`,
            height: "100%",
            borderRadius: 1,
            bgcolor: pct > 0 ? "#fbbf24" : "var(--ui-rgba-84-198-255-450)",
          }}
        />
      </Box>
    </Box>
  );
}

/* Analytics (top-level page) */
