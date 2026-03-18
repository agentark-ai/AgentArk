import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Autocomplete,
  Avatar,
  Box,
  Button,
  Checkbox,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Drawer,
  Divider,
  FormControlLabel,
  Grid2,
  IconButton,
  List,
  ListItem,
  ListItemText,
  Link,
  Menu,
  MenuItem,
  Stack,
  Switch,
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
  Typography
} from "@mui/material";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ArrowDropDownRoundedIcon from "@mui/icons-material/ArrowDropDownRounded";
import AttachFileRoundedIcon from "@mui/icons-material/AttachFileRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import FileDownloadRoundedIcon from "@mui/icons-material/FileDownloadRounded";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import ArrowUpwardRoundedIcon from "@mui/icons-material/ArrowUpwardRounded";
import StopRoundedIcon from "@mui/icons-material/StopRounded";
import CloseIcon from "@mui/icons-material/Close";
import FilterListRoundedIcon from "@mui/icons-material/FilterListRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import SmartToyRoundedIcon from "@mui/icons-material/SmartToyRounded";
import PersonRoundedIcon from "@mui/icons-material/PersonRounded";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type MouseEvent, type ReactNode } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../api/client";
import AgentLogo from "../assets/logo.svg";
import { IntegrationsPanel } from "./IntegrationsPanel";
import { ObservabilityPanel } from "./ObservabilityPanel";
import { SuggestionRunDialog, type SuggestionRunState } from "./SuggestionRunDialog";
import { SwarmManager } from "./SwarmManager";
import type { ArkPulseRemediationSpec, ArkPulseRunFixRequest, SkillImportResponse, LlmAnalyticsResponse } from "../types";
import { useUiStore } from "../store/uiStore";

const REFRESH_MS = 8000;
const IMPORT_SECURITY_FORCE_RISK_THRESHOLD = 8;
const DEVELOPER_MODE_STORAGE_KEY = "agentark.developer_mode";
const DEVELOPER_MODE_EVENT = "agentark:developer-mode-change";
const OLLAMA_DEFAULT_BASE_URL = "http://localhost:11434";
const OPENROUTER_DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const SHOW_EXPERIMENTAL_AUTONOMY_TOOLS = false;
const CHAT_LAST_CONVERSATION_STORAGE_KEY = "agentark.chat.lastConversationId";
const CHAT_PENDING_RUN_STORAGE_KEY = "agentark.chat.pendingRun";
const CHAT_PENDING_RUN_TTL_MS = 45 * 60 * 1000;
const CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS = 16000;
const CHAT_PENDING_STREAM_STEPS_MAX = 48;
const CHAT_LAUNCH_RUN_EVENT = "agentark.chat.launch-run";
const CHAT_RUN_STATUS_EVENT = "agentark.chat.run-status";
type ImportRiskBand = "secure" | "review" | "risky";

type ChatPendingRunSnapshot = {
  conversationId: string;
  message: string;
  projectId: string;
  startedAt: number;
  streamingResponse?: string;
  streamingSteps?: JsonRecord[];
  failedUserMessage?: string;
};

type ChatLaunchRunDetail = {
  message: string;
  conversationId?: string;
  projectId?: string;
  navigateToChat?: boolean;
  source?: string;
  resolve?: (started: boolean) => void;
  reject?: (message: string) => void;
};

type ChatRunStatusDetail = {
  conversationId: string;
  status: "completed" | "error";
  source?: string;
  message: string;
};

type ChatExecutionMode = "auto" | "chat" | "task";

type ActiveChatTaskState = {
  id: string;
  description: string;
  status: string;
  workType: string;
};

const MODEL_FALLBACKS_BY_PROVIDER: Record<string, string[]> = {
  openai: ["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini", "o3"],
  "openai-subscription": ["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini", "o3"],
  anthropic: ["claude-opus-4-20250514", "claude-sonnet-4-20250514", "claude-3-7-sonnet-latest", "claude-3-5-haiku-latest"],
  openrouter: ["openai/gpt-5", "anthropic/claude-sonnet-4", "google/gemini-2.5-pro"],
  "openai-compatible": [],
  ollama: [],
};

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
  window.dispatchEvent(new CustomEvent(DEVELOPER_MODE_EVENT, { detail: { enabled: next } }));
}

type JsonRecord = Record<string, unknown>;
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
    buildPayload: (detail) => ({ command: detail })
  },
  {
    id: "read_file",
    label: "Read a file",
    actionKind: "file_read",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/report.txt",
    buildPayload: (detail) => ({ path: detail })
  },
  {
    id: "write_file",
    label: "Create or edit a file",
    actionKind: "file_write",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/notes.txt",
    buildPayload: (detail) => ({ path: detail, operation: "write" })
  },
  {
    id: "open_url",
    label: "Open a URL or call an API",
    actionKind: "http_get",
    detailLabel: "URL",
    detailPlaceholder: "https://api.example.com/status",
    buildPayload: (detail) => ({ url: detail })
  },
  {
    id: "run_code",
    label: "Run generated code",
    actionKind: "code_execute",
    detailLabel: "What should the code do?",
    detailPlaceholder: "Summarize CSV rows and return totals",
    buildPayload: (detail) => ({ instruction: detail })
  },
  {
    id: "email_action",
    label: "Read or send an email",
    actionKind: "gmail_reply",
    detailLabel: "Email task",
    detailPlaceholder: "Reply with a short status update",
    buildPayload: (detail) => ({ message: detail })
  }
];

type SkillImportSummary = {
  result: SkillImportResponse;
  message?: string;
};

type ImportCallback = (summary: SkillImportSummary) => Promise<void> | void;

type SkillEditorForm = {
  name: string;
  description: string;
  version: string;
  requiredInputsCsv: string;
  emoji: string;
  toolsCsv: string;
  workflow: string;
};

export type WorkspaceView =
  | "chat"
  | "tasks"
  | "skills"
  | "apps"
  | "moltbook"
  | "goals"
  | "autonomy"
  | "documents"
  | "memory"
  | "projects"
  | "swarm"
  | "trace"
  | "status"
  | "analytics"
  | "arkpulse"
  | "settings";

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
  if (typeof value === "number" || typeof value === "boolean") return String(value);
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

function compactUnknown(value: unknown, maxLen = 2200): string {
  if (value == null) return "";
  if (typeof value === "string") return value.trim().slice(0, maxLen);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    const serialized = JSON.stringify(value, null, 2);
    if (!serialized) return "";
    if (serialized.length <= maxLen) return serialized;
    return `${serialized.slice(0, maxLen)}...`;
  } catch {
    return "";
  }
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
  text = text.replace(/\s*\(\d+\s*[smh]\s*idle\)\.?\s*/gi, " ");
  text = text.replace(/\bno new output yet\b\.?/gi, "");
  text = text.replace(/\s+\./g, ".");
  text = text.replace(/([.!?])\s*[.!?]+/g, "$1");
  text = text.replace(/\s+/g, " ").trim();
  if (!text) return "Working on the current step. No new output yet.";
  if (!/[.!?]$/.test(text)) text += ".";
  return `${text} No new output yet.`;
}

function looksLikeHtmlPayload(text: string): boolean {
  const trimmed = text.trim();
  return (
    /^<!doctype html/i.test(trimmed) ||
    /^<html\b/i.test(trimmed) ||
    (/<(html|head|body|title|div|script|main)\b/i.test(trimmed) && /<\/(html|body|div|script|main)>/i.test(trimmed))
  );
}

function looksLikeSourcePayload(text: string): boolean {
  const sample = text.trim().split(/\r?\n/).slice(0, 10).join("\n");
  if (!sample) return false;
  return (
    /^(from\s+\w+\s+import|import\s+[\w.{},* ]+|def\s+\w+\(|class\s+\w+|async\s+def\s+\w+\()/m.test(sample) ||
    /^(const|let|var|function|export|import)\s/m.test(sample) ||
    /^\s*#include\s+[<"]/m.test(sample) ||
    /^package\s+[\w.]+;$/m.test(sample)
  );
}

function summarizeJsonActivityPayload(value: unknown): string {
  if (Array.isArray(value)) {
    return value.length === 0
      ? "Returned empty list."
      : `Returned list with ${value.length} item${value.length === 1 ? "" : "s"}.`;
  }
  const obj = asRecord(value);
  const keys = Object.keys(obj);
  if (keys.length === 0) return "Returned empty object.";

  const apps = Array.isArray(obj.apps) ? asRecords(obj.apps) : [];
  const matchedApp = asRecord(obj.matched_app);
  if (apps.length > 0 || Object.keys(matchedApp).length > 0) {
    const matchedTitle = str(matchedApp.title, "").trim();
    if (matchedTitle) {
      return `Matched ${Math.max(apps.length, 1)} app${Math.max(apps.length, 1) === 1 ? "" : "s"}; selected ${matchedTitle}.`;
    }
    return `Matched ${apps.length} app${apps.length === 1 ? "" : "s"} and loaded app metadata.`;
  }

  const title = str(obj.title, "").trim();
  const fileBytes = num(obj.file_bytes, -1);
  if (title && fileBytes >= 0) {
    return `Loaded metadata for ${title} (${formatBytes(fileBytes)}).`;
  }

  const visibleKeys = keys.slice(0, 4).join(", ");
  const remaining = keys.length > 4 ? `, +${keys.length - 4} more` : "";
  return `Returned structured data: ${visibleKeys}${remaining}.`;
}

function summarizeActivityDetail(detail: string): string {
  const trimmed = (detail || "").trim();
  if (!trimmed) return "";

  if (/^still working:/i.test(trimmed) || /\bno new output yet\b/i.test(trimmed)) {
    return normalizeHeartbeatDetailText(trimmed);
  }

  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
    try {
      return summarizeJsonActivityPayload(JSON.parse(trimmed));
    } catch {
      // Fall through to other heuristics.
    }
  }

  if (looksLikeHtmlPayload(trimmed)) {
    const titleMatch = trimmed.match(/<title[^>]*>([^<]+)<\/title>/i);
    const title = titleMatch?.[1]?.trim();
    return title ? `Read HTML document: ${title}.` : "Read HTML document.";
  }

  if (looksLikeSourcePayload(trimmed)) {
    const lineCount = trimmed.split(/\r?\n/).length;
    return `Read source file contents (${lineCount} line${lineCount === 1 ? "" : "s"}).`;
  }

  if (trimmed.length > 240 && /[{}[\]<>;]/.test(trimmed)) {
    return "Returned verbose tool output.";
  }

  return trimmed;
}

function isHumanReadableStatus(detail: string): boolean {
  const trimmed = (detail || "").trim();
  if (!trimmed || trimmed.length > 120) return false;
  if (looksLikeHtmlPayload(trimmed) || looksLikeSourcePayload(trimmed)) return false;
  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
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
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function generateConversationId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `conv-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function loadChatPendingRunSnapshot(): ChatPendingRunSnapshot | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(CHAT_PENDING_RUN_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<ChatPendingRunSnapshot>;
    const conversationId = typeof parsed.conversationId === "string" ? parsed.conversationId.trim() : "";
    const startedAt = typeof parsed.startedAt === "number" ? parsed.startedAt : 0;
    if (!conversationId || startedAt <= 0) {
      window.sessionStorage.removeItem(CHAT_PENDING_RUN_STORAGE_KEY);
      return null;
    }
    if (Date.now() - startedAt > CHAT_PENDING_RUN_TTL_MS) {
      window.sessionStorage.removeItem(CHAT_PENDING_RUN_STORAGE_KEY);
      return null;
    }
    const streamingResponse =
      typeof parsed.streamingResponse === "string"
        ? parsed.streamingResponse.slice(0, CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS)
        : "";
    const streamingSteps = Array.isArray(parsed.streamingSteps)
      ? asRecords(parsed.streamingSteps)
          .slice(-CHAT_PENDING_STREAM_STEPS_MAX)
          .map((raw) => {
            const normalized: JsonRecord = {};
            const icon = str(raw.icon, "").trim();
            const title = str(raw.title, "").trim();
            const detail = str(raw.detail, "").trim();
            const stepType = str(raw.step_type, "").trim();
            const data = compactUnknown(raw.data, 800);
            if (icon) normalized.icon = icon.slice(0, 64);
            if (title) normalized.title = title.slice(0, 220);
            if (detail) normalized.detail = detail.slice(0, 900);
            if (stepType) normalized.step_type = stepType.slice(0, 80);
            if (data) normalized.data = data;
            return normalized;
          })
      : [];
    return {
      conversationId,
      message: typeof parsed.message === "string" ? parsed.message : "",
      projectId: typeof parsed.projectId === "string" ? parsed.projectId : "",
      startedAt,
      streamingResponse,
      streamingSteps,
      failedUserMessage:
        typeof parsed.failedUserMessage === "string" ? parsed.failedUserMessage : ""
    };
  } catch {
    return null;
  }
}

function storeChatPendingRunSnapshot(snapshot: ChatPendingRunSnapshot | null): void {
  if (typeof window === "undefined") return;
  try {
    if (!snapshot) {
      window.sessionStorage.removeItem(CHAT_PENDING_RUN_STORAGE_KEY);
      return;
    }
    window.sessionStorage.setItem(CHAT_PENDING_RUN_STORAGE_KEY, JSON.stringify(snapshot));
  } catch {
    // Ignore storage failures.
  }
}

function stripAttachmentContextMarker(text: string): string {
  return text
    .replace(/\n\n\[Attached documents indexed for retrieval:[\s\S]*\]$/i, "")
    .trimEnd();
}

type ChatMarkdownBlock =
  | { type: "heading"; level: number; text: string }
  | { type: "code"; language: string; content: string }
  | { type: "ul"; items: string[] }
  | { type: "ol"; items: string[] }
  | { type: "paragraph"; text: string };

function lineStartsMarkdownBlock(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return true;
  if (/^#{1,6}\s+/.test(trimmed)) return true;
  if (/^```/.test(trimmed)) return true;
  if (/^[-*]\s+/.test(trimmed)) return true;
  if (/^\d+\.\s+/.test(trimmed)) return true;
  return false;
}

function isContextualCredentialMatch(matchText: string): boolean {
  const lower = (matchText || "").toLowerCase();
  if (!lower) return false;
  const placeholders = ["your-api-key", "your_api_key", "example", "dummy", "changeme", "replace_me", "test-key", "sample-key"];
  return lower.includes("$") || lower.includes("${") || placeholders.some((token) => lower.includes(token));
}

function isContextualImportFinding(finding: JsonRecord): boolean {
  const category = str(finding.category, "").toLowerCase();
  if (category === "networkaccess" || category === "environmentaccess") return true;
  if (category === "credentialpattern") {
    return isContextualCredentialMatch(str(finding.matched_text, ""));
  }
  return false;
}

function computeImportRiskSummary(security: SkillImportResponse["security"] | null | undefined): {
  score10: number;
  band: ImportRiskBand;
  bandLabel: string;
  chipColor: "success" | "warning" | "error";
  rawSeverity: number;
  totalFindings: number;
  contextualFindings: number;
} {
  if (!security) {
    return {
      score10: 0,
      band: "secure",
      bandLabel: "Secure",
      chipColor: "success",
      rawSeverity: 0,
      totalFindings: 0,
      contextualFindings: 0
    };
  }

  const findings = Array.isArray(security.findings) ? security.findings : [];
  const findingRecords = findings.map((item) => asRecord(item));
  const explicitSeverity = Math.max(0, num(security.total_severity, 0));
  const summedSeverity = findingRecords.reduce((sum, finding) => sum + Math.max(0, num(finding.severity, 0)), 0);
  const rawSeverity = explicitSeverity > 0 ? explicitSeverity : summedSeverity;
  const serverTotalFindings = num(security.total_findings, -1);
  const serverContextualFindings = num(security.contextual_findings, -1);
  const totalFindings = serverTotalFindings >= 0 ? serverTotalFindings : findingRecords.length;
  const contextualFindings =
    serverContextualFindings >= 0
      ? Math.min(totalFindings, serverContextualFindings)
      : findingRecords.filter((finding) => isContextualImportFinding(finding)).length;
  const contextualRatio = totalFindings > 0 ? contextualFindings / totalFindings : 0;

  const providedRiskScore = num(security.risk_score_10, -1);
  const providedBand = str(security.risk_band, "").toLowerCase();

  // Base normalization: existing static-analysis severity scale mapped to 0..10.
  let score = providedRiskScore >= 0 ? Math.min(10, providedRiskScore) : Math.min(10, rawSeverity / 4);

  // Common integration patterns (curl/env refs/placeholders) should count as context, not danger.
  if (contextualRatio >= 0.8) {
    // Nearly all findings are standard patterns — cap score regardless of backend value.
    score = Math.min(score, 4);
  } else if (contextualRatio >= 0.5) {
    score = Math.min(score, 6);
  }

  const threatLevel = str(security.threat_level, "").toLowerCase();
  if (threatLevel === "malicious" && contextualRatio < 0.8) {
    score = Math.max(score, 8.5);
  } else if (threatLevel === "suspicious" && contextualRatio < 0.5) {
    score = Math.max(score, 5);
  }
  if (toBool(security.blocked) && contextualRatio < 0.8) {
    score = Math.max(score, 8.5);
  }

  const score10 = Math.max(0, Math.min(10, Math.round(score * 10) / 10));
  const resolvedBand = providedBand === "secure" || providedBand === "review" || providedBand === "risky"
    ? (providedBand as ImportRiskBand)
    : score10 < 5
    ? "secure"
    : score10 < 8
    ? "review"
    : "risky";
  if (resolvedBand === "secure") {
    return {
      score10,
      band: "secure",
      bandLabel: "Secure",
      chipColor: "success",
      rawSeverity,
      totalFindings,
      contextualFindings
    };
  }
  if (resolvedBand === "review") {
    return {
      score10,
      band: "review",
      bandLabel: "Needs review",
      chipColor: "warning",
      rawSeverity,
      totalFindings,
      contextualFindings
    };
  }
  return {
    score10,
    band: "risky",
    bandLabel: "Risky",
    chipColor: "error",
    rawSeverity,
    totalFindings,
    contextualFindings
  };
}

function isUserActionableDoctorFinding(value: unknown): boolean {
  const row = asRecord(value);
  if (!Object.prototype.hasOwnProperty.call(row, "user_actionable")) return true;
  return toBool(row.user_actionable);
}

function parseArkPulseRemediationSpec(value: unknown): ArkPulseRemediationSpec | null {
  const row = asRecord(value);
  const kind = str(row.kind, "").trim().toLowerCase();
  if (!kind) return null;
  if (kind === "tunnel_start_verify") {
    return { kind: "tunnel_start_verify" };
  }
  if (kind === "tunnel_restart_verify") {
    return { kind: "tunnel_restart_verify" };
  }
  if (kind === "app_restart") {
    const appId = str(row.app_id, "").trim();
    if (!appId) return null;
    return { kind: "app_restart", app_id: appId };
  }
  if (kind === "shell_command") {
    const command = str(row.command, "").trim();
    if (!command) return null;
    return { kind: "shell_command", command };
  }
  return null;
}

function describeArkPulseRemediation(remediation: ArkPulseRemediationSpec | null): string {
  if (!remediation) return "-";
  if (remediation.kind === "tunnel_start_verify") {
    return "Start tunnel and verify /tunnel/status returns active + URL";
  }
  if (remediation.kind === "tunnel_restart_verify") {
    return "Restart tunnel and verify public reachability";
  }
  if (remediation.kind === "app_restart") {
    return `Restart app ${remediation.app_id} and re-check health`;
  }
  return remediation.command.trim() || "-";
}

function classifyLegacyRunnableArkPulseFixCommand(value: string): ArkPulseRemediationSpec | null {
  const normalized = (value || "").trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower === "-" || lower === "n/a" || lower === "none") return null;

  if (lower.includes("start tunnel") && lower.includes("/tunnel/status")) {
    return { kind: "tunnel_start_verify" };
  }
  if (lower.includes("restart") && lower.includes("tunnel")) {
    return { kind: "tunnel_restart_verify" };
  }
  const appRestartMatch = normalized.match(/^POST\s+\/api\/apps\/([A-Za-z0-9_-]+)\/restart$/i);
  if (appRestartMatch?.[1]) {
    return { kind: "app_restart", app_id: appRestartMatch[1] };
  }

  if (
    lower.includes("\n") ||
    lower.includes("\r") ||
    lower.includes("||") ||
    lower.includes(";") ||
    lower.includes("`") ||
    lower.includes("$(")
  ) {
    return null;
  }

  const segments = normalized
    .split("&&")
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0);
  if (segments.length < 2) return null;

  const cdSegment = segments[0].toLowerCase();
  if (!cdSegment.startsWith("cd /app/data/apps/")) return null;

  const supported = segments.slice(1).every((segment) => {
    const seg = segment.toLowerCase();
    return (
      seg.startsWith("pip-compile requirements.txt") ||
      seg.startsWith("rg -n ") ||
      seg === "cargo generate-lockfile" ||
      seg.startsWith("npm pkg delete ") ||
      seg.startsWith("mv .env ")
    );
  });

  if (!supported) return null;
  return { kind: "shell_command", command: normalized };
}

function getRunnableArkPulseRemediation(value: unknown): ArkPulseRemediationSpec | null {
  const row = asRecord(value);
  return parseArkPulseRemediationSpec(row.remediation) ?? classifyLegacyRunnableArkPulseFixCommand(str(row.fix_command, "").trim());
}

function getArkPulseFixText(value: unknown): string {
  const row = asRecord(value);
  const remediation = parseArkPulseRemediationSpec(row.remediation);
  if (remediation) return describeArkPulseRemediation(remediation);
  const fix = str(row.fix_command, "").trim();
  if (fix) return fix;
  return "-";
}

function parseChatMarkdown(source: string): ChatMarkdownBlock[] {
  const text = (source || "").replace(/\r\n/g, "\n");
  const lines = text.split("\n");
  const blocks: ChatMarkdownBlock[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i] ?? "";
    const trimmed = line.trim();

    if (!trimmed) {
      i += 1;
      continue;
    }

    const codeFenceMatch = trimmed.match(/^```([A-Za-z0-9_+-]+)?\s*$/);
    if (codeFenceMatch) {
      const language = (codeFenceMatch[1] || "").trim().toLowerCase();
      i += 1;
      const codeLines: string[] = [];
      while (i < lines.length && !(lines[i] || "").trim().startsWith("```")) {
        codeLines.push(lines[i] ?? "");
        i += 1;
      }
      if (i < lines.length && (lines[i] || "").trim().startsWith("```")) i += 1;
      blocks.push({
        type: "code",
        language,
        content: codeLines.join("\n")
      });
      continue;
    }

    const headingMatch = trimmed.match(/^(#{1,6})\s+(.*)$/);
    if (headingMatch) {
      blocks.push({
        type: "heading",
        level: headingMatch[1].length,
        text: headingMatch[2].trim()
      });
      i += 1;
      continue;
    }

    if (/^[-*]\s+/.test(trimmed)) {
      const items: string[] = [];
      while (i < lines.length) {
        const itemLine = (lines[i] || "").trim();
        const match = itemLine.match(/^[-*]\s+(.*)$/);
        if (!match) break;
        items.push(match[1].trim());
        i += 1;
      }
      blocks.push({ type: "ul", items });
      continue;
    }

    if (/^\d+\.\s+/.test(trimmed)) {
      const items: string[] = [];
      while (i < lines.length) {
        const itemLine = (lines[i] || "").trim();
        const match = itemLine.match(/^\d+\.\s+(.*)$/);
        if (!match) break;
        items.push(match[1].trim());
        i += 1;
      }
      blocks.push({ type: "ol", items });
      continue;
    }

    const paragraphLines: string[] = [line];
    i += 1;
    while (i < lines.length) {
      const next = lines[i] ?? "";
      if (!next.trim()) {
        i += 1;
        break;
      }
      if (lineStartsMarkdownBlock(next)) break;
      paragraphLines.push(next);
      i += 1;
    }
    blocks.push({ type: "paragraph", text: paragraphLines.join("\n").trim() });
  }

  return blocks;
}

function renderInlineMarkdown(text: string): ReactNode[] {
  const source = text || "";
  if (!source) return [];
  const tokenRegex = /(`[^`\n]+`|\*\*[^*]+?\*\*|__[^_]+?__|\*[^*\n]+?\*|_[^_\n]+?_|(?:https?:\/\/[^\s<>()]+)|\[[^\]]+\]\(([^)\s]+)\))/g;
  const nodes: ReactNode[] = [];
  let index = 0;
  let lastIndex = 0;
  let match: RegExpExecArray | null = null;

  const pushText = (value: string) => {
    if (!value) return;
    nodes.push(<span key={`t-${index++}`}>{value}</span>);
  };

  const splitUrlTrailingPunctuation = (value: string): { href: string; trailing: string } => {
    let href = value;
    let trailing = "";
    while (href.length > 0 && /[.,!?;:)]/.test(href[href.length - 1] || "")) {
      trailing = `${href[href.length - 1]}${trailing}`;
      href = href.slice(0, -1);
    }
    return { href, trailing };
  };

  while ((match = tokenRegex.exec(source)) !== null) {
    const token = match[0];
    const start = match.index;
    if (start > lastIndex) pushText(source.slice(lastIndex, start));

    if (token.startsWith("`") && token.endsWith("`")) {
      nodes.push(
        <code key={`c-${index++}`} className="chat-md-inline-code">
          {token.slice(1, -1)}
        </code>
      );
    } else if ((token.startsWith("**") && token.endsWith("**")) || (token.startsWith("__") && token.endsWith("__"))) {
      nodes.push(<strong key={`b-${index++}`}>{token.slice(2, -2)}</strong>);
    } else if ((token.startsWith("*") && token.endsWith("*")) || (token.startsWith("_") && token.endsWith("_"))) {
      nodes.push(<em key={`i-${index++}`}>{token.slice(1, -1)}</em>);
    } else if (token.startsWith("[")) {
      const linkMatch = token.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
      if (linkMatch) {
        const rawHref = linkMatch[2].trim();
        const safeHref =
          rawHref.startsWith("http://") ||
          rawHref.startsWith("https://") ||
          rawHref.startsWith("/");
        if (!safeHref) {
          pushText(token);
          lastIndex = tokenRegex.lastIndex;
          continue;
        }
        nodes.push(
          <a
            key={`l-${index++}`}
            href={rawHref}
            target="_blank"
            rel="noopener noreferrer"
            className="chat-md-link"
          >
            {linkMatch[1]}
          </a>
        );
      } else {
        pushText(token);
      }
    } else if (token.startsWith("http://") || token.startsWith("https://")) {
      const { href, trailing } = splitUrlTrailingPunctuation(token);
      nodes.push(
        <a key={`u-${index++}`} href={href} target="_blank" rel="noopener noreferrer" className="chat-md-link">
          {href}
        </a>
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
    out.push(<span key={`line-${i}`}>{renderInlineMarkdown(lines[i] || "")}</span>);
    if (i < lines.length - 1) out.push(<br key={`br-${i}`} />);
  }
  return out;
}

// Async-loaded react-markdown for proper GFM rendering
let _ReactMarkdown: React.ComponentType<{ children: string; remarkPlugins?: unknown[]; components?: Record<string, unknown> }> | null = null;
let _remarkGfm: unknown = null;
let _mdReady = false;
let _mdLoadPromise: Promise<void> | null = null;

function ensureMarkdownLoaded(): Promise<void> {
  if (_mdReady) return Promise.resolve();
  if (_mdLoadPromise) return _mdLoadPromise;
  _mdLoadPromise = Promise.all([
    import("react-markdown").then((m) => { _ReactMarkdown = m.default as typeof _ReactMarkdown; }),
    import("remark-gfm").then((m) => { _remarkGfm = m.default; }),
  ]).then(() => { _mdReady = true; });
  return _mdLoadPromise;
}

// Eagerly start loading
ensureMarkdownLoaded();

function MarkdownBody({ text }: { text: string }) {
  const [ready, setReady] = useState(_mdReady);
  useEffect(() => { if (!ready) ensureMarkdownLoaded().then(() => setReady(true)); }, [ready]);

  if (!ready || !_ReactMarkdown) {
    return <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>{text}</Typography>;
  }

  const Md = _ReactMarkdown;
  return (
    <Md
      remarkPlugins={_remarkGfm ? [_remarkGfm as never] : []}
      components={{
        h1: ({ children }: { children?: React.ReactNode }) => <Typography className="chat-md-heading chat-md-h1">{children}</Typography>,
        h2: ({ children }: { children?: React.ReactNode }) => <Typography className="chat-md-heading chat-md-h2">{children}</Typography>,
        h3: ({ children }: { children?: React.ReactNode }) => <Typography className="chat-md-heading chat-md-h3">{children}</Typography>,
        h4: ({ children }: { children?: React.ReactNode }) => <Typography className="chat-md-heading chat-md-h4">{children}</Typography>,
        p: ({ children }: { children?: React.ReactNode }) => <Typography variant="body2" className="chat-md-paragraph">{children}</Typography>,
        a: ({ href, children }: { href?: string; children?: React.ReactNode }) => <a className="chat-md-link" href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
        code: ({ className, children }: { className?: string; children?: React.ReactNode }) => {
          const isBlock = className?.startsWith("language-");
          if (!isBlock) return <code className="chat-md-inline-code">{children}</code>;
          const lang = className?.replace("language-", "") || "";
          return (
            <Box className="chat-md-code-wrap">
              {lang ? <div className="chat-md-code-lang">{lang}</div> : null}
              <pre className="chat-md-code"><code>{children}</code></pre>
            </Box>
          );
        },
        ul: ({ children }: { children?: React.ReactNode }) => <Box component="ul" className="chat-md-list">{children}</Box>,
        ol: ({ children }: { children?: React.ReactNode }) => <Box component="ol" className="chat-md-list">{children}</Box>,
        table: ({ children }: { children?: React.ReactNode }) => (
          <Box sx={{ overflowX: "auto", my: 1 }}>
            <table style={{ borderCollapse: "collapse", width: "100%", fontSize: "0.85rem" }}>{children}</table>
          </Box>
        ),
        th: ({ children }: { children?: React.ReactNode }) => <th style={{ border: "1px solid rgba(255,255,255,0.12)", padding: "6px 10px", textAlign: "left", fontWeight: 600 }}>{children}</th>,
        td: ({ children }: { children?: React.ReactNode }) => <td style={{ border: "1px solid rgba(255,255,255,0.08)", padding: "6px 10px" }}>{children}</td>,
        blockquote: ({ children }: { children?: React.ReactNode }) => (
          <Box sx={{ borderLeft: "3px solid rgba(47,212,255,0.4)", pl: 1.5, my: 0.5, color: "text.secondary" }}>{children}</Box>
        ),
        hr: () => <Box component="hr" sx={{ border: "none", borderTop: "1px solid rgba(255,255,255,0.08)", my: 1 }} />,
      } as Record<string, unknown>}
    >
      {text}
    </Md>
  );
}

function renderChatMarkdown(text: string): ReactNode {
  if (!text?.trim()) return null;
  return (
    <Box className="chat-markdown">
      <MarkdownBody text={text} />
    </Box>
  );
}

function extractFirstCodeFence(text: string): string {
  const source = (text || "").trim();
  if (!source) return "";
  const match = source.match(/```[a-zA-Z0-9_+-]*\n([\s\S]*?)```/);
  if (match && match[1]) return match[1].trim();
  return "";
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
  "htm"
]);

function splitSupportedChatAttachments(files: File[]): { accepted: File[]; rejected: string[] } {
  const accepted: File[] = [];
  const rejected: string[] = [];
  for (const file of files) {
    const name = (file.name || "").trim();
    const dotIdx = name.lastIndexOf(".");
    const ext = dotIdx >= 0 ? name.slice(dotIdx + 1).toLowerCase() : "";
    if (CHAT_ATTACHMENT_EXTENSIONS.has(ext)) {
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
        const nested = str(parsed.error, "").trim() || str(parsed.message, "").trim();
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

function defaultSkillEditorForm(name = ""): SkillEditorForm {
  return {
    name: name || "new-action",
    description: "",
    version: "1.0.0",
    requiredInputsCsv: "",
    emoji: "",
    toolsCsv: "",
    workflow: ""
  };
}

function splitActionFrontmatter(content: string): { frontmatter: string | null; body: string } {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!match) return { frontmatter: null, body: content };
  return { frontmatter: match[1] ?? "", body: match[2] ?? "" };
}

function unquoteYamlScalar(value: string): string {
  const v = value.trim();
  if (!v) return "";
  if (v.startsWith("\"") && v.endsWith("\"")) {
    try {
      const parsed = JSON.parse(v);
      return typeof parsed === "string" ? parsed : v.slice(1, -1);
    } catch {
      return v.slice(1, -1);
    }
  }
  if (v.startsWith("'") && v.endsWith("'")) return v.slice(1, -1).replace(/''/g, "'");
  return v;
}

function quoteYamlScalar(value: string): string {
  return JSON.stringify(value ?? "");
}

function parseInlineStringArray(value: string): string[] {
  const trimmed = value.trim();
  if (!trimmed) return [];
  if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
    try {
      const parsed = JSON.parse(trimmed);
      if (Array.isArray(parsed)) {
        return parsed
          .map((item) => (typeof item === "string" ? item.trim() : ""))
          .filter(Boolean);
      }
    } catch {
      // Fall through to a tolerant split below.
    }
    const raw = trimmed.slice(1, -1);
    return raw
      .split(",")
      .map((item) => unquoteYamlScalar(item))
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return trimmed
    .split(",")
    .map((item) => unquoteYamlScalar(item))
    .map((item) => item.trim())
    .filter(Boolean);
}

function dedupeStrings(values: string[]): string[] {
  return Array.from(new Set(values.map((item) => item.trim()).filter(Boolean)));
}

function parseToolsCsv(csv: string): string[] {
  return dedupeStrings(
    csv
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
  );
}

function parseRequiredInputsCsv(csv: string): string[] {
  return dedupeStrings(
    csv
      .split(",")
      .map((item) =>
        item
          .trim()
          .replace(/[^A-Za-z0-9_-]/g, "")
      )
      .filter(Boolean)
  );
}

type HookTriggerValue =
  | "pre_message"
  | "post_message"
  | "pre_action"
  | "post_action"
  | "on_consolidate"
  | "on_error";

function sanitizeHookName(value: string): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9-_\s]/g, "")
    .replace(/[_\s]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function inferHookTriggerFromInstruction(text: string, defaultTrigger: HookTriggerValue = "post_action"): HookTriggerValue {
  const t = (text || "").toLowerCase();
  if (!t) return defaultTrigger;
  if (t.includes("on error") || t.includes("error") || t.includes("fail")) return "on_error";
  if (t.includes("before action") || t.includes("pre action")) return "pre_action";
  if (t.includes("after action") || t.includes("post action") || t.includes("on success") || t.includes("when done")) return "post_action";
  if (t.includes("before message") || t.includes("pre message")) return "pre_message";
  if (t.includes("after message") || t.includes("post message") || t.includes("after reply")) return "post_message";
  if (t.includes("consolidate") || t.includes("memory")) return "on_consolidate";
  return defaultTrigger;
}

function extractFirstUrl(text: string): string {
  const m = (text || "").match(/https?:\/\/[^\s]+/i);
  return m ? m[0] : "";
}

function extractCronExpression(text: string): string {
  const tokens = (text || "")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const isCronToken = (token: string) => /^[0-9A-Za-z*/,\-]+$/.test(token);
  for (let i = 0; i < tokens.length; i += 1) {
    for (const width of [6, 5]) {
      if (i + width > tokens.length) continue;
      const slice = tokens.slice(i, i + width);
      if (slice.every(isCronToken)) {
        return slice.join(" ");
      }
    }
  }
  return "";
}

function inferTaskCronFromInstruction(text: string): string {
  const t = (text || "").trim().toLowerCase();
  if (!t) return "";
  const explicitCron = extractCronExpression(text);
  if (explicitCron) return explicitCron;

  if (t.includes("every 5") && t.includes("min")) return "*/5 * * * *";
  if (t.includes("every 10") && t.includes("min")) return "*/10 * * * *";
  if (t.includes("every 15") && t.includes("min")) return "*/15 * * * *";
  if (t.includes("every 30") && t.includes("min")) return "*/30 * * * *";
  if (t.includes("hourly") || t.includes("every hour")) return "0 * * * *";
  if (t.includes("weekday")) return "0 9 * * 1-5";
  if (t.includes("weekly")) return "0 9 * * 1";
  if (t.includes("monthly")) return "0 9 1 * *";
  if (t.includes("daily") || t.includes("every day")) return "0 9 * * *";
  return "";
}

function isRunOnceInstruction(text: string): boolean {
  const t = (text || "").toLowerCase();
  return t.includes("once") || t.includes("now") || t.includes("immediately");
}

function isHookAttachedToAction(hookName: string, actionName: string): boolean {
  const hn = sanitizeHookName(hookName);
  const an = sanitizeHookName(actionName);
  if (!hn || !an) return false;
  return hn.startsWith(`action-${an}-`);
}

function isHookRecordAttachedToAction(hook: JsonRecord, actionName: string): boolean {
  const explicit = sanitizeHookName(str(hook.action_name, ""));
  const an = sanitizeHookName(actionName);
  if (explicit && an && explicit === an) return true;
  return isHookAttachedToAction(str(hook.name, ""), actionName);
}

function parseSkillEditorForm(content: string, fallbackName: string): SkillEditorForm {
  const { frontmatter, body } = splitActionFrontmatter(content);
  const form = defaultSkillEditorForm(fallbackName);
  if (!frontmatter) {
    form.workflow = content.trim();
    return form;
  }

  const tools: string[] = [];
  const requiredInputs: string[] = [];
  let section: string | null = null;
  let listTarget: "tools" | "requiredInputs" | null = null;
  const lines = frontmatter.split(/\r?\n/);
  for (const rawLine of lines) {
    const line = rawLine.replace(/\t/g, "  ");
    if (!line.trim()) continue;

    const top = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (top) {
      const key = top[1];
      const value = top[2].trim();
      section = null;
      listTarget = null;

      if (key === "name") {
        if (value) form.name = unquoteYamlScalar(value);
        continue;
      }
      if (key === "description") {
        if (value) form.description = unquoteYamlScalar(value);
        continue;
      }
      if (key === "version") {
        if (value) form.version = unquoteYamlScalar(value);
        continue;
      }
      if (key === "required_inputs" || key === "requiredInputs" || key === "required") {
        if (value) {
          requiredInputs.push(...parseInlineStringArray(value));
        } else {
          section = "required_inputs";
          listTarget = "requiredInputs";
        }
        continue;
      }
      if (key === "metadata") {
        if (value) {
          const m = value.match(/emoji\s*:\s*(.+)$/);
          if (m) form.emoji = unquoteYamlScalar(m[1]);
        } else {
          section = "metadata";
        }
        continue;
      }
      if (key === "requires") {
        if (value) {
          const m = value.match(/tools\s*:\s*(.+)$/);
          if (m) tools.push(...parseInlineStringArray(m[1]));
        } else {
          section = "requires";
        }
        continue;
      }
      continue;
    }

    const nested = line.match(/^\s{2,}([A-Za-z0-9_-]+):\s*(.*)$/);
    if (nested && section) {
      const key = nested[1];
      const value = nested[2].trim();
      listTarget = null;
      if (section === "metadata" && key === "emoji") {
        form.emoji = unquoteYamlScalar(value);
        continue;
      }
      if (section === "requires" && key === "tools") {
        if (value) {
          tools.push(...parseInlineStringArray(value));
        } else {
          listTarget = "tools";
        }
        continue;
      }
      continue;
    }

    const listItem = line.match(/^\s*-\s*(.+)$/);
    if (listItem && section === "requires" && listTarget === "tools") {
      tools.push(unquoteYamlScalar(listItem[1]));
      continue;
    }
    if (listItem && section === "required_inputs" && listTarget === "requiredInputs") {
      requiredInputs.push(unquoteYamlScalar(listItem[1]));
      continue;
    }
  }

  form.toolsCsv = dedupeStrings(tools).join(", ");
  form.requiredInputsCsv = parseRequiredInputsCsv(requiredInputs.join(", ")).join(", ");
  form.workflow = body.trim();
  if (!form.name.trim()) form.name = fallbackName || "new-action";
  if (!form.version.trim()) form.version = "1.0.0";
  return form;
}

function extractUnknownFrontmatterLines(frontmatter: string): string[] {
  const lines = frontmatter.split(/\r?\n/);
  const kept: string[] = [];
  let skipKnownBlock = false;

  for (const line of lines) {
    const top = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (top) {
      const key = top[1];
      if (
        key === "name" ||
        key === "description" ||
        key === "version" ||
        key === "required_inputs" ||
        key === "requiredInputs" ||
        key === "required"
      ) {
        skipKnownBlock = false;
        continue;
      }
      if (key === "metadata" || key === "requires") {
        skipKnownBlock = true;
        continue;
      }
      skipKnownBlock = false;
      kept.push(line);
      continue;
    }

    if (skipKnownBlock) {
      if (line.trim() === "" || /^\s+/.test(line)) continue;
    }
    kept.push(line);
  }

  while (kept.length > 0 && !kept[0].trim()) kept.shift();
  while (kept.length > 0 && !kept[kept.length - 1].trim()) kept.pop();
  return kept;
}

function buildSkillMdFromForm(currentContent: string, form: SkillEditorForm): string {
  const { frontmatter } = splitActionFrontmatter(currentContent);
  const unknownLines = frontmatter ? extractUnknownFrontmatterLines(frontmatter) : [];
  const tools = parseToolsCsv(form.toolsCsv);
  const requiredInputs = parseRequiredInputsCsv(form.requiredInputsCsv);
  const frontmatterLines = [
    `name: ${quoteYamlScalar((form.name || "").trim())}`,
    `description: ${quoteYamlScalar((form.description || "").trim())}`,
    `version: ${quoteYamlScalar((form.version || "").trim() || "1.0.0")}`,
    `required_inputs: [${requiredInputs.map((item) => quoteYamlScalar(item)).join(", ")}]`,
    "metadata:",
    `  emoji: ${quoteYamlScalar((form.emoji || "").trim())}`,
    "requires:",
    `  tools: [${tools.map((tool) => quoteYamlScalar(tool)).join(", ")}]`
  ];

  if (unknownLines.length > 0) {
    frontmatterLines.push("");
    frontmatterLines.push(...unknownLines);
  }

  const workflow = (form.workflow || "").trim();
  return `---\n${frontmatterLines.join("\n")}\n---\n\n${workflow}\n`;
}

function normalizeActionName(value: string): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9-_\s]/g, "")
    .replace(/[_\s]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function isValidActionName(value: string): boolean {
  return /^[a-z0-9-]+$/.test((value || "").trim());
}

function extractActionMdFromModelOutput(text: string): string {
  const raw = (text || "").trim();
  if (!raw) return "";

  // Prefer fenced markdown/code blocks when present.
  const fenceRegex = /```(?:md|markdown|txt|yaml)?\s*([\s\S]*?)```/gi;
  const blocks: string[] = [];
  let match: RegExpExecArray | null = null;
  while ((match = fenceRegex.exec(raw)) !== null) {
    blocks.push((match[1] || "").trim());
  }
  for (const block of blocks) {
    if (block.startsWith("---")) return block;
  }
  if (blocks.length > 0) return blocks[0];

  if (raw.startsWith("---")) return raw;
  const frontmatterIdx = raw.indexOf("\n---\n");
  if (frontmatterIdx >= 0) {
    const maybe = raw.slice(raw.lastIndexOf("---", frontmatterIdx - 1)).trim();
    if (maybe.startsWith("---")) return maybe;
  }
  return raw;
}

function formatCompactValue(value: unknown): { text: string; tooltip?: string } {
  if (value == null) return { text: "-" };
  if (typeof value === "string") return { text: value };
  if (typeof value === "number") return { text: Number.isFinite(value) ? String(value) : "-" };
  if (typeof value === "boolean") return { text: value ? "true" : "false" };

  if (Array.isArray(value)) {
    // Avoid dumping JSON; just give a hint.
    const sample = value
      .slice(0, 4)
      .map((v) => (typeof v === "string" ? v : typeof v === "number" ? String(v) : typeof v === "boolean" ? (v ? "true" : "false") : "…"))
      .join(", ");
    const tooltip = sample ? `Examples: ${sample}${value.length > 4 ? ` (+${value.length - 4})` : ""}` : undefined;
    return { text: `List (${value.length})`, tooltip };
  }

  if (typeof value === "object") {
    const rec = asRecord(value);
    const title = str(rec.title, "") || str(rec.name, "") || str(rec.id, "");
    const keys = Object.keys(rec);
    const keyHint = keys.slice(0, 4).join(", ");
    const more = keys.length > 4 ? `, +${keys.length - 4}` : "";
    const tooltip = keys.length ? `Fields: ${keyHint}${more}` : undefined;
    if (title) return { text: title, tooltip };
    return { text: keys.length ? `Object(${keyHint}${more})` : "Object", tooltip };
  }

  return { text: String(value) };
}

function looksLikeUrl(value: string): boolean {
  const v = (value || "").trim();
  return v.startsWith("http://") || v.startsWith("https://");
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

function extractAccessKeyFromUrl(pathOrUrl: string, baseOrigin: string): string {
  const value = (pathOrUrl || "").trim();
  if (!value) return "";
  try {
    const fallbackBase = (baseOrigin || "http://localhost").trim();
    const parsed = new URL(value, fallbackBase);
    return (parsed.searchParams.get("key") || "").trim();
  } catch {
    return "";
  }
}

function dedupeLinkTargets(targets: Array<{ label: string; url: string }>): Array<{ label: string; url: string }> {
  const seen = new Set<string>();
  const out: Array<{ label: string; url: string }> = [];
  for (const item of targets) {
    const url = (item.url || "").trim();
    if (!url || seen.has(url)) continue;
    seen.add(url);
    out.push({ label: item.label, url });
  }
  return out;
}

function looksLikeUuid(value: string): boolean {
  const v = (value || "").trim();
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(v);
}

function looksLikeIsoTimestamp(value: string): boolean {
  const v = (value || "").trim();
  if (!/^\d{4}-\d{2}-\d{2}T/.test(v)) return false;
  const dt = new Date(v);
  return !Number.isNaN(dt.getTime());
}

function formatTimestampForHumans(value: string): { label: string; tooltip: string } {
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return { label: value, tooltip: value };
  const now = new Date();
  const sameYear = dt.getFullYear() === now.getFullYear();
  const fmt = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    ...(sameYear ? {} : { year: "numeric" }),
    hour: "2-digit",
    minute: "2-digit"
  });
  return { label: fmt.format(dt), tooltip: value };
}

/** Format a trace step time string to local human-readable.
 *  Input: ISO timestamp like "2026-03-16T09:02:40Z" or "2026-03-16T09:02:40Z (1396ms)"
 *  Output: "12 Mar, 2:32 PM (1396ms)" — local time with optional duration
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
  const time = dt.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });
  return durationPart ? `${time} ${durationPart}` : time;
}

function formatRelativeTimeFromNow(date: Date): string {
  const diffMs = Date.now() - date.getTime();
  const isPast = diffMs >= 0;
  const absMs = Math.abs(diffMs);
  const absSec = Math.round(absMs / 1000);
  const unit = (count: number, singular: string, plural: string) =>
    `${count} ${count === 1 ? singular : plural}`;

  if (absSec < 30) return "just now";

  const absMin = Math.round(absSec / 60);
  if (absMin < 60) {
    const display = unit(absMin, "minute", "minutes");
    return isPast ? `${display} ago` : `in ${display}`;
  }

  const absHours = Math.round(absMin / 60);
  if (absHours < 24) {
    const display = unit(absHours, "hour", "hours");
    return isPast ? `${display} ago` : `in ${display}`;
  }

  const absDays = Math.round(absHours / 24);
  if (absDays < 7) {
    const display = unit(absDays, "day", "days");
    return isPast ? `${display} ago` : `in ${display}`;
  }

  const absWeeks = Math.round(absDays / 7);
  if (absWeeks < 5) {
    const display = unit(absWeeks, "week", "weeks");
    return isPast ? `${display} ago` : `in ${display}`;
  }

  const absMonths = Math.round(absDays / 30);
  if (absMonths < 12) {
    const display = unit(absMonths, "month", "months");
    return isPast ? `${display} ago` : `in ${display}`;
  }

  const absYears = Math.round(absDays / 365);
  const display = unit(absYears, "year", "years");
  return isPast ? `${display} ago` : `in ${display}`;
}

function formatChatTimestamp(value: string): { label: string; tooltip: string } {
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return { label: value, tooltip: value };

  const absolute = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit"
  }).format(dt);
  const relative = formatRelativeTimeFromNow(dt);
  const tooltip = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    timeZoneName: "short"
  }).format(dt);

  return { label: `${absolute} · ${relative}`, tooltip };
}

/** Format any ISO timestamp string into a human-readable relative label with absolute tooltip. */
function humanTs(raw: string): { label: string; tip: string } {
  const v = (raw || "").trim();
  if (!v || v === "-") return { label: "-", tip: "" };
  const dt = new Date(v);
  if (Number.isNaN(dt.getTime())) return { label: v, tip: v };
  const label = formatRelativeTimeFromNow(dt);
  const tip = new Intl.DateTimeFormat(undefined, {
    month: "short", day: "2-digit", year: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit", timeZoneName: "short",
  }).format(dt);
  return { label, tip };
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

function boolLabelForKey(key: string, value: boolean): { label: string; color: "success" | "warning" | "default" } {
  const k = (key || "").toLowerCase();
  if (k.includes("enabled")) return { label: value ? "Enabled" : "Disabled", color: value ? "success" : "warning" };
  if (k.includes("active")) return { label: value ? "Active" : "Inactive", color: value ? "success" : "warning" };
  if (k.includes("connected")) return { label: value ? "Connected" : "Not connected", color: value ? "success" : "warning" };
  return { label: value ? "Yes" : "No", color: value ? "success" : "default" };
}

function DataTable({ rows, columns }: { rows: JsonRecord[]; columns: string[] }) {
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
                      return (
                        <span title={out.tooltip || ""}>
                          {out.text}
                        </span>
                      );
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
  maxRows
}: {
  title: string;
  data: JsonRecord;
  emptyLabel?: string;
  maxRows?: number;
}) {
  const entries = Object.entries(data || {});
  const shown = entries.slice(0, maxRows ?? 14);
  return (
    <Box className="metadata-box">
      <Typography variant="caption" color="text.secondary">
        {title}
      </Typography>
      <Stack spacing={0.6} sx={{ mt: 0.75 }}>
        {shown.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            {emptyLabel || "No details available."}
          </Typography>
        ) : (
          shown.map(([k, v]) => {
            const out = formatCompactValue(v);
            const keyLower = (k || "").toLowerCase();
            const renderValue = () => {
              if (typeof v === "string" && looksLikeUrl(v)) {
                const trimmed = v.trim();
                const label = trimmed.length > 54 ? `${trimmed.slice(0, 54)}…` : trimmed;
                return (
                  <Typography variant="body2" sx={{ wordBreak: "break-all" }} title={trimmed}>
                    <a href={trimmed} target="_blank" rel="noreferrer" style={{ color: "inherit", textDecoration: "underline" }}>
                      {label}
                    </a>
                  </Typography>
                );
              }
              if (typeof v === "string" && (looksLikeIsoTimestamp(v) || keyLower.endsWith("_at") || keyLower.includes("timestamp"))) {
                const t = formatTimestampForHumans(v);
                return <Chip size="small" variant="outlined" label={t.label} title={t.tooltip} />;
              }
              if (typeof v === "boolean") {
                const b = boolLabelForKey(k, v);
                return <Chip size="small" label={b.label} color={b.color} variant={v ? "filled" : "outlined"} />;
              }
              if (typeof v === "number" && Number.isFinite(v)) {
                if (keyLower.includes("ms") || keyLower.includes("duration")) {
                  return <Chip size="small" variant="outlined" label={`${Math.round(v)} ms`} />;
                }
                if (keyLower.includes("count") || keyLower.includes("total") || keyLower.includes("remaining")) {
                  return <Chip size="small" variant="outlined" label={String(v)} />;
                }
              }
              if (typeof v === "string" && (looksLikeUuid(v) || keyLower.endsWith("_id") || keyLower === "id")) {
                const trimmed = v.trim();
                const label = trimmed.length > 22 ? `${trimmed.slice(0, 8)}…${trimmed.slice(-6)}` : trimmed;
                return (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={label}
                    title={trimmed}
                    onClick={async () => {
                      try {
                        await navigator.clipboard.writeText(trimmed);
                      } catch {
                        // ignore
                      }
                    }}
                    sx={{ cursor: "pointer" }}
                  />
                );
              }
              return (
                <Typography
                  variant="body2"
                  sx={{ minWidth: 0, flex: "1 1 auto", wordBreak: "break-word" }}
                  title={out.tooltip || ""}
                >
                  {out.text}
                </Typography>
              );
            };
            return (
              <Stack key={k} direction="row" spacing={1} alignItems="baseline">
                <Typography variant="caption" color="text.secondary" sx={{ width: 160, flex: "0 0 auto" }}>
                  {k}
                </Typography>
                {renderValue()}
              </Stack>
            );
          })
        )}
        {entries.length > shown.length ? (
          <Typography variant="caption" color="text.secondary">
            {entries.length - shown.length} more field(s) not shown.
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
}

type BulkImportItem = {
  url: string;
  selected: boolean;
  analyzed: boolean;
  status?: string;
  discovered: BulkImportDiscoveredSkill[];
};

type BulkImportDiscoveredSkill = {
  key: string;
  parentUrl: string;
  url: string;
  name: string;
  selected: boolean;
  status?: string;
  preview?: SkillImportResponse;
  importResult?: SkillImportResponse;
  error?: string;
};

function BulkImportDialog({
  open,
  onClose,
  onImported,
  onAfterImport
}: {
  open: boolean;
  onClose: () => void;
  onImported?: ImportCallback;
  onAfterImport?: (name: string, importResult: SkillImportResponse) => Promise<void>;
}) {
  const [urlsText, setUrlsText] = useState("");
  const [items, setItems] = useState<BulkImportItem[]>([]);
  const [analyzing, setAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [force, setForce] = useState(false);
  const [model, setModel] = useState("");
  const [analysisDone, setAnalysisDone] = useState(false);

  const parseUrlsFromText = (text: string): string[] => {
    const urls = text
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith("#"))
      .map((u) => {
        // Auto-fix common GitHub mistake: /blob/ → /tree/ for folder URLs
        if (u.includes("github.com/") && u.includes("/blob/") && !u.match(/\.\w+$/)) {
          return u.replace("/blob/", "/tree/");
        }
        return u;
      });
    const uniq: string[] = [];
    for (const u of urls) {
      if (!uniq.includes(u)) uniq.push(u);
    }
    return uniq;
  };

  const requiresForceForResult = (result: SkillImportResponse | undefined): boolean => {
    if (!result) return false;
    const risk = computeImportRiskSummary(result.security);
    const blocked = toBool(result.security?.blocked) || result.status === "blocked";
    return blocked || risk.score10 >= IMPORT_SECURITY_FORCE_RISK_THRESHOLD;
  };

  const normalizeDiscoveredSkills = (
    sourceUrl: string,
    previewResult: SkillImportResponse
  ): BulkImportDiscoveredSkill[] => {
    const importedChildren = Array.isArray(previewResult.imported) ? previewResult.imported : [];
    if (importedChildren.length > 0) {
      return importedChildren.map((entry, idx) => {
        const childResult = entry?.result;
        const childUrl = str(entry?.url, sourceUrl);
        const childName = str(childResult?.name, `skill-${idx + 1}`);
        const needsForce = requiresForceForResult(childResult);
        return {
          key: `${sourceUrl}::${childUrl}::${idx}`,
          parentUrl: sourceUrl,
          url: childUrl,
          name: childName,
          selected: force || !needsForce,
          status:
            str(childResult?.message, "") ||
            (childResult?.status === "blocked"
              ? "Blocked by security verification"
              : "Preview ready"),
          preview: childResult
        };
      });
    }

    const singleNeedsForce = requiresForceForResult(previewResult);
    return [
      {
        key: `${sourceUrl}::single`,
        parentUrl: sourceUrl,
        url: sourceUrl,
        name: str(previewResult.name, "imported-skill"),
        selected: force || !singleNeedsForce,
        status:
          str(previewResult.message, "") ||
          (previewResult.status === "blocked" ? "Blocked by security verification" : "Preview ready"),
        preview: previewResult
      }
    ];
  };

  const buildItemsFromText = (): BulkImportItem[] =>
    parseUrlsFromText(urlsText).map((url) => ({
      url,
      selected: true,
      analyzed: false,
      discovered: []
    }));

  const selectedDiscoveredSkills: BulkImportDiscoveredSkill[] = items.flatMap((item) =>
    item.selected ? item.discovered.filter((skill) => skill.selected) : []
  );
  const selectedSkillCount = selectedDiscoveredSkills.length;
  const riskySelectedCount = selectedDiscoveredSkills.filter((skill) =>
    requiresForceForResult(skill.preview)
  ).length;

  useEffect(() => {
    if (!open) {
      setError(null);
      setAnalyzing(false);
      setImporting(false);
      setAnalysisDone(false);
      return;
    }
    setUrlsText("");
    setItems([]);
    setAnalyzing(false);
    setImporting(false);
    setAnalysisDone(false);
    setError(null);
    setForce(false);
    setModel("");
  }, [open]);

  const updateDiscoveredSkill = (
    parentUrl: string,
    skillKey: string,
    patch: Partial<BulkImportDiscoveredSkill>
  ) => {
    setItems((prev) =>
      prev.map((item) => {
        if (item.url !== parentUrl) return item;
        return {
          ...item,
          discovered: item.discovered.map((skill) =>
            skill.key === skillKey ? { ...skill, ...patch } : skill
          )
        };
      })
    );
  };

  const handleAnalyzeSelected = async () => {
    setError(null);
    setAnalysisDone(false);
    const effectiveItems = items.length > 0 ? items : buildItemsFromText();
    if (!effectiveItems.length) {
      setError("Paste at least one import URL before analyzing.");
      return;
    }
    const toAnalyze = effectiveItems.filter((item) => item.selected);
    if (!toAnalyze.length) {
      setError("Select at least one URL to analyze.");
      return;
    }
    if (items.length === 0) setItems(effectiveItems);

    setAnalyzing(true);
    for (const item of toAnalyze) {
      setItems((prev) =>
        prev.map((x) => (x.url === item.url ? { ...x, status: "Analyzing security...", analyzed: false } : x))
      );
      try {
        const preview = await api.importSkill({
          url: item.url,
          force,
          model: model.trim() || undefined,
          preview_only: true
        });
        const discovered = normalizeDiscoveredSkills(item.url, preview);
        setItems((prev) =>
          prev.map((x) =>
            x.url === item.url
              ? {
                  ...x,
                  analyzed: true,
                  status:
                    preview.message ||
                    `Previewed ${discovered.length} skill${discovered.length === 1 ? "" : "s"}.`,
                  discovered
                }
              : x
          )
        );
      } catch (err) {
        const message = `Error: ${errMessage(err)}`;
        setItems((prev) =>
          prev.map((x) =>
            x.url === item.url
              ? {
                  ...x,
                  analyzed: true,
                  status: message,
                  discovered: []
                }
              : x
          )
        );
      }
    }
    setAnalyzing(false);
    setAnalysisDone(true);
  };

  const handleImportSelected = async () => {
    setError(null);
    if (!analysisDone) {
      setError("Analyze selected URLs first so you can review security and choose which skills to import.");
      return;
    }
    if (!selectedSkillCount) {
      setError("Select at least one skill to import.");
      return;
    }
    if (!force && riskySelectedCount > 0) {
      setError(
        `Selected set includes ${riskySelectedCount} risky skill(s). Enable override or deselect them before importing.`
      );
      return;
    }

    const selectedByParent = new Map<string, BulkImportDiscoveredSkill[]>();
    for (const skill of selectedDiscoveredSkills) {
      const bucket = selectedByParent.get(skill.parentUrl) || [];
      bucket.push(skill);
      selectedByParent.set(skill.parentUrl, bucket);
    }

    setImporting(true);
    for (const [parentUrl, selectedSkills] of selectedByParent.entries()) {
      for (const skill of selectedSkills) {
        updateDiscoveredSkill(skill.parentUrl, skill.key, {
          status: "Importing...",
          error: undefined
        });
      }
      try {
        const result = await api.importSkill({
          url: parentUrl,
          force,
          model: model.trim() || undefined,
          selected_urls: selectedSkills.map((skill) => skill.url)
        });

        const importedEntries = Array.isArray(result.imported) ? result.imported : [];
        const importedByUrl = new Map<string, SkillImportResponse>();
        for (const entry of importedEntries) {
          const childResult = entry?.result;
          const childUrl = str(entry?.url, "").trim();
          if (!childResult || !childUrl) continue;
          importedByUrl.set(childUrl, childResult);
        }

        if (importedEntries.length > 0) {
          for (const skill of selectedSkills) {
            const childResult = importedByUrl.get(skill.url);
            if (!childResult) {
              updateDiscoveredSkill(skill.parentUrl, skill.key, {
                status: "Error: selected skill was not returned by bulk import response."
              });
              continue;
            }
            let childMessage = childResult.message || `Imported ${childResult.name}`;
            if (childResult.status === "blocked") {
              childMessage =
                childResult.message || "Blocked by security verification (enable override and retry).";
            } else if (childResult.status === "needs_secrets") {
              childMessage =
                childResult.message ||
                `Imported ${childResult.name} (disabled until secrets are configured)`;
            }

            updateDiscoveredSkill(skill.parentUrl, skill.key, {
              status: childMessage,
              importResult: childResult
            });
            await onAfterImport?.(childResult.name, childResult);
            await onImported?.({ result: childResult, message: childMessage });
          }
        } else {
          let statusMessage = result.message || `Imported ${result.name}`;
          if (result.status === "blocked") {
            statusMessage = result.message || "Blocked by security verification (enable override and retry).";
          } else if (result.status === "needs_secrets") {
            statusMessage = result.message || `Imported ${result.name} (disabled until secrets are configured)`;
          }
          for (const skill of selectedSkills) {
            updateDiscoveredSkill(skill.parentUrl, skill.key, {
              status: statusMessage,
              importResult: result
            });
          }
          await onAfterImport?.(result.name, result);
          await onImported?.({ result, message: statusMessage });
        }
      } catch (err) {
        const message = `Error: ${errMessage(err)}`;
        for (const skill of selectedSkills) {
          updateDiscoveredSkill(skill.parentUrl, skill.key, { status: message, error: message });
        }
      }
    }
    setImporting(false);
    onClose();
  };

  const titleCaseWords = (value: string): string =>
    value
      .split(/[\s_-]+/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");

  const compactStatusToken = (value: string): string => value.toLowerCase().replace(/[^a-z0-9]+/g, "");

  const buildBulkSkillStatus = (
    skill: BulkImportDiscoveredSkill
  ): { label: string; color: "default" | "success" | "warning" | "error" | "info"; detail: string } => {
    const rawStatus = str(skill.status, "").trim();
    const rawLower = rawStatus.toLowerCase();
    const result = skill.importResult || skill.preview;
    const blocked = toBool(result?.security?.blocked) || result?.status === "blocked";

    if (rawLower === "importing...") {
      return { label: "Importing", color: "info", detail: "" };
    }
    if (skill.error || rawLower.startsWith("error")) {
      return {
        label: "Import failed",
        color: "error",
        detail: rawStatus.replace(/^error:\s*/i, "").trim() || "Import failed."
      };
    }
    if (blocked) {
      return {
        label: "Blocked",
        color: "error",
        detail: rawStatus || str(result?.message, "Blocked by security verification.")
      };
    }
    if (result?.status === "needs_secrets") {
      return {
        label: "Needs secrets",
        color: "warning",
        detail:
          rawStatus ||
          str(result?.message, "Imported template is disabled until required secrets are configured.")
      };
    }
    if (skill.importResult && result?.status === "ok") {
      return {
        label: "Imported",
        color: "success",
        detail: rawStatus || str(result?.message, `Imported ${str(result?.name, "skill")}.`)
      };
    }
    if (skill.preview) {
      return {
        label: "Preview ready",
        color: "info",
        detail: rawStatus || str(result?.message, "Preview completed. Select and import when ready.")
      };
    }
    return {
      label: "Pending",
      color: "default",
      detail: rawStatus || "Waiting for analysis."
    };
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Bulk Import</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1.25}>
          {error ? <Alert severity="error">{error}</Alert> : null}
          <Typography variant="body2" color="text.secondary">
            Paste one or more skill URLs (one per line). Then run Analyze to review discovered skills and security before any import.
          </Typography>
          <Alert severity="info" variant="outlined" sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem" } }}>
            Getting 403 errors? GitHub rate-limits unauthenticated requests. Go to Settings &gt; Integrations &gt; GitHub and add a Personal Access Token for higher limits.
          </Alert>
          <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
            {`Examples:
https://github.com/org/repo/tree/main/skills
https://raw.githubusercontent.com/org/repo/main/skills/my-skill/SKILL.md
https://clawhub.ai/org/my-skill`}
          </Typography>
          <TextField
            fullWidth
            multiline
            minRows={3}
            maxRows={8}
            label="Import URLs"
            value={urlsText}
            onChange={(e) => {
              setUrlsText(e.target.value);
              setItems([]);
              setAnalysisDone(false);
              setError(null);
            }}
            placeholder={"https://github.com/openclaw/skills/tree/main/skills\nhttps://clawhub.ai/org/my-skill"}
          />
          <TextField
            fullWidth
            size="small"
            label="Model override (optional)"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
          <FormControlLabel
            control={<Switch checked={force} onChange={(e) => setForce(e.target.checked)} />}
            label="Override warnings (import anyway)"
          />
          {analysisDone ? (
            <Typography variant="caption" color="text.secondary">
              Selected for import: {selectedSkillCount} skill{selectedSkillCount === 1 ? "" : "s"}.
            </Typography>
          ) : null}
          {!force && riskySelectedCount > 0 ? (
            <Alert severity="warning">
              {riskySelectedCount} selected skill{riskySelectedCount === 1 ? "" : "s"} exceed the risk threshold ({IMPORT_SECURITY_FORCE_RISK_THRESHOLD}/10) or are blocked. Enable override or deselect those entries.
            </Alert>
          ) : null}
          {items.length > 0 ? (
            <Stack spacing={1}>
              {items.map((it) => (
                <Box key={it.url} className="bulk-import-source-card">
                  <Stack direction="row" spacing={1} alignItems="flex-start" className="bulk-import-source-header">
                    <Checkbox
                      size="small"
                      checked={it.selected}
                      onChange={(event) => {
                        const checked = event.target.checked;
                        setItems((prev) =>
                          prev.map((item) => (item.url === it.url ? { ...item, selected: checked } : item))
                        );
                      }}
                      disabled={analyzing || importing}
                    />
                    <Box sx={{ flex: 1, minWidth: 0 }}>
                      <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.25 }}>
                        Source URL
                      </Typography>
                      <Typography variant="body2" className="bulk-import-source-url">
                        {it.url}
                      </Typography>
                    </Box>
                  </Stack>
                  <Typography
                    variant="caption"
                    color={it.status?.startsWith("Error") ? "error" : "text.secondary"}
                    sx={{ display: "block", mt: 0.5 }}
                  >
                    {it.status || "Pending"}
                  </Typography>

                  {it.analyzed && it.discovered.length > 0 ? (
                    <TableContainer className="table-shell bulk-import-table-shell" sx={{ mt: 1 }}>
                      <Table size="small" className="bulk-import-table">
                        <TableHead>
                          <TableRow>
                            <TableCell padding="checkbox" className="bulk-import-col-select">Import</TableCell>
                            <TableCell className="bulk-import-col-skill">Skill</TableCell>
                            <TableCell className="bulk-import-col-source">Skill URL</TableCell>
                            <TableCell className="bulk-import-col-risk">Risk</TableCell>
                            <TableCell className="bulk-import-col-security">Security</TableCell>
                            <TableCell className="bulk-import-col-findings">Findings</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {it.discovered.map((skill) => {
                            const result = skill.importResult || skill.preview;
                            const risk = computeImportRiskSummary(result?.security);
                            const findingsCount = risk.totalFindings;
                            const blocked = toBool(result?.security?.blocked) || result?.status === "blocked";
                            const threatRaw = str(result?.security?.threat_level, "").trim().toLowerCase();
                            const threatLabel = threatRaw ? titleCaseWords(threatRaw) : "Unknown";
                            const threatColor =
                              threatRaw === "malicious"
                                ? "error"
                                : threatRaw === "suspicious" || threatRaw === "elevated"
                                ? "warning"
                                : threatRaw
                                ? "success"
                                : "default";
                            const securityLabel = blocked ? "Blocked" : risk.band === "secure" ? "Clean" : risk.bandLabel;
                            const securityColor = blocked ? "error" : risk.chipColor;
                            const statusMeta = buildBulkSkillStatus(skill);
                            const statusDetail =
                              compactStatusToken(statusMeta.label) === compactStatusToken(statusMeta.detail)
                                ? ""
                                : statusMeta.detail;
                            return (
                              <TableRow key={skill.key}>
                                <TableCell padding="checkbox">
                                  <Checkbox
                                    size="small"
                                    checked={skill.selected}
                                    onChange={(event) => {
                                      const checked = event.target.checked;
                                      setItems((prev) =>
                                        prev.map((item) => {
                                          if (item.url !== it.url) return item;
                                          return {
                                            ...item,
                                            discovered: item.discovered.map((entry) =>
                                              entry.key === skill.key ? { ...entry, selected: checked } : entry
                                            )
                                          };
                                        })
                                      );
                                    }}
                                    disabled={!it.selected || analyzing || importing}
                                  />
                                </TableCell>
                                <TableCell>
                                  <Typography variant="body2" className="bulk-import-wrap">
                                    {skill.name}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography variant="caption" color="text.secondary" className="bulk-import-wrap">
                                    {skill.url}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography variant="caption" sx={{ color: risk.chipColor === "success" ? "rgba(15,240,179,0.8)" : risk.chipColor === "error" ? "#ff5f57" : risk.chipColor === "warning" ? "#febc2e" : "rgba(180,220,200,0.6)" }}>
                                    {risk.score10.toFixed(1)}/10 · {risk.bandLabel}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography variant="caption" sx={{ color: threatColor === "success" ? "rgba(15,240,179,0.8)" : threatColor === "error" ? "#ff5f57" : threatColor === "warning" ? "#febc2e" : "rgba(180,220,200,0.6)" }}>
                                    {threatLabel} · {securityLabel}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography variant="caption" color={findingsCount > 0 ? "text.primary" : "text.secondary"}>
                                    {findingsCount === 0 ? "None" : `${findingsCount}${risk.contextualFindings > 0 ? ` (${risk.contextualFindings} contextual)` : ""}`}
                                  </Typography>
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  ) : null}
                </Box>
              ))}
            </Stack>
          ) : null}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
        <Button
          variant="outlined"
          disabled={importing || analyzing || !urlsText.trim()}
          onClick={handleAnalyzeSelected}
        >
          {analyzing ? "Analyzing..." : "Analyze"}
        </Button>
        <Button
          variant="contained"
          disabled={
            importing ||
            analyzing ||
            !analysisDone ||
            selectedSkillCount === 0 ||
            (!force && riskySelectedCount > 0)
          }
          onClick={handleImportSelected}
        >
          {importing ? "Importing..." : `Import Selected (${selectedSkillCount})`}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function ImportUrlDialog({
  open,
  onClose,
  onImported,
  onAfterImport
}: {
  open: boolean;
  onClose: () => void;
  onImported?: ImportCallback;
  onAfterImport?: (name: string, importResult: SkillImportResponse) => Promise<void>;
}) {
  const [url, setUrl] = useState("");
  const [model, setModel] = useState("");
  const [force, setForce] = useState(false);
  const [loading, setLoading] = useState(false);
  const [previewReady, setPreviewReady] = useState(false);
  const [importCommitted, setImportCommitted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [importResult, setImportResult] = useState<SkillImportResponse | null>(null);
  const [secretDrafts, setSecretDrafts] = useState<Record<string, { storeAs: string; value: string; useBuiltin: boolean }>>({});
  const [savingSecrets, setSavingSecrets] = useState(false);
  const [secretsSaved, setSecretsSaved] = useState(false);
  const importRisk = useMemo(
    () => computeImportRiskSummary(importResult?.security),
    [importResult]
  );
  const securityBlocked = toBool(importResult?.security?.blocked);
  const importRequiresForce =
    previewReady && !force && (securityBlocked || importRisk.score10 >= IMPORT_SECURITY_FORCE_RISK_THRESHOLD);

  const buildSecretDraftsFromResult = (result: SkillImportResponse) => {
    const required = result.secrets?.required_env || [];
    const bindings = result.secrets?.bindings || {};
    const drafts: Record<string, { storeAs: string; value: string; useBuiltin: boolean }> = {};
    for (const env of required) {
      const binding = bindings[env];
      drafts[env] = {
        storeAs: binding && binding !== "builtin" ? binding : env,
        value: "",
        useBuiltin: binding === "builtin"
      };
    }
    setSecretDrafts(drafts);
  };

  const runImport = async (previewOnly: boolean) => {
    if (!url.trim()) return;
    setLoading(true);
    setError(null);
    setInfo(null);
    setSecretsSaved(false);
    if (previewOnly) {
      setImportCommitted(false);
    }
    try {
      const result = await api.importSkill({
        url: url.trim(),
        model: model.trim() || undefined,
        force,
        preview_only: previewOnly
      });

      setImportResult(result);
      buildSecretDraftsFromResult(result);

      let message = result.message || (previewOnly ? `Preview ready for ${result.name}` : `Imported ${result.name}`);
      if (result.status === "blocked") {
        message = result.message || "Blocked by security verification. Enable override to continue.";
      } else if (!previewOnly && result.status === "needs_secrets") {
        message = result.message || `Imported ${result.name} (disabled until secrets are configured)`;
      }
      setInfo(message);

      if (previewOnly) {
        setPreviewReady(true);
        return;
      }

      setPreviewReady(false);
      setImportCommitted(true);

      // Auto-save secrets if the user pre-filled them during preview
      const requiredEnvs = result.secrets?.required_env || [];
      if (result.name && requiredEnvs.length > 0) {
        const filledSecrets = requiredEnvs
          .map((env) => {
            const d = secretDrafts[env] || { storeAs: env, value: "", useBuiltin: false };
            if (d.useBuiltin) return { env, store_as: "builtin" as const };
            const storeAs = (d.storeAs || env).trim();
            const value = (d.value || "").trim();
            return value ? { env, store_as: storeAs, value } : null;
          })
          .filter(Boolean);
        if (filledSecrets.length > 0) {
          try {
            const secretsOut = await api.setSkillSecrets(result.name, { secrets: filledSecrets as { env: string; store_as: string; value?: string }[] });
            if ((secretsOut.missing_env || []).length === 0) {
              setSecretsSaved(true);
              setInfo(`Imported ${result.name} — secrets saved automatically.`);
            }
          } catch { /* silent — user can still save manually */ }
        }
      }

      const importedChildren = Array.isArray(result.imported) ? result.imported : [];
      if (importedChildren.length > 0) {
        for (const child of importedChildren) {
          const childResult = child?.result;
          if (!childResult?.name) continue;
          const childMessage =
            childResult.message ||
            (childResult.status === "needs_secrets"
              ? `Imported ${childResult.name} (disabled until secrets are configured)`
              : `Imported ${childResult.name}`);
          await onAfterImport?.(childResult.name, childResult);
          await onImported?.({ result: childResult, message: childMessage });
        }
      } else {
        await onAfterImport?.(result.name, result);
        await onImported?.({ result, message });
      }
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setLoading(false);
    }
  };

  const handleAnalyze = async () => runImport(true);
  const handleImport = async () => runImport(false);

  const handleSaveSecrets = async () => {
    if (!importResult?.name) return;
    if (!importCommitted) {
      setError("Import template first, then save secrets.");
      return;
    }
    const required = importResult.secrets?.required_env || [];
    if (required.length === 0) return;
    setSavingSecrets(true);
    setError(null);
    try {
      const payload = required.map((env) => {
        const d = secretDrafts[env] || { storeAs: env, value: "", useBuiltin: false };
        if (d.useBuiltin) return { env, store_as: "builtin" };
        const storeAs = (d.storeAs || env).trim();
        const value = (d.value || "").trim();
        return value ? { env, store_as: storeAs, value } : { env, store_as: storeAs };
      });
      const secretsOut = await api.setSkillSecrets(importResult.name, { secrets: payload });
      if ((secretsOut.missing_env || []).length > 0) {
        setError(`Some keys are still missing: ${secretsOut.missing_env.join(", ")}`);
      } else {
        setSecretsSaved(true);
        setInfo("Secrets saved. The skill remains disabled until you manually enable it in Skills.");
      }
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setSavingSecrets(false);
    }
  };

  const handleClose = () => {
    if (loading) return;
    setError(null);
    setInfo(null);
    setImportResult(null);
    setPreviewReady(false);
    setImportCommitted(false);
    setSecretDrafts({});
    setSavingSecrets(false);
    setSecretsSaved(false);
    onClose();
  };

  return (
    <Dialog open={open} onClose={handleClose} maxWidth="sm" fullWidth>
      <DialogTitle>Import from URL</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1}>
          {error && <Alert severity="error">{error}</Alert>}
          {info && <Alert severity="info">{info}</Alert>}
          <Typography variant="caption" color="text.secondary">
            Supports direct SKILL.md links plus GitHub and ClawHub/OpenClaw skill page URLs.
          </Typography>
          <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
            {`Examples:
1. https://github.com/org/repo/tree/main/skills/market-analysis
2. https://raw.githubusercontent.com/org/repo/main/skills/market-analysis/SKILL.md
3. https://clawhub.ai/pskoett/self-improving-agent`}
          </Typography>
          <TextField
            fullWidth
            size="small"
            label="Import URL"
            value={url}
            onChange={(event) => {
              setUrl(event.target.value);
              setPreviewReady(false);
              setImportCommitted(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
          />
          <TextField
            fullWidth
            size="small"
            label="Model override (optional)"
            value={model}
            onChange={(event) => {
              setModel(event.target.value);
              setPreviewReady(false);
              setImportCommitted(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
          />
          <FormControlLabel
            control={<Switch checked={force} onChange={(event) => setForce(event.target.checked)} />}
            label="Override all warnings (import anyway)"
          />
          {importResult?.security ? (() => {
            const riskColor = importRisk.score10 >= 8 ? "#ff5f57" : importRisk.score10 >= 5 ? "#febc2e" : "#0ff0b3";
            const riskBg = importRisk.score10 >= 8 ? "rgba(255,95,87,0.06)" : importRisk.score10 >= 5 ? "rgba(254,188,46,0.06)" : "rgba(15,240,179,0.04)";
            const riskEmoji = importRisk.score10 >= 8 ? "High Risk" : importRisk.score10 >= 5 ? "Needs Review" : "Safe";
            const categoryLabels: Record<string, string> = {
              NetworkAccess: "Network access",
              CredentialPattern: "Credential pattern",
              EnvironmentAccess: "Environment variable",
              FileSystem: "File system access",
              CodeExecution: "Code execution",
              DataExfiltration: "Data exfiltration",
            };
            return (
            <Box sx={{ mt: 1, border: `1px solid ${riskColor}22`, borderRadius: "10px", background: riskBg, overflow: "hidden" }}>
              <Box sx={{ px: 1.5, py: 1, display: "flex", alignItems: "center", gap: 1, borderBottom: `1px solid ${riskColor}15` }}>
                <Box sx={{ width: 8, height: 8, borderRadius: "50%", background: riskColor, boxShadow: `0 0 6px ${riskColor}60`, flexShrink: 0 }} />
                <Typography variant="subtitle2" sx={{ fontSize: "12px", fontWeight: 700, color: riskColor }}>
                  {riskEmoji} — {importRisk.score10.toFixed(1)}/10
                </Typography>
                <Box sx={{ flex: 1 }} />
                {securityBlocked ? (
                  <Typography variant="caption" sx={{ color: "#ff5f57", fontWeight: 600 }}>BLOCKED</Typography>
                ) : importRequiresForce ? (
                  <Typography variant="caption" sx={{ color: "#febc2e", fontWeight: 600 }}>OVERRIDE REQUIRED</Typography>
                ) : null}
              </Box>
              <Box sx={{ px: 1.5, py: 1 }}>
                <Typography variant="caption" sx={{ color: "rgba(200,230,210,0.55)", display: "block", mb: 0.5 }}>
                  {(() => {
                    const raw = str(importResult.security.threat_level, "unknown");
                    const ctxRatio = importRisk.totalFindings > 0 ? importRisk.contextualFindings / importRisk.totalFindings : 0;
                    const display = raw.toLowerCase() === "malicious" && ctxRatio >= 0.8
                      ? "Standard integration patterns"
                      : `Threat level: ${raw}`;
                    return display;
                  })()}
                  {importRisk.totalFindings > 0 ? ` · ${importRisk.totalFindings} signal${importRisk.totalFindings === 1 ? "" : "s"} found` : " · No signals"}
                  {importRisk.contextualFindings > 0 ? ` (${importRisk.contextualFindings} likely safe — common in integrations)` : ""}
                </Typography>
                {Array.isArray(importResult.security.findings) && importResult.security.findings.length > 0 ? (
                  <Stack spacing={0.3} sx={{ mt: 0.75 }}>
                    {(importResult.security.findings as unknown[]).slice(0, 15).map((rawFinding, idx) => {
                      const f = asRecord(rawFinding);
                      const sev = num(f.severity, 0);
                      const sevColor = sev >= 6 ? "#ff5f57" : sev >= 3 ? "#febc2e" : "#0ff0b3";
                      const sevLabel = sev >= 6 ? "HIGH" : sev >= 3 ? "MED" : "LOW";
                      const cat = str(f.category, "");
                      const humanCat = categoryLabels[cat] || cat.replace(/([A-Z])/g, " $1").trim().toLowerCase();
                      return (
                        <Box key={`${idx}-${cat}`} sx={{ display: "flex", gap: 1, py: 0.35, alignItems: "baseline" }}>
                          <Typography sx={{ fontSize: "9.5px", fontWeight: 700, color: sevColor, minWidth: "30px", flexShrink: 0 }}>{sevLabel}</Typography>
                          <Typography sx={{ fontSize: "10.5px", color: "rgba(200,230,210,0.7)", flex: 1 }}>
                            {humanCat}{num(f.line, -1) >= 0 ? ` at line ${num(f.line)}` : ""}
                            {str(f.matched_text, "").trim() ? <span style={{ color: "rgba(130,170,160,0.4)" }}>{` — ${str(f.matched_text).slice(0, 80)}`}</span> : null}
                          </Typography>
                        </Box>
                      );
                    })}
                  </Stack>
                ) : (
                  <Typography variant="caption" sx={{ color: "#0ff0b3" }}>
                    No signals detected — looks safe.
                  </Typography>
                )}
              </Box>
            </Box>
            );
          })() : null}
          {Array.isArray(importResult?.imported) && importResult.imported.length > 0 ? (
            <Box className="term-shell" sx={{ mt: 1, p: 0 }}>
              <Box sx={{ px: 1.5, py: 0.75, borderBottom: "1px solid rgba(0,255,170,0.06)" }}>
                <Typography sx={{ fontFamily: "inherit", fontSize: "10.5px", fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase", color: "rgba(0,255,170,0.4)" }}>
                  Per-Skill Analysis
                </Typography>
              </Box>
              <Stack spacing={0} sx={{ px: 1.5, py: 0.5 }}>
                {importResult.imported.map((entry, idx) => {
                  const child = entry?.result;
                  const sec = child?.security;
                  const findingsCount = Array.isArray(sec?.findings) ? sec?.findings.length : 0;
                  const childRisk = computeImportRiskSummary(sec);
                  const riskColor = childRisk.score10 >= 8 ? "#ff5f57" : childRisk.score10 >= 5 ? "#febc2e" : "rgba(15,240,179,0.7)";
                  return (
                    <Box key={`${entry?.url || child?.name || idx}-${idx}`} sx={{ display: "flex", gap: 1.5, py: 0.4, borderBottom: "1px solid rgba(0,255,170,0.03)", alignItems: "baseline" }}>
                      <Typography sx={{ fontFamily: "inherit", fontSize: "10.5px", color: "rgba(200,230,210,0.8)", minWidth: "120px", flexShrink: 0 }}>{child?.name || "-"}</Typography>
                      <Typography sx={{ fontFamily: "inherit", fontSize: "10px", color: riskColor, fontWeight: 600, minWidth: "55px", flexShrink: 0 }}>{childRisk.score10.toFixed(1)}/10</Typography>
                      <Typography sx={{ fontFamily: "inherit", fontSize: "10px", color: "rgba(180,220,200,0.5)", flex: 1 }}>{str(sec?.threat_level, "-")} · {findingsCount} signals</Typography>
                    </Box>
                  );
                })}
              </Stack>
              <Stack spacing={0.75} sx={{ mt: 1 }}>
                {importResult.imported.map((entry, idx) => {
                  const child = entry?.result;
                  if (!child) return null;
                  const sec = child.security;
                  const warnings = Array.isArray(sec?.warnings) ? sec?.warnings : [];
                  const findings = Array.isArray(sec?.findings) ? sec?.findings : [];
                  if (warnings.length === 0 && findings.length === 0) return null;
                  return (
                    <Box key={`skill-sec-${entry?.url || child?.name || idx}-${idx}`} sx={{ border: "1px solid rgba(108,156,212,0.18)", borderRadius: 1, p: 1 }}>
                      <Typography variant="caption" sx={{ display: "block", mb: 0.5 }}>
                        {child.name || "-"} details
                      </Typography>
                      {warnings.length > 0 ? (
                        <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
                          Warnings: {warnings.slice(0, 3).join(" | ")}
                        </Typography>
                      ) : null}
                      {findings.length > 0 ? (
                        <Stack spacing={0.25} sx={{ mt: 0.5 }}>
                          {findings.slice(0, 3).map((rawFinding, fidx) => {
                            const f = asRecord(rawFinding);
                            return (
                              <Typography key={`finding-${fidx}-${str(f.category, "")}`} variant="caption" color="text.secondary" sx={{ display: "block" }}>
                                [{str(f.category, "-")}] line {num(f.line, -1) >= 0 ? num(f.line) : "-"}: {str(f.description, "-").slice(0, 180)}
                              </Typography>
                            );
                          })}
                        </Stack>
                      ) : null}
                    </Box>
                  );
                })}
              </Stack>
              {Array.isArray(importResult.failed) && importResult.failed.length > 0 ? (
                <Alert severity="warning" sx={{ mt: 1 }}>
                  Failed imports: {importResult.failed.length}
                </Alert>
              ) : null}
            </Box>
          ) : null}
          {(importResult?.secrets?.required_env || []).length > 0 ? (
            <Box sx={{ mt: 1 }}>
              <Typography variant="subtitle2" mb={1}>
                Required credentials
              </Typography>
              {!importCommitted ? (
                <Box sx={{ mb: 1, p: 1, borderRadius: "6px", background: "rgba(254,188,46,0.06)", border: "1px solid rgba(254,188,46,0.15)" }}>
                  <Typography variant="caption" sx={{ color: "#febc2e", fontWeight: 600 }}>
                    Fill in credentials below, then click "Import Template" to save them.
                  </Typography>
                </Box>
              ) : null}
              <Stack spacing={1}>
                {(importResult?.secrets?.required_env || []).map((env) => {
                  const d = secretDrafts[env] || { storeAs: env, value: "", useBuiltin: false };
                  const missing = (importResult?.secrets?.missing_env || []).includes(env);
                  return (
                    <Box key={env} sx={{ border: "1px solid rgba(108,156,212,0.18)", borderRadius: 1, p: 1 }}>
                      <Stack direction="row" justifyContent="space-between" alignItems="center">
                        <Typography variant="body2" fontWeight={700}>
                          {env}
                        </Typography>
                        <Chip size="small" color={missing ? "warning" : "success"} label={missing ? "missing" : "configured"} />
                      </Stack>
                      <Stack direction={{ xs: "column", md: "row" }} spacing={1} mt={1}>
                        <TextField
                          fullWidth
                          size="small"
                          label="Store as"
                          value={d.storeAs}
                          disabled={d.useBuiltin}
                          onChange={(e) =>
                            setSecretDrafts((prev) => ({ ...prev, [env]: { ...d, storeAs: e.target.value } }))
                          }
                        />
                        <TextField
                          fullWidth
                          size="small"
                          type="password"
                          label="Value (optional)"
                          value={d.value}
                          disabled={d.useBuiltin}
                          onChange={(e) =>
                            setSecretDrafts((prev) => ({ ...prev, [env]: { ...d, value: e.target.value } }))
                          }
                        />
                      </Stack>
                      <FormControlLabel
                        control={
                          <Switch
                            checked={d.useBuiltin}
                            onChange={(e) =>
                              setSecretDrafts((prev) => ({ ...prev, [env]: { ...d, useBuiltin: e.target.checked } }))
                            }
                          />
                        }
                        label="Use builtin provider key"
                      />
                    </Box>
                  );
                })}
                <Button
                  variant="outlined"
                  disabled={savingSecrets || secretsSaved || !importCommitted}
                  onClick={handleSaveSecrets}
                >
                  {savingSecrets ? "Saving..." : secretsSaved ? "Secrets saved" : !importCommitted ? "Import template first" : "Save secrets"}
                </Button>
              </Stack>
            </Box>
          ) : null}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={handleClose} disabled={loading}>
          Close
        </Button>
        <Button
          variant="contained"
          disabled={loading || !url.trim() || importRequiresForce}
          onClick={previewReady ? handleImport : handleAnalyze}
        >
          {loading
            ? previewReady
              ? "Importing..."
              : "Analyzing..."
            : previewReady
              ? "Import Template"
              : "Analyze Template"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function SkillSecretsDialog({
  open,
  skillName,
  onClose
}: {
  open: boolean;
  skillName: string | null;
  onClose: () => void;
}) {
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [secrets, setSecrets] = useState<{ required_env: string[]; missing_env: string[]; bindings: Record<string, string> } | null>(null);
  const [drafts, setDrafts] = useState<Record<string, { storeAs: string; value: string; useBuiltin: boolean }>>({});

  useEffect(() => {
    if (!open || !skillName) return;
    setLoading(true);
    setError(null);
    setInfo(null);
    setSecrets(null);
    setDrafts({});
    api
      .getSkillSecrets(skillName)
      .then((out) => {
        setSecrets(out);
        const next: Record<string, { storeAs: string; value: string; useBuiltin: boolean }> = {};
        for (const env of out.required_env || []) {
          const binding = (out.bindings || {})[env];
          next[env] = {
            storeAs: binding && binding !== "builtin" ? binding : env,
            value: "",
            useBuiltin: binding === "builtin"
          };
        }
        setDrafts(next);
      })
      .catch((err) => setError(errMessage(err)))
      .finally(() => setLoading(false));
  }, [open, skillName]);

  const save = async () => {
    if (!skillName || !secrets) return;
    setSaving(true);
    setError(null);
    setInfo(null);
    try {
      const payload = (secrets.required_env || []).map((env) => {
        const d = drafts[env] || { storeAs: env, value: "", useBuiltin: false };
        if (d.useBuiltin) return { env, store_as: "builtin" };
        const storeAs = (d.storeAs || env).trim();
        const value = (d.value || "").trim();
        return value ? { env, store_as: storeAs, value } : { env, store_as: storeAs };
      });
      const out = await api.setSkillSecrets(skillName, { secrets: payload });
      setSecrets(out);
      if ((out.missing_env || []).length > 0) {
        setError(`Some keys are still missing: ${out.missing_env.join(", ")}`);
      } else {
        setInfo("Secrets saved. The skill remains disabled until you manually enable it in Skills.");
      }
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Secrets: {skillName || ""}</DialogTitle>
      <DialogContent dividers>
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>
          Secrets are private API keys or tokens used by this skill at runtime.
        </Typography>
        {loading ? <Typography variant="body2" color="text.secondary">Loading...</Typography> : null}
        {error ? <Alert severity="error">{error}</Alert> : null}
        {info ? <Alert severity="info">{info}</Alert> : null}
        {!loading && secrets ? (
          <Stack spacing={1.25}>
            {(secrets.required_env || []).length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No required credentials detected for this skill.
              </Typography>
            ) : (
              (secrets.required_env || []).map((env) => {
                const d = drafts[env] || { storeAs: env, value: "", useBuiltin: false };
                const missing = (secrets.missing_env || []).includes(env);
                return (
                  <Box key={env} sx={{ border: "1px solid rgba(108,156,212,0.18)", borderRadius: 1, p: 1 }}>
                    <Stack direction="row" justifyContent="space-between" alignItems="center">
                      <Typography variant="body2" fontWeight={700}>
                        {env}
                      </Typography>
                      <Chip size="small" color={missing ? "warning" : "success"} label={missing ? "missing" : "configured"} />
                    </Stack>
                    <Stack direction={{ xs: "column", md: "row" }} spacing={1} mt={1}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Store as"
                        value={d.storeAs}
                        disabled={d.useBuiltin}
                        onChange={(e) => setDrafts((prev) => ({ ...prev, [env]: { ...d, storeAs: e.target.value } }))}
                      />
                      <TextField
                        fullWidth
                        size="small"
                        type="password"
                        label="Value (optional)"
                        value={d.value}
                        disabled={d.useBuiltin}
                        onChange={(e) => setDrafts((prev) => ({ ...prev, [env]: { ...d, value: e.target.value } }))}
                      />
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={d.useBuiltin}
                          onChange={(e) => setDrafts((prev) => ({ ...prev, [env]: { ...d, useBuiltin: e.target.checked } }))}
                        />
                      }
                      label="Use builtin provider key"
                    />
                  </Box>
                );
              })
            )}
          </Stack>
        ) : null}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
        <Button variant="contained" onClick={save} disabled={saving || loading || !secrets || (secrets.required_env || []).length === 0}>
          {saving ? "Saving..." : "Save"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function QueryTable({
  title,
  path,
  arrayKey,
  columns,
  autoRefresh,
  emptyLabel,
  queryKey
}: {
  title: string;
  path: string;
  arrayKey: string;
  columns: string[];
  autoRefresh: boolean;
  emptyLabel: string;
  queryKey: string;
}) {
  const q = useQuery({
    queryKey: [queryKey],
    queryFn: () => api.rawGet(path),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const rows = pickRecords(q.data, arrayKey);

  return (
    <Box className="list-shell">
      <Typography variant="h6" mb={1}>
        {title}
      </Typography>
      {q.error ? (
        <Alert severity="error">{errMessage(q.error)}</Alert>
      ) : rows.length === 0 ? (
        <Typography variant="body2" color="text.secondary">
          {emptyLabel}
        </Typography>
      ) : (
        <DataTable rows={rows} columns={columns} />
      )}
    </Box>
  );
}

function RowOpsMenu({ actions, ariaLabel = "Row actions" }: { actions: RowMenuAction[]; ariaLabel?: string }) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton size="small" aria-label={ariaLabel} onClick={(e) => setAnchorEl(e.currentTarget)}>
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu anchorEl={anchorEl} open={open} onClose={closeMenu}>
        {actions.map((action, idx) => (
          <MenuItem
            key={`${action.label}-${idx}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={() => {
              closeMenu();
              if (action.disabled) return;
              void action.onClick();
            }}
            sx={
              action.tone === "error"
                ? { color: "error.main" }
                : action.tone === "warning"
                  ? { color: "warning.main" }
                  : undefined
            }
          >
            {action.label}
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

function ChatManager({ autoRefresh, isActive }: { autoRefresh: boolean; isActive: boolean }) {
  const queryClient = useQueryClient();
  const chatAutoRefresh = autoRefresh && isActive;
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [draftProjectId, setDraftProjectId] = useState("");
  const [prompt, setPrompt] = useState("");
  const [chatExecutionMode, setChatExecutionMode] = useState<ChatExecutionMode>("auto");
  const [deepResearchEnabled, setDeepResearchEnabled] = useState(false);
  const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatNotice, setChatNotice] = useState<string | null>(null);
  const [activeChatTask, setActiveChatTask] = useState<ActiveChatTaskState | null>(null);
  const [isStreaming, setIsStreaming] = useState(false);
  const [pendingUserMessage, setPendingUserMessage] = useState<string | null>(null);
  const [failedUserMessage, setFailedUserMessage] = useState<string | null>(null);
  const [streamingResponse, setStreamingResponse] = useState("");
  const [streamingSteps, setStreamingSteps] = useState<JsonRecord[]>([]);
  const [streamingProgressMessages, setStreamingProgressMessages] = useState<string[]>([]);
  const [streamTraceOpen, setStreamTraceOpen] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [conversationSidebarOpen, setConversationSidebarOpen] = useState(false);
  const [activityAutoFollow, setActivityAutoFollow] = useState(true);
  const [secretHelperMode, setSecretHelperMode] = useState<"reuse" | "manual">("reuse");
  const [secretHelperKey, setSecretHelperKey] = useState("OPENAI_API_KEY");
  const [secretHelperValue, setSecretHelperValue] = useState("");
  const [secretHelperBusy, setSecretHelperBusy] = useState(false);
  const [isDragOverChat, setIsDragOverChat] = useState(false);
  const [deployedFiles, setDeployedFiles] = useState<Array<{ name: string; content: string }>>([]);
  const [liveFileWrites, setLiveFileWrites] = useState<Record<string, LiveFileWriteState>>({});
  const [codeViewerOpen, setCodeViewerOpen] = useState(false);
  const [codeViewerFileIdx, setCodeViewerFileIdx] = useState(0);
  const [previewDialogOpen, setPreviewDialogOpen] = useState(false);
  const [messageTraceOpen, setMessageTraceOpen] = useState<Record<string, boolean>>({});
  const [traceStepsById, setTraceStepsById] = useState<Record<string, JsonRecord[]>>({});
  const [traceLoadingById, setTraceLoadingById] = useState<Record<string, boolean>>({});
  const [traceErrorById, setTraceErrorById] = useState<Record<string, string>>({});
  const [lastRunSteps, setLastRunSteps] = useState<JsonRecord[]>([]);
  const [conversationMenuAnchor, setConversationMenuAnchor] = useState<HTMLElement | null>(null);
  const [conversationMenuTarget, setConversationMenuTarget] = useState<JsonRecord | null>(null);
  const [pendingRunSnapshot, setPendingRunSnapshot] = useState<ChatPendingRunSnapshot | null>(
    () => loadChatPendingRunSnapshot()
  );
  const chatBackgroundRefresh = isStreaming || pendingRunSnapshot !== null;
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const dragDepthRef = useRef(0);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const streamLockRef = useRef(false);
  const recentSendRef = useRef<{ fingerprint: string; at: number } | null>(null);
  const streamingStepsRef = useRef<JsonRecord[]>([]);
  const streamingStepKeySeqRef = useRef(1);
  const workspaceActivityRef = useRef<HTMLDivElement | null>(null);

  const convQ = useQuery({
    queryKey: ["chat-conversations"],
    queryFn: () => api.rawGet("/conversations?limit=30"),
    refetchInterval: chatAutoRefresh || chatBackgroundRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: chatBackgroundRefresh
  });
  const projectsQ = useQuery({
    queryKey: ["chat-projects"],
    queryFn: () => api.rawGet("/projects"),
    refetchInterval: chatAutoRefresh || chatBackgroundRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: chatBackgroundRefresh
  });

  const conversations = pickRecords(convQ.data, "conversations");
  const projects = pickRecords(projectsQ.data, "projects");
  const selectedConversation = useMemo(
    () => conversations.find((conv) => str(conv.id, "") === conversationId) ?? null,
    [conversations, conversationId]
  );
  const selectedMessageCount = num(selectedConversation?.message_count, 0);
  const selectedConversationUpdatedAtMs = Date.parse(str(selectedConversation?.updated_at, ""));
  const recentlyTouchedEmptyConversation =
    selectedMessageCount === 0 &&
    Number.isFinite(selectedConversationUpdatedAtMs) &&
    Date.now() - selectedConversationUpdatedAtMs < 10 * 60 * 1000;
  const hasPendingSnapshotForConversation =
    !!conversationId && pendingRunSnapshot?.conversationId === conversationId;
  const isStreamingForCurrentConversation = isStreaming && hasPendingSnapshotForConversation;
  const shouldPollMessages =
    !!conversationId &&
    (isStreamingForCurrentConversation ||
      hasPendingSnapshotForConversation ||
      recentlyTouchedEmptyConversation);
  const messagesQ = useQuery({
    queryKey: ["chat-messages", conversationId],
    queryFn: () => api.rawGet(`/conversations/${encodeURIComponent(conversationId || "")}/messages?limit=100`),
    enabled: !!conversationId && (isActive || shouldPollMessages),
    refetchInterval: shouldPollMessages ? 2000 : chatAutoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: shouldPollMessages
  });

  const messages = conversationId ? pickRecords(messagesQ.data, "messages") : [];
  const latestAssistantTraceId = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const candidate = messages[i];
      if (str(candidate.role, "").toLowerCase() !== "assistant") continue;
      const traceId = str(candidate.trace_id, "").trim();
      if (traceId) return traceId;
    }
    return "";
  }, [messages]);
  const projectNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const project of projects) {
      const id = str(project.id, "").trim();
      if (!id) continue;
      map.set(id, str(project.name, id));
    }
    return map;
  }, [projects]);
  const selectedConversationProjectId = str(selectedConversation?.project_id, "").trim();
  const activeProjectId = selectedConversationProjectId || draftProjectId;

  useEffect(() => {
    if (!pendingRunSnapshot) {
      storeChatPendingRunSnapshot(null);
      return;
    }
    const snapshotSteps = streamingSteps
      .slice(-CHAT_PENDING_STREAM_STEPS_MAX)
      .map((step) => {
        const compacted: JsonRecord = {};
        const icon = str(step.icon, "").trim();
        const title = str(step.title, "").trim();
        const detail = str(step.detail, "").trim();
        const stepType = str(step.step_type, "").trim();
        const data = compactUnknown(step.data, 800);
        if (icon) compacted.icon = icon.slice(0, 64);
        if (title) compacted.title = title.slice(0, 220);
        if (detail) compacted.detail = detail.slice(0, 900);
        if (stepType) compacted.step_type = stepType.slice(0, 80);
        if (data) compacted.data = data;
        return compacted;
      });
    storeChatPendingRunSnapshot({
      ...pendingRunSnapshot,
      message: pendingUserMessage ?? pendingRunSnapshot.message,
      projectId: activeProjectId || pendingRunSnapshot.projectId || "",
      streamingResponse: streamingResponse.slice(0, CHAT_PENDING_STREAM_RESPONSE_MAX_CHARS),
      streamingSteps: snapshotSteps,
      failedUserMessage: failedUserMessage ?? ""
    });
  }, [
    pendingRunSnapshot,
    pendingUserMessage,
    failedUserMessage,
    streamingResponse,
    streamingSteps,
    activeProjectId
  ]);

  useEffect(() => {
    if (typeof window === "undefined" || !conversationId) return;
    try {
      window.sessionStorage.setItem(CHAT_LAST_CONVERSATION_STORAGE_KEY, conversationId);
    } catch {
      // Ignore storage failures.
    }
  }, [conversationId]);

  useEffect(() => {
    const pending = pendingRunSnapshot ?? loadChatPendingRunSnapshot();
    if (
      pending &&
      conversations.some((conv) => str(conv.id, "") === pending.conversationId)
    ) {
      const shouldSelectPendingConversation = !conversationId;
      const viewingPendingConversation = conversationId === pending.conversationId;
      if (shouldSelectPendingConversation) {
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
          setStreamingResponse(pending.streamingResponse);
        }
        if (
          Array.isArray(pending.streamingSteps) &&
          pending.streamingSteps.length > 0 &&
          streamingStepsRef.current.length === 0
        ) {
          const restoredSteps = pending.streamingSteps.map((step) =>
            ensureActivityStepTime(asRecord(step))
          );
          setStreamingSteps(restoredSteps);
          streamingStepsRef.current = restoredSteps;
        }
        return;
      }
    }

    if (conversationId || conversations.length === 0 || typeof window === "undefined") return;
    try {
      const lastSelected = window.sessionStorage
        .getItem(CHAT_LAST_CONVERSATION_STORAGE_KEY)
        ?.trim();
      if (
        lastSelected &&
        conversations.some((conv) => str(conv.id, "") === lastSelected)
      ) {
        setConversationId(lastSelected);
      }
    } catch {
      // Ignore storage failures.
    }
  }, [
    conversationId,
    conversations,
    pendingRunSnapshot,
    pendingUserMessage,
    failedUserMessage,
    streamingResponse,
    streamingSteps.length
  ]);

  useEffect(() => {
    if (!pendingRunSnapshot) return;
    if (conversationId !== pendingRunSnapshot.conversationId) return;
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
    if (preservedSteps.length > 0) {
      setLastRunSteps(preservedSteps);
    }
    storeChatPendingRunSnapshot(null);
    setPendingRunSnapshot(null);
    setPendingUserMessage(null);
    setStreamingResponse("");
    setStreamingSteps([]);
    streamingStepsRef.current = [];
  }, [pendingRunSnapshot, conversationId, messages, streamingSteps]);

  useEffect(() => {
    if (!latestAssistantTraceId || isStreaming || hasPendingSnapshotForConversation) return;
    if (traceStepsById[latestAssistantTraceId] || traceLoadingById[latestAssistantTraceId] || traceErrorById[latestAssistantTraceId]) return;
    void loadTraceForId(latestAssistantTraceId);
  }, [
    latestAssistantTraceId,
    isStreaming,
    hasPendingSnapshotForConversation,
    traceStepsById,
    traceLoadingById,
    traceErrorById
  ]);

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
      web_search: "Web search"
    };
    if (direct[normalized]) return direct[normalized];
    return normalized
      .replace(/[_-]+/g, " ")
      .replace(/\b\w/g, (ch) => ch.toUpperCase());
  };

  const toolStartCopy = (name: string): { label: string; detail: string } => {
    const normalized = (name || "").trim().toLowerCase();
    const byTool: Record<string, { label: string; detail: string }> = {
      app_deploy: { label: "Deploying public link", detail: "Starting deployment and publishing access link." },
      build_check: { label: "Running checks", detail: "Checking compile and build health." },
      run_tests: { label: "Running checks", detail: "Running tests to validate behavior." },
      lint_check: { label: "Running checks", detail: "Checking code quality and style." },
      source_read: { label: "Reading project files", detail: "Reviewing existing code before changes." },
      source_write: { label: "Creating project files", detail: "Creating or updating project files." },
      source_edit: { label: "Creating project files", detail: "Applying code changes in project files." },
      source_list: { label: "Scanning project files", detail: "Checking project structure." },
      source_search: { label: "Searching project files", detail: "Looking for the right place to edit." },
      web_search: { label: "Searching sources", detail: "Looking up relevant online sources." },
      browse: { label: "Opening source page", detail: "Trying to open the requested web page." },
      schedule_task: { label: "Setting recurring monitor", detail: "Creating the schedule for automatic runs." },
      frontend_build: { label: "Installing dependencies", detail: "Preparing dependencies and building dashboard UI." }
    };
    return byTool[normalized] || {
      label: `Running ${toHumanToolName(name).toLowerCase()}`,
      detail: "Executing this action."
    };
  };

  const simplifyConsoleDetail = (detail: string): string => {
    let text = (detail || "").replace(/\s+/g, " ").trim();
    if (!text) return "";

    if (/^loaded \d+ messages?, packed \d+/i.test(text)) return "Collected recent chat context.";
    if (/channel:\s*\w+\s*\|\s*length:\s*\d+\s*chars/i.test(text)) return "Reading your request.";
    if (/mem0 pending/i.test(text)) return "Checking saved memory context.";
    if (/found \d+ relevant memories/i.test(text)) {
      const m = text.match(/found\s+(\d+)\s+relevant memories/i);
      const count = m?.[1] || "0";
      return `Found ${count} related memory item${count === "1" ? "" : "s"}.`;
    }
    if (/complex\s*[-=]?>\s*direct llm/i.test(text)) return "Using a direct execution strategy.";
    if (/using primary model/i.test(text)) return "Selected the best available model.";
    if (/response length:\s*\d+\s*chars/i.test(text) || /tool calls:\s*\d+/i.test(text)) {
      return "Prepared the next response.";
    }
    if (/proof id:|verification id:/i.test(text)) return "Saved a verifiable execution record.";
    if (/running in sandboxed environment/i.test(text)) return "Running this action in a safe workspace.";
    if (/install(ing)? dependencies|npm install|pnpm install|yarn install|cargo fetch/i.test(text)) {
      return "Installing dependencies.";
    }
    if (isSafetyPolicyBlockedText(text)) {
      return "Blocked by safety policy. The agent needs a different approach.";
    }
    if (/approval required|needs approval|awaiting approval|requires approval/i.test(text)) {
      return "Waiting for your approval/input.";
    }
    if (/browse failed; used search fallback/i.test(text)) {
      return "Could not open the page directly, switched to web search.";
    }
    if (/http error 404/i.test(text) || /\b404\b.*not found/i.test(text)) {
      return "Page not found (404). Trying alternate sources.";
    }
    if (/search results for:/i.test(text)) return "Found search results and selected relevant sources.";
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

  const humanizeStep = (
    title: string,
    detail: string,
    stepType: string
  ): { label: string; detail: string; kind?: string; tone?: string } => {
    const t = title.toLowerCase();
    // Log-style: short typed label + actual detail from the step data
    if (t === "message received" || t.startsWith("message received")) {
      return { label: "Reading your request", detail: detail || "" };
    }
    if (t === "memory layer" || t.startsWith("memory layer")) {
      return { label: "Loading memory", detail: detail || "Checking saved context and preferences" };
    }
    if (t === "memory retrieval" || t.startsWith("memory retrieval")) {
      return { label: "Searching memory", detail: detail || "Looking for related past conversations" };
    }
    if (t === "context packing" || t.startsWith("context packing")) {
      return { label: "Building context", detail: detail || "Assembling conversation history" };
    }
    if (t === "llm routing decision" || t.startsWith("llm routing decision")) {
      return { label: "Choosing strategy", detail: detail || "Deciding the best execution approach" };
    }
    if (t === "model selection" || t.startsWith("model selection")) {
      return { label: "Selecting model", detail: detail || "Picking the best available model" };
    }
    if (t === "llm request" || t.startsWith("llm request")) {
      return { label: "Thinking", detail: detail || "Sending request to AI model" };
    }
    if (t === "parallel thinking started" || t.startsWith("parallel thinking started")) {
      return { label: "Parallel reasoning", detail: detail || "Exploring multiple approaches simultaneously" };
    }
    if (t === "parallel thinking complete" || t.startsWith("parallel thinking complete")) {
      return { label: "Reasoning complete", detail: detail || "Merged parallel results", kind: "Done", tone: "tone-success" };
    }
    if (t === "autopilot proceed" || t.startsWith("autopilot proceed")) {
      return { label: "Running autonomously", detail: detail || "Proceeding without user input" };
    }
    if (t.startsWith("tool started:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      const humanName = toHumanToolName(rawName);
      return {
        label: `Running ${humanName}`,
        detail: detail || "",
        kind: "Running",
        tone: "tone-action"
      };
    }
    if (t.startsWith("tool finished:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      const summarized = summarizeActivityDetail(detail);
      if (isSafetyPolicyBlockedText(detail) || isSafetyPolicyBlockedText(summarized)) {
        return {
          label: `${toHumanToolName(rawName)} blocked`,
          detail: "Blocked by safety policy. The agent needs a different approach.",
          kind: "Issue",
          tone: "tone-error"
        };
      }
      return {
        label: `${toHumanToolName(rawName)} completed`,
        detail:
          summarized && !isHumanReadableStatus(summarized)
            ? summarized
            : summarized && summarized !== `${toHumanToolName(rawName)} completed`
              ? summarized
              : "",
        kind: "Done",
        tone: "tone-success"
      };
    }
    if (t.startsWith("tool progress:")) {
      const rawName = title.split(":").slice(1).join(":").trim();
      return {
        label: `Running ${toHumanToolName(rawName)}`,
        detail: detail || "Working...",
        kind: "Running",
        tone: "tone-action"
      };
    }
    if (stepType.includes("tool_start")) {
      return {
        label: "Executing action",
        detail: detail || "",
        kind: "Running",
        tone: "tone-action"
      };
    }
    if (stepType.includes("tool_progress")) {
      return { label: "Action in progress", detail: detail || "", kind: "Running", tone: "tone-action" };
    }
    if (stepType.includes("tool_result")) {
      return { label: "Action completed", detail: detail || "", kind: "Done", tone: "tone-success" };
    }
    if (t.includes("approval")) {
      return { label: "Waiting for approval", detail: detail || "", tone: "tone-thinking" };
    }
    if (t === "response complete" || t.startsWith("response complete")) {
      return { label: "Response delivered", detail: detail || "", kind: "Done", tone: "tone-success" };
    }
    if (t === "llm response received" || t.startsWith("llm response received")) {
      return { label: "Response received", detail: detail || "Model finished generating", kind: "Update" };
    }
    if (t === "self evolve" || t.startsWith("self evolve") || t.startsWith("running self evolve")) {
      return { label: "Self-evolving", detail: detail || "Autonomous code modification started", kind: "Running", tone: "tone-action" };
    }
    // Fallback: use raw title as-is
    const fallbackLabel = title || stepType.replace(/[_-]+/g, " ").trim();
    return { label: fallbackLabel, detail };
  };

  const isHeartbeatStreamingStep = (value: JsonRecord): boolean => {
    const title = normalizeStatusText(str(value.title, ""));
    const icon = normalizeStatusText(str(value.icon, ""));
    const detail = normalizeStatusText(str(value.detail, ""));
    const stepType = normalizeStatusText(str(value.step_type, str(value.type, "")));
    return (
      title.includes("still working") ||
      icon === "wait" ||
      stepType.includes("still work") ||
      stepType.includes("heartbeat") ||
      (stepType.includes("thinking") &&
        detail.includes("no new output") &&
        detail.includes("idle"))
    );
  };

  const streamingStepDedupKey = (value: JsonRecord): string => {
    const stepType = normalizeStatusText(str(value.step_type, str(value.type, "step")));
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

  const attachStreamingStepStableKey = (
    value: JsonRecord,
    preferredKey?: string
  ): JsonRecord => {
    const existing = getStreamingStepStableKey(value);
    if (existing) return value;
    return {
      ...value,
      __streamKey: preferredKey || `stream-step-${streamingStepKeySeqRef.current++}`
    };
  };

  const buildStepCard = (step: JsonRecord, index: number) => {
    const stepType = str(step.step_type, str(step.type, "step")).toLowerCase();
    const title = str(step.title, "").trim();
    const fullDetail = extractStepDetailText(step, 2800);
    const rawDetail = fullDetail.slice(0, 900);
    const human = humanizeStep(title, rawDetail, stepType);
    const humanDetailRaw = str(human.detail, "").trim();
    const summarizedDetail = humanDetailRaw ? summarizeActivityDetail(humanDetailRaw) : "";
    let detail = summarizedDetail ? simplifyConsoleDetail(summarizedDetail) : "";
    const time = str(step.time, "");
    const baseLabel = stepType.replace(/[_-]+/g, " ").trim() || "step";
    const rawLabel = human.label || title || baseLabel;
    // Only capitalize if label doesn't contain file paths/extensions
    const label = /\.\w{1,5}\b|\//.test(rawLabel) ? rawLabel : rawLabel.replace(/\b\w/g, (ch) => ch.toUpperCase());
    let tone = "tone-neutral";
    let kind = "Update";
    const labelLower = label.toLowerCase();
    if (stepType.includes("tool_start")) {
      tone = "tone-tool";
      kind = "Running";
    } else if (stepType.includes("tool_progress")) {
      tone = "tone-action";
      kind = "Running";
    } else if (stepType.includes("tool_result") || stepType.includes("result") || stepType.includes("complete") || stepType.includes("success")) {
      tone = "tone-success";
      kind = "Done";
    } else if (stepType === "info") {
      tone = "tone-neutral";
      kind = "Done";
    } else if (stepType.includes("error") || stepType.includes("fail")) {
      tone = "tone-error";
      kind = "Issue";
    } else if (stepType.includes("think") || stepType.includes("plan") || stepType.includes("reason")) {
      tone = "tone-thinking";
      kind = "Planning";
    } else if (stepType.includes("action") || stepType.includes("execute")) {
      tone = "tone-action";
      kind = "Running";
    } else if (stepType.includes("response") || stepType.includes("final") || stepType.includes("summary")) {
      tone = "tone-synthesis";
      kind = "Done";
    } else if (/start|running|loading|checking|choosing|selecting|generating/.test(labelLower)) {
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
    return {
      id: stableId || `${time || "live"}-${index}-${label}`,
      index,
      tone,
      kind,
      label,
      detail,
      detailFull,
      rawDetailFull: humanDetailRaw ? (fullDetail || rawDetail) : "",
      isHeartbeat: isHeartbeatStreamingStep(step),
      time
    };
  };

  const safeBuildStepCard = (step: unknown, index: number) => {
    const record = asRecord(step);
    try {
      return buildStepCard(record, index);
    } catch {
      const stepType = str(record.step_type, str(record.type, "step")).toLowerCase();
      const title = str(record.title, "").trim();
      const rawDetail = extractStepDetailText(record, 600);
      const label = title || stepType.replace(/[_-]+/g, " ").trim() || "Activity update";
      const stableId = getStreamingStepStableKey(record);
      return {
        id: stableId || `${str(record.time, "live") || "live"}-${index}-${label}`,
        index,
        tone: "tone-neutral",
        kind: "Update",
        label,
        detail: rawDetail ? simplifyConsoleDetail(rawDetail) : "",
        detailFull: "",
        rawDetailFull: rawDetail,
        isHeartbeat: false,
        time: str(record.time, "")
      };
    }
  };

  const streamingTraceCards = useMemo(
    () => streamingSteps.map((step, idx) => safeBuildStepCard(step, idx)).slice(-24),
    [streamingSteps]
  );
  const pickPrimaryActivityCard = (
    cards: Array<ReturnType<typeof buildStepCard>>
  ): ReturnType<typeof buildStepCard> | null => {
    if (cards.length === 0) return null;
    const last = cards[cards.length - 1];
    if (!last.isHeartbeat) return last;
    return [...cards].reverse().find((card) => !card.isHeartbeat) || last;
  };
  const streamingActivity = useMemo(() => {
    const last = pickPrimaryActivityCard(streamingTraceCards);
    if (!last) return "Thinking...";
    const kind = (last.kind || "").toLowerCase();
    if (kind.includes("planning") || kind.includes("thinking")) return "Thinking...";
    if (kind.includes("memory") || kind.includes("loading")) return "Recalling context...";
    if (kind.includes("done") || kind.includes("update")) return "Writing response...";
    return "Working...";
  }, [streamingTraceCards]);

  const traceSummaryText = (
    cards: Array<ReturnType<typeof buildStepCard>>,
    opts?: { loading?: boolean; streaming?: boolean; error?: string }
  ) => {
    if (opts?.error) return "Activity details unavailable.";
    if (opts?.loading && cards.length === 0) return "View activity";
    if (cards.length === 0) return opts?.streaming ? "Waiting for first activity update..." : "View activity";
    const last = pickPrimaryActivityCard(cards) || cards[cards.length - 1];
    const count = countMeaningfulActivityCards(cards);
    return `${count} update${count === 1 ? "" : "s"} | Now: ${last.label}`;
  };

  const traceSummaryFromSteps = (
    steps: JsonRecord[],
    opts?: { loading?: boolean; streaming?: boolean; error?: string }
  ) => {
    if (opts?.error) return "Activity details unavailable.";
    if (opts?.loading && steps.length === 0) return "View activity";
    if (steps.length === 0) return opts?.streaming ? "Waiting for first activity update..." : "View activity";
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
      normalizedPrimaryIndex
    );
    const normalizedCount =
      normalizedSteps.filter((step) => !isHeartbeatStreamingStep(step)).length ||
      normalizedSteps.length;
    return `${normalizedCount} update${normalizedCount === 1 ? "" : "s"} | Now: ${normalizedPrimaryCard.label}`;
    let primaryIndex = steps.length - 1;
    for (let i = steps.length - 1; i >= 0; i -= 1) {
      if (!isHeartbeatStreamingStep(steps[i])) {
        primaryIndex = i;
        break;
      }
    }
    const primaryCard = safeBuildStepCard(steps[primaryIndex], primaryIndex);
    return `${steps.length} update${steps.length === 1 ? "" : "s"} â€¢ Now: ${primaryCard.label}`;
  };

  const parseTraceSteps = (payload: unknown): JsonRecord[] => {
    const rec = asRecord(payload);
    const raw = Array.isArray(rec.steps) ? rec.steps : Array.isArray(rec.trace) ? rec.trace : [];
    return compressActivitySteps(
      raw
        .filter((x) => x && typeof x === "object")
        .map((x) => normalizeActivityStepTime(asRecord(x)))
    );
  };

  const loadTraceForId = async (traceId: string) => {
    if (!traceId) return;
    if (traceStepsById[traceId] || traceLoadingById[traceId] || traceErrorById[traceId]) return;
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

  const startNewConversation = () => {
    if (!isStreaming) {
      storeChatPendingRunSnapshot(null);
      setPendingRunSnapshot(null);
    }
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
    setDraftProjectId("");
    setPrompt("");
    setDeepResearchEnabled(false);
    setAttachedFiles([]);
    setChatError(null);
    setChatNotice(null);
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponse("");
    setStreamingSteps([]);
    streamingStepsRef.current = [];
    setTraceStepsById({});
    setTraceLoadingById({});
    setTraceErrorById({});
    setLastRunSteps([]);
    setLiveFileWrites({});
    setDeployedFiles([]);
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    setMessageTraceOpen({});
  };

  const openConversationById = (id: string) => {
    if (!id) return;
    setChatError(null);
    if (conversationId === id) return;
    setPendingUserMessage(null);
    setFailedUserMessage(null);
    setStreamingResponse("");
    setStreamingSteps([]);
    streamingStepsRef.current = [];
    setTraceStepsById({});
    setTraceLoadingById({});
    setTraceErrorById({});
    setLastRunSteps([]);
    setLiveFileWrites({});
    setDeployedFiles([]);
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    setMessageTraceOpen({});
    setConversationId(id);
    if (typeof window !== "undefined" && window.innerWidth < 980) {
      setConversationSidebarOpen(false);
    }
    if (pendingRunSnapshot?.conversationId === id) {
      if (pendingRunSnapshot.message) {
        setPendingUserMessage(pendingRunSnapshot.message);
      }
      if (pendingRunSnapshot.failedUserMessage) {
        setFailedUserMessage(pendingRunSnapshot.failedUserMessage);
      }
      if (pendingRunSnapshot.streamingResponse) {
        setStreamingResponse(pendingRunSnapshot.streamingResponse);
      }
      if (
        Array.isArray(pendingRunSnapshot.streamingSteps) &&
        pendingRunSnapshot.streamingSteps.length > 0
      ) {
        setStreamingSteps(pendingRunSnapshot.streamingSteps);
        streamingStepsRef.current = pendingRunSnapshot.streamingSteps;
      }
    }
  };

  const queueAttachedFiles = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    const incoming = Array.from(files);
    const { accepted, rejected } = splitSupportedChatAttachments(incoming);
    if (rejected.length > 0) {
      const preview = rejected.slice(0, 3).join(", ");
      const extra = rejected.length > 3 ? ` (+${rejected.length - 3} more)` : "";
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
            f.lastModified === file.lastModified
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

  const removeAttachedFile = (idx: number) => {
    setAttachedFiles((prev) => prev.filter((_, i) => i !== idx));
  };

  const uploadAttachmentsForKnowledge = async (files: File[]) => {
    if (files.length === 0) return [] as Array<{ id: string; filename: string; chunks: number }>;
    const projectId = activeProjectId.trim();
    const uploaded: Array<{ id: string; filename: string; chunks: number }> = [];
    for (const file of files) {
      const formData = new FormData();
      formData.append("file", file, file.name);
      if (projectId) formData.append("project_id", projectId);
      const out = asRecord(await api.rawPostForm("/documents/upload-file", formData));
      const id = str(out.id, "");
      if (!id) {
        throw new Error(`Failed to index '${file.name}'.`);
      }
      uploaded.push({
        id,
        filename: str(out.filename, file.name),
        chunks: num(out.chunks, 0)
      });
    }
    return uploaded;
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
          str(obj.reason, "")
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
      /missing ['"`]?files['"`]?/i.test(message) ||
      /object mapping filename to content/i.test(message)
    ) {
      return "Deploy payload was malformed (missing files). Retry the request; AgentArk will regenerate a valid app_deploy payload.";
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
    if (/openai api error/i.test(message) || /anthropic api error/i.test(message)) {
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
    if (/http error 404/i.test(message) || /\b404\b.*not found/i.test(message)) {
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

  const exportConversationById = async (targetId: string, titleHint?: string) => {
    if (!targetId) return;
    setChatError(null);
    try {
      let exportMessages = messages;
      if (conversationId !== targetId || exportMessages.length === 0) {
        const payload = await api.rawGet(`/conversations/${encodeURIComponent(targetId)}/messages?limit=200`);
        exportMessages = pickRecords(payload, "messages");
      }
      const title = (titleHint || str(selectedConversation?.title, "chat")).trim() || "chat";
      const safe = title.replace(/[^\w.-]+/g, "_").replace(/^_+|_+$/g, "").toLowerCase() || "chat";
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
      downloadTextFile(`${safe}-${stamp}.txt`, lines.join("\n"), "text/plain;charset=utf-8");
      setChatNotice("Chat exported.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const downloadTextFile = (filename: string, content: string, mimeType = "text/plain;charset=utf-8") => {
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

  const exportAssistantMessage = async (message: JsonRecord, previousUserPrompt?: string) => {
    try {
      const content = str(message.content, "").trim();
      if (!content) throw new Error("Nothing to export.");
      const prompt = (previousUserPrompt || "").trim();
      const conversationTitle = str(selectedConversation?.title, "").trim() || "research";
      const heading = prompt || conversationTitle || "research";
      const safe = heading.replace(/[^\w.-]+/g, "_").replace(/^_+|_+$/g, "").toLowerCase() || "research";
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      const timestamp = str(message.timestamp, "").trim();
      const traceId = str(message.trace_id, "").trim();
      const lines: string[] = [];
      lines.push(`# ${heading}`);
      if (conversationId) lines.push(`conversation_id: ${conversationId}`);
      if (timestamp) lines.push(`assistant_timestamp: ${timestamp}`);
      if (traceId) lines.push(`trace_id: ${traceId}`);
      lines.push(`exported_at: ${new Date().toISOString()}`);
      lines.push("");
      if (prompt) {
        lines.push("## Prompt");
        lines.push(prompt);
        lines.push("");
      }
      lines.push("## Response");
      lines.push(content);
      lines.push("");
      downloadTextFile(`${safe}-${stamp}.md`, lines.join("\n"), "text/markdown;charset=utf-8");
      setChatNotice("Reply exported.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const copyMessage = async (message: JsonRecord) => {
    try {
      const role = str(message.role, "").toLowerCase();
      const content = str(message.content, "");
      await copyText(role === "user" ? stripAttachmentContextMarker(content) : content);
      setChatNotice("Message copied.");
    } catch (err) {
      setChatError(normalizeChatError(errMessage(err)));
    }
  };

  const deleteConversationMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/conversations/${encodeURIComponent(id)}`),
    onSuccess: async (_data, id) => {
      if (conversationId === id) startNewConversation();
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-messages", id] });
      setChatNotice("Chat deleted.");
    },
    onError: (err) => {
      setChatError(normalizeChatError(errMessage(err)));
    }
  });

  const deleteConversation = async (id: string) => {
    if (!id || isStreaming || deleteConversationMutation.isPending) return;
    const shouldDelete = typeof window === "undefined" ? true : window.confirm("Delete this chat and all its messages?");
    if (!shouldDelete) return;
    setChatError(null);
    await deleteConversationMutation.mutateAsync(id);
  };

  const openConversationMenu = (event: MouseEvent<HTMLElement>, conv: JsonRecord) => {
    event.stopPropagation();
    setConversationMenuAnchor(event.currentTarget);
    setConversationMenuTarget(conv);
  };

  const closeConversationMenu = () => {
    setConversationMenuAnchor(null);
    setConversationMenuTarget(null);
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
      time: directTime
    };
  };

  const ensureActivityStepTime = (step: JsonRecord, fallbackTime?: string): JsonRecord => {
    const normalized = normalizeActivityStepTime(step);
    if (str(normalized.time, "").trim()) return normalized;
    const stamp = (fallbackTime || new Date().toISOString()).trim();
    if (!stamp) return normalized;
    return {
      ...normalized,
      time: stamp
    };
  };

  const normalizeActivityStepForDisplay = (step: JsonRecord): JsonRecord => {
    const timedStep = normalizeActivityStepTime(step);
    if (isHeartbeatStreamingStep(timedStep)) {
      return {
        ...timedStep,
        step_type: "heartbeat",
        title: "Still Working",
        detail: normalizeHeartbeatDetailText(str(timedStep.detail, ""))
      };
    }

    const title = str(timedStep.title, "");
    const stepType = normalizeStatusText(str(timedStep.step_type, str(timedStep.type, "")));
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
      detail: summarized
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
          if (streamingStepDisplayKey(out[lastIdx]) === streamingStepDisplayKey(step)) {
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
      out.push(step);
    }
    return out;
  };

  const trimTrailingHeartbeatSteps = (steps: JsonRecord[]): JsonRecord[] => {
    const next = [...steps];
    while (next.length > 0 && isHeartbeatStreamingStep(asRecord(next[next.length - 1]))) {
      next.pop();
    }
    return next;
  };

  const countMeaningfulActivityCards = (
    cards: Array<ReturnType<typeof buildStepCard>>
  ): number => {
    const nonHeartbeat = cards.filter((card) => !card.isHeartbeat).length;
    return nonHeartbeat > 0 ? nonHeartbeat : cards.length;
  };

  const summarizeToolStartPayload = (name: string, payload: unknown): string => {
    const normalizedName = (name || "").trim().toLowerCase();
    if (normalizedName === "app_deploy") {
      const root = asRecord(payload);
      const nested = asRecord(root.payload);
      const rootFiles = asRecord(root.files);
      const nestedFiles = asRecord(nested.files);
      const filesObj = Object.keys(rootFiles).length > 0 ? rootFiles : nestedFiles;
      const fileCount = Object.keys(filesObj).length;
      const entryCommand = str(root.entry_command, str(nested.entry_command, "")).trim();
      if (fileCount > 0) {
        return `Preparing deployment package (${fileCount} file${fileCount === 1 ? "" : "s"}${entryCommand ? ", dynamic runtime" : ", static runtime"}).`;
      }
      return "Preparing deployment package.";
    }
    const compact = compactUnknown(payload, 320);
    if (!compact) return `Starting ${toHumanToolName(name)}.`;
    return simplifyConsoleDetail(compact);
  };

  const pushStreamingStep = (step: JsonRecord) => {
    setStreamingSteps((prev) => {
      const normalizedStep = ensureActivityStepTime(normalizeActivityStepForDisplay(step));
      const incomingHeartbeat = isHeartbeatStreamingStep(normalizedStep);
      let next: JsonRecord[];
      if (incomingHeartbeat) {
        const existingIndex = prev.findIndex((row) => isHeartbeatStreamingStep(row));
        if (existingIndex >= 0) {
          if (streamingStepDisplayKey(prev[existingIndex]) === streamingStepDisplayKey(normalizedStep)) {
            return prev;
          }
          next = [...prev];
          next[existingIndex] = attachStreamingStepStableKey(
            normalizedStep,
            getStreamingStepStableKey(prev[existingIndex])
          );
        } else {
          next = [...prev, attachStreamingStepStableKey(normalizedStep)];
        }
      } else {
        next = [...prev];
        const heartbeatIndex = next.findIndex((row) => isHeartbeatStreamingStep(row));
        if (heartbeatIndex >= 0) {
          next.splice(heartbeatIndex, 1);
        }
        const lastIdx = next.length - 1;
        const incomingKey = streamingStepDedupKey(normalizedStep);
        if (lastIdx >= 0 && streamingStepDedupKey(next[lastIdx]) === incomingKey) {
          next[lastIdx] = attachStreamingStepStableKey(
            normalizedStep,
            getStreamingStepStableKey(next[lastIdx])
          );
        } else {
          next.push(attachStreamingStepStableKey(normalizedStep));
        }
      }
      streamingStepsRef.current = next;
      return next;
    });
  };

  const runStreamingChat = async (
    message: string,
    files: File[] = [],
    opts?: {
      sensitive?: boolean;
      conversationIdOverride?: string;
      projectIdOverride?: string;
      statusSource?: string;
      deepResearch?: boolean;
      executionMode?: ChatExecutionMode;
    }
  ): Promise<boolean> => {
    const requestedConversationOverride = (opts?.conversationIdOverride || "").trim();
    const requestedProjectOverride = (opts?.projectIdOverride || "").trim();
    const targetConversationId =
      requestedConversationOverride || conversationId || generateConversationId();
    const targetProjectId = requestedProjectOverride || activeProjectId || "";
    let activeMessage =
      message.trim() ||
      (files.length > 0
        ? "Please analyze the attached documents and answer using them."
        : "");
    if (/^\s*use current (llm|model) (key|config)\s*$/i.test(activeMessage)) {
      const fallbackKey = (secretHelperKey || "OPENAI_API_KEY").trim().toUpperCase();
      activeMessage = `use current llm key for ${fallbackKey}`;
    }
    if (!activeMessage || isStreaming || streamLockRef.current) return false;
    const now = Date.now();
    const deepResearch = Boolean(opts?.deepResearch);
    const executionMode = opts?.executionMode || chatExecutionMode;
    const fingerprint = `${targetConversationId || "__new__"}::${targetProjectId || "__no_project__"}::${activeMessage
      .toLowerCase()
      .replace(/\s+/g, " ")
      .trim()}::${deepResearch ? "research" : "chat"}::${executionMode}`;
    const lastSend = recentSendRef.current;
    if (lastSend && lastSend.fingerprint === fingerprint && now - lastSend.at < 1500) {
      setChatNotice("Duplicate send ignored.");
      return false;
    }
    recentSendRef.current = { fingerprint, at: now };
    streamLockRef.current = true;

    setChatError(null);
    const sensitiveMessage =
      Boolean(opts?.sensitive) ||
      /^\s*set secret\s+/i.test(activeMessage) ||
      /^\s*use current (llm|model) (key|config)(?:\s+for\s+.+)?\s*$/i.test(activeMessage);
    setPendingUserMessage(sensitiveMessage ? null : activeMessage);
    setFailedUserMessage(null);
    setStreamingResponse("");
    setStreamingSteps([]);
    setStreamingProgressMessages([]);
    setLiveFileWrites({});
    setDeployedFiles([]);
    setCodeViewerFileIdx(0);
    setActiveChatTask(null);
    setStreamTraceOpen(false);
    setIsStreaming(true);

    if (conversationId !== targetConversationId) {
      setConversationId(targetConversationId);
    }
    if (!selectedConversationProjectId && targetProjectId && draftProjectId !== targetProjectId) {
      setDraftProjectId(targetProjectId);
    }
    const initialPendingSnapshot: ChatPendingRunSnapshot = {
      conversationId: targetConversationId,
      message: sensitiveMessage ? "" : activeMessage,
      projectId: targetProjectId,
      startedAt: Date.now(),
      streamingResponse: "",
      streamingSteps: [],
      failedUserMessage: ""
    };
    storeChatPendingRunSnapshot(initialPendingSnapshot);
    setPendingRunSnapshot(initialPendingSnapshot);

    let resolvedConversationId = targetConversationId;
    let payloadMessage = activeMessage;
    let streamError: string | null = null;
    const absorbConversationId = (payload: unknown) => {
      const obj = asRecord(payload);
      const cid = str(obj.conversation_id, str(obj.cid, str(obj.conversationId, "")));
      if (cid) {
        resolvedConversationId = cid;
        setPendingRunSnapshot((prev) => {
          const base = prev ?? initialPendingSnapshot;
          const next = { ...base, conversationId: cid };
          storeChatPendingRunSnapshot(next);
          return next;
        });
      }
    };

    try {
      if (files.length > 0) {
        setChatNotice(`Indexing ${files.length} attachment${files.length === 1 ? "" : "s"}...`);
        const uploaded = await uploadAttachmentsForKnowledge(files);
        if (uploaded.length > 0) {
          const refs = uploaded.map((item) => `doc:${item.id}`).join(", ");
          const names = uploaded.map((item) => item.filename).join(", ");
          payloadMessage = `${activeMessage}\n\n[Attached documents indexed for retrieval: ${refs}; files: ${names}]`;
          setChatNotice(
            `Indexed ${uploaded.length} attachment${uploaded.length === 1 ? "" : "s"} for retrieval.`
          );
        }
      }
        await api.chatStream(
          {
            message: payloadMessage,
            channel: "web",
            conversation_id: targetConversationId,
            project_id: targetProjectId || undefined,
            deep_research: deepResearch,
            execution_mode: executionMode,
            attachments_present: files.length > 0
          },
          {
          onEvent: (_eventName, payload) => {
            absorbConversationId(payload);
          },
          onToken: (token) => {
            setStreamingResponse((prev) => prev + token);
          },
          onThinking: (step) => {
            absorbConversationId(step);
            pushStreamingStep(step);
          },
          onToolStart: (name, payload) => {
            const payloadSummary = summarizeToolStartPayload(name, payload);
            pushStreamingStep({
              step_type: "tool_start",
              title: `Tool started: ${name}`,
              detail: payloadSummary || `Starting ${toHumanToolName(name)}.`,
              data: name
            });
            // Capture deployed app files for code viewer
            if (name === "app_deploy" && payload && typeof payload === "object") {
              const rec = payload as Record<string, unknown>;
              const nested = rec.payload && typeof rec.payload === "object"
                ? (rec.payload as Record<string, unknown>)
                : null;
              const files = (rec.files as Record<string, string> | undefined)
                || (nested?.files as Record<string, string> | undefined);
              const fileNames = Array.isArray(rec.file_names)
                ? rec.file_names
                : Array.isArray(nested?.file_names)
                  ? nested?.file_names
                  : [];
              const captured = files && typeof files === "object"
                ? Object.entries(files)
                    .filter(([, v]) => typeof v === "string")
                    .map(([k, v]) => ({ name: k, content: v as string }))
                : fileNames
                    .map((name) => str(name, "").trim())
                    .filter(Boolean)
                    .map((name) => ({ name, content: "" }));
              if (captured.length > 0) {
                setDeployedFiles((prev) => {
                  const merged = new Map(prev.map((file) => [file.name, file] as const));
                  for (const file of captured) {
                    const existing = merged.get(file.name);
                    merged.set(file.name, existing && existing.content ? existing : file);
                  }
                  return Array.from(merged.values());
                });
                setLiveFileWrites((prev) => {
                  const next = { ...prev };
                  for (const file of captured) {
                    if (!next[file.name]) {
                      const totalLines =
                        file.content.length > 0
                          ? file.content.split(/\r?\n/).length
                          : 0;
                      next[file.name] = {
                        content: file.content ? `${file.content}${file.content.endsWith("\n") ? "" : "\n"}` : "",
                        line: 0,
                        totalLines,
                        done: false
                      };
                    }
                  }
                  return next;
                });
                setCodeViewerFileIdx(0);
                setWorkspaceOpen(true);
              }
            }
          },
          onToolResult: (name, content, payload) => {
            const preview = content.trim().slice(0, 1600);
            const detail = simplifyConsoleDetail(summarizeActivityDetail(preview));
            if (name === "app_deploy") {
              setLiveFileWrites((prev) => {
                const next: Record<string, LiveFileWriteState> = {};
                for (const [file, state] of Object.entries(prev)) {
                  next[file] = { ...state, done: true };
                }
                return next;
              });
            }
            pushStreamingStep({
              step_type: "tool_result",
              title: `Tool finished: ${name || "tool"}`,
              detail: detail || preview,
              data: payload || detail || preview
            });
          },
          onToolProgress: (name, content, payload) => {
            const preview = content.trim().slice(0, 1600);
            const detail = simplifyConsoleDetail(summarizeActivityDetail(preview));
            const payloadObj = asRecord(payload);
            if (toBool(payloadObj.chat_visible)) {
              const chatMessage = str(payloadObj.chat_message, detail || preview).trim();
              if (chatMessage) {
                setStreamingProgressMessages((prev) => {
                  if (prev.some((entry) => entry === chatMessage)) return prev;
                  return [...prev.slice(-5), chatMessage];
                });
              }
            }
            const isFileWriteProgress =
              (name === "app_deploy" && str(payloadObj.kind, "") === "file_write") ||
              name === "file_write";
            if (isFileWriteProgress) {
              const fileName = str(payloadObj.file, "").trim();
              if (fileName) {
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
                  return {
                    ...prev,
                    [fileName]: {
                      content: nextContent,
                      line: Math.max(currentLine, lineNo),
                      totalLines: totalLines > 0 ? totalLines : (current?.totalLines ?? 0),
                      done
                    }
                  };
                });
                setDeployedFiles((prev) => {
                  if (prev.some((f) => f.name === fileName)) return prev;
                  return [...prev, { name: fileName, content: "" }];
                });
                setWorkspaceOpen(true);
              }
            }
            pushStreamingStep({
              step_type: "tool_progress",
              title: `Tool progress: ${name || "tool"}`,
              detail: detail || preview,
              data: Object.keys(payloadObj).length > 0 ? payloadObj : detail || preview
            });
          },
          onTaskStarted: (payload) => {
            const taskId = str(payload.task_id, "");
            const description = str(payload.description, "Task");
            const workType = str(payload.work_type, "task");
            if (!taskId) return;
            setActiveChatTask({
              id: taskId,
              description,
              status: "in_progress",
              workType
            });
            setChatNotice(`Task started: ${description}`);
            void queryClient.invalidateQueries({ queryKey: ["tasks"] });
            void queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
          },
          onTaskStatus: (payload) => {
            const taskId = str(payload.task_id, "");
            const description = str(payload.description, "Task");
            const status = str(payload.status, "");
            const workType = str(payload.work_type, "task");
            if (!taskId || !status) return;
            setActiveChatTask((prev) => ({
              id: taskId,
              description: description || prev?.description || "Task",
              status,
              workType: workType || prev?.workType || "task"
            }));
            const statusLabel =
              status === "completed"
                ? "completed"
                : status === "failed"
                  ? "failed"
                  : status === "paused"
                    ? "paused"
                    : status === "awaiting_approval"
                      ? "awaiting approval"
                      : status.replace(/_/g, " ");
            setChatNotice(`Task ${statusLabel}: ${description}`);
            void queryClient.invalidateQueries({ queryKey: ["tasks"] });
            void queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
          },
          onContent: (payload) => {
            const text = str(payload.content, "");
            if (text) setStreamingResponse(text);
            absorbConversationId(payload);
          },
          onError: (messageText) => {
            streamError = normalizeChatError(messageText);
          }
        }
      );
    } catch (err) {
      streamError = normalizeChatError(errMessage(err));
    } finally {
      if (streamError) {
        setChatError(streamError);
        if (!sensitiveMessage) {
          setFailedUserMessage(activeMessage);
        }
      }
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      if (!streamError) {
        setFailedUserMessage(null);
        const candidateConversationId = resolvedConversationId || targetConversationId;
        if (candidateConversationId) {
          try {
            await api.rawGet(`/conversations/${encodeURIComponent(candidateConversationId)}`);
            resolvedConversationId = candidateConversationId;
          } catch {
            resolvedConversationId = "";
          }
        }
        if (!resolvedConversationId) {
          try {
            const latest = await api.rawGet("/conversations?limit=1");
            const newest = pickRecords(latest, "conversations")[0];
            const newestId = str(newest?.id, "");
            if (newestId) resolvedConversationId = newestId;
          } catch {
            // Ignore fallback lookup failures; chat can still be selected manually.
          }
        }
        if (resolvedConversationId) {
          setConversationId(resolvedConversationId);
          await queryClient.invalidateQueries({ queryKey: ["chat-messages", resolvedConversationId] });
        }
      }
      if (!streamError) setAttachedFiles([]);
      if (!streamError && streamingStepsRef.current.length > 0) {
        setLastRunSteps(trimTrailingHeartbeatSteps(streamingStepsRef.current));
      }
      if (typeof window !== "undefined" && opts?.statusSource) {
        window.dispatchEvent(
          new CustomEvent<ChatRunStatusDetail>(CHAT_RUN_STATUS_EVENT, {
            detail: {
              conversationId: resolvedConversationId || targetConversationId,
              source: opts.statusSource,
              status: streamError ? "error" : "completed",
              message: streamError
                ? streamError
                : "ArkPulse fix completed. Review Chat for the result."
            }
          })
        );
      }
      if (streamError) {
        storeChatPendingRunSnapshot(null);
        setPendingRunSnapshot(null);
        setPendingUserMessage(null);
        setStreamingSteps([]);
        streamingStepsRef.current = [];
        setStreamingResponse("");
      }
      setIsStreaming(false);
      streamLockRef.current = false;
    }
    return !streamError;
  };

  useEffect(() => {
    if (typeof window === "undefined") return;
    const handleLaunchRun = (event: Event) => {
      const detail = (event as CustomEvent<ChatLaunchRunDetail>).detail;
      const message = str(detail?.message, "").trim();
      if (!message) {
        detail?.reject?.("No message provided.");
        return;
      }
      if (isStreaming || streamLockRef.current) {
        detail?.reject?.("Chat is already busy with another run. Wait for it to finish, then retry this fix.");
        return;
      }
      detail?.resolve?.(true);
      void runStreamingChat(message, [], {
        conversationIdOverride: str(detail?.conversationId, "").trim() || undefined,
        projectIdOverride: str(detail?.projectId, "").trim() || undefined,
        statusSource: str(detail?.source, "").trim() || undefined
      }).catch((err) => {
        if (typeof window !== "undefined" && detail?.source) {
          window.dispatchEvent(
            new CustomEvent<ChatRunStatusDetail>(CHAT_RUN_STATUS_EVENT, {
              detail: {
                conversationId: str(detail?.conversationId, "").trim(),
                source: detail.source,
                status: "error",
                message: errMessage(err)
              }
            })
          );
        }
      });
    };
    window.addEventListener(CHAT_LAUNCH_RUN_EVENT, handleLaunchRun as EventListener);
    return () => {
      window.removeEventListener(CHAT_LAUNCH_RUN_EVENT, handleLaunchRun as EventListener);
    };
  }, [isStreaming, runStreamingChat]);

  // Pin scroll to bottom during streaming — useLayoutEffect runs before paint
  // so the user never sees the intermediate jank position.
  const stickToBottom = useRef(true);
  useLayoutEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    if (stickToBottom.current) {
      thread.scrollTop = thread.scrollHeight;
    }
  });
  // Track whether user is near bottom to decide if we should auto-stick
  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    const onScroll = () => {
      const nearBottom = thread.scrollHeight - thread.scrollTop - thread.clientHeight < 80;
      stickToBottom.current = nearBottom;
    };
    thread.addEventListener("scroll", onScroll, { passive: true });
    return () => thread.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    if (!pendingUserMessage) return;
    // Don't clear pending message while streaming — avoids layout jump
    // when the pending row disappears and the real message appears above.
    if (isStreaming) return;
    const pendingNormalized = stripAttachmentContextMarker(pendingUserMessage).trim();
    if (!pendingNormalized) {
      setPendingUserMessage(null);
      return;
    }
    const lastUserMessage = [...messages]
      .reverse()
      .find((m) => str(m.role, "").toLowerCase() === "user");
    if (!lastUserMessage) return;
    const lastUserText = stripAttachmentContextMarker(str(lastUserMessage.content, "")).trim();
    if (lastUserText === pendingNormalized) {
      setPendingUserMessage(null);
    }
  }, [messages, pendingUserMessage, isStreaming]);

  useEffect(() => {
    if (!chatNotice) return;
    const timer = window.setTimeout(() => setChatNotice(null), 2200);
    return () => window.clearTimeout(timer);
  }, [chatNotice]);

  const hasRecoveredStream = !isStreamingForCurrentConversation && hasPendingSnapshotForConversation;
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
  const finalMessageLanded =
    !isStreamingForCurrentConversation &&
    messages.length > streamStartMsgCount.current &&
    str(lastMsg?.role, "").toLowerCase() === "assistant";
  // Show streaming bubble while streaming OR while waiting for final message to land
  const showStreamingAssistant =
    isStreamingForCurrentConversation || (hasRecoveredStream && !finalMessageLanded);
  const visiblePendingUserMessage = hasPendingSnapshotForConversation ? pendingUserMessage : null;
  const visibleFailedUserMessage = hasPendingSnapshotForConversation ? failedUserMessage : null;
  const visibleStreamingResponse = hasPendingSnapshotForConversation ? streamingResponse : "";
  const visibleStreamingProgressMessages = hasPendingSnapshotForConversation
    ? streamingProgressMessages
    : [];
  const hasLiveThreadActivity = Boolean(
    visiblePendingUserMessage ||
    isStreamingForCurrentConversation ||
    hasPendingSnapshotForConversation ||
    visibleStreamingResponse.trim()
  );
  const hasRenderableThread = messages.length > 0 || hasLiveThreadActivity;
  const showEmptyHero =
    !hasRenderableThread &&
    !showStreamingAssistant &&
    !visiblePendingUserMessage &&
    !visibleFailedUserMessage;
  const starterPrompts = [
    {
      label: "Review recent changes",
      prompt: "Review recent changes and list only the critical risks."
    },
    {
      label: "Plan a UI cleanup",
      prompt: "Plan and implement a cleaner UI that fits the viewport better."
    },
    {
      label: "Summarize the project",
      prompt: "Summarize the current project architecture and suggest the next steps."
    }
  ];
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const latestAssistantMessageText = str(
    [...messages].reverse().find((m) => str(m.role, "").toLowerCase() === "assistant")?.content,
    ""
  );
  const latestAssistantTraceSteps = latestAssistantTraceId
    ? traceStepsById[latestAssistantTraceId] || []
    : [];
  const completedLastRunSteps = trimTrailingHeartbeatSteps(lastRunSteps);
  const completedPersistedTraceSteps = trimTrailingHeartbeatSteps(latestAssistantTraceSteps);
  const completedWorkspaceSteps =
    completedPersistedTraceSteps.length >= completedLastRunSteps.length &&
    completedPersistedTraceSteps.length > 0
      ? completedPersistedTraceSteps
      : completedLastRunSteps;

  const workspaceSteps =
    (showStreamingAssistant || hasPendingSnapshotForConversation) && streamingSteps.length > 0
      ? trimTrailingHeartbeatSteps(streamingSteps)
      : completedWorkspaceSteps;
  const workspaceCards = useMemo(() => {
    return workspaceSteps.map((step, idx) => safeBuildStepCard(step, idx));
  }, [workspaceSteps]);
  const expandedTraceCardsById = useMemo(() => {
    const expanded = new Set<string>();
    messages.forEach((message, idx) => {
      const messageId = str(message.id, String(idx));
      if (!messageTraceOpen[messageId]) return;
      const traceId = str(message.trace_id, "").trim();
      if (traceId) expanded.add(traceId);
    });
    const out: Record<string, Array<ReturnType<typeof buildStepCard>>> = {};
    expanded.forEach((traceId) => {
      const steps = traceStepsById[traceId] || [];
      out[traceId] = steps.map((step, idx) => safeBuildStepCard(step, idx));
    });
    return out;
  }, [messages, messageTraceOpen, traceStepsById]);
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
        detail: row.detail || "",
        time: row.time || "",
        tone: row.tone
      });
    }
    return rows.slice(-16);
  }, [workspaceCards]);
  const progressDoneCount = useMemo(
    () => progressRows.filter((row) => row.status === "done").length,
    [progressRows]
  );

  const codeFromCards = (() => {
    for (let i = workspaceCards.length - 1; i >= 0; i -= 1) {
      const detail = str(workspaceCards[i]?.rawDetailFull, workspaceCards[i]?.detail || "").trim();
      const fenced = extractFirstCodeFence(detail);
      if (fenced) return fenced;
      if (detail.length > 80 && /(import |const |function |class |=>|<div|SELECT |INSERT |CREATE )/i.test(detail)) {
        return detail;
      }
    }
    return "";
  })();
  const codeSnapshot =
    codeFromCards ||
    extractFirstCodeFence(streamingResponse) ||
    extractFirstCodeFence(latestAssistantMessageText);
  const activeCodeFile = deployedFiles[codeViewerFileIdx] ?? null;
  const activeLiveWrite = activeCodeFile ? liveFileWrites[activeCodeFile.name] : undefined;
  const codeViewerContent = activeLiveWrite
    ? activeLiveWrite.content
    : (activeCodeFile?.content ?? "");
  const codeViewerWriteStatus = activeLiveWrite
    ? activeLiveWrite.totalLines > 0
      ? `${Math.min(activeLiveWrite.line, activeLiveWrite.totalLines)}/${activeLiveWrite.totalLines} lines written${activeLiveWrite.done ? " (done)" : ""}`
      : activeLiveWrite.done
        ? "File write complete"
        : "Writing file..."
    : "";

  const appsWorkspaceQ = useQuery({
    queryKey: ["chat-workspace-apps"],
    queryFn: () => api.rawGet("/api/apps"),
    enabled: workspaceOpen,
    refetchInterval: workspaceOpen && autoRefresh ? REFRESH_MS : false
  });
  const tunnelWorkspaceQ = useQuery({
    queryKey: ["chat-workspace-tunnel"],
    queryFn: () => api.rawGet("/tunnel/status"),
    enabled: workspaceOpen,
    refetchInterval: workspaceOpen && autoRefresh ? REFRESH_MS : false
  });
  const workspaceApps = pickRecords(appsWorkspaceQ.data, "apps");
  const workspaceTunnel = asRecord(tunnelWorkspaceQ.data);
  const workspaceTunnelBaseUrl = str(workspaceTunnel.url, "").trim().replace(/\/+$/, "");
  const workspaceSelectedPublicAppId = str(workspaceTunnel.selected_app_id, "").trim();
  const activeWorkspaceApp = useMemo(() => {
    if (workspaceApps.length === 0) return null;
    const running = workspaceApps.find(
      (app) => toBool(app.running) || str(app.running, "").toLowerCase() === "true"
    );
    return running || workspaceApps[0];
  }, [workspaceApps]);
  const previewPath = str(activeWorkspaceApp?.access_url, str(activeWorkspaceApp?.url, "")).trim();
  const previewUrl = toAbsoluteAppUrl(previewPath, origin);
  const previewImagePath = useMemo(() => {
    const streamImage = extractPreviewImageUrl(streamingResponse);
    if (streamImage) return streamImage;
    return extractPreviewImageUrl(latestAssistantMessageText);
  }, [streamingResponse, latestAssistantMessageText]);
  const previewImageUrl = toAbsoluteAppUrl(previewImagePath, origin);
  const publicPreviewUrl =
    workspaceTunnelBaseUrl && workspaceTunnelBaseUrl !== origin && workspaceSelectedPublicAppId
      ? toAbsoluteAppUrl(previewPath, workspaceTunnelBaseUrl)
      : "";
  const runtimeMode = str(activeWorkspaceApp?.runtime_mode, "").toLowerCase();
  const runtimeSummary = (() => {
    if (runtimeMode === "isolated_container") {
      return { label: "Sandboxed container", tone: "success" as const };
    }
    if (runtimeMode === "local_process_fallback") {
      return { label: "Local process fallback", tone: "warning" as const };
    }
    if (runtimeMode === "static") {
      return { label: "Static app hosting", tone: "default" as const };
    }
    if (runtimeMode === "stopped") {
      return { label: "Stopped", tone: "default" as const };
    }
    if (toBool(activeWorkspaceApp?.is_static)) {
      return { label: "Static app hosting", tone: "default" as const };
    }
    if (toBool(activeWorkspaceApp?.running)) {
      return { label: "Dynamic app runtime", tone: "default" as const };
    }
    return { label: "No app runtime yet", tone: "default" as const };
  })();
  const showWorkspacePanel = workspaceOpen;
  const showConversationSidebar = conversationSidebarOpen;
  const chatErrorLower = (chatError || "").toLowerCase();
  const chatErrorNormalized = chatError ? normalizeChatError(chatError).toLowerCase() : "";
  const apiKeyActionNeeded =
    !!chatError &&
    (/required api key is missing/.test(chatErrorLower) ||
      /missing authorization/.test(chatErrorLower) ||
      /bearer.{0,3}api.?key/i.test(chatErrorLower) ||
      /missing.*api[_\s-]?key/.test(chatErrorLower) ||
      /api.?key.*required/.test(chatErrorLower) ||
      /api.?key.*required/.test(chatErrorNormalized) ||
      /^unauthorized\b/.test(chatErrorLower));
  const extractedKeyHints =
    chatError?.match(/\b[A-Z][A-Z0-9_]{2,}\b/g)?.filter((v) => /KEY|TOKEN|SECRET/.test(v)) ?? [];
  const suggestedSecretKey = extractedKeyHints[0] || secretHelperKey;
  const latestRunningCard = useMemo(
    () => [...workspaceCards].reverse().find((row) => row.kind === "Running" || row.kind === "Planning") || null,
    [workspaceCards]
  );
  const latestIssueCard = useMemo(
    () => [...workspaceCards].reverse().find((row) => row.kind === "Issue") || null,
    [workspaceCards]
  );
  const safetyPolicyBlocked = isSafetyPolicyBlockedText(
    `${latestIssueCard?.label || ""} ${latestIssueCard?.detail || ""} ${latestIssueCard?.detailFull || ""}`
  );
  const hasCompletedWorkspaceRun =
    !showStreamingAssistant &&
    !latestIssueCard &&
    workspaceCards.length > 0 &&
    latestWorkspaceCard?.kind === "Done";
  const progressSummary = !progressRows.length
    ? "No activity yet"
    : showStreamingAssistant
      ? `${workspaceCards.length} log line${workspaceCards.length === 1 ? "" : "s"}`
      : hasCompletedWorkspaceRun
        ? `Run completed • ${workspaceCards.length} log line${workspaceCards.length === 1 ? "" : "s"}`
        : `${workspaceCards.length} log line${workspaceCards.length === 1 ? "" : "s"}`;
  const runState = apiKeyActionNeeded
    ? ("waiting_input" as const)
    : isStreamingForCurrentConversation
      ? ("running" as const)
      : ("stopped" as const);
  const runStateLabel =
    runState === "running"
      ? "RUNNING"
      : runState === "waiting_input"
        ? "WAITING INPUT"
        : "STOPPED";
  const runStateChipColor = runState === "running" ? "info" : runState === "waiting_input" ? "warning" : "default";
  const workspaceStatusCopy = useMemo(() => {
    if (apiKeyActionNeeded) {
      return {
        line1: "Status: Waiting for your approval/input",
        line2: "An API key is needed before deployment can continue.",
        tone: "warning"
      };
    }
    if (safetyPolicyBlocked) {
      return {
        line1: "Status: Blocked by safety policy",
        line2: "The agent tried a disallowed tool and needs a different approach.",
        tone: "warning"
      };
    }
    if (isStreamingForCurrentConversation) {
      const active = latestRunningCard || latestWorkspaceCard;
      return {
        line1: "Status: Running",
        line2: active?.detail || "Agent is actively running actions.",
        tone: "info"
      };
    }
    if (latestWorkspaceCard?.kind === "Done") {
      return {
        line1: "Status: Completed",
        line2: `Last completed step: ${latestWorkspaceCard.label}`,
        tone: "default"
      };
    }
    if (latestWorkspaceCard) {
      return {
        line1: "Status: Stopped",
        line2: `Last completed step: ${latestWorkspaceCard.label}`,
        tone: "default"
      };
    }
    return {
      line1: "Status: Stopped",
      line2: "Send a request to start a run.",
      tone: "default"
    };
  }, [apiKeyActionNeeded, isStreamingForCurrentConversation, latestRunningCard, latestWorkspaceCard, safetyPolicyBlocked]);
  const nowDoingLabel = useMemo(() => {
    if (apiKeyActionNeeded) return "Waiting for your approval/input";
    if (safetyPolicyBlocked) return "Blocked by safety policy";
    const active = latestRunningCard || latestWorkspaceCard;
    return active?.label || "Waiting for next step";
  }, [apiKeyActionNeeded, latestRunningCard, latestWorkspaceCard, safetyPolicyBlocked]);

  const submitSecretHelper = async (modeOverride?: "reuse" | "manual") => {
    if (secretHelperBusy || isStreaming) return;
    const key = (secretHelperKey || "").trim().toUpperCase();
    const mode = modeOverride || secretHelperMode;
    if (!key) {
      setChatError("Enter which key name to set first (example: OPENAI_API_KEY).");
      return;
    }
    setSecretHelperBusy(true);
    setChatError(null);
    try {
      if (mode === "reuse") {
        const ok = await runStreamingChat(`use current llm key for ${key}`, [], { sensitive: true });
        if (!ok) return;
        setChatNotice(`Saved ${key} from current model key.`);
        return;
      }
      if (!secretHelperValue.trim()) {
        setChatError("Enter the key value first.");
        return;
      }
      const ok = await runStreamingChat(`set secret ${key}=${secretHelperValue}`, [], {
        sensitive: true
      });
      if (!ok) return;
      setSecretHelperValue("");
      setChatNotice(`${key} saved securely.`);
    } finally {
      setSecretHelperBusy(false);
    }
  };

  useEffect(() => {
    if (!apiKeyActionNeeded) return;
    if (!secretHelperKey || secretHelperKey === "OPENAI_API_KEY") {
      setSecretHelperKey(suggestedSecretKey);
    }
  }, [apiKeyActionNeeded, suggestedSecretKey, secretHelperKey]);

  useEffect(() => {
    if (!activityAutoFollow) return;
    const node = workspaceActivityRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
  }, [workspaceCards.length, activityAutoFollow, isStreaming]);

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
          lg: showWorkspacePanel
            ? "minmax(0,1fr) clamp(300px, 24vw, 340px)"
            : "minmax(0,1fr)",
          xl: showWorkspacePanel
            ? showConversationSidebar
              ? "clamp(210px, 16vw, 232px) minmax(0,1fr) clamp(300px, 24vw, 340px)"
              : "minmax(0,1fr) clamp(300px, 24vw, 340px)"
            : showConversationSidebar
              ? "clamp(210px, 16vw, 232px) minmax(0,1fr)"
              : "minmax(0,1fr)"
        },
        gap: 1
      }}
    >
      {showConversationSidebar ? (
        <Box
          className="list-shell chat-sidebar"
          sx={{
            minHeight: 0,
            display: "flex",
            flexDirection: "column",
            maxHeight: { xs: 280, xl: "none" }
          }}
        >
          <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
            <Typography variant="h6">Conversations</Typography>
            <Button size="small" onClick={startNewConversation}>
              New
            </Button>
          </Stack>

          <Box sx={{ flex: 1, minHeight: 0, overflow: "auto", pr: 0.5 }}>
            <Stack spacing={0.5} className="conversation-list">
              {conversations.length === 0 ? (
                <Typography variant="body2" color="text.secondary">
                  No conversations yet.
                </Typography>
              ) : (
                conversations.map((conv) => {
                  const id = str(conv.id, "");
                  const active = conversationId === id;
                  const title = str(conv.title, "Untitled")
                    .replace(/\s+/g, " ")
                    .trim() || "Untitled";
                  return (
                    <Box
                      key={id}
                      className={active ? "conversation-card active" : "conversation-card"}
                      onClick={() => {
                        openConversationById(id);
                      }}
                      role="button"
                      tabIndex={0}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          openConversationById(id);
                        }
                      }}
                    >
                      <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={0.5}>
                        <Box sx={{ minWidth: 0, flex: 1 }}>
                          <div className="conversation-card-title" title={title}>
                            {title}
                          </div>
                          {(() => {
                            const updatedAt = str(conv.updated_at, "");
                            if (!updatedAt) return null;
                            const parsed = formatChatTimestamp(updatedAt);
                            return (
                              <Typography
                                variant="caption"
                                color="text.secondary"
                                sx={{ display: "block", mt: 0.15, opacity: 0.88 }}
                                title={parsed.tooltip}
                              >
                                {parsed.label}
                              </Typography>
                            );
                          })()}
                        </Box>
                        <Tooltip title="Chat options">
                          <span>
                            <IconButton
                              size="small"
                              className="conversation-card-menu"
                              onClick={(e) => {
                                openConversationMenu(e, conv);
                              }}
                              disabled={deleteConversationMutation.isPending}
                            >
                              <MoreVertIcon fontSize="small" />
                            </IconButton>
                          </span>
                        </Tooltip>
                      </Stack>
                    </Box>
                  );
                })
              )}
            </Stack>
          </Box>
          <Menu anchorEl={conversationMenuAnchor} open={Boolean(conversationMenuAnchor)} onClose={closeConversationMenu}>
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
      ) : null}

      <Box
        className={`list-shell chat-shell chat-density-immersive${isDragOverChat ? " chat-shell-drop-active" : ""}`}
        sx={{ minHeight: 0, display: "flex", flexDirection: "column", position: "relative" }}
        onDragEnter={handleChatDragEnter}
        onDragOver={handleChatDragOver}
        onDragLeave={handleChatDragLeave}
        onDrop={handleChatDrop}
      >
        <Stack direction={{ xs: "column", sm: "row" }} justifyContent="space-between" alignItems={{ xs: "stretch", sm: "center" }} spacing={1} mb={1}>
          <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0, flexWrap: "wrap" }} useFlexGap>
            <Button
              size="small"
              variant={showConversationSidebar ? "contained" : "outlined"}
              startIcon={showConversationSidebar ? <ChevronLeftRoundedIcon fontSize="small" /> : <ChevronRightRoundedIcon fontSize="small" />}
              onClick={() => setConversationSidebarOpen((prev) => !prev)}
              sx={{ textTransform: "none", borderRadius: 999 }}
            >
              {showConversationSidebar ? "Hide conversations" : "Conversations"}
            </Button>
            <Avatar src={AgentLogo} variant="rounded" sx={{ width: 28, height: 28, bgcolor: "rgba(12,22,40,0.85)" }} />
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="h6">Chat</Typography>
              <Typography variant="caption" color="text.secondary">
                Keep the active task centered.
              </Typography>
            </Box>
          </Stack>
          <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0, flexWrap: "wrap", justifyContent: { xs: "flex-start", sm: "flex-end" } }} useFlexGap>
            <Tooltip title={showWorkspacePanel ? "Hide agent activity" : "Show agent activity"}>
              <span
                className={`activity-toggle-pill${showWorkspacePanel ? " active" : ""}${isStreamingForCurrentConversation ? " streaming" : ""}`}
                onClick={() => setWorkspaceOpen((prev) => !prev)}
                style={{ display: "inline-flex" }}
              >
                <span className="toggle-dot" />
                Activity
              </span>
            </Tooltip>
            {conversationId ? (
              <Typography variant="caption" color="text.secondary" sx={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" }}>
                ID: {conversationId}
              </Typography>
            ) : (
              <Typography variant="caption" color="text.secondary">
                Draft chat
              </Typography>
            )}
          </Stack>
        </Stack>
        <Box
          className="chat-main-column"
          sx={{
            flex: 1,
            minHeight: 0,
            width: "100%",
            display: "flex",
            flexDirection: "column"
          }}
        >
          <Stack direction={{ xs: "column", md: "row" }} spacing={1} sx={{ mb: 1 }}>
            <TextField
              fullWidth
              size="small"
              select
              label="Project Scope"
              value={selectedConversationProjectId || draftProjectId}
              onChange={(e) => setDraftProjectId(e.target.value)}
              disabled={Boolean(selectedConversation)}
              sx={{ maxWidth: { xs: "100%", md: 360 } }}
            >
              <MenuItem value="">No project</MenuItem>
              {projects.map((project) => {
                const id = str(project.id, "");
                if (!id) return null;
                return (
                  <MenuItem key={id} value={id}>
                    {str(project.name, id)}
                  </MenuItem>
                );
              })}
            </TextField>
          </Stack>

          <Box
            ref={threadRef}
            sx={{ flex: 1, minHeight: 0, overflow: "auto" }}
            className={`chat-thread chat-thread-immersive${showEmptyHero ? " chat-thread-empty" : ""}`}
          >
            {showEmptyHero ? (
              <Box className="chat-empty-state">
                <Typography variant="overline" className="chat-empty-kicker">
                  {conversationId ? "Draft Conversation" : "Focused Chat"}
                </Typography>
                <Typography variant="h4" className="chat-empty-title">
                  {conversationId ? "This conversation is ready for its first task." : "Tell AgentArk the outcome you want."}
                </Typography>
                <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 520 }}>
                  {conversationId
                    ? "Send one message and AgentArk will turn this draft into a working run."
                    : "Start from the result, not the implementation details. The workspace is shaped to keep that request centered."}
                </Typography>
                <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" justifyContent="center">
                  {starterPrompts.map((item) => (
                    <Button
                      key={item.label}
                      size="small"
                      variant="outlined"
                      className="chat-quick-cmd-btn"
                      onClick={() => {
                        setPrompt(item.prompt);
                        setChatError(null);
                      }}
                    >
                      {item.label}
                    </Button>
                  ))}
                </Stack>
              </Box>
            ) : (
            <Stack spacing={1.2}>
              {messages.map((message, idx) => {
                const role = str(message.role, "").toLowerCase();
                const isUser = role === "user";
                const messageId = str(message.id, String(idx));
                // Skip the last user message if it duplicates the pending message during streaming
                if (isUser && isStreamingForCurrentConversation && visiblePendingUserMessage && idx === messages.length - 1) {
                  const pendingNorm = stripAttachmentContextMarker(visiblePendingUserMessage).trim();
                  const msgNorm = stripAttachmentContextMarker(str(message.content, "")).trim();
                  if (pendingNorm === msgNorm) return null;
                }
                const tsRaw = str(message.timestamp, "");
                const ts = tsRaw ? formatChatTimestamp(tsRaw) : null;
                const content = str(message.content);
                const renderedContent = isUser ? stripAttachmentContextMarker(content) : content;
                const previousUserPrompt = !isUser
                  ? (() => {
                      for (let cursor = idx - 1; cursor >= 0; cursor -= 1) {
                        const candidate = asRecord(messages[cursor]);
                        if (str(candidate.role, "").toLowerCase() !== "user") continue;
                        return stripAttachmentContextMarker(str(candidate.content, ""));
                      }
                      return "";
                    })()
                  : "";
                const traceId = str(message.trace_id, "").trim();
                const hasTrace = !isUser && !!traceId;
                const traceLoading = hasTrace ? Boolean(traceLoadingById[traceId]) : false;
                const traceError = hasTrace ? str(traceErrorById[traceId], "").trim() : "";
                const rawTraceSteps = hasTrace ? traceStepsById[traceId] || [] : [];
                const traceExpanded = Boolean(messageTraceOpen[messageId]);
                const traceCards = traceExpanded && hasTrace ? expandedTraceCardsById[traceId] || [] : [];
                const traceSummary = traceSummaryFromSteps(rawTraceSteps, { loading: traceLoading, error: traceError });
                return (
                  <Box key={messageId} className={isUser ? "chat-row chat-row-user" : "chat-row"}>
                    {!isUser ? (
                      <Avatar
                        variant="rounded"
                        className="chat-avatar"
                        sx={{ width: 30, height: 30, bgcolor: "rgba(12,22,40,0.85)" }}
                      >
                        <SmartToyRoundedIcon sx={{ fontSize: 16 }} />
                      </Avatar>
                    ) : null}
                    <Box className={isUser ? "chat-bubble chat-bubble-user" : "chat-bubble chat-bubble-assistant"}>
                      <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={0.5}>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          title={ts?.tooltip || undefined}
                        >
                          {isUser ? "You" : "AgentArk"}{ts ? ` | ${ts.label}` : ""}
                        </Typography>
                        <Stack direction="row" spacing={0.25} alignItems="center">
                          {!isUser ? (
                            <Tooltip title="Download reply">
                              <IconButton
                                size="small"
                                onClick={() => {
                                  void exportAssistantMessage(message, previousUserPrompt);
                                }}
                                sx={{ color: "rgba(189, 216, 249, 0.9)" }}
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
                              sx={{ color: "rgba(189, 216, 249, 0.9)" }}
                            >
                              <ContentCopyRoundedIcon fontSize="small" />
                            </IconButton>
                          </Tooltip>
                        </Stack>
                      </Stack>
                      {/* Trace steps shown in Activity panel on the right */}
                      {isUser ? (
                        <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                          {renderedContent}
                        </Typography>
                      ) : (
                        renderChatMarkdown(renderedContent)
                      )}
                    </Box>
                    {isUser ? (
                      <Avatar className="chat-avatar chat-avatar-user" sx={{ width: 30, height: 30, bgcolor: "rgba(47,212,255,0.18)" }}>
                        <PersonRoundedIcon sx={{ fontSize: 16 }} />
                      </Avatar>
                    ) : null}
                  </Box>
                );
              })}

              {visiblePendingUserMessage && showStreamingAssistant ? (
                <Box className="chat-row chat-row-user">
                  <Box className="chat-bubble chat-bubble-user">
                    <Typography variant="caption" color="text.secondary">
                      You | sending...
                    </Typography>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {visiblePendingUserMessage}
                    </Typography>
                  </Box>
                  <Avatar className="chat-avatar chat-avatar-user" sx={{ width: 30, height: 30, bgcolor: "rgba(47,212,255,0.18)" }}>
                    U
                  </Avatar>
                </Box>
              ) : null}

              {visibleFailedUserMessage && !isStreamingForCurrentConversation ? (
                <Box className="chat-row chat-row-user">
                  <Box className="chat-bubble chat-bubble-user">
                    <Typography variant="caption" color="warning.main">
                      You | not sent
                    </Typography>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {visibleFailedUserMessage}
                    </Typography>
                  </Box>
                  <Avatar className="chat-avatar chat-avatar-user" sx={{ width: 30, height: 30, bgcolor: "rgba(47,212,255,0.18)" }}>
                    U
                  </Avatar>
                </Box>
              ) : null}

              {showStreamingAssistant && visibleStreamingProgressMessages.length > 0
                ? visibleStreamingProgressMessages.map((msg) => (
                    <Box className="chat-row" key={`stream-progress-${msg}`}>
                      <Avatar
                        variant="rounded"
                        className="chat-avatar"
                        sx={{ width: 30, height: 30, bgcolor: "rgba(12,22,40,0.85)" }}
                      >
                        <SmartToyRoundedIcon sx={{ fontSize: 16 }} />
                      </Avatar>
                      <Box className="chat-bubble chat-bubble-assistant">
                        <Typography variant="caption" color="text.secondary">
                          AgentArk | working...
                        </Typography>
                        <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                          {msg}
                        </Typography>
                      </Box>
                    </Box>
                  ))
                : null}

              {showStreamingAssistant ? (
                <Box className="chat-row">
                  <Avatar
                    variant="rounded"
                    className="chat-avatar"
                    sx={{ width: 30, height: 30, bgcolor: "rgba(12,22,40,0.85)" }}
                  >
                    <SmartToyRoundedIcon sx={{ fontSize: 16 }} />
                  </Avatar>
                  <Box className="chat-bubble chat-bubble-assistant chat-bubble-streaming">
                    <Typography variant="caption" color="text.secondary" className="chat-streaming-status">
                      {visibleStreamingResponse.trim() ? "AgentArk is streaming..." : streamingActivity}
                    </Typography>
                    {visibleStreamingResponse.trim() ? (
                      <>
                        {renderChatMarkdown(visibleStreamingResponse)}
                        <span className="stream-caret" />
                      </>
                    ) : (
                      <div className="typing-dots" aria-label="typing">
                        <span />
                        <span />
                        <span />
                      </div>
                    )}
                  </Box>
                </Box>
              ) : null}
            </Stack>
          )}
        </Box>

        {convQ.error || messagesQ.error || (chatError && !apiKeyActionNeeded) ? (
          <Alert severity="error" sx={{ mt: 1 }}>
            {normalizeChatError(chatError || errMessage(convQ.error || messagesQ.error))}
          </Alert>
        ) : null}
        {apiKeyActionNeeded ? (
          <Box className="chat-action-required" sx={{ mt: 1 }}>
            <Stack spacing={1}>
              <Typography variant="subtitle2">Waiting for your input</Typography>
              <Typography variant="body2" color="text.secondary">
                I need an API key before I can continue building and deploying this app.
              </Typography>
              <Stack direction={{ xs: "column", md: "row" }} spacing={1} className="chat-action-options">
                <Button
                  size="small"
                  variant={secretHelperMode === "reuse" ? "contained" : "outlined"}
                  onClick={async () => {
                    setSecretHelperMode("reuse");
                    await submitSecretHelper("reuse");
                  }}
                >
                  Use current LLM key
                </Button>
                <Button
                  size="small"
                  variant={secretHelperMode === "manual" ? "contained" : "outlined"}
                  onClick={() => setSecretHelperMode("manual")}
                >
                  Add API key manually
                </Button>
              </Stack>
              <Typography variant="caption" color="text.secondary">
                Quick message also works: <code>use current llm key</code>
              </Typography>
              <Stack direction={{ xs: "column", md: "row" }} spacing={1} alignItems={{ md: "center" }}>
                <TextField
                  size="small"
                  label="Key name"
                  value={secretHelperKey}
                  onChange={(e) => setSecretHelperKey(e.target.value.toUpperCase())}
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
                  <Typography variant="caption" color="text.secondary" sx={{ flex: 1 }}>
                    Reuses your current model key and stores it encrypted for this app.
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
                  {secretHelperBusy ? "Saving..." : "Continue build"}
                </Button>
              </Stack>
            </Stack>
          </Box>
        ) : null}
        {chatNotice && !(convQ.error || messagesQ.error || chatError) ? (
          <Alert severity="info" sx={{ mt: 1 }}>
            {chatNotice}
          </Alert>
        ) : null}
        {isDragOverChat ? (
          <Box className="chat-drop-overlay">
            <Typography variant="subtitle2">Drop files to attach</Typography>
            <Typography variant="caption" color="text.secondary">
              Supported: TXT, MD, JSON, CSV, XML, YAML, PDF, DOCX, LOG, HTML
            </Typography>
          </Box>
        ) : null}

        <input
          ref={fileInputRef}
          type="file"
          multiple
          accept=".txt,.md,.markdown,.json,.csv,.tsv,.xml,.yaml,.yml,.pdf,.docx,.log,.html,.htm"
          style={{ display: "none" }}
          onChange={(e) => {
            queueAttachedFiles(e.target.files);
            e.currentTarget.value = "";
          }}
        />
        {attachedFiles.length > 0 ? (
          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" sx={{ mb: 0.5 }}>
            {attachedFiles.map((file, idx) => (
              <Chip
                key={`${file.name}-${file.size}-${file.lastModified}-${idx}`}
                size="small"
                label={file.name}
                onDelete={isStreaming ? undefined : () => removeAttachedFile(idx)}
              />
            ))}
          </Stack>
        ) : null}
        {activeChatTask ? (
          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" alignItems="center" sx={{ mb: 0.5 }}>
            <Chip
              size="small"
              color={
                activeChatTask.status === "completed"
                  ? "success"
                  : activeChatTask.status === "failed"
                    ? "error"
                    : activeChatTask.status === "paused" || activeChatTask.status === "awaiting_approval"
                      ? "warning"
                      : "info"
              }
              label={`Task ${activeChatTask.status === "in_progress" ? "running" : activeChatTask.status.replace(/_/g, " ")}`}
            />
            <Chip
              size="small"
              variant="outlined"
              label={
                activeChatTask.workType === "app"
                  ? "App"
                  : activeChatTask.workType === "import"
                    ? "Import"
                    : activeChatTask.workType === "automation"
                      ? "Automation"
                      : activeChatTask.workType === "workspace"
                        ? "Workspace"
                        : activeChatTask.workType === "research"
                          ? "Research"
                          : "Task"
              }
            />
            <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0 }}>
              {activeChatTask.description}
            </Typography>
          </Stack>
        ) : null}
        <Box className="chat-composer-shell">
          <textarea
            className="chat-composer-textarea"
            placeholder="Message (Enter to send, Shift+Enter for newline)"
            aria-label="Message"
            value={prompt}
            onChange={(e) => {
              setPrompt(e.target.value);
              const el = e.target;
              el.style.height = "auto";
              el.style.height = `${Math.min(el.scrollHeight, 150)}px`;
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                if (isStreaming || (!prompt.trim() && attachedFiles.length === 0)) return;
                const msg = prompt.trim();
                setPrompt("");
                (e.target as HTMLTextAreaElement).style.height = "auto";
                setChatError(null);
                void runStreamingChat(msg, attachedFiles, {
                  deepResearch: deepResearchEnabled,
                  executionMode: chatExecutionMode
                });
              }
            }}
            rows={1}
            disabled={false}
          />
          <div className="chat-composer-actions">
            <Stack direction="row" spacing={0.5} alignItems="center" sx={{ mr: 0.5 }}>
              {([
                {
                  value: "auto",
                  label: "Auto",
                  tip: "Promote tool, file, app, import, research, or repeatable work into a task."
                },
                {
                  value: "chat",
                  label: "Ask",
                  tip: "Keep this as a chat-only run."
                },
                {
                  value: "task",
                  label: "Task",
                  tip: "Always create a durable task and stream the live work."
                }
              ] as Array<{ value: ChatExecutionMode; label: string; tip: string }>).map((mode) => (
                <Tooltip key={mode.value} title={mode.tip}>
                  <Chip
                    size="small"
                    clickable={!isStreaming}
                    disabled={isStreaming}
                    label={mode.label}
                    variant={chatExecutionMode === mode.value ? "filled" : "outlined"}
                    color={chatExecutionMode === mode.value ? "primary" : "default"}
                    onClick={() => {
                      if (!isStreaming) setChatExecutionMode(mode.value);
                    }}
                    sx={{ height: 24 }}
                  />
                </Tooltip>
              ))}
            </Stack>
            <Tooltip title={deepResearchEnabled ? "Deep Research enabled — slower, source-backed" : "Enable Deep Research"}>
              <FormControlLabel
                control={
                  <Switch
                    size="small"
                    checked={deepResearchEnabled}
                    onChange={() => setDeepResearchEnabled((prev) => !prev)}
                    disabled={isStreaming}
                    sx={{
                      "& .MuiSwitch-switchBase.Mui-checked": { color: "#2fd4ff" },
                      "& .MuiSwitch-switchBase.Mui-checked + .MuiSwitch-track": { backgroundColor: "rgba(47, 212, 255, 0.4)" },
                    }}
                  />
                }
                label="Research"
                slotProps={{ typography: { sx: { fontSize: "0.7rem", fontWeight: 600, color: deepResearchEnabled ? "#2fd4ff" : "text.secondary" } } }}
                sx={{ ml: 0, mr: 0.5 }}
              />
            </Tooltip>
            <Tooltip title="Attach files">
              <IconButton
                size="small"
                className="chat-composer-action-btn"
                onClick={() => fileInputRef.current?.click()}
                disabled={isStreaming}
              >
                <AttachFileRoundedIcon fontSize="small" />
              </IconButton>
            </Tooltip>
            {isStreaming ? (
              <IconButton
                size="small"
                className="chat-composer-stop-btn"
                onClick={() => {
                  // abort handled by parent streaming logic
                }}
              >
                <StopRoundedIcon fontSize="small" />
              </IconButton>
            ) : (
              <IconButton
                id="chat-send-btn"
                size="small"
                className="chat-composer-send-btn"
                disabled={!prompt.trim() && attachedFiles.length === 0}
                onClick={async () => {
                  setChatError(null);
                  const msg = prompt.trim();
                  setPrompt("");
                  const ta = document.querySelector(".chat-composer-textarea") as HTMLTextAreaElement | null;
                  if (ta) ta.style.height = "auto";
                  await runStreamingChat(msg, attachedFiles, {
                    deepResearch: deepResearchEnabled,
                    executionMode: chatExecutionMode
                  });
                }}
              >
                <ArrowUpwardRoundedIcon fontSize="small" />
              </IconButton>
            )}
          </div>
        </Box>
        </Box>
      </Box>

      {showWorkspacePanel ? (
        <Box
          className="list-shell chat-workspace-shell"
          sx={{ minHeight: 0, display: { xs: "none", lg: "flex" }, flexDirection: "column", p: 1 }}
        >
          <Box className="activity-status-bar">
            <span className={`activity-status-dot${safetyPolicyBlocked ? " error" : showStreamingAssistant ? " running" : " idle"}`} />
            <span className="activity-status-text">
              {showStreamingAssistant
                ? workspaceStatusCopy.line1 || (isStreamingForCurrentConversation ? "Processing..." : "Recovered latest progress")
                : workspaceCards.length > 0
                  ? workspaceStatusCopy.line1 || runStateLabel
                  : runStateLabel === "STOPPED"
                    ? "Waiting for activity"
                    : workspaceStatusCopy.line1 || runStateLabel}
            </span>
            <span className="activity-step-count">{workspaceCards.length} step{workspaceCards.length === 1 ? "" : "s"}</span>
          </Box>

          <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }} className="chat-workspace-sections">
              <Box className="term-shell">
                <Box className="term-titlebar">
                  <span className="term-tl-dot" style={{ background: "#ff5f57" }} />
                  <span className="term-tl-dot" style={{ background: "#febc2e" }} />
                  <span className="term-tl-dot" style={{ background: "#28c840" }} />
                  <Typography variant="caption" className="term-titlebar-text">
                    agentark://console
                  </Typography>
                  <Box sx={{ flex: 1 }} />
                  <Typography variant="caption" className="term-titlebar-stats">
                    {progressSummary}
                  </Typography>
                </Box>
                  <Box
                    className="term-body"
                    ref={workspaceActivityRef}
                    onScroll={() => {
                      const node = workspaceActivityRef.current;
                      if (!node) return;
                      const nearBottom = node.scrollHeight - node.scrollTop - node.clientHeight < 22;
                      if (nearBottom && !activityAutoFollow) setActivityAutoFollow(true);
                      if (!nearBottom && activityAutoFollow) setActivityAutoFollow(false);
                    }}
                  >
                    {workspaceCards.length === 0 ? (
                      <Box className="term-line">
                        <span className="term-prompt">&gt;</span>
                        <span className="term-text term-dim">awaiting_activity<span className="term-block-cursor" /></span>
                      </Box>
                    ) : (
                      workspaceCards.map((row, idx) => {
                        const isLast = idx === workspaceCards.length - 1;
                        const isActive = isLast && isStreamingForCurrentConversation;
                        const lineTone =
                          row.kind === "Issue"
                            ? "var(--danger, #ff7a7a)"
                            : row.kind === "Done"
                              ? "var(--success, #74f7bf)"
                              : row.kind === "Running"
                                ? "var(--accent-strong, #7fe7ff)"
                                : row.kind === "Planning"
                                  ? "var(--warning, #ffd36a)"
                                  : "rgba(191, 222, 255, 0.92)";
                        return (
                          <Box key={`activity-${row.id}`} className={`term-line${isActive ? " term-line-active" : ""}`}>
                            <span className="term-prompt" style={{ color: lineTone }}>•</span>
                            <Box className="term-content">
                              <span className={`term-label${isActive ? " term-typewriter" : ""}`} style={{ color: lineTone }}>
                                {row.time ? `[${formatTraceStepTime(row.time)}] ` : ""}{row.label}
                              </span>
                              {(row.detailFull || row.detail) ? (
                                <span className="term-detail">{row.detailFull || row.detail || ""}</span>
                              ) : null}
                            </Box>
                          </Box>
                        );
                      })
                    )}
                  </Box>
              </Box>

            {deployedFiles.length > 0 ? (
              <Accordion className="chat-workspace-section" defaultExpanded disableGutters>
                <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ minHeight: 34 }}>
                  <Typography variant="subtitle2">Files ({deployedFiles.length})</Typography>
                </AccordionSummary>
                <AccordionDetails sx={{ p: "4px 8px 8px" }}>
                  <Stack spacing={0.5}>
                    {deployedFiles.map((f, i) => (
                      <Box
                        key={f.name}
                        className="deployed-file-row"
                        onClick={() => { setCodeViewerFileIdx(i); setCodeViewerOpen(true); }}
                      >
                        <span className="deployed-file-icon">&#128196;</span>
                        <span className="deployed-file-name">{f.name}</span>
                        <span className="deployed-file-size">
                          {(() => {
                            const live = liveFileWrites[f.name];
                            if (!live) return `${(f.content.length / 1024).toFixed(1)}KB`;
                            if (live.totalLines > 0) {
                              return `${Math.min(live.line, live.totalLines)}/${live.totalLines} lines${live.done ? " done" : ""}`;
                            }
                            return live.done ? "written" : "writing...";
                          })()}
                        </span>
                      </Box>
                    ))}
                  </Stack>
                </AccordionDetails>
              </Accordion>
            ) : codeSnapshot ? (
              <Accordion className="chat-workspace-section" disableGutters>
                <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ minHeight: 34 }}>
                  <Typography variant="subtitle2">Code</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <pre className="chat-workspace-pre"><code>{codeSnapshot}</code></pre>
                </AccordionDetails>
              </Accordion>
            ) : null}

            {previewUrl && deployedFiles.length > 0 ? (
              <Accordion className="chat-workspace-section chat-workspace-section-preview" disableGutters>
                <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ minHeight: 34 }}>
                  <Typography variant="subtitle2">Preview</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <Box className="chat-workspace-preview">
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ display: "block", mb: 0.7 }}
                      noWrap
                      title={previewUrl}
                    >
                      Local:{" "}
                      <Link href={previewUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                        {previewUrl}
                      </Link>
                    </Typography>
                    {publicPreviewUrl ? (
                      <Typography
                        variant="caption"
                        color="info.main"
                        sx={{ display: "block", mb: 0.7 }}
                        noWrap
                        title={publicPreviewUrl}
                      >
                        Public:{" "}
                        <Link href={publicPreviewUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                        {publicPreviewUrl}
                      </Link>
                    </Typography>
                    ) : null}
                    <Stack direction="row" spacing={0.8} sx={{ mt: 0.7 }}>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={() => window.open(previewUrl, "_blank", "noopener,noreferrer")}
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
                      <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.7 }}>
                        Screenshot preview will appear after deployment validation captures it.
                      </Typography>
                    ) : null}
                  </Box>
                </AccordionDetails>
              </Accordion>
            ) : null}
          </Box>
        </Box>
      ) : null}

      {/* Code Viewer Dialog */}
      <Dialog
        open={codeViewerOpen}
        onClose={() => setCodeViewerOpen(false)}
        maxWidth="lg"
        fullWidth
        PaperProps={{ className: "code-viewer-dialog" }}
      >
        <DialogTitle sx={{ p: "10px 16px", borderBottom: "1px solid rgba(100,160,230,0.18)" }}>
          <Stack direction="row" justifyContent="space-between" alignItems="center">
            <Box>
              <Typography variant="subtitle1" sx={{ fontWeight: 600 }}>Generated Files</Typography>
              {activeCodeFile && codeViewerWriteStatus ? (
                <Typography variant="caption" color="text.secondary">
                  {activeCodeFile.name} • {codeViewerWriteStatus}
                </Typography>
              ) : null}
            </Box>
            <IconButton size="small" onClick={() => setCodeViewerOpen(false)}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Stack>
          {deployedFiles.length > 1 && (
            <Box className="code-file-tabs" sx={{ mt: 0.5 }}>
              {deployedFiles.map((f, i) => (
                <button
                  key={f.name}
                  className={`code-file-tab${i === codeViewerFileIdx ? " code-file-tab-active" : ""}`}
                  onClick={() => setCodeViewerFileIdx(i)}
                >
                  {f.name}
                </button>
              ))}
            </Box>
          )}
        </DialogTitle>
        <DialogContent sx={{ p: 0 }}>
          {activeCodeFile && (
            <>
              <pre className="code-viewer-pre">
                <code>{codeViewerContent}</code>
              </pre>
              {activeLiveWrite && !activeLiveWrite.done ? (
                <Box sx={{ px: 1.5, pb: 1 }}>
                  <Typography variant="caption" color="text.secondary">
                    Writing in progress...
                  </Typography>
                </Box>
              ) : null}
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
        <DialogTitle sx={{ p: "10px 16px", borderBottom: "1px solid rgba(100,160,230,0.18)" }}>
          Deployment Preview
        </DialogTitle>
        <DialogContent sx={{ p: 2 }}>
          <Stack spacing={1}>
            <Typography variant="body2">
              Local:{" "}
              <Link href={previewUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                {previewUrl}
              </Link>
            </Typography>
            {publicPreviewUrl ? (
              <Typography variant="body2">
                Public:{" "}
                <Link href={publicPreviewUrl} target="_blank" rel="noopener noreferrer" underline="hover">
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
                  border: "1px solid rgba(108, 156, 212, 0.2)",
                  background: "rgba(8, 16, 30, 0.7)"
                }}
              />
            ) : (
              <Alert severity="info">No screenshot is available yet for this deployment.</Alert>
            )}
          </Stack>
        </DialogContent>
      </Dialog>
    </Box>
  );
}
function TasksManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [createTaskOpen, setCreateTaskOpen] = useState(false);
  const [quickIntent, setQuickIntent] = useState("");
  const [schedulePreset, setSchedulePreset] = useState("once");
  const [customCron, setCustomCron] = useState("");
  const [requireApproval, setRequireApproval] = useState(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [description, setDescription] = useState("");
  const [action, setAction] = useState("daily_brief");
  const [argumentsJson, setArgumentsJson] = useState("{}");
  const [cron, setCron] = useState("");
  const [approval, setApproval] = useState("auto");
  const [formError, setFormError] = useState<string | null>(null);
  const [selectedTask, setSelectedTask] = useState<JsonRecord | null>(null);

  function resetTaskCreateForm(): void {
    setQuickIntent("");
    setSchedulePreset("once");
    setCustomCron("");
    setRequireApproval(false);
    setManualOpen(false);
    setDescription("");
    setAction("daily_brief");
    setArgumentsJson("{}");
    setCron("");
    setApproval("auto");
    setFormError(null);
  }

  function closeCreateTaskDialog(): void {
    setCreateTaskOpen(false);
    setFormError(null);
  }

  function statusLabel(raw: string): string {
    const s = (raw || "").toLowerCase();
    if (s.includes("awaitingapproval")) return "Needs approval";
    if (s.includes("paused")) return "Paused";
    if (s.includes("inprogress")) return "Running";
    if (s.includes("pending")) return "Queued";
    if (s.includes("completed")) return "Done";
    if (s.includes("failed")) return "Failed";
    if (s.includes("cancelled")) return "Cancelled";
    return raw || "-";
  }

  function statusColor(raw: string): "success" | "warning" | "error" | "default" | "info" {
    const s = (raw || "").toLowerCase();
    if (s.includes("failed")) return "error";
    if (s.includes("awaitingapproval")) return "warning";
    if (s.includes("paused")) return "warning";
    if (s.includes("inprogress")) return "info";
    if (s.includes("pending")) return "default";
    if (s.includes("completed")) return "success";
    return "default";
  }

  const tasksQ = useQuery({
    queryKey: ["tasks-manager"],
    queryFn: () => api.rawGet("/tasks?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const opMutation = useMutation({
    mutationFn: ({
      path,
      method,
      payload
    }: {
      path: string;
      method: "POST" | "DELETE";
      payload?: unknown;
    }) => (method === "DELETE" ? api.rawDelete(path) : api.rawPost(path, payload ?? {})),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });

  const aiCreateMutation = useMutation({
    mutationFn: async () => {
      const intent = quickIntent.trim();
      if (!intent) throw new Error("Describe what you want to automate.");

      const planRaw = await api.rawPost("/tasks/plan", { description: intent });
      const plan = asRecord(asRecord(planRaw).plan);
      const rawSteps = Array.isArray(plan.steps) ? plan.steps : [];
      const steps = rawSteps
        .map(asRecord)
        .map((step) => ({
          action: str(step.action, "").trim(),
          arguments: asRecord(step.arguments)
        }))
        .filter((step) => !!step.action);

      if (steps.length === 0) {
        throw new Error("AI planner returned no runnable steps. Try a more specific request.");
      }

      let cronValue: string | null = null;
      if (schedulePreset === "every_15") cronValue = "*/15 * * * *";
      else if (schedulePreset === "hourly") cronValue = "0 * * * *";
      else if (schedulePreset === "daily_9") cronValue = "0 9 * * *";
      else if (schedulePreset === "weekday_9") cronValue = "0 9 * * 1-5";
      else if (schedulePreset === "custom") cronValue = customCron.trim() || null;

      const summary = str(plan.summary, "").trim();
      await opMutation.mutateAsync({
        path: "/tasks",
        method: "POST",
        payload: {
          description: summary || intent,
          action: "plan",
          arguments: { steps },
          cron: cronValue,
          approval: requireApproval ? "require" : "auto"
        }
      });
    },
    onSuccess: async () => {
      resetTaskCreateForm();
      setCreateTaskOpen(false);
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });

  const tasks = pickRecords(tasksQ.data, "tasks");
  const counts = useMemo(() => {
    const by = { total: tasks.length, queued: 0, running: 0, needs_approval: 0, paused: 0, done: 0 };
    for (const t of tasks) {
      const s = str(t.status, "").toLowerCase();
      if (s.includes("awaitingapproval")) by.needs_approval += 1;
      else if (s.includes("paused")) by.paused += 1;
      else if (s.includes("inprogress")) by.running += 1;
      else if (s.includes("pending")) by.queued += 1;
      else if (s.includes("completed")) by.done += 1;
    }
    return by;
  }, [tasks]);

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack
          direction={{ xs: "column", sm: "row" }}
          spacing={1.5}
          alignItems={{ xs: "flex-start", sm: "center" }}
          justifyContent="space-between"
        >
          <Box>
            <Typography variant="h6">Tasks</Typography>
            <Typography variant="body2" color="text.secondary">
              Describe what you want in plain English. AgentArk can generate a runnable task for you.
            </Typography>
          </Box>
          <Button variant="contained" onClick={() => {
            setFormError(null);
            setCreateTaskOpen(true);
          }}>
            Create Task
          </Button>
        </Stack>
      </Box>

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, sm: 6, md: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 120 }}>
            <Typography variant="caption" color="text.secondary">Total</Typography>
            <Typography variant="h5">{counts.total}</Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, md: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 120 }}>
            <Typography variant="caption" color="text.secondary">Queued</Typography>
            <Typography variant="h5">{counts.queued}</Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, md: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 120 }}>
            <Typography variant="caption" color="text.secondary">Needs Approval</Typography>
            <Typography variant="h5">{counts.needs_approval}</Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, md: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 120 }}>
            <Typography variant="caption" color="text.secondary">Done</Typography>
            <Typography variant="h5">{counts.done}</Typography>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, sm: 6, md: 3 }}>
          <Box className="list-shell" sx={{ minHeight: 120 }}>
            <Typography variant="caption" color="text.secondary">Paused</Typography>
            <Typography variant="h5">{counts.paused}</Typography>
          </Box>
        </Grid2>
      </Grid2>

      <Box className="list-shell">
        <Typography variant="h6" mb={1}>
          Task List
        </Typography>
        <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
          <Table size="small" sx={{ minWidth: 860 }}>
            <TableHead>
              <TableRow>
                <TableCell>Description</TableCell>
                <TableCell>Action</TableCell>
                <TableCell>Status</TableCell>
                <TableCell>Schedule</TableCell>
                <TableCell>Created</TableCell>
                <TableCell align="right">Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {tasks.map((task) => {
                const id = str(task.id, "");
                const cronExpr = str(task.cron, "");
                const schedule = cronExpr ? `cron: ${cronExpr}` : "manual";
                const rawStatus = str(task.status, "-");
                return (
                  <TableRow key={id}>
                    <TableCell sx={{ maxWidth: 520 }}>
                      <Typography variant="body2" noWrap title={str(task.description)}>
                        {str(task.description)}
                      </Typography>
                    </TableCell>
                    <TableCell sx={{ maxWidth: 220 }}>
                      <Typography variant="body2" noWrap title={str(task.action)}>
                        {str(task.action)}
                      </Typography>
                    </TableCell>
                    <TableCell>
                      <Chip size="small" label={statusLabel(rawStatus)} color={statusColor(rawStatus)} />
                    </TableCell>
                    <TableCell sx={{ maxWidth: 220 }}>
                      <Typography variant="body2" noWrap title={schedule}>
                        {schedule}
                      </Typography>
                    </TableCell>
                    <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(task.created_at)).tip}>{humanTs(str(task.created_at)).label}</TableCell>
                    <TableCell align="right">
                      <RowOpsMenu
                        actions={[
                          {
                            label: "View",
                            onClick: () => setSelectedTask(asRecord(task))
                          },
                          {
                            label: "Approve",
                            disabled: !rawStatus.toLowerCase().includes("awaitingapproval"),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/approve`, method: "POST" })
                          },
                          {
                            label: "Reject",
                            tone: "warning",
                            disabled: !rawStatus.toLowerCase().includes("awaitingapproval"),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/reject`, method: "POST" })
                          },
                          {
                            label: "Pause",
                            disabled: !["pending", "awaitingapproval"].some((token) => rawStatus.toLowerCase().includes(token)),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/pause`, method: "POST" })
                          },
                          {
                            label: "Resume",
                            disabled: !rawStatus.toLowerCase().includes("paused"),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/resume`, method: "POST" })
                          },
                          {
                            label: "Cancel",
                            tone: "warning",
                            disabled: !["pending", "awaitingapproval", "paused", "inprogress"].some((token) => rawStatus.toLowerCase().includes(token)),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/cancel`, method: "POST" })
                          },
                          {
                            label: "Retry",
                            disabled: !["failed", "cancelled"].some((token) => rawStatus.toLowerCase().includes(token)),
                            onClick: () => opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}/retry`, method: "POST" })
                          },
                          {
                            label: "Delete",
                            tone: "error",
                            divider: true,
                            onClick: async () => {
                              const ok = window.confirm("Delete this task? This cannot be undone.");
                              if (!ok) return;
                              opMutation.mutate({ path: `/tasks/${encodeURIComponent(id)}`, method: "DELETE" });
                            }
                          }
                        ]}
                        ariaLabel="Task options"
                      />
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>

      <Dialog open={selectedTask != null} onClose={() => setSelectedTask(null)} maxWidth="md" fullWidth>
        <DialogTitle>{str(selectedTask?.description, "Task")}</DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Stack direction="row" spacing={1} flexWrap="wrap" alignItems="center">
              <Chip
                size="small"
                label={statusLabel(str(selectedTask?.status, ""))}
                color={statusColor(str(selectedTask?.status, ""))}
              />
              <Chip size="small" variant="outlined" label={str(selectedTask?.cron, "") ? "Scheduled" : "Manual"} />
              <Chip size="small" variant="outlined" label={`Action: ${str(selectedTask?.action, "-")}`} />
            </Stack>

            <Typography variant="caption" color="text.secondary">
              Created: <span title={humanTs(str(selectedTask?.created_at, "-")).tip}>{humanTs(str(selectedTask?.created_at, "-")).label}</span>
            </Typography>

            {str(selectedTask?.cron, "") ? (
              <Box className="metadata-box">
                <Typography variant="caption" color="text.secondary">
                  Schedule
                </Typography>
                <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                  {str(selectedTask?.cron)}
                </Typography>
              </Box>
            ) : null}

            {str(selectedTask?.result, "") ? (
              <Box className="metadata-box">
                <Typography variant="caption" color="text.secondary">
                  Last Result
                </Typography>
                <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                  {str(selectedTask?.result)}
                </Typography>
              </Box>
            ) : (
              <Typography variant="body2" color="text.secondary">
                No result yet.
              </Typography>
            )}

            <KeyValuePanel title="Arguments" data={asRecord(selectedTask?.arguments)} emptyLabel="No arguments." maxRows={18} />
            <KeyValuePanel title="System fields" data={asRecord(selectedTask)} emptyLabel="No extra fields." maxRows={10} />
          </Stack>
        </DialogContent>
      </Dialog>

      <Dialog open={createTaskOpen} onClose={closeCreateTaskDialog} maxWidth="md" fullWidth>
        <DialogTitle>Create Task</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 0.5 }}>
            <Box className="list-shell">
              <Typography variant="h6" mb={1}>
                Create Task (Easy)
              </Typography>
              <Grid2 container spacing={1}>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    multiline
                    minRows={2}
                    label="What should AgentArk do?"
                    placeholder="Example: Every weekday at 9am send me a daily brief in Telegram."
                    value={quickIntent}
                    onChange={(e) => setQuickIntent(e.target.value)}
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 4 }}>
                  <TextField
                    fullWidth
                    size="small"
                    select
                    label="When"
                    value={schedulePreset}
                    onChange={(e) => setSchedulePreset(e.target.value)}
                  >
                    <MenuItem value="once">One-time (run once)</MenuItem>
                    <MenuItem value="every_15">Every 15 minutes</MenuItem>
                    <MenuItem value="hourly">Hourly</MenuItem>
                    <MenuItem value="daily_9">Daily at 9:00</MenuItem>
                    <MenuItem value="weekday_9">Weekdays at 9:00</MenuItem>
                    <MenuItem value="custom">Custom cron</MenuItem>
                  </TextField>
                </Grid2>
                {schedulePreset === "custom" ? (
                  <Grid2 size={{ xs: 12, md: 8 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Custom cron"
                      placeholder="*/10 * * * *"
                      value={customCron}
                      onChange={(e) => setCustomCron(e.target.value)}
                      helperText="Use 5 fields (min hour day month weekday)."
                    />
                  </Grid2>
                ) : null}
                <Grid2 size={{ xs: 12 }}>
                  <FormControlLabel
                    control={<Switch checked={requireApproval} onChange={(e) => setRequireApproval(e.target.checked)} />}
                    label="Require approval before execution"
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <Button
                    variant="contained"
                    disabled={aiCreateMutation.isPending || opMutation.isPending || !quickIntent.trim()}
                    onClick={async () => {
                      setFormError(null);
                      try {
                        await aiCreateMutation.mutateAsync();
                      } catch (e) {
                        const msg = errMessage(e);
                        if (msg.toLowerCase().includes("llm planning failed")) {
                          setFormError("AI planner needs an active LLM model. Configure one in Settings > Models, or use Manual mode below.");
                        } else {
                          setFormError(msg);
                        }
                      }
                    }}
                  >
                    {aiCreateMutation.isPending ? "Creating..." : "Create with AI"}
                  </Button>
                </Grid2>
              </Grid2>
              {formError ? <Alert severity="error" sx={{ mt: 1 }}>{formError}</Alert> : null}
            </Box>

            <Accordion expanded={manualOpen} onChange={() => setManualOpen((p) => !p)} className="accordion-shell">
              <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>Manual Mode (Optional)</Typography>
              </AccordionSummary>
              <AccordionDetails>
                <Grid2 container spacing={1}>
                  <Grid2 size={{ xs: 12, md: 4 }}>
                    <TextField fullWidth size="small" label="Description" value={description} onChange={(e) => setDescription(e.target.value)} />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 2 }}>
                    <TextField fullWidth size="small" label="Action" value={action} onChange={(e) => setAction(e.target.value)} />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 3 }}>
                    <TextField fullWidth size="small" label="Cron" value={cron} onChange={(e) => setCron(e.target.value)} placeholder="*/10 * * * *" />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 3 }}>
                    <TextField fullWidth size="small" select label="Approval" value={approval} onChange={(e) => setApproval(e.target.value)}>
                      <MenuItem value="auto">auto</MenuItem>
                      <MenuItem value="require">require</MenuItem>
                    </TextField>
                  </Grid2>
                  <Grid2 size={{ xs: 12 }}>
                    <TextField fullWidth multiline minRows={2} label="Arguments JSON" value={argumentsJson} onChange={(e) => setArgumentsJson(e.target.value)} />
                  </Grid2>
                  <Grid2 size={{ xs: 12 }}>
                    <Button
                      variant="outlined"
                      disabled={opMutation.isPending || !description.trim()}
                      onClick={async () => {
                        setFormError(null);
                        try {
                          const parsed = JSON.parse(argumentsJson || "{}");
                          await opMutation.mutateAsync({
                            path: "/tasks",
                            method: "POST",
                            payload: { description: description.trim(), action: action.trim(), arguments: parsed, cron: cron.trim() || null, approval }
                          });
                          resetTaskCreateForm();
                          setCreateTaskOpen(false);
                        } catch (e) {
                          setFormError(errMessage(e));
                        }
                      }}
                    >
                      Add Manual Task
                    </Button>
                  </Grid2>
                </Grid2>
              </AccordionDetails>
            </Accordion>
          </Stack>
        </DialogContent>
      </Dialog>
    </Stack>
  );
}

function SkillsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [lastImport, setLastImport] = useState<SkillImportSummary | null>(null);
  const [testResults, setTestResults] = useState<Record<string, string>>({});
  const [skillMenuAnchor, setSkillMenuAnchor] = useState<{ el: HTMLElement; name: string } | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editTargetName, setEditTargetName] = useState<string | null>(null);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(getDeveloperModeEnabled);
  const [editForm, setEditForm] = useState<SkillEditorForm>(defaultSkillEditorForm());
  const [editContent, setEditContent] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const [editLoading, setEditLoading] = useState(false);
  const [createWizardEnabled, setCreateWizardEnabled] = useState(true);
  const [createWizardStep, setCreateWizardStep] = useState(0);
  const [editAttachHook, setEditAttachHook] = useState(false);
  const [editHookInstruction, setEditHookInstruction] = useState("");
  const [editHookTrigger, setEditHookTrigger] = useState<HookTriggerValue>("on_error");
  const [editHookUrl, setEditHookUrl] = useState("");
  const [editAttachTask, setEditAttachTask] = useState(false);
  const [editTaskInstruction, setEditTaskInstruction] = useState("");
  const [editTaskCron, setEditTaskCron] = useState("");
  const [aiCreateOpen, setAiCreateOpen] = useState(false);
  const [aiPrompt, setAiPrompt] = useState("");
  const [aiNameHint, setAiNameHint] = useState("");
  const [aiError, setAiError] = useState<string | null>(null);
  const [skillsTab, setSkillsTab] = useState<"manage" | "system">("manage");
  const [skillSearch, setSkillSearch] = useState("");
  const [skillSort, setSkillSort] = useState<"name" | "imported">("name");
  const [secretsName, setSecretsName] = useState<string | null>(null);
  const [hooksOpen, setHooksOpen] = useState(false);
  const [hooksTargetAction, setHooksTargetAction] = useState<string | null>(null);
  const [hookInstruction, setHookInstruction] = useState("");
  const [hookName, setHookName] = useState("");
  const [hookTrigger, setHookTrigger] = useState<HookTriggerValue>("post_action");
  const [hookUrl, setHookUrl] = useState("");
  const [hookError, setHookError] = useState<string | null>(null);
  const editRawMode = developerModeEnabled;

  useEffect(() => {
    const refreshDeveloperMode = () => setDeveloperModeEnabledState(getDeveloperModeEnabled());
    window.addEventListener(DEVELOPER_MODE_EVENT, refreshDeveloperMode as EventListener);
    window.addEventListener("storage", refreshDeveloperMode);
    return () => {
      window.removeEventListener(DEVELOPER_MODE_EVENT, refreshDeveloperMode as EventListener);
      window.removeEventListener("storage", refreshDeveloperMode);
    };
  }, []);

  const skillsQ = useQuery({
    queryKey: ["skills-manager"],
    queryFn: () => api.rawGet("/skills"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const hooksQ = useQuery({
    queryKey: ["skills-hooks"],
    queryFn: () => api.rawGet("/hooks"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const hookRunsQ = useQuery({
    queryKey: ["skills-hook-runs"],
    queryFn: () => api.rawGet("/hooks/runs?limit=200"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const handleImported = async (summary: SkillImportSummary) => {
    setLastImport(summary);
    await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
  };
  const afterImport = async () => {
    await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
  };

  const setEnabledMutation = useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) => api.setSkillEnabled(name, enabled),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
    }
  });

  const testMutation = useMutation({
    mutationFn: ({ name }: { name: string }) => api.testSkill(name),
    onMutate: ({ name }) => {
      setTestResults((prev) => ({ ...prev, [name]: "Running..." }));
    },
    onSuccess: (out, { name }) => {
      const outputPreview = str(out.output, "").replace(/\s+/g, " ").trim();
      const outputSuffix =
        outputPreview.length > 0
          ? `: ${outputPreview.slice(0, 180)}${outputPreview.length > 180 ? "..." : ""}`
          : "";
      const status =
        out.status === "needs_input"
          ? out.message || "Needs required input."
          : out.status === "ok"
            ? out.mode === "workflow"
              ? `Workflow test completed${outputSuffix}`
              : `Skill test completed${outputSuffix}`
            : out.error || out.message || "Test returned";
      setTestResults((prev) => ({ ...prev, [name]: status }));
    },
    onError: (err, { name }) => {
      setTestResults((prev) => ({ ...prev, [name]: errMessage(err) }));
    }
  });

  const deleteSkillMutation = useMutation({
    mutationFn: (name: string) => api.deleteSkill(name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
    }
  });

  const addHookMutation = useMutation({
    mutationFn: (payload: {
      name: string;
      trigger: HookTriggerValue;
      hook_type: string;
      url?: string;
      action_name?: string;
    }) => api.rawPost("/hooks", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-hooks"] });
      await queryClient.invalidateQueries({ queryKey: ["skills-hook-runs"] });
    }
  });

  const removeHookMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/hooks/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-hooks"] });
      await queryClient.invalidateQueries({ queryKey: ["skills-hook-runs"] });
    }
  });

  const skills = pickRecords(skillsQ.data, "skills");
  const hooks = asRecords(hooksQ.data);
  const hookRuns = asRecords(hookRunsQ.data);
  const hooksForSelectedAction = hooksTargetAction
    ? hooks.filter((h) => isHookRecordAttachedToAction(h, hooksTargetAction))
    : hooks;
  const skillSearchFilter = (a: JsonRecord) => {
    if (!skillSearch.trim()) return true;
    const q = skillSearch.toLowerCase();
    return str(a.name, "").toLowerCase().includes(q) || str(a.description, "").toLowerCase().includes(q);
  };
  const skillSortFn = (a: JsonRecord, b: JsonRecord) => {
    if (skillSort === "imported") {
      const ta = str(a.imported_at, "");
      const tb = str(b.imported_at, "");
      if (tb && ta) return tb.localeCompare(ta);
      if (tb) return 1;
      if (ta) return -1;
      return str(a.name, "").localeCompare(str(b.name, ""));
    }
    return str(a.name, "").localeCompare(str(b.name, ""));
  };
  const systemSkills = skills.filter((a) => str(a.source).toLowerCase() === "system").filter(skillSearchFilter).sort(skillSortFn);
  const bundledSkills = skills.filter((a) => str(a.source).toLowerCase() === "bundled").filter(skillSearchFilter).sort(skillSortFn);
  const customSkills = skills.filter((a) => str(a.source).toLowerCase() === "custom").filter(skillSearchFilter).sort(skillSortFn);
  const availableToolNames = dedupeStrings(systemSkills.map((a) => str(a.name, "").trim()).filter(Boolean));
  const allSkillNames = dedupeStrings(skills.map((a) => str(a.name, "").trim()).filter(Boolean));
  const hookLastRunById = useMemo(() => {
    const map: Record<string, JsonRecord> = {};
    for (const run of hookRuns) {
      const id = str(run.hook_id, "");
      if (!id || map[id]) continue;
      map[id] = run;
    }
    return map;
  }, [hookRuns]);

  const closeEditor = () => {
    setEditOpen(false);
    setEditTargetName(null);
    setEditForm(defaultSkillEditorForm());
    setEditContent("");
    setEditError(null);
    setEditLoading(false);
    setCreateWizardEnabled(true);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
  };

  const closeHooksDialog = () => {
    setHooksOpen(false);
    setHooksTargetAction(null);
    setHookInstruction("");
    setHookName("");
    setHookTrigger("post_action");
    setHookUrl("");
    setHookError(null);
  };

  const openHooksDialog = (actionName?: string) => {
    const target = actionName?.trim() || null;
    const baseName = target ? "hook" : "custom-hook";
    setHooksTargetAction(target);
    setHookInstruction(target ? `notify me when ${target} fails` : "");
    setHookTrigger(target ? "on_error" : "post_action");
    setHookName(baseName);
    setHookUrl("");
    setHookError(null);
    setHooksOpen(true);
  };

  const applyHookInstruction = () => {
    const trigger = inferHookTriggerFromInstruction(hookInstruction, hooksTargetAction ? "on_error" : "post_action");
    const extractedUrl = extractFirstUrl(hookInstruction);
    const actionPart = hooksTargetAction ? "" : "custom-";
    const triggerPart = trigger.replace(/_/g, "-");
    const suggestedName = sanitizeHookName(`${actionPart}${triggerPart}`) || "custom-hook";
    setHookTrigger(trigger);
    if (!hookName.trim()) setHookName(suggestedName);
    if (extractedUrl && !hookUrl.trim()) setHookUrl(extractedUrl);
  };

  const applyEditHookInstruction = () => {
    const trigger = inferHookTriggerFromInstruction(editHookInstruction, "on_error");
    const extractedUrl = extractFirstUrl(editHookInstruction);
    setEditHookTrigger(trigger);
    if (extractedUrl && !editHookUrl.trim()) setEditHookUrl(extractedUrl);
  };

  const applyEditTaskInstruction = () => {
    const inferredCron = inferTaskCronFromInstruction(editTaskInstruction);
    if (inferredCron && !editTaskCron.trim()) setEditTaskCron(inferredCron);
  };

  const saveHookFromDialog = async () => {
    setHookError(null);
    try {
      const effectiveTrigger = inferHookTriggerFromInstruction(hookInstruction, hookTrigger);
      const effectiveUrl = hookUrl.trim() || extractFirstUrl(hookInstruction);
      if (!effectiveUrl) {
        setHookError("Send update URL is required.");
        return;
      }
      const rawName = sanitizeHookName(hookName) || "hook";
      const finalName = hooksTargetAction
        ? (isHookAttachedToAction(rawName, hooksTargetAction)
            ? rawName
            : sanitizeHookName(`action-${hooksTargetAction}-${rawName}`))
        : rawName;
      await addHookMutation.mutateAsync({
        name: finalName,
        trigger: effectiveTrigger,
        hook_type: "webhook",
        url: effectiveUrl,
        action_name: hooksTargetAction || undefined
      });
      closeHooksDialog();
    } catch (err) {
      setHookError(errMessage(err));
    }
  };

  const openEditor = async (name: string) => {
    setEditError(null);
    setEditLoading(true);
    setEditTargetName(name);
    setEditForm(defaultSkillEditorForm(name));
    setEditContent("");
    setEditOpen(true);
    setCreateWizardEnabled(false);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
    try {
      const loadDetails = async () => {
        const out = (await api.rawGet(`/skills/${encodeURIComponent(name)}`)) as JsonRecord;
        const content = str(out.content, "");
        const parsed = parseSkillEditorForm(content, name);
        setEditContent(content);
        setEditForm({ ...parsed, name });
      };

      try {
        await loadDetails();
      } catch (err) {
        const message = errMessage(err);
        if (message.toLowerCase().includes("rate limit")) {
          await new Promise((resolve) => setTimeout(resolve, 1200));
          await loadDetails();
        } else {
          throw err;
        }
      }
    } catch (err) {
      const message = errMessage(err);
      if (message.toLowerCase().includes("rate limit")) {
        setEditError("Skill details are temporarily rate-limited. Wait a few seconds and reopen the editor.");
      } else {
        setEditError(message);
      }
    } finally {
      setEditLoading(false);
    }
  };

  const openNewEditor = (initial?: { name?: string; content?: string }) => {
    const initialName = normalizeActionName(initial?.name || "new-action") || "new-action";
    const initialContent = (initial?.content || "").trim();
    const parsed = initialContent
      ? parseSkillEditorForm(initialContent, initialName)
      : defaultSkillEditorForm(initialName);
    const normalizedName = normalizeActionName(parsed.name || initialName) || "new-action";
    const form = { ...parsed, name: normalizedName };
    const content = initialContent || buildSkillMdFromForm("", form);
    setEditTargetName(null);
    setEditForm(form);
    setEditContent(content);
    setEditError(null);
    setCreateWizardEnabled(true);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
    setEditOpen(true);
  };

  const aiGenerateMutation = useMutation({
    mutationFn: async ({ prompt, nameHint }: { prompt: string; nameHint: string }) => {
      const fallbackName = normalizeActionName(nameHint || "new-action") || "new-action";
      const toolsText = availableToolNames.length > 0 ? availableToolNames.join(", ") : "web_search";
      const existingText = allSkillNames.length > 0 ? allSkillNames.join(", ") : "(none)";
      const generationPrompt = [
        "Create a complete SKILL.md for AgentArk.",
        "",
        "Return ONLY the SKILL.md content. No explanation, no markdown fences.",
        "The file must use YAML frontmatter exactly with keys: name, description, version, required_inputs, metadata.emoji, requires.tools.",
        "Use version \"1.0.0\".",
        "Skill name must be lowercase letters, numbers, and hyphens only.",
        `Name hint: ${fallbackName}`,
        `Available tool skills to reference in workflow guidance: ${toolsText}`,
        `Existing skill names (avoid collisions): ${existingText}`,
        "",
        "Task request:",
        prompt.trim()
      ].join("\n");

      const out = (await api.chat({ message: generationPrompt, channel: "web" })) as JsonRecord;
      const raw = str(out.response, "");
      const actionMd = extractActionMdFromModelOutput(raw);
      if (!actionMd.trim()) throw new Error("AI did not return skill content.");
      return { actionMd, fallbackName };
    },
    onSuccess: ({ actionMd, fallbackName }) => {
      const parsed = parseSkillEditorForm(actionMd, fallbackName);
      const normalizedName = normalizeActionName(parsed.name || fallbackName) || "new-action";
      const normalizedForm = { ...parsed, name: normalizedName };
      const normalizedContent = buildSkillMdFromForm(actionMd, normalizedForm);
      setAiError(null);
      setAiCreateOpen(false);
      setAiPrompt("");
      setAiNameHint("");
      openNewEditor({ name: normalizedName, content: normalizedContent });
    },
    onError: (err) => {
      setAiError(errMessage(err));
    }
  });

  const saveEditor = async () => {
    setEditError(null);
    try {
      const createMode = !editTargetName;
      let targetName = editTargetName || normalizeActionName(editForm.name);
      if (createMode && editRawMode) {
        const parsed = parseSkillEditorForm(editContent, targetName || "new-action");
        const parsedName = normalizeActionName(parsed.name);
        if (parsedName) targetName = parsedName;
      }
      if (!targetName) targetName = "new-action";

      if (createMode && !isValidActionName(targetName)) {
        setEditError("Skill name must use lowercase letters, numbers, and hyphens only.");
        return;
      }

      const formForSave: SkillEditorForm = {
        ...editForm,
        name: targetName,
        version: (editForm.version || "").trim() || "1.0.0"
      };
      const finalContent = editRawMode ? editContent : buildSkillMdFromForm(editContent, formForSave);

      if (createMode) {
        const out = (await api.rawPost("/skills", { name: targetName, content: finalContent, force: false })) as JsonRecord;
        const status = str(out.status, "ok").toLowerCase();
        if (status === "blocked") {
          setEditError(str(out.error, str(out.message, "Skill was blocked by security verification.")));
          return;
        }
      } else {
        await api.rawPost(`/skills/${encodeURIComponent(targetName)}`, { content: finalContent });
      }

      const editEffectiveUrl = editHookUrl.trim() || extractFirstUrl(editHookInstruction);
      if (editAttachHook && editEffectiveUrl) {
        const hookBase = sanitizeHookName(inferHookTriggerFromInstruction(editHookInstruction, editHookTrigger).replace(/_/g, "-")) || "hook";
        const hookName = sanitizeHookName(`action-${targetName}-${hookBase}`) || `action-${targetName}-hook`;
        await addHookMutation.mutateAsync({
          name: hookName,
          trigger: inferHookTriggerFromInstruction(editHookInstruction, editHookTrigger),
          hook_type: "webhook",
          url: editEffectiveUrl,
          action_name: targetName
        });
      }

      if (editAttachTask) {
        const inferredCron = inferTaskCronFromInstruction(editTaskInstruction);
        const effectiveCron = editTaskCron.trim() || inferredCron;
        const runOnce = isRunOnceInstruction(editTaskInstruction);
        if (!effectiveCron && !runOnce) {
          setEditError("Could not understand schedule. Try: every day at 9am, hourly, weekdays, or paste a cron.");
          return;
        }
        await api.rawPost("/tasks", {
          description: `Run skill '${targetName}' automatically`,
          action: targetName,
          arguments: {},
          cron: runOnce ? null : effectiveCron,
          approval: "auto"
        });
      }

      closeEditor();
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    } catch (err) {
      setEditError(errMessage(err));
    }
  };

  const toggleEnabled = async (name: string, nextEnabled: boolean) => {
    if (nextEnabled) {
      try {
        const secrets = await api.getSkillSecrets(name);
        if ((secrets.missing_env || []).length > 0) {
          setLastImport({
            result: { status: "needs_secrets", name, message: "Missing secrets", secrets: { missing_env: secrets.missing_env, required_env: secrets.required_env, bindings: secrets.bindings } },
            message: `Cannot enable '${name}' until secrets are configured: ${secrets.missing_env.join(", ")}`
          });
          setSecretsName(name);
          return;
        }
      } catch (err) {
        setLastImport({
          result: { status: "error", name, message: "Secrets check failed" },
          message: `Cannot enable '${name}': ${errMessage(err)}`
        });
        return;
      }
    }
    await setEnabledMutation.mutateAsync({ name, enabled: nextEnabled });
  };

  const renderActionRow = (action: JsonRecord, type: "system" | "bundled" | "custom") => {
    const name = str(action.name, "Untitled");
    const description = str(action.description, "No description");
    const version = str(action.version, "?");
    const enabled = toBool(action.enabled);
    const testMessage = testResults[name];
    const isTesting = testMutation.isPending && testMutation.variables?.name === name;
    const isSystem = type === "system";

    const menuOpen = skillMenuAnchor?.name === name;

    return (
      <Box
        key={`${type}-${name}`}
        className="action-row"
        sx={{
          width: "100%",
          opacity: isSystem ? 0.7 : 1,
          filter: isSystem ? "saturate(0.85)" : "none"
        }}
      >
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={2}>
          <Stack spacing={0.5} sx={{ flex: 1, minWidth: 0 }}>
            <Stack direction="row" alignItems="center" spacing={1}>
              <Typography variant="subtitle1" fontWeight={600} noWrap>
                {name}
              </Typography>
              {!enabled && !isSystem ? (
                <Chip label="Disabled" size="small" color="warning" variant="outlined" sx={{ height: 20, fontSize: "0.65rem" }} />
              ) : null}
            </Stack>
            <Typography variant="caption" color="text.secondary" noWrap>
              {description}
            </Typography>
            {testMessage ? (
              <Typography variant="caption" color="text.secondary">
                {testMessage}
              </Typography>
            ) : null}
          </Stack>
          <Stack direction="row" spacing={0.5} alignItems="center">
            <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "nowrap" }}>
              v{version}
            </Typography>
            {!isSystem ? (
              <>
                <IconButton
                  size="small"
                  onClick={(e: MouseEvent<HTMLButtonElement>) => setSkillMenuAnchor({ el: e.currentTarget, name })}
                >
                  <MoreVertIcon fontSize="small" />
                </IconButton>
                <Menu
                  anchorEl={menuOpen ? skillMenuAnchor.el : null}
                  open={menuOpen}
                  onClose={() => setSkillMenuAnchor(null)}
                  slotProps={{ paper: { sx: { minWidth: 160 } } }}
                >
                  <MenuItem onClick={() => { setSkillMenuAnchor(null); openEditor(name); }}>
                    Edit
                  </MenuItem>
                  <MenuItem onClick={() => { setSkillMenuAnchor(null); setSecretsName(name); }}>
                    Secrets
                  </MenuItem>
                  <MenuItem
                    disabled={isTesting || !enabled}
                    onClick={() => { setSkillMenuAnchor(null); testMutation.mutate({ name }); }}
                  >
                    {isTesting ? "Testing..." : "Run test"}
                  </MenuItem>
                  <MenuItem
                    disabled={setEnabledMutation.isPending}
                    onClick={() => { setSkillMenuAnchor(null); toggleEnabled(name, !enabled); }}
                  >
                    {enabled ? "Disable" : "Enable"}
                  </MenuItem>
                  {developerModeEnabled ? (
                    <MenuItem onClick={() => { setSkillMenuAnchor(null); openHooksDialog(name); }}>
                      Automations
                    </MenuItem>
                  ) : null}
                  <Divider />
                  <MenuItem
                    disabled={deleteSkillMutation.isPending}
                    sx={{ color: "error.main" }}
                    onClick={async () => {
                      setSkillMenuAnchor(null);
                      const ok = window.confirm(`Delete skill "${name}"? This cannot be undone.`);
                      if (ok) deleteSkillMutation.mutate(name);
                    }}
                  >
                    Delete
                  </MenuItem>
                </Menu>
              </>
            ) : null}
          </Stack>
        </Stack>
      </Box>
    );
  };

  const isCreateMode = !editTargetName;
  const useCreateWizard = isCreateMode && !editRawMode && createWizardEnabled;
  const scheduleInference = editTaskCron.trim() || inferTaskCronFromInstruction(editTaskInstruction);
  const scheduleBlocked = editAttachTask && !scheduleInference && !isRunOnceInstruction(editTaskInstruction);
  const hookBlocked = editAttachHook && !(editHookUrl.trim() || extractFirstUrl(editHookInstruction));
  const wizardStepBlocked =
    createWizardStep === 0
      ? !editForm.name.trim() || !isValidActionName(editForm.name) || !editForm.description.trim()
      : createWizardStep === 2
        ? hookBlocked || scheduleBlocked
        : false;

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Typography variant="h6">Skills</Typography>
          {skillsTab === "manage" ? (
            <Stack direction="row" spacing={1}>
              <Button size="small" variant="outlined" onClick={() => setAiCreateOpen(true)}>
                Create Skill
              </Button>
              <Button size="small" variant="outlined" onClick={() => setImportOpen(true)}>
                Import URL
              </Button>
              <Button size="small" variant="outlined" onClick={() => setBulkOpen(true)}>
                Bulk Import
              </Button>
            </Stack>
          ) : null}
        </Stack>
        <Typography variant="body2" color="text.secondary">
          System skills: {systemSkills.length}, custom skills: {customSkills.length}, automations: {hooks.length}.
        </Typography>
        {skillsTab === "manage" ? (
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.5 }}>
            Start with AI Quick Create. Use Advanced Editor only when you need manual SKILL.md control.
          </Typography>
        ) : null}
        {lastImport?.message ? (
          <Alert sx={{ mt: 1 }} severity={lastImport.result.status === "blocked" ? "warning" : "info"}>
            {lastImport.message}
          </Alert>
        ) : null}
        <Tabs
          value={skillsTab}
          onChange={(_, value: "manage" | "system") => setSkillsTab(value)}
          sx={{ mt: 1 }}
        >
          <Tab value="manage" label="My Skills" />
          <Tab value="system" label="System Skills" />
        </Tabs>
        <Stack direction="row" spacing={1} alignItems="center" sx={{ mt: 1.5 }}>
          <TextField
            size="small"
            placeholder="Search skills by name or description..."
            value={skillSearch}
            onChange={(e) => setSkillSearch(e.target.value)}
            sx={{ flex: 1 }}
            slotProps={{ input: { sx: { fontSize: "0.85rem" } } }}
          />
          <TextField
            select
            size="small"
            value={skillSort}
            onChange={(e) => setSkillSort(e.target.value as "name" | "imported")}
            sx={{ minWidth: 140 }}
            slotProps={{ input: { sx: { fontSize: "0.85rem" } } }}
          >
            <MenuItem value="name">Sort: Name</MenuItem>
            <MenuItem value="imported">Sort: Newest</MenuItem>
          </TextField>
        </Stack>
      </Box>

      {skillsTab === "manage" ? (
        <>
          {customSkills.length > 0 ? (
            <Box className="list-shell">
              <Stack spacing={1}>
                <Typography variant="h6">Custom Skills</Typography>
                <Stack spacing={1}>{customSkills.map((act) => renderActionRow(act, "custom"))}</Stack>
              </Stack>
            </Box>
          ) : null}

          {developerModeEnabled ? (
            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack direction="row" justifyContent="space-between" alignItems="center">
                  <Stack spacing={0.25}>
                    <Typography variant="h6">Automations</Typography>
                    <Typography variant="caption" color="text.secondary">
                      Advanced automation manager (Developer mode). Create from an action row.
                    </Typography>
                  </Stack>
                </Stack>
                {hooksQ.error ? (
                  <Alert severity="error">{errMessage(hooksQ.error)}</Alert>
                ) : hookRunsQ.error ? (
                  <Alert severity="warning">Automations loaded, but run reports failed: {errMessage(hookRunsQ.error)}</Alert>
                ) : hooks.length === 0 ? (
                  <Typography variant="body2" color="text.secondary">
                    No automations yet.
                  </Typography>
                ) : (
                  <TableContainer className="table-shell">
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Name</TableCell>
                          <TableCell>Trigger</TableCell>
                          <TableCell>Type</TableCell>
                          <TableCell>URL</TableCell>
                          <TableCell>Enabled</TableCell>
                          <TableCell>Last run</TableCell>
                          <TableCell align="right">Ops</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {hooks.map((hook, idx) => {
                          const id = str(hook.id, `hook-${idx}`);
                          const lastRun = hookLastRunById[id];
                          const runStatus = str(lastRun?.status, "-");
                          const runAttempts = num(lastRun?.attempts, 0);
                          const runError = str(lastRun?.error, "");
                          return (
                            <TableRow key={id}>
                              <TableCell>{str(hook.name, "-")}</TableCell>
                              <TableCell>{str(hook.trigger, "-")}</TableCell>
                              <TableCell>{str(hook.hook_type, "-")}</TableCell>
                              <TableCell sx={{ maxWidth: 280 }}>
                                <Typography variant="caption" color="text.secondary" noWrap title={str(hook.url, "-")}>
                                  {str(hook.url, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell>{boolText(hook.enabled)}</TableCell>
                              <TableCell sx={{ maxWidth: 240 }}>
                                {lastRun ? (
                                  <Typography
                                    variant="caption"
                                    color={runStatus === "failed" ? "error.main" : "text.secondary"}
                                    noWrap
                                    title={runError || str(lastRun?.timestamp, "")}
                                  >
                                    {runStatus}
                                    {runAttempts > 0 ? ` (${runAttempts})` : ""}
                                  </Typography>
                                ) : (
                                  <Typography variant="caption" color="text.secondary">
                                    never
                                  </Typography>
                                )}
                              </TableCell>
                              <TableCell align="right">
                                <RowOpsMenu
                                  actions={[
                                    {
                                      label: "Remove",
                                      tone: "error",
                                      disabled: removeHookMutation.isPending,
                                      onClick: async () => {
                                        const ok = window.confirm("Remove this automation?");
                                        if (!ok) return;
                                        try {
                                          await removeHookMutation.mutateAsync(id);
                                        } catch (err) {
                                          setLastImport({
                                            result: { status: "error", name: str(hook.name, "automation"), message: errMessage(err) },
                                            message: `Failed to remove automation '${str(hook.name, "automation")}': ${errMessage(err)}`
                                          });
                                        }
                                      }
                                    }
                                  ]}
                                  ariaLabel="Automation options"
                                />
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                )}
              </Stack>
            </Box>
          ) : null}

          <Box className="list-shell">
            <Accordion
              defaultExpanded={false}
              elevation={0}
              sx={{
                background: "transparent",
                "&::before": { display: "none" }
              }}
            >
              <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ px: 0 }}>
                <Stack spacing={0.25}>
                  <Typography variant="h6">Bundled Skills</Typography>
                  <Typography variant="caption" color="text.secondary">
                    Ready-made skills you can enable and use.
                  </Typography>
                </Stack>
              </AccordionSummary>
              <AccordionDetails sx={{ px: 0, pt: 0 }}>
                {bundledSkills.length === 0 ? (
                  <Typography variant="body2" color="text.secondary">
                    No bundled skills detected.
                  </Typography>
                ) : (
                  <Stack spacing={1}>{bundledSkills.map((act) => renderActionRow(act, "bundled"))}</Stack>
                )}
              </AccordionDetails>
            </Accordion>
          </Box>
        </>
      ) : (
        <Box className="list-shell">
          <Stack spacing={1}>
            <Typography variant="h6">System Skills</Typography>
            <Typography variant="caption" color="text.secondary">
              Built-in and locked. Always available.
            </Typography>
            {systemSkills.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No system skills detected.
              </Typography>
            ) : (
              <Stack spacing={1}>{systemSkills.map((act) => renderActionRow(act, "system"))}</Stack>
            )}
          </Stack>
        </Box>
      )}

      <ImportUrlDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={handleImported}
        onAfterImport={afterImport}
      />
      <BulkImportDialog
        open={bulkOpen}
        onClose={() => setBulkOpen(false)}
        onImported={handleImported}
        onAfterImport={afterImport}
      />

      <Dialog open={aiCreateOpen} onClose={() => setAiCreateOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Create Skill</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Alert severity="info">
              AI Quick Create is recommended for beginners. Describe your goal in plain language.
            </Alert>
            <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
              {`Prompt examples:
1. Track top 10 AI startups weekly, compare funding/news changes, and output a ranked briefing with sources.
2. Review competitor pricing pages every day and generate a change log with impact notes.
3. Generate a pre-meeting research brief for a company from latest news, filings, and leadership updates.
4. If this analysis fails, send update to URL (for example: your Twilio/Telegram notifier endpoint).
5. Run this every weekday at 9am and send a summary after each run.`}
            </Typography>
            {aiError ? <Alert severity="error">{aiError}</Alert> : null}
            <TextField
              fullWidth
              size="small"
              label="Skill name (optional)"
              placeholder="example: market-analyzer"
              value={aiNameHint}
              onChange={(e) => setAiNameHint(normalizeActionName(e.target.value))}
              helperText="If blank, AI will suggest a name."
            />
            <TextField
              fullWidth
              multiline
              minRows={6}
              label="What should this skill do?"
              placeholder="Example: Find small-cap momentum stocks, validate with latest filings, then output top 5 picks with risks."
              value={aiPrompt}
              onChange={(e) => setAiPrompt(e.target.value)}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setAiCreateOpen(false);
              openNewEditor();
            }}
          >
            Advanced Editor
          </Button>
          <Button onClick={() => setAiCreateOpen(false)}>Close</Button>
          <Button
            variant="contained"
            disabled={aiGenerateMutation.isPending || !aiPrompt.trim()}
            onClick={() => {
              setAiError(null);
              aiGenerateMutation.mutate({ prompt: aiPrompt.trim(), nameHint: aiNameHint.trim() });
            }}
          >
            {aiGenerateMutation.isPending ? "Creating..." : "Create with AI"}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog open={editOpen} onClose={closeEditor} maxWidth="md" fullWidth>
        <DialogTitle>{editTargetName ? `Edit skill: ${editTargetName}` : "Create skill"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {editError ? <Alert severity="error">{editError}</Alert> : null}
            {editLoading ? <Alert severity="info">Loading skill details...</Alert> : null}
            {editRawMode ? (
              <Alert severity="warning">
                Developer mode is enabled. You are editing raw SKILL.md directly.
              </Alert>
            ) : (
              <Alert severity="info">
                Beginner mode is on. Fill simple fields and AgentArk will generate the SKILL file for you.
                Need raw SKILL.md editing? Enable Developer mode in Settings -&gt; Advanced.
              </Alert>
            )}

            {isCreateMode && !editRawMode ? (
              <FormControlLabel
                control={<Switch checked={createWizardEnabled} onChange={(e) => setCreateWizardEnabled(e.target.checked)} />}
                label="Use 3-step wizard (recommended). Turn off to use the classic editor."
              />
            ) : null}

            {editRawMode ? (
              <TextField
                fullWidth
                multiline
                minRows={16}
                value={editContent}
                onChange={(e) => setEditContent(e.target.value)}
                label="SKILL.md"
              />
            ) : useCreateWizard ? (
              <Stack spacing={1.25}>
                <Tabs value={createWizardStep} onChange={(_, v) => setCreateWizardStep(Number(v) || 0)} variant="fullWidth">
                  <Tab label="1. What it does" value={0} />
                  <Tab label="2. Inputs" value={1} />
                  <Tab label="3. Run automatically" value={2} />
                </Tabs>

                {createWizardStep === 0 ? (
                  <Grid2 container spacing={1.25}>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Skill name"
                        value={editForm.name}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, name: normalizeActionName(e.target.value) }))}
                        helperText="Use lowercase letters, numbers, and hyphens. Example: market-analysis"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Version"
                        value={editForm.version}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, version: e.target.value }))}
                        helperText="Default: 1.0.0"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Description"
                        value={editForm.description}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, description: e.target.value }))}
                        helperText="One line: what this skill does."
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        multiline
                        minRows={10}
                        label="Workflow instructions"
                        value={editForm.workflow}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, workflow: e.target.value }))}
                        helperText="Write clear instructions for how this skill should execute."
                      />
                    </Grid2>
                  </Grid2>
                ) : null}

                {createWizardStep === 1 ? (
                  <Grid2 container spacing={1.25}>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Required inputs (optional)"
                        placeholder="from, to, budget"
                        value={editForm.requiredInputsCsv}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, requiredInputsCsv: e.target.value }))}
                        helperText="Comma separated field names. If missing at runtime, user will be asked (or fallback used in scheduled runs)."
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Emoji (optional)"
                        value={editForm.emoji}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, emoji: e.target.value }))}
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 8 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Tools (comma separated)"
                        placeholder="web_search, file_read"
                        value={editForm.toolsCsv}
                        onChange={(e) => setEditForm((prev) => ({ ...prev, toolsCsv: e.target.value }))}
                        helperText="These are skills/tools your workflow may rely on."
                      />
                    </Grid2>
                  </Grid2>
                ) : null}
              </Stack>
            ) : (
              <Grid2 container spacing={1.25}>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Skill name"
                    value={editForm.name}
                    disabled={!!editTargetName}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, name: normalizeActionName(e.target.value) }))}
                    helperText={
                      editTargetName
                        ? "Skill name is fixed for existing skills."
                        : "Use lowercase letters, numbers, and hyphens. Example: market-analysis"
                    }
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Version"
                    value={editForm.version}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, version: e.target.value }))}
                    helperText="Default: 1.0.0"
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Description"
                    value={editForm.description}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, description: e.target.value }))}
                    helperText="One line: what this skill does."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Required inputs (optional)"
                    placeholder="from, to, budget"
                    value={editForm.requiredInputsCsv}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, requiredInputsCsv: e.target.value }))}
                    helperText="Comma separated field names. If missing at runtime, user will be asked (or fallback used in scheduled runs)."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 4 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Emoji (optional)"
                    value={editForm.emoji}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, emoji: e.target.value }))}
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 8 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Tools (comma separated)"
                    placeholder="web_search, file_read"
                    value={editForm.toolsCsv}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, toolsCsv: e.target.value }))}
                    helperText="These are skills/tools your workflow may rely on."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    multiline
                    minRows={14}
                    label="Workflow instructions"
                    value={editForm.workflow}
                    onChange={(e) => setEditForm((prev) => ({ ...prev, workflow: e.target.value }))}
                    helperText="Write clear instructions for how this skill should execute."
                  />
                </Grid2>
              </Grid2>
            )}

            {!useCreateWizard || createWizardStep === 2 ? (
              <Box className="metadata-box">
              <Stack spacing={1}>
                <FormControlLabel
                  control={<Switch checked={editAttachHook} onChange={(e) => setEditAttachHook(e.target.checked)} />}
                  label="Run automatically (optional)"
                />
                {editAttachHook ? (
                  <Stack spacing={1}>
                    <TextField
                      fullWidth
                      size="small"
                      multiline
                      minRows={2}
                      label="When should this run? (plain language)"
                      value={editHookInstruction}
                      onChange={(e) => setEditHookInstruction(e.target.value)}
                      placeholder="Examples: when this skill fails | after each run | before this skill starts"
                    />
                    {developerModeEnabled ? (
                      <>
                        <Stack direction="row" spacing={1}>
                          <Button size="small" variant="outlined" onClick={applyEditHookInstruction}>
                            Interpret Text
                          </Button>
                          <Typography variant="caption" color="text.secondary" sx={{ alignSelf: "center" }}>
                            Infers trigger and URL.
                          </Typography>
                        </Stack>
                        <Grid2 container spacing={1}>
                          <Grid2 size={{ xs: 12, md: 4 }}>
                            <TextField
                              fullWidth
                              size="small"
                              select
                              label="When to run"
                              value={editHookTrigger}
                              onChange={(e) => setEditHookTrigger((e.target.value as HookTriggerValue) || "on_error")}
                            >
                              <MenuItem value="pre_message">pre_message</MenuItem>
                              <MenuItem value="post_message">post_message</MenuItem>
                              <MenuItem value="pre_action">pre_action</MenuItem>
                              <MenuItem value="post_action">post_action</MenuItem>
                              <MenuItem value="on_consolidate">on_consolidate</MenuItem>
                              <MenuItem value="on_error">on_error</MenuItem>
                            </TextField>
                          </Grid2>
                          <Grid2 size={{ xs: 12, md: 8 }}>
                            <TextField
                              fullWidth
                              size="small"
                              label="Send update to URL"
                              value={editHookUrl}
                              onChange={(e) => setEditHookUrl(e.target.value)}
                              placeholder="https://example.com/hook"
                            />
                          </Grid2>
                        </Grid2>
                      </>
                    ) : (
                      <TextField
                        fullWidth
                        size="small"
                        label="Send update to URL"
                        value={editHookUrl}
                        onChange={(e) => setEditHookUrl(e.target.value)}
                        placeholder="https://example.com/hook"
                        helperText="Required to enable this automation."
                      />
                    )}
                    <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
                      {`Automation examples:
1. when this skill fails
2. after each successful run
3. before this skill starts
4. when this skill fails, send update to URL https://example.com/hook
5. when this skill fails, send update to URL https://your-notifier.example/twilio`}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      For phone/SMS/WhatsApp/Telegram alerts, use your notification URL endpoint to forward via Twilio or your preferred channel integration.
                    </Typography>
                  </Stack>
                ) : null}
                <Divider />
                <FormControlLabel
                  control={<Switch checked={editAttachTask} onChange={(e) => setEditAttachTask(e.target.checked)} />}
                  label="Schedule this skill (optional)"
                />
                {editAttachTask ? (
                  <Stack spacing={1}>
                    <TextField
                      fullWidth
                      size="small"
                      multiline
                      minRows={2}
                      label="When should this run? (plain language)"
                      value={editTaskInstruction}
                      onChange={(e) => setEditTaskInstruction(e.target.value)}
                      placeholder="Examples: every day at 9am | hourly | weekdays at 9am | once now"
                    />
                    <Stack direction="row" spacing={1}>
                      <Button size="small" variant="outlined" onClick={applyEditTaskInstruction}>
                        Interpret Text
                      </Button>
                      <Typography variant="caption" color="text.secondary" sx={{ alignSelf: "center" }}>
                        Infers cron schedule.
                      </Typography>
                    </Stack>
                    <TextField
                      fullWidth
                      size="small"
                      label="Cron (optional, auto-filled)"
                      value={editTaskCron}
                      onChange={(e) => setEditTaskCron(e.target.value)}
                      placeholder="0 9 * * *"
                      helperText="Use 5-field cron. Leave blank if you prefer plain language."
                    />
                    <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
                      {`Schedule examples:
1. every day at 9am
2. every 15 minutes
3. weekdays at 9am
4. once now`}
                    </Typography>
                  </Stack>
                ) : null}
              </Stack>
            </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeEditor}>Close</Button>
          {useCreateWizard && createWizardStep > 0 ? (
            <Button onClick={() => setCreateWizardStep((s) => Math.max(0, s - 1))}>
              Back
            </Button>
          ) : null}
          {useCreateWizard ? (
            <Button
              variant="contained"
              onClick={() => {
                if (createWizardStep < 2) {
                  setCreateWizardStep((s) => Math.min(2, s + 1));
                } else {
                  saveEditor();
                }
              }}
              disabled={wizardStepBlocked || editLoading}
            >
              {createWizardStep < 2 ? "Next" : "Save"}
            </Button>
          ) : (
            <Button
              variant="contained"
              onClick={saveEditor}
              disabled={
                editLoading ||
                (editRawMode ? !editContent.trim() : !editForm.description.trim()) ||
                hookBlocked ||
                scheduleBlocked
              }
            >
              Save
            </Button>
          )}
        </DialogActions>
      </Dialog>

      <Dialog open={hooksOpen} onClose={closeHooksDialog} maxWidth="sm" fullWidth>
        <DialogTitle>{hooksTargetAction ? `Automations for ${hooksTargetAction}` : "Create Automation"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {hookError ? <Alert severity="error">{hookError}</Alert> : null}
            <Alert severity="info">
              Describe in plain language and AgentArk will infer trigger defaults.
            </Alert>
            <Typography variant="caption" color="text.secondary">
              Advanced automation editor (Developer mode).
            </Typography>
            <TextField
              fullWidth
              multiline
              minRows={2}
              label="When should this run? (plain language)"
              value={hookInstruction}
              onChange={(e) => setHookInstruction(e.target.value)}
              placeholder={hooksTargetAction ? `when ${hooksTargetAction} fails` : "after each run"}
            />
            <Stack direction="row" spacing={1}>
              <Button size="small" variant="outlined" onClick={applyHookInstruction}>
                Interpret Text
              </Button>
              <Typography variant="caption" color="text.secondary" sx={{ alignSelf: "center" }}>
                Fills trigger and URL when detectable.
              </Typography>
            </Stack>
            <TextField
              fullWidth
              size="small"
              label="Automation name"
              value={hookName}
              onChange={(e) => setHookName(sanitizeHookName(e.target.value))}
            />
            <TextField
              fullWidth
              size="small"
              select
              label="When to run"
              value={hookTrigger}
              onChange={(e) => setHookTrigger((e.target.value as HookTriggerValue) || "post_action")}
            >
              <MenuItem value="pre_message">pre_message</MenuItem>
              <MenuItem value="post_message">post_message</MenuItem>
              <MenuItem value="pre_action">pre_action</MenuItem>
              <MenuItem value="post_action">post_action</MenuItem>
              <MenuItem value="on_consolidate">on_consolidate</MenuItem>
              <MenuItem value="on_error">on_error</MenuItem>
            </TextField>
            <TextField
              fullWidth
              size="small"
              label="Send update to URL"
              value={hookUrl}
              onChange={(e) => setHookUrl(e.target.value)}
              placeholder="https://example.com/hook"
            />
            {hooksTargetAction ? (
              <>
                <Divider />
                <Typography variant="subtitle2">Existing automations for this skill</Typography>
                {hooksForSelectedAction.length === 0 ? (
                  <Typography variant="body2" color="text.secondary">
                    No automations attached yet.
                  </Typography>
                ) : (
                  <Stack spacing={0.6}>
                    {hooksForSelectedAction.map((h, idx) => (
                      <Box key={str(h.id, `dialog-hook-${idx}`)} className="console-line">
                        <Typography variant="caption" color="text.secondary">
                          {str(h.trigger, "-")} | {boolText(h.enabled)}
                        </Typography>
                        <Typography variant="body2" noWrap title={str(h.name, "-")}>
                          {str(h.name, "-")}
                        </Typography>
                      </Box>
                    ))}
                  </Stack>
                )}
              </>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeHooksDialog}>Close</Button>
          <Button
            variant="contained"
            disabled={addHookMutation.isPending || !(hookUrl.trim() || extractFirstUrl(hookInstruction))}
            onClick={saveHookFromDialog}
          >
            {addHookMutation.isPending ? "Saving..." : "Save Automation"}
          </Button>
        </DialogActions>
      </Dialog>

      <SkillSecretsDialog open={secretsName != null} skillName={secretsName} onClose={() => setSecretsName(null)} />
    </Stack>
  );
}

function AppsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const appsQ = useQuery({ queryKey: ["apps-manager"], queryFn: () => api.rawGet("/api/apps"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const tunnelQ = useQuery({
    queryKey: ["apps-manager-tunnel-status"],
    queryFn: () => api.rawGet("/tunnel/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const evolutionQ = useQuery({
    queryKey: ["settings-evolution"],
    queryFn: () => api.rawGet("/settings/evolution"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const [tunnelActionError, setTunnelActionError] = useState<string | null>(null);
  const [tunnelActionState, setTunnelActionState] = useState<"idle" | "starting" | "stopping">("idle");
  const [tunnelActionAppId, setTunnelActionAppId] = useState<string>("");
  const [appsActionError, setAppsActionError] = useState<string | null>(null);
  const [appsActionSuccess, setAppsActionSuccess] = useState<string | null>(null);
  const [appsActionBusy, setAppsActionBusy] = useState<string | null>(null);

  const opMutation = useMutation({
    mutationFn: ({ path, method, body }: { path: string; method: "POST" | "DELETE"; body?: JsonRecord }) =>
      (method === "DELETE" ? api.rawDelete(path) : api.rawPost(path, body ?? {})),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager-tunnel-status"] });
    }
  });
  const updateEvolutionSettingsMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/settings/evolution", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
    }
  });
  const tunnelStartMutation = useMutation({
    mutationFn: (payload: { app_id?: string }) => api.rawPost("/tunnel/start", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["apps-manager-tunnel-status"] });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    }
  });
  const tunnelStopMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/stop", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["apps-manager-tunnel-status"] });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    }
  });

  const apps = pickRecords(appsQ.data, "apps");
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const tunnel = asRecord(tunnelQ.data);
  const tunnelBaseUrl = str(tunnel.url, "").trim().replace(/\/+$/, "");
  const tunnelActive = toBool(tunnel.active);
  const tunnelAvailable = toBool(tunnel.available);
  const tunnelErrorText = str(tunnel.error, "").trim();
  const selectedPublicAppId = str(tunnel.selected_app_id, "").trim();
  const evolution = asRecord(evolutionQ.data);
  const deployGuardDefault = toBool(evolution.deploy_guard_default);
  const tunnelStarting = tunnelActionState === "starting" || tunnelStartMutation.isPending;
  const tunnelStopping = tunnelActionState === "stopping" || tunnelStopMutation.isPending;

  useEffect(() => {
    if (tunnelActionState === "starting") {
      if (tunnelBaseUrl || tunnelErrorText) {
        setTunnelActionState("idle");
      }
      return;
    }
    if (tunnelActionState === "stopping" && !tunnelActive) {
      setTunnelActionState("idle");
    }
  }, [tunnelActionState, tunnelBaseUrl, tunnelErrorText, tunnelActive]);

  useEffect(() => {
    if (appsActionSuccess) {
      const timer = setTimeout(() => setAppsActionSuccess(null), 3500);
      return () => clearTimeout(timer);
    }
  }, [appsActionSuccess]);

  useEffect(() => {
    if (tunnelActionState === "idle") return;
    const timer = setInterval(() => {
      void tunnelQ.refetch();
      void appsQ.refetch();
    }, 1200);
    return () => clearInterval(timer);
  }, [tunnelActionState, tunnelQ, appsQ]);

  const refreshLinks = async () => {
    setTunnelActionError(null);
    await Promise.all([appsQ.refetch(), tunnelQ.refetch()]);
  };
  const refreshAppState = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["apps-manager"] }),
      queryClient.invalidateQueries({ queryKey: ["apps-manager-tunnel-status"] })
    ]);
  };
  const runAppOp = async (opts: { label: string; path: string; method: "POST" | "DELETE"; body?: JsonRecord }) => {
    setAppsActionError(null);
    setAppsActionSuccess(null);
    setAppsActionBusy(opts.label);
    try {
      await opMutation.mutateAsync({ path: opts.path, method: opts.method, body: opts.body });
      await refreshAppState();
      setAppsActionSuccess(`${opts.label} completed.`);
    } catch (e) {
      setAppsActionError(errMessage(e));
    } finally {
      setAppsActionBusy(null);
    }
  };
  const setDeployGuardDefault = async (enabled: boolean) => {
    setAppsActionError(null);
    setAppsActionSuccess(null);
    try {
      await updateEvolutionSettingsMutation.mutateAsync({ deploy_guard_default: enabled });
      setAppsActionSuccess(`Deploy guard default ${enabled ? "enabled" : "disabled"}.`);
    } catch (e) {
      setAppsActionError(errMessage(e));
    }
  };
  const startTunnel = async (appId?: string) => {
    setTunnelActionError(null);
    setTunnelActionState("starting");
    setTunnelActionAppId(appId || "");
    try {
      await tunnelStartMutation.mutateAsync(appId ? { app_id: appId } : {});
      await refreshLinks();
    } catch (e) {
      setTunnelActionState("idle");
      setTunnelActionError(errMessage(e));
    }
  };
  const stopTunnel = async () => {
    setTunnelActionError(null);
    setTunnelActionState("stopping");
    try {
      await tunnelStopMutation.mutateAsync();
      await refreshLinks();
    } catch (e) {
      setTunnelActionState("idle");
      setTunnelActionError(errMessage(e));
    }
  };

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }} spacing={1} mb={1}>
          <Box>
            <Typography variant="h6">Deployed Apps</Typography>
            <Typography variant="caption" color="text.secondary">
              Manage app runtime, public exposure, and guard defaults.
            </Typography>
          </Box>
          <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
            <Chip
              size="small"
              color={deployGuardDefault ? "warning" : "default"}
              label={deployGuardDefault ? "Guard default ON" : "Guard default OFF"}
            />
            <Tooltip title="Toggle the default access guard behavior used for new app deploys." arrow>
              <span>
                <Button
                  size="small"
                  variant="outlined"
                  disabled={updateEvolutionSettingsMutation.isPending}
                  onClick={() => void setDeployGuardDefault(!deployGuardDefault)}
                >
                  {deployGuardDefault ? "Disable Default Guard" : "Enable Default Guard"}
                </Button>
              </span>
            </Tooltip>
          </Stack>
        </Stack>
        {evolutionQ.error ? <Alert severity="error" sx={{ mb: 1 }}>{errMessage(evolutionQ.error)}</Alert> : null}
        {tunnelQ.error ? <Alert severity="error" sx={{ mb: 1 }}>{errMessage(tunnelQ.error)}</Alert> : null}
        {tunnelErrorText ? <Alert severity="error" sx={{ mb: 1 }}>{tunnelErrorText}</Alert> : null}
        {tunnelActionError ? <Alert severity="error" sx={{ mb: 1 }}>{tunnelActionError}</Alert> : null}
        {appsActionError ? <Alert severity="error" sx={{ mb: 1 }}>{appsActionError}</Alert> : null}
        {appsActionSuccess ? <Alert severity="success" sx={{ mb: 1 }}>{appsActionSuccess}</Alert> : null}
        <TableContainer className="table-shell">
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Title</TableCell>
                <TableCell>ID</TableCell>
                <TableCell>Running</TableCell>
                <TableCell>Links</TableCell>
                <TableCell align="right">Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {apps.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5}>
                    <Typography variant="body2" color="text.secondary">
                      There are no deployed apps at this time. When you create any app with agent, it will show here.
                    </Typography>
                  </TableCell>
                </TableRow>
              ) : (
                apps.map((appItem) => {
                  const id = str(appItem.id, "");
                  const url = str(appItem.url, "");
                  const accessUrl = str(appItem.access_url, "");
                  const accessKey = str(appItem.access_key, "").trim() || extractAccessKeyFromUrl(accessUrl, origin);
                  const localUrl = toAbsoluteAppUrl(url, origin);
                  const localAccessUrl = toAbsoluteAppUrl(accessUrl || url, origin);
                  const isSelectedPublicApp = selectedPublicAppId === id;
                  const runtimeMode = str(appItem.runtime_mode, "").trim().toLowerCase();
                  const isStaticApp = runtimeMode === "static";
                  const isRunning = toBool(appItem.running);
                  const canStopApp = !isStaticApp && isRunning;
                  const canRestartApp = !isStaticApp || isRunning;
                  const appTunnelActive = tunnelActive && !!tunnelBaseUrl && !!selectedPublicAppId;
                  const publicUrl = appTunnelActive ? toAbsoluteAppUrl(url, tunnelBaseUrl) : "";
                  const publicAccessUrl =
                    appTunnelActive ? toAbsoluteAppUrl(accessUrl || url, tunnelBaseUrl) : "";
                  const hasProtectedVariant = !!accessUrl && localAccessUrl !== localUrl;
                  const controlPlaneTunnelOnly = tunnelActive && !!tunnelBaseUrl && !selectedPublicAppId;
                  const publicLinkUrl = hasProtectedVariant ? publicAccessUrl || publicUrl : publicUrl;
                  const publicShareUrl = publicLinkUrl;
                  const localShareUrl = hasProtectedVariant ? localAccessUrl : localUrl;
                  const shareUrl = publicShareUrl || localShareUrl;
                  const openTargets = dedupeLinkTargets([
                    { label: "Open Local", url: localUrl },
                    { label: "Open Local (Guarded)", url: hasProtectedVariant ? localAccessUrl : "" },
                    { label: "Open Public", url: publicUrl },
                    { label: "Open Public (Guarded)", url: hasProtectedVariant ? publicAccessUrl : "" }
                  ]);
                  return (
                    <TableRow key={id}>
                      <TableCell>{str(appItem.title)}</TableCell>
                      <TableCell>{id}</TableCell>
                      <TableCell>{str(appItem.running)}</TableCell>
                      <TableCell sx={{ maxWidth: 420 }}>
                        <Stack spacing={0.2}>
                          {localUrl ? (
                            <Typography variant="caption" component="div" noWrap title={localUrl}>
                              Local:{" "}
                              <Link href={localUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                                {localUrl}
                              </Link>
                            </Typography>
                          ) : (
                            <Typography variant="caption" component="div" noWrap title={url || "-"}>
                              Local: {url || "-"}
                            </Typography>
                          )}
                          {hasProtectedVariant ? (
                            <Typography variant="caption" component="div" noWrap title={accessKey || localAccessUrl}>
                              Access Key: {accessKey || "-"}
                            </Typography>
                          ) : null}
                          <Typography variant="caption" component="div" color={toBool(appItem.access_guard_enabled) ? "warning.main" : "text.secondary"} noWrap>
                            Guard: {toBool(appItem.access_guard_enabled) ? "enabled" : "disabled"}
                          </Typography>
                          {publicLinkUrl ? (
                            <Typography variant="caption" component="div" color="info.main" noWrap title={publicLinkUrl}>
                              Public:{" "}
                              <Link href={publicLinkUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                                {publicLinkUrl}
                              </Link>
                            </Typography>
                          ) : tunnelStarting && tunnelActionAppId === id ? (
                            <Typography variant="caption" component="div" color="info.main">
                              Public: starting tunnel...
                            </Typography>
                          ) : tunnelStopping && isSelectedPublicApp ? (
                            <Typography variant="caption" component="div" color="text.secondary">
                              Public: stopping tunnel...
                            </Typography>
                          ) : controlPlaneTunnelOnly ? (
                            <Typography variant="caption" component="div" color="text.secondary">
                              Public: control-plane tunnel active. Expose this app publicly to get a working app link.
                            </Typography>
                          ) : (
                            <Typography variant="caption" component="div" color="text.secondary">
                              Public: tunnel inactive
                            </Typography>
                          )}
                        </Stack>
                      </TableCell>
                      <TableCell align="right">
                        <RowOpsMenu
                          actions={[
                            ...openTargets.map((target, idx) => ({
                              label: target.label,
                              divider: idx === 0 ? false : undefined,
                              onClick: () => {
                                window.open(target.url, "_blank", "noopener,noreferrer");
                              }
                            })),
                            {
                              label: publicShareUrl ? "Copy Public Link" : "Copy Local Link",
                              divider: openTargets.length > 0,
                              disabled: !shareUrl,
                              onClick: async () => {
                                if (!shareUrl) return;
                                try {
                                  await navigator.clipboard.writeText(shareUrl);
                                } catch {
                                  window.prompt("Copy this link", shareUrl);
                                }
                              }
                            },
                            ...(accessKey
                              ? [{
                                  label: "Copy Access Key",
                                  onClick: async () => {
                                    try {
                                      await navigator.clipboard.writeText(accessKey);
                                    } catch {
                                      window.prompt("Copy this access key", accessKey);
                                    }
                                  }
                                }]
                              : []),
                            {
                              label: toBool(appItem.access_guard_enabled) ? "Disable App Guard" : "Enable App Guard",
                              disabled: updateEvolutionSettingsMutation.isPending || appsActionBusy != null,
                              onClick: () =>
                                void runAppOp({
                                  label: toBool(appItem.access_guard_enabled) ? "Disable App Guard" : "Enable App Guard",
                                  path: `/api/apps/${encodeURIComponent(id)}/access-guard`,
                                  method: "POST",
                                  body: { enabled: !toBool(appItem.access_guard_enabled) }
                                })
                            },
                            {
                              label: "Regenerate Access Key",
                              disabled:
                                updateEvolutionSettingsMutation.isPending ||
                                appsActionBusy != null ||
                                !toBool(appItem.access_guard_enabled),
                              onClick: () =>
                                void runAppOp({
                                  label: "Regenerate Access Key",
                                  path: `/api/apps/${encodeURIComponent(id)}/access-guard`,
                                  method: "POST",
                                  body: { enabled: true, regenerate_key: true }
                                })
                            },
                            {
                              label:
                                tunnelStarting
                                  ? "Starting Public Tunnel..."
                                  : tunnelActive && selectedPublicAppId === id
                                    ? "Refresh Public Exposure"
                                    : tunnelActive && selectedPublicAppId && selectedPublicAppId !== id
                                      ? "Set as Public Landing App"
                                      : "Start Public Tunnel",
                              divider: true,
                              disabled: tunnelStarting || !tunnelAvailable,
                              onClick: () => startTunnel(id)
                            },
                            {
                              label: tunnelStopping ? "Stopping Public Tunnel..." : "Stop Public Tunnel",
                              disabled: tunnelStopping || !tunnelActive,
                              onClick: stopTunnel
                            },
                            {
                              label: "Refresh Public Link",
                              onClick: refreshLinks
                            },
                            {
                              label: !canStopApp ? "Stop Unavailable" : "Stop",
                              divider: true,
                              disabled: !canStopApp || appsActionBusy != null,
                              onClick: () =>
                                void runAppOp({
                                  label: "Stop App",
                                  path: `/api/apps/${encodeURIComponent(id)}/stop`,
                                  method: "POST"
                                })
                            },
                            {
                              label: isStaticApp ? "Reload Metadata" : isRunning ? "Restart" : "Start App",
                              disabled: appsActionBusy != null || !canRestartApp,
                              onClick: () =>
                                void runAppOp({
                                  label: isStaticApp ? "Reload Metadata" : isRunning ? "Restart App" : "Start App",
                                  path: `/api/apps/${encodeURIComponent(id)}/restart`,
                                  method: "POST"
                                })
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              disabled: appsActionBusy != null,
                              onClick: () =>
                                void runAppOp({
                                  label: "Delete App",
                                  path: `/api/apps/${encodeURIComponent(id)}`,
                                  method: "DELETE"
                                })
                            }
                          ]}
                          ariaLabel="App options"
                        />
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>
    </Stack>
  );
}

function GoalsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  type GoalLoopPayload = {
    goal: string;
    constraints?: string;
    due_date?: string;
    report_cron?: string;
    preview_only?: boolean;
    plan_override?: JsonRecord;
  };
  const [description, setDescription] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [autopilotEnabled, setAutopilotEnabled] = useState(true);
  const [guardrails, setGuardrails] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [scheduleKey, setScheduleKey] = useState("daily_9");
  const [reportCron, setReportCron] = useState("0 0 9 * * *"); // 09:00 daily (UTC unless server uses user tz)
  const [selectedGoalId, setSelectedGoalId] = useState<string | null>(null); // goal_id from arguments
  const [planPreview, setPlanPreview] = useState<JsonRecord | null>(null);
  const [goalCreateOpen, setGoalCreateOpen] = useState(false);
  const [goalConfirmOpen, setGoalConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const schedulePresets: { key: string; label: string; cron: string | null; hint?: string }[] = [
    { key: "run_5", label: "Every 5 minutes", cron: "0 */5 * * * *" },
    { key: "run_10", label: "Every 10 minutes", cron: "0 */10 * * * *" },
    { key: "run_30", label: "Every 30 minutes", cron: "0 */30 * * * *" },
    { key: "hourly", label: "Hourly", cron: "0 0 * * * *" },
    { key: "daily_9", label: "Daily (09:00)", cron: "0 0 9 * * *" },
    { key: "weekly_mon_9", label: "Weekly (Mon 09:00)", cron: "0 0 9 * * 1" },
    { key: "monthly_1_9", label: "Monthly (1st 09:00)", cron: "0 0 9 1 * *" },
    { key: "custom", label: "Custom", cron: null, hint: "Cron uses 6 fields: sec min hour day month weekday" }
  ];
  const scheduleLabel = (key: string) => {
    for (const p of schedulePresets) {
      if (p.key === key) return p.label;
    }
    return "Custom";
  };

  const goalsQ = useQuery({
    queryKey: ["goals-list"],
    queryFn: () => api.rawGet("/goals?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const progressPath = selectedGoalId ? `/autonomy/goals/progress?goal_id=${encodeURIComponent(selectedGoalId)}` : "/autonomy/goals/progress";
  const progressQ = useQuery({
    queryKey: ["goals-progress", selectedGoalId],
    queryFn: () => api.rawGet(progressPath),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const createMutation = useMutation({
    mutationFn: (payload: { description: string; due_date?: string }) => api.rawPost("/goals", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    }
  });

  const autopilotPreviewMutation = useMutation({
    mutationFn: (payload: GoalLoopPayload) =>
      api.rawPost("/autonomy/goals/loop", { ...payload, preview_only: true })
  });

  const autopilotMutation = useMutation({
    mutationFn: (payload: GoalLoopPayload) => api.rawPost("/autonomy/goals/loop", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    }
  });

  const runNowMutation = useMutation({
    mutationFn: (goalId: string) => api.rawPost("/autonomy/goals/report_now", { goal_id: goalId }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    }
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/goals/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });

  const summary = asRecord(asRecord(progressQ.data).summary);
  const goals = pickRecords(goalsQ.data, "goals");
  const progressItems = pickRecords(progressQ.data, "items");

  const examples = [
    "Build a weekly arXiv dashboard for RL + time series",
    "Ship a working prototype by Friday",
    "Audit the app for security issues and write a fix plan"
  ];

  const resetGoalDraft = (nextAutopilot: boolean, nextDescription = "") => {
    setDescription(nextDescription);
    setDueDate("");
    setGuardrails("");
    setScheduleKey("daily_9");
    setReportCron("0 0 9 * * *");
    setAdvancedOpen(false);
    setAutopilotEnabled(nextAutopilot);
    setGoalConfirmOpen(false);
    setPlanPreview(null);
    setError(null);
  };

  const openGoalDialog = (nextAutopilot = true, nextDescription = "") => {
    resetGoalDraft(nextAutopilot, nextDescription);
    setGoalCreateOpen(true);
  };

  const buildGoalLoopPayload = (): GoalLoopPayload => ({
    goal: description.trim(),
    constraints: guardrails.trim() || undefined,
    due_date: dueDate.trim() || undefined,
    report_cron: reportCron.trim() || undefined
  });

  const submitGoalDraft = async () => {
    setError(null);
    try {
      const goalText = description.trim();
      if (autopilotEnabled) {
        if (!goalText) {
          setError("Goal is required.");
          return;
        }
        const previewOut = await autopilotPreviewMutation.mutateAsync(buildGoalLoopPayload());
        const preview = asRecord(asRecord(previewOut).plan_preview);
        setPlanPreview(Object.keys(preview).length ? preview : null);
        setGoalCreateOpen(false);
        setGoalConfirmOpen(true);
        return;
      } else {
        await createMutation.mutateAsync({
          description: goalText,
          due_date: dueDate.trim() || undefined
        });
      }
      setGoalCreateOpen(false);
      resetGoalDraft(true);
    } catch (e) {
      setError(errMessage(e));
    }
  };

  const confirmAutopilotGoal = async () => {
    setError(null);
    try {
      const out = await autopilotMutation.mutateAsync({
        ...buildGoalLoopPayload(),
        plan_override: planPreview || undefined
      });
      const gid = str(asRecord(out).goal_id, "");
      if (gid) setSelectedGoalId(gid);
      setGoalConfirmOpen(false);
      setGoalCreateOpen(false);
      resetGoalDraft(true);
    } catch (e) {
      setError(errMessage(e));
    }
  };

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }} spacing={1} mb={1}>
          <Stack spacing={0.25}>
            <Typography variant="h6">Goals</Typography>
            <Typography variant="caption" color="text.secondary">
              Track outcomes and spin up AI autopilot loops when needed.
            </Typography>
          </Stack>
          <Stack direction="row" spacing={1}>
            <Button size="small" variant="outlined" onClick={() => openGoalDialog(true)}>
              Create Goal
            </Button>
          </Stack>
        </Stack>
      </Box>

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 110 }}><Typography variant="caption" color="text.secondary">Autopilot Items</Typography><Typography variant="h5">{num(summary.total)}</Typography><Typography variant="caption" color="text.secondary">Recent tasks tied to goals</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 110 }}><Typography variant="caption" color="text.secondary">Completed</Typography><Typography variant="h5">{num(summary.completed)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 110 }}><Typography variant="caption" color="text.secondary">Pending/Running</Typography><Typography variant="h5">{num(summary.pending_or_running)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 110 }}><Typography variant="caption" color="text.secondary">Failed</Typography><Typography variant="h5">{num(summary.failed)}</Typography></Box></Grid2>
      </Grid2>

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 6 }}>
          <Box className="list-shell">
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">Goals</Typography>
            </Stack>
            {goalsQ.error ? (
              <Alert severity="error">{errMessage(goalsQ.error)}</Alert>
            ) : goals.length === 0 ? (
              <Typography variant="body2" color="text.secondary">No goals yet.</Typography>
            ) : (
              <Box className="metadata-box" sx={{ maxHeight: 520 }}>
                <Stack spacing={1}>
                  {goals.map((g) => {
                    const id = str(g.id, "");
                    const goalId = str(g.goal_id, "");
                    const hasAutopilot = g.autopilot === true && !!goalId;
                    const isSelected = hasAutopilot && selectedGoalId === goalId;
                    const title = str(g.goal, "").trim() || str(g.description, "Goal").replace(/^Goal:\\s*/i, "");
                    return (
                      <Box key={id} className="action-row">
                        <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={1}>
                          <Button
                            variant="text"
                            size="small"
                            sx={{
                              justifyContent: "flex-start",
                              textAlign: "left",
                              flex: 1,
                              ...(isSelected
                                ? {
                                    border: "1px solid rgba(47,212,255,0.35)",
                                    background: "rgba(47,212,255,0.08)"
                                  }
                                : {})
                            }}
                            onClick={() => setSelectedGoalId(hasAutopilot ? (isSelected ? null : goalId) : null)}
                          >
                            <Stack alignItems="flex-start" spacing={0.3}>
                              <Stack direction="row" spacing={1} alignItems="center">
                                <Typography variant="body2" fontWeight={700}>{title}</Typography>
                                {hasAutopilot ? <Chip size="small" label="Autopilot" /> : <Chip size="small" label="Manual" variant="outlined" />}
                              </Stack>
                              <Typography variant="caption" color="text.secondary">
                                {str(g.status)}{str(g.due_date) ? ` | due ${str(g.due_date)}` : ""}{str(g.created_at) ? <span title={humanTs(str(g.created_at)).tip}>{` | created ${humanTs(str(g.created_at)).label}`}</span> : ""}
                              </Typography>
                            </Stack>
                          </Button>
                          <Stack direction="row" spacing={1} alignItems="center">
                            {!hasAutopilot ? (
                              <Button
                                size="small"
                                disabled={autopilotMutation.isPending}
                                onClick={async () => {
                                  setError(null);
                                  setPlanPreview(null);
                                  try {
                                    const out = await autopilotMutation.mutateAsync({
                                      goal: title,
                                      due_date: str(g.due_date) || undefined,
                                      constraints: guardrails.trim() || undefined,
                                      report_cron: reportCron.trim() || undefined
                                    });
                                    const newGoalId = str(asRecord(out).goal_id, "");
                                    if (newGoalId) setSelectedGoalId(newGoalId);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                }}
                              >
                                Start Autopilot
                              </Button>
                            ) : (
                              <Button size="small" onClick={() => setSelectedGoalId(isSelected ? null : goalId)}>
                                {isSelected ? "Deselect" : "View"}
                              </Button>
                            )}
                            <Button size="small" color="error" disabled={deleteMutation.isPending} onClick={() => deleteMutation.mutate(id)}>
                              Delete
                            </Button>
                          </Stack>
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            )}
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 6 }}>
          <Box className="list-shell">
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">{selectedGoalId ? "Autopilot Activity (selected goal)" : "Autopilot Activity (all goals)"}</Typography>
              <Stack direction="row" spacing={1} alignItems="center">
                {selectedGoalId ? (
                  <Button size="small" disabled={runNowMutation.isPending} onClick={() => runNowMutation.mutate(selectedGoalId)}>
                    Run now
                  </Button>
                ) : null}
                {selectedGoalId ? <Button size="small" onClick={() => setSelectedGoalId(null)}>Clear</Button> : null}
              </Stack>
            </Stack>
            {progressQ.error ? (
              <Alert severity="error">{errMessage(progressQ.error)}</Alert>
            ) : progressItems.length === 0 ? (
              <Typography variant="body2" color="text.secondary">No goal-linked items yet.</Typography>
            ) : (
              <Box className="metadata-box" sx={{ maxHeight: 520 }}>
                <Stack spacing={1}>
                  {progressItems.map((it) => {
                    const id = str(it.id, "");
                    const status = str(it.status, "");
                    const statusColor = status.includes("Failed") ? "error" : status.includes("Completed") ? "success" : "warning";
                    return (
                      <Box key={id} className="action-row">
                        <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={1}>
                          <Stack spacing={0.3} sx={{ minWidth: 0 }}>
                            <Typography variant="body2" fontWeight={700} noWrap>{str(it.description, "Task")}</Typography>
                            <Typography variant="caption" color="text.secondary" noWrap>
                              {str(it.action)} | <span title={humanTs(str(it.created_at)).tip}>{humanTs(str(it.created_at)).label}</span>
                            </Typography>
                          </Stack>
                          <Chip size="small" label={status || "Unknown"} color={statusColor as any} />
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            )}
          </Box>
        </Grid2>
      </Grid2>

      {error ? <Alert severity="error">{error}</Alert> : null}

      <Dialog open={goalCreateOpen} onClose={() => setGoalCreateOpen(false)} maxWidth="md" fullWidth>
        <DialogTitle>Set a Goal</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Stack direction={{ xs: "column", sm: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", sm: "center" }}>
              <Typography variant="caption" color="text.secondary">
                Use plain language. Autopilot enables AI planning and scheduled progress loops.
              </Typography>
              <FormControlLabel
                control={<Switch checked={autopilotEnabled} onChange={(e) => setAutopilotEnabled(e.target.checked)} />}
                label="Autopilot"
              />
            </Stack>
            <Grid2 container spacing={1} alignItems="stretch">
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  label="What do you want to achieve?"
                  placeholder="Describe your goal in one sentence."
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField fullWidth label="Due date (optional)" placeholder="YYYY-MM-DD" value={dueDate} onChange={(e) => setDueDate(e.target.value)} />
              </Grid2>
              {autopilotEnabled ? (
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Guardrails (optional)"
                    placeholder="Example: Ask before deleting files. Keep it under 3 steps. No external posting."
                    value={guardrails}
                    onChange={(e) => setGuardrails(e.target.value)}
                  />
                </Grid2>
              ) : null}
              {autopilotEnabled ? (
                <Grid2 size={{ xs: 12 }}>
                  <Accordion expanded={advancedOpen} onChange={() => setAdvancedOpen((p) => !p)} className="accordion-shell">
                    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>Advanced</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Stack spacing={1}>
                        <TextField
                          fullWidth
                          size="small"
                          select
                          label="Check-in schedule"
                          value={scheduleKey}
                          onChange={(e) => {
                            const next = e.target.value;
                            setScheduleKey(next);
                            let preset: (typeof schedulePresets)[number] | undefined = undefined;
                            for (const p of schedulePresets) {
                              if (p.key === next) {
                                preset = p;
                                break;
                              }
                            }
                            if (preset && preset.cron) setReportCron(preset.cron);
                          }}
                          helperText="When Autopilot is enabled, this schedules a periodic progress report task."
                        >
                          {schedulePresets.map((p) => (
                            <MenuItem key={p.key} value={p.key}>
                              {p.label}
                            </MenuItem>
                          ))}
                        </TextField>
                        {scheduleKey === "custom" ? (
                          <TextField
                            fullWidth
                            size="small"
                            label="Custom cron (6 fields)"
                            value={reportCron}
                            onChange={(e) => setReportCron(e.target.value)}
                            helperText={(() => {
                              for (const p of schedulePresets) {
                                if (p.key === "custom") return p.hint || "";
                              }
                              return "";
                            })()}
                          />
                        ) : (
                          <Typography variant="caption" color="text.secondary">
                            Selected: {scheduleLabel(scheduleKey)} ({reportCron})
                          </Typography>
                        )}
                      </Stack>
                    </AccordionDetails>
                  </Accordion>
                </Grid2>
              ) : null}
            </Grid2>
            <Stack direction="row" spacing={1} flexWrap="wrap" sx={{ opacity: 0.9 }}>
              {examples.map((ex) => (
                <Chip
                  key={ex}
                  size="small"
                  label={ex}
                  onClick={() => setDescription(ex)}
                  variant="outlined"
                  sx={{ mb: 0.5 }}
                />
              ))}
            </Stack>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setGoalCreateOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            startIcon={
              autopilotEnabled && autopilotPreviewMutation.isPending ? (
                <CircularProgress size={14} color="inherit" />
              ) : undefined
            }
            disabled={
              !description.trim() ||
              createMutation.isPending ||
              autopilotMutation.isPending ||
              autopilotPreviewMutation.isPending
            }
            onClick={submitGoalDraft}
          >
            {autopilotEnabled
              ? autopilotPreviewMutation.isPending
                ? "Generating..."
                : "Create with AI"
              : "Save Goal"}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog
        open={goalConfirmOpen}
        onClose={() => {
          if (autopilotMutation.isPending) return;
          setGoalConfirmOpen(false);
        }}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Confirm Goal Before Create</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Alert severity="info">
              AI has prepared a draft. Review and edit details before creating this goal.
            </Alert>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  label="Goal"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  label="Due date (optional)"
                  placeholder="YYYY-MM-DD"
                  value={dueDate}
                  onChange={(e) => setDueDate(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Guardrails (optional)"
                  value={guardrails}
                  onChange={(e) => setGuardrails(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Report cron"
                  value={reportCron}
                  onChange={(e) => setReportCron(e.target.value)}
                  helperText="6-field cron expression for periodic progress reports."
                />
              </Grid2>
            </Grid2>

            {planPreview ? (
              <Stack spacing={1}>
                <TextField
                  fullWidth
                  size="small"
                  label="AI summary"
                  value={str(planPreview.summary, "")}
                  onChange={(e) =>
                    setPlanPreview((prev) => (prev ? { ...prev, summary: e.target.value } : prev))
                  }
                />

                {Array.isArray(planPreview.steps) && planPreview.steps.length > 0 ? (
                  <Stack spacing={1}>
                    {(planPreview.steps as unknown[]).slice(0, 12).map((rawStep, idx) => {
                      const step = asRecord(rawStep);
                      const args = asRecord(step.arguments);
                      const argKeys = Object.keys(args);
                      const updateStepField = (field: "title" | "action" | "why", value: string) => {
                        setPlanPreview((prev) => {
                          if (!prev) return prev;
                          const currentSteps = Array.isArray(prev.steps) ? [...(prev.steps as unknown[])] : [];
                          const existingStep = asRecord(currentSteps[idx]);
                          currentSteps[idx] = { ...existingStep, [field]: value };
                          return { ...prev, steps: currentSteps };
                        });
                      };
                      return (
                        <Box key={`goal-step-${idx}`} className="action-row">
                          <Stack spacing={0.8}>
                            <Typography variant="caption" color="text.secondary">
                              Step {idx + 1}
                            </Typography>
                            <TextField
                              fullWidth
                              size="small"
                              label="Title"
                              value={str(step.title, `Step ${idx + 1}`)}
                              onChange={(e) => updateStepField("title", e.target.value)}
                            />
                            <TextField
                              fullWidth
                              size="small"
                              label="Action"
                              value={str(step.action, "research")}
                              onChange={(e) => updateStepField("action", e.target.value)}
                            />
                            <TextField
                              fullWidth
                              size="small"
                              label="Why"
                              value={str(step.why, "")}
                              onChange={(e) => updateStepField("why", e.target.value)}
                            />
                            <Typography variant="caption" color="text.secondary">
                              Args: {argKeys.length ? argKeys.join(", ") : "-"}
                            </Typography>
                          </Stack>
                        </Box>
                      );
                    })}
                  </Stack>
                ) : (
                  <Typography variant="body2" color="text.secondary">
                    AI returned no steps. You can still create the goal.
                  </Typography>
                )}
              </Stack>
            ) : (
              <Alert severity="warning">
                AI draft is unavailable. Update fields above and create directly.
              </Alert>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              if (autopilotMutation.isPending) return;
              setGoalConfirmOpen(false);
              setGoalCreateOpen(true);
            }}
          >
            Back
          </Button>
          <Button
            onClick={() => {
              if (autopilotMutation.isPending) return;
              setGoalConfirmOpen(false);
              resetGoalDraft(true);
            }}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            startIcon={autopilotMutation.isPending ? <CircularProgress size={14} color="inherit" /> : undefined}
            disabled={autopilotMutation.isPending || !description.trim()}
            onClick={confirmAutopilotGoal}
          >
            {autopilotMutation.isPending ? "Creating..." : "Confirm & Create"}
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}

function AutonomyManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [autonomyMode, setAutonomyMode] = useState<"off" | "assist" | "auto">("assist");
  const [alwaysAskHighRisk, setAlwaysAskHighRisk] = useState(true);
  const [onlyApprovedSkills, setOnlyApprovedSkills] = useState(true);
  const [quietHoursStart, setQuietHoursStart] = useState("");
  const [quietHoursEnd, setQuietHoursEnd] = useState("");
  const [dailyRunLimit, setDailyRunLimit] = useState("40");
  const [settingsHydrated, setSettingsHydrated] = useState(false);

  const [incidentResult, setIncidentResult] = useState<JsonRecord | null>(null);
  const [rollingBackEventId, setRollingBackEventId] = useState<string | null>(null);

  const [triageLabelsCsv, setTriageLabelsCsv] = useState("Act now, Delegate, Ignore");
  const [triageMessagesJson, setTriageMessagesJson] = useState("");
  const [triageResult, setTriageResult] = useState<JsonRecord | null>(null);

  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [sessionResponse, setSessionResponse] = useState("");
  const [browserRespondResult, setBrowserRespondResult] = useState<JsonRecord | null>(null);
  const [suggestionRun, setSuggestionRun] = useState<SuggestionRunState | null>(null);
  const [suggestionRunOpen, setSuggestionRunOpen] = useState(false);
  const [suggestionRunMinimized, setSuggestionRunMinimized] = useState(false);
  const [activeSuggestionActionId, setActiveSuggestionActionId] = useState<string | null>(null);

  const settingsQ = useQuery({
    queryKey: ["autonomy-settings"],
    queryFn: () => api.rawGet("/autonomy/settings")
  });
  const briefingQ = useQuery({
    queryKey: ["autonomy-briefing"],
    queryFn: () => api.rawGet("/autonomy/briefing"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const notificationsQ = useQuery({
    queryKey: ["autonomy-unread-notifications"],
    queryFn: () => api.rawGet("/notifications?unread=true&limit=120"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const incidentsQ = useQuery({
    queryKey: ["autonomy-incidents-live"],
    queryFn: () => api.rawGet("/autonomy/incidents/live"),
    enabled: showAdvanced,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const timelineQ = useQuery({
    queryKey: ["autonomy-timeline"],
    queryFn: () => api.rawGet("/autonomy/timeline?limit=120"),
    enabled: showAdvanced && SHOW_EXPERIMENTAL_AUTONOMY_TOOLS,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const browserSessionsQ = useQuery({
    queryKey: ["autonomy-browser-sessions"],
    queryFn: () => api.rawGet("/browser/sessions"),
    enabled: showAdvanced,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const browserStatusQ = useQuery({
    queryKey: ["autonomy-browser-session-status", selectedSessionId],
    queryFn: () => api.rawGet(`/browser/sessions/${encodeURIComponent(selectedSessionId)}/status`),
    enabled: showAdvanced && !!selectedSessionId,
    refetchInterval: autoRefresh && !!selectedSessionId ? REFRESH_MS : false
  });
  const suggestionTraceId = suggestionRun?.traceId || "";
  const suggestionDetailId = suggestionRun?.suggestionId || "";
  const suggestionTraceQ = useQuery({
    queryKey: ["autonomy-suggestion-trace", suggestionTraceId],
    queryFn: () => api.rawGet(`/trace/${encodeURIComponent(suggestionTraceId)}`),
    enabled: !!suggestionTraceId && suggestionRunOpen,
    refetchInterval:
      suggestionRunOpen && !!suggestionTraceId && suggestionRun?.status === "running" ? REFRESH_MS : false
  });
  const suggestionDetailQ = useQuery({
    queryKey: ["autonomy-suggestion-detail", suggestionDetailId],
    queryFn: () => api.rawGet(`/autonomy/suggestions/${encodeURIComponent(suggestionDetailId)}`),
    enabled: !!suggestionDetailId && suggestionRunOpen,
    refetchInterval:
      suggestionRunOpen && !!suggestionDetailId && suggestionRun?.status === "running" ? REFRESH_MS : false
  });

  const saveAutonomySettingsMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/autonomy/settings", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
    }
  });
  const executeIncidentMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/autonomy/incidents/${encodeURIComponent(id)}/execute`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-incidents-live"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-timeline"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });
  const rollbackMutation = useMutation({
    mutationFn: (payload: { event_id: string; operation?: string }) => api.rawPost("/autonomy/timeline/rollback", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-timeline"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
    }
  });
  const triageMutation = useMutation({
    mutationFn: (payload: { labels?: string[]; messages: unknown[] }) => api.rawPost("/autonomy/inbox/triage", payload)
  });
  const browserRespondMutation = useMutation({
    mutationFn: (payload: { id: string; response: string }) =>
      api.rawPost(`/browser/sessions/${encodeURIComponent(payload.id)}/respond`, { response: payload.response }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-browser-sessions"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-browser-session-status", selectedSessionId] });
    }
  });
  const acceptSuggestionMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/autonomy/suggestions/${encodeURIComponent(id)}/accept`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });
  const dismissSuggestionMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/autonomy/suggestions/${encodeURIComponent(id)}/dismiss`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
    }
  });

  const incidents = pickRecords(incidentsQ.data, "incidents");
  const timelineEvents = pickRecords(timelineQ.data, "events");
  const triageRows = pickRecords(triageResult, "triage");
  const browserSessions = pickRecords(browserSessionsQ.data, "sessions");
  const browserStatus = asRecord(browserStatusQ.data);
  const suggestionTrace = asRecord(suggestionTraceQ.data);
  const suggestionTraceSteps = pickRecords(suggestionTraceQ.data, "steps");
  const suggestionDetail = asRecord(asRecord(suggestionDetailQ.data).suggestion);
  const suggestionAcceptedOutcomes = pickRecords(suggestionDetail, "accepted_outcomes");

  function severityChipColor(sev: string): "error" | "warning" | "info" | "success" | "default" {
    const s = (sev || "").toLowerCase();
    if (s === "critical" || s === "high" || s === "error") return "error";
    if (s === "medium" || s === "warn" || s === "warning") return "warning";
    if (s === "low") return "info";
    if (s === "ok" || s === "info") return "success";
    return "default";
  }

  function parseCsv(value: string): string[] {
    return value
      .split(",")
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
  }

  function parseTriageMessages(value: string): unknown[] {
    const trimmed = value.trim();
    if (!trimmed) return [];
    const parsed: unknown = JSON.parse(trimmed);
    if (!Array.isArray(parsed)) {
      throw new Error("Messages JSON must be an array.");
    }
    return parsed;
  }

  function effectiveRollbackOperation(operation: string, status: string): string {
    if (operation !== "toggle_notification_read") return operation;
    return status.toLowerCase() === "read" ? "mark_unread" : "mark_read";
  }

  function rollbackLabel(operation: string): string {
    const op = (operation || "").toLowerCase();
    if (op === "cancel_task") return "Cancel task";
    if (op === "cancel_watcher") return "Cancel watcher";
    if (op === "mark_unread") return "Mark unread";
    if (op === "mark_read") return "Mark read";
    if (op === "toggle_notification_read") return "Toggle read";
    return "Rollback";
  }

  const settingsRecord = asRecord(asRecord(settingsQ.data).settings);
  const briefingRecord = asRecord(briefingQ.data);
  const queueSummary = asRecord(asRecord(briefingRecord.trust_summary).queue);
  const topRisks = pickRecords(briefingRecord, "top_risks");
  const suggestedAutomations = pickRecords(briefingRecord, "suggested_automations");
  const suggestionScan = asRecord(briefingRecord.suggestion_scan);
  const attentionRisks = topRisks.filter((risk) => {
    const hay = `${str(risk.type, "")} ${str(risk.title, "")} ${str(risk.detail, "")}`.toLowerCase();
    return !(
      hay.includes("arkpulse") ||
      hay.includes("auth-related security events") ||
      hay.includes("security events were logged")
    );
  });
  const unreadNotifications = pickRecords(notificationsQ.data, "notifications");
  const awaitingApprovals = num(queueSummary.awaiting_approval, 0);
  const missingInputs = unreadNotifications.filter((row) => {
    const source = str(row.source, "").toLowerCase();
    const title = str(row.title, "").toLowerCase();
    const body = str(row.body, "").toLowerCase();
    return (
      source === "workflow_inputs" ||
      title.includes("missing input") ||
      body.includes("missing input") ||
      title.includes("required input") ||
      body.includes("required input")
    );
    }).length;
  const suggestionScanStatus = str(suggestionScan.last_status, "scheduled");
  const suggestionScanLabel =
    suggestionScanStatus === "completed"
      ? "Ready"
      : suggestionScanStatus === "deferred_busy"
      ? "Deferred"
      : suggestionScanStatus === "running"
      ? "Scanning"
      : suggestionScanStatus === "no_user_chat"
      ? "Waiting for chat"
      : suggestionScanStatus === "error"
      ? "Needs attention"
      : "Scheduled";
  const modeIndicator = autonomyMode === "auto" ? "Auto" : autonomyMode === "assist" ? "Assist" : "Off";
  const timelineTabIndex = 1;
  const triageTabIndex = 2;
  const browserTabIndex = SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? 3 : 1;
  const waitingStatusLine =
    awaitingApprovals === 0 && missingInputs === 0
      ? `Mode: ${modeIndicator} | You're all set. Nothing is waiting on you.`
      : `Mode: ${modeIndicator} | Waiting on you: ${awaitingApprovals} approval${awaitingApprovals === 1 ? "" : "s"}, ${missingInputs} required input${missingInputs === 1 ? "" : "s"}`;
  const modePlainHint =
    autonomyMode === "off"
      ? "You start everything manually."
      : autonomyMode === "assist"
      ? "Agent prepares work and asks before sensitive actions."
      : "Agent runs allowed work automatically and only asks when required.";
  const configuredModeRaw = str(settingsRecord.autonomy_mode, "assist").toLowerCase();
  const configuredMode: "off" | "assist" | "auto" =
    configuredModeRaw === "off" || configuredModeRaw === "auto" || configuredModeRaw === "assist"
      ? configuredModeRaw
      : "assist";
  const configuredAlwaysAskHighRisk = Boolean(settingsRecord.always_ask_high_risk ?? true);
  const configuredOnlyApprovedSkills = Boolean(settingsRecord.only_approved_skills ?? true);
  const configuredQuietHoursStart = str(settingsRecord.quiet_hours_start, "").trim();
  const configuredQuietHoursEnd = str(settingsRecord.quiet_hours_end, "").trim();
  const configuredDailyRunLimit =
    typeof settingsRecord.daily_run_limit === "number" && Number.isFinite(settingsRecord.daily_run_limit)
      ? Math.round(settingsRecord.daily_run_limit)
      : null;
  const normalizedQuietHoursStart = quietHoursStart.trim();
  const normalizedQuietHoursEnd = quietHoursEnd.trim();
  const normalizedLimitText = dailyRunLimit.trim();
  let parsedLimitForUi: number | null = null;
  let dailyRunLimitInvalid = false;
  if (normalizedLimitText.length > 0) {
    const n = Number(normalizedLimitText);
    if (!Number.isFinite(n) || n < 1) {
      dailyRunLimitInvalid = true;
    } else {
      parsedLimitForUi = Math.round(n);
    }
  }
  const guardrailsDirty =
    settingsHydrated &&
    (autonomyMode !== configuredMode ||
      alwaysAskHighRisk !== configuredAlwaysAskHighRisk ||
      onlyApprovedSkills !== configuredOnlyApprovedSkills ||
      normalizedQuietHoursStart !== configuredQuietHoursStart ||
      normalizedQuietHoursEnd !== configuredQuietHoursEnd ||
      parsedLimitForUi !== configuredDailyRunLimit);

  function openSettingsTab(tabName: string) {
    const nextPath = "/ui/settings";
    const nextSearch = `?settings_tab=${encodeURIComponent(tabName)}`;
    const nextUrl = `${nextPath}${nextSearch}`;
    const current = `${window.location.pathname}${window.location.search}`;
    if (current !== nextUrl) {
      window.history.pushState(null, "", nextUrl);
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }

  function recommendedTabForRisk(risk: JsonRecord): string {
    const bag = `${str(risk.type, "")} ${str(risk.title, "")} ${str(risk.detail, "")}`.toLowerCase();
    if (bag.includes("auth") || bag.includes("security")) return "security";
    return "system";
  }

  function suggestionKindColor(kind: string): "default" | "success" | "warning" | "info" | "error" {
    const normalized = kind.toLowerCase();
    if (normalized === "watcher") return "info";
    if (normalized === "app") return "success";
    if (normalized === "workflow") return "warning";
    if (normalized === "task") return "default";
    return "default";
  }

  function openWorkspacePanel(view: string) {
    const normalized = (view || "").toLowerCase();
    const path =
      normalized === "apps"
        ? "/ui/apps"
        : normalized === "tasks"
          ? "/ui/tasks"
          : normalized === "watchers"
            ? "/ui/watchers"
            : "";
    if (!path) return;
    if (window.location.pathname !== path) {
      window.history.pushState(null, "", path);
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }

  async function runSuggestionAccept(suggestion: JsonRecord) {
    const suggestionId = str(suggestion.id, "");
    if (!suggestionId) return;
    const title = str(suggestion.title, "Suggested automation");
    setError(null);
    setSuccess(null);
    setActiveSuggestionActionId(suggestionId);
    setSuggestionRun({
      title,
      status: "running",
      summary: "Launching real execution run...",
      startedAt: new Date().toISOString(),
      suggestionId
    });
    setSuggestionRunOpen(true);
    setSuggestionRunMinimized(false);

    try {
      const response = asRecord(await acceptSuggestionMutation.mutateAsync(suggestionId));
      const run = asRecord(response.run);
      setSuggestionRun({
        title: str(run.title, title),
        status: "running",
        summary: str(run.summary, "Suggestion run started."),
        traceId: str(response.trace_id, str(run.trace_id, "")),
        startedAt: str(run.started_at, ""),
        suggestionId
      });
      setSuccess("Suggestion run started.");
    } catch (e) {
      const message = errMessage(e);
      setSuggestionRun((current) => ({
        title: current?.title || title,
        status: "error",
        summary: message,
        traceId: current?.traceId,
        startedAt: current?.startedAt,
        completedAt: new Date().toISOString(),
        suggestionId
      }));
      setSuccess(null);
      setError(message);
    } finally {
      setActiveSuggestionActionId(null);
    }
  }

  async function runSuggestionDismiss(suggestion: JsonRecord) {
    const suggestionId = str(suggestion.id, "");
    if (!suggestionId) return;
    setError(null);
    setSuccess(null);
    setActiveSuggestionActionId(suggestionId);
    try {
      await dismissSuggestionMutation.mutateAsync(suggestionId);
      setSuccess("Suggestion dismissed.");
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setActiveSuggestionActionId(null);
    }
  }

  useEffect(() => {
    if (!suggestionRun?.traceId) return;
    if (suggestionTraceQ.isLoading || suggestionTraceQ.error || !Object.keys(suggestionTrace).length) return;
    const traceStatusRaw = str(suggestionTrace.status, suggestionRun.status).toLowerCase();
    const lastSuggestionStep = asRecord(suggestionTraceSteps[suggestionTraceSteps.length - 1]);
    const lastSuggestionConsoleView = buildTraceStepConsoleView(
      suggestionTrace,
      suggestionTraceSteps,
      lastSuggestionStep
    );
    const nextStatus: "running" | "completed" | "error" =
      traceStatusRaw === "failed" || traceStatusRaw === "error" || traceStatusRaw === "warning"
        ? "error"
        : traceStatusRaw === "completed"
        ? "completed"
        : "running";
    const nextSummary =
      str(suggestionTrace.response, "").trim() ||
      lastSuggestionConsoleView.detail ||
      suggestionRun.summary;
    const nextCompletedAt = str(suggestionTrace.completed_at, suggestionRun.completedAt || "");
    const nextStartedAt = str(suggestionTrace.started_at, suggestionRun.startedAt || "");
    if (
      suggestionRun.status !== nextStatus ||
      suggestionRun.summary !== nextSummary ||
      suggestionRun.completedAt !== nextCompletedAt ||
      suggestionRun.startedAt !== nextStartedAt
    ) {
      setSuggestionRun((current) =>
        current
          ? {
              ...current,
              status: nextStatus,
              summary: nextSummary,
              startedAt: nextStartedAt || current.startedAt,
              completedAt: nextCompletedAt || current.completedAt
            }
          : current
      );
    }
  }, [suggestionRun, suggestionTrace, suggestionTraceQ.isLoading, suggestionTraceQ.error, suggestionTraceSteps]);

  useEffect(() => {
    if (!suggestionRun?.suggestionId) return;
    if (suggestionDetailQ.isLoading || suggestionDetailQ.error || !Object.keys(suggestionDetail).length) return;
    const runStatusRaw = str(suggestionDetail.run_status, suggestionRun.status).toLowerCase();
    const nextStatus: "running" | "completed" | "error" =
      runStatusRaw === "failed" || runStatusRaw === "error"
        ? "error"
        : runStatusRaw === "completed"
          ? "completed"
          : "running";
    const outcomeTitles = suggestionAcceptedOutcomes
      .map((row) => str(row.title, "").trim())
      .filter(Boolean);
    const outcomeSummary = outcomeTitles.length
      ? `Saved ${suggestionAcceptedOutcomes.length} outcome${suggestionAcceptedOutcomes.length === 1 ? "" : "s"}: ${outcomeTitles.slice(0, 3).join(", ")}${outcomeTitles.length > 3 ? ` (+${outcomeTitles.length - 3} more)` : ""}`
      : "";
    const currentSummary = str(suggestionRun.summary, "").trim();
    const genericSummary =
      !currentSummary ||
      currentSummary === "Launching real execution run..." ||
      currentSummary === "Suggestion run started." ||
      currentSummary.startsWith("Launched a real ");
    const nextSummary =
      str(suggestionDetail.last_run_error, "").trim() ||
      (nextStatus === "completed" && outcomeSummary && genericSummary ? outcomeSummary : suggestionRun.summary);
    const nextCompletedAt = str(suggestionDetail.last_run_completed_at, suggestionRun.completedAt || "");
    const nextStartedAt = str(suggestionDetail.last_run_started_at, suggestionRun.startedAt || "");
    if (
      suggestionRun.status !== nextStatus ||
      suggestionRun.summary !== nextSummary ||
      suggestionRun.completedAt !== nextCompletedAt ||
      suggestionRun.startedAt !== nextStartedAt
    ) {
      setSuggestionRun((current) =>
        current
          ? {
              ...current,
              status: nextStatus,
              summary: nextSummary,
              startedAt: nextStartedAt || current.startedAt,
              completedAt: nextCompletedAt || current.completedAt
            }
          : current
      );
    }
  }, [suggestionRun, suggestionDetail, suggestionDetailQ.isLoading, suggestionDetailQ.error, suggestionAcceptedOutcomes]);

  useEffect(() => {
    if (settingsHydrated) return;
    if (!Object.keys(settingsRecord).length) return;
    const rawMode = str(settingsRecord.autonomy_mode, "assist").toLowerCase();
    if (rawMode === "off" || rawMode === "auto" || rawMode === "assist") {
      setAutonomyMode(rawMode);
    } else {
      setAutonomyMode("assist");
    }
    setAlwaysAskHighRisk(Boolean(settingsRecord.always_ask_high_risk ?? true));
    setOnlyApprovedSkills(Boolean(settingsRecord.only_approved_skills ?? true));
    setQuietHoursStart(str(settingsRecord.quiet_hours_start, ""));
    setQuietHoursEnd(str(settingsRecord.quiet_hours_end, ""));
    const configuredLimit = settingsRecord.daily_run_limit;
    if (typeof configuredLimit === "number" && Number.isFinite(configuredLimit)) {
      setDailyRunLimit(String(configuredLimit));
    } else {
      setDailyRunLimit("");
    }
    setSettingsHydrated(true);
  }, [settingsHydrated, settingsRecord]);

  useEffect(() => {
    if (!showAdvanced) return;
    const maxAllowedTab = SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? 3 : 1;
    if (tab > maxAllowedTab) {
      setTab(0);
    }
  }, [showAdvanced, tab]);

  async function saveBeginnerAutonomySettings(modeOverride?: "off" | "assist" | "auto") {
    setError(null);
    setSuccess(null);
    const selectedMode = modeOverride ?? autonomyMode;
    const normalizedLimit = dailyRunLimit.trim();
    let parsedLimit: number | null = null;
    if (normalizedLimit.length > 0) {
      const n = Number(normalizedLimit);
      if (!Number.isFinite(n) || n < 1) {
        setError("Daily run limit must be a positive number.");
        return;
      }
      parsedLimit = Math.round(n);
    }
    try {
      await saveAutonomySettingsMutation.mutateAsync({
        autonomy_mode: selectedMode,
        always_ask_high_risk: alwaysAskHighRisk,
        only_approved_skills: onlyApprovedSkills,
        quiet_hours_start: quietHoursStart.trim() || null,
        quiet_hours_end: quietHoursEnd.trim() || null,
        daily_run_limit: parsedLimit
      });
      setSuccess("Autonomy settings saved.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack spacing={1.25}>
          <Typography variant="h6">Automation Mode</Typography>
          <Typography variant="caption" color="text.secondary">
            Choose how hands-off you want this agent to be. Anything that needs your decision appears here.
          </Typography>
          <Alert severity="info" sx={{ py: 0.75 }}>
            {waitingStatusLine}
          </Alert>
          <Typography variant="body2" color="text.secondary">
            {modePlainHint}
          </Typography>
          <Stack direction={{ xs: "column", md: "row" }} spacing={1}>
            <Button
              variant={autonomyMode === "off" ? "contained" : "outlined"}
              onClick={async () => {
                setAutonomyMode("off");
                await saveBeginnerAutonomySettings("off");
              }}
              disabled={saveAutonomySettingsMutation.isPending}
            >
              Off
            </Button>
            <Button
              variant={autonomyMode === "assist" ? "contained" : "outlined"}
              onClick={async () => {
                setAutonomyMode("assist");
                await saveBeginnerAutonomySettings("assist");
              }}
              disabled={saveAutonomySettingsMutation.isPending}
            >
              Assist (Recommended)
            </Button>
            <Button
              variant={autonomyMode === "auto" ? "contained" : "outlined"}
              onClick={async () => {
                setAutonomyMode("auto");
                await saveBeginnerAutonomySettings("auto");
              }}
              disabled={saveAutonomySettingsMutation.isPending}
            >
              Auto
            </Button>
          </Stack>
          <Box component="ul" sx={{ m: 0, pl: 2, color: "text.secondary" }}>
            <Typography component="li" variant="caption" color="text.secondary">
              Off: You review and start every run manually.
            </Typography>
            <Typography component="li" variant="caption" color="text.secondary">
              Assist (recommended): Agent drafts work first, then asks before sensitive steps.
            </Typography>
            <Typography component="li" variant="caption" color="text.secondary">
              Auto: Agent proceeds end-to-end within your safety limits.
            </Typography>
            <Typography component="li" variant="caption" color="text.secondary">
              Tip: Keep Assist on until you're comfortable with Auto.
            </Typography>
          </Box>
          {!showAdvanced ? (
            <Stack spacing={1}>
              <Alert severity="success" sx={{ py: 0.75 }}>
                Beginner safety defaults are active: ask before risky actions, approved skills only, and daily run cap.
              </Alert>
              <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
                <Button
                  color="warning"
                  variant="outlined"
                  onClick={async () => {
                    setAutonomyMode("off");
                    await saveBeginnerAutonomySettings("off");
                  }}
                  disabled={saveAutonomySettingsMutation.isPending}
                >
                  Turn Off
                </Button>
                  <Button size="small" onClick={() => { setTab(0); setShowAdvanced(true); }}>
                    Show developer mode
                  </Button>
              </Stack>
            </Stack>
          ) : (
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12 }}>
                <Alert severity="info" sx={{ py: 0.75 }}>
                  Developer mode: full controls for safety policies, limits, and advanced automation tools.
                </Alert>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={alwaysAskHighRisk}
                      onChange={(e) => setAlwaysAskHighRisk(e.target.checked)}
                    />
                  }
                  label="Ask me before risky actions"
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={onlyApprovedSkills}
                      onChange={(e) => setOnlyApprovedSkills(e.target.checked)}
                    />
                  }
                  label="Use only approved skills"
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  type="time"
                  label="Quiet hours start (local)"
                  value={quietHoursStart}
                  onChange={(e) => setQuietHoursStart(e.target.value)}
                  InputLabelProps={{ shrink: true }}
                  helperText="Agent avoids starting new runs after this time."
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  type="time"
                  label="Quiet hours end (local)"
                  value={quietHoursEnd}
                  onChange={(e) => setQuietHoursEnd(e.target.value)}
                  InputLabelProps={{ shrink: true }}
                  helperText="Agent resumes normal runs after this time."
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  type="number"
                  label="Daily run limit"
                  value={dailyRunLimit}
                  onChange={(e) => setDailyRunLimit(e.target.value)}
                  inputProps={{ min: 1, max: 1000 }}
                  error={dailyRunLimitInvalid}
                  helperText={
                    dailyRunLimitInvalid
                      ? "Enter a positive number (1 or more), or leave blank."
                      : "Safety cap for total runs each day. Leave blank for no cap."
                  }
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
                  <Button
                    variant="contained"
                    onClick={() => saveBeginnerAutonomySettings()}
                    disabled={
                      saveAutonomySettingsMutation.isPending ||
                      settingsQ.isFetching ||
                      !guardrailsDirty ||
                      dailyRunLimitInvalid
                    }
                  >
                    {saveAutonomySettingsMutation.isPending ? "Saving..." : "Save Safety Settings"}
                  </Button>
                  <Button
                    color="warning"
                    variant="outlined"
                    onClick={async () => {
                      setAutonomyMode("off");
                      await saveBeginnerAutonomySettings("off");
                    }}
                    disabled={saveAutonomySettingsMutation.isPending}
                  >
                    Turn Off
                  </Button>
                  <Button size="small" onClick={() => { setShowAdvanced(false); setTab(0); }}>
                    Hide developer mode
                  </Button>
                </Stack>
              </Grid2>
            </Grid2>
          )}
        </Stack>
      </Box>

      <Box className="list-shell">
        <Stack spacing={1.25}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={1} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }}>
            <Box>
              <Typography variant="subtitle2">Suggested Automations</Typography>
              <Typography variant="caption" color="text.secondary">
                Chat is scanned every 12 hours only when the server is quiet. Busy periods defer automatically. Low-signal chat and conversations that already turned into apps are skipped.
              </Typography>
            </Box>
            <Chip
              size="small"
              color={suggestionScanStatus === "error" ? "error" : suggestionScanStatus === "deferred_busy" ? "warning" : suggestionScanStatus === "completed" ? "success" : "default"}
              label={`Scan: ${suggestionScanLabel}`}
            />
          </Stack>
          <Alert severity={suggestionScanStatus === "error" ? "error" : suggestionScanStatus === "deferred_busy" ? "warning" : "info"} sx={{ py: 0.75 }}>
            Last run: {str(suggestionScan.last_completed_at, "") ? humanTs(str(suggestionScan.last_completed_at, "")).label : "Not yet"} | Next run: {str(suggestionScan.next_due_at, "") ? humanTs(str(suggestionScan.next_due_at, "")).label : "Scheduling"} | Batch cap: {num(suggestionScan.last_examined_chats, 0) > 0 ? `${num(suggestionScan.last_examined_chats, 0)} chat(s) last pass` : "12 chats per pass"} | Tracked chats: {num(suggestionScan.tracked_chats, 0)}
          </Alert>
          {suggestedAutomations.length === 0 ? (
            <Typography variant="body2" color="text.secondary">
              No undeployed chat wishes are waiting right now.
            </Typography>
          ) : (
            <Stack spacing={1}>
              {suggestedAutomations.map((suggestion, idx) => {
                const suggestionId = str(suggestion.id, `suggestion-${idx}`);
                const kind = str(suggestion.kind, "automation");
                const busy = activeSuggestionActionId === suggestionId;
                return (
                  <Box key={suggestionId} className="action-row">
                    <Stack spacing={1}>
                      <Stack direction={{ xs: "column", md: "row" }} spacing={1} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }}>
                        <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" alignItems="center">
                          <Chip size="small" color={suggestionKindColor(kind)} label={str(suggestion.kind, "automation")} />
                          <Typography variant="body2" sx={{ fontWeight: 600 }}>
                            {str(suggestion.title, "Suggested automation")}
                          </Typography>
                        </Stack>
                        <Stack direction="row" spacing={1}>
                          <Button
                            size="small"
                            variant="contained"
                            disabled={busy}
                            onClick={() => void runSuggestionAccept(suggestion)}
                          >
                            {busy && acceptSuggestionMutation.isPending ? "Starting..." : "Accept"}
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            color="inherit"
                            disabled={busy}
                            onClick={() => void runSuggestionDismiss(suggestion)}
                          >
                            {busy && dismissSuggestionMutation.isPending ? "Dismissing..." : "Dismiss"}
                          </Button>
                        </Stack>
                      </Stack>
                      <Typography variant="body2" color="text.secondary">
                        {str(suggestion.detail, "")}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Why this was suggested: {str(suggestion.rationale, "")}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Source chat: {str(suggestion.source_snippet, "")}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Accept launches a real execution run, opens a live trace window, and shows step-by-step logs while the agent builds the app, watcher, or workflow.
                      </Typography>
                    </Stack>
                  </Box>
                );
              })}
            </Stack>
          )}
        </Stack>
      </Box>

      {attentionRisks.length > 0 ? (
        <Box className="list-shell">
          <Typography variant="subtitle2" mb={0.75}>Needs Your Attention</Typography>
          <Stack spacing={0.75}>
            {attentionRisks.slice(0, 4).map((risk, idx) => (
              <Stack
                key={`risk-${idx}`}
                direction={{ xs: "column", sm: "row" }}
                spacing={1}
                alignItems={{ xs: "flex-start", sm: "center" }}
                justifyContent="space-between"
                className="action-row"
              >
                <Stack spacing={0.25} sx={{ minWidth: 0 }}>
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {str(risk.title, "Risk")}
                  </Typography>
                  <Typography variant="caption" color="text.secondary" noWrap title={str(risk.detail, "")}>
                    {str(risk.detail, "")}
                  </Typography>
                </Stack>
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => openSettingsTab(recommendedTabForRisk(risk))}
                >
                  Open
                </Button>
              </Stack>
            ))}
          </Stack>
        </Box>
      ) : null}

      {showAdvanced ? (
        <Box className="list-shell">
          <Typography variant="subtitle2" mb={0.5}>Developer Tools</Typography>
          <Tabs
            value={tab}
            onChange={(_, value) => setTab(Number(value) || 0)}
            variant="scrollable"
            scrollButtons="auto"
            allowScrollButtonsMobile
            sx={{ mt: 1 }}
          >
            <Tab label="Live Incidents" value={0} />
            {SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? <Tab label="Timeline & Rollback" value={timelineTabIndex} /> : null}
            {SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? <Tab label="Inbox Triage" value={triageTabIndex} /> : null}
            <Tab label="Browser Sessions" value={browserTabIndex} />
          </Tabs>
          {!SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? (
            <Typography variant="caption" color="text.secondary" sx={{ mt: 0.75, display: "block" }}>
              Timeline rollback and inbox triage are hidden by default to keep this view focused. Specialist agents are selected automatically from chat and saved runs. Configure them in Agents.
            </Typography>
          ) : null}
        </Box>
      ) : null}

      {showAdvanced && tab === 0 ? (
        <Box className="list-shell">
          <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
            <Typography variant="h6">Live Incidents</Typography>
            <Button size="small" onClick={() => queryClient.invalidateQueries({ queryKey: ["autonomy-incidents-live"] })}>
              Refresh
            </Button>
          </Stack>
          {incidentsQ.error ? (
            <Alert severity="error">{errMessage(incidentsQ.error)}</Alert>
          ) : incidents.length === 0 ? (
            <Typography variant="body2" color="text.secondary">No incidents right now.</Typography>
          ) : (
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Severity</TableCell>
                    <TableCell>Title</TableCell>
                    <TableCell>Detail</TableCell>
                    <TableCell>ID</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {incidents.map((incident, idx) => {
                    const id = str(incident.id, `incident-${idx}`);
                    return (
                      <TableRow key={id}>
                        <TableCell>
                          <Chip size="small" label={str(incident.severity, "-")} color={severityChipColor(str(incident.severity, ""))} />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 260 }}>
                          <Typography variant="body2" noWrap title={str(incident.title, "-")}>
                            {str(incident.title, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 420 }}>
                          <Typography variant="body2" noWrap title={str(incident.detail, "-")}>
                            {str(incident.detail, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 180 }}>
                          <Typography variant="caption" color="text.secondary" noWrap title={id}>
                            {id}
                          </Typography>
                        </TableCell>
                        <TableCell align="right">
                          <RowOpsMenu
                            actions={[
                              {
                                label: "Run Playbook",
                                disabled: executeIncidentMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  setSuccess(null);
                                  setIncidentResult(null);
                                  try {
                                    const out = asRecord(await executeIncidentMutation.mutateAsync(id));
                                    setIncidentResult(out);
                                    setSuccess("Incident playbook executed.");
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                }
                              }
                            ]}
                            ariaLabel="Incident options"
                          />
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          )}
          {incidentResult ? (
            <Box sx={{ mt: 1 }}>
              <KeyValuePanel title="Last playbook result" data={incidentResult} />
            </Box>
          ) : null}
        </Box>
      ) : null}

      {showAdvanced && SHOW_EXPERIMENTAL_AUTONOMY_TOOLS && tab === timelineTabIndex ? (
        <Box className="list-shell">
          <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
            <Typography variant="h6">Timeline & Rollback</Typography>
            <Button size="small" onClick={() => queryClient.invalidateQueries({ queryKey: ["autonomy-timeline"] })}>
              Refresh
            </Button>
          </Stack>
          {timelineQ.error ? (
            <Alert severity="error">{errMessage(timelineQ.error)}</Alert>
          ) : timelineEvents.length === 0 ? (
            <Typography variant="body2" color="text.secondary">No timeline events yet.</Typography>
          ) : (
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Time</TableCell>
                    <TableCell>Source</TableCell>
                    <TableCell>Title</TableCell>
                    <TableCell>Status</TableCell>
                    <TableCell>Detail</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {timelineEvents.map((event, idx) => {
                    const eventId = str(event.id, `event-${idx}`);
                    const status = str(event.status, "");
                    const rollback = asRecord(event.rollback);
                    const operation = str(rollback.operation, "");
                    const effectiveOp = effectiveRollbackOperation(operation, status);
                    const canRollback = !!operation && operation !== "none";
                    return (
                      <TableRow key={eventId}>
                        <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(event.timestamp, "-")).tip}>{humanTs(str(event.timestamp, "-")).label}</TableCell>
                        <TableCell>{str(event.source, "-")}</TableCell>
                        <TableCell sx={{ maxWidth: 280 }}>
                          <Typography variant="body2" noWrap title={str(event.title, "-")}>
                            {str(event.title, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell>{status || "-"}</TableCell>
                        <TableCell sx={{ maxWidth: 360 }}>
                          <Typography variant="caption" color="text.secondary" noWrap title={str(event.detail, "-")}>
                            {str(event.detail, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell align="right">
                          {canRollback ? (
                            <RowOpsMenu
                              actions={[
                                {
                                  label: rollingBackEventId === eventId ? "Applying..." : rollbackLabel(effectiveOp || operation),
                                  disabled: rollbackMutation.isPending || rollingBackEventId === eventId,
                                  onClick: async () => {
                                    setError(null);
                                    setSuccess(null);
                                    setRollingBackEventId(eventId);
                                    try {
                                      await rollbackMutation.mutateAsync({
                                        event_id: eventId,
                                        operation: effectiveOp || undefined
                                      });
                                      setSuccess(`Rollback applied: ${rollbackLabel(effectiveOp || operation)}.`);
                                    } catch (e) {
                                      setError(errMessage(e));
                                    } finally {
                                      setRollingBackEventId(null);
                                    }
                                  }
                                }
                              ]}
                              ariaLabel="Timeline event options"
                            />
                          ) : (
                            <Typography variant="caption" color="text.secondary">n/a</Typography>
                          )}
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </Box>
      ) : null}

      {showAdvanced && SHOW_EXPERIMENTAL_AUTONOMY_TOOLS && tab === triageTabIndex ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography variant="h6" mb={1}>Inbox Triage</Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Labels"
                  value={triageLabelsCsv}
                  onChange={(e) => setTriageLabelsCsv(e.target.value)}
                  helperText="Comma-separated labels. Default: Act now, Delegate, Ignore"
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={5}
                  label="Messages JSON (optional)"
                  value={triageMessagesJson}
                  onChange={(e) => setTriageMessagesJson(e.target.value)}
                  placeholder='[{"id":"m1","from":"boss@company.com","subject":"Budget","snippet":"Need approval today"}]'
                  helperText="Leave empty to triage recent notifications automatically."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Button
                  variant="contained"
                  disabled={triageMutation.isPending}
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    setTriageResult(null);
                    try {
                      const out = asRecord(
                        await triageMutation.mutateAsync({
                          labels: parseCsv(triageLabelsCsv),
                          messages: parseTriageMessages(triageMessagesJson)
                        })
                      );
                      setTriageResult(out);
                      setSuccess("Inbox triage complete.");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {triageMutation.isPending ? "Running..." : "Run Triage"}
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            <Typography variant="h6" mb={1}>Triage Results</Typography>
            {triageRows.length === 0 ? (
              <Typography variant="body2" color="text.secondary">Run triage to see classification and draft replies.</Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Message</TableCell>
                      <TableCell>Label</TableCell>
                      <TableCell>Reason</TableCell>
                      <TableCell>Draft Reply</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {triageRows.map((row, idx) => (
                      <TableRow key={str(row.message_id, `triage-${idx}`)}>
                        <TableCell sx={{ maxWidth: 180 }}>
                          <Typography variant="caption" color="text.secondary" noWrap title={str(row.message_id, "-")}>
                            {str(row.message_id, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell>
                          <Chip size="small" label={str(row.label, "-")} variant="outlined" />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 320 }}>
                          <Typography variant="body2" noWrap title={str(row.reason, "-")}>
                            {str(row.reason, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 480 }}>
                          <Typography variant="body2" noWrap title={str(row.draft_reply, "-")}>
                            {str(row.draft_reply, "-")}
                          </Typography>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}

      {showAdvanced && tab === browserTabIndex ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">Browser Sessions</Typography>
              <Button size="small" onClick={() => queryClient.invalidateQueries({ queryKey: ["autonomy-browser-sessions"] })}>
                Refresh
              </Button>
            </Stack>
            {browserSessionsQ.error ? (
              <Alert severity="error">{errMessage(browserSessionsQ.error)}</Alert>
            ) : browserSessions.length === 0 ? (
              <Typography variant="body2" color="text.secondary">No active browser sessions.</Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>ID</TableCell>
                      <TableCell>Task</TableCell>
                      <TableCell>Status</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {browserSessions.map((session, idx) => {
                      const id = str(session.id, `session-${idx}`);
                      return (
                        <TableRow key={id}>
                          <TableCell sx={{ maxWidth: 180 }}>
                            <Typography variant="caption" color="text.secondary" noWrap title={id}>
                              {id}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 360 }}>
                            <Typography variant="body2" noWrap title={str(session.task, "-")}>
                              {str(session.task, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 260 }}>
                            <Typography variant="body2" noWrap title={str(session.status, "-")}>
                              {str(session.status, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Select",
                                  onClick: () => {
                                    setSelectedSessionId(id);
                                    setSessionResponse("");
                                  }
                                },
                                {
                                  label: "Status",
                                  onClick: async () => {
                                    if (selectedSessionId !== id) {
                                      setSelectedSessionId(id);
                                      return;
                                    }
                                    await browserStatusQ.refetch();
                                  }
                                }
                              ]}
                              ariaLabel="Browser session options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>

          <Box className="list-shell">
            <Typography variant="h6" mb={1}>Respond to Session</Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Selected session ID"
                  value={selectedSessionId}
                  onChange={(e) => setSelectedSessionId(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Button
                  fullWidth
                  variant="outlined"
                  disabled={!selectedSessionId.trim() || browserStatusQ.isFetching}
                  onClick={() => browserStatusQ.refetch()}
                >
                  {browserStatusQ.isFetching ? "Checking..." : "Check Status"}
                </Button>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Typography variant="body2" color="text.secondary">
                  Current status: {str(browserStatus.status, str(browserStatus.error, selectedSessionId ? "unknown" : "select a session"))}
                </Typography>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={3}
                  label="Response"
                  value={sessionResponse}
                  onChange={(e) => setSessionResponse(e.target.value)}
                  placeholder="Example: Continue with the first result and summarize key points."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Button
                  variant="contained"
                  disabled={!selectedSessionId.trim() || !sessionResponse.trim() || browserRespondMutation.isPending}
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    setBrowserRespondResult(null);
                    try {
                      const out = asRecord(
                        await browserRespondMutation.mutateAsync({
                          id: selectedSessionId.trim(),
                          response: sessionResponse.trim()
                        })
                      );
                      setBrowserRespondResult(out);
                      setSuccess("Response sent to browser session.");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {browserRespondMutation.isPending ? "Sending..." : "Send Response"}
                </Button>
              </Grid2>
            </Grid2>
            {browserRespondResult ? (
              <Box sx={{ mt: 1 }}>
                <KeyValuePanel title="Last response result" data={browserRespondResult} />
              </Box>
            ) : null}
          </Box>
        </Stack>
      ) : null}

      <SuggestionRunDialog
        run={suggestionRun}
        open={suggestionRunOpen}
        minimized={suggestionRunMinimized}
        trace={suggestionTrace}
        traceSteps={suggestionTraceSteps}
        traceLoading={suggestionTraceQ.isLoading}
        traceError={suggestionTraceQ.error}
        detailError={suggestionDetailQ.error}
        acceptedOutcomes={suggestionAcceptedOutcomes}
        onClose={() => setSuggestionRunOpen(false)}
        onMinimize={() => setSuggestionRunMinimized(true)}
        onRestore={() => setSuggestionRunMinimized(false)}
        onOpenWorkspacePanel={openWorkspacePanel}
        getConsoleView={(stepRecord) => buildTraceStepConsoleView(suggestionTrace, suggestionTraceSteps, stepRecord)}
        getTraceStepColor={traceStepColor}
        humanTs={humanTs}
        errMessage={errMessage}
      />

      {settingsQ.error || briefingQ.error || notificationsQ.error || error || (showAdvanced && (timelineQ.error || browserStatusQ.error)) ? (
        <Alert severity="error">
          {error ||
            errMessage(
              settingsQ.error ||
              briefingQ.error ||
              notificationsQ.error ||
              (showAdvanced ? timelineQ.error || browserStatusQ.error : null)
            )}
        </Alert>
      ) : null}
      {success ? <Alert severity="success">{success}</Alert> : null}
    </Stack>
  );
}

function DocumentsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [projectId, setProjectId] = useState("");
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [selectedFileName, setSelectedFileName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const docsQ = useQuery({ queryKey: ["documents-manager"], queryFn: () => api.rawGet("/documents?limit=100"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const projectsQ = useQuery({ queryKey: ["documents-projects"], queryFn: () => api.rawGet("/projects"), refetchInterval: autoRefresh ? REFRESH_MS : false });

  const uploadFileMutation = useMutation({
    mutationFn: async () => {
      if (!selectedFile) throw new Error("No file selected");
      const formData = new FormData();
      formData.append("file", selectedFile, selectedFile.name);
      if (projectId.trim()) formData.append("project_id", projectId.trim());
      return api.rawPostForm("/documents/upload-file", formData);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
    }
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/documents/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
    }
  });

  const docs = pickRecords(docsQ.data, "documents");
  const projects = pickRecords(projectsQ.data, "projects");
  const projectMap = useMemo(() => {
    const m = new Map<string, string>();
    projects.forEach((project) => {
      m.set(str(project.id, ""), str(project.name, "Untitled"));
    });
    return m;
  }, [projects]);

  const handleFileSelected = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setError(null);
    setSelectedFile(file);
    setSelectedFileName(file.name);
    event.target.value = "";
  };

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <input
          ref={fileInputRef}
          type="file"
          hidden
          accept=".txt,.md,.markdown,.json,.csv,.tsv,.xml,.html,.htm,.yaml,.yml,.log,.ini,.toml,.sql,.js,.ts,.tsx,.jsx,.py,.rs,.go,.java,.c,.cpp,.h,.hpp,.sh,.bat,.ps1,.pdf,.docx,text/*,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
          onChange={handleFileSelected}
        />
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} alignItems={{ xs: "flex-start", sm: "center" }} mb={1}>
          <Typography variant="h6" sx={{ flex: 1 }}>
            Documents
          </Typography>
          <Button
            variant="contained"
            size="small"
            disabled={uploadFileMutation.isPending}
            onClick={() => fileInputRef.current?.click()}
          >
            Upload Document
          </Button>
        </Stack>

        {selectedFile ? (
          <Box className="metadata-box" sx={{ mb: 1.25 }}>
            <Grid2 container spacing={1} alignItems="center">
              <Grid2 size={{ xs: 12, md: projects.length > 0 ? 4 : 8 }}>
                <Typography variant="body2" sx={{ wordBreak: "break-word" }}>
                  Selected: {selectedFileName}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  Supports PDF, DOCX, TXT, MD, JSON, CSV and code/text files.
                </Typography>
              </Grid2>
              {projects.length > 0 ? (
                <Grid2 size={{ xs: 12, md: 4 }}>
                  <TextField
                    fullWidth
                    size="small"
                    select
                    label="Project (optional)"
                    value={projectId}
                    onChange={(e) => setProjectId(e.target.value)}
                    InputLabelProps={{ shrink: true }}
                    SelectProps={{ displayEmpty: true }}
                  >
                    <MenuItem value="">Global</MenuItem>
                    {projects.map((project) => <MenuItem key={str(project.id, "")} value={str(project.id, "")}>{str(project.name)}</MenuItem>)}
                  </TextField>
                </Grid2>
              ) : null}
              <Grid2 size={{ xs: 12, md: projects.length > 0 ? 4 : 4 }}>
                <Stack direction="row" spacing={1}>
                  <Button
                    variant="contained"
                    disabled={uploadFileMutation.isPending || !selectedFile}
                    onClick={async () => {
                      setError(null);
                      try {
                        await uploadFileMutation.mutateAsync();
                        setSelectedFile(null);
                        setSelectedFileName("");
                      } catch (e) {
                        setError(errMessage(e));
                      }
                    }}
                  >
                    {uploadFileMutation.isPending ? "Uploading..." : "Upload"}
                  </Button>
                  <Button
                    variant="text"
                    onClick={() => {
                      setSelectedFile(null);
                      setSelectedFileName("");
                      setError(null);
                      if (fileInputRef.current) fileInputRef.current.value = "";
                    }}
                  >
                    Clear
                  </Button>
                </Stack>
              </Grid2>
            </Grid2>
          </Box>
        ) : null}

        <TableContainer className="table-shell">
          <Table size="small">
            <TableHead><TableRow><TableCell>Filename</TableCell><TableCell>Project</TableCell><TableCell>Type</TableCell><TableCell>Chunks</TableCell><TableCell>Size</TableCell><TableCell>Created</TableCell><TableCell>Ops</TableCell></TableRow></TableHead>
            <TableBody>
              {docs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7}>
                    <Typography variant="body2" color="text.secondary">
                      No documents yet. Click "Upload Document" to add your first file.
                    </Typography>
                  </TableCell>
                </TableRow>
              ) : (
                docs.map((doc) => {
                  const id = str(doc.id, "");
                  const pid = str(doc.project_id, "");
                  return (
                    <TableRow key={id}>
                      <TableCell>{str(doc.filename)}</TableCell>
                      <TableCell>{pid ? projectMap.get(pid) || pid : "-"}</TableCell>
                      <TableCell>{str(doc.content_type)}</TableCell>
                      <TableCell>{str(doc.chunk_count)}</TableCell>
                      <TableCell>{formatBytes(doc.file_size)}</TableCell>
                      <TableCell title={humanTs(str(doc.created_at)).tip}>{humanTs(str(doc.created_at)).label}</TableCell>
                      <TableCell align="right">
                        <RowOpsMenu
                          actions={[
                            {
                              label: "Delete",
                              tone: "error",
                              onClick: () => deleteMutation.mutate(id)
                            }
                          ]}
                          ariaLabel="Document options"
                        />
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>

      {docsQ.error || projectsQ.error || error ? <Alert severity="error">{error || errMessage(docsQ.error || projectsQ.error)}</Alert> : null}
    </Stack>
  );
}

function MemoryManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedFact, setSelectedFact] = useState<JsonRecord | null>(null);
  const [memoryTab, setMemoryTab] = useState(0);
  const [prefKey, setPrefKey] = useState("");
  const [prefValue, setPrefValue] = useState("");
  const [prefConfidence, setPrefConfidence] = useState("0.85");
  const [prefSource, setPrefSource] = useState("");
  const [dataKind, setDataKind] = useState("note");
  const [dataTitle, setDataTitle] = useState("");
  const [dataContent, setDataContent] = useState("");
  const [dataUrl, setDataUrl] = useState("");
  const [knowledgeTitle, setKnowledgeTitle] = useState("");
  const [knowledgeContent, setKnowledgeContent] = useState("");
  const [knowledgeSource, setKnowledgeSource] = useState("");
  const [knowledgeUrl, setKnowledgeUrl] = useState("");
  const [knowledgeTags, setKnowledgeTags] = useState("");

  const invalidateMemoryQueries = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["memory-stats"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-facts"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-user-data"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-knowledge"] })
    ]);
  };

  const statsQ = useQuery({
    queryKey: ["memory-stats"],
    queryFn: () => api.rawGet("/memory/stats"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const factsQ = useQuery({
    queryKey: ["memory-facts"],
    queryFn: () => api.rawGet("/memory/facts?limit=50"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const preferencesQ = useQuery({
    queryKey: ["memory-preferences"],
    queryFn: () => api.rawGet("/memory/preferences?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const userDataQ = useQuery({
    queryKey: ["memory-user-data"],
    queryFn: () => api.rawGet("/memory/user-data?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const knowledgeQ = useQuery({
    queryKey: ["memory-knowledge"],
    queryFn: () => api.rawGet("/memory/knowledge?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const createPreferenceMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/memory/preferences", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });
  const deletePreferenceMutation = useMutation({
    mutationFn: (endpoint: string) => api.rawDelete(endpoint),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });
  const createUserDataMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/memory/user-data", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });
  const deleteUserDataMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/memory/user-data/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });
  const createKnowledgeMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/memory/knowledge", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });
  const deleteKnowledgeMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/memory/knowledge/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    }
  });

  const stats = asRecord(statsQ.data);
  const facts = pickRecords(factsQ.data, "facts");
  const preferences = pickRecords(preferencesQ.data, "preferences");
  const userDataItems = pickRecords(userDataQ.data, "items");
  const knowledgeItems = pickRecords(knowledgeQ.data, "items");

  const parseSources = (value: unknown): string[] => {
    if (Array.isArray(value)) return value.map((v) => String(v));
    if (typeof value !== "string" || !value.trim()) return [];
    try {
      const parsed = JSON.parse(value);
      if (Array.isArray(parsed)) return parsed.map((v) => String(v));
    } catch {
      // Keep fallback below.
    }
    return [value];
  };

  return (
    <Stack spacing={2}>
      {/* ── Compact stat row ── */}
      <Box sx={{ display: "grid", gridTemplateColumns: { xs: "repeat(2, 1fr)", sm: "repeat(3, 1fr)", md: "repeat(5, 1fr)" }, gap: 1.5 }}>
        {[
          { label: "Episodes", value: num(stats.episodes), color: "#2fd4ff" },
          { label: "Facts", value: num(stats.facts), color: "#14f195" },
          { label: "Preferences", value: num(stats.preferences), color: "#a78bfa" },
          { label: "User Data", value: num(stats.user_data), color: "#f59e0b" },
          { label: "Knowledge", value: num(stats.knowledge), color: "#f472b6" },
        ].map((s) => (
          <Box
            key={s.label}
            sx={{
              p: 1.5,
              borderRadius: 2,
              border: "1px solid rgba(255,255,255,0.06)",
              background: "rgba(255,255,255,0.02)",
              display: "flex",
              alignItems: "center",
              gap: 1.5,
            }}
          >
            <Typography variant="h5" sx={{ fontWeight: 600, color: s.color, lineHeight: 1, minWidth: 28 }}>{s.value}</Typography>
            <Typography variant="caption" sx={{ color: "rgba(180,200,225,0.55)", fontSize: "0.72rem", lineHeight: 1.2 }}>{s.label}</Typography>
          </Box>
        ))}
      </Box>

      {/* ── Memory tabs ── */}
      <Tabs
        value={memoryTab}
        onChange={(_e, next) => setMemoryTab(next)}
        variant="scrollable"
        allowScrollButtonsMobile
        sx={{ minHeight: 0, "& .MuiTab-root": { minHeight: 0, py: 0.5, fontSize: "0.8rem" } }}
      >
        <Tab label={`Facts (${facts.length})`} />
        <Tab label={`Preferences (${preferences.length})`} />
        <Tab label={`User Data (${userDataItems.length})`} />
        <Tab label={`Knowledge (${knowledgeItems.length})`} />
      </Tabs>

      {memoryTab === 0 ? (
        <Box className="list-shell">
          <Typography variant="h6" mb={1}>
            Semantic Facts
          </Typography>
          {factsQ.error ? <Alert severity="error">{errMessage(factsQ.error)}</Alert> : null}
          {facts.length === 0 ? (
            <Typography variant="body2" color="text.secondary">
              No facts yet.
            </Typography>
          ) : (
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Fact</TableCell>
                    <TableCell>Confidence</TableCell>
                    <TableCell>Created</TableCell>
                    <TableCell>Sources</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {facts.slice(0, 50).map((f, idx) => {
                    const id = str(f.id, String(idx));
                    const sources = parseSources(f.sources);
                    return (
                      <TableRow key={id}>
                        <TableCell sx={{ maxWidth: 640 }}>
                          <Typography variant="body2" noWrap title={str(f.fact, "-")}>
                            {str(f.fact, "-")}
                          </Typography>
                        </TableCell>
                        <TableCell>{num(f.confidence, 0).toFixed(2)}</TableCell>
                        <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(f.created_at, "-")).tip}>{humanTs(str(f.created_at, "-")).label}</TableCell>
                        <TableCell>{sources.length}</TableCell>
                        <TableCell align="right">
                          <RowOpsMenu
                            actions={[
                              {
                                label: "View",
                                onClick: () => setSelectedFact(asRecord(f))
                              }
                            ]}
                            ariaLabel="Fact options"
                          />
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </Box>
      ) : null}

      {memoryTab === 1 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              Add Preference
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField fullWidth size="small" label="Key" placeholder="timezone" value={prefKey} onChange={(e) => setPrefKey(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField fullWidth size="small" label="Value" placeholder="Asia/Kolkata" value={prefValue} onChange={(e) => setPrefValue(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 2 }}>
                <TextField fullWidth size="small" type="number" label="Confidence" inputProps={{ min: 0, max: 1, step: 0.05 }} value={prefConfidence} onChange={(e) => setPrefConfidence(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField fullWidth size="small" label="Source (optional)" placeholder="user_message" value={prefSource} onChange={(e) => setPrefSource(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12 }} sx={{ display: "flex", justifyContent: "flex-end" }}>
                <Button
                  variant="contained"
                  disabled={createPreferenceMutation.isPending || !prefKey.trim() || !prefValue.trim()}
                  onClick={async () => {
                    setError(null);
                    try {
                      const parsedConfidence = Number(prefConfidence);
                      await createPreferenceMutation.mutateAsync({
                        key: prefKey.trim(),
                        value: prefValue.trim(),
                        confidence: Number.isFinite(parsedConfidence) ? parsedConfidence : 0.85,
                        source: prefSource.trim() || undefined
                      });
                      setPrefKey("");
                      setPrefValue("");
                      setPrefSource("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save Preference
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              Preferences
            </Typography>
            {preferencesQ.error ? <Alert severity="error">{errMessage(preferencesQ.error)}</Alert> : null}
            {preferences.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No preferences yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Key</TableCell>
                      <TableCell>Value</TableCell>
                      <TableCell>Confidence</TableCell>
                      <TableCell>Source</TableCell>
                      <TableCell>Scope</TableCell>
                      <TableCell>Updated</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {preferences.map((pref, idx) => {
                      const key = str(pref.key, String(idx));
                      const projectId = typeof pref.project_id === "string" ? pref.project_id : "";
                      const endpoint = projectId
                        ? `/memory/preferences/${encodeURIComponent(key)}?project_id=${encodeURIComponent(projectId)}`
                        : `/memory/preferences/${encodeURIComponent(key)}`;
                      return (
                        <TableRow key={`${projectId || "global"}-${key}-${idx}`}>
                          <TableCell sx={{ whiteSpace: "nowrap" }}>{key}</TableCell>
                          <TableCell sx={{ maxWidth: 480 }}>
                            <Typography variant="body2" noWrap title={str(pref.value, "-")}>
                              {str(pref.value, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell>{num(pref.confidence, 0).toFixed(2)}</TableCell>
                          <TableCell>{str(pref.source, "-")}</TableCell>
                          <TableCell>{projectId || "Global"}</TableCell>
                          <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(pref.updated_at, "-")).tip}>{humanTs(str(pref.updated_at, "-")).label}</TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Delete",
                                  tone: "error",
                                  divider: true,
                                  onClick: async () => {
                                    setError(null);
                                    try {
                                      await deletePreferenceMutation.mutateAsync(endpoint);
                                    } catch (e) {
                                      setError(errMessage(e));
                                    }
                                  }
                                }
                              ]}
                              ariaLabel="Preference options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}

      {memoryTab === 2 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              Add User Data
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField fullWidth size="small" label="Kind" placeholder="note | link | file" value={dataKind} onChange={(e) => setDataKind(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 5 }}>
                <TextField fullWidth size="small" label="Title" placeholder="Quarterly roadmap doc" value={dataTitle} onChange={(e) => setDataTitle(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField fullWidth size="small" label="URL (optional)" placeholder="https://..." value={dataUrl} onChange={(e) => setDataUrl(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField fullWidth size="small" multiline minRows={3} label="Content" placeholder="Summary or notes" value={dataContent} onChange={(e) => setDataContent(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12 }} sx={{ display: "flex", justifyContent: "flex-end" }}>
                <Button
                  variant="contained"
                  disabled={createUserDataMutation.isPending || !dataKind.trim() || !dataTitle.trim()}
                  onClick={async () => {
                    setError(null);
                    try {
                      await createUserDataMutation.mutateAsync({
                        kind: dataKind.trim(),
                        title: dataTitle.trim(),
                        content: dataContent.trim(),
                        url: dataUrl.trim() || undefined
                      });
                      setDataKind("note");
                      setDataTitle("");
                      setDataContent("");
                      setDataUrl("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save User Data
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              User Data
            </Typography>
            {userDataQ.error ? <Alert severity="error">{errMessage(userDataQ.error)}</Alert> : null}
            {userDataItems.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No user data items yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Kind</TableCell>
                      <TableCell>Title</TableCell>
                      <TableCell>Content</TableCell>
                      <TableCell>URL</TableCell>
                      <TableCell>Updated</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {userDataItems.map((item, idx) => {
                      const id = str(item.id, String(idx));
                      const url = str(item.url, "");
                      return (
                        <TableRow key={id}>
                          <TableCell>{str(item.kind, "-")}</TableCell>
                          <TableCell sx={{ maxWidth: 220 }}>
                            <Typography variant="body2" noWrap title={str(item.title, "-")}>
                              {str(item.title, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 380 }}>
                            <Typography variant="body2" noWrap title={str(item.content, "-")}>
                              {str(item.content, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 260 }}>
                            {url ? (
                              <Typography component="a" href={url} target="_blank" rel="noopener noreferrer" variant="body2" sx={{ color: "var(--mui-palette-info-main)", textDecoration: "none" }}>
                                Open
                              </Typography>
                            ) : (
                              <Typography variant="body2" color="text.secondary">-</Typography>
                            )}
                          </TableCell>
                          <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(item.updated_at, "-")).tip}>{humanTs(str(item.updated_at, "-")).label}</TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Delete",
                                  tone: "error",
                                  divider: true,
                                  onClick: async () => {
                                    setError(null);
                                    try {
                                      await deleteUserDataMutation.mutateAsync(id);
                                    } catch (e) {
                                      setError(errMessage(e));
                                    }
                                  }
                                }
                              ]}
                              ariaLabel="User data options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}

      {memoryTab === 3 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              Add Knowledge Base Item
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 5 }}>
                <TextField fullWidth size="small" label="Title" placeholder="How we deploy production apps" value={knowledgeTitle} onChange={(e) => setKnowledgeTitle(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField fullWidth size="small" label="Source (optional)" placeholder="runbook" value={knowledgeSource} onChange={(e) => setKnowledgeSource(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField fullWidth size="small" label="URL (optional)" placeholder="https://..." value={knowledgeUrl} onChange={(e) => setKnowledgeUrl(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField fullWidth size="small" multiline minRows={3} label="Content" placeholder="Durable, reusable knowledge" value={knowledgeContent} onChange={(e) => setKnowledgeContent(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 9 }}>
                <TextField fullWidth size="small" label="Tags (optional)" placeholder="ops, deployment, production" value={knowledgeTags} onChange={(e) => setKnowledgeTags(e.target.value)} />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 3 }} sx={{ display: "flex", justifyContent: { xs: "flex-end", md: "stretch" }, alignItems: "stretch" }}>
                <Button
                  fullWidth
                  variant="contained"
                  disabled={createKnowledgeMutation.isPending || !knowledgeTitle.trim() || !knowledgeContent.trim()}
                  onClick={async () => {
                    setError(null);
                    try {
                      await createKnowledgeMutation.mutateAsync({
                        title: knowledgeTitle.trim(),
                        content: knowledgeContent.trim(),
                        source: knowledgeSource.trim() || undefined,
                        url: knowledgeUrl.trim() || undefined,
                        tags: knowledgeTags.trim() || undefined
                      });
                      setKnowledgeTitle("");
                      setKnowledgeContent("");
                      setKnowledgeSource("");
                      setKnowledgeUrl("");
                      setKnowledgeTags("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save Knowledge
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            <Typography variant="h6" mb={1}>
              Knowledge Base
            </Typography>
            {knowledgeQ.error ? <Alert severity="error">{errMessage(knowledgeQ.error)}</Alert> : null}
            {knowledgeItems.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No knowledge items yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Title</TableCell>
                      <TableCell>Content</TableCell>
                      <TableCell>Source</TableCell>
                      <TableCell>Tags</TableCell>
                      <TableCell>Updated</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {knowledgeItems.map((item, idx) => {
                      const id = str(item.id, String(idx));
                      return (
                        <TableRow key={id}>
                          <TableCell sx={{ maxWidth: 260 }}>
                            <Typography variant="body2" noWrap title={str(item.title, "-")}>
                              {str(item.title, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell sx={{ maxWidth: 420 }}>
                            <Typography variant="body2" noWrap title={str(item.content, "-")}>
                              {str(item.content, "-")}
                            </Typography>
                          </TableCell>
                          <TableCell>{str(item.source, "-")}</TableCell>
                          <TableCell>{str(item.tags, "-")}</TableCell>
                          <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(item.updated_at, "-")).tip}>{humanTs(str(item.updated_at, "-")).label}</TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Delete",
                                  tone: "error",
                                  divider: true,
                                  onClick: async () => {
                                    setError(null);
                                    try {
                                      await deleteKnowledgeMutation.mutateAsync(id);
                                    } catch (e) {
                                      setError(errMessage(e));
                                    }
                                  }
                                }
                              ]}
                              ariaLabel="Knowledge options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}

      {statsQ.error || factsQ.error || preferencesQ.error || userDataQ.error || knowledgeQ.error || error ? (
        <Alert severity="error">
          {error || errMessage(statsQ.error || factsQ.error || preferencesQ.error || userDataQ.error || knowledgeQ.error)}
        </Alert>
      ) : null}

      <Dialog open={selectedFact != null} onClose={() => setSelectedFact(null)} maxWidth="md" fullWidth>
        <DialogTitle>Fact</DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Typography variant="caption" color="text.secondary">
              Confidence: {num(selectedFact?.confidence, 0)} | Created: <span title={humanTs(str(selectedFact?.created_at, "-")).tip}>{humanTs(str(selectedFact?.created_at, "-")).label}</span>
            </Typography>
            <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
              {str(selectedFact?.fact, "-")}
            </Typography>
            <Divider />
            <Typography variant="subtitle2">Sources</Typography>
            {parseSources(selectedFact?.sources).length ? (
              <Stack spacing={0.5}>
                {parseSources(selectedFact?.sources).slice(0, 50).map((s, i) => (
                  <Box key={`src-${i}`} className="console-line">
                    <Typography variant="body2" sx={{ fontFamily: "JetBrains Mono, monospace" }}>
                      {String(s)}
                    </Typography>
                  </Box>
                ))}
              </Stack>
            ) : (
              <Typography variant="body2" color="text.secondary">
                No sources recorded.
              </Typography>
            )}
          </Stack>
        </DialogContent>
      </Dialog>
    </Stack>
  );
}
function ProjectsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<JsonRecord | null>(null);
  const [deleteProject, setDeleteProject] = useState<JsonRecord | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [editForm, setEditForm] = useState({
    name: "",
    description: "",
    system_prompt: "",
    personality: "",
    tools_filter: "",
    active: true
  });

  const projectsQ = useQuery({ queryKey: ["projects-manager"], queryFn: () => api.rawGet("/projects"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const conversationsQ = useQuery({ queryKey: ["projects-conversations"], queryFn: () => api.rawGet("/conversations?limit=100"), refetchInterval: autoRefresh ? REFRESH_MS : false });

  const createMutation = useMutation({ mutationFn: () => api.rawPost("/projects", { name: name.trim(), description: description.trim() }), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ["projects-manager"] }); } });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/projects/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["projects-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["projects-conversations"] });
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-stats"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-facts"] });
      setDeleteProject(null);
      setDeleteConfirm("");
    }
  });
  const updateMutation = useMutation({
    mutationFn: (payload: { id: string; body: Record<string, unknown> }) =>
      api.rawPut(`/projects/${encodeURIComponent(payload.id)}`, payload.body),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["projects-manager"] });
      setSelectedProject(null);
    }
  });

  const projects = pickRecords(projectsQ.data, "projects");
  const conversations = pickRecords(conversationsQ.data, "conversations");
  const counts = useMemo(() => {
    const map = new Map<string, number>();
    conversations.forEach((conv) => {
      const pid = str(conv.project_id, "");
      if (!pid) return;
      map.set(pid, (map.get(pid) || 0) + 1);
    });
    return map;
  }, [conversations]);

  return (
    <Stack spacing={2}>
      <Dialog open={createOpen} onClose={() => setCreateOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Create Project</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ mt: 1 }}>
            <TextField fullWidth size="small" label="Name" value={name} onChange={(e) => setName(e.target.value)} />
            <TextField fullWidth size="small" label="Description" value={description} onChange={(e) => setDescription(e.target.value)} />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCreateOpen(false)}>Cancel</Button>
          <Button variant="contained" disabled={createMutation.isPending || !name.trim()} onClick={async () => { setError(null); try { await createMutation.mutateAsync(); setName(""); setDescription(""); setCreateOpen(false); } catch (e) { setError(errMessage(e)); } }}>Create</Button>
        </DialogActions>
      </Dialog>

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell">
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">Projects</Typography>
              <Button size="small" variant="contained" onClick={() => setCreateOpen(true)}>New Project</Button>
            </Stack>
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Name</TableCell>
                    <TableCell>Description</TableCell>
                    <TableCell>Conversations</TableCell>
                    <TableCell>Updated</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {projects.map((project) => {
                    const id = str(project.id, "");
                    const pname = str(project.name, "");
                    return (
                      <TableRow key={id}>
                        <TableCell>{str(project.name)}</TableCell>
                        <TableCell>{str(project.description)}</TableCell>
                        <TableCell>{counts.get(id) || 0}</TableCell>
                        <TableCell title={humanTs(str(project.updated_at, str(project.created_at))).tip}>{humanTs(str(project.updated_at, str(project.created_at))).label}</TableCell>
                        <TableCell align="right">
                          <RowOpsMenu
                            actions={[
                              {
                                label: "Edit",
                                onClick: () => {
                                  const pr = asRecord(project);
                                  setSelectedProject(pr);
                                  setEditForm({
                                    name: str(pr.name, ""),
                                    description: str(pr.description, ""),
                                    system_prompt: str(pr.system_prompt, ""),
                                    personality: str(pr.personality, ""),
                                    tools_filter: str(pr.tools_filter, ""),
                                    active: pr.active !== false
                                  });
                                }
                              },
                              {
                                label: "Delete",
                                tone: "error",
                                divider: true,
                                onClick: () => {
                                  setDeleteProject(asRecord(project));
                                  setDeleteConfirm("");
                                }
                              }
                            ]}
                            ariaLabel="Project options"
                          />
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}><QueryTable title="Project Conversations" path="/conversations?limit=100" arrayKey="conversations" columns={["title", "project_id", "channel", "updated_at"]} autoRefresh={autoRefresh} emptyLabel="No conversations mapped to projects." queryKey="projects-conversation-table" /></Grid2>
      </Grid2>

      {projectsQ.error || conversationsQ.error || error ? <Alert severity="error">{error || errMessage(projectsQ.error || conversationsQ.error)}</Alert> : null}

      <Dialog open={selectedProject != null} onClose={() => setSelectedProject(null)} maxWidth="md" fullWidth>
        <DialogTitle>Edit Project</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2}>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Name"
                  value={editForm.name}
                  onChange={(e) => setEditForm((p) => ({ ...p, name: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={editForm.active}
                      onChange={(e) => setEditForm((p) => ({ ...p, active: e.target.checked }))}
                    />
                  }
                  label="Active"
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Description"
                  value={editForm.description}
                  onChange={(e) => setEditForm((p) => ({ ...p, description: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={4}
                  label="System Prompt (optional)"
                  value={editForm.system_prompt}
                  onChange={(e) => setEditForm((p) => ({ ...p, system_prompt: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Personality (optional)"
                  value={editForm.personality}
                  onChange={(e) => setEditForm((p) => ({ ...p, personality: e.target.value }))}
                  placeholder="e.g. friendly"
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Tools Filter (optional)"
                  value={editForm.tools_filter}
                  onChange={(e) => setEditForm((p) => ({ ...p, tools_filter: e.target.value }))}
                  placeholder="Comma-separated allowlist"
                />
              </Grid2>
            </Grid2>

            <Stack direction="row" spacing={1} justifyContent="flex-end">
              <Button onClick={() => setSelectedProject(null)}>Cancel</Button>
              <Button
                variant="contained"
                disabled={updateMutation.isPending || !editForm.name.trim()}
                onClick={async () => {
                  const id = str(selectedProject?.id, "");
                  if (!id) return;
                  setError(null);
                  try {
                    await updateMutation.mutateAsync({
                      id,
                      body: {
                        name: editForm.name.trim(),
                        description: editForm.description.trim(),
                        system_prompt: editForm.system_prompt.trim() || undefined,
                        personality: editForm.personality.trim() || undefined,
                        tools_filter: editForm.tools_filter.trim() || undefined,
                        active: editForm.active
                      }
                    });
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Save
              </Button>
            </Stack>
          </Stack>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteProject != null} onClose={() => setDeleteProject(null)} maxWidth="sm" fullWidth>
        <DialogTitle>Delete Project</DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Alert severity="warning">
              This permanently deletes the project and ALL associated data: conversations, messages, documents, document chunks, episodic memories, and semantic facts.
            </Alert>
            <Typography variant="body2">
              Type the project name to confirm deletion: <b>{str(deleteProject?.name, "")}</b>
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Project name"
              value={deleteConfirm}
              onChange={(e) => setDeleteConfirm(e.target.value)}
            />
            <Stack direction="row" spacing={1} justifyContent="flex-end">
              <Button onClick={() => setDeleteProject(null)}>Cancel</Button>
              <Button
                color="error"
                variant="contained"
                disabled={
                  deleteMutation.isPending ||
                  !str(deleteProject?.id, "").trim() ||
                  deleteConfirm.trim() !== str(deleteProject?.name, "")
                }
                onClick={async () => {
                  const id = str(deleteProject?.id, "");
                  if (!id) return;
                  setError(null);
                  try {
                    await deleteMutation.mutateAsync(id);
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Delete Permanently
              </Button>
            </Stack>
          </Stack>
        </DialogContent>
      </Dialog>
    </Stack>
  );
}

function formatTraceDuration(durationMs: unknown): string {
  const ms = num(durationMs, -1);
  if (ms < 0) return "pending";
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) return `${totalSeconds >= 10 ? totalSeconds.toFixed(0) : totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  return `${minutes}m ${seconds}s`;
}

function traceStatusColor(status: string): "default" | "success" | "warning" | "error" {
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

function traceStepColor(stepType: string): "default" | "success" | "warning" | "error" {
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
  const combined = `${str(step.title, "")}\n${str(step.detail, "")}\n${formatTraceData(step.data)}`.toLowerCase();
  return /execution record saved|execution proof generated|verification id:|proof id:/.test(combined);
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
      if (/execution record saved|execution proof generated|verification id:|proof id:/.test(combined)) return null;
      if (/memory available|context packing|selected the best available model|using a direct execution strategy|prepared the next response/.test(combined) && !dataText) {
        return null;
      }
      const summary = detail || dataText || title;
      return {
        title: title || "Step",
        detail: truncateTraceEvidence(summary),
        type
      };
    })
    .filter((item): item is TraceEvidenceItem => Boolean(item))
    .slice(-4);
}

function extractTraceArtifacts(trace: JsonRecord, steps: JsonRecord[]): string[] {
  const sources = [
    str(trace.response, ""),
    ...steps.flatMap((step) => [str(step.detail, ""), formatTraceData(step.data)])
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

function buildExecutionProofConsoleEvidence(trace: JsonRecord, steps: JsonRecord[]): string {
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
  lines.push("Open Trace Detail for the verification record and the full evidence.");

  return lines.join("\n");
}

function buildTraceStepConsoleView(trace: JsonRecord, steps: JsonRecord[], step: JsonRecord): TraceStepConsoleView {
  const detail = str(step.detail, "").trim();
  const dataText = formatTraceData(step.data);

  if (!isExecutionProofStep(step)) {
    return { detail, dataText };
  }

  return {
    detail: "Verifiable execution record saved. The evidence for this run is summarized below.",
    dataText: buildExecutionProofConsoleEvidence(trace, steps)
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
  return value
    .map((item) => str(item, "").trim())
    .filter(Boolean);
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
  evidence?: string;
};

function buildEvolutionReviewCards(steps: JsonRecord[]): EvolutionReviewCard[] {
  const cards: EvolutionReviewCard[] = [];
  steps.forEach((step, idx) => {
    const data = parseTraceDataRecord(step.data);
    const traceKind = str(data.trace_kind, "").trim().toLowerCase();
    if (!traceKind.startsWith("self_evolve.")) return;

    const status = str(step.type, str(step.step_type, "info")).trim() || "info";
    const title = str(step.title, "Evolution").trim();
    const detail = str(step.detail, "").trim();
    const chips: string[] = [];
    const evidence: string[] = [];
    let rationale = "";

    if (traceKind === "self_evolve.request") {
      const mode = str(data.mode, "policy");
      const request = str(data.request, "").trim();
      chips.push(`Mode ${mode}`);
      if (toBool(data.apply_promotion)) chips.push("Promotion enabled");
      if (toBool(data.allow_code_writes)) chips.push("Code writes allowed");
      const canaryRollout = num(data.canary_rollout_percent, -1);
      if (canaryRollout > 0) chips.push(`Canary ${canaryRollout}%`);
      rationale = request;
    } else if (traceKind === "self_evolve.policy.result") {
      const evaluatedCandidates = num(data.evaluated_candidates, 0);
      const baselineAccuracy = percentageLabel(data.baseline_accuracy, 0);
      const candidateAccuracy = percentageLabel(data.best_candidate_accuracy, 0);
      const gain = num(data.accuracy_gain, Number.NaN);
      const candidateSource = str(data.candidate_source, "").trim();
      const changedFields = stringList(data.changed_fields);
      const notes = stringList(data.notes);
      chips.push(`${evaluatedCandidates} candidate${evaluatedCandidates === 1 ? "" : "s"}`);
      if (baselineAccuracy || candidateAccuracy) {
        chips.push(`${baselineAccuracy || "?"} -> ${candidateAccuracy || "?"}`);
      }
      if (Number.isFinite(gain)) chips.push(`Gain ${gain >= 0 ? "+" : ""}${(gain * 100).toFixed(1)} pts`);
      if (candidateSource) chips.push(candidateSource);
      rationale = `Gate: ${str(data.promotion_gate, "unknown")}`;
      if (num(data.wins, -1) >= 0 || num(data.losses, -1) >= 0) {
        evidence.push(`Wins/Losses: ${num(data.wins, 0)} / ${num(data.losses, 0)}`);
      }
      const pValue = num(data.p_value, Number.NaN);
      if (Number.isFinite(pValue)) evidence.push(`P-value: ${pValue.toFixed(4)}`);
      if (changedFields.length) evidence.push(`Changed fields: ${changedFields.join(", ")}`);
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
        evidence.push(`Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`);
      }
      const replayReason = str(replay.reason, "").trim();
      if (replayReason) rationale = replayReason;
      const baselineSamples = num(asRecord(replay.baseline).samples, -1);
      const candidateSamples = num(asRecord(replay.candidate).samples, -1);
      if (baselineSamples >= 0 || candidateSamples >= 0) {
        evidence.push(`Replay samples: baseline ${Math.max(0, baselineSamples)} | candidate ${Math.max(0, candidateSamples)}`);
      }
      const successGain = num(replay.success_gain, Number.NaN);
      if (Number.isFinite(successGain)) evidence.push(`Replay gain: ${(successGain * 100).toFixed(1)} pts`);
    } else if (traceKind === "self_evolve.code.blocked") {
      chips.push("Code evolution");
      chips.push("Blocked");
      rationale = str(data.request, "").trim();
    } else if (traceKind === "self_evolve.code.result") {
      const filesChanged = stringList(data.files_changed);
      const securityWarnings = stringList(data.security_warnings);
      const iterations = num(data.iterations_used, 0);
      chips.push(`${filesChanged.length} file${filesChanged.length === 1 ? "" : "s"}`);
      chips.push(`${iterations} iteration${iterations === 1 ? "" : "s"}`);
      if (toBool(data.push_recommended)) chips.push("Push suggested");
      rationale = str(data.diff_summary, "").trim();
      if (filesChanged.length) evidence.push(`Files changed: ${filesChanged.join(", ")}`);
      if (securityWarnings.length) evidence.push(`Security warnings: ${securityWarnings.join(" | ")}`);
      const error = str(data.error, "").trim();
      if (error) evidence.push(`Error: ${error}`);
    } else if (traceKind === "self_evolve.manual_action.result" || traceKind === "self_evolve.manual_action.request") {
      const action = str(data.action, "").trim().replace(/_/g, " ");
      const canaryState = asRecord(data.canary_state);
      chips.push(action || "Manual action");
      if (Object.keys(canaryState).length > 0) {
        chips.push(toBool(canaryState.enabled) ? "Canary enabled" : "Canary disabled");
      }
      rationale = str(data.message, detail).trim();
      const baselineVersion = str(canaryState.baseline_version, "").trim();
      const candidateVersion = str(canaryState.candidate_version, "").trim();
      if (baselineVersion || candidateVersion) {
        evidence.push(`Versions: ${baselineVersion || "baseline"} -> ${candidateVersion || "candidate"}`);
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
      evidence: evidence.join("\n") || undefined
    });
  });
  return cards;
}

function evolutionTraceIdHint(payload: unknown): string {
  const traceId = str(asRecord(payload).trace_id, "").trim();
  return traceId ? ` Trace ${traceId.slice(0, 8)} recorded.` : "";
}

function TraceManager({ autoRefresh }: { autoRefresh: boolean }) {
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);

  const traceQ = useQuery({ queryKey: ["trace-manager"], queryFn: () => api.rawGet("/trace?limit=40"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const traceDetailQ = useQuery({ queryKey: ["trace-detail", selectedTraceId], queryFn: () => api.rawGet(`/trace/${encodeURIComponent(selectedTraceId || "")}`), enabled: !!selectedTraceId });
  const exportLogsQ = useQuery({ queryKey: ["settings-observability-logs"], queryFn: () => api.rawGet("/settings/observability/logs"), refetchInterval: autoRefresh ? 30000 : false });
  const exportLogs = pickRecords(asRecord(exportLogsQ.data), "logs");

  const traceData = asRecord(traceQ.data);
  const history = pickRecords(traceData, "history");
  const selectedTrace = asRecord(traceDetailQ.data);
  const steps = pickRecords(traceDetailQ.data, "steps");
  const historyTotal = num(traceData.history_total, history.length);
  const selectedTraceStatus = str(selectedTrace.status, selectedTraceId ? "running" : "-");
  const selectedTraceProofId = str(selectedTrace.proof_id, "");
  const selectedTraceChannel = str(selectedTrace.channel, "chat");
  const selectedTraceResponse = str(selectedTrace.response, "").trim();
  const traceEvidence = buildTraceEvidenceItems(steps);
  const traceArtifacts = extractTraceArtifacts(selectedTrace, steps);
  const evolutionReviewCards = useMemo(() => buildEvolutionReviewCards(steps), [steps]);
  const traceOutcomeSummary =
    selectedTraceStatus === "completed"
      ? `Completed successfully in ${formatTraceDuration(selectedTrace.duration_ms)}`
      : selectedTraceStatus === "failed"
        ? `Failed after ${formatTraceDuration(selectedTrace.duration_ms)}`
        : `Status: ${selectedTraceStatus}`;

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Stack direction="row" alignItems="center" justifyContent="space-between" mb={1}>
          <Typography variant="h6">Trace History</Typography>
          <Typography variant="caption" color="text.secondary">
            Showing {history.length} of {historyTotal} runs
          </Typography>
        </Stack>
        {history.length === 0 ? (
          <Alert severity="info">
            No trace history is available yet. New chat runs will appear here with detailed execution steps. Older runs from before trace persistence was enabled may not be listed.
          </Alert>
        ) : (
          <TableContainer className="table-shell">
            <Table size="small" sx={{ tableLayout: "fixed" }}>
              <TableHead>
                <TableRow>
                  <TableCell width="14%">Started</TableCell>
                  <TableCell width="12%">Source</TableCell>
                  <TableCell width="40%">Message</TableCell>
                  <TableCell width="12%">Status</TableCell>
                  <TableCell width="10%">Steps</TableCell>
                  <TableCell width="12%">Duration</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {history.map((item, idx) => {
                  const id = str(item.id, `trace-${idx}`);
                  const status = str(item.status, "running");
                  return (
                    <TableRow
                      key={id}
                      hover
                      onClick={() => setSelectedTraceId(id)}
                      sx={{ cursor: "pointer" }}
                    >
                      <TableCell>
                        <Typography variant="body2" noWrap title={humanTs(str(item.started_at)).tip}>
                          {humanTs(str(item.started_at)).label}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" noWrap title={str(item.channel)}>
                          {str(item.channel)}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" fontWeight={600} noWrap title={str(item.message_preview)}>
                          {str(item.message_preview)}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                          <Box component="span" sx={{ width: 8, height: 8, borderRadius: "50%", flexShrink: 0, bgcolor: status === "completed" ? "rgba(74,210,157,0.85)" : status === "failed" ? "rgba(255,100,100,0.85)" : "rgba(180,200,220,0.5)" }} />
                          <Typography variant="body2" color="text.secondary" noWrap>{status}</Typography>
                        </Box>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" noWrap>
                          {num(item.step_count, 0)}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" noWrap>
                          {formatTraceDuration(item.duration_ms)}
                        </Typography>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </Box>

      {traceQ.error || traceDetailQ.error ? (
        <Alert severity="error">{errMessage(traceQ.error || traceDetailQ.error)}</Alert>
      ) : null}

      <Dialog open={selectedTraceId != null} onClose={() => setSelectedTraceId(null)} maxWidth="lg" fullWidth>
        <DialogTitle sx={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 2 }}>
          <Box>
            <Typography variant="h6">Trace Detail</Typography>
            <Typography variant="caption" color="text.secondary">
              <span title={humanTs(str(selectedTrace.started_at)).tip}>{humanTs(str(selectedTrace.started_at)).label}</span> | {selectedTraceChannel}
            </Typography>
          </Box>
          <IconButton size="small" onClick={() => setSelectedTraceId(null)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          {traceDetailQ.isLoading ? (
            <Typography variant="body2" color="text.secondary">Loading trace...</Typography>
          ) : (
            <Stack spacing={1.5}>
              {/* Summary bar */}
              <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                <Chip size="small" color={traceStatusColor(selectedTraceStatus)} label={selectedTraceStatus} />
                <Typography variant="body2" color="text.secondary">
                  {num(selectedTrace.step_count, steps.length)} steps | {formatTraceDuration(selectedTrace.duration_ms)}
                  {selectedTrace.total_tokens ? ` | ${num(selectedTrace.total_tokens, 0)} tokens` : ""}
                  {str(selectedTrace.model) ? ` | ${str(selectedTrace.model)}` : ""}
                </Typography>
                {selectedTraceProofId ? (
                  <Typography variant="caption" sx={{ fontFamily: "monospace", color: "text.secondary" }}>
                    ID: {selectedTraceProofId.slice(0, 12)}...
                  </Typography>
                ) : null}
              </Stack>

              {/* User message */}
              <Box sx={{ p: 1.25, borderRadius: 1.5, bgcolor: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.08)" }}>
                <Typography variant="caption" color="text.secondary" sx={{ mb: 0.25, display: "block" }}>Input</Typography>
                <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>{str(selectedTrace.message)}</Typography>
              </Box>

              {/* Response (collapsed if long) */}
              {selectedTraceResponse ? (
                <Box sx={{ p: 1.25, borderRadius: 1.5, bgcolor: "rgba(70,174,255,0.05)", border: "1px solid rgba(70,174,255,0.15)" }}>
                  <Typography variant="caption" color="text.secondary" sx={{ mb: 0.25, display: "block" }}>Output</Typography>
                  <Typography variant="body2" sx={{ whiteSpace: "pre-wrap", maxHeight: 200, overflow: "auto" }}>
                    {selectedTraceResponse}
                  </Typography>
                </Box>
              ) : null}

              {evolutionReviewCards.length > 0 ? (
                <Box sx={{ p: 1.25, borderRadius: 1.5, bgcolor: "rgba(120, 166, 255, 0.05)", border: "1px solid rgba(120, 166, 255, 0.16)" }}>
                  <Typography variant="subtitle2" sx={{ mb: 1 }}>Evolution Review</Typography>
                  <Stack spacing={1}>
                    {evolutionReviewCards.map((card) => (
                      <Box
                        key={card.key}
                        sx={{
                          p: 1,
                          borderRadius: 1.25,
                          bgcolor: "rgba(255,255,255,0.025)",
                          border: "1px solid rgba(255,255,255,0.06)"
                        }}
                      >
                        <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap" sx={{ mb: 0.5 }}>
                          <Typography variant="body2" fontWeight={700}>{card.title}</Typography>
                          <Chip size="small" color={traceStepColor(card.status)} label={card.status} />
                          {card.chips.map((chip) => (
                            <Chip key={`${card.key}-${chip}`} size="small" variant="outlined" label={chip} />
                          ))}
                        </Stack>
                        {card.detail ? (
                          <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-wrap" }}>
                            {card.detail}
                          </Typography>
                        ) : null}
                        {card.rationale ? (
                          <Typography variant="caption" sx={{ display: "block", mt: 0.75, whiteSpace: "pre-wrap" }}>
                            Why: {card.rationale}
                          </Typography>
                        ) : null}
                        {card.evidence ? (
                          <Box
                            component="pre"
                            sx={{
                              mt: 0.75,
                              mb: 0,
                              p: 0.75,
                              whiteSpace: "pre-wrap",
                              wordBreak: "break-word",
                              fontSize: 11,
                              borderRadius: 1,
                              bgcolor: "rgba(255,255,255,0.03)",
                              border: "1px solid rgba(255,255,255,0.06)"
                            }}
                          >
                            {card.evidence}
                          </Box>
                        ) : null}
                      </Box>
                    ))}
                  </Stack>
                </Box>
              ) : null}

              {/* Artifacts */}
              {traceArtifacts.length > 0 ? (
                <Stack direction="row" spacing={0.5} useFlexGap flexWrap="wrap" alignItems="center">
                  <Typography variant="caption" color="text.secondary">Artifacts:</Typography>
                  {traceArtifacts.map((a) => <Chip key={a} size="small" variant="outlined" label={a} />)}
                </Stack>
              ) : null}

              {/* Execution steps — the main view */}
              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>Execution Steps</Typography>
                <Box className="metadata-box" sx={{ maxHeight: 500 }}>
                  {steps.length === 0 ? (
                    <Typography variant="body2" color="text.secondary">No steps recorded.</Typography>
                  ) : (
                    <Stack spacing={0.5}>
                      {steps.map((step, idx) => {
                        const consoleView = buildTraceStepConsoleView(selectedTrace, steps, step);
                        const stepTime = formatTraceStepTime(str(step.time));
                        return (
                          <Box key={`${str(step.time, "step")}-${idx}`} sx={{ py: 0.5, px: 1, borderRadius: 1, "&:hover": { bgcolor: "rgba(255,255,255,0.03)" } }}>
                            <Stack direction="row" spacing={1} alignItems="baseline" useFlexGap flexWrap="wrap">
                              <Typography variant="caption" color="text.secondary" sx={{ minWidth: 70, fontFamily: "monospace", fontSize: "0.7rem" }}>
                                {stepTime}
                              </Typography>
                              <Box component="span" sx={{ width: 6, height: 6, borderRadius: "50%", flexShrink: 0, mt: 0.5, bgcolor: str(step.type).includes("error") || str(step.type).includes("fail") ? "rgba(255,100,100,0.85)" : str(step.type).includes("success") || str(step.type).includes("complete") ? "rgba(74,210,157,0.85)" : str(step.type).includes("think") || str(step.type).includes("reason") ? "rgba(255,211,106,0.85)" : "rgba(120,160,210,0.5)" }} />
                              <Typography variant="body2" fontWeight={600} sx={{ flex: 1 }}>{str(step.title)}</Typography>
                            </Stack>
                            {consoleView.detail ? (
                              <Typography variant="caption" color="text.secondary" sx={{ pl: "86px", display: "block", whiteSpace: "pre-wrap" }}>
                                {consoleView.detail}
                              </Typography>
                            ) : null}
                            {consoleView.dataText ? (
                              <Box component="pre" sx={{ ml: "86px", mt: 0.25, mb: 0, p: 0.75, whiteSpace: "pre-wrap", wordBreak: "break-word", fontSize: 11, borderRadius: 1, bgcolor: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)", maxHeight: 120, overflow: "auto" }}>
                                {consoleView.dataText}
                              </Box>
                            ) : null}
                          </Box>
                        );
                      })}
                    </Stack>
                  )}
                </Box>
              </Box>

              {/* Timing footer */}
              <Typography variant="caption" color="text.secondary">
                Started: <span title={humanTs(str(selectedTrace.started_at)).tip}>{humanTs(str(selectedTrace.started_at)).label}</span>
                {selectedTrace.completed_at ? <>{" | Completed: "}<span title={humanTs(str(selectedTrace.completed_at)).tip}>{humanTs(str(selectedTrace.completed_at)).label}</span></> : ""}
              </Typography>
            </Stack>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSelectedTraceId(null)}>Close</Button>
        </DialogActions>
      </Dialog>

      {/* Observability Export Delivery Logs */}
      {exportLogs.length > 0 ? (
        <Box className="list-shell">
          <Stack spacing={0.5} mb={1}>
            <Typography variant="h6">Export Delivery</Typography>
            <Typography variant="caption" color="text.secondary">
              Recent pushes to the observability platform.
            </Typography>
          </Stack>
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Time</TableCell>
                  <TableCell>Status</TableCell>
                  <TableCell>Event</TableCell>
                  <TableCell>Message</TableCell>
                  <TableCell>Trace</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {exportLogs.slice(0, 20).map((entry, idx) => {
                  const level = str(entry.level, "").toLowerCase();
                  const ts = humanTs(str(entry.timestamp, ""));
                  const traceId = str(entry.trace_id, "").trim();
                  return (
                    <TableRow
                      key={`exp-${str(entry.id, "log")}-${idx}`}
                      hover
                      onClick={() => { if (traceId) setSelectedTraceId(traceId); }}
                      sx={{ cursor: traceId ? "pointer" : "default" }}
                    >
                      <TableCell sx={{ whiteSpace: "nowrap" }}>
                        <Typography variant="body2" noWrap title={ts.tip}>{ts.label}</Typography>
                      </TableCell>
                      <TableCell>
                        <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                          <Box component="span" sx={{ width: 8, height: 8, borderRadius: "50%", flexShrink: 0, bgcolor: level === "error" ? "rgba(255,100,100,0.85)" : level === "success" ? "rgba(74,210,157,0.85)" : "rgba(180,200,220,0.5)" }} />
                          <Typography variant="body2" color="text.secondary" noWrap>{level || "info"}</Typography>
                        </Box>
                      </TableCell>
                      <TableCell><Typography variant="body2" noWrap>{str(entry.event, "-")}</Typography></TableCell>
                      <TableCell sx={{ maxWidth: 520 }}>
                        <Typography variant="body2" color={level === "error" ? "error" : "text.secondary"} noWrap title={str(entry.message, "-")}>
                          {str(entry.message, "-")}
                        </Typography>
                      </TableCell>
                      <TableCell sx={{ fontFamily: "monospace", fontSize: "0.76rem" }}>
                        {traceId ? traceId.slice(0, 8) : "-"}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        </Box>
      ) : null}
    </Stack>
  );
}

function StatusManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);

  const statusQ = useQuery({ queryKey: ["status-page-status"], queryFn: () => api.rawGet("/status"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const profileQ = useQuery({ queryKey: ["status-page-profile"], queryFn: () => api.rawGet("/profile"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const securityQ = useQuery({ queryKey: ["status-page-security"], queryFn: () => api.rawGet("/security/status"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const watchersQ = useQuery({ queryKey: ["status-page-watchers"], queryFn: () => api.rawGet("/watchers"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const securityLogsQ = useQuery({ queryKey: ["status-page-security-logs"], queryFn: () => api.rawGet("/security/logs?limit=20"), refetchInterval: autoRefresh ? REFRESH_MS : false });

  const cancelMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/cancel`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["status-page-watchers"] });
    }
  });
  const pauseMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/pause`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["status-page-watchers"] });
    }
  });
  const resumeMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/resume`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["status-page-watchers"] });
    }
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/watchers/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["status-page-watchers"] });
    }
  });

  const status = asRecord(statusQ.data);
  const profile = asRecord(profileQ.data);
  const security = asRecord(securityQ.data);
  const watchers = pickRecords(watchersQ.data, "watchers");

  return (
    <Stack spacing={2}>
      <Grid2 container spacing={2} alignItems="stretch">
        <Grid2 size={{ xs: 12, md: 3 }} sx={{ display: "flex" }}><Box className="list-shell" sx={{ minHeight: 120, height: "100%", width: "100%" }}><Typography variant="caption" color="text.secondary">DID</Typography><Typography variant="body2" sx={{ wordBreak: "break-all" }}>{str(status.did)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }} sx={{ display: "flex" }}><Box className="list-shell" sx={{ minHeight: 120, height: "100%", width: "100%" }}><Typography variant="caption" color="text.secondary">Tasks Pending</Typography><Typography variant="h5">{num(status.tasks_pending)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }} sx={{ display: "flex" }}><Box className="list-shell" sx={{ minHeight: 120, height: "100%", width: "100%" }}><Typography variant="caption" color="text.secondary">Skills Loaded</Typography><Typography variant="h5">{num(status.skills_loaded, num(status.actions_loaded))}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }} sx={{ display: "flex" }}><Box className="list-shell" sx={{ minHeight: 120, height: "100%", width: "100%" }}><Typography variant="caption" color="text.secondary">Memory Entries</Typography><Typography variant="h5">{num(status.memory_entries)}</Typography></Box></Grid2>
      </Grid2>

      <Grid2 container spacing={2} alignItems="stretch">
        <Grid2 size={{ xs: 12, lg: 4 }} sx={{ display: "flex" }}>
          <Box className="list-shell" sx={{ height: "100%", width: "100%" }}>
            <Typography variant="h6" mb={1}>Profile</Typography>
            <Stack spacing={0.5}>
              <Typography variant="body2">Name: {str(profile.name, "-")}</Typography>
              <Typography variant="body2">Location: {str(profile.location, "-")}</Typography>
              <Typography variant="body2">Timezone: {str(profile.timezone, "-")}</Typography>
              <Typography variant="body2">Language: {str(profile.language, "-")}</Typography>
              <Typography variant="body2">Tone: {str(profile.tone, "-")}</Typography>
            </Stack>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 4 }} sx={{ display: "flex" }}>
          <Box className="list-shell" sx={{ height: "100%", width: "100%" }}>
            <Typography variant="h6" mb={1}>Security</Typography>
            <Stack spacing={0.5}>
              <Typography variant="body2">Mode: {str(security.encryption_mode)}</Typography>
              {toBool(security.using_default) ? (
                <Typography variant="body2" color="warning.main">Using default password — set a custom one in Settings.</Typography>
              ) : (
                <Typography variant="body2" color="success.main">Custom master password active.</Typography>
              )}
            </Stack>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 4 }} sx={{ display: "flex" }}>
          <Box className="list-shell" sx={{ height: "100%", width: "100%" }}>
            <Typography variant="h6" mb={1}>Watchers</Typography>
            {watchers.length === 0 ? (
              <Typography variant="body2" color="text.secondary">No active watchers.</Typography>
            ) : (
              <Stack spacing={1}>
                {watchers.map((w) => {
                  const id = str(w.id, "");
                  const rawStatus = str(w.status, "");
                  const statusLower = rawStatus.toLowerCase();
                  const isActive = statusLower.includes("active");
                  const isPaused = statusLower.includes("paused");
                  return (
                    <Box key={id} className="action-row">
                      <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={1}>
                        <Stack>
                          <Typography variant="body2">{str(w.description)}</Typography>
                          <Typography variant="caption" color="text.secondary">
                            {rawStatus} | every {str(w.interval_secs)}s | notify {str(w.notify_channel, "-")}
                          </Typography>
                        </Stack>
                        <RowOpsMenu
                          ariaLabel="Watcher actions"
                          actions={[
                            {
                              label: "Pause",
                              disabled: !isActive || pauseMutation.isPending,
                              onClick: async () => { setError(null); try { await pauseMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Resume",
                              disabled: !isPaused || resumeMutation.isPending,
                              onClick: async () => { setError(null); try { await resumeMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Stop",
                              tone: "warning",
                              disabled: (!isActive && !isPaused) || cancelMutation.isPending,
                              onClick: async () => { setError(null); try { await cancelMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              disabled: deleteMutation.isPending,
                              onClick: async () => {
                                const ok = window.confirm("Delete this watcher? This cannot be undone.");
                                if (!ok) return;
                                setError(null);
                                try { await deleteMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); }
                              },
                            },
                          ]}
                        />
                      </Stack>
                    </Box>
                  );
                })}
              </Stack>
            )}
          </Box>
        </Grid2>
      </Grid2>

      <QueryTable title="Security Logs" path="/security/logs?limit=20" arrayKey="logs" columns={["event_type", "severity", "message", "source", "created_at", "count"]} autoRefresh={autoRefresh} emptyLabel="No security logs yet." queryKey="security-logs-table" />

      {statusQ.error || profileQ.error || securityQ.error || watchersQ.error || securityLogsQ.error || error ? (
        <Alert severity="error">{error || errMessage(statusQ.error || profileQ.error || securityQ.error || watchersQ.error || securityLogsQ.error)}</Alert>
      ) : null}
    </Stack>
  );
}

function watcherStatusLabel(raw: unknown): string {
  const value = str(raw, "").trim();
  if (!value) return "-";
  return value
    .replace(/_/g, " ")
    .replace(/\b\w/g, (m) => m.toUpperCase());
}

function watcherStatusColor(raw: unknown): "success" | "warning" | "error" | "default" | "info" {
  const value = str(raw, "").toLowerCase();
  if (value.includes("active")) return "success";
  if (value.includes("paused")) return "warning";
  if (value.includes("triggered")) return "info";
  if (value.includes("failed") || value.includes("timed") || value.includes("cancelled")) return "error";
  return "default";
}

function watcherConditionSummary(raw: unknown): string {
  const condition = asRecord(raw);
  const entries = Object.entries(condition);
  if (entries.length === 0) return "-";
  const [kind, payload] = entries[0];
  const body = asRecord(payload);
  if (kind === "not_empty") return "Trigger when results are not empty";
  if (kind === "contains") return `Trigger when result contains "${str(body.keyword, "")}"`;
  if (kind === "matches") return `Trigger when result matches ${str(body.pattern, "")}`;
  if (kind === "custom") return str(body.description, "Custom condition");
  return kind.replace(/_/g, " ");
}

function watcherPollOutcomeLabel(raw: unknown): string {
  const value = str(raw, "").trim();
  if (!value) return "Unknown";
  return value
    .replace(/_/g, " ")
    .replace(/\b\w/g, (m) => m.toUpperCase());
}

function watcherPollOutcomeColor(raw: unknown): "success" | "warning" | "error" | "default" | "info" {
  const value = str(raw, "").trim().toLowerCase();
  if (value === "matched") return "success";
  if (value === "error") return "error";
  if (value === "no_match") return "default";
  return "info";
}

function watcherPreviewText(raw: unknown, maxChars = 180): string {
  const text =
    typeof raw === "string"
      ? formatTraceData(raw)
      : raw == null
        ? ""
        : JSON.stringify(raw, null, 2);
  const compact = text.replace(/\s+/g, " ").trim();
  if (!compact) return "";
  if (compact.length <= maxChars) return compact;
  return `${compact.slice(0, Math.max(0, maxChars - 1))}…`;
}

function watcherPayloadText(raw: unknown): string {
  if (typeof raw === "string") return formatTraceData(raw);
  if (raw == null) return "";
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return String(raw);
  }
}

function WatcherPayloadPanel({
  title,
  value,
  emptyLabel
}: {
  title: string;
  value: unknown;
  emptyLabel: string;
}) {
  const text = watcherPayloadText(value).trim();
  return (
    <Box className="metadata-box">
      <Typography variant="caption" color="text.secondary">
        {title}
      </Typography>
      {text ? (
        <Typography
          component="pre"
          variant="body2"
          sx={{
            mt: 0.75,
            mb: 0,
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
          }}
        >
          {text}
        </Typography>
      ) : (
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.75 }}>
          {emptyLabel}
        </Typography>
      )}
    </Box>
  );
}

function WatchersManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedWatcherId, setSelectedWatcherId] = useState<string | null>(null);
  const watchersQ = useQuery({
    queryKey: ["watchers-page-watchers"],
    queryFn: () => api.rawGet("/watchers"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const pauseMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/pause`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    }
  });
  const resumeMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/resume`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    }
  });
  const cancelMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/watchers/${encodeURIComponent(id)}/cancel`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    }
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/watchers/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["watchers-page-watchers"] });
    }
  });

  const watchers = pickRecords(watchersQ.data, "watchers");
  const selectedWatcher = useMemo(
    () => watchers.find((watcher) => str(watcher.id, "") === selectedWatcherId) ?? null,
    [selectedWatcherId, watchers]
  );
  const activeCount = watchers.filter((w) => str(w.status, "").toLowerCase().includes("active")).length;
  const pausedCount = watchers.filter((w) => str(w.status, "").toLowerCase().includes("paused")).length;
  const triggeredCount = watchers.filter((w) => str(w.status, "").toLowerCase().includes("triggered")).length;
  const failedCount = watchers.filter((w) => {
    const status = str(w.status, "").toLowerCase();
    return status.includes("failed") || status.includes("timed") || status.includes("cancelled");
  }).length;

  return (
    <Stack spacing={2}>
      {watchers.length > 0 ? (
        <Grid2 container spacing={2} alignItems="stretch">
          <Grid2 size={{ xs: 6, md: 3 }} sx={{ display: "flex" }}>
            <Box className="list-shell" sx={{ minHeight: 80, height: "100%", width: "100%" }}>
              <Typography variant="caption" color="text.secondary">Active</Typography>
              <Typography variant="h5">{activeCount}</Typography>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 6, md: 3 }} sx={{ display: "flex" }}>
            <Box className="list-shell" sx={{ minHeight: 80, height: "100%", width: "100%" }}>
              <Typography variant="caption" color="text.secondary">Paused</Typography>
              <Typography variant="h5">{pausedCount}</Typography>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 6, md: 3 }} sx={{ display: "flex" }}>
            <Box className="list-shell" sx={{ minHeight: 80, height: "100%", width: "100%" }}>
              <Typography variant="caption" color="text.secondary">Triggered</Typography>
              <Typography variant="h5">{triggeredCount}</Typography>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 6, md: 3 }} sx={{ display: "flex" }}>
            <Box className="list-shell" sx={{ minHeight: 80, height: "100%", width: "100%" }}>
              <Typography variant="caption" color="text.secondary">Stopped / Failed</Typography>
              <Typography variant="h5">{failedCount}</Typography>
            </Box>
          </Grid2>
        </Grid2>
      ) : null}

      {watchers.length === 0 ? (
        <Box sx={{ py: 8, textAlign: "center" }}>
          <Typography variant="h6" color="text.secondary">No watchers</Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            Ask AgentArk to watch something until a condition is met, then notify a channel or take action.
          </Typography>
        </Box>
      ) : (
      <Box className="list-shell" sx={{ minHeight: 0 }}>
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Typography variant="h6">Watchers</Typography>
        </Stack>
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Watcher</TableCell>
                  <TableCell>Poll</TableCell>
                  <TableCell>Condition</TableCell>
                  <TableCell>Status</TableCell>
                  <TableCell>Activity</TableCell>
                  <TableCell align="right">Ops</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {watchers.map((w, idx) => {
                  const id = str(w.id, String(idx));
                  const rawStatus = str(w.status, "");
                  const statusLower = rawStatus.toLowerCase();
                  const isActive = statusLower.includes("active");
                  const isPaused = statusLower.includes("paused");
                  const isHistoryOnly = toBool(w.history_only);
                  const lastPollAt = str(w.last_poll_at, "").trim();
                  const createdAt = str(w.created_at, "").trim();
                  const lastPollLabel = lastPollAt ? formatTimestampForHumans(lastPollAt).label : "Never";
                  const createdLabel = createdAt ? formatTimestampForHumans(createdAt).label : "-";
                  const triggerResult = str(w.trigger_result, "").trim();
                  const lastResult = str(w.last_result, "").trim();
                  const lastError = str(w.last_error, "").trim() || str(w.status_error, "").trim();
                  const lastOutcome = str(w.last_poll_outcome, "").trim();
                  const intervalLabel = isHistoryOnly ? "-" : formatDurationFromSeconds(num(w.interval_secs, 0));
                  const timeoutLabel = isHistoryOnly ? "-" : formatDurationFromSeconds(num(w.timeout_secs, 0));
                  const notificationAttempts = asRecords(w.notification_attempts);
                  const latestAttempt = notificationAttempts.length
                    ? notificationAttempts[notificationAttempts.length - 1]
                    : null;
                  const latestAttemptAt = str(latestAttempt?.attempted_at, "").trim();
                  const latestAttemptLabel = latestAttemptAt
                    ? formatTimestampForHumans(latestAttemptAt).label
                    : "";
                  return (
                    <TableRow key={id}>
                      <TableCell sx={{ maxWidth: 260 }}>
                        <Typography variant="body2" noWrap title={str(w.description, "")}>
                          {str(w.description, "-")}
                        </Typography>
                        <Typography variant="caption" color="text.secondary" noWrap>
                          {str(w.notify_channel, "-")} • {formatDurationFromSeconds(num(w.timeout_secs, 0))}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" noWrap>{str(w.poll_action, "-")}</Typography>
                        <Typography variant="caption" color="text.secondary">{intervalLabel}</Typography>
                      </TableCell>
                      <TableCell sx={{ maxWidth: 200 }}>
                        <Typography variant="body2" noWrap title={watcherConditionSummary(w.condition)}>
                          {watcherConditionSummary(w.condition)}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Stack direction="row" spacing={0.75} alignItems="center" useFlexGap flexWrap="wrap">
                          <Chip
                            size="small"
                            label={watcherStatusLabel(rawStatus)}
                            color={watcherStatusColor(rawStatus)}
                          />
                          {isHistoryOnly ? (
                            <Chip size="small" variant="outlined" label="History" />
                          ) : null}
                        </Stack>
                      </TableCell>
                      <TableCell>
                        <Stack direction="row" spacing={0.75} alignItems="center">
                          <Typography variant="caption" color="text.secondary">
                            {num(w.poll_count, 0)}x
                          </Typography>
                          {lastOutcome ? (
                            <Chip size="small" variant="outlined" label={watcherPollOutcomeLabel(lastOutcome)} color={watcherPollOutcomeColor(lastOutcome)} sx={{ height: 18, fontSize: "0.65rem" }} />
                          ) : null}
                        </Stack>
                        <Typography variant="caption" color="text.secondary" noWrap>{lastPollLabel}</Typography>
                      </TableCell>
                      <TableCell align="right">
                        <RowOpsMenu
                          ariaLabel="Watcher actions"
                          actions={[
                            {
                              label: "Inspect",
                              onClick: () => { setError(null); setSelectedWatcherId(id); },
                            },
                            {
                              label: "Pause",
                              disabled: isHistoryOnly || !isActive || pauseMutation.isPending,
                              onClick: async () => { setError(null); try { await pauseMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Resume",
                              disabled: isHistoryOnly || !isPaused || resumeMutation.isPending,
                              onClick: async () => { setError(null); try { await resumeMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Stop",
                              tone: "warning",
                              disabled: isHistoryOnly || (!isActive && !isPaused) || cancelMutation.isPending,
                              onClick: async () => { setError(null); try { await cancelMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } },
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              disabled: deleteMutation.isPending,
                              onClick: async () => {
                                const ok = window.confirm("Delete this watcher? This cannot be undone.");
                                if (!ok) return;
                                setError(null);
                                try { await deleteMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); }
                              },
                            },
                          ]}
                        />
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
      </Box>
      )}

      <Dialog
        open={selectedWatcher != null}
        onClose={() => setSelectedWatcherId(null)}
        maxWidth="sm"
        fullWidth
        PaperProps={{
          sx: {
            background: "rgba(10, 15, 28, 0.97)",
            border: "1px solid rgba(47, 212, 255, 0.18)",
            backdropFilter: "blur(20px)",
          },
        }}
      >
        <DialogTitle sx={{ pb: 0.5 }}>
          <Typography variant="body1" sx={{ fontWeight: 600, lineHeight: 1.4 }}>
            {str(selectedWatcher?.description, "Watcher")}
          </Typography>
          <Stack direction="row" spacing={0.75} alignItems="center" sx={{ mt: 0.75 }}>
            <Chip
              size="small"
              label={watcherStatusLabel(selectedWatcher?.status)}
              color={watcherStatusColor(selectedWatcher?.status)}
            />
            {toBool(selectedWatcher?.history_only) ? (
              <Chip size="small" variant="outlined" label="History" />
            ) : null}
            {str(selectedWatcher?.last_poll_outcome, "").trim() ? (
              <Chip
                size="small"
                variant="outlined"
                label={watcherPollOutcomeLabel(selectedWatcher?.last_poll_outcome)}
                color={watcherPollOutcomeColor(selectedWatcher?.last_poll_outcome)}
              />
            ) : null}
            <Typography variant="caption" color="text.secondary" sx={{ ml: "auto !important" }}>
              {str(selectedWatcher?.id, "-").slice(0, 12)}
            </Typography>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {/* Config summary */}
            <Stack spacing={0.75}>
              {[
                { label: "Action", value: str(selectedWatcher?.poll_action, "-") },
                { label: "Interval", value: toBool(selectedWatcher?.history_only) ? "-" : formatDurationFromSeconds(num(selectedWatcher?.interval_secs, 0)) },
                { label: "Timeout", value: toBool(selectedWatcher?.history_only) ? "-" : formatDurationFromSeconds(num(selectedWatcher?.timeout_secs, 0)) },
                { label: "Notify", value: str(selectedWatcher?.notify_channel, "-") },
                { label: "Polls", value: String(num(selectedWatcher?.poll_count, 0)) },
                { label: "Created", value: humanTs(str(selectedWatcher?.created_at, "-")).label, tip: humanTs(str(selectedWatcher?.created_at, "-")).tip },
                ...(str(selectedWatcher?.last_poll_at, "").trim() ? [{ label: "Last poll", value: humanTs(str(selectedWatcher?.last_poll_at, "")).label, tip: humanTs(str(selectedWatcher?.last_poll_at, "")).tip }] : []),
              ].map((row) => (
                <Stack key={row.label} direction="row" spacing={1.5} alignItems="baseline">
                  <Typography variant="caption" color="text.secondary" sx={{ minWidth: 70, flexShrink: 0 }}>
                    {row.label}
                  </Typography>
                  <Typography variant="body2" title={(row as { tip?: string }).tip || ""}>
                    {row.value}
                  </Typography>
                </Stack>
              ))}
            </Stack>

            {/* Condition */}
            {watcherConditionSummary(selectedWatcher?.condition) ? (
              <Box>
                <Typography variant="caption" color="text.secondary">Condition</Typography>
                <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                  {watcherConditionSummary(selectedWatcher?.condition)}
                </Typography>
              </Box>
            ) : null}

            {/* On trigger */}
            {str(selectedWatcher?.on_trigger, "").trim() ? (
              <Box>
                <Typography variant="caption" color="text.secondary">On trigger</Typography>
                <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                  {str(selectedWatcher?.on_trigger, "-")}
                </Typography>
              </Box>
            ) : null}

            {/* Error (only if present) */}
            {(str(selectedWatcher?.last_error, "").trim() || str(selectedWatcher?.status_error, "").trim()) ? (
              <Alert severity="error" variant="outlined" sx={{ py: 0.5 }}>
                <Typography variant="body2" sx={{ fontFamily: "monospace", fontSize: "0.8rem", wordBreak: "break-word" }}>
                  {str(selectedWatcher?.last_error, "").trim() || str(selectedWatcher?.status_error, "").trim()}
                </Typography>
              </Alert>
            ) : null}

            {/* Latest poll payload (only if present) */}
            {(() => {
              const payloadText = watcherPayloadText(selectedWatcher?.last_result).trim();
              return payloadText ? (
                <Box>
                  <Typography variant="caption" color="text.secondary">Latest poll result</Typography>
                  <Typography
                    component="pre"
                    variant="body2"
                    sx={{
                      mt: 0.5,
                      mb: 0,
                      p: 1,
                      maxHeight: 160,
                      overflow: "auto",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                      fontSize: "0.78rem",
                      background: "rgba(0,0,0,0.3)",
                      borderRadius: 1,
                    }}
                  >
                    {payloadText}
                  </Typography>
                </Box>
              ) : null;
            })()}

            {/* Trigger payload (only if present) */}
            {(() => {
              const triggerText = watcherPayloadText(selectedWatcher?.trigger_result).trim();
              return triggerText ? (
                <Box>
                  <Typography variant="caption" color="text.secondary">Trigger payload</Typography>
                  <Typography
                    component="pre"
                    variant="body2"
                    sx={{
                      mt: 0.5,
                      mb: 0,
                      p: 1,
                      maxHeight: 160,
                      overflow: "auto",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                      fontSize: "0.78rem",
                      background: "rgba(0,0,0,0.3)",
                      borderRadius: 1,
                    }}
                  >
                    {triggerText}
                  </Typography>
                </Box>
              ) : null;
            })()}

            {/* Notification attempts (only if present) */}
            {asRecords(selectedWatcher?.notification_attempts).length > 0 ? (
              <Box>
                <Typography variant="caption" color="text.secondary" sx={{ mb: 0.5, display: "block" }}>
                  Notifications ({asRecords(selectedWatcher?.notification_attempts).length})
                </Typography>
                <Stack spacing={0.5}>
                  {asRecords(selectedWatcher?.notification_attempts)
                    .slice()
                    .reverse()
                    .map((attempt, idx) => {
                      const message = str(attempt.message, "").trim();
                      const errorText = str(attempt.error, "").trim();
                      return (
                        <Box
                          key={`${str(attempt.attempted_at, String(idx))}-${idx}`}
                          sx={{ borderBottom: "1px solid rgba(62,143,214,0.08)", pb: 0.75, mb: 0.25 }}
                        >
                          <Stack direction="row" spacing={1} alignItems="center" sx={{ mb: 0.35 }}>
                            <Chip
                              size="small"
                              label={toBool(attempt.success) ? "sent" : "failed"}
                              color={toBool(attempt.success) ? "success" : "error"}
                              variant="outlined"
                              sx={{ height: 20, fontSize: "0.7rem" }}
                            />
                            <Typography variant="caption" color="text.secondary">
                              {str(attempt.attempted_at, "").trim()
                                ? formatTimestampForHumans(str(attempt.attempted_at, "")).label
                                : "-"}
                            </Typography>
                            <Typography variant="caption" color="text.secondary">
                              {str(attempt.channel, "")}
                            </Typography>
                          </Stack>
                          {errorText ? (
                            <Typography variant="caption" color="error" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
                              {errorText}
                            </Typography>
                          ) : message ? (
                            <Typography variant="caption" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word", color: "text.secondary" }}>
                              {message}
                            </Typography>
                          ) : null}
                        </Box>
                      );
                    })}
                </Stack>
              </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSelectedWatcherId(null)}>Close</Button>
        </DialogActions>
      </Dialog>

      {watchersQ.error || error ? (
        <Alert severity="error">{error || errMessage(watchersQ.error)}</Alert>
      ) : null}
    </Stack>
  );
}

function SettingsManager({
  autoRefresh,
  initialTab,
  hideSettingsNav
}: {
  autoRefresh: boolean;
  initialTab?: number | null;
  hideSettingsNav?: boolean;
}) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState(() => {
    if (typeof initialTab === "number") return initialTab;
    if (typeof window === "undefined") return 0;
    const raw = new URLSearchParams(window.location.search).get("settings_tab");
    if (!raw) return 0;
    const normalized = raw.trim().toLowerCase();
    const byName: Record<string, number> = {
      quick: 0,
      setup: 0,
      models: 1,
      channels: 2,
      integrations: 2,
      media: 3,
      security: 4,
      advanced: 5,
      moltbook: 7,
      mcp: 8,
      memory: 12,
      system: 9,
      trace: 11,
      evolution: 13
    };
    if (normalized in byName) return byName[normalized];
    const asNumber = Number(normalized);
    if (Number.isFinite(asNumber) && Math.trunc(asNumber) === 10) return 2;
    return Number.isFinite(asNumber) ? Math.max(0, Math.trunc(asNumber)) : 0;
  });
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [modelConnectivityWarning, setModelConnectivityWarning] = useState<string | null>(null);
  const [initialized, setInitialized] = useState(false);
  const [apiKeyRevealed, setApiKeyRevealed] = useState(false);
  const [apiKeyNowMs, setApiKeyNowMs] = useState(() => Date.now());
  const [secCurrentPassword, setSecCurrentPassword] = useState("");
  const [secNewPassword, setSecNewPassword] = useState("");
  const [secConfirmPassword, setSecConfirmPassword] = useState("");
  const [showPasswordInputs, setShowPasswordInputs] = useState(false);
  const [passwordDialogMode, setPasswordDialogMode] = useState<PasswordDialogMode | null>(null);
  const [vaultPassword, setVaultPassword] = useState("");
  const [vaultEditorOpen, setVaultEditorOpen] = useState(false);
  const [vaultEditorKey, setVaultEditorKey] = useState("");
  const [vaultEditorValue, setVaultEditorValue] = useState("");
  const [showVaultSecretValue, setShowVaultSecretValue] = useState(false);
  const [securityLogsDialogOpen, setSecurityLogsDialogOpen] = useState(false);
  const [selectedSecurityLog, setSelectedSecurityLog] = useState<JsonRecord | null>(null);
  const [selectedPulseEvent, setSelectedPulseEvent] = useState<JsonRecord | null>(null);
  const [activePulseFixId, setActivePulseFixId] = useState<string | null>(null);
  const [selectedMoltbookEvent, setSelectedMoltbookEvent] = useState<JsonRecord | null>(null);
  const [pulsePollState, setPulsePollState] = useState<{ baselineEventId: string; deadlineAt: number } | null>(null);
  const [moltbookPollState, setMoltbookPollState] = useState<{ baselineEventId: string; deadlineAt: number } | null>(null);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(getDeveloperModeEnabled);
  const [trustPresetId, setTrustPresetId] = useState(TRUST_APPROVAL_PRESETS[0]?.id ?? "run_terminal_command");
  const [trustPresetDetail, setTrustPresetDetail] = useState("ls -la");
  const [trustUseAdvancedInput, setTrustUseAdvancedInput] = useState(false);
  const [trustActionKind, setTrustActionKind] = useState("shell");
  const [trustPayloadJson, setTrustPayloadJson] = useState("{}");
  const [trustResult, setTrustResult] = useState<JsonRecord | null>(null);
  const [tunnelSelectedProviderId, setTunnelSelectedProviderId] = useState("");
  const [tunnelDraftValues, setTunnelDraftValues] = useState<Record<string, string>>({});
  const [showTunnelAdvanced, setShowTunnelAdvanced] = useState(false);
  const [tunnelPanelNotice, setTunnelPanelNotice] = useState<{
    severity: "success" | "info";
    text: string;
  } | null>(null);
  const [resumeTunnelStartAfterPassword, setResumeTunnelStartAfterPassword] = useState(false);

  useEffect(() => {
    if (typeof initialTab === "number" && tab !== initialTab) {
      setTab(initialTab);
    }
  }, [initialTab, tab]);

  useEffect(() => {
    const refreshDeveloperMode = () => setDeveloperModeEnabledState(getDeveloperModeEnabled());
    window.addEventListener(DEVELOPER_MODE_EVENT, refreshDeveloperMode as EventListener);
    window.addEventListener("storage", refreshDeveloperMode);
    return () => {
      window.removeEventListener(DEVELOPER_MODE_EVENT, refreshDeveloperMode as EventListener);
      window.removeEventListener("storage", refreshDeveloperMode);
    };
  }, []);

  useEffect(() => {
    if (!success) return;
    const timer = window.setTimeout(() => setSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [success]);

  useEffect(() => {
    const timer = window.setInterval(() => setApiKeyNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const settingsQ = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.rawGet("/settings"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const mediaQ = useQuery({
    queryKey: ["settings-media"],
    queryFn: () => api.rawGet("/settings/media"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const modelsQ = useQuery({
    queryKey: ["models"],
    queryFn: () => api.rawGet("/models"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const apiKeyQ = useQuery({
    queryKey: ["settings-api-key"],
    queryFn: () => api.rawGet("/settings/api-key"),
    refetchInterval: 10000,
    refetchIntervalInBackground: true
  });
  const tunnelQ = useQuery({
    queryKey: ["tunnel-status"],
    queryFn: () => api.rawGet("/tunnel/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const tunnelProvidersQ = useQuery({
    queryKey: ["tunnel-providers"],
    queryFn: () => api.rawGet("/tunnel/providers"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const securityStatusQ = useQuery({
    queryKey: ["security-status"],
    queryFn: () => api.rawGet("/security/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const securityLogsQ = useQuery({
    queryKey: ["settings-security-logs-dialog"],
    queryFn: () => api.rawGet("/security/logs?limit=80"),
    enabled: tab === 4 && securityLogsDialogOpen,
    refetchInterval: securityLogsDialogOpen && autoRefresh ? REFRESH_MS : false
  });
  const observabilityLogsQ = useQuery({
    queryKey: ["settings-observability-logs"],
    queryFn: () => api.rawGet("/settings/observability/logs?limit=40"),
    enabled: tab === 5,
    refetchInterval: tab === 5 && autoRefresh ? REFRESH_MS : false
  });
  const vaultSecretsQ = useQuery({
    queryKey: ["settings-secrets"],
    queryFn: () => api.rawGet("/settings/secrets"),
    refetchInterval: false
  });
  const pulseQ = useQuery({
    queryKey: ["arkpulse-log"],
    queryFn: () => api.rawGet("/arkpulse?limit=40"),
    refetchInterval: pulsePollState ? 2000 : autoRefresh ? REFRESH_MS : false
  });
  const moltbookStatusQ = useQuery({
    queryKey: ["moltbook-status"],
    queryFn: () => api.rawGet("/moltbook/status"),
    refetchInterval: moltbookPollState ? 2000 : autoRefresh ? REFRESH_MS : false
  });
  const moltbookLogQ = useQuery({
    queryKey: ["moltbook-log"],
    queryFn: () => api.rawGet("/moltbook/log?limit=500"),
    refetchInterval: moltbookPollState ? 2000 : autoRefresh ? REFRESH_MS : false
  });
  const evolutionQ = useQuery({
    queryKey: ["settings-evolution"],
    queryFn: () => api.rawGet("/settings/evolution"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const evolutionDevQ = useQuery({
    queryKey: ["settings-evolution-dev"],
    queryFn: () => api.rawGet("/settings/evolution/dev?limit=5000"),
    enabled: developerModeEnabled && tab === 13,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const settings = asRecord(settingsQ.data);
  const observabilitySettings = asRecord(settings.observability);
  const media = asRecord(mediaQ.data);
  const modelsPayload = asRecord(modelsQ.data);
  const evolution = asRecord(evolutionQ.data);
  const evolutionCanary = asRecord(evolution.canary);
  const evolutionDev = asRecord(evolutionDevQ.data);
  const evolutionStrategyMetrics = pickRecords(evolutionDev, "strategy_metrics");
  const evolutionLineage = pickRecords(evolutionDev, "lineage_recent");
  const observabilityLogsPayload = asRecord(observabilityLogsQ.data);
  const observabilityLogs = pickRecords(observabilityLogsPayload, "logs");
  const observabilityIssues = Array.isArray(observabilityLogsPayload.issues)
    ? observabilityLogsPayload.issues.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    : [];
  const configuredProviders = useMemo(() => { 
    const raw = media.configured; 
    if (!Array.isArray(raw)) return []; 
    return raw.filter((x) => typeof x === "string") as string[]; 
  }, [media.configured]); 
 

  const [form, setForm] = useState({
    bot_name: "AgentArk",
    personality: "friendly",
    timezone: "",
    language: "English",
    tone: "",
    email_format: "",
    daily_brief_enabled: false,
    daily_brief_time: "09:00",
    daily_brief_channel: "telegram",
    smart_routing: true,
    app_deploy_model_id: "",

    llm_provider: "ollama",
    llm_model: "",
    llm_base_url: "http://localhost:11434",
    llm_api_key: "",

    llm_fallback_provider: "",
    llm_fallback_model: "",
    llm_fallback_base_url: "",
    llm_fallback_api_key: "",

    telegram_enabled: false,
    telegram_bot_token: "",
    telegram_allowed_users_csv: "",

    whatsapp_enabled: false,
    whatsapp_mode: "baileys",
    whatsapp_access_token: "",
    whatsapp_phone_number_id: "",
    whatsapp_verify_token: "agentark_verify",
    whatsapp_bridge_url: "http://127.0.0.1:8999",
    whatsapp_dm_policy: "pairing",
    whatsapp_allowed_numbers_csv: "",

    auto_approve_csv: "",

    default_image_provider: "",
    image_model: "",
    fallback_image_provider: "",
    default_video_provider: "",
    fallback_video_provider: "",
    media_provider_keys_json: "",
    media_key_replicate: "",
    media_key_fal: "",
    media_key_stability_ai: "",
    media_key_together: "",
    media_key_openai_dalle: "",
    media_key_google_gemini: "",
    media_key_runway: "",
    media_key_luma: "",

    search_primary: "lightpanda",
    search_fallback1: "playwright",
    search_fallback2: "duckduckgo",
    search_serper_key: "",
    search_searxng_url: "",
    search_brave_key: "",

    moltbook_api_key: "",
    moltbook_enabled: false,
    moltbook_mode: "autopost",
    moltbook_sync_frequency: "every_12_hours",
    moltbook_write_enabled: true,
    moltbook_defer_when_busy: true,

    observability_enabled: false,
    observability_provider: "langtrace",
    observability_endpoint: "",
    observability_service_name: "agentark",
    observability_header_name: "x-api-key",
    observability_privacy_mode: "metadata_only",
    observability_auth_token: ""
  });
  const [savedFormSnapshot, setSavedFormSnapshot] = useState("");

  function snapshotSettingsForm(value: typeof form): string {
    return JSON.stringify(value);
  }

  function snapshotObservabilityForm(value: typeof form): string {
    return JSON.stringify({
      observability_enabled: value.observability_enabled,
      observability_provider: value.observability_provider,
      observability_endpoint: value.observability_endpoint,
      observability_service_name: value.observability_service_name,
      observability_header_name: value.observability_header_name,
      observability_privacy_mode: value.observability_privacy_mode,
      observability_auth_token: value.observability_auth_token
    });
  }

  function parseSavedSettingsSnapshot(): typeof form | null {
    if (!savedFormSnapshot.trim()) return null;
    try {
      return JSON.parse(savedFormSnapshot) as typeof form;
    } catch {
      return null;
    }
  }

  const effectiveDirty = dirty && snapshotSettingsForm(form) !== savedFormSnapshot;

  function setField<K extends keyof typeof form>(key: K, value: (typeof form)[K]) {
    setForm((prev) => ({ ...prev, [key]: value }));
    setDirty(true);
    setSuccess(null);
  }

  function parseCsvList(csv: string): string[] {
    return csv
      .split(/[,\\n]/g)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }

  function parseTelegramUsers(csv: string): number[] {
    const parts = parseCsvList(csv);
    const out: number[] = [];
    for (const p of parts) {
      const n = Number(p);
      if (!Number.isFinite(n)) throw new Error(`Invalid Telegram user id: '${p}'`);
      out.push(n);
    }
    return out;
  }

  function parseMediaProvidersJson(input: string): Record<string, string> {
    const trimmed = input.trim();
    if (!trimmed) return {};
    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      throw new Error("Media provider keys must be valid JSON (object mapping provider -> api_key).");
    }
    if (!isRecord(parsed)) throw new Error("Media provider keys must be a JSON object.");
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof v !== "string") throw new Error(`Media provider key for '${k}' must be a string.`);
      out[k] = v;
    }
    return out;
  }

  function recordToStringMap(value: unknown): Record<string, string> {
    const raw = asRecord(value);
    const out: Record<string, string> = {};
    for (const [key, entry] of Object.entries(raw)) {
      if (entry === null || entry === undefined) continue;
      out[key] = String(entry);
    }
    return out;
  }

  function syncTunnelDraftFromPayload(payloadLike: unknown, preferredProviderId?: string) {
    const payload = asRecord(payloadLike);
    const providers = pickRecords(payload, "providers");
    if (providers.length === 0) return;
    const preferred =
      preferredProviderId?.trim() ||
      str(payload.selected_provider, "").trim() ||
      str(providers[0]?.id, "").trim();
    const selected =
      providers.find((provider) => str(provider.id, "").trim() === preferred) || providers[0];
    const nextId = str(selected.id, "").trim();
    if (!nextId) return;
    setTunnelSelectedProviderId(nextId);
    setTunnelDraftValues(recordToStringMap(asRecord(selected.config_values)));
  }

  function hydrateFromServer() {
    const tgUsers = Array.isArray(settings.telegram_allowed_users) ? (settings.telegram_allowed_users as unknown[]) : [];
    const waNums = Array.isArray(settings.whatsapp_allowed_numbers) ? (settings.whatsapp_allowed_numbers as unknown[]) : [];
    const autoApprove = Array.isArray(settings.auto_approve) ? (settings.auto_approve as unknown[]) : [];
    const modelPool = Array.isArray(settings.model_pool) ? (settings.model_pool as unknown[]) : [];
    const modelPoolIds = modelPool
      .map((slot) => str(asRecord(slot).id, "").trim())
      .filter((id) => id.length > 0);
    const appDeployModelIdRaw = str(settings.app_deploy_model_id, "").trim();
    const appDeployModelId = modelPoolIds.includes(appDeployModelIdRaw) ? appDeployModelIdRaw : "";

    const nextForm = ({
      ...form,
      bot_name: str(settings.bot_name, form.bot_name),
      personality: str(settings.personality, form.personality),
      timezone: str(settings.timezone, ""),
      language: str(settings.language, form.language),
      tone: str(settings.tone, form.tone),
      email_format: str(settings.email_format, form.email_format),
      daily_brief_enabled: toBool(settings.daily_brief_enabled),
      daily_brief_time: str(settings.daily_brief_time, "09:00"),
      daily_brief_channel: str(settings.daily_brief_channel, "telegram"),
      smart_routing: toBool(settings.smart_routing),
      app_deploy_model_id: appDeployModelId,

      llm_provider: str(settings.llm_provider, "ollama"),
      llm_model: str(settings.llm_model, ""),
      llm_base_url: str(settings.llm_base_url, "http://localhost:11434"),
      llm_api_key: "",

      llm_fallback_provider: str(settings.llm_fallback_provider, ""),
      llm_fallback_model: str(settings.llm_fallback_model, ""),
      llm_fallback_base_url: str(settings.llm_fallback_base_url, ""),
      llm_fallback_api_key: "",

      telegram_enabled: toBool(settings.telegram_enabled),
      telegram_bot_token: "",
      telegram_allowed_users_csv: tgUsers
        .map((v) => (typeof v === "number" ? String(v) : typeof v === "string" ? v : ""))
        .filter((v) => v.trim().length > 0)
        .join(", "),

      whatsapp_enabled: toBool(settings.whatsapp_enabled),
      whatsapp_mode: str(settings.whatsapp_mode, "baileys"),
      whatsapp_access_token: "",
      whatsapp_phone_number_id: str(settings.whatsapp_phone_number_id, ""),
      whatsapp_verify_token: str(settings.whatsapp_verify_token, "agentark_verify"),
      whatsapp_bridge_url: str(settings.whatsapp_bridge_url, "http://127.0.0.1:8999"),
      whatsapp_dm_policy: str(settings.whatsapp_dm_policy, "pairing"),
      whatsapp_allowed_numbers_csv: waNums
        .map((v) => (typeof v === "string" ? v : ""))
        .filter((v) => v.trim().length > 0)
        .join(", "),

      auto_approve_csv: autoApprove
        .map((v) => (typeof v === "string" ? v : ""))
        .filter((v) => v.trim().length > 0)
        .join(", "),

      default_image_provider: str(media.default_image_provider ?? settings.default_image_provider, ""),
      image_model: str(media.image_model ?? settings.image_model, ""),
      fallback_image_provider: str(media.fallback_image_provider ?? settings.fallback_image_provider, ""),
      default_video_provider: str(media.default_video_provider ?? settings.default_video_provider, ""),
      fallback_video_provider: str(media.fallback_video_provider ?? settings.fallback_video_provider, ""),
      media_provider_keys_json: "",
      media_key_replicate: "",
      media_key_fal: "",
      media_key_stability_ai: "",
      media_key_together: "",
      media_key_openai_dalle: "",
      media_key_google_gemini: "",
      media_key_runway: "",
      media_key_luma: "",

      search_primary: str(settings.search_primary, "lightpanda"),
      search_fallback1: str(settings.search_fallback1, "playwright"),
      search_fallback2: str(settings.search_fallback2, "duckduckgo"),
      search_serper_key: "",
      search_searxng_url: str(settings.search_searxng_url, ""),
      search_brave_key: "",

      moltbook_api_key: "",
      moltbook_enabled: toBool(settings.moltbook_enabled),
      moltbook_mode: str(settings.moltbook_mode, "autopost"),
      moltbook_sync_frequency: str(settings.moltbook_sync_frequency, "every_12_hours"),
      moltbook_write_enabled:
        settings.moltbook_write_enabled == null ? true : toBool(settings.moltbook_write_enabled),
      moltbook_defer_when_busy: toBool(settings.moltbook_defer_when_busy),

      observability_enabled: toBool(observabilitySettings.enabled),
      observability_provider: str(observabilitySettings.provider, "langtrace"),
      observability_endpoint: str(observabilitySettings.endpoint, ""),
      observability_service_name: str(observabilitySettings.service_name, "agentark"),
      observability_header_name: str(observabilitySettings.header_name, "x-api-key"),
      observability_privacy_mode: str(observabilitySettings.privacy_mode, "metadata_only"),
      observability_auth_token: ""
    });

    setForm(nextForm);
    setSavedFormSnapshot(snapshotSettingsForm(nextForm));

    setDirty(false);
    setError(null);
    setSuccess(null);
  }

  // Initialize form from backend once; keep defaults if backend is down.
  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.isSuccess) return;
    hydrateFromServer();
    setInitialized(true);
    setDirty(false);
  }, [initialized, settingsQ.isSuccess, settingsQ.dataUpdatedAt]);

  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.data || !mediaQ.data) return;
    hydrateFromServer();
    setInitialized(true);
    setDirty(false);
  }, [initialized, settingsQ.data, mediaQ.data]); // eslint-disable-line react-hooks/exhaustive-deps

  // Safety: clear dirty once after hydration settles (handles race between effects)
  const hydrationDirtyCleared = useRef(false);
  useEffect(() => {
    if (initialized && !hydrationDirtyCleared.current) {
      hydrationDirtyCleared.current = true;
      setDirty(false);
    }
  }, [initialized]);

  const saveMutation = useMutation({
    mutationFn: async () => {
      const mediaKeys = parseMediaProvidersJson(form.media_provider_keys_json);
      const mediaProviders: Record<string, string> = { ...mediaKeys };
      const mediaFieldKeys: Array<[string, string]> = [
        ["replicate", form.media_key_replicate],
        ["fal", form.media_key_fal],
        ["stability_ai", form.media_key_stability_ai],
        ["together", form.media_key_together],
        ["openai_dalle", form.media_key_openai_dalle],
        ["google_gemini", form.media_key_google_gemini],
        ["runway", form.media_key_runway],
        ["luma", form.media_key_luma]
      ];
      for (const [k, v] of mediaFieldKeys) {
        const trimmed = (v || "").trim();
        if (trimmed) {
          mediaProviders[k] = trimmed;
          if (k === "openai_dalle") mediaProviders["openai_sora"] = trimmed;
          if (k === "google_gemini") mediaProviders["google_veo"] = trimmed;
        }
      }
      const payload: Record<string, unknown> = {
        bot_name: form.bot_name || "AgentArk",
        personality: form.personality || "friendly",
        // Send empty strings to clear fields (null means "skip update" on backend).
        timezone: form.timezone,
        language: form.language,
        tone: form.tone,
        email_format: form.email_format,
        daily_brief_enabled: form.daily_brief_enabled,
        daily_brief_time: form.daily_brief_time || "09:00",
        daily_brief_channel: form.daily_brief_channel || "telegram",
        smart_routing: form.smart_routing,
        app_deploy_model_id: form.app_deploy_model_id,

        llm_provider: form.llm_provider,
        llm_model: form.llm_model,
        llm_base_url: form.llm_base_url || null,
        llm_api_key: form.llm_api_key || null,

        llm_fallback_provider: form.llm_fallback_provider || null,
        llm_fallback_model: form.llm_fallback_model || null,
        llm_fallback_base_url: form.llm_fallback_base_url || null,
        llm_fallback_api_key: form.llm_fallback_api_key || null,

        telegram_enabled: !!form.telegram_enabled,
        telegram_bot_token: form.telegram_bot_token || null,
        telegram_allowed_users: parseTelegramUsers(form.telegram_allowed_users_csv),

        whatsapp_enabled: !!form.whatsapp_enabled,
        whatsapp_mode: form.whatsapp_mode || null,
        whatsapp_access_token: form.whatsapp_access_token || null,
        whatsapp_phone_number_id: form.whatsapp_phone_number_id || null,
        whatsapp_verify_token: form.whatsapp_verify_token || null,
        whatsapp_bridge_url: form.whatsapp_bridge_url || null,
        whatsapp_dm_policy: form.whatsapp_dm_policy || null,
        whatsapp_allowed_numbers: parseCsvList(form.whatsapp_allowed_numbers_csv),

        auto_approve: parseCsvList(form.auto_approve_csv),

        media_providers: mediaProviders,
        default_image_provider: form.default_image_provider || null,
        image_model: form.image_model || null,
        fallback_image_provider: form.fallback_image_provider || null,
        default_video_provider: form.default_video_provider || null,
        fallback_video_provider: form.fallback_video_provider || null,

        search_primary: form.search_primary || null,
        search_fallback1: form.search_fallback1 || null,
        search_fallback2: form.search_fallback2 || null,
        search_serper_key: form.search_serper_key || null,
        search_searxng_url: form.search_searxng_url || null,
        search_brave_key: form.search_brave_key || null,

        moltbook_api_key: form.moltbook_api_key || null,
        moltbook_enabled: form.moltbook_enabled,
        moltbook_mode: form.moltbook_mode || null,
        moltbook_sync_frequency: form.moltbook_sync_frequency || null,
        moltbook_write_enabled: form.moltbook_write_enabled,
        moltbook_defer_when_busy: form.moltbook_defer_when_busy,

        observability: {
          enabled: form.observability_enabled,
          provider: form.observability_provider || "langtrace",
          endpoint: form.observability_endpoint || "",
          service_name: form.observability_service_name || "agentark",
          header_name: form.observability_header_name || "x-api-key",
          privacy_mode: form.observability_privacy_mode || "metadata_only",
          // Only send auth_token when user entered a new value — blank means "keep existing"
          ...(form.observability_auth_token.trim() ? { auth_token: form.observability_auth_token } : {})
        }
      };

      return await api.rawPost("/settings", payload);
    },
    onSuccess: async () => {
      setError(null);
      setSuccess("Saved settings.");
      setDirty(false);
      const savedSnapshot = parseSavedSettingsSnapshot();
      const observabilityChanged =
        !savedSnapshot ||
        snapshotObservabilityForm(form) !== snapshotObservabilityForm(savedSnapshot);
      const shouldTestObservability =
        observabilityChanged &&
        form.observability_enabled &&
        form.observability_endpoint.trim().length > 0 &&
        (
          form.observability_auth_token.trim().length > 0 ||
          toBool(observabilitySettings.auth_token_configured)
        );
      setForm((prev) => {
        const nextForm = {
          ...prev,
        llm_api_key: "",
        llm_fallback_api_key: "",
        telegram_bot_token: "",
        whatsapp_access_token: "",
        media_provider_keys_json: "",
        media_key_replicate: "",
        media_key_fal: "",
        media_key_stability_ai: "",
        media_key_together: "",
        media_key_openai_dalle: "",
        media_key_google_gemini: "",
        media_key_runway: "",
        media_key_luma: "",
        search_serper_key: "",
        search_brave_key: "",
        moltbook_api_key: "",
        observability_auth_token: ""
        };
        setSavedFormSnapshot(snapshotSettingsForm(nextForm));
        return nextForm;
      });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-media"] });
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-observability-logs"] });
      if (shouldTestObservability) {
        try {
          await api.rawPost("/settings/observability/test", {});
          await queryClient.invalidateQueries({ queryKey: ["settings-observability-logs"] });
          setSuccess("Saved settings. Sent a test observability trace.");
        } catch (e) {
          await queryClient.invalidateQueries({ queryKey: ["settings-observability-logs"] });
          setSuccess("Saved settings.");
          setError(`Observability test failed after save: ${errMessage(e)}`);
        }
      }
    },
    onError: (e) => {
      setSuccess(null);
      setError(errMessage(e));
    }
  });

  const runMoltbookMutation = useMutation({
    mutationFn: () => api.rawPost("/moltbook/run", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["moltbook-log"] });
      await queryClient.invalidateQueries({ queryKey: ["moltbook-status"] });
    }
  });
  const testObservabilityMutation = useMutation({
    mutationFn: () => api.rawPost("/settings/observability/test", {}),
    onSuccess: async () => {
      setError(null);
      setSuccess("Sent a test observability trace.");
      await queryClient.invalidateQueries({ queryKey: ["settings-observability-logs"] });
    },
    onError: (e) => {
      setSuccess(null);
      setError(errMessage(e));
    }
  });
  const updateEvolutionSettingsMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/settings/evolution", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution-dev"] });
    }
  });
  const runEvolutionDevActionMutation = useMutation({
    mutationFn: (action: string) => api.rawPost("/settings/evolution/dev/action", { action }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution-dev"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
      await queryClient.invalidateQueries({ queryKey: ["trace-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["trace-detail"] });
    }
  });

  const modelSlotsLive = useMemo(() => pickRecords(modelsPayload, "models"), [modelsPayload]);
  const settingsPayloadError = str(settings.error, "").trim();
  const modelsPayloadError = str(modelsPayload.error, "").trim();
  const [stableModelSlots, setStableModelSlots] = useState<JsonRecord[]>([]);
  const [stableSettingsComplete, setStableSettingsComplete] = useState(false);
  const consecutiveEmptyModelSnapshotsRef = useRef(0);
  const consecutiveIncompleteSettingsRef = useRef(0);

  useEffect(() => {
    if (modelSlotsLive.length > 0) {
      consecutiveEmptyModelSnapshotsRef.current = 0;
      setStableModelSlots(modelSlotsLive);
      return;
    }
    if (modelsQ.isFetching || modelsQ.isError || modelsPayloadError) return;
    if (!modelsQ.isSuccess) return;
    consecutiveEmptyModelSnapshotsRef.current += 1;
    if (consecutiveEmptyModelSnapshotsRef.current >= 2) {
      setStableModelSlots([]);
    }
  }, [modelSlotsLive, modelsQ.isFetching, modelsQ.isError, modelsQ.isSuccess, modelsPayloadError]);

  const modelSlots = useMemo(() => {
    if (modelSlotsLive.length > 0) return modelSlotsLive;
    if (stableModelSlots.length > 0) return stableModelSlots;
    return modelSlotsLive;
  }, [modelSlotsLive, stableModelSlots]);

  useEffect(() => {
    const hasSnapshotIssue =
      !!settingsPayloadError || !!modelsPayloadError || settingsQ.isError || modelsQ.isError;
    const computedComplete = toBool(settings.settings_complete) || modelSlotsLive.length > 0 || stableModelSlots.length > 0;
    if (computedComplete) {
      consecutiveIncompleteSettingsRef.current = 0;
      if (!stableSettingsComplete) setStableSettingsComplete(true);
      return;
    }
    if (hasSnapshotIssue || settingsQ.isFetching || modelsQ.isFetching || !settingsQ.isSuccess || !modelsQ.isSuccess) {
      return;
    }
    consecutiveIncompleteSettingsRef.current += 1;
    if (consecutiveIncompleteSettingsRef.current >= 2 && stableSettingsComplete) {
      setStableSettingsComplete(false);
    }
  }, [
    settings.settings_complete,
    modelSlotsLive.length,
    stableModelSlots.length,
    settingsPayloadError,
    modelsPayloadError,
    settingsQ.isError,
    modelsQ.isError,
    settingsQ.isFetching,
    modelsQ.isFetching,
    settingsQ.isSuccess,
    modelsQ.isSuccess,
    stableSettingsComplete
  ]);

  const moltbookEvents = pickRecords(moltbookLogQ.data, "events");

  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [modelEditingId, setModelEditingId] = useState<string | null>(null);
  const [modelAdvancedOpen, setModelAdvancedOpen] = useState(false);
  const [openaiSubAuth, setOpenaiSubAuth] = useState<{
    message: string;
    authUrl: string;
    deviceCode: string;
    running: boolean;
    openedBrowser: boolean;
  } | null>(null);
  const [codexAuthBusy, setCodexAuthBusy] = useState(false);
  const [modelForm, setModelForm] = useState({
    label: "",
    role: "primary",
    provider: "ollama",
    model: "",
    base_url: OLLAMA_DEFAULT_BASE_URL,
    api_key: "",
    enabled: true
  });
  const previousModelProviderRef = useRef(modelForm.provider);

  useEffect(() => {
    if (modelForm.role !== "research") return;
    setModelForm((p) => ({
      ...p,
      provider: "openrouter",
      model: p.model || "perplexity/sonar-deep-research",
      base_url: p.base_url || OPENROUTER_DEFAULT_BASE_URL
    }));
  }, [modelForm.role]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const prevProvider = previousModelProviderRef.current;
    if (prevProvider === modelForm.provider) return;
    previousModelProviderRef.current = modelForm.provider;
    setOpenaiSubAuth(null);

    setModelForm((p) => {
      const current = p.base_url.trim();
      let next = p.base_url;
      if (p.provider === "openrouter") {
        if (!current || current === OLLAMA_DEFAULT_BASE_URL) next = OPENROUTER_DEFAULT_BASE_URL;
      } else if (p.provider === "ollama") {
        if (!current || current === OPENROUTER_DEFAULT_BASE_URL) next = OLLAMA_DEFAULT_BASE_URL;
      } else if (
        (p.provider === "openai" ||
          p.provider === "anthropic" ||
          p.provider === "openai-subscription" ||
          p.provider === "codex-cli") &&
        (current === OLLAMA_DEFAULT_BASE_URL || current === OPENROUTER_DEFAULT_BASE_URL)
      ) {
        next = "";
      }
      return next === p.base_url ? p : { ...p, base_url: next };
    });
  }, [modelForm.provider]);

  const discoverModelsQ = useQuery({
    queryKey: ["discover-models", modelForm.provider, modelForm.api_key, modelForm.base_url],
    queryFn: async () => {
      const p = modelForm.provider;
      if (p === "openai-compatible") return [] as string[];
      const params = new URLSearchParams();
      if (modelForm.api_key.trim()) params.set("api_key", modelForm.api_key.trim());
      if (modelForm.base_url.trim()) params.set("base_url", modelForm.base_url.trim());
      try {
        const resp = asRecord(await api.rawGet(`/models/discover/${encodeURIComponent(p)}?${params.toString()}`));
        const models = resp.models;
        if (Array.isArray(models)) return models.map((m: unknown) => str((m as Record<string, unknown>).id, "")).filter(Boolean);
      } catch { /* ignore */ }
      return [] as string[];
    },
    enabled: modelDialogOpen && modelForm.provider !== "openai-compatible",
    staleTime: 60_000,
    retry: false,
  });
  const modelOptions = (discoverModelsQ.data?.length ? discoverModelsQ.data : MODEL_FALLBACKS_BY_PROVIDER[modelForm.provider]) || [];

  function openAddModel() {
    setModelEditingId(null);
    setModelAdvancedOpen(false);
    setModelConnectivityWarning(null);
    setOpenaiSubAuth(null);
    setModelForm({
      label: "",
      role: "primary",
      provider: "ollama",
      model: "",
      base_url: OLLAMA_DEFAULT_BASE_URL,
      api_key: "",
      enabled: true
    });
    setModelDialogOpen(true);
  }

  function openEditModel(slot: JsonRecord) {
    setModelEditingId(str(slot.id, ""));
    setModelAdvancedOpen(false);
    setModelConnectivityWarning(null);
    setOpenaiSubAuth(null);
    setModelForm({
      label: str(slot.label, ""),
      role: str(slot.role, "primary"),
      provider:
        str(slot.provider, "ollama") === "codex-cli"
          ? "openai-subscription"
          : str(slot.provider, "ollama"),
      model: str(slot.model, ""),
      base_url: str(slot.base_url, ""),
      api_key: "",
      enabled: toBool(slot.enabled)
    });
    setModelDialogOpen(true);
  }

  async function startOpenaiSubscriptionOAuth() {
    if (codexAuthBusy) return;
    setCodexAuthBusy(true);
    setError(null);
    try {
      const response = asRecord(await api.rawPost("/models/openai-subscription/oauth/start", {}));
      const message = str(response.message, "").trim() || "OpenAI Subscription sign-in started.";
      const authUrl = str(response.auth_url, "").trim();
      const deviceCode = str(response.device_code, "").trim();
      const running = toBool(response.running);
      let openedInBrowser = false;
      if (authUrl) {
        const tab = window.open(authUrl, "_blank", "noopener,noreferrer");
        openedInBrowser = !!tab;
      }
      const openedBrowser = toBool(response.opened_browser) || openedInBrowser;
      setOpenaiSubAuth({ message, authUrl, deviceCode, running, openedBrowser });
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setCodexAuthBusy(false);
    }
  }

  async function checkOpenaiSubscriptionOAuthStatus() {
    if (codexAuthBusy) return;
    setCodexAuthBusy(true);
    setOpenaiSubAuth(null);
    setError(null);
    try {
      const response = asRecord(await api.rawGet("/models/openai-subscription/oauth/status"));
      const connected = toBool(response.connected);
      const message = str(response.message, "").trim();
      const authUrl = str(response.auth_url, "").trim();
      const deviceCode = str(response.device_code, "").trim();
      const running = toBool(response.running);
      const openedBrowser = false;
      if (connected) {
        setOpenaiSubAuth({
          message: message || "OpenAI Subscription login is connected.",
          authUrl,
          deviceCode,
          running,
          openedBrowser
        });
      } else {
        setOpenaiSubAuth({
          message: message || "OpenAI Subscription login is not connected yet.",
          authUrl,
          deviceCode,
          running,
          openedBrowser
        });
      }
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setCodexAuthBusy(false);
    }
  }

  const saveModelMutation = useMutation({
    mutationFn: async () => {
      const provider = modelForm.provider;
      const baseUrl = modelForm.base_url.trim();
      const normalizedBaseUrl =
        provider === "openrouter"
          ? baseUrl || OPENROUTER_DEFAULT_BASE_URL
          : provider === "ollama"
            ? baseUrl || OLLAMA_DEFAULT_BASE_URL
            : provider === "openai-subscription" || provider === "codex-cli"
              ? ""
            : provider === "openai-compatible"
              ? baseUrl
              : "";
      const payload: Record<string, unknown> = {
        label: modelForm.label.trim(),
        role: modelForm.role,
        provider,
        model: modelForm.model.trim(),
        base_url: normalizedBaseUrl || null,
        api_key: modelForm.api_key.trim() || null,
        enabled: modelForm.enabled
      };

      if (!payload.label || !payload.model) throw new Error("Label and model are required.");

      if (modelEditingId) {
        const response = asRecord(await api.rawPut(`/models/${encodeURIComponent(modelEditingId)}`, payload));
        const connectivityRaw = response.connectivity;
        const hasConnectivity = connectivityRaw !== undefined && connectivityRaw !== null;
        const connectivity = asRecord(connectivityRaw);
        return {
          connectivityOk: hasConnectivity ? toBool(connectivity.ok) : true,
          connectivityError: hasConnectivity ? str(connectivity.error, "").trim() : ""
        };
      }
      const response = asRecord(await api.rawPost("/models", payload));
      const connectivityRaw = response.connectivity;
      const hasConnectivity = connectivityRaw !== undefined && connectivityRaw !== null;
      const connectivity = asRecord(connectivityRaw);
      return {
        connectivityOk: hasConnectivity ? toBool(connectivity.ok) : true,
        connectivityError: hasConnectivity ? str(connectivity.error, "").trim() : ""
      };
    },
    onSuccess: async (result: { connectivityOk: boolean; connectivityError: string }) => {
      const wasEdit = !!modelEditingId;
      setModelDialogOpen(false);
      if (!result.connectivityOk) {
        setModelConnectivityWarning(
          `Model saved, but connection test failed: ${result.connectivityError || "could not reach provider"}. Runs may fail until fixed.`
        );
        setSuccess(wasEdit ? "Model updated (connectivity issue detected)." : "Model added (connectivity issue detected).");
      } else {
        setModelConnectivityWarning(null);
        setSuccess(wasEdit ? "Model updated." : "Model added.");
      }
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const deleteModelMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/models/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const toggleModelEnabledMutation = useMutation({
    mutationFn: async (slot: JsonRecord) => {
      const id = str(slot.id, "");
      const payload: Record<string, unknown> = {
        label: str(slot.label, ""),
        role: str(slot.role, "primary"),
        provider: str(slot.provider, "ollama"),
        model: str(slot.model, ""),
        base_url: str(slot.base_url, "") || null,
        enabled: !toBool(slot.enabled)
      };
      return await api.rawPut(`/models/${encodeURIComponent(id)}`, payload);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const hasTelegramToken = toBool(settings.has_telegram_token);
  const telegramDeliveryReady = toBool(settings.telegram_delivery_ready);
  const hasWhatsAppToken = toBool(settings.has_whatsapp_token);
  const whatsappDeliveryReady = toBool(settings.whatsapp_delivery_ready);
  const hasPrimaryApiKey = toBool(settings.has_api_key);
  const hasFallbackApiKey = toBool(settings.has_fallback_api_key);
  const dailyBriefDeliveryWarning = !form.daily_brief_enabled
    ? ""
    : form.daily_brief_channel === "telegram"
      ? !hasTelegramToken
        ? "Telegram is not configured yet."
        : !telegramDeliveryReady
          ? "Telegram bot is connected, but there is no delivery target yet. Message the bot once or add an allowed user ID."
          : ""
      : form.daily_brief_channel === "whatsapp"
        ? !hasWhatsAppToken
          ? "WhatsApp is not configured yet."
          : !whatsappDeliveryReady
            ? "WhatsApp is configured, but there is no delivery target yet. Send the agent a WhatsApp message first."
            : ""
        : "";
  const settingsComplete = stableSettingsComplete || toBool(settings.settings_complete) || modelSlots.length > 0;
  const showSetupRequired =
    !settingsComplete &&
    !settingsPayloadError &&
    !modelsPayloadError &&
    settingsQ.isSuccess &&
    modelsQ.isSuccess &&
    !settingsQ.isFetching &&
    !modelsQ.isFetching;
  const modelsRefreshIssue = modelsPayloadError || (modelsQ.isError ? errMessage(modelsQ.error) : "");
  const showingModelFallback =
    modelSlotsLive.length === 0 &&
    stableModelSlots.length > 0 &&
    (modelsQ.isFetching || !!modelsRefreshIssue);

  const apiKeyPayload = asRecord(apiKeyQ.data);
  const apiKeyIssuedAtUnix = num(apiKeyPayload.issued_at_unix, 0);
  const apiKeyExpiresAtUnix = num(apiKeyPayload.expires_at_unix, 0);
  const apiKeyRemainingFromServer = num(apiKeyPayload.remaining_seconds, 0);
  const apiKeyRemainingSeconds = useMemo(() => {
    if (apiKeyExpiresAtUnix > 0) {
      return Math.max(0, apiKeyExpiresAtUnix - Math.floor(apiKeyNowMs / 1000));
    }
    return Math.max(0, apiKeyRemainingFromServer);
  }, [apiKeyExpiresAtUnix, apiKeyNowMs, apiKeyRemainingFromServer]);
  const apiKeyRotated = toBool(apiKeyPayload.rotated);
  const tunnel = asRecord(tunnelQ.data);
  const tunnelProvidersPayload = asRecord(tunnelProvidersQ.data);
  const tunnelProviders = pickRecords(tunnelProvidersPayload, "providers");
  const serverSelectedTunnelProviderId = str(
    tunnelProvidersPayload.selected_provider,
    str(tunnel.provider, "cloudflare")
  );
  const selectedTunnelProviderRecord =
    tunnelProviders.find((provider) => str(provider.id, "") === tunnelSelectedProviderId) ||
    tunnelProviders.find((provider) => str(provider.id, "") === serverSelectedTunnelProviderId) ||
    tunnelProviders[0] ||
    null;
  const selectedTunnelConfigFields = selectedTunnelProviderRecord
    ? pickRecords(selectedTunnelProviderRecord, "config_fields")
    : [];
  const selectedTunnelStoredSecretFields = Array.isArray(selectedTunnelProviderRecord?.stored_secret_fields)
    ? (selectedTunnelProviderRecord?.stored_secret_fields as unknown[]).filter(
        (value): value is string => typeof value === "string" && value.trim().length > 0
      )
    : [];
  const selectedTunnelConfigHelp = str(selectedTunnelProviderRecord?.config_help, "").trim();
  const selectedTunnelDescription = str(selectedTunnelProviderRecord?.description, "").trim();
  const selectedTunnelAvailable = toBool(selectedTunnelProviderRecord?.available);
  const selectedTunnelConfigured = toBool(selectedTunnelProviderRecord?.configured);
  const hasTunnelAdvancedFields = selectedTunnelConfigFields.some(
    (field) => str(field.key, "").trim() === "binary_path"
  );
  const visibleTunnelConfigFields = selectedTunnelConfigFields.filter(
    (field) => showTunnelAdvanced || str(field.key, "").trim() !== "binary_path"
  );
  const tunnelStatusSummary = toBool(tunnel.active) ? "Public link is live" : "Public link is off";
  const tunnelSetupSummary = !selectedTunnelAvailable
    ? "Tunnel tool not found on this server"
    : !selectedTunnelConfigured
      ? "Setup incomplete"
      : "Ready";
  const sec = asRecord(securityStatusQ.data);

  useEffect(() => {
    if (tunnelProviders.length === 0) return;
    const currentValid = tunnelProviders.some(
      (provider) => str(provider.id, "").trim() === tunnelSelectedProviderId
    );
    if (!currentValid) {
      syncTunnelDraftFromPayload(tunnelProvidersPayload);
    }
  }, [tunnelProviders, tunnelProvidersPayload, tunnelSelectedProviderId]);

  useEffect(() => {
    setShowTunnelAdvanced(false);
    setTunnelPanelNotice(null);
  }, [tunnelSelectedProviderId, serverSelectedTunnelProviderId]);

  const securityLogs = pickRecords(securityLogsQ.data, "logs");
  const usingDefaultMasterPassword = toBool(sec.using_default);
  const hasCustomMasterPassword = toBool(sec.master_password_set) && !usingDefaultMasterPassword;
  const vaultSecrets = pickRecords(vaultSecretsQ.data, "entries");
  const pulseEvents = pickRecords(pulseQ.data, "events").sort((a, b) => {
    const aTs = Date.parse(str(a.timestamp, ""));
    const bTs = Date.parse(str(b.timestamp, ""));
    return (Number.isFinite(bTs) ? bTs : 0) - (Number.isFinite(aTs) ? aTs : 0);
  });
  const pulseMeta = asRecord(pulseQ.data);
  const pulseRunning = toBool(pulseMeta.running);
  const latestPulseEventId = str(asRecord(pulseEvents[0]).id, "");
  const moltbookStatus = asRecord(moltbookStatusQ.data);
  const latestMoltbookEventId = str(asRecord(moltbookEvents[0]).id, "");
  const moltbookRunRows = buildMoltbookRunRows(moltbookEvents);
  const selectedMoltbookRunId = str(selectedMoltbookEvent?.run_id, "").trim();
  const selectedMoltbookRunEvents = selectedMoltbookRunId
    ? moltbookEvents
        .filter((event) => str(event.run_id, "").trim() === selectedMoltbookRunId)
        .slice()
        .sort((a, b) => moltbookEventTimestampValue(a) - moltbookEventTimestampValue(b))
    : [];
  const selectedMoltbookDialogEvents =
    selectedMoltbookRunEvents.length > 0
      ? selectedMoltbookRunEvents
      : selectedMoltbookEvent
        ? [selectedMoltbookEvent]
        : [];
  const selectedMoltbookRepresentativeEvent =
    pickMoltbookRepresentativeEvent(selectedMoltbookDialogEvents) ?? selectedMoltbookEvent;
  const selectedMoltbookRunCounts = getMoltbookRunCounts(selectedMoltbookDialogEvents);
  const selectedMoltbookRunSummary = buildMoltbookRunSummary(selectedMoltbookDialogEvents);
  const selectedMoltbookRunLevel = buildMoltbookRunLevel(selectedMoltbookDialogEvents);
  const selectedMoltbookRunTrigger = collectMoltbookRunTrigger(selectedMoltbookDialogEvents);
  const selectedMoltbookRunLinks = collectMoltbookRunLinks(selectedMoltbookDialogEvents);
  const selectedMoltbookPostActivity = collectMoltbookRunPostActivity(selectedMoltbookDialogEvents);
  const moltbookRunning = toBool(moltbookStatus.running);
  const moltbookRunBusy = runMoltbookMutation.isPending || moltbookRunning || Boolean(moltbookPollState);
  const moltbookLastStatus = str(moltbookStatus.last_status, "").toLowerCase();
  const moltbookLastRunStats = asRecord(moltbookStatus.last_run_stats);
  const moltbookNeedsConnection =
    moltbookLastStatus === "not_connected" ||
    moltbookLastStatus === "not_configured" ||
    moltbookLastStatus === "error";
  const moltbookSchedulePresets = [
    { value: "every_minute", label: "Every 1 minute" },
    { value: "every_5_minutes", label: "Every 5 minutes" },
    { value: "every_10_minutes", label: "Every 10 minutes" },
    { value: "every_30_minutes", label: "Every 30 minutes" },
    { value: "hourly", label: "Every hour" },
    { value: "every_3_hours", label: "Every 3 hours" },
    { value: "every_6_hours", label: "Every 6 hours" },
    { value: "every_12_hours", label: "Every 12 hours" },
    { value: "daily", label: "Every 24 hours" },
    { value: "weekly", label: "Once a week" }
  ] as const;
  const moltbookPresetValues = new Set<string>(moltbookSchedulePresets.map((item) => item.value));
  const moltbookScheduleMode = moltbookPresetValues.has(form.moltbook_sync_frequency)
    ? form.moltbook_sync_frequency
    : "__custom__";
  const moltbookParticipationModes = [
    {
      value: "autopost",
      label: "Engage",
      shortLabel: "Recommended default",
      description: "Best for most users. AgentArk reads the feed, replies when useful, upvotes strong work, and creates new posts when there is something worth contributing."
    },
    {
      value: "assist",
      label: "Assist",
      shortLabel: "Interactive only",
      description: "Reads Moltbook continuously, and on manual runs it can reply, vote, and draft contributions without scheduling those actions automatically."
    },
    {
      value: "read_only",
      label: "Read Only",
      shortLabel: "Observe only",
      description: "Fetches posts for awareness and internal context, but never replies, votes, or posts."
    },
    {
      value: "off",
      label: "Off",
      shortLabel: "Disabled",
      description: "Keeps the connector registered but stops sync activity and engagement."
    }
  ] as const;
  const selectedMoltbookParticipationMode =
    moltbookParticipationModes.find((option) => option.value === form.moltbook_mode) ??
    moltbookParticipationModes[0];

  const selectedPulseDetails = asRecord(selectedPulseEvent?.details);
  const selectedPulseFindings = pickRecords(selectedPulseDetails, "doctor_findings").filter((f) =>
    isUserActionableDoctorFinding(f)
  );
  const selectedPulseScore = num(selectedPulseDetails.doctor_score, -1);
  const selectedPulseStatus = str(selectedPulseEvent?.status, "-");
  const selectedPulseStatusOk = selectedPulseStatus.toLowerCase() === "ok";
  const selectedPulseTimestampRaw = str(selectedPulseEvent?.timestamp, "-");
  const selectedPulseCaptured = looksLikeIsoTimestamp(selectedPulseTimestampRaw)
    ? formatTimestampForHumans(selectedPulseTimestampRaw)
    : { label: selectedPulseTimestampRaw, tooltip: selectedPulseTimestampRaw };
  const selectedPulseGuidance = (() => {
    if (selectedPulseFindings.length === 0 && (selectedPulseStatusOk || selectedPulseScore >= 90)) {
      return {
        severity: "success" as const,
        title: "System health looks good.",
        detail: "No active issues were detected in this run."
      };
    }
    if (selectedPulseFindings.length > 0) {
      const issueLabel = selectedPulseFindings.length === 1 ? "issue" : "issues";
      return {
        severity: "warning" as const,
        title: `${selectedPulseFindings.length} ${issueLabel} need attention.`,
        detail: "Use the recommended remediation under each issue, then run ArkPulse again."
      };
    }
    return {
      severity: "info" as const,
      title: "No direct findings were returned.",
      detail: "Review the snapshot for context and run another check after changes."
    };
  })();
  const selectedPulseSnapshot: { label: string; value: string }[] = [
    { label: "Pending tasks", value: String(num(selectedPulseDetails.pending_tasks, 0)) },
    { label: "Running tasks", value: String(num(selectedPulseDetails.running_tasks, 0)) },
    { label: "Completed tasks", value: String(num(selectedPulseDetails.completed_tasks, 0)) },
    { label: "Deployed apps", value: String(Array.isArray(selectedPulseDetails.deployed_apps) ? selectedPulseDetails.deployed_apps.length : 0) },
    { label: "Health checks", value: String(Array.isArray(selectedPulseDetails.health_checks) ? selectedPulseDetails.health_checks.length : 0) },
    { label: "Memories", value: String(num(selectedPulseDetails.total_memories, 0)) },
    { label: "Watchers", value: String(num(selectedPulseDetails.active_watchers, 0)) },
    { label: "Uptime", value: formatDurationFromSeconds(selectedPulseDetails.uptime_secs) }
  ];
  const latestPulseEvent = asRecord(pulseEvents[0]);
  const latestPulseDetails = asRecord(latestPulseEvent.details);
  const latestPulseFindingsCount = pickRecords(latestPulseDetails, "doctor_findings").filter((f) =>
    isUserActionableDoctorFinding(f)
  ).length;
  const latestPulseScore = num(latestPulseDetails.doctor_score, -1);
  const latestPulseStatus = str(latestPulseEvent.status, "").toLowerCase();
  const latestPulseHeadline =
    pulseRunning
      ? "ArkPulse is currently running."
      : pulseEvents.length === 0
      ? "No health checks yet."
      : latestPulseFindingsCount > 0
      ? `${latestPulseFindingsCount} issue${latestPulseFindingsCount === 1 ? "" : "s"} need attention.`
      : latestPulseStatus === "ok" || latestPulseScore >= 90
      ? "System health looks good."
      : "Health check completed.";
  const latestPulseSubtitle =
    pulseRunning
      ? "Please wait for this run to finish before starting another."
      : pulseEvents.length === 0
      ? "Click Run now to generate your first ArkPulse report."
      : latestPulseFindingsCount > 0
      ? "Open the latest report and start with Fix #1."
      : "No urgent action needed right now.";
  const latestPulseNavCount =
    latestPulseFindingsCount > 0 ? latestPulseFindingsCount : latestPulseStatus === "alert" ? 1 : 0;

  useEffect(() => {
    if (!pulsePollState) return;
    if (Date.now() >= pulsePollState.deadlineAt) {
      setPulsePollState(null);
      return;
    }
    if (!pulseRunning && latestPulseEventId && latestPulseEventId !== pulsePollState.baselineEventId) {
      setPulsePollState(null);
    }
  }, [pulsePollState, pulseRunning, latestPulseEventId]);

  useEffect(() => {
    if (!moltbookPollState) return;
    if (Date.now() >= moltbookPollState.deadlineAt) {
      setMoltbookPollState(null);
      return;
    }
    if (!moltbookRunning && latestMoltbookEventId && latestMoltbookEventId !== moltbookPollState.baselineEventId) {
      setMoltbookPollState(null);
    }
  }, [moltbookPollState, moltbookRunning, latestMoltbookEventId]);

  function severityChipColor(sev: string): "error" | "warning" | "info" | "success" | "default" {
    const s = (sev || "").toLowerCase();
    if (s === "critical" || s === "high" || s === "error") return "error";
    if (s === "medium" || s === "warn" || s === "warning") return "warning";
    if (s === "low") return "info";
    if (s === "ok" || s === "info") return "success";
    return "default";
  }

  function moltbookTriggerLabel(raw: string): string {
    const t = (raw || "").toLowerCase();
    if (t === "manual") return "Manual";
    if (t === "scheduler") return "Scheduled";
    return raw || "-";
  }

  function moltbookToolActionName(raw: string): string {
    const normalized = (raw || "").trim().toLowerCase();
    if (!normalized) return "Tool call";
    const mapped: Record<string, string> = {
      feed: "Read feed",
      search: "Search posts",
      create_post: "Create post",
      comment: "Create comment",
      upvote_post: "Upvote post",
      status: "Check status",
      me: "Load profile",
      register: "Register agent"
    };
    if (mapped[normalized]) return mapped[normalized];
    return normalized
      .replace(/_/g, " ")
      .replace(/\b\w/g, (m) => m.toUpperCase());
  }

  function moltbookActionLabel(action: string, details: JsonRecord): string {
    const a = (action || "").toLowerCase();
    if (a === "skipped_disabled") return "Skipped: Disabled";
    if (a === "skipped_off_mode") return "Skipped: Mode off";
    if (a === "deferred_busy") return "Deferred: Busy";
    if (a === "skipped_busy_max_defers") return "Skipped: Busy (max defers)";
    if (a === "not_connected") return "Not connected";
    if (a === "run_started") return "Run started";
    if (a === "run_completed") return "Run completed";
    if (a === "status_checked") return "Status checked";
    if (a === "engagement_plan_created") return "Engagement planned";
    if (a === "engagement_plan_fallback") return "Fallback plan used";
    if (a === "engagement_skipped_mode") return "Engagement skipped";
    if (a === "engagement_skipped_disabled") return "Engagement disabled";
    if (a === "engagement_skipped_empty_feed") return "No feed items";
    if (a === "engagement_skipped_not_needed") return "No action needed";
    if (a === "feed_fetched" || a === "feed_read") return "Feed fetched";
    if (a === "post_created") return "Post created";
    if (a === "comment_created") return "Comment created";
    if (a === "comment_failed") return "Comment failed";
    if (a === "post_upvoted") return "Post upvoted";
    if (a === "upvote_failed") return "Upvote failed";
    if (a.startsWith("tool_")) {
      return `Tool call: ${moltbookToolActionName(str(details.sub_action, a.slice(5)))}`;
    }
    if (a.startsWith("error_")) return `Error: ${action}`;
    // Fall back to the raw action code.
    return action || "-";
  }

function moltbookSummary(action: string, details: JsonRecord): string | null {
  const a = (action || "").toLowerCase();
  const titlePreview = str(details.title_preview, "").trim();
  const contentPreview = str(details.content_preview, "").trim();
  const queryPreview = str(details.query_preview, "").trim();
  const summaryPreview = str(details.summary_preview, str(details.summary, "")).trim();
  const errorPreview = str(details.error, "").trim();
  if (a === "run_completed") {
    const readCount = num(details.read_count, 0);
    const commentCount = num(details.comment_count, 0);
    const upvoteCount = num(details.upvote_count, 0);
    const postCount = num(details.post_count, toBool(details.posted) ? 1 : 0);
    const nextRunAt = str(details.next_run_at, "").trim();
    const nextLabel = nextRunAt ? formatTimestampForHumans(nextRunAt).label : "";
    const parts = [`Read ${readCount} post${readCount === 1 ? "" : "s"}`];
    if (commentCount > 0) parts.push(`${commentCount} comment${commentCount === 1 ? "" : "s"}`);
    if (upvoteCount > 0) parts.push(`${upvoteCount} upvote${upvoteCount === 1 ? "" : "s"}`);
    if (postCount > 0) parts.push(`${postCount} new post${postCount === 1 ? "" : "s"}`);
    if (commentCount + upvoteCount + postCount === 0) {
      parts.push("no public action taken");
    }
    if (nextLabel) {
      parts.push(`next run ${nextLabel}`);
    }
    const runSummaryText = parts.join(" | ");
    return runSummaryText;
    const normalizedRunSummary = parts.join(" • ");
    return normalizedRunSummary;
    const runSummary = parts.join(" • ");
    return runSummary;
    return parts.join(" • ");
  }
  if (a === "engagement_plan_created" || a === "engagement_plan_fallback") {
    return summaryPreview || "Prepared an engagement plan.";
  }
  if (
    a === "engagement_skipped_mode" ||
    a === "engagement_skipped_disabled" ||
    a === "engagement_skipped_empty_feed" ||
    a === "engagement_skipped_not_needed"
  ) {
    return str(details.reason, "").trim() || "No engagement action was taken.";
  }
  if (a === "feed_read" || a === "feed_fetched") {
    const readCount = num(details.count, num(details.read_count, 0));
    if (readCount > 0) {
      return `Fetched ${readCount} recent post${readCount === 1 ? "" : "s"}.`;
    }
    return "Fetched the feed but found no posts.";
  }
  if (a === "post_created") {
    const title = str(asRecord(details.request).title, "").trim();
    return title ? `Published: ${title}` : "Published a new Moltbook post.";
  }
  if (a === "comment_created") {
    return contentPreview ? `Reply: ${contentPreview}` : "Posted a reply on Moltbook.";
  }
  if (a === "comment_failed") {
    return errorPreview ? `Reply failed: ${errorPreview}` : "Could not post the Moltbook reply.";
  }
  if (a === "post_upvoted") {
    const postId = str(details.post_id, "").trim();
    return postId ? `Upvoted post ${postId}` : "Upvoted a Moltbook post.";
  }
  if (a === "upvote_failed") {
    return errorPreview ? `Upvote failed: ${errorPreview}` : "Could not upvote the Moltbook post.";
  }
  if (a === "memory_saved") {
    return summaryPreview || "Saved a Moltbook feed summary to memory.";
  }
  if (a === "memory_save_failed") {
    return errorPreview
      ? `Could not save the Moltbook summary to memory: ${errorPreview}`
      : "Could not save the Moltbook summary to memory.";
  }
  if (a.startsWith("tool_")) {
    const subAction = str(details.sub_action, a.slice(5)).trim().toLowerCase();
    if (errorPreview) {
      return errorPreview;
    }
    if (subAction === "search" && queryPreview) {
      return `Query: ${queryPreview}`;
    }
    if (subAction === "create_post") {
      if (titlePreview) return `Title: ${titlePreview}`;
      if (contentPreview) return `Content: ${contentPreview}`;
      return "Created a Moltbook post.";
    }
    if (subAction === "comment" && contentPreview) {
      return `Comment: ${contentPreview}`;
    }
    if (subAction === "upvote_post") {
      const postId = str(details.post_id, "").trim();
      return postId ? `Target post: ${postId}` : "Upvoted a Moltbook post.";
    }
    if (subAction === "feed") {
      return "Read recent Moltbook feed items.";
    }
    if (subAction === "status") {
      return "Checked Moltbook connection status.";
    }
    if (subAction === "me") {
      return "Loaded the Moltbook profile.";
    }
  }
  return null;
}

function moltbookReason(action: string, details: JsonRecord): string | null {
  const explicit = str(details.reason, "").trim();
  if (explicit) return explicit;

  const a = (action || "").toLowerCase();
  if (a === "skipped_disabled") return "Moltbook is disabled on this page.";
  if (a === "skipped_off_mode") return "Moltbook mode is set to off.";
  if (a === "deferred_busy") return "Deferred because the server was busy.";
  if (a === "skipped_busy_max_defers") return "Skipped because the server stayed busy after multiple defers.";
  if (a === "engagement_skipped_mode") return "This mode does not allow autonomous engagement right now.";
  if (a === "engagement_skipped_disabled") return "Autonomous Moltbook engagement is disabled.";
  if (a === "engagement_skipped_empty_feed") return "There was nothing new in the feed to engage with.";
  if (a === "engagement_skipped_not_needed") return "The current feed did not justify a public action.";

  if (a === "not_connected") {
    const status = str(details.status, "").toLowerCase();
    const err = str(details.error, "").trim();
    if (status === "not_configured") {
      return "Moltbook API key is not configured. Enter it on this page and save.";
    }
    if (status === "error") {
      return err
        ? `Moltbook authentication failed: ${err}`
        : "Moltbook authentication failed (invalid API key or unclaimed agent).";
    }
    return "Could not connect to Moltbook.";
  }

  if (a.startsWith("tool_")) {
    const err = str(details.error, "").trim();
    if (err) return `Tool call failed: ${err}`;
  }

  return null;
}

type MoltbookLinkEntry = {
  label: string;
  url: string;
};

function deriveMoltbookPostUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const raw = value.trim();
  if (!raw) return null;
  if (raw.startsWith("https://www.moltbook.com/post/")) return raw;
  const match = raw.match(/\/api\/v1\/posts\/([0-9a-f-]+)/i);
  if (match?.[1]) {
    return `https://www.moltbook.com/post/${match[1]}`;
  }
  return null;
}

function collectMoltbookLinks(details: JsonRecord): MoltbookLinkEntry[] {
  const out: MoltbookLinkEntry[] = [];
  const seen = new Set<string>();

  const push = (label: string, urlLike: unknown) => {
    if (typeof urlLike !== "string") return;
    const url = urlLike.trim();
    if (!url.startsWith("http://") && !url.startsWith("https://")) return;
    const key = `${label}|${url}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({ label, url });
  };

  const rawUrls = details.urls;
  if (Array.isArray(rawUrls)) {
    for (const entry of rawUrls) {
      if (isRecord(entry)) {
        push(str(entry.label, "Link"), entry.url);
      } else if (typeof entry === "string") {
        push("Link", entry);
      }
    }
  }

  const readPosts = Array.isArray(details.read_posts) ? details.read_posts : [];
  for (const entry of readPosts.slice(0, 4)) {
    if (!isRecord(entry)) continue;
    const title = str(entry.title, "").trim();
    push(title ? `Post: ${title}` : "Feed post", entry.url);
    push(title ? `Post: ${title}` : "Feed post", deriveMoltbookPostUrl(entry.post_api_url));
  }

  const engagedPostIds = Array.isArray(details.engaged_post_ids)
    ? details.engaged_post_ids.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    : [];
  for (const postId of engagedPostIds.slice(0, 4)) {
    push("Post URL", `https://www.moltbook.com/post/${encodeURIComponent(postId)}`);
  }

  const postId = str(details.post_id, "").trim();
  if (postId) {
    push("Post URL", `https://www.moltbook.com/post/${encodeURIComponent(postId)}`);
  }

  push("Claim URL", details.claim_url);
  push("Article URL", details.article_url);
  push("Post URL", details.post_url);
  push("URL", details.url);
  push("Post URL", deriveMoltbookPostUrl(details.post_api_url));
  push("Post URL", deriveMoltbookPostUrl(details.api_url));
  return out;
}

function collectMoltbookRunLinks(events: JsonRecord[]): MoltbookLinkEntry[] {
  const out: MoltbookLinkEntry[] = [];
  const seen = new Set<string>();
  for (const event of events) {
    for (const link of collectMoltbookLinks(asRecord(event.details))) {
      const key = link.url;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(link);
    }
  }
  out.sort((a, b) => {
    const aApi = a.label.toLowerCase().includes("api") ? 1 : 0;
    const bApi = b.label.toLowerCase().includes("api") ? 1 : 0;
    if (aApi !== bApi) return aApi - bApi;
    return a.label.localeCompare(b.label);
  });
  return out;
}

function moltbookEventTimestampValue(event: JsonRecord): number {
  const raw = str(event.timestamp, "").trim();
  if (!raw) return 0;
  const parsed = Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function pickMoltbookRepresentativeEvent(events: JsonRecord[]): JsonRecord | null {
  if (events.length === 0) return null;
  const completed = events
    .filter((event) => str(event.action, "").toLowerCase() === "run_completed")
    .sort((a, b) => moltbookEventTimestampValue(b) - moltbookEventTimestampValue(a))[0];
  if (completed) return completed;
  return events
    .slice()
    .sort((a, b) => moltbookEventTimestampValue(b) - moltbookEventTimestampValue(a))[0] ?? null;
}

type MoltbookRunCounts = {
  readCount: number;
  commentCount: number;
  upvoteCount: number;
  postCount: number;
  stepCount: number;
};

type MoltbookRunPostAction = {
  kind: "read" | "commented" | "liked" | "posted" | "comment_failed" | "like_failed" | "post_failed";
  label: string;
  summary: string | null;
  reason: string | null;
  timestamp: string;
  timestampValue: number;
  level: string;
};

type MoltbookRunPostActivity = {
  key: string;
  postId: string;
  title: string;
  url: string | null;
  submolt: string;
  author: string;
  actions: MoltbookRunPostAction[];
  lastActivityAt: number;
};

function moltbookActivityKey(postId: string, url: string | null, title: string, fallbackPrefix: string): string | null {
  if (postId) return `post:${postId}`;
  if (url) return `url:${url}`;
  if (title) return `${fallbackPrefix}:${title.trim().toLowerCase()}`;
  return null;
}

function pushMoltbookRunPostAction(
  target: MoltbookRunPostActivity,
  action: MoltbookRunPostAction
): void {
  const duplicate = target.actions.some(
    (candidate) =>
      candidate.kind === action.kind &&
      candidate.timestamp === action.timestamp &&
      (candidate.summary ?? "") === (action.summary ?? "") &&
      (candidate.reason ?? "") === (action.reason ?? "")
  );
  if (duplicate) return;
  target.actions.push(action);
  target.lastActivityAt = Math.max(target.lastActivityAt, action.timestampValue);
}

function collectMoltbookRunPostActivity(events: JsonRecord[]): MoltbookRunPostActivity[] {
  const buckets = new Map<string, MoltbookRunPostActivity>();

  const getBucket = (
    key: string,
    seed: { postId?: string; title?: string; url?: string | null; submolt?: string; author?: string }
  ): MoltbookRunPostActivity => {
    const existing = buckets.get(key);
    if (existing) {
      if (!existing.postId && seed.postId) existing.postId = seed.postId;
      if ((!existing.title || existing.title.startsWith("Post ")) && seed.title) existing.title = seed.title;
      if (!existing.url && seed.url) existing.url = seed.url;
      if (!existing.submolt && seed.submolt) existing.submolt = seed.submolt;
      if (!existing.author && seed.author) existing.author = seed.author;
      return existing;
    }
    const created: MoltbookRunPostActivity = {
      key,
      postId: seed.postId ?? "",
      title: seed.title || (seed.postId ? `Post ${seed.postId}` : "Untitled post"),
      url: seed.url ?? null,
      submolt: seed.submolt ?? "",
      author: seed.author ?? "",
      actions: [],
      lastActivityAt: 0,
    };
    buckets.set(key, created);
    return created;
  };

  const sortedEvents = events
    .slice()
    .sort((a, b) => moltbookEventTimestampValue(a) - moltbookEventTimestampValue(b));

  for (const event of sortedEvents) {
    const action = str(event.action, "").toLowerCase();
    const details = asRecord(event.details);
    const timestamp = str(event.timestamp, "");
    const timestampValue = moltbookEventTimestampValue(event);
    const level = str(event.level, "").toLowerCase();
    const summary = moltbookSummary(action, details);
    const reason = moltbookReason(action, details);

    if (action === "feed_read" || action === "feed_fetched") {
      const readPosts = Array.isArray(details.read_posts) ? details.read_posts : [];
      for (const entry of readPosts) {
        if (!isRecord(entry)) continue;
        const postId = str(entry.id, "").trim();
        const title = str(entry.title, "").trim();
        const url =
          str(entry.url, "").trim() ||
          deriveMoltbookPostUrl(entry.post_api_url) ||
          deriveMoltbookPostUrl(entry.url);
        const key = moltbookActivityKey(postId, url || null, title, "read");
        if (!key) continue;
        const bucket = getBucket(key, {
          postId,
          title,
          url: url || null,
          submolt: str(entry.submolt, "").trim(),
          author: str(entry.author, "").trim(),
        });
        pushMoltbookRunPostAction(bucket, {
          kind: "read",
          label: "Read",
          summary: "Included in this feed read.",
          reason: null,
          timestamp,
          timestampValue,
          level,
        });
      }
      continue;
    }

    let postId = str(details.post_id, "").trim();
    let title = str(details.post_title, "").trim();
    let url =
      str(details.post_url, "").trim() ||
      deriveMoltbookPostUrl(details.post_api_url) ||
      deriveMoltbookPostUrl(details.api_url) ||
      null;
    let submolt = str(details.submolt, "").trim();
    let author = str(details.author, "").trim();
    let kind: MoltbookRunPostAction["kind"] | null = null;

    if (action === "comment_created") {
      kind = "commented";
    } else if (action === "comment_failed") {
      kind = "comment_failed";
    } else if (action === "post_upvoted") {
      kind = "liked";
    } else if (action === "upvote_failed") {
      kind = "like_failed";
    } else if (action === "post_created") {
      const request = asRecord(details.request);
      title = str(request.title, "").trim();
      submolt = str(request.submolt, "").trim();
      postId = postId || str(details.posted_id, "").trim();
      kind = "posted";
      author = "AgentArk";
    } else if (action === "post_failed") {
      const request = asRecord(details.request);
      title = str(request.title, "").trim();
      submolt = str(request.submolt, "").trim();
      kind = "post_failed";
      author = "AgentArk";
    }

    if (!kind) continue;
    const key = moltbookActivityKey(postId, url, title, action);
    if (!key) continue;
    const bucket = getBucket(key, {
      postId,
      title,
      url,
      submolt,
      author,
    });
    pushMoltbookRunPostAction(bucket, {
      kind,
      label:
        kind === "commented"
          ? "Commented"
          : kind === "comment_failed"
            ? "Comment failed"
            : kind === "liked"
              ? "Liked"
              : kind === "like_failed"
                ? "Like failed"
                : kind === "posted"
                  ? "Posted new"
                  : "Post failed",
      summary,
      reason,
      timestamp,
      timestampValue,
      level,
    });
  }

  return Array.from(buckets.values())
    .map((entry) => ({
      ...entry,
      actions: entry.actions
        .slice()
        .sort((a, b) => b.timestampValue - a.timestampValue),
    }))
    .sort((a, b) => b.lastActivityAt - a.lastActivityAt || a.title.localeCompare(b.title));
}

function getMoltbookRunCounts(events: JsonRecord[]): MoltbookRunCounts {
  const completed = events.find((event) => str(event.action, "").toLowerCase() === "run_completed");
  const completedDetails = asRecord(completed?.details);
  let readCount = 0;
  let commentCount = 0;
  let upvoteCount = 0;
  let postCount = 0;

  for (const event of events) {
    const action = str(event.action, "").toLowerCase();
    const details = asRecord(event.details);
    if (action === "feed_read" || action === "feed_fetched") {
      readCount = Math.max(readCount, num(details.count, num(details.read_count, 0)));
      continue;
    }
    if (action === "comment_created") {
      commentCount += 1;
      continue;
    }
    if (action === "post_upvoted") {
      upvoteCount += 1;
      continue;
    }
    if (action === "post_created") {
      postCount += 1;
    }
  }

  return {
    readCount: Math.max(readCount, num(completedDetails.read_count, 0)),
    commentCount: Math.max(commentCount, num(completedDetails.comment_count, 0)),
    upvoteCount: Math.max(upvoteCount, num(completedDetails.upvote_count, 0)),
    postCount: Math.max(postCount, num(completedDetails.post_count, toBool(completedDetails.posted) ? 1 : 0)),
    stepCount: events.length
  };
}

function collectMoltbookRunTrigger(events: JsonRecord[]): string {
  for (const event of events) {
    const trigger = str(asRecord(event.details).trigger, "").trim();
    if (trigger) return trigger;
  }
  return "";
}

function isMoltbookRunGroup(events: JsonRecord[]): boolean {
  return (
    Boolean(collectMoltbookRunTrigger(events)) ||
    events.some((event) => {
      const action = str(event.action, "").toLowerCase();
      return action === "run_started" || action === "run_completed";
    })
  );
}

function buildMoltbookRunSummary(events: JsonRecord[]): string {
  const completed = events.find((event) => str(event.action, "").toLowerCase() === "run_completed");
  if (completed) {
    return (
      moltbookSummary("run_completed", asRecord(completed.details)) ||
      "Run completed."
    );
  }

  const { readCount, commentCount, upvoteCount, postCount } = getMoltbookRunCounts(events);
  const parts: string[] = [];
  if (readCount > 0) parts.push(`Read ${readCount} post${readCount === 1 ? "" : "s"}`);
  if (commentCount > 0) parts.push(`${commentCount} comment${commentCount === 1 ? "" : "s"}`);
  if (upvoteCount > 0) parts.push(`${upvoteCount} like${upvoteCount === 1 ? "" : "s"}`);
  if (postCount > 0) parts.push(`${postCount} post${postCount === 1 ? "" : "s"} created`);
  if (parts.length > 0) return parts.join(" | ");

  const representative = pickMoltbookRepresentativeEvent(events);
  const representativeAction = str(representative?.action, "");
  return moltbookSummary(representativeAction, asRecord(representative?.details)) || "Run recorded.";
}

function buildMoltbookRunLevel(events: JsonRecord[]): "error" | "warning" | "success" {
  const levels = events.map((event) => str(event.level, "").toLowerCase());
  if (levels.some((level) => level === "error")) return "error";
  if (levels.some((level) => level === "warning" || level === "warn")) return "warning";
  return "success";
}

function buildMoltbookRunRows(events: JsonRecord[]): JsonRecord[] {
  const grouped = new Map<string, JsonRecord[]>();
  for (const event of events) {
    const runId = str(event.run_id, "").trim() || str(event.id, "").trim();
    if (!runId) continue;
    const existing = grouped.get(runId);
    if (existing) {
      existing.push(event);
    } else {
      grouped.set(runId, [event]);
    }
  }
  return Array.from(grouped.values())
    .filter((group) => isMoltbookRunGroup(group))
    .map((group) => pickMoltbookRepresentativeEvent(group))
    .filter((event): event is JsonRecord => event != null)
    .sort((a, b) => moltbookEventTimestampValue(b) - moltbookEventTimestampValue(a));
}

  async function copyClipboardText(value: string): Promise<void> {
    const text = value.trim();
    if (!text) throw new Error("Nothing to copy.");
    const nav = typeof navigator !== "undefined" ? navigator : null;
    if (nav?.clipboard?.writeText) {
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
  }

  const regenerateApiKeyMutation = useMutation({
    mutationFn: () => api.rawPost("/settings/api-key/regenerate", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-api-key"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  async function refreshTunnelQueries() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["tunnel-status"] }),
      queryClient.invalidateQueries({ queryKey: ["tunnel-providers"] }),
      queryClient.invalidateQueries({ queryKey: ["apps-manager-tunnel-status"] }),
      queryClient.invalidateQueries({ queryKey: ["chat-workspace-tunnel"] })
    ]);
  }

  const tunnelSaveMutation = useMutation({
    mutationFn: (payload: { provider: string; values: Record<string, string> }) =>
      api.rawPost("/tunnel/configure", payload),
    onSuccess: async (raw, variables) => {
      const response = asRecord(raw);
      await refreshTunnelQueries();
      syncTunnelDraftFromPayload(response.settings, variables.provider);
    },
    onError: (e) => setError(errMessage(e))
  });

  const tunnelTestMutation = useMutation({
    mutationFn: (payload: { provider: string }) => api.rawPost("/tunnel/test", payload),
    onError: (e) => setError(errMessage(e))
  });

  const tunnelStartMutation = useMutation({
    mutationFn: (payload: { provider?: string }) => api.rawPost("/tunnel/start", payload),
    onSuccess: async () => {
      await refreshTunnelQueries();
    },
    onError: (e) => setError(errMessage(e))
  });

  const tunnelStopMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/stop", {}),
    onSuccess: async () => {
      await refreshTunnelQueries();
    },
    onError: (e) => setError(errMessage(e))
  });

  async function saveSelectedTunnelProviderSettings() {
    const provider = tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId;
    if (!provider) throw new Error("Choose a tunnel provider first.");
    const response = asRecord(
      await tunnelSaveMutation.mutateAsync({
        provider,
        values: tunnelDraftValues
      })
    );
    return response;
  }

  async function performTunnelStart() {
    await saveSelectedTunnelProviderSettings();
    const response = asRecord(
      await tunnelStartMutation.mutateAsync({
        provider: tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId
      })
    );
    const startedUrl = str(response.url, "").trim();
    const startMessage = startedUrl
      ? `Public link is ready. ${startedUrl}`
      : "Tunnel is starting. The public link will appear here in a few seconds.";
    setTunnelPanelNotice({
      severity: startedUrl ? "success" : "info",
      text: startMessage
    });
    setSuccess(str(response.message, "Tunnel start requested."));
  }

  async function maybeResumeTunnelStartAfterPassword() {
    if (!resumeTunnelStartAfterPassword) return;
    setResumeTunnelStartAfterPassword(false);
    setTunnelPanelNotice({
      severity: "info",
      text: "Custom password saved. Creating your public link now..."
    });
    try {
      await performTunnelStart();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelProviderSave() {
    setError(null);
    try {
      const response = await saveSelectedTunnelProviderSettings();
      setTunnelPanelNotice({
        severity: "success",
        text: str(response.message, "Tunnel settings saved.")
      });
      setSuccess(str(response.message, "Tunnel settings saved."));
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelProviderTest() {
    setError(null);
    try {
      await saveSelectedTunnelProviderSettings();
      const response = asRecord(
        await tunnelTestMutation.mutateAsync({
          provider: tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId
        })
      );
      const message = str(response.message, "Tunnel provider test passed.").trim();
      const detail = str(response.detail, "").trim();
      setTunnelPanelNotice({
        severity: "success",
        text: detail ? `${message} ${detail}` : message
      });
      setSuccess(detail ? `${message} ${detail}` : message);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelStart() {
    setError(null);
    if (!hasCustomMasterPassword) {
      setResumeTunnelStartAfterPassword(true);
      setTunnelPanelNotice({
        severity: "info",
        text: "Set a custom AgentArk password first. The public link will start right after that."
      });
      openPasswordDialog(toBool(sec.master_password_set) ? "change" : "set");
      return;
    }
    try {
      await performTunnelStart();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelStop() {
    setError(null);
    try {
      const response = asRecord(await tunnelStopMutation.mutateAsync());
      setTunnelPanelNotice({
        severity: "info",
        text: str(response.message, "Public link stopped.")
      });
      setSuccess(str(response.message, "Tunnel stopped."));
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const restartMutation = useMutation({
    mutationFn: () => api.rawPost("/restart", {}),
    onSuccess: () => setSuccess("Restart scheduled. Page will reload shortly."),
    onError: (e) => setError(errMessage(e))
  });

  const triggerPulseMutation = useMutation({
    mutationFn: () => api.rawPost("/arkpulse/trigger", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["arkpulse-log"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const runPulseFixMutation = useMutation({
    mutationFn: async (payload: {
      fixCommand: string;
      remediation?: ArkPulseRemediationSpec | null;
      issueTitle: string;
      target: string;
      eventTimestamp?: string;
      findingIndex?: number;
    }) => {
      const body: ArkPulseRunFixRequest = {
        issue_title: payload.issueTitle,
        target: payload.target,
        event_timestamp: payload.eventTimestamp || undefined,
        finding_index: payload.findingIndex
      };
      const fixCommand = payload.fixCommand.trim();
      if (fixCommand) {
        body.fix_command = fixCommand;
      }
      if (payload.remediation) {
        body.remediation = payload.remediation;
      }
      const out = asRecord(
        await api.rawPost("/arkpulse/fix", body)
      );
      const status = str(out.status, "").toLowerCase();
      if (status === "error") {
        const errorText = str(out.error, "").trim() || str(out.message, "").trim() || "ArkPulse fix failed.";
        throw new Error(errorText);
      }
      return out;
    },
    onSuccess: async (raw) => {
      const message = str(raw.message, "").trim();
      const output = str(raw.output, "").trim();
      const mode = str(raw.mode, "").trim().toLowerCase();
      if (message && output) {
        setSuccess(`${message}\n\n${output}`);
      } else if (message) {
        setSuccess(message);
      } else {
        setSuccess("ArkPulse fix completed.");
      }
      const baselineEventId = latestPulseEventId;
      setSelectedPulseEvent(null);
      if (!pulseRunning) {
        setPulsePollState({
          baselineEventId,
          deadlineAt: Date.now() + 2 * 60 * 1000
        });
        if (mode !== "app_restart") {
          try {
            await api.rawPost("/arkpulse/trigger", {});
          } catch (e) {
            setPulsePollState(null);
            setError(`Fix ran, but ArkPulse refresh failed: ${errMessage(e)}`);
          }
        }
      }
      await queryClient.invalidateQueries({ queryKey: ["arkpulse-log"] });
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-workspace-tunnel"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-workspace-apps"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const trustEvaluateMutation = useMutation({
    mutationFn: (payload: { action_kind: string; payload: unknown }) =>
      api.rawPost("/autonomy/trust/evaluate", payload)
  });
  const selectedTrustPreset =
    TRUST_APPROVAL_PRESETS.find((item) => item.id === trustPresetId) ?? TRUST_APPROVAL_PRESETS[0];

  const setPasswordMutation = useMutation({
    mutationFn: (password: string) => api.rawPost("/security/set-password", { password }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      if (resumeTunnelStartAfterPassword) {
        await maybeResumeTunnelStartAfterPassword();
      } else {
        setSuccess("Custom password saved.");
      }
    },
    onError: (e) => setError(errMessage(e))
  });

  const changePasswordMutation = useMutation({
    mutationFn: (payload: { current_password: string; new_password: string }) =>
      api.rawPost("/security/change-password", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      if (resumeTunnelStartAfterPassword) {
        await maybeResumeTunnelStartAfterPassword();
      } else {
        setSuccess("Custom password updated.");
      }
    },
    onError: (e) => setError(errMessage(e))
  });

  const removePasswordMutation = useMutation({
    mutationFn: (password: string) => api.rawPost("/security/remove-password", { password }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSuccess("Custom password removed.");
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      setResumeTunnelStartAfterPassword(false);
    },
    onError: (e) => setError(errMessage(e))
  });

  const passwordMutationPending =
    setPasswordMutation.isPending || changePasswordMutation.isPending || removePasswordMutation.isPending;

  const upsertVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; value: string; password?: string }) =>
      api.rawPost("/settings/secrets/upsert", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setSuccess("Secret saved.");
    },
    onError: (e) => setError(errMessage(e))
  });

  const deleteVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; password?: string }) =>
      api.rawPost("/settings/secrets/delete", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setSuccess("Secret deleted.");
    },
    onError: (e) => setError(errMessage(e))
  });

  function resolveVaultPasswordForSensitiveOps(): string | null | undefined {
    if (!hasCustomMasterPassword) return undefined;
    const pw = vaultPassword.trim();
    if (!pw) {
      setError("Master password is required for secret changes.");
      return null;
    }
    return pw;
  }

  function openVaultEditor() {
    setError(null);
    setSuccess(null);
    setVaultEditorKey("");
    setVaultEditorValue("");
    setShowVaultSecretValue(false);
    setVaultEditorOpen(true);
  }

  function closeVaultEditor() {
    if (upsertVaultSecretMutation.isPending) return;
    setVaultEditorOpen(false);
    setVaultEditorKey("");
    setVaultEditorValue("");
    setShowVaultSecretValue(false);
  }

  async function submitVaultEditor() {
    const key = vaultEditorKey.trim();
    const value = vaultEditorValue;
    if (!key) {
      setError("Secret key is required.");
      return;
    }
    if (!value.trim()) {
      setError("Secret value is required.");
      return;
    }
    const pw = resolveVaultPasswordForSensitiveOps();
    if (pw === null) return;
    setError(null);
    try {
      await upsertVaultSecretMutation.mutateAsync({
        key,
        value,
        password: pw || undefined
      });
      closeVaultEditor();
    } catch {
      // handled by mutation onError
    }
  }

  function resetPasswordInputs() {
    setSecCurrentPassword("");
    setSecNewPassword("");
    setSecConfirmPassword("");
    setShowPasswordInputs(false);
  }

  function openPasswordDialog(mode: PasswordDialogMode) {
    setError(null);
    setSuccess(null);
    resetPasswordInputs();
    setPasswordDialogMode(mode);
  }

  function closePasswordDialog() {
    if (passwordMutationPending) return;
    setPasswordDialogMode(null);
    setResumeTunnelStartAfterPassword(false);
    resetPasswordInputs();
  }

  async function submitPasswordDialog() {
    if (!passwordDialogMode) return;
    setError(null);
    setSuccess(null);
    try {
      if (passwordDialogMode === "set") {
        const pw = secNewPassword;
        if (pw.length < 8) {
          setError("Password must be at least 8 characters.");
          return;
        }
        if (pw !== secConfirmPassword) {
          setError("Passwords do not match.");
          return;
        }
        await setPasswordMutation.mutateAsync(pw);
      } else if (passwordDialogMode === "change") {
        const pw = secNewPassword;
        if (pw.length < 8) {
          setError("New password must be at least 8 characters.");
          return;
        }
        if (pw !== secConfirmPassword) {
          setError("Passwords do not match.");
          return;
        }
        await changePasswordMutation.mutateAsync({
          current_password: secCurrentPassword,
          new_password: pw
        });
      } else if (passwordDialogMode === "remove") {
        await removePasswordMutation.mutateAsync(secCurrentPassword);
      }
      setPasswordDialogMode(null);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const settingsNavGroups: Array<{ id: string; label: string; items: Array<{ value: number; label: string }> }> = [
    {
      id: "setup",
      label: "Setup",
      items: [
        { value: 0, label: "General" },
        { value: 1, label: "Models" },
        { value: 2, label: "Integrations" },
        { value: 3, label: "Media" }
      ]
    },
    {
      id: "knowledge",
      label: "Knowledge",
      items: [
        { value: 12, label: "Memory" },
        { value: 8, label: "MCP Servers" }
      ]
    },
    {
      id: "operations",
      label: "Operations",
      items: [{ value: 13, label: "Evolution" }]
    },
    {
      id: "security",
      label: "Security",
      items: [
        { value: 4, label: "Security" },
        { value: 5, label: "Advanced" }
      ]
    }
  ];
  const settingsNavActual = settingsNavGroups.flatMap((group) => group.items);
  const selectedSettingsNav =
    settingsNavActual.find((item) => item.value === tab) ||
    (tab === 7
      ? {
          value: 7,
          label: "Moltbook"
        }
      : tab === 9
        ? {
            value: 9,
            label: latestPulseNavCount > 0 ? `ArkPulse (${latestPulseNavCount})` : "ArkPulse"
          }
        : tab === 11
          ? {
              value: 11,
              label: "Trace"
            }
          : settingsNavActual[0]);
  const tabSupportsSave = ![9, 11].includes(tab);

  return (
    <Stack spacing={2}>
      {showSetupRequired ? (
        <Alert severity="warning">
          Setup required: configure at least one model in the Models tab, then Save Settings.
        </Alert>
      ) : null}

      <Box className="settings-shell-layout" sx={hideSettingsNav ? { gridTemplateColumns: "1fr !important" } : undefined}>
        {!hideSettingsNav ? (
        <Box className="settings-sidebar">
          <Box className="settings-brand">
            <Avatar src={AgentLogo} variant="rounded" sx={{ width: 28, height: 28 }} />
            <Stack spacing={0.1}>
              <Typography variant="subtitle2">AgentArk</Typography>
              <Typography variant="caption" color="text.secondary">
                Settings
              </Typography>
            </Stack>
          </Box>
          <Stack spacing={0.2} className="settings-nav-list" sx={{ display: { xs: "none", md: "flex" } }}>
            {settingsNavGroups.map((group, groupIdx) => (
              <Box key={`settings-nav-group-${group.id}`}>
                <Typography className="settings-nav-group-label">
                  {group.label}
                </Typography>
                {group.items.map((item) => (
                  <Button
                    key={`settings-nav-${item.value}`}
                    className={`settings-nav-btn${tab === item.value ? " active" : ""}`}
                    variant="text"
                    onClick={() => setTab(item.value)}
                  >
                    <span>{item.label}</span>
                  </Button>
                ))}
                {groupIdx < settingsNavGroups.length - 1 ? (
                  <div className="settings-nav-divider" />
                ) : null}
              </Box>
            ))}
          </Stack>
          <Tabs
            value={tab}
            onChange={(_, v) => setTab(Number(v) || 0)}
            variant="scrollable"
            scrollButtons="auto"
            sx={{ display: { xs: "flex", md: "none" } }}
          >
            {settingsNavActual.map((item) => (
              <Tab key={`settings-mobile-${item.value}`} value={item.value} label={item.label} />
            ))}
          </Tabs>
        </Box>
        ) : null}
        <Box className="settings-main">
          <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mb: 1.5 }}>
            <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "1rem" }}>
              {selectedSettingsNav?.label || "Settings"}
            </Typography>
            {tabSupportsSave ? (
              <Stack direction="row" spacing={1} alignItems="center">
                {modelsQ.isFetching && showingModelFallback ? (
                  <Chip size="small" color="warning" variant="outlined" label="Reconnecting..." />
                ) : null}
                <Button
                  size="small"
                  variant="contained"
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    try {
                      await saveMutation.mutateAsync();
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                  disabled={saveMutation.isPending || !effectiveDirty}
                >
                  Save
                </Button>
              </Stack>
            ) : null}
          </Stack>

      {tab === 0 ? (
        <Stack spacing={2.5}>
          {/* ── Status Overview ── */}
          <Box>
            <Typography className="settings-section-label">Status</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr 1fr", md: "repeat(4, 1fr)" }, gap: 1.5 }}>
              {[
                {
                  label: "Primary API Key",
                  tone: hasPrimaryApiKey ? "success" : "muted",
                  status: hasPrimaryApiKey ? "Connected" : "Not configured"
                },
                {
                  label: "Fallback API Key",
                  tone: hasFallbackApiKey ? "success" : "muted",
                  status: hasFallbackApiKey ? "Connected" : "Not configured"
                },
                {
                  label: "Telegram",
                  tone: !hasTelegramToken ? "muted" : telegramDeliveryReady ? "success" : "warning",
                  status: !hasTelegramToken ? "Not configured" : telegramDeliveryReady ? "Ready to deliver" : "No recipient"
                },
                {
                  label: "WhatsApp",
                  tone: !hasWhatsAppToken ? "muted" : whatsappDeliveryReady ? "success" : "warning",
                  status: !hasWhatsAppToken ? "Not configured" : whatsappDeliveryReady ? "Ready to deliver" : "No recipient"
                },
              ].map((s) => (
                <Box
                  key={s.label}
                  sx={{
                    p: 1.5,
                    borderRadius: 2,
                    border: "1px solid",
                    borderColor:
                      s.tone === "success"
                        ? "rgba(20,241,149,0.18)"
                        : s.tone === "warning"
                          ? "rgba(255,180,50,0.24)"
                          : "rgba(255,255,255,0.06)",
                    background:
                      s.tone === "success"
                        ? "rgba(20,241,149,0.04)"
                        : s.tone === "warning"
                          ? "rgba(255,180,50,0.08)"
                          : "rgba(255,255,255,0.02)",
                    display: "flex",
                    alignItems: "center",
                    gap: 1,
                  }}
                >
                  <Box
                    sx={{
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      flexShrink: 0,
                      background:
                        s.tone === "success"
                          ? "#14f195"
                          : s.tone === "warning"
                            ? "rgba(255,180,50,0.9)"
                            : "rgba(255,255,255,0.15)",
                      boxShadow:
                        s.tone === "success"
                          ? "0 0 6px rgba(20,241,149,0.4)"
                          : s.tone === "warning"
                            ? "0 0 6px rgba(255,180,50,0.35)"
                            : "none",
                    }}
                  />
                  <Stack spacing={0}>
                    <Typography variant="caption" sx={{ color: "rgba(180,200,225,0.55)", fontSize: "0.68rem", lineHeight: 1.2 }}>{s.label}</Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        fontWeight: 500,
                        fontSize: "0.8rem",
                        color:
                          s.tone === "muted"
                            ? "rgba(180,200,225,0.45)"
                            : "rgba(225,242,255,0.9)"
                      }}
                    >
                      {s.status}
                    </Typography>
                  </Stack>
                </Box>
              ))}
            </Box>
            <Box sx={{ display: "flex", gap: 2, mt: 1.5, flexWrap: "wrap" }}>
              <Chip size="small" variant="outlined" label={modelsQ.isLoading && modelSlots.length === 0 ? "Loading models…" : `${modelSlots.length} model${modelSlots.length !== 1 ? "s" : ""}`} sx={{ borderColor: "rgba(47,212,255,0.25)", color: "rgba(47,212,255,0.85)", fontSize: "0.72rem" }} />
              <Chip size="small" variant="outlined" label={configuredProviders.length ? configuredProviders.join(", ") : "No media providers"} sx={{ borderColor: "rgba(255,255,255,0.08)", color: "rgba(180,200,225,0.55)", fontSize: "0.72rem" }} />
              {settingsComplete ? (
                <Chip size="small" variant="outlined" label="Setup complete" sx={{ borderColor: "rgba(20,241,149,0.25)", color: "rgba(20,241,149,0.85)", fontSize: "0.72rem" }} />
              ) : (
                <Chip size="small" variant="outlined" label="Setup incomplete" sx={{ borderColor: "rgba(255,180,50,0.3)", color: "rgba(255,180,50,0.85)", fontSize: "0.72rem" }} />
              )}
            </Box>
          </Box>

          <hr className="settings-divider" />

          {/* ── Identity ── */}
          <Stack spacing={2}>
            <Typography className="settings-section-label" sx={{ mb: "0 !important" }}>Identity</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1.5 }}>
              <TextField label="Bot Name" value={form.bot_name} onChange={(e) => setField("bot_name", e.target.value)} fullWidth size="small" />
              <TextField
                label="Personality"
                select
                value={form.personality}
                onChange={(e) => setField("personality", e.target.value)}
                fullWidth
                size="small"
              >
                <MenuItem value="friendly">Friendly</MenuItem>
                <MenuItem value="professional">Professional</MenuItem>
                <MenuItem value="casual">Casual</MenuItem>
                <MenuItem value="technical">Technical</MenuItem>
                <MenuItem value="creative">Creative</MenuItem>
                <MenuItem value="concise">Concise</MenuItem>
              </TextField>
              <TextField label="Language" value={form.language} onChange={(e) => setField("language", e.target.value)} fullWidth size="small" placeholder="e.g. English" />
              <TextField
                label="Tone"
                select
                value={form.tone}
                onChange={(e) => setField("tone", e.target.value)}
                fullWidth
                size="small"
                InputLabelProps={{ shrink: true }}
                SelectProps={{ displayEmpty: true }}
              >
                <MenuItem value="">Default</MenuItem>
                <MenuItem value="concise">Concise</MenuItem>
                <MenuItem value="friendly">Friendly</MenuItem>
                <MenuItem value="professional">Professional</MenuItem>
                <MenuItem value="casual">Casual</MenuItem>
                <MenuItem value="technical">Technical</MenuItem>
                <MenuItem value="creative">Creative</MenuItem>
              </TextField>
            </Box>
          </Stack>

          <hr className="settings-divider" />

          {/* ── Preferences ── */}
          <Stack spacing={2}>
            <Typography className="settings-section-label" sx={{ mb: "0 !important" }}>Preferences</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1.5 }}>
              <Autocomplete
                freeSolo
                options={[
                  "UTC",
                  "America/New_York",
                  "America/Chicago",
                  "America/Denver",
                  "America/Los_Angeles",
                  "America/Phoenix",
                  "America/Toronto",
                  "America/Vancouver",
                  "Europe/London",
                  "Europe/Paris",
                  "Europe/Berlin",
                  "Asia/Dubai",
                  "Asia/Kolkata",
                  "Asia/Singapore",
                  "Asia/Tokyo",
                  "Australia/Sydney"
                ]}
                value={form.timezone || ""}
                onChange={(_, v) => setField("timezone", String(v ?? ""))}
                inputValue={form.timezone || ""}
                onInputChange={(_, v) => setField("timezone", v)}
                renderInput={(params) => (
                  <TextField
                    {...params}
                    label="Timezone"
                    placeholder="e.g. America/New_York"
                    fullWidth
                    size="small"
                  />
                )}
              />
              <TextField
                label="Email Format"
                select
                value={form.email_format}
                onChange={(e) => setField("email_format", e.target.value)}
                fullWidth
                size="small"
                InputLabelProps={{ shrink: true }}
                SelectProps={{ displayEmpty: true }}
              >
                <MenuItem value="">Default</MenuItem>
                <MenuItem value="bullets">Bullets</MenuItem>
                <MenuItem value="sections">Sections</MenuItem>
                <MenuItem value="narrative">Narrative</MenuItem>
              </TextField>
            </Box>
          </Stack>

          <hr className="settings-divider" />

          <Box>
            <Typography className="settings-section-label">Daily Brief</Typography>
            <Stack spacing={1.25}>
              <Box className="metadata-box">
                <Stack
                  direction={{ xs: "column", md: "row" }}
                  spacing={1.5}
                  justifyContent="space-between"
                  alignItems={{ xs: "flex-start", md: "center" }}
                >
                  <Stack spacing={0.35}>
                    <Typography variant="subtitle2">Morning Summary</Typography>
                    <Typography variant="caption" color="text.secondary">
                      Send a recurring daily brief using your selected timezone.
                    </Typography>
                  </Stack>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={form.daily_brief_enabled}
                        onChange={(e) => setField("daily_brief_enabled", e.target.checked)}
                      />
                    }
                    label={form.daily_brief_enabled ? "Enabled" : "Disabled"}
                  />
                </Stack>
              </Box>
              <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1.5 }}>
                <TextField
                  label="Preferred Delivery Time"
                  type="time"
                  value={form.daily_brief_time}
                  onChange={(e) => setField("daily_brief_time", e.target.value)}
                  fullWidth
                  size="small"
                  InputLabelProps={{ shrink: true }}
                  inputProps={{ step: 60 }}
                  helperText="24-hour time. The brief follows the timezone above."
                />
                <TextField
                  label="Delivery Channel"
                  select
                  value={form.daily_brief_channel}
                  onChange={(e) => setField("daily_brief_channel", e.target.value)}
                  fullWidth
                  size="small"
                  InputLabelProps={{ shrink: true }}
                >
                  <MenuItem value="telegram">Telegram</MenuItem>
                  <MenuItem value="whatsapp">WhatsApp</MenuItem>
                  <MenuItem value="email">Email</MenuItem>
                </TextField>
              </Box>
              <Typography variant="caption" color="text.secondary">
                Turning this off pauses the scheduled brief but keeps your preferred time saved.
              </Typography>
              {dailyBriefDeliveryWarning ? (
                <Alert severity="warning">{dailyBriefDeliveryWarning}</Alert>
              ) : null}
            </Stack>
          </Box>
        </Stack>
      ) : null}

      {tab === 1 ? (
        <Stack spacing={2} data-tour-target="settings-models">
          <Box sx={{ minHeight: 0 }}>
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Stack spacing={0.3}>
                <Typography className="settings-section-label" sx={{ mb: "0 !important" }}>Model Pool</Typography>
                <Typography variant="caption" color="text.secondary">
                  Configure multiple models for different roles. Changes apply immediately.
                </Typography>
              </Stack>
              <Button size="small" variant="contained" onClick={openAddModel}>
                Add Model
              </Button>
            </Stack>

            <Stack direction="row" spacing={2} alignItems="center" sx={{ mb: 1 }}>
              <FormControlLabel
                control={
                  <Switch
                    checked={form.smart_routing}
                    onChange={(e) => setField("smart_routing", e.target.checked)}
                  />
                }
                label="Smart Routing"
              />
              <Typography variant="caption" color="text.secondary">
                When off, the agent uses the primary model for everything.
              </Typography>
            </Stack>
            <TextField
              label="App Deploy Model (optional)"
              select
              fullWidth
              size="small"
              sx={{ mb: 1.25, maxWidth: 560 }}
              value={form.app_deploy_model_id}
              onChange={(e) => setField("app_deploy_model_id", e.target.value)}
              helperText="If not set, app deploy uses the default primary model."
            >
              <MenuItem value="">Default (Primary model)</MenuItem>
              {modelSlots.map((slot) => {
                const id = str(slot.id, "");
                const label = str(slot.label, "Model");
                const role = str(slot.role, "primary");
                const model = str(slot.model, "");
                const enabled = toBool(slot.enabled);
                return (
                  <MenuItem key={id || `${label}:${model}`} value={id} disabled={!enabled}>
                    {enabled ? `${label} [${role}] - ${model}` : `${label} [${role}] - ${model} (disabled)`}
                  </MenuItem>
                );
              })}
            </TextField>

            {modelsQ.isLoading && modelSlots.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                Loading models...
              </Typography>
            ) : modelsRefreshIssue && modelSlots.length === 0 ? (
              <Alert severity="warning">
                Could not refresh model list right now. Please retry in a moment.
              </Alert>
            ) : modelSlots.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No models configured. Add a model to complete setup.
              </Typography>
            ) : (
              <Stack spacing={1}>
                {showingModelFallback ? (
                  <Alert severity="info">
                    Showing last known model list while refresh is in progress.
                  </Alert>
                ) : null}
                <TableContainer className="table-shell">
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        <TableCell>Label</TableCell>
                        <TableCell>Role</TableCell>
                        <TableCell>Provider</TableCell>
                        <TableCell>Model</TableCell>
                        <TableCell>Enabled</TableCell>
                        <TableCell>API Key</TableCell>
                        <TableCell align="right">Ops</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {modelSlots.map((slot) => {
                        const id = str(slot.id, "");
                        const enabled = toBool(slot.enabled);
                        return (
                          <TableRow key={id}>
                            <TableCell>{str(slot.label, "-")}</TableCell>
                            <TableCell>{str(slot.role, "-")}</TableCell>
                            <TableCell>
                              {str(slot.provider, "-") === "codex-cli"
                                ? "openai-subscription"
                                : str(slot.provider, "-")}
                            </TableCell>
                            <TableCell sx={{ wordBreak: "break-word" }}>{str(slot.model, "-")}</TableCell>
                            <TableCell>{enabled ? "yes" : "no"}</TableCell>
                            <TableCell>{toBool(slot.has_api_key) ? "configured" : "-"}</TableCell>
                            <TableCell align="right">
                              <RowOpsMenu
                                actions={[
                                  {
                                    label: "Edit",
                                    onClick: () => openEditModel(slot)
                                  },
                                  {
                                    label: enabled ? "Disable" : "Enable",
                                    disabled: toggleModelEnabledMutation.isPending,
                                    onClick: async () => {
                                      setError(null);
                                      try {
                                        await toggleModelEnabledMutation.mutateAsync(slot);
                                      } catch (e) {
                                        setError(errMessage(e));
                                      }
                                    }
                                  },
                                  {
                                    label: "Delete",
                                    tone: "error",
                                    divider: true,
                                    disabled: deleteModelMutation.isPending,
                                    onClick: async () => {
                                      const ok = window.confirm("Delete this model slot?");
                                      if (!ok) return;
                                      setError(null);
                                      try {
                                        await deleteModelMutation.mutateAsync(id);
                                      } catch (e) {
                                        setError(errMessage(e));
                                      }
                                    }
                                  }
                                ]}
                                ariaLabel="Model options"
                              />
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
              </Stack>
            )}
          </Box>

          <Dialog open={modelDialogOpen} onClose={() => setModelDialogOpen(false)} fullWidth maxWidth="sm">
            <DialogTitle>{modelEditingId ? "Edit Model" : "Add Model"}</DialogTitle>
            <DialogContent>
              <Stack spacing={1.5} sx={{ mt: 1 }}>
                <TextField
                  label="Label"
                  value={modelForm.label}
                  onChange={(e) => setModelForm((p) => ({ ...p, label: e.target.value }))}
                  fullWidth
                />
                <TextField
                  label="Role"
                  select
                  value={modelForm.role}
                  onChange={(e) => setModelForm((p) => ({ ...p, role: e.target.value }))}
                  fullWidth
                >
                  <MenuItem value="primary">primary</MenuItem>
                  <MenuItem value="fast">fast</MenuItem>
                  <MenuItem value="code">code</MenuItem>
                  <MenuItem value="research">research</MenuItem>
                  <MenuItem value="fallback">fallback</MenuItem>
                </TextField>
                <TextField
                  label="Provider"
                  select
                  value={modelForm.provider}
                  onChange={(e) => setModelForm((p) => ({ ...p, provider: e.target.value }))}
                  fullWidth
                >
                  <MenuItem value="ollama">ollama</MenuItem>
                  <MenuItem value="anthropic">anthropic</MenuItem>
                  <MenuItem value="openai">openai</MenuItem>
                  <MenuItem value="openai-subscription">openai-subscription (OAuth)</MenuItem>
                  <MenuItem value="openrouter">openrouter</MenuItem>
                  <MenuItem value="openai-compatible">openai-compatible</MenuItem>
                </TextField>
                <Autocomplete
                  freeSolo
                  options={modelOptions}
                  value={modelForm.model}
                  onChange={(_, v) => setModelForm((p) => ({ ...p, model: String(v ?? "") }))}
                  inputValue={modelForm.model}
                  onInputChange={(_, v) => setModelForm((p) => ({ ...p, model: v }))}
                  renderInput={(params) => (
                    <TextField
                      {...params}
                      label="Model"
                      fullWidth
                      placeholder={
                        modelForm.provider === "openai-subscription"
                          ? "Choose OpenAI model"
                          : "Enter model id"
                      }
                    />
                  )}
                />
                {modelForm.provider === "openai-subscription" || modelForm.provider === "codex-cli" ? (
                  <Stack spacing={1}>
                    <Alert severity="info">
                      Connect your OpenAI subscription with browser OAuth. You can reconnect any time, especially if auth expires.
                      <br /><br />
                      <strong>First time?</strong> Enable device code auth in your OpenAI account: go to{" "}
                      <a href="https://chatgpt.com/settings/security" target="_blank" rel="noopener noreferrer" style={{ color: "inherit" }}>
                        chatgpt.com/settings/security
                      </a>{" "}
                      → toggle <strong>"Enable device code authorization"</strong> on.
                    </Alert>
                    <Stack direction="row" spacing={1}>
                      <Button
                        variant="contained"
                        size="small"
                        onClick={startOpenaiSubscriptionOAuth}
                        disabled={codexAuthBusy}
                      >
                        {codexAuthBusy ? "Starting..." : modelEditingId ? "Reconnect OAuth" : "Connect via Browser"}
                      </Button>
                      <Button
                        variant="outlined"
                        size="small"
                        onClick={checkOpenaiSubscriptionOAuthStatus}
                        disabled={codexAuthBusy}
                      >
                        Check Status
                      </Button>
                      <Button
                        variant="text"
                        size="small"
                        onClick={() => {
                          const authUrl = (openaiSubAuth?.authUrl || "").trim();
                          if (!authUrl) return;
                          window.open(authUrl, "_blank", "noopener,noreferrer");
                        }}
                        disabled={codexAuthBusy || !(openaiSubAuth?.authUrl || "").trim()}
                      >
                        Open URL
                      </Button>
                    </Stack>
                    {(openaiSubAuth?.deviceCode || "").trim() ? (
                      <Stack direction="row" spacing={0.8} alignItems="center" sx={{ minWidth: 0 }}>
                        <Typography variant="caption" color="text.secondary">
                          Device code:
                        </Typography>
                        <Typography
                          variant="caption"
                          component="code"
                          sx={{
                            px: 0.8,
                            py: 0.2,
                            borderRadius: 1,
                            bgcolor: "rgba(0,0,0,0.22)",
                            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"
                          }}
                        >
                          {(openaiSubAuth?.deviceCode || "").trim()}
                        </Typography>
                        <IconButton
                          size="small"
                          onClick={async () => {
                            try {
                              await navigator.clipboard.writeText((openaiSubAuth?.deviceCode || "").trim());
                              setSuccess("Device code copied.");
                            } catch {
                              setError("Could not copy device code.");
                            }
                          }}
                          aria-label="Copy device code"
                        >
                          <ContentCopyRoundedIcon fontSize="inherit" />
                        </IconButton>
                      </Stack>
                    ) : null}
                    {(openaiSubAuth?.authUrl || "").trim() ? (
                      <Stack direction="row" spacing={0.8} alignItems="center" sx={{ minWidth: 0 }}>
                        <Link
                          href={(openaiSubAuth?.authUrl || "").trim()}
                          target="_blank"
                          rel="noopener noreferrer"
                          underline="hover"
                          sx={{ fontSize: "0.75rem", wordBreak: "break-all", flex: 1, minWidth: 0 }}
                        >
                          {(openaiSubAuth?.authUrl || "").trim()}
                        </Link>
                        <IconButton
                          size="small"
                          onClick={async () => {
                            try {
                              await navigator.clipboard.writeText((openaiSubAuth?.authUrl || "").trim());
                              setSuccess("OAuth URL copied.");
                            } catch {
                              setError("Could not copy URL.");
                            }
                          }}
                          aria-label="Copy OAuth URL"
                        >
                          <ContentCopyRoundedIcon fontSize="inherit" />
                        </IconButton>
                      </Stack>
                    ) : null}
                    {openaiSubAuth && !openaiSubAuth.openedBrowser && (openaiSubAuth.authUrl || "").trim() ? (
                      <Typography variant="caption" color="warning.main">
                        Browser did not open automatically. Click "Open URL" above to complete sign-in.
                      </Typography>
                    ) : null}
                    {openaiSubAuth?.running ? (
                      <Typography variant="caption" color="info.main">
                        Login is in progress. Finish auth in browser/device flow, then click Check Status.
                      </Typography>
                    ) : null}
                    {openaiSubAuth?.message ? (
                      <Typography variant="caption" color="text.secondary">
                        {openaiSubAuth.message}
                      </Typography>
                    ) : null}
                  </Stack>
                ) : (
                  <TextField
                    label="API Key (optional)"
                    value={modelForm.api_key}
                    onChange={(e) => setModelForm((p) => ({ ...p, api_key: e.target.value }))}
                    fullWidth
                    type="password"
                    helperText={modelEditingId ? "Leave blank to keep the current key." : undefined}
                  />
                )}
                <Accordion expanded={modelAdvancedOpen} onChange={(_, expanded) => setModelAdvancedOpen(expanded)} disableGutters>
                  <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                    <Typography variant="body2">Advanced</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    {["ollama", "openrouter", "openai-compatible"].includes(modelForm.provider) ? (
                      <TextField
                        label={modelForm.provider === "openai-compatible" ? "Base URL" : "Base URL (optional)"}
                        value={modelForm.base_url}
                        onChange={(e) => setModelForm((p) => ({ ...p, base_url: e.target.value }))}
                        fullWidth
                        helperText={
                          modelForm.provider === "openrouter"
                            ? `Default: ${OPENROUTER_DEFAULT_BASE_URL}`
                            : modelForm.provider === "ollama"
                              ? `Default: ${OLLAMA_DEFAULT_BASE_URL}`
                              : "Required for OpenAI-compatible providers."
                        }
                      />
                    ) : (
                      <Typography variant="caption" color="text.secondary">
                        No advanced provider settings for this model.
                      </Typography>
                    )}
                  </AccordionDetails>
                </Accordion>
                <FormControlLabel
                  control={<Switch checked={modelForm.enabled} onChange={(e) => setModelForm((p) => ({ ...p, enabled: e.target.checked }))} />}
                  label="Enabled"
                />
                <Stack direction="row" spacing={1} justifyContent="flex-end">
                  <Button onClick={() => setModelDialogOpen(false)}>Cancel</Button>
                  <Button
                    variant="contained"
                    onClick={async () => {
                      setError(null);
                      setModelConnectivityWarning(null);
                      try {
                        await saveModelMutation.mutateAsync();
                      } catch (e) {
                        setError(errMessage(e));
                      }
                    }}
                    disabled={saveModelMutation.isPending}
                  >
                    Save
                  </Button>
                </Stack>
              </Stack>
            </DialogContent>
          </Dialog>
        </Stack>
      ) : null}

      {tab === 3 ? (
        <Grid2 container spacing={1.5} alignItems="stretch">
          <Grid2 size={{ xs: 12, lg: 6 }} sx={{ display: "flex" }}>
            <Box sx={{ minHeight: 0, width: "100%" }}>
              <Typography className="settings-section-label">
                Provider Keys
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Keys are stored encrypted. Leave blank to keep current keys.
              </Typography>
              <Stack spacing={1.2} sx={{ mt: 1 }}>
                <TextField label="Replicate API Key" value={form.media_key_replicate} onChange={(e) => setField("media_key_replicate", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="FAL API Key" value={form.media_key_fal} onChange={(e) => setField("media_key_fal", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="Stability AI API Key" value={form.media_key_stability_ai} onChange={(e) => setField("media_key_stability_ai", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="Together API Key" value={form.media_key_together} onChange={(e) => setField("media_key_together", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="OpenAI API Key (DALL-E/Sora)" value={form.media_key_openai_dalle} onChange={(e) => setField("media_key_openai_dalle", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="Google AI API Key (Gemini/Veo)" value={form.media_key_google_gemini} onChange={(e) => setField("media_key_google_gemini", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="Runway API Key" value={form.media_key_runway} onChange={(e) => setField("media_key_runway", e.target.value)} fullWidth size="small" type="password" />
                <TextField label="Luma API Key" value={form.media_key_luma} onChange={(e) => setField("media_key_luma", e.target.value)} fullWidth size="small" type="password" />
              </Stack>
              <Divider sx={{ my: 2 }} />
              <Typography variant="caption" color="text.secondary">
                Detected configured providers: {configuredProviders.length ? configuredProviders.join(", ") : "(none detected)"}
              </Typography>
            </Box>
          </Grid2>

          <Grid2 size={{ xs: 12, lg: 6 }} sx={{ display: "flex" }}>
            <Box className="list-shell" sx={{ minHeight: 0, width: "100%" }}>
              <Typography variant="h6" mb={1}>
                Defaults
              </Typography>
              <Stack spacing={1.2}>
                <TextField label="Default Image Provider" value={form.default_image_provider} onChange={(e) => setField("default_image_provider", e.target.value)} fullWidth size="small" />
                <TextField label="Image Model" value={form.image_model} onChange={(e) => setField("image_model", e.target.value)} fullWidth size="small" />
                <TextField label="Fallback Image Provider" value={form.fallback_image_provider} onChange={(e) => setField("fallback_image_provider", e.target.value)} fullWidth size="small" />
                <TextField label="Default Video Provider" value={form.default_video_provider} onChange={(e) => setField("default_video_provider", e.target.value)} fullWidth size="small" />
                <TextField label="Fallback Video Provider" value={form.fallback_video_provider} onChange={(e) => setField("fallback_video_provider", e.target.value)} fullWidth size="small" />
              </Stack>
              <Divider sx={{ my: 2 }} />
              <Typography variant="h6" mb={1}>
                Advanced (JSON)
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Optional JSON mapping provider to key, e.g. {"{\"openai\":\"sk-...\",\"replicate\":\"...\"}"}
              </Typography>
              <TextField
                label="media_providers JSON"
                value={form.media_provider_keys_json}
                onChange={(e) => setField("media_provider_keys_json", e.target.value)}
                fullWidth
                multiline
                minRows={6}
                sx={{ mt: 1 }}
              />
            </Box>
          </Grid2>
        </Grid2>
      ) : null}

      {tab === 4 ? (
        <Grid2 container spacing={1.5}>
          <Grid2 size={{ xs: 12, lg: 6 }}>
            <Stack spacing={2}>
              <Box sx={{ minHeight: 0 }}>
                <Stack spacing={1}>
                  <Typography className="settings-section-label">Security & Master Password</Typography>
                  {securityStatusQ.isLoading ? (
                    <Typography variant="body2" color="text.secondary">
                      Loading security status...
                    </Typography>
                  ) : securityStatusQ.error ? (
                    <Alert severity="error">{errMessage(securityStatusQ.error)}</Alert>
                  ) : (
                    <Stack spacing={1}>
                      <Typography variant="caption" color="text.secondary">
                        Mode: {str(sec.encryption_mode) === "password" ? "password" : "keyfile"}
                      </Typography>
                      {str(sec.encryption_mode) !== "password" ? (
                        <Alert
                          severity="warning"
                          sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}
                        >
                          No master password is active yet.
                        </Alert>
                      ) : toBool(sec.using_default) ? (
                        <Alert
                          severity="warning"
                          sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}
                        >
                          Default password is active. Treat this as not configured until you set your own custom master password.
                        </Alert>
                      ) : (
                        <Alert
                          severity="success"
                          sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}
                        >
                          Custom master password active.
                        </Alert>
                      )}
                      <Typography variant="caption" color="text.secondary">
                        Password setup opens a secure dialog. Changes apply immediately in this running AgentArk session.
                      </Typography>
                      <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                        {hasCustomMasterPassword ? (
                          <Button
                            variant="contained"
                            size="large"
                            onClick={() => openPasswordDialog("change")}
                            disabled={passwordMutationPending}
                          >
                            Change Password
                          </Button>
                        ) : (
                          <Button
                            variant="contained"
                            size="large"
                            onClick={() => openPasswordDialog(toBool(sec.master_password_set) ? "change" : "set")}
                            disabled={passwordMutationPending}
                          >
                            Set Custom Password
                          </Button>
                        )}
                        {hasCustomMasterPassword ? (
                          <Button
                            color="error"
                            variant="outlined"
                            size="large"
                            onClick={() => openPasswordDialog("remove")}
                            disabled={passwordMutationPending}
                          >
                            Remove Password
                          </Button>
                        ) : null}
                      </Stack>
                    </Stack>
                  )}
                </Stack>
              </Box>

              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Typography variant="h6" mb={1}>
                  Remote Access (Tunnel)
                </Typography>
                {tunnelQ.isLoading || tunnelProvidersQ.isLoading ? (
                  <Typography variant="body2" color="text.secondary">
                    Loading tunnel settings...
                  </Typography>
                ) : tunnelQ.error || tunnelProvidersQ.error ? (
                  <Alert severity="error">{errMessage(tunnelQ.error || tunnelProvidersQ.error)}</Alert>
                ) : (
                  <Stack spacing={1}>
                    <Typography variant="caption" color="text.secondary">
                      Status: {tunnelStatusSummary} | Provider: {str(tunnel.provider_label, str(selectedTunnelProviderRecord?.label, "-"))} | Setup: {tunnelSetupSummary}
                    </Typography>
                    <TextField
                      label="Provider"
                      select
                      size="small"
                      fullWidth
                      value={tunnelSelectedProviderId || serverSelectedTunnelProviderId}
                      onChange={(e) => {
                        const next = e.target.value;
                        syncTunnelDraftFromPayload(tunnelProvidersPayload, next);
                      }}
                    >
                      {tunnelProviders.map((provider) => {
                        const id = str(provider.id, "");
                        const label = str(provider.label, id || "Provider");
                        return (
                          <MenuItem key={id} value={id}>
                            {label}
                          </MenuItem>
                        );
                      })}
                    </TextField>
                    {selectedTunnelDescription ? (
                      <Typography variant="caption" color="text.secondary">
                        {selectedTunnelDescription}
                      </Typography>
                    ) : null}
                    {selectedTunnelConfigHelp ? (
                      <Alert severity="info" sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}>
                        {selectedTunnelConfigHelp}
                      </Alert>
                    ) : null}
                    {str(selectedTunnelProviderRecord?.id, "") === "bore" ? (
                      <Alert severity="warning" sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}>
                        Bore is fine for raw app sharing, but AgentArk control-plane remote access requires an HTTPS tunnel provider such as Cloudflare, ngrok, or Tailscale Funnel.
                      </Alert>
                    ) : null}
                    {hasTunnelAdvancedFields ? (
                      <Button
                        size="small"
                        variant="text"
                        sx={{ alignSelf: "flex-start", px: 0 }}
                        onClick={() => setShowTunnelAdvanced((prev) => !prev)}
                      >
                        {showTunnelAdvanced ? "Hide advanced options" : "Show advanced options"}
                      </Button>
                    ) : null}
                    {visibleTunnelConfigFields.length === 0 ? (
                      <Typography variant="caption" color="text.secondary">
                        {selectedTunnelAvailable
                          ? "No extra setup is needed for this provider."
                          : "Install the tunnel tool on the server, then retry. Advanced options let you override the binary path if needed."}
                      </Typography>
                    ) : null}
                    {visibleTunnelConfigFields.map((field) => {
                      const key = str(field.key, "");
                      const inputType = str(field.input_type, "text");
                      const options = Array.isArray(field.options)
                        ? field.options.filter((value): value is string => typeof value === "string")
                        : [];
                      const value = tunnelDraftValues[key] ?? "";
                      const storedSecret = inputType === "password" && selectedTunnelStoredSecretFields.includes(key);
                      const helperText =
                        inputType === "password" && storedSecret && !value.trim()
                          ? "A value is already saved. Enter a new value only if you want to replace it."
                          : undefined;
                      return (
                        <TextField
                          key={key}
                          label={str(field.label, key || "Field")}
                          value={value}
                          onChange={(e) =>
                            setTunnelDraftValues((prev) => ({
                              ...prev,
                              [key]: e.target.value
                            }))
                          }
                          fullWidth
                          size="small"
                          required={toBool(field.required)}
                          placeholder={str(field.placeholder, "") || undefined}
                          type={inputType === "password" ? "password" : "text"}
                          multiline={inputType === "textarea"}
                          minRows={inputType === "textarea" ? 3 : undefined}
                          select={inputType === "select"}
                          helperText={helperText}
                        >
                          {inputType === "select"
                            ? options.map((option) => (
                                <MenuItem key={option} value={option}>
                                  {option}
                                </MenuItem>
                              ))
                            : null}
                        </TextField>
                      );
                    })}
                    {str(tunnel.url, "").trim() ? (
                      <TextField
                        label="Public Link"
                        value={str(tunnel.url)}
                        fullWidth
                        size="small"
                        InputProps={{ readOnly: true }}
                      />
                    ) : null}
                    {tunnelPanelNotice ? (
                      <Alert severity={tunnelPanelNotice.severity}>{tunnelPanelNotice.text}</Alert>
                    ) : null}
                    {str(tunnel.error, "").trim() ? <Alert severity="error">{str(tunnel.error)}</Alert> : null}
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={handleTunnelProviderSave}
                        disabled={tunnelSaveMutation.isPending}
                      >
                        {tunnelSaveMutation.isPending ? "Saving..." : "Save"}
                      </Button>
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={handleTunnelProviderTest}
                        disabled={tunnelSaveMutation.isPending || tunnelTestMutation.isPending}
                      >
                        {tunnelTestMutation.isPending ? "Checking..." : "Check Setup"}
                      </Button>
                      <Button
                        size="small"
                        variant="contained"
                        onClick={handleTunnelStart}
                        disabled={
                          tunnelSaveMutation.isPending ||
                          tunnelStartMutation.isPending ||
                          toBool(tunnel.active) ||
                          !selectedTunnelAvailable
                        }
                      >
                        {tunnelStartMutation.isPending
                          ? "Starting..."
                          : hasCustomMasterPassword
                            ? "Get Public Link"
                            : "Set Password & Get Public Link"}
                      </Button>
                      <Button
                        size="small"
                        onClick={handleTunnelStop}
                        disabled={tunnelStopMutation.isPending || !toBool(tunnel.active)}
                      >
                        {tunnelStopMutation.isPending ? "Stopping..." : "Stop Public Link"}
                      </Button>
                      <Button
                        size="small"
                        onClick={async () => {
                          const url = str(tunnel.url, "");
                          if (!url) return;
                          await navigator.clipboard.writeText(url);
                          setSuccess("Tunnel URL copied.");
                        }}
                        disabled={!str(tunnel.url, "").trim()}
                      >
                        Copy Link
                      </Button>
                    </Stack>
                    <Alert
                      severity="warning"
                      sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}
                    >
                      {hasCustomMasterPassword
                        ? "Anyone with the URL reaches your AgentArk sign-in page. They still need your custom AgentArk password to get in, and you should stop the public link when you no longer need it."
                        : "Public control-plane links stay blocked until you set a custom AgentArk password. After that, visitors sign in with that password and AgentArk keeps the internal server key private."}
                    </Alert>
                  </Stack>
                )}
              </Box>

              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Stack spacing={1}>
                  <Typography variant="h6">Secrets Vault</Typography>
                  <Typography variant="caption" color="text.secondary">
                    View encrypted secrets used by models, integrations, search, and workflows. Values stay masked in the UI.
                  </Typography>
                  {hasCustomMasterPassword ? (
                    <TextField
                      label="Master password (required for add/delete when set)"
                      value={vaultPassword}
                      onChange={(e) => setVaultPassword(e.target.value)}
                      fullWidth
                      size="small"
                      type="password"
                    />
                  ) : (
                    <Alert severity="info">
                      No custom master password is set. Secrets are still encrypted at rest, and the vault only shows masked snippets.
                    </Alert>
                  )}
                  <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                    <Button
                      size="small"
                      onClick={async () => {
                        setError(null);
                        await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
                      }}
                      disabled={vaultSecretsQ.isLoading}
                    >
                      Refresh
                    </Button>
                    <Button
                      size="small"
                      variant="outlined"
                      onClick={openVaultEditor}
                    >
                      Add Custom Secret
                    </Button>
                  </Stack>

                  {vaultSecretsQ.isLoading ? (
                    <Typography variant="body2" color="text.secondary">
                      Loading secrets...
                    </Typography>
                  ) : vaultSecretsQ.error ? (
                    <Alert severity="error">{errMessage(vaultSecretsQ.error)}</Alert>
                  ) : vaultSecrets.length === 0 ? (
                    <Typography variant="body2" color="text.secondary">
                      No encrypted secrets stored yet.
                    </Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>Key</TableCell>
                            <TableCell>Source</TableCell>
                            <TableCell>Value</TableCell>
                            <TableCell align="right">Ops</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {vaultSecrets.map((row, idx) => {
                            const key = str(row.key, "");
                            const shownValue = str(row.masked, "");
                            const source = str(row.source, "custom");
                            const deletable = toBool(row.deletable);
                            return (
                              <TableRow key={`${key}-${idx}`}>
                                <TableCell sx={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}>{key}</TableCell>
                                <TableCell>
                                  <Typography variant="body2" sx={{ textTransform: "capitalize" }}>
                                    {source.replace(/[-_]+/g, " ")}
                                  </Typography>
                                </TableCell>
                                <TableCell sx={{ maxWidth: 360 }}>
                                  <Typography
                                    variant="body2"
                                    title={shownValue}
                                    sx={{
                                      whiteSpace: "nowrap",
                                      overflow: "hidden",
                                      textOverflow: "ellipsis"
                                    }}
                                  >
                                    {shownValue || "-"}
                                  </Typography>
                                </TableCell>
                                <TableCell align="right">
                                  <Stack direction="row" spacing={0.5} justifyContent="flex-end">
                                    {deletable ? (
                                      <Button
                                        size="small"
                                        color="error"
                                        onClick={async () => {
                                          const ok = window.confirm(`Delete secret '${key}'?`);
                                          if (!ok) return;
                                          const pw = resolveVaultPasswordForSensitiveOps();
                                          if (pw === null) return;
                                          setError(null);
                                          try {
                                            await deleteVaultSecretMutation.mutateAsync({
                                              key,
                                              password: pw || undefined
                                            });
                                          } catch {
                                            // handled by mutation onError
                                          }
                                        }}
                                        disabled={deleteVaultSecretMutation.isPending}
                                      >
                                        Delete
                                      </Button>
                                    ) : (
                                      <Typography variant="caption" color="text.secondary">
                                        Managed elsewhere
                                      </Typography>
                                    )}
                                  </Stack>
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  )}

                </Stack>
              </Box>
            </Stack>
          </Grid2>

          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Stack direction={{ xs: "column", sm: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", sm: "center" }} spacing={1}>
                <Stack spacing={0.25}>
                  <Typography variant="h6">Security Logs</Typography>
                  <Typography variant="caption" color="text.secondary">
                    Hidden by default. Open the viewer to inspect individual events.
                  </Typography>
                </Stack>
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => {
                    setSelectedSecurityLog(null);
                    setSecurityLogsDialogOpen(true);
                  }}
                >
                  Open Logs
                </Button>
              </Stack>
            </Box>
          </Grid2>
        </Grid2>
      ) : null}

      {tab === 5 ? (
        <Stack spacing={2.5}>
          {/* ── Warning banner ── */}
          <Box className="adv-banner">
            <span className="adv-banner-icon">&#9888;</span>
            Advanced settings can impact stability or security. Change only if you understand the effect.
          </Box>

          <Box className="adv-group">
            <div className="adv-group-header">
              <div className="adv-group-header-icon" style={{ background: "rgba(74, 222, 128, 0.10)", border: "1px solid rgba(74, 222, 128, 0.22)" }}>
                <SmartToyRoundedIcon sx={{ fontSize: 16, color: "#4ade80" }} />
              </div>
              <div>
                <div className="adv-group-header-title">Observability</div>
                <div className="adv-group-header-sub">Optional export of AgentArk traces to Langtrace, LangSmith, or OTLP collectors</div>
              </div>
            </div>
            <ObservabilityPanel
              values={{
                enabled: form.observability_enabled,
                provider: form.observability_provider,
                endpoint: form.observability_endpoint,
                serviceName: form.observability_service_name,
                headerName: form.observability_header_name,
                privacyMode: form.observability_privacy_mode,
                authToken: form.observability_auth_token,
                authTokenConfigured: toBool(observabilitySettings.auth_token_configured)
              }}
              logs={observabilityLogs}
              issues={observabilityIssues}
              logsLoading={observabilityLogsQ.isLoading}
              logsError={observabilityLogsQ.error ? errMessage(observabilityLogsQ.error) : null}
              testing={testObservabilityMutation.isPending}
              onValueChange={(next) => {
                if (Object.prototype.hasOwnProperty.call(next, "enabled")) {
                  setField("observability_enabled", !!next.enabled);
                }
                if (typeof next.provider === "string") {
                  setField("observability_provider", next.provider);
                }
                if (typeof next.endpoint === "string") {
                  setField("observability_endpoint", next.endpoint);
                }
                if (typeof next.serviceName === "string") {
                  setField("observability_service_name", next.serviceName);
                }
                if (typeof next.headerName === "string") {
                  setField("observability_header_name", next.headerName);
                }
                if (typeof next.privacyMode === "string") {
                  setField("observability_privacy_mode", next.privacyMode);
                }
                if (typeof next.authToken === "string") {
                  setField("observability_auth_token", next.authToken);
                }
              }}
              onTest={async () => {
                setError(null);
                setSuccess(null);
                try {
                  await testObservabilityMutation.mutateAsync();
                } catch {
                  // handled by mutation onError
                }
              }}
            />
          </Box>

          {/* ── System Controls group ── */}
          <Box className="adv-group">
            <div className="adv-group-header">
              <div className="adv-group-header-icon" style={{ background: "rgba(47, 212, 255, 0.10)", border: "1px solid rgba(47, 212, 255, 0.22)" }}>
                <SettingsRoundedIcon sx={{ fontSize: 16, color: "#2fd4ff" }} />
              </div>
              <div>
                <div className="adv-group-header-title">System Controls</div>
                <div className="adv-group-header-sub">Core runtime and interface options</div>
              </div>
            </div>

            <div className="adv-row">
              <Stack spacing={0.2}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>Restart Bot</Typography>
                <Typography variant="caption" color="text.secondary">
                  Restarts AgentArk to apply runtime and security changes.
                </Typography>
              </Stack>
              <Button
                size="small"
                color="warning"
                variant="outlined"
                onClick={async () => {
                  const ok = window.confirm("Restart AgentArk?");
                  if (!ok) return;
                  setError(null);
                  setSuccess(null);
                  try {
                    await restartMutation.mutateAsync();
                    setTimeout(() => window.location.reload(), 2000);
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
                disabled={restartMutation.isPending}
                sx={{ whiteSpace: "nowrap" }}
              >
                Restart Bot
              </Button>
            </div>

            <div className="adv-row">
              <Stack spacing={0.2}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>Developer Mode</Typography>
                <Typography variant="caption" color="text.secondary">
                  Enables raw SKILL.md editing. Keep off for beginner-friendly forms.
                </Typography>
              </Stack>
              <FormControlLabel
                control={
                  <Switch
                    checked={developerModeEnabled}
                    onChange={(e) => {
                      const next = e.target.checked;
                      setDeveloperModeEnabled(next);
                      setDeveloperModeEnabledState(next);
                      setError(null);
                      setSuccess(next ? "Developer mode enabled." : "Developer mode disabled.");
                    }}
                  />
                }
                label={developerModeEnabled ? "On" : "Off"}
                sx={{ mr: 0 }}
              />
            </div>

            <div className="adv-row">
              <Stack spacing={0.2}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>Guided Tour</Typography>
                <Typography variant="caption" color="text.secondary">
                  Re-run the onboarding walkthrough to review core features.
                </Typography>
              </Stack>
              <Button
                size="small"
                variant="outlined"
                onClick={() => {
                  try { window.localStorage.setItem("agentark.tour.completed", "0"); } catch {}
                  const { startTour } = useUiStore.getState();
                  startTour();
                }}
                sx={{ whiteSpace: "nowrap" }}
              >
                Restart Tour
              </Button>
            </div>
          </Box>

          {/* ── Permissions group ── */}
          <Box className="adv-group">
            <div className="adv-group-header">
              <div className="adv-group-header-icon" style={{ background: "rgba(20, 241, 149, 0.10)", border: "1px solid rgba(20, 241, 149, 0.22)" }}>
                <span style={{ fontSize: 15 }}>&#128274;</span>
              </div>
              <div>
                <div className="adv-group-header-title">Permissions</div>
                <div className="adv-group-header-sub">Action approval and auto-approve settings</div>
              </div>
            </div>

            {/* Auto-Approve Skills */}
            <Typography variant="body2" sx={{ fontWeight: 600, mb: 0.5 }}>Auto-Approve Skills</Typography>
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1.5 }}>
              Select skills that can run without approval. Backend validates and may reject dangerous entries.
            </Typography>
            {(() => {
              const items = [
                "web_search",
                "research",
                "generate_image",
                "generate_video",
                "browse",
                "file_read",
                "file_write",
                "http_get",
                "shell",
                "code_execute",
                "schedule_task",
                "list_tasks",
                "clipboard_read",
                "clipboard_write",
                "gmail_scan",
                "gmail_reply"
              ];
              const set = new Set(parseCsvList(form.auto_approve_csv));
              const update = (name: string, checked: boolean) => {
                const next = new Set(set);
                if (checked) next.add(name);
                else next.delete(name);
                setField("auto_approve_csv", Array.from(next).sort().join(", "));
              };
              return (
                <>
                  <Grid2 container spacing={1}>
                    {items.map((name) => {
                      const active = set.has(name);
                      return (
                        <Grid2 key={name} size={{ xs: 6, md: 4, lg: 3 }}>
                          <div
                            className={`adv-skill-pill${active ? " active" : ""}`}
                            onClick={() => update(name, !active)}
                          >
                            <Typography variant="caption" sx={{ fontFamily: "'JetBrains Mono', monospace", fontSize: "0.7rem", letterSpacing: "0.02em" }}>
                              {name}
                            </Typography>
                            <Switch size="small" checked={active} onChange={(e) => update(name, e.target.checked)} />
                          </div>
                        </Grid2>
                      );
                    })}
                  </Grid2>
                  <TextField
                    label="Custom (CSV)"
                    value={form.auto_approve_csv}
                    onChange={(e) => setField("auto_approve_csv", e.target.value)}
                    fullWidth
                    size="small"
                    placeholder="comma separated action names"
                    sx={{ mt: 1.5 }}
                  />
                </>
              );
            })()}
          </Box>

          {/* ── API Access group ── */}
          <Box className="adv-group">
            <div className="adv-group-header">
              <div className="adv-group-header-icon" style={{ background: "rgba(255, 180, 50, 0.10)", border: "1px solid rgba(255, 180, 50, 0.22)" }}>
                <span style={{ fontSize: 15 }}>&#128273;</span>
              </div>
              <div>
                <div className="adv-group-header-title">API Access</div>
                <div className="adv-group-header-sub">HTTP API key management</div>
              </div>
            </div>

            {apiKeyQ.isLoading ? (
              <Typography variant="body2" color="text.secondary">Loading API key...</Typography>
            ) : apiKeyQ.error ? (
              <Alert severity="error">{errMessage(apiKeyQ.error)}</Alert>
            ) : (
              <Stack spacing={1.5}>
                <Stack direction="row" spacing={2} alignItems="center" flexWrap="wrap">
                  <Typography variant="caption" color="text.secondary" sx={{ flex: "1 1 auto" }}>
                    Used as <code style={{ background: "rgba(47,212,255,0.08)", padding: "1px 5px", borderRadius: 3, fontSize: "0.72rem", color: "#2fd4ff" }}>Authorization: Bearer &lt;key&gt;</code> for all HTTP requests.
                  </Typography>
                  <Chip
                    size="small"
                    color={apiKeyRemainingSeconds > 0 ? "info" : "warning"}
                    label={`Rotates in ${formatDurationClock(apiKeyRemainingSeconds)}`}
                  />
                </Stack>
                {apiKeyRotated ? (
                  <Chip size="small" color="success" label="API key rotated automatically" />
                ) : null}
                <TextField
                  label="Key"
                  value={apiKeyRevealed ? str(apiKeyPayload.key, "") : str(apiKeyPayload.masked, "")}
                  fullWidth
                  size="small"
                  InputProps={{
                    readOnly: true,
                    sx: { fontFamily: "'JetBrains Mono', 'Fira Code', monospace", fontSize: "0.78rem", letterSpacing: "0.04em" }
                  }}
                />
                {apiKeyIssuedAtUnix > 0 ? (() => {
                  const { label: issuedLabel, tip: issuedTip } = humanTs(new Date(apiKeyIssuedAtUnix * 1000).toISOString());
                  return (
                    <Tooltip title={issuedTip} placement="top">
                      <Typography variant="caption" color="text.secondary" sx={{ cursor: "default" }}>
                        Issued {issuedLabel}
                      </Typography>
                    </Tooltip>
                  );
                })() : null}
                {apiKeyExpiresAtUnix > 0 ? (() => {
                  const { label: expiresLabel, tip: expiresTip } = humanTs(new Date(apiKeyExpiresAtUnix * 1000).toISOString());
                  return (
                    <Tooltip title={expiresTip} placement="top">
                      <Typography variant="caption" color="text.secondary" sx={{ cursor: "default" }}>
                        Expires {expiresLabel}
                      </Typography>
                    </Tooltip>
                  );
                })() : null}
                <Stack direction="row" spacing={1}>
                  <Button size="small" variant="outlined" onClick={() => setApiKeyRevealed((v) => !v)}>
                    {apiKeyRevealed ? "Hide" : "Reveal"}
                  </Button>
                  <Button
                    size="small"
                    variant="outlined"
                    onClick={async () => {
                      const key = str(apiKeyPayload.key, "");
                      if (!key) return;
                      await navigator.clipboard.writeText(key);
                      setSuccess("API key copied.");
                    }}
                    disabled={!str(apiKeyPayload.key, "").trim()}
                  >
                    Copy
                  </Button>
                  <Button
                    size="small"
                    color="warning"
                    variant="outlined"
                    onClick={async () => {
                      const ok = window.confirm("Regenerate API key? Old key will stop working.");
                      if (!ok) return;
                      setError(null);
                      setSuccess(null);
                      try {
                        await regenerateApiKeyMutation.mutateAsync();
                        setApiKeyRevealed(true);
                        setSuccess("API key regenerated.");
                      } catch (e) {
                        setError(errMessage(e));
                      }
                    }}
                    disabled={regenerateApiKeyMutation.isPending}
                  >
                    Regenerate
                  </Button>
                </Stack>
              </Stack>
            )}
          </Box>

        </Stack>
      ) : null}

      {tab === 7 ? (
        <Stack spacing={2}>
          {/* ── Header + Enable + API Key ── */}
          <Box className="list-shell">
            <Stack spacing={1.5}>
              <Stack direction="row" justifyContent="space-between" alignItems="center">
                <Stack direction="row" spacing={1.5} alignItems="center">
                  <Typography variant="h6">Moltbook</Typography>
                  <Box sx={{
                    width: 8, height: 8, borderRadius: "50%",
                    bgcolor: form.moltbook_enabled ? (moltbookNeedsConnection ? "#f59e0b" : "#22c55e") : "#555",
                  }} />
                  <Typography variant="caption" color="text.secondary">
                    {form.moltbook_enabled ? (moltbookNeedsConnection ? "Not connected" : "Connected") : "Disabled"}
                  </Typography>
                </Stack>
                <FormControlLabel
                  control={<Switch checked={form.moltbook_enabled} onChange={(e) => setField("moltbook_enabled", e.target.checked)} />}
                  label="Enabled"
                />
              </Stack>
              <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 680 }}>
                Decentralized agent-to-agent network. Zero-knowledge — no user data or conversation content leaves your instance.
              </Typography>
              {form.moltbook_enabled ? (
                <Stack spacing={0.5} sx={{ mt: 0.5 }}>
                  <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
                    API Key
                  </Typography>
                  <TextField
                    type="password"
                    value={form.moltbook_api_key}
                    onChange={(e) => setField("moltbook_api_key", e.target.value)}
                    size="small"
                    placeholder="mk-..."
                    fullWidth
                    sx={{ maxWidth: 420 }}
                    autoComplete="new-password"
                    name="moltbook_integration_key"
                    inputProps={{
                      autoComplete: "new-password",
                      "data-1p-ignore": "true",
                      "data-lpignore": "true",
                      "data-form-type": "other"
                    }}
                  />
                  <Typography variant="caption" color="text.secondary">
                    Required to connect. Get your key at moltbook.com
                  </Typography>
                </Stack>
              ) : null}
              <Grid2 container spacing={2} sx={{ mt: 0.5, opacity: form.moltbook_enabled ? 1 : 0.4, pointerEvents: form.moltbook_enabled ? "auto" : "none", transition: "opacity 0.2s" }}>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  label="Participation Mode"
                  select
                  value={form.moltbook_mode}
                  onChange={(e) => {
                    const val = e.target.value;
                    if (val === "autopost") {
                      setForm((prev) => ({ ...prev, moltbook_mode: val, moltbook_write_enabled: true }));
                      setDirty(true);
                      setSuccess(null);
                      return;
                    }
                    setField("moltbook_mode", val);
                  }}
                  fullWidth
                  size="small"
                  disabled={!form.moltbook_enabled}
                >
                  {moltbookParticipationModes.map((option) => (
                    <MenuItem key={option.value} value={option.value}>
                      {option.label}{option.value === "autopost" ? " (recommended)" : ""} — {option.shortLabel}
                    </MenuItem>
                  ))}
                </TextField>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  label="Run Schedule"
                  select
                  value={moltbookScheduleMode}
                  onChange={(e) => {
                    const next = e.target.value;
                    if (next === "__custom__") {
                      if (moltbookPresetValues.has(form.moltbook_sync_frequency)) {
                        setField("moltbook_sync_frequency", "0 0 */12 * * *");
                      }
                      return;
                    }
                    setField("moltbook_sync_frequency", next);
                  }}
                  fullWidth
                  size="small"
                  disabled={!form.moltbook_enabled}
                >
                  {moltbookSchedulePresets.map((option) => (
                    <MenuItem key={option.value} value={option.value}>{option.label}</MenuItem>
                  ))}
                  <MenuItem value="__custom__">Custom cron</MenuItem>
                </TextField>
              </Grid2>
              {moltbookScheduleMode === "__custom__" ? (
                <Grid2 size={{ xs: 12, md: 4 }}>
                  <TextField
                    label="Custom Cron"
                    value={form.moltbook_sync_frequency}
                    onChange={(e) => setField("moltbook_sync_frequency", e.target.value)}
                    fullWidth
                    size="small"
                    placeholder="0 0 */6 * * *"
                    disabled={!form.moltbook_enabled}
                  />
                </Grid2>
              ) : null}
              <Grid2 size={{ xs: 12 }}>
                <Stack direction="row" spacing={3} alignItems="center">
                  <FormControlLabel
                    control={<Switch size="small" checked={form.moltbook_write_enabled} onChange={(e) => setField("moltbook_write_enabled", e.target.checked)} disabled={!form.moltbook_enabled} />}
                    label={<Typography variant="body2">Allow autonomous engagement</Typography>}
                  />
                  <FormControlLabel
                    control={<Switch size="small" checked={form.moltbook_defer_when_busy} onChange={(e) => setField("moltbook_defer_when_busy", e.target.checked)} disabled={!form.moltbook_enabled} />}
                    label={<Typography variant="body2">Defer When Busy</Typography>}
                  />
                </Stack>
                {form.moltbook_mode === "autopost" && !form.moltbook_write_enabled ? (
                  <Alert severity="warning" sx={{ mt: 1 }}>
                    Engage mode is selected, but autonomous actions are currently turned off. Enable <strong>Allow autonomous engagement</strong> if you want AgentArk to reply, vote, and post automatically.
                  </Alert>
                ) : null}
              </Grid2>
            </Grid2>
            </Stack>
          </Box>

          {/* ── Connector Status (inline compact) ── */}
          <Box className="list-shell" sx={{ opacity: form.moltbook_enabled ? 1 : 0.4, pointerEvents: form.moltbook_enabled ? "auto" : "none", transition: "opacity 0.2s" }}>
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Stack direction="row" spacing={1.5} alignItems="center">
                <Typography variant="subtitle2">Connector</Typography>
                {form.moltbook_enabled ? (
                  <Chip
                    size="small"
                    label={str(moltbookStatus.last_status, "unknown")}
                    color={str(moltbookStatus.last_status, "").toLowerCase() === "ok" ? "success" : "default"}
                    variant="outlined"
                  />
                ) : null}
              </Stack>
              <Button
                size="small"
                variant="outlined"
                onClick={async () => {
                  setError(null);
                  setSuccess(null);
                  try {
                    const out = asRecord(await runMoltbookMutation.mutateAsync());
                    const status = str(out.status, "ok").toLowerCase();
                    if (status === "ok") {
                      const readCount = num(out.read_count, 0);
                      const commentCount = num(out.comment_count, 0);
                      const upvoteCount = num(out.upvote_count, 0);
                      const postCount = num(out.post_count, toBool(out.posted) ? 1 : 0);
                      const parts = [`Read ${readCount} post${readCount === 1 ? "" : "s"}`];
                      if (commentCount > 0) parts.push(`${commentCount} comment${commentCount === 1 ? "" : "s"}`);
                      if (upvoteCount > 0) parts.push(`${upvoteCount} upvote${upvoteCount === 1 ? "" : "s"}`);
                      if (postCount > 0) parts.push(`${postCount} new post${postCount === 1 ? "" : "s"}`);
                      if (commentCount + upvoteCount + postCount === 0) parts.push("no public action taken");
                      setSuccess(`Moltbook run completed. ${parts.join(", ")}.`);
                    } else if (status === "started") {
                      setMoltbookPollState({
                        baselineEventId: latestMoltbookEventId,
                        deadlineAt: Date.now() + 3 * 60 * 1000
                      });
                      setSuccess("Moltbook run started in the background. Watch Connector status or Moltbook Activity for completion.");
                    } else if (status === "running") {
                      setMoltbookPollState({
                        baselineEventId: latestMoltbookEventId,
                        deadlineAt: Date.now() + 3 * 60 * 1000
                      });
                      setSuccess(str(out.message, "Moltbook run is already in progress."));
                    } else if (status === "not_connected") {
                      setError("Not connected. Save your API key first, then run.");
                    } else if (status === "disabled") {
                      setError("Moltbook is disabled.");
                    } else if (status === "off_mode") {
                      setError("Mode is off.");
                    } else if (status === "deferred_busy" || status === "skipped_busy") {
                      setError("Deferred: system busy.");
                    } else if (status === "not_due") {
                      setError("Not due yet.");
                    } else {
                      setSuccess(`Status: ${status}`);
                    }
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
                disabled={!form.moltbook_enabled || moltbookRunBusy}
              >
                {moltbookRunBusy ? "Running..." : "Run now"}
              </Button>
            </Stack>

            {moltbookStatusQ.error ? <Alert severity="error" sx={{ mb: 1 }}>{errMessage(moltbookStatusQ.error)}</Alert> : null}
            {form.moltbook_enabled && moltbookNeedsConnection ? (
              <Alert severity="warning" variant="outlined" sx={{ mb: 1, py: 0.3 }}>Not connected. Add API key, save, then run.</Alert>
            ) : null}
            <Stack direction="row" spacing={3} useFlexGap flexWrap="wrap" sx={{ fontSize: "0.82rem" }}>
              <Typography variant="caption" color="text.secondary">
                Last run <Box component="span" sx={{ color: "text.primary", fontWeight: 500 }} title={form.moltbook_enabled ? humanTs(str(moltbookStatus.last_run_at, "-")).tip : ""}>{form.moltbook_enabled ? humanTs(str(moltbookStatus.last_run_at, "-")).label : "-"}</Box>
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Next run <Box component="span" sx={{ color: "text.primary", fontWeight: 500 }} title={form.moltbook_enabled ? humanTs(str(moltbookStatus.next_run_at, "-")).tip : ""}>{form.moltbook_enabled ? humanTs(str(moltbookStatus.next_run_at, "-")).label : "-"}</Box>
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Last engagement <Box component="span" sx={{ color: "text.primary", fontWeight: 500 }} title={form.moltbook_enabled ? humanTs(str(moltbookStatus.last_engagement_at, "-")).tip : ""}>{form.moltbook_enabled ? humanTs(str(moltbookStatus.last_engagement_at, "-")).label : "-"}</Box>
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Summary <Box component="span" sx={{ color: "text.primary", fontWeight: 500 }}>
                  {form.moltbook_enabled
                    ? `read ${num(moltbookLastRunStats.read_count, 0)}, comments ${num(moltbookLastRunStats.comment_count, 0)}, upvotes ${num(moltbookLastRunStats.upvote_count, 0)}, posts ${num(moltbookLastRunStats.post_count, toBool(moltbookLastRunStats.posted) ? 1 : 0)}`
                    : "-"}
                </Box>
              </Typography>
            </Stack>
          </Box>

          <Box className="list-shell" sx={{ minHeight: 0 }}>
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">Moltbook Activity</Typography>
            </Stack>
            {moltbookLogQ.error ? <Alert severity="error">{errMessage(moltbookLogQ.error)}</Alert> : null}
            {moltbookRunRows.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No Moltbook runs yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Timestamp</TableCell>
                      <TableCell>Level</TableCell>
                      <TableCell>Action</TableCell>
                      <TableCell>Run</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {moltbookRunRows.slice(0, 40).map((ev, idx) => {
                      const details = asRecord(ev.details);
                      const runId = str(ev.run_id, "").trim();
                      const runEvents = runId
                        ? moltbookEvents.filter((event) => str(event.run_id, "").trim() === runId)
                        : [ev];
                      const counts = getMoltbookRunCounts(runEvents);
                      const rawAction = str(ev.action, "-");
                      const label = moltbookActionLabel(rawAction, details);
                      const summary = buildMoltbookRunSummary(runEvents);
                      const trigger = collectMoltbookRunTrigger(runEvents);
                      const triggerLabel = trigger ? moltbookTriggerLabel(trigger) : "";
                      const stepCount = counts.stepCount;
                      const hover = [
                        label,
                        summary ? `Summary: ${summary}` : "",
                        triggerLabel ? `Trigger: ${triggerLabel}` : "",
                        `${stepCount} step${stepCount === 1 ? "" : "s"}`
                      ]
                        .filter(Boolean)
                        .join("\n");
                      const evTs = humanTs(str(ev.timestamp, "-"));
                      const levelStr = buildMoltbookRunLevel(runEvents);
                      const levelColor = levelStr === "error" ? "#ff6b6b" : levelStr === "warning" ? "#ffa726" : "#66bb6a";
                      return (
                      <TableRow key={str(ev.id, String(idx))}>
                        <TableCell sx={{ whiteSpace: "nowrap" }}>
                          <Typography variant="body2" title={evTs.tip}>{evTs.label}</Typography>
                        </TableCell>
                        <TableCell>
                          <Chip
                            size="small"
                            label={levelStr}
                            color={levelStr === "error" ? "error" : levelStr === "warning" ? "warning" : "success"}
                          />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 420 }}>
                          <Stack spacing={0.25}>
                            <Typography variant="body2" noWrap title={hover}>
                              {label}
                            </Typography>
                            <Stack direction="row" spacing={0.6} flexWrap="wrap" useFlexGap>
                              <Chip size="small" variant="outlined" label={`${counts.readCount} read`} />
                              <Chip size="small" variant="outlined" label={`${counts.commentCount} commented`} />
                              <Chip size="small" variant="outlined" label={`${counts.upvoteCount} liked`} />
                              <Chip size="small" variant="outlined" label={`${counts.postCount} posted`} />
                            </Stack>
                            {triggerLabel || stepCount > 0 ? (
                              <Typography variant="caption" color="text.secondary" noWrap title={hover}>
                                {triggerLabel ? `${triggerLabel} | ` : ""}
                                {stepCount} step{stepCount === 1 ? "" : "s"}
                              </Typography>
                            ) : null}
                          </Stack>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 120 }}>
                          <Typography variant="body2" noWrap title={str(ev.run_id, "-")} sx={{ fontFamily: "monospace", fontSize: "0.75rem", opacity: 0.6 }}>
                            {str(ev.run_id, "-").slice(0, 8)}
                          </Typography>
                          <Typography variant="caption" color="text.secondary">
                            {stepCount} step{stepCount === 1 ? "" : "s"}
                          </Typography>
                        </TableCell>
                        <TableCell align="right">
                          <RowOpsMenu
                            actions={[
                              {
                                label: "View",
                                onClick: () => setSelectedMoltbookEvent(ev)
                              }
                            ]}
                            ariaLabel="Moltbook event options"
                          />
                        </TableCell>
                      </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Stack>
      ) : null}

      {tab === 2 ? (
        <Box className="list-shell">
          <IntegrationsPanel autoRefresh={autoRefresh} embedded mode="integrations" />
        </Box>
      ) : null}

      {tab === 11 ? <TraceManager autoRefresh={autoRefresh} /> : null}

      {tab === 8 ? (
        <Box className="list-shell">
          <IntegrationsPanel autoRefresh={autoRefresh} embedded mode="mcp" />
        </Box>
      ) : null}

      {tab === 12 ? <MemoryManager autoRefresh={autoRefresh} /> : null}

      {tab === 9 ? ( 
        <Stack spacing={2}>
          <Grid2 container spacing={2} alignItems="stretch">
            <Grid2 size={{ xs: 12 }}>
              <Box className="list-shell" sx={{ minHeight: 0, height: "100%", display: "flex", flexDirection: "column" }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
                  <Typography variant="h6">ArkPulse</Typography>
                  <Button
                    size="small"
                    onClick={async () => {
                      setError(null);
                      const baselineEventId = latestPulseEventId;
                      setPulsePollState({
                        baselineEventId,
                        deadlineAt: Date.now() + 2 * 60 * 1000
                      });
                      try {
                        const out = asRecord(await triggerPulseMutation.mutateAsync());
                        const status = str(out.status, "").toLowerCase();
                        if (status === "running") {
                          setSuccess(str(out.message, "ArkPulse is already running."));
                        } else {
                          setSuccess(str(out.message, "ArkPulse check started."));
                        }
                      } catch (e) {
                        setPulsePollState(null);
                        setError(errMessage(e));
                      }
                    }}
                    disabled={triggerPulseMutation.isPending || pulseRunning}
                  >
                    {triggerPulseMutation.isPending || pulseRunning ? "Running..." : "Run now"}
                  </Button>
                </Stack>
                {pulseQ.error ? <Alert severity="error">{errMessage(pulseQ.error)}</Alert> : null}
                {!pulseQ.error ? (
                  <Alert severity={pulseRunning ? "info" : latestPulseFindingsCount > 0 ? "warning" : "success"} sx={{ mb: 1 }}>
                    <Typography variant="subtitle2">{latestPulseHeadline}</Typography>
                    <Typography variant="body2" color="text.secondary">
                      {latestPulseSubtitle}
                    </Typography>
                  </Alert>
                ) : null}
                {pulseEvents.length === 0 ? (
                  <Stack spacing={1} sx={{ flex: 1 }}>
                    <Typography variant="body2" color="text.secondary">
                      No ArkPulse events yet.
                    </Typography>
                    <Box className="metadata-box" sx={{ maxHeight: "none" }}>
                      <Typography variant="caption" color="text.secondary">
                        What is ArkPulse?
                      </Typography>
                      <Stack spacing={0.6} sx={{ mt: 0.75 }}>
                        <Typography variant="body2" color="text.secondary">
                          Periodic system check that summarizes operational health, safety posture, and execution drift.
                        </Typography>
                        <Typography variant="body2" color="text.secondary">
                          Run it after changing models, channels, or adding a new integration.
                        </Typography>
                        <Typography variant="body2" color="text.secondary">
                          Results show up here as an event stream with findings and a score.
                        </Typography>
                      </Stack>
                    </Box>
                    <Box sx={{ flex: 1 }} />
                  </Stack>
                ) : (
                  <TableContainer className="table-shell" sx={{ flex: 1, minHeight: 0 }}>
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Captured</TableCell>
                          <TableCell>Result</TableCell>
                          <TableCell>Health</TableCell>
                          <TableCell>Issues</TableCell>
                          <TableCell>Next step</TableCell>
                          <TableCell align="right">Ops</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {pulseEvents.slice(0, 40).map((ev, idx) => {
                          const details = asRecord(ev.details);
                          const findings = pickRecords(details, "doctor_findings").filter((f) =>
                            isUserActionableDoctorFinding(f)
                          );
                          const score = num(details.doctor_score, -1);
                          const status = str(ev.status, "-");
                          const ok = status.toLowerCase() === "ok";
                          const nextStep =
                            Array.isArray(findings) && findings.length > 0
                              ? "Open details and run Fix #1"
                              : "No action needed";
                          return (
                            <TableRow key={str(ev.id, String(idx))}>
                              <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(ev.timestamp, "-")).tip}>{humanTs(str(ev.timestamp, "-")).label}</TableCell>
                              <TableCell>
                                <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                                  <Box component="span" sx={{ width: 8, height: 8, borderRadius: "50%", flexShrink: 0, bgcolor: ok ? "rgba(74,210,157,0.85)" : "rgba(255,180,60,0.85)" }} />
                                  <Typography variant="body2" color="text.secondary" noWrap>{ok ? "OK" : status || "check"}</Typography>
                                </Box>
                              </TableCell>
                              <TableCell>{score >= 0 ? score : "-"}</TableCell>
                              <TableCell>{Array.isArray(findings) ? findings.length : 0}</TableCell>
                              <TableCell sx={{ maxWidth: 320 }}>
                                <Typography variant="body2" noWrap title={nextStep}>
                                  {nextStep}
                                </Typography>
                              </TableCell>
                              <TableCell align="right">
                                <RowOpsMenu
                                  actions={[
                                    {
                                      label: "View",
                                      onClick: () => setSelectedPulseEvent(ev)
                                    }
                                  ]}
                                  ariaLabel="ArkPulse event options"
                                />
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                )}
              </Box>
            </Grid2>
          </Grid2>
        </Stack>
      ) : null}

      {tab === 13 ? (
        <Stack spacing={2}>
          <Alert severity="info" sx={{ borderRadius: 2 }}>
            Evolution lets AgentArk improve its own routing and decision-making over time. It tests new strategies via canary rollouts, measures performance, and automatically promotes winners.
          </Alert>
          <Grid2 container spacing={2}>
            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Typography variant="h6" mb={1}>Evolution Status</Typography>
                <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1.5 }}>
                  Current state of the self-evolution engine. When enabled, AgentArk automatically generates and tests improved routing policies.
                </Typography>
                {evolutionQ.isLoading ? (
                  <Typography variant="body2" color="text.secondary">Loading evolution status...</Typography>
                ) : evolutionQ.error ? (
                  <Alert severity="error">{errMessage(evolutionQ.error)}</Alert>
                ) : (
                  <Stack spacing={1}>
                    <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                      <Tooltip title="When On, AgentArk periodically generates improved routing policies and tests them automatically." arrow>
                        <Typography variant="body2" sx={{ cursor: "help", textDecoration: "underline dotted", textDecorationColor: "rgba(140,170,210,0.35)", textUnderlineOffset: 3 }}>Self-evolve:</Typography>
                      </Tooltip>
                      <Chip size="small" color={toBool(evolution.self_evolve_enabled) ? "success" : "default"} label={toBool(evolution.self_evolve_enabled) ? "On" : "Off"} />
                    </Stack>
                    <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                      <Tooltip title="Canary deploys route a percentage of traffic to a new candidate policy to compare its performance against the current baseline before fully promoting it." arrow>
                        <Typography variant="body2" sx={{ cursor: "help", textDecoration: "underline dotted", textDecorationColor: "rgba(140,170,210,0.35)", textUnderlineOffset: 3 }}>Canary:</Typography>
                      </Tooltip>
                      <Chip size="small" color={toBool(evolutionCanary.enabled) ? "warning" : "default"} label={toBool(evolutionCanary.enabled) ? "On" : "Off"} />
                      <Tooltip title="The percentage of requests routed to the candidate policy for testing." arrow>
                        <Typography variant="caption" color="text.secondary" sx={{ cursor: "help" }}>
                          Rollout: {num(evolutionCanary.rollout_percent, 0)}%
                        </Typography>
                      </Tooltip>
                    </Stack>
                    <Tooltip title="The currently active routing policy. All non-canary traffic uses this version." arrow placement="right">
                      <Typography variant="body2" sx={{ cursor: "help" }}>
                        Baseline: {str(evolutionCanary.baseline_version, "routing-policy-default-v1")}
                      </Typography>
                    </Tooltip>
                    <Tooltip title="The new policy being tested via canary rollout. Shows '-' when no candidate is active." arrow placement="right">
                      <Typography variant="body2" sx={{ cursor: "help" }}>
                        Candidate: {str(evolutionCanary.candidate_version, "-")}
                      </Typography>
                    </Tooltip>
                    <Tooltip title="Result of the most recent promotion attempt (e.g. whether a candidate was promoted or rejected based on metrics)." arrow placement="right">
                      <Typography variant="body2" sx={{ cursor: "help" }}>
                        Last promotion: {str(evolution.last_promotion_result, "No evolution runs yet")}
                      </Typography>
                    </Tooltip>
                    <Tooltip title="How promotions are decided: 'auto' promotes automatically when metrics pass, 'manual' requires developer action, 'none' means evolution is idle." arrow placement="right">
                      <Typography variant="body2" sx={{ cursor: "help" }}>
                        Promotion mode: {str(evolution.promotion_mode, "none")}
                      </Typography>
                    </Tooltip>
                    <Tooltip title="The replay gate re-runs past conversations against the candidate to verify it doesn't regress before promotion." arrow placement="right">
                      <Typography variant="body2" sx={{ cursor: "help" }}>
                        Replay gate: {str(evolution.replay_gate_result, "-")}
                      </Typography>
                    </Tooltip>
                  </Stack>
                )}
              </Box>
            </Grid2>
            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Typography variant="h6" mb={1}>Deploy Guard Default</Typography>
                <Typography variant="body2" color="text.secondary" mb={1}>
                  Controls whether apps deployed by the agent require an access guard by default. When OFF, deployed apps are publicly accessible unless the request explicitly sets an access guard.
                </Typography>
                <Stack spacing={1}>
                  <Stack direction="row" spacing={1} alignItems="center">
                    <Typography variant="body2">Current default:</Typography>
                    <Chip
                      size="small"
                      color={toBool(evolution.deploy_guard_default) ? "warning" : "default"}
                      label={toBool(evolution.deploy_guard_default) ? "ON" : "OFF"}
                    />
                  </Stack>
                  <Stack direction="row" spacing={1}>
                    <Tooltip title="Require an access guard on all app deploys by default. Recommended for production use." arrow>
                      <span>
                        <Button
                          size="small"
                          variant="contained"
                          onClick={async () => {
                            setError(null);
                            setSuccess(null);
                            try {
                              await updateEvolutionSettingsMutation.mutateAsync({ deploy_guard_default: true });
                              setSuccess("Deploy guard default enabled.");
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={updateEvolutionSettingsMutation.isPending || toBool(evolution.deploy_guard_default)}
                        >
                          Enable Default Guard
                        </Button>
                      </span>
                    </Tooltip>
                    <Tooltip title="Allow apps to be deployed without an access guard unless explicitly specified." arrow>
                      <span>
                        <Button
                          size="small"
                          onClick={async () => {
                            setError(null);
                            setSuccess(null);
                            try {
                              await updateEvolutionSettingsMutation.mutateAsync({ deploy_guard_default: false });
                              setSuccess("Deploy guard default disabled.");
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={updateEvolutionSettingsMutation.isPending || !toBool(evolution.deploy_guard_default)}
                        >
                          Keep Default Off
                        </Button>
                      </span>
                    </Tooltip>
                  </Stack>
                </Stack>
              </Box>
            </Grid2>
          </Grid2>

          {developerModeEnabled ? (
            <Grid2 container spacing={2}>
              <Grid2 size={{ xs: 12 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Stack direction={{ xs: "column", md: "row" }} spacing={1} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }}>
                    <Box>
                      <Typography variant="h6">Developer Controls</Typography>
                      <Typography variant="caption" color="text.secondary">Manual overrides for canary deployments and policy management.</Typography>
                    </Box>
                    <Stack direction="row" spacing={1}>
                      <Tooltip title="Stop the active canary rollout and route 100% of traffic back to the baseline policy." arrow>
                        <span>
                          <Button
                            size="small"
                            onClick={async () => {
                              setError(null);
                              setSuccess(null);
                              try {
                                const result = await runEvolutionDevActionMutation.mutateAsync("disable_canary");
                                setSuccess(`Canary disabled.${evolutionTraceIdHint(result)}`);
                              } catch (e) {
                                setError(errMessage(e));
                              }
                            }}
                            disabled={runEvolutionDevActionMutation.isPending}
                          >
                            Disable Canary
                          </Button>
                        </span>
                      </Tooltip>
                      <Tooltip title="Immediately promote the candidate policy to become the new baseline. Use when you're confident the candidate performs well." arrow>
                        <span>
                          <Button
                            size="small"
                            variant="contained"
                            onClick={async () => {
                              const ok = window.confirm("Promote candidate policy to baseline now?");
                              if (!ok) return;
                              setError(null);
                              setSuccess(null);
                              try {
                                const result = await runEvolutionDevActionMutation.mutateAsync("promote_candidate");
                                setSuccess(`Candidate promoted to baseline.${evolutionTraceIdHint(result)}`);
                              } catch (e) {
                                setError(errMessage(e));
                              }
                            }}
                            disabled={runEvolutionDevActionMutation.isPending}
                          >
                            Promote Candidate
                          </Button>
                        </span>
                      </Tooltip>
                      <Tooltip title="Revert the baseline policy to its previous stored snapshot. Use if the current baseline is performing poorly." arrow>
                        <span>
                          <Button
                            size="small"
                            color="warning"
                            onClick={async () => {
                              const ok = window.confirm("Rollback baseline policy to stored snapshot?");
                              if (!ok) return;
                              setError(null);
                              setSuccess(null);
                              try {
                                const result = await runEvolutionDevActionMutation.mutateAsync("rollback_baseline");
                                setSuccess(`Rolled back to baseline snapshot.${evolutionTraceIdHint(result)}`);
                              } catch (e) {
                                setError(errMessage(e));
                              }
                            }}
                            disabled={runEvolutionDevActionMutation.isPending}
                          >
                            Rollback Baseline
                          </Button>
                        </span>
                      </Tooltip>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>

              <Grid2 size={{ xs: 12, lg: 6 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Tooltip title="Performance metrics for task-routing strategies. Strategies determine how the agent selects which model or approach to use for each request." arrow placement="top-start">
                    <Typography variant="h6" mb={1} sx={{ cursor: "help" }}>Strategy Metrics</Typography>
                  </Tooltip>
                  {evolutionDevQ.isLoading ? (
                    <Typography variant="body2" color="text.secondary">Loading developer metrics...</Typography>
                  ) : evolutionDevQ.error ? (
                    <Alert severity="error">{errMessage(evolutionDevQ.error)}</Alert>
                  ) : evolutionStrategyMetrics.length === 0 ? (
                    <Typography variant="body2" color="text.secondary">No strategy metrics yet.</Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>Version</TableCell>
                            <TableCell align="right">Samples</TableCell>
                            <TableCell align="right">Success</TableCell>
                            <TableCell align="right">Errors</TableCell>
                            <TableCell align="right">p95 (ms)</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {evolutionStrategyMetrics.map((row, idx) => (
                            <TableRow key={`${str(row.version, "strategy")}-${idx}`}>
                              <TableCell>{str(row.version, "-")}</TableCell>
                              <TableCell align="right">{num(row.samples, 0)}</TableCell>
                              <TableCell align="right">{(num(row.success_rate, 0) * 100).toFixed(1)}%</TableCell>
                              <TableCell align="right">{(num(row.error_rate, 0) * 100).toFixed(1)}%</TableCell>
                              <TableCell align="right">{row.p95_latency_ms == null ? "-" : num(row.p95_latency_ms, 0)}</TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  )}
                </Box>
              </Grid2>

              <Grid2 size={{ xs: 12 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Tooltip title="History of all evolution promotion attempts. Shows when candidates were tested, whether they improved accuracy, and if they were promoted to become the new baseline." arrow placement="top-start">
                    <Typography variant="h6" mb={1} sx={{ cursor: "help" }}>Lineage</Typography>
                  </Tooltip>
                  {evolutionDevQ.isLoading ? (
                    <Typography variant="body2" color="text.secondary">Loading lineage...</Typography>
                  ) : evolutionDevQ.error ? (
                    <Alert severity="error">{errMessage(evolutionDevQ.error)}</Alert>
                  ) : evolutionLineage.length === 0 ? (
                    <Typography variant="body2" color="text.secondary">No lineage entries found.</Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>Timestamp</TableCell>
                            <TableCell>Source</TableCell>
                            <TableCell align="right">Gain</TableCell>
                            <TableCell align="right">p-value</TableCell>
                            <TableCell align="right">Promoted</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {evolutionLineage.slice().reverse().map((row, idx) => (
                            <TableRow key={`${str(row.entry_id, "lineage")}-${idx}`}>
                              <TableCell sx={{ whiteSpace: "nowrap" }} title={humanTs(str(row.timestamp_utc, "-")).tip}>{humanTs(str(row.timestamp_utc, "-")).label}</TableCell>
                              <TableCell>{str(row.candidate_source, "-")}</TableCell>
                              <TableCell align="right">{num(row.accuracy_gain, 0).toFixed(4)}</TableCell>
                              <TableCell align="right">{num(row.p_value, 1).toFixed(4)}</TableCell>
                              <TableCell align="right">{toBool(row.promoted) ? "yes" : "no"}</TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  )}
                </Box>
              </Grid2>
            </Grid2>
          ) : (
            <Alert severity="info">
              Enable Developer mode in Settings -&gt; Advanced to view replay metrics, lineage, and manual canary controls.
            </Alert>
          )}
        </Stack>
      ) : null}
        </Box>
      </Box>

      <Dialog
        open={securityLogsDialogOpen}
        onClose={() => {
          setSecurityLogsDialogOpen(false);
          setSelectedSecurityLog(null);
        }}
        maxWidth="lg"
        fullWidth
      >
        <DialogTitle>Security Logs</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1}>
            <Stack direction="row" justifyContent="space-between" alignItems="center">
              <Typography variant="caption" color="text.secondary">
                event_type | severity | message
              </Typography>
              <Button size="small" onClick={() => void securityLogsQ.refetch()} disabled={securityLogsQ.isFetching}>
                {securityLogsQ.isFetching ? "Refreshing..." : "Refresh"}
              </Button>
            </Stack>
            {securityLogsQ.isLoading ? (
              <Typography variant="body2" color="text.secondary">
                Loading security logs...
              </Typography>
            ) : securityLogsQ.error ? (
              <Alert severity="error">{errMessage(securityLogsQ.error)}</Alert>
            ) : securityLogs.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No security logs yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>event_type</TableCell>
                      <TableCell>severity</TableCell>
                      <TableCell>message</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {securityLogs.map((row, idx) => {
                      const eventType = str(row.event_type, "-");
                      const severity = str(row.severity, "-");
                      const message = str(row.message, "-");
                      const rowKey = `${str(row.created_at, "")}:${eventType}:${idx}`;
                      const selected = selectedSecurityLog === row;
                      return (
                        <TableRow
                          key={rowKey}
                          hover
                          selected={selected}
                          sx={{ cursor: "pointer" }}
                          onClick={() => setSelectedSecurityLog(row)}
                        >
                          <TableCell sx={{ whiteSpace: "nowrap" }}>{eventType}</TableCell>
                          <TableCell sx={{ whiteSpace: "nowrap" }}>
                            <Chip size="small" label={severity} color={severityChipColor(severity)} />
                          </TableCell>
                          <TableCell sx={{ maxWidth: 560 }}>
                            <Typography variant="body2" noWrap title={message}>
                              {message}
                            </Typography>
                          </TableCell>
                          <TableCell align="right">
                            <Button
                              size="small"
                              onClick={(e) => {
                                e.stopPropagation();
                                setSelectedSecurityLog(row);
                              }}
                            >
                              View
                            </Button>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
            )}

            {selectedSecurityLog ? (
              <Box className="metadata-box">
                <Stack spacing={0.5}>
                  <Typography variant="subtitle2">Selected Log</Typography>
                  <Typography variant="body2">
                    <strong>event_type:</strong> {str(selectedSecurityLog.event_type, "-")}
                  </Typography>
                  <Typography variant="body2">
                    <strong>severity:</strong> {str(selectedSecurityLog.severity, "-")}
                  </Typography>
                  <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                    <strong>message:</strong> {str(selectedSecurityLog.message, "-")}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    source: {str(selectedSecurityLog.source, "-")} | created_at: <span title={humanTs(str(selectedSecurityLog.created_at, "-")).tip}>{humanTs(str(selectedSecurityLog.created_at, "-")).label}</span> | count: {str(selectedSecurityLog.count, "-")}
                  </Typography>
                </Stack>
              </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setSecurityLogsDialogOpen(false);
              setSelectedSecurityLog(null);
            }}
          >
            Close
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog open={selectedPulseEvent != null} onClose={() => setSelectedPulseEvent(null)} maxWidth="lg" fullWidth>
        <DialogTitle>{str(selectedPulseEvent?.summary, "ArkPulse Details")}</DialogTitle>
        <DialogContent>
          <Stack spacing={1.25}>
            <Stack direction={{ xs: "column", sm: "row" }} spacing={1} alignItems={{ xs: "flex-start", sm: "center" }}>
              <Chip size="small" variant="outlined" label={`Captured: ${selectedPulseCaptured.label}`} title={selectedPulseCaptured.tooltip} />
              <Chip
                size="small"
                label={`Status: ${selectedPulseStatus}`}
                color={selectedPulseStatusOk ? "success" : "warning"}
                variant={selectedPulseStatusOk ? "filled" : "outlined"}
              />
            </Stack>
            <Alert severity={selectedPulseGuidance.severity} variant="outlined">
              <Typography variant="subtitle2">{selectedPulseGuidance.title}</Typography>
              <Typography variant="body2" color="text.secondary">
                {selectedPulseGuidance.detail}
              </Typography>
            </Alert>
            <Divider />
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Box className="metadata-box">
                  <Typography variant="caption" color="text.secondary">
                    Health score
                  </Typography>
                  <Typography variant="h5">
                    {selectedPulseScore >= 0 ? selectedPulseScore : "-"}
                  </Typography>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Box className="metadata-box">
                  <Typography variant="caption" color="text.secondary">
                    Findings
                  </Typography>
                  <Typography variant="h5">{selectedPulseFindings.length}</Typography>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Box className="metadata-box">
                  <Typography variant="caption" color="text.secondary">
                    Watchers
                  </Typography>
                  <Typography variant="h5">{num(selectedPulseDetails.active_watchers, 0)}</Typography>
                </Box>
              </Grid2>
            </Grid2>

            <Typography variant="subtitle2" mt={1}>
              Fix these first
            </Typography>
            {selectedPulseFindings.length === 0 ? (
              <Alert severity="success" variant="outlined">
                No findings in this run.
              </Alert>
            ) : (
              <Stack spacing={1}>
                {selectedPulseFindings.slice(0, 20).map((f, idx) => {
                  const fr = asRecord(f);
                  const sev = str(fr.severity, "");
                  const title = str(fr.title, "Issue");
                  const target = str(fr.target, "-");
                  const cause = str(fr.root_cause, "-");
                  const typedRemediation = parseArkPulseRemediationSpec(fr.remediation);
                  const runnableRemediation = typedRemediation ?? getRunnableArkPulseRemediation(fr);
                  const rawFixCommand = str(fr.fix_command, "").trim();
                  const displayRemediation = typedRemediation ?? runnableRemediation;
                  const fix = displayRemediation ? describeArkPulseRemediation(displayRemediation) : getArkPulseFixText(fr);
                  const canCopyFix = fix.trim().length > 0 && fix.trim() !== "-";
                  const canRunFix = runnableRemediation != null;
                  const fixActionId = `${title}:${target}:${idx}`;
                  const fixBusy = runPulseFixMutation.isPending && activePulseFixId === fixActionId;
                  return (
                    <Box key={`${title}-${idx}`} className="metadata-box">
                      <Stack spacing={0.75}>
                        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                          <Chip size="small" label={sev || "-"} color={severityChipColor(sev)} />
                          <Typography variant="subtitle2">{`Fix #${idx + 1}: ${title}`}</Typography>
                        </Stack>
                        <Typography variant="body2" color="text.secondary">
                          Target: {target}
                        </Typography>
                        <Typography variant="body2" color="text.secondary">
                          Why this matters: {cause}
                        </Typography>
                        <Box
                          sx={{
                            border: "1px solid rgba(62,143,214,0.24)",
                            borderRadius: 1,
                            p: 1,
                            background: "rgba(5,16,31,0.45)"
                          }}
                        >
                          <Typography variant="caption" color="text.secondary">
                            Recommended remediation
                          </Typography>
                          <Typography
                            variant="body2"
                            sx={{
                              mt: 0.5,
                              fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                              whiteSpace: "pre-wrap",
                              overflowWrap: "anywhere"
                            }}
                          >
                            {fix}
                          </Typography>
                        </Box>
                        <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={!canCopyFix}
                            onClick={async () => {
                              setError(null);
                              setSuccess(null);
                              try {
                                await copyClipboardText(fix);
                                setSuccess("Remediation copied.");
                              } catch (e) {
                                setError(errMessage(e));
                              }
                            }}
                          >
                            Copy remediation
                          </Button>
                          <Button
                            size="small"
                            variant="contained"
                            disabled={!canRunFix || runPulseFixMutation.isPending}
                            onClick={async () => {
                              setError(null);
                              setSuccess(null);
                              setActivePulseFixId(fixActionId);
                              try {
                                await runPulseFixMutation.mutateAsync({
                                  fixCommand: rawFixCommand,
                                  remediation: typedRemediation,
                                  issueTitle: title,
                                  target,
                                  eventTimestamp: str(selectedPulseEvent?.timestamp, ""),
                                  findingIndex: idx
                                });
                              } catch {
                                // handled by mutation onError
                              } finally {
                                setActivePulseFixId((prev) => (prev === fixActionId ? null : prev));
                              }
                            }}
                          >
                            {fixBusy ? "Running..." : "Run fix now"}
                          </Button>
                        </Stack>
                        <Typography variant="caption" color="text.secondary">
                          {typedRemediation
                            ? "Runs directly from ArkPulse using the finding's typed remediation."
                            : canRunFix
                              ? "Legacy event: ArkPulse is using the saved fix command fallback for this run."
                              : "This remediation is advisory only. Copy and run manually."}
                        </Typography>
                      </Stack>
                    </Box>
                  );
                })}
              </Stack>
            )}

            <Typography variant="subtitle2" mt={0.5}>
              Current system snapshot
            </Typography>
            <Grid2 container spacing={1}>
              {selectedPulseSnapshot.map((item) => (
                <Grid2 key={item.label} size={{ xs: 6, md: 3 }}>
                  <Box className="metadata-box" sx={{ minHeight: 86 }}>
                    <Typography variant="caption" color="text.secondary">
                      {item.label}
                    </Typography>
                    <Typography variant="h6">{item.value}</Typography>
                  </Box>
                </Grid2>
              ))}
            </Grid2>

            {developerModeEnabled ? (
              <Accordion disableGutters sx={{ background: "transparent", boxShadow: "none", border: "1px solid rgba(62,143,214,0.24)", borderRadius: 1 }}>
                <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                  <Typography variant="subtitle2">Technical signals (developer mode)</Typography>
                </AccordionSummary>
                <AccordionDetails sx={{ pt: 0 }}>
                  <KeyValuePanel title="Raw signals" data={asRecord(selectedPulseEvent?.details)} emptyLabel="No extra signals." maxRows={24} />
                </AccordionDetails>
              </Accordion>
            ) : null}
          </Stack>
        </DialogContent>
      </Dialog>

      {settingsQ.isLoading || mediaQ.isLoading ? (
        <Typography variant="body2" color="text.secondary">
          Loading settings...
        </Typography>
      ) : null}
      <Dialog
        open={selectedMoltbookEvent != null}
        onClose={() => setSelectedMoltbookEvent(null)}
        maxWidth="lg"
        fullWidth
      >
        <DialogTitle>Moltbook Run</DialogTitle>
        <DialogContent>
          <Stack spacing={1.5} sx={{ pt: 0.5 }}>
            <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
              <Typography variant="subtitle1">
                {moltbookActionLabel(
                  str(selectedMoltbookRepresentativeEvent?.action, ""),
                  asRecord(selectedMoltbookRepresentativeEvent?.details)
                )}
              </Typography>
              <Chip
                size="small"
                label={selectedMoltbookRunLevel}
                color={
                  selectedMoltbookRunLevel === "error"
                    ? "error"
                    : selectedMoltbookRunLevel === "warning"
                      ? "warning"
                      : "success"
                }
              />
            </Stack>

            <Typography variant="caption" color="text.secondary">
              <span title={humanTs(str(selectedMoltbookRepresentativeEvent?.timestamp, "")).tip}>
                {humanTs(str(selectedMoltbookRepresentativeEvent?.timestamp, "")).label}
              </span>
              {" | "}Run: {str(selectedMoltbookRepresentativeEvent?.run_id, "-")}
              {" | "}{selectedMoltbookRunCounts.stepCount} step{selectedMoltbookRunCounts.stepCount === 1 ? "" : "s"}
            </Typography>

            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 6, md: 3 }}>
                <Box className="metadata-box" sx={{ minHeight: 84 }}>
                  <Typography variant="caption" color="text.secondary">Read</Typography>
                  <Typography variant="h6">{selectedMoltbookRunCounts.readCount}</Typography>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 6, md: 3 }}>
                <Box className="metadata-box" sx={{ minHeight: 84 }}>
                  <Typography variant="caption" color="text.secondary">Commented</Typography>
                  <Typography variant="h6">{selectedMoltbookRunCounts.commentCount}</Typography>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 6, md: 3 }}>
                <Box className="metadata-box" sx={{ minHeight: 84 }}>
                  <Typography variant="caption" color="text.secondary">Liked</Typography>
                  <Typography variant="h6">{selectedMoltbookRunCounts.upvoteCount}</Typography>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 6, md: 3 }}>
                <Box className="metadata-box" sx={{ minHeight: 84 }}>
                  <Typography variant="caption" color="text.secondary">Posted</Typography>
                  <Typography variant="h6">{selectedMoltbookRunCounts.postCount}</Typography>
                </Box>
              </Grid2>
            </Grid2>

            {selectedMoltbookRunTrigger ? (
              <Alert severity="info">Trigger: {moltbookTriggerLabel(selectedMoltbookRunTrigger)}</Alert>
            ) : null}
            {selectedMoltbookRunSummary ? (
              <Alert
                severity={
                  selectedMoltbookRunLevel === "error"
                    ? "error"
                    : selectedMoltbookRunLevel === "warning"
                      ? "warning"
                      : "success"
                }
              >
                {selectedMoltbookRunSummary}
              </Alert>
            ) : null}

            {selectedMoltbookPostActivity.length ? (
              <Box className="metadata-box">
                <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                  <Typography variant="subtitle2">Per-post activity</Typography>
                  <Chip
                    size="small"
                    label={`${selectedMoltbookPostActivity.length} post${selectedMoltbookPostActivity.length === 1 ? "" : "s"}`}
                    sx={{ height: 20, fontSize: "0.7rem" }}
                  />
                </Stack>
                <Typography variant="caption" color="text.secondary">
                  Grouped by post so you can see exactly what this run read, liked, commented on, or published.
                </Typography>
                <Stack spacing={1} sx={{ mt: 1 }}>
                  {selectedMoltbookPostActivity.map((entry) => {
                    const lastActivity = humanTs(entry.actions[0]?.timestamp ?? "");
                    const uniqueActionKinds = Array.from(new Set(entry.actions.map((item) => item.kind)));
                    return (
                      <Box
                        key={entry.key}
                        sx={{
                          border: "1px solid rgba(62,143,214,0.18)",
                          borderRadius: 1,
                          p: 1.1,
                          background: "rgba(10, 15, 28, 0.35)",
                        }}
                      >
                        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                          <Typography variant="body2" sx={{ fontWeight: 600 }}>
                            {entry.url ? (
                              <Link href={entry.url} target="_blank" rel="noreferrer" underline="hover">
                                {entry.title}
                              </Link>
                            ) : (
                              entry.title
                            )}
                          </Typography>
                          {uniqueActionKinds.map((kind) => (
                            <Chip
                              key={`${entry.key}-${kind}`}
                              size="small"
                              variant="outlined"
                              label={
                                kind === "read"
                                  ? "Read"
                                  : kind === "commented"
                                    ? "Commented"
                                    : kind === "liked"
                                      ? "Liked"
                                      : kind === "posted"
                                        ? "Posted new"
                                        : kind === "comment_failed"
                                          ? "Comment failed"
                                          : kind === "like_failed"
                                            ? "Like failed"
                                            : "Post failed"
                              }
                              color={
                                kind === "comment_failed" || kind === "like_failed" || kind === "post_failed"
                                  ? "warning"
                                  : kind === "posted"
                                    ? "success"
                                    : "info"
                              }
                              sx={{ height: 20, fontSize: "0.68rem" }}
                            />
                          ))}
                        </Stack>
                        {(entry.submolt || entry.author || entry.postId) ? (
                          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.25 }}>
                            {[
                              entry.submolt ? `m/${entry.submolt}` : "",
                              entry.author ? `author ${entry.author}` : "",
                              entry.postId ? `id ${entry.postId}` : "",
                              entry.actions.length ? `last activity ${lastActivity.label}` : "",
                            ]
                              .filter(Boolean)
                              .join(" • ")}
                          </Typography>
                        ) : null}
                        <Stack spacing={0.55} sx={{ mt: 0.9 }}>
                          {entry.actions.map((item, index) => {
                            const actionTime = humanTs(item.timestamp);
                            return (
                              <Box
                                key={`${entry.key}-${item.kind}-${item.timestamp}-${index}`}
                                sx={{
                                  pl: 1,
                                  borderLeft:
                                    item.kind === "comment_failed" || item.kind === "like_failed" || item.kind === "post_failed"
                                      ? "2px solid rgba(245,158,11,0.55)"
                                      : "2px solid rgba(62,143,214,0.3)",
                                }}
                              >
                                <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                                  <Typography variant="caption" sx={{ fontWeight: 600 }}>
                                    {item.label}
                                  </Typography>
                                  <Typography variant="caption" color="text.secondary">
                                    <span title={actionTime.tip}>{actionTime.label}</span>
                                  </Typography>
                                </Stack>
                                {item.summary ? (
                                  <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.15 }}>
                                    {item.summary}
                                  </Typography>
                                ) : null}
                                {item.reason ? (
                                  <Typography
                                    variant="caption"
                                    color={
                                      item.kind === "comment_failed" || item.kind === "like_failed" || item.kind === "post_failed"
                                        ? "warning.main"
                                        : "text.secondary"
                                    }
                                    sx={{ display: "block", mt: 0.15 }}
                                  >
                                    {item.reason}
                                  </Typography>
                                ) : null}
                              </Box>
                            );
                          })}
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            ) : null}

            {selectedMoltbookRunLinks.length ? (
              <Box className="metadata-box">
                <Typography variant="subtitle2">Links</Typography>
                <Typography variant="caption" color="text.secondary">
                  Open posts, articles, or API references related to this run.
                </Typography>
                <Stack spacing={0.75} sx={{ mt: 1 }}>
                  {selectedMoltbookRunLinks.map((link) => (
                    <Box
                      key={`${link.label}-${link.url}`}
                      sx={{
                        border: "1px solid rgba(62,143,214,0.18)",
                        borderRadius: 1,
                        p: 1
                      }}
                    >
                      <Link href={link.url} target="_blank" rel="noreferrer" underline="hover">
                        {link.label}
                      </Link>
                      <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.35, wordBreak: "break-all" }}>
                        {link.url}
                      </Typography>
                    </Box>
                  ))}
                </Stack>
              </Box>
            ) : null}

            <Divider />

            <Accordion
              disableGutters
              defaultExpanded={false}
              sx={{
                background: "rgba(10, 15, 28, 0.6)",
                boxShadow: "none",
                border: "1px solid rgba(62,143,214,0.18)",
                borderRadius: "8px !important",
                "&:before": { display: "none" },
              }}
            >
              <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                <Stack direction="row" spacing={1} alignItems="center">
                  <Typography variant="subtitle2">Run steps</Typography>
                  <Chip size="small" label={`${selectedMoltbookDialogEvents.length} steps`} sx={{ height: 20, fontSize: "0.7rem" }} />
                </Stack>
              </AccordionSummary>
              <AccordionDetails sx={{ pt: 0 }}>
                <Stack spacing={0} divider={<Divider sx={{ borderColor: "rgba(62,143,214,0.10)" }} />}>
                  {selectedMoltbookDialogEvents.map((event, index) => {
                    const details = asRecord(event.details);
                    const action = str(event.action, "");
                    const summary = moltbookSummary(action, details);
                    const reason = moltbookReason(action, details);
                    const links = collectMoltbookLinks(details);
                    const level = str(event.level, "").toLowerCase();
                    const severity =
                      level === "error"
                        ? "error"
                        : level === "warning" || level === "warn"
                          ? "warning"
                          : "success";
                    const timestamp = humanTs(str(event.timestamp, ""));
                    return (
                      <Box key={str(event.id, `${index}`)} sx={{ py: 1, px: 0.5 }}>
                        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
                          <Chip size="small" label={level || "info"} color={severity} sx={{ height: 18, fontSize: "0.65rem" }} />
                          <Typography variant="body2" sx={{ fontWeight: 500 }}>
                            {index + 1}. {moltbookActionLabel(action, details)}
                          </Typography>
                          <Typography variant="caption" color="text.secondary">
                            <span title={timestamp.tip}>{timestamp.label}</span>
                          </Typography>
                        </Stack>
                        {summary ? <Typography variant="caption" color="text.secondary" sx={{ mt: 0.25, display: "block" }}>{summary}</Typography> : null}
                        {reason ? <Typography variant="caption" color="warning.main" sx={{ mt: 0.25, display: "block" }}>Reason: {reason}</Typography> : null}
                        {links.length ? (
                          <Stack direction="row" spacing={1} sx={{ mt: 0.35, flexWrap: "wrap" }} useFlexGap>
                            {links.map((link) => (
                              <Link
                                key={`${str(event.id, `${index}`)}-${link.label}-${link.url}`}
                                href={link.url}
                                target="_blank"
                                rel="noreferrer"
                                underline="hover"
                                variant="caption"
                                sx={{ wordBreak: "break-all" }}
                              >
                                {link.label}
                              </Link>
                            ))}
                          </Stack>
                        ) : null}
                      </Box>
                    );
                  })}
                </Stack>
              </AccordionDetails>
            </Accordion>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSelectedMoltbookEvent(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={vaultEditorOpen}
        onClose={closeVaultEditor}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Add Secret</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <TextField
              label="Secret key"
              value={vaultEditorKey}
              onChange={(e) => setVaultEditorKey(e.target.value)}
              fullWidth
              size="small"
              helperText="Allowed: letters, numbers, _, -, :, ."
            />
            <TextField
              label="Secret value"
              value={vaultEditorValue}
              onChange={(e) => setVaultEditorValue(e.target.value)}
              fullWidth
              size="small"
              multiline
              minRows={3}
              type={showVaultSecretValue ? "text" : "password"}
              placeholder="Paste secret value"
            />
            <FormControlLabel
              control={
                <Switch
                  checked={showVaultSecretValue}
                  onChange={(e) => setShowVaultSecretValue(e.target.checked)}
                />
              }
              label="Show secret value"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeVaultEditor} disabled={upsertVaultSecretMutation.isPending}>
            Cancel
          </Button>
          <Button
            variant="contained"
            onClick={submitVaultEditor}
            disabled={upsertVaultSecretMutation.isPending}
          >
            {upsertVaultSecretMutation.isPending ? "Saving..." : "Save Secret"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={passwordDialogMode != null}
        onClose={closePasswordDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {passwordDialogMode === "set"
            ? "Set Master Password"
            : passwordDialogMode === "change"
              ? "Change Master Password"
              : "Remove Master Password"}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              {resumeTunnelStartAfterPassword
                ? "Save a custom AgentArk password to finish creating the public link."
                : "Password changes apply immediately to this running AgentArk session."}
            </Alert>
            <FormControlLabel
              control={
                <Switch
                  checked={showPasswordInputs}
                  onChange={(e) => setShowPasswordInputs(e.target.checked)}
                />
              }
              label="Show password text"
            />
            {passwordDialogMode === "set" ? (
              <>
                <TextField
                  label="New password (min 8 chars)"
                  value={secNewPassword}
                  onChange={(e) => setSecNewPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="Confirm new password"
                  value={secConfirmPassword}
                  onChange={(e) => setSecConfirmPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
            {passwordDialogMode === "change" ? (
              <>
                <TextField
                  label="Current password (blank uses default, if applicable)"
                  value={secCurrentPassword}
                  onChange={(e) => setSecCurrentPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="New password (min 8 chars)"
                  value={secNewPassword}
                  onChange={(e) => setSecNewPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="Confirm new password"
                  value={secConfirmPassword}
                  onChange={(e) => setSecConfirmPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
            {passwordDialogMode === "remove" ? (
              <>
                <Typography variant="body2" color="text.secondary">
                  Removes the master password and returns to keyfile-based encryption.
                </Typography>
                <TextField
                  label="Current password"
                  value={secCurrentPassword}
                  onChange={(e) => setSecCurrentPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closePasswordDialog} disabled={passwordMutationPending}>
            Cancel
          </Button>
          <Button
            variant="contained"
            color={passwordDialogMode === "remove" ? "error" : "primary"}
            onClick={submitPasswordDialog}
            disabled={passwordMutationPending}
          >
            {passwordMutationPending
              ? "Saving..."
              : passwordDialogMode === "set"
                ? "Set Password"
                : passwordDialogMode === "change"
                  ? "Change Password"
                  : "Remove Password"}
          </Button>
        </DialogActions>
      </Dialog>
      {settingsQ.error || mediaQ.error || modelsQ.error || moltbookLogQ.error ? (
        <Alert severity="error">{errMessage(settingsQ.error || mediaQ.error || modelsQ.error || moltbookLogQ.error)}</Alert>
      ) : null}
      {error ? <Alert severity="error">{error}</Alert> : null}
      {modelConnectivityWarning ? <Alert severity="warning">{modelConnectivityWarning}</Alert> : null}
      {success ? <Alert severity="success">{success}</Alert> : null}
    </Stack>
  );
}

/* ───────────── Analytics (top-level page) ───────────── */

function AnalyticsManager({ autoRefresh }: { autoRefresh: boolean }) {
  type AnalyticsRange = "1h" | "2h" | "6h" | "24h" | "3d" | "7d" | "14d" | "21d" | "30d" | "45d" | "60d" | "90d" | "custom";
  type BreakdownView = "model" | "channel" | "purpose";

  const RANGE_PRESETS: { value: AnalyticsRange; label: string; hours: number }[] = [
    { value: "1h", label: "1 hour", hours: 1 },
    { value: "2h", label: "2 hours", hours: 2 },
    { value: "6h", label: "6 hours", hours: 6 },
    { value: "24h", label: "24 hours", hours: 24 },
    { value: "3d", label: "3 days", hours: 72 },
    { value: "7d", label: "7 days", hours: 168 },
    { value: "14d", label: "14 days", hours: 336 },
    { value: "21d", label: "21 days", hours: 504 },
    { value: "30d", label: "30 days", hours: 720 },
    { value: "45d", label: "45 days", hours: 1080 },
    { value: "60d", label: "60 days", hours: 1440 },
    { value: "90d", label: "90 days", hours: 2160 },
  ];

  function bucketForHours(hours: number): "hour" | "day" | "week" {
    if (hours <= 72) return "hour";
    if (hours <= 24 * 120) return "day";
    return "week";
  }

  function toLocalDatetimeInput(date: Date): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
  }

  function parseInputDate(value: string): Date | null {
    const t = Date.parse(value);
    return Number.isFinite(t) ? new Date(t) : null;
  }

  function compactNumber(value: number): string {
    if (!Number.isFinite(value)) return "0";
    if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
    if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
    return value.toLocaleString();
  }

  function shortVersionLabel(value: string, max = 28): string {
    if (!value || value.length <= max) return value;
    const head = Math.max(10, Math.floor((max - 3) / 2));
    const tail = Math.max(8, max - head - 3);
    return `${value.slice(0, head)}...${value.slice(-tail)}`;
  }

  const [activeRange, setActiveRange] = useState<AnalyticsRange>("24h");
  const [breakdownView, setBreakdownView] = useState<BreakdownView>("model");
  const [customDialogOpen, setCustomDialogOpen] = useState(false);
  const defaultCustomTo = useMemo(() => toLocalDatetimeInput(new Date()), []);
  const defaultCustomFrom = useMemo(
    () => toLocalDatetimeInput(new Date(Date.now() - 30 * 24 * 60 * 60 * 1000)),
    []
  );
  const [customFrom, setCustomFrom] = useState(defaultCustomFrom);
  const [customTo, setCustomTo] = useState(defaultCustomTo);
  const [appliedCustomFrom, setAppliedCustomFrom] = useState(defaultCustomFrom);
  const [appliedCustomTo, setAppliedCustomTo] = useState(defaultCustomTo);

  const customFromDate = useMemo(() => parseInputDate(customFrom), [customFrom]);
  const customToDate = useMemo(() => parseInputDate(customTo), [customTo]);
  const appliedFromDate = useMemo(() => parseInputDate(appliedCustomFrom), [appliedCustomFrom]);
  const appliedToDate = useMemo(() => parseInputDate(appliedCustomTo), [appliedCustomTo]);
  const customRangeInvalid =
    !customFromDate ||
    !customToDate ||
    customFromDate.getTime() >= customToDate.getTime();

  // Compute effective from/to ISO strings and bucket for the active range
  const { effectiveFrom, effectiveTo, effectiveBucket } = useMemo(() => {
    if (activeRange === "custom") {
      const from = appliedFromDate?.toISOString() ?? "";
      const to = appliedToDate?.toISOString() ?? "";
      const diffMs = (appliedToDate?.getTime() ?? 0) - (appliedFromDate?.getTime() ?? 0);
      const diffHours = diffMs / (1000 * 60 * 60);
      return { effectiveFrom: from, effectiveTo: to, effectiveBucket: bucketForHours(diffHours) };
    }
    const preset = RANGE_PRESETS.find((p) => p.value === activeRange);
    const hours = preset?.hours ?? 24;
    const now = new Date();
    const from = new Date(now.getTime() - hours * 60 * 60 * 1000);
    return {
      effectiveFrom: from.toISOString(),
      effectiveTo: now.toISOString(),
      effectiveBucket: bucketForHours(hours),
    };
  }, [activeRange, appliedFromDate, appliedToDate]);

  const analyticsQ = useQuery({
    queryKey: ["llm-analytics", activeRange, effectiveFrom, effectiveTo, effectiveBucket],
    queryFn: () =>
      api.getLlmAnalytics({
        range: activeRange === "custom" ? "custom" : activeRange,
        bucket: effectiveBucket,
        from: effectiveFrom || undefined,
        to: effectiveTo || undefined,
      }),
    enabled: activeRange !== "custom" || Boolean(appliedFromDate && appliedToDate),
    refetchInterval: autoRefresh ? (effectiveBucket === "hour" ? 30000 : 120000) : false,
  });

  const handleRangeChange = (range: AnalyticsRange) => {
    if (range === "custom") {
      setCustomFrom(defaultCustomFrom);
      setCustomTo(toLocalDatetimeInput(new Date()));
      setCustomDialogOpen(true);
      return;
    }
    setActiveRange(range);
  };

  const applyCustomRange = () => {
    if (customRangeInvalid) return;
    setAppliedCustomFrom(customFrom);
    setAppliedCustomTo(customTo);
    setActiveRange("custom");
    setCustomDialogOpen(false);
  };
  const policyMetricsQ = useQuery({
    queryKey: ["analytics-policy-metrics"],
    queryFn: () => api.rawGet("/settings/evolution/dev?limit=5000"),
    refetchInterval: autoRefresh ? 120000 : false
  });

  const resp = analyticsQ.data as LlmAnalyticsResponse | undefined;
  const activeError = analyticsQ.error;
  const totals = resp?.totals;
  const policyMetricsPayload = asRecord(policyMetricsQ.data);
  const policyMetricsRows = pickRecords(policyMetricsPayload, "policy_metrics")
    .slice()
    .sort((a, b) => num(b.samples, 0) - num(a.samples, 0))
    .slice(0, 8);
  const byModelRows = (resp?.by_model || []).slice(0, 4);
  const breakdownRows =
    breakdownView === "model"
      ? resp?.by_model || []
      : breakdownView === "channel"
        ? resp?.by_channel || []
        : resp?.by_purpose || [];

  const palette = ["#2fd4ff", "#14f195", "#fbbf24", "#d946ef", "#60a5fa", "#f97316"];
  const seriesNames = byModelRows.map((r) => str(r.model, str(r.provider, "Other")));
  const spendSeries = byModelRows.map((r) => (typeof r.cost_usd === "number" ? r.cost_usd : 0));
  const requestSeries = byModelRows.map((r) => num(r.request_count, 0));
  const tokenSeries = byModelRows.map((r) => num(r.total_tokens, 0));

  function miniBarsOption(values: number[]) {
    return {
      backgroundColor: "transparent",
      animationDuration: 400,
      grid: { left: 2, right: 2, top: 10, bottom: 8, containLabel: false },
      tooltip: {
        trigger: "axis",
        backgroundColor: "rgba(6,14,28,0.95)",
        borderColor: "rgba(84,198,255,0.25)",
        textStyle: { color: "#d8edff" },
        axisPointer: {
          type: "shadow",
          shadowStyle: {
            color: "rgba(84,198,255,0.08)"
          }
        }
      },
      xAxis: {
        type: "category",
        data: seriesNames,
        boundaryGap: true,
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false }
      },
      yAxis: {
        type: "value",
        max: (value: { max: number }) => (value.max > 0 ? value.max * 1.18 : 1),
        splitLine: { show: false },
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false }
      },
      series: [
        {
          type: "bar",
          data: values.map((v, idx) => ({
            value: v,
            itemStyle: {
              color: palette[idx % palette.length],
              borderRadius: [999, 999, 0, 0],
              shadowBlur: 10,
              shadowColor: "rgba(0,0,0,0.22)"
            }
          })),
          showBackground: true,
          backgroundStyle: {
            color: "rgba(108,156,212,0.08)",
            borderRadius: [999, 999, 0, 0]
          },
          barWidth: 14,
          barMaxWidth: 14,
          barMinHeight: 6,
          barCategoryGap: "62%"
        }
      ]
    };
  }

  const spendValue = typeof totals?.cost_usd === "number" ? `$${totals.cost_usd.toFixed(4)}` : "n/a";
  const requestsValue = compactNumber(num(totals?.request_count, 0));
  const tokensValue = compactNumber(num(totals?.total_tokens, 0));
  const policyMetricsOption = {
    backgroundColor: "transparent",
    animationDuration: 400,
    grid: { left: 56, right: 58, top: 24, bottom: 88 },
    legend: {
      bottom: 12,
      textStyle: { color: "#a9c4df" }
    },
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(6,14,28,0.95)",
      borderColor: "rgba(84,198,255,0.25)",
      textStyle: { color: "#d8edff" }
    },
    xAxis: {
      type: "category",
      data: policyMetricsRows.map((row) => str(row.version, "-")),
      axisLabel: {
        color: "#8fb2d1",
        rotate: 16,
        formatter: (value: string) => shortVersionLabel(value)
      },
      axisLine: { lineStyle: { color: "rgba(108,156,212,0.25)" } }
    },
    yAxis: [
      {
        type: "value",
        name: "Success/Error %",
        min: 0,
        max: 100,
        axisLabel: {
          color: "#8fb2d1",
          formatter: "{value}%"
        },
        splitLine: { lineStyle: { color: "rgba(108,156,212,0.12)" } }
      },
      {
        type: "value",
        name: "p95 ms",
        axisLabel: { color: "#8fb2d1" },
        splitLine: { show: false }
      }
    ],
    series: [
      {
        name: "Success",
        type: "bar",
        data: policyMetricsRows.map((row) => Number((num(row.success_rate, 0) * 100).toFixed(1))),
        itemStyle: { color: "#14f195", borderRadius: [6, 6, 0, 0] }
      },
      {
        name: "Errors",
        type: "bar",
        data: policyMetricsRows.map((row) => Number((num(row.error_rate, 0) * 100).toFixed(1))),
        itemStyle: { color: "#ff7b72", borderRadius: [6, 6, 0, 0] }
      },
      {
        name: "p95 latency",
        type: "line",
        yAxisIndex: 1,
        smooth: true,
        symbolSize: 8,
        data: policyMetricsRows.map((row) =>
          row.p95_latency_ms == null ? null : num(row.p95_latency_ms, 0)
        ),
        lineStyle: { color: "#2fd4ff", width: 2.5 },
        itemStyle: { color: "#2fd4ff" }
      }
    ]
  };

  return (
    <Stack spacing={1.5} sx={{ pb: 3 }}>
      <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "stretch", md: "center" }} spacing={1}>
        <Typography variant="h4" sx={{ fontWeight: 700, letterSpacing: -0.6, color: "#ecf5ff" }}>
          Analytics
        </Typography>
        <TextField
          select
          size="small"
          value={activeRange}
          onChange={(e) => {
            const val = e.target.value as AnalyticsRange | "open_custom";
            if (val === "open_custom") {
              handleRangeChange("custom");
            } else {
              handleRangeChange(val);
            }
          }}
          sx={{ minWidth: 180 }}
        >
          <MenuItem disabled sx={{ fontSize: "0.75rem", opacity: 0.6, py: 0.25 }}>Hours</MenuItem>
          <MenuItem value="1h">1 hour</MenuItem>
          <MenuItem value="2h">2 hours</MenuItem>
          <MenuItem value="6h">6 hours</MenuItem>
          <MenuItem value="24h">24 hours</MenuItem>
          <MenuItem disabled sx={{ fontSize: "0.75rem", opacity: 0.6, py: 0.25 }}>Days</MenuItem>
          <MenuItem value="3d">3 days</MenuItem>
          <MenuItem value="7d">7 days</MenuItem>
          <MenuItem value="14d">14 days</MenuItem>
          <MenuItem value="21d">21 days</MenuItem>
          <MenuItem value="30d">30 days</MenuItem>
          <MenuItem value="45d">45 days</MenuItem>
          <MenuItem value="60d">60 days</MenuItem>
          <MenuItem value="90d">90 days</MenuItem>
          <Divider />
          {activeRange === "custom" ? (
            <MenuItem value="custom">Custom ({appliedCustomFrom.replace("T", " ")} — {appliedCustomTo.replace("T", " ")})</MenuItem>
          ) : null}
          <MenuItem value={"open_custom" as string}>Custom range...</MenuItem>
        </TextField>
      </Stack>

      <Dialog open={customDialogOpen} onClose={() => setCustomDialogOpen(false)} maxWidth="xs" fullWidth>
        <DialogTitle>Custom Date Range</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ mt: 1 }}>
            <TextField
              size="small"
              label="From"
              type="datetime-local"
              value={customFrom}
              onChange={(e) => setCustomFrom(e.target.value)}
              InputLabelProps={{ shrink: true }}
              fullWidth
            />
            <TextField
              size="small"
              label="To"
              type="datetime-local"
              value={customTo}
              onChange={(e) => setCustomTo(e.target.value)}
              InputLabelProps={{ shrink: true }}
              fullWidth
              error={customRangeInvalid}
              helperText={customRangeInvalid ? "To must be later than From." : undefined}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCustomDialogOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={applyCustomRange} disabled={customRangeInvalid}>
            Apply
          </Button>
        </DialogActions>
      </Dialog>

      {activeError ? <Alert severity="error">{String(activeError)}</Alert> : null}

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "1fr", lg: "repeat(3, minmax(0, 1fr))" },
          gap: 1.5
        }}
      >
        {[
          { title: "Spend", value: spendValue, values: spendSeries },
          { title: "Requests", value: requestsValue, values: requestSeries },
          { title: "Tokens", value: tokensValue, values: tokenSeries }
        ].map((card) => (
          <Box
            key={card.title}
            className="list-shell"
            sx={{
              p: 1.6,
              borderRadius: "12px",
              border: "1px solid rgba(108,156,212,0.18)",
              background: "linear-gradient(170deg, rgba(6,15,29,0.95), rgba(3,9,21,0.9))"
            }}
          >
            <Typography variant="subtitle1" sx={{ color: "#d8edff", fontWeight: 600 }}>
              {card.title}
            </Typography>
            <Typography variant="h4" sx={{ color: "#f3fbff", fontWeight: 700, mb: 0.6 }}>
              {card.value}
            </Typography>
            <ReactECharts option={miniBarsOption(card.values)} style={{ height: 104 }} />
            <Stack spacing={0.5} sx={{ mt: 0.8 }}>
              {byModelRows.map((row, idx) => (
                <Stack key={`${card.title}-legend-${idx}`} direction="row" justifyContent="space-between" alignItems="center">
                  <Stack direction="row" spacing={0.8} alignItems="center" sx={{ minWidth: 0 }}>
                    <Box sx={{ width: 8, height: 8, borderRadius: "50%", bgcolor: palette[idx % palette.length], flex: "0 0 auto" }} />
                    <Typography variant="body2" noWrap title={str(row.model, str(row.provider, "Other"))}>
                      {str(row.model, str(row.provider, "Other"))}
                    </Typography>
                  </Stack>
                  <Typography variant="body2" color="text.secondary">
                    {card.title === "Spend"
                      ? (typeof row.cost_usd === "number" ? `$${row.cost_usd.toFixed(4)}` : "n/a")
                      : card.title === "Requests"
                        ? compactNumber(num(row.request_count, 0))
                        : compactNumber(num(row.total_tokens, 0))}
                  </Typography>
                </Stack>
              ))}
            </Stack>
          </Box>
        ))}
      </Box>

      {/* ── Usage Over Time ── */}
      {(resp?.series || []).length > 1 ? (
        <Grid2 container spacing={1.5}>
          <Grid2 size={{ xs: 12, lg: 6 }}>
            <Box className="list-shell" sx={{ p: 1.6 }}>
              <Typography variant="subtitle1" sx={{ color: "#e8f4ff", fontWeight: 600, mb: 0.5 }}>Requests Over Time</Typography>
              <ReactECharts style={{ height: 220 }} option={{
                backgroundColor: "transparent",
                animationDuration: 400,
                grid: { left: 48, right: 16, top: 16, bottom: 32 },
                tooltip: { trigger: "axis", backgroundColor: "rgba(6,14,28,0.95)", borderColor: "rgba(84,198,255,0.25)", textStyle: { color: "#d8edff" } },
                xAxis: { type: "category", data: (resp?.series || []).map((p) => p.bucket_start.slice(5, 16).replace("T", " ")), axisLabel: { color: "#8fb2d1", fontSize: 10 }, axisLine: { lineStyle: { color: "rgba(108,156,212,0.25)" } } },
                yAxis: { type: "value", axisLabel: { color: "#8fb2d1" }, splitLine: { lineStyle: { color: "rgba(108,156,212,0.10)" } } },
                series: [{ type: "bar", data: (resp?.series || []).map((p) => p.request_count), itemStyle: { color: "#2fd4ff", borderRadius: [4, 4, 0, 0] }, barMaxWidth: 32 }]
              }} />
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12, lg: 6 }}>
            <Box className="list-shell" sx={{ p: 1.6 }}>
              <Typography variant="subtitle1" sx={{ color: "#e8f4ff", fontWeight: 600, mb: 0.25 }}>Tokens Over Time</Typography>
              <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.75 }}>
                All LLM traffic, split into primary response generation vs helper/classifier passes.
              </Typography>
              <ReactECharts style={{ height: 220 }} option={{
                backgroundColor: "transparent",
                animationDuration: 400,
                grid: { left: 56, right: 16, top: 16, bottom: 32 },
                legend: {
                  top: 0,
                  textStyle: { color: "#9fc3e6", fontSize: 11 }
                },
                tooltip: { trigger: "axis", backgroundColor: "rgba(6,14,28,0.95)", borderColor: "rgba(84,198,255,0.25)", textStyle: { color: "#d8edff" } },
                xAxis: { type: "category", data: (resp?.series || []).map((p) => p.bucket_start.slice(5, 16).replace("T", " ")), axisLabel: { color: "#8fb2d1", fontSize: 10 }, axisLine: { lineStyle: { color: "rgba(108,156,212,0.25)" } } },
                yAxis: { type: "value", axisLabel: { color: "#8fb2d1" }, splitLine: { lineStyle: { color: "rgba(108,156,212,0.10)" } } },
                series: [
                  {
                    type: "line",
                    name: "Primary prompt",
                    data: (resp?.series || []).map((p) => p.primary_prompt_tokens),
                    smooth: true,
                    areaStyle: { opacity: 0.12 },
                    lineStyle: { color: "#14f195", width: 2 },
                    itemStyle: { color: "#14f195" }
                  },
                  {
                    type: "line",
                    name: "Primary completion",
                    data: (resp?.series || []).map((p) => p.primary_completion_tokens),
                    smooth: true,
                    areaStyle: { opacity: 0.12 },
                    lineStyle: { color: "#2fd4ff", width: 2 },
                    itemStyle: { color: "#2fd4ff" }
                  },
                  {
                    type: "line",
                    name: "Helper prompt",
                    data: (resp?.series || []).map((p) => p.helper_prompt_tokens),
                    smooth: true,
                    lineStyle: { color: "#fbbf24", width: 2, type: "dashed" },
                    itemStyle: { color: "#fbbf24" }
                  },
                  {
                    type: "line",
                    name: "Helper completion",
                    data: (resp?.series || []).map((p) => p.helper_completion_tokens),
                    smooth: true,
                    lineStyle: { color: "#c084fc", width: 2, type: "dashed" },
                    itemStyle: { color: "#c084fc" }
                  }
                ]
              }} />
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ p: 1.6 }}>
              <Typography variant="subtitle1" sx={{ color: "#e8f4ff", fontWeight: 600, mb: 0.5 }}>Cost Over Time</Typography>
              <ReactECharts style={{ height: 200 }} option={{
                backgroundColor: "transparent",
                animationDuration: 400,
                grid: { left: 56, right: 16, top: 16, bottom: 32 },
                tooltip: { trigger: "axis", backgroundColor: "rgba(6,14,28,0.95)", borderColor: "rgba(84,198,255,0.25)", textStyle: { color: "#d8edff" }, valueFormatter: (v: number) => `$${v.toFixed(4)}` },
                xAxis: { type: "category", data: (resp?.series || []).map((p) => p.bucket_start.slice(5, 16).replace("T", " ")), axisLabel: { color: "#8fb2d1", fontSize: 10 }, axisLine: { lineStyle: { color: "rgba(108,156,212,0.25)" } } },
                yAxis: { type: "value", axisLabel: { color: "#8fb2d1", formatter: (v: number) => `$${v.toFixed(3)}` }, splitLine: { lineStyle: { color: "rgba(108,156,212,0.10)" } } },
                series: [{ type: "line", data: (resp?.series || []).map((p) => p.cost_usd ?? 0), smooth: true, areaStyle: { opacity: 0.2, color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#fbbf24" }, { offset: 1, color: "rgba(251,191,36,0)" }] } }, lineStyle: { color: "#fbbf24", width: 2.5 }, itemStyle: { color: "#fbbf24" } }]
              }} />
            </Box>
          </Grid2>
        </Grid2>
      ) : null}

      <Box className="list-shell">
        <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }} spacing={1} sx={{ mb: 1 }}>
          <Typography variant="h6">By Model</Typography>
          <Typography variant="caption" color="text.secondary">
            Range: {str(asRecord(resp?.range).since, "-")} to {str(asRecord(resp?.range).until, "-")}
          </Typography>
        </Stack>
        {(resp?.by_model || []).length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No analytics data yet for the selected range.
          </Typography>
        ) : (
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Model</TableCell>
                  <TableCell align="right">Requests</TableCell>
                  <TableCell align="right">Tokens</TableCell>
                  <TableCell align="right">Cost</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {(resp?.by_model || []).slice(0, 30).map((row, idx) => {
                  const label = `${str(row.provider, "-")} / ${str(row.model, "-")}`;
                  return (
                    <TableRow key={`analytics-row-${idx}`}>
                      <TableCell sx={{ maxWidth: 340 }}>
                        <Typography variant="body2" noWrap title={label}>
                          {label}
                        </Typography>
                      </TableCell>
                      <TableCell align="right">{num(row.request_count, 0).toLocaleString()}</TableCell>
                      <TableCell align="right">{num(row.total_tokens, 0).toLocaleString()}</TableCell>
                      <TableCell align="right">
                        {typeof row.cost_usd === "number" ? `$${row.cost_usd.toFixed(4)}` : "n/a"}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </Box>

      <Grid2 container spacing={1.5}>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell" sx={{ p: 1.6 }}>
            <Typography variant="h6" sx={{ color: "#e8f4ff", fontWeight: 600 }}>
              Routing Policy Performance
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1.25 }}>
              This compares the live routing policy versions AgentArk has used. Success and error rates show outcome quality, and p95 shows slower tail latency.
            </Typography>
            {policyMetricsQ.isLoading ? (
              <Typography variant="body2" color="text.secondary">
                Loading policy metrics...
              </Typography>
            ) : policyMetricsQ.error ? (
              <Alert severity="error">{errMessage(policyMetricsQ.error)}</Alert>
            ) : policyMetricsRows.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No routing policy metrics yet.
              </Typography>
            ) : (
              <ReactECharts option={policyMetricsOption} style={{ height: 320 }} />
            )}
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <Box className="list-shell" sx={{ p: 1.6, minHeight: "100%" }}>
            <Typography variant="h6" sx={{ color: "#e8f4ff", fontWeight: 600 }}>
              Policy Metrics
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1.25 }}>
              Current routing policy versions ranked by traffic volume.
            </Typography>
            {policyMetricsQ.isLoading ? (
              <Typography variant="body2" color="text.secondary">
                Loading policy metrics...
              </Typography>
            ) : policyMetricsQ.error ? (
              <Alert severity="error">{errMessage(policyMetricsQ.error)}</Alert>
            ) : policyMetricsRows.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No routing policy metrics yet.
              </Typography>
            ) : (
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Version</TableCell>
                      <TableCell align="right">Samples</TableCell>
                      <TableCell align="right">Success</TableCell>
                      <TableCell align="right">Errors</TableCell>
                      <TableCell align="right">p95 (ms)</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {policyMetricsRows.map((row, idx) => (
                      <TableRow key={`${str(row.version, "policy")}-${idx}`}>
                        <TableCell title={str(row.version, "-")}>{shortVersionLabel(str(row.version, "-"), 34)}</TableCell>
                        <TableCell align="right">{num(row.samples, 0)}</TableCell>
                        <TableCell align="right">{(num(row.success_rate, 0) * 100).toFixed(1)}%</TableCell>
                        <TableCell align="right">{(num(row.error_rate, 0) * 100).toFixed(1)}%</TableCell>
                        <TableCell align="right">{row.p95_latency_ms == null ? "-" : num(row.p95_latency_ms, 0)}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableContainer>
            )}
          </Box>
        </Grid2>
      </Grid2>
    </Stack>
  );
}

export function NativeWorkspace({
  view,
  autoRefresh,
  showAdvanced,
  settingsInitialTab
}: {
  view: WorkspaceView;
  autoRefresh: boolean;
  showAdvanced: boolean;
  settingsInitialTab?: number | null;
}) {
  const isChat = view === "chat";
  return (
    <Box
      sx={{
        p: { xs: 0.5, md: 0.75 },
        height: "100%",
        overflow: isChat ? "hidden" : "auto",
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        minWidth: 0,
        width: "100%"
      }}
    >
      <Box sx={{ display: view === "chat" ? "flex" : "none", flex: 1, minHeight: 0, minWidth: 0, width: "100%" }}>
        <ChatManager autoRefresh={autoRefresh} isActive={view === "chat"} />
      </Box>
      {view === "tasks" ? <TasksManager autoRefresh={autoRefresh} /> : null}
      {view === "skills" ? <SkillsManager autoRefresh={autoRefresh} /> : null}
      {view === "apps" ? <AppsManager autoRefresh={autoRefresh} /> : null}
      {view === "moltbook" ? (
        <SettingsManager autoRefresh={autoRefresh} initialTab={7} hideSettingsNav />
      ) : null}
      {view === "goals" ? <GoalsManager autoRefresh={autoRefresh} /> : null}
      {view === "autonomy" ? <AutonomyManager autoRefresh={autoRefresh} /> : null}
      {view === "documents" ? <DocumentsManager autoRefresh={autoRefresh} /> : null}
      {view === "projects" ? <ProjectsManager autoRefresh={autoRefresh} /> : null}
      {view === "swarm" ? <SwarmManager autoRefresh={autoRefresh} /> : null}
      {view === "trace" ? <TraceManager autoRefresh={autoRefresh} /> : null}
      {view === "status" ? <WatchersManager autoRefresh={autoRefresh} /> : null}
      {view === "analytics" ? <AnalyticsManager autoRefresh={autoRefresh} /> : null}
      {view === "arkpulse" ? (
        <SettingsManager autoRefresh={autoRefresh} initialTab={9} hideSettingsNav />
      ) : null}
      {view === "settings" ? (
        <SettingsManager autoRefresh={autoRefresh} initialTab={settingsInitialTab} />
      ) : null}
      {["tasks", "skills", "apps"].includes(view) ? <Divider sx={{ mt: 2 }} /> : null}
    </Box>
  );
}


