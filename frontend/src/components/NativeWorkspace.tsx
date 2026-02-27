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
import MoreVertIcon from "@mui/icons-material/MoreVert";
import ArrowUpwardRoundedIcon from "@mui/icons-material/ArrowUpwardRounded";
import StopRoundedIcon from "@mui/icons-material/StopRounded";
import CloseIcon from "@mui/icons-material/Close";
import FilterListRoundedIcon from "@mui/icons-material/FilterListRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type MouseEvent, type ReactNode } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../api/client";
import AgentLogo from "../assets/logo.svg";
import { IntegrationsPanel } from "./IntegrationsPanel";
import type { SkillImportResponse, LlmAnalyticsResponse } from "../types";
import { useUiStore } from "../store/uiStore";

const REFRESH_MS = 8000;
const IMPORT_SECURITY_FORCE_RISK_THRESHOLD = 8;
const DEVELOPER_MODE_STORAGE_KEY = "agentark.developer_mode";
const DEVELOPER_MODE_EVENT = "agentark:developer-mode-change";
const OLLAMA_DEFAULT_BASE_URL = "http://localhost:11434";
const OPENROUTER_DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const SHOW_EXPERIMENTAL_AUTONOMY_TOOLS = false;
type ImportRiskBand = "secure" | "review" | "risky";

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
type VaultEditorMode = "add" | "edit";
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
  | "goals"
  | "autonomy"
  | "documents"
  | "memory"
  | "projects"
  | "swarm"
  | "trace"
  | "status"
  | "analytics"
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
  if (providedRiskScore < 0) {
    if (contextualRatio >= 0.75) {
      score *= 0.65;
    } else if (contextualRatio >= 0.5) {
      score *= 0.8;
    }
  }

  const threatLevel = str(security.threat_level, "").toLowerCase();
  if (threatLevel === "malicious") {
    score = Math.max(score, 8.5);
  } else if (threatLevel === "suspicious") {
    score = Math.max(score, 5);
  }
  if (toBool(security.blocked)) {
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

function renderChatMarkdown(text: string): ReactNode {
  const blocks = parseChatMarkdown(text || "");
  if (blocks.length === 0) return null;

  return (
    <Box className="chat-markdown">
      {blocks.map((block, idx) => {
        const key = `${block.type}-${idx}`;
        if (block.type === "heading") {
          return (
            <Typography key={key} className={`chat-md-heading chat-md-h${Math.min(6, Math.max(1, block.level))}`}>
              {renderInlineMarkdown(block.text)}
            </Typography>
          );
        }
        if (block.type === "code") {
          return (
            <Box key={key} className="chat-md-code-wrap">
              {block.language ? <div className="chat-md-code-lang">{block.language}</div> : null}
              <pre className="chat-md-code">
                <code>{block.content}</code>
              </pre>
            </Box>
          );
        }
        if (block.type === "ul" || block.type === "ol") {
          const ListTag = block.type === "ul" ? "ul" : "ol";
          return (
            <Box key={key} component={ListTag} className="chat-md-list">
              {block.items.map((item, itemIdx) => (
                <li key={`${key}-item-${itemIdx}`}>{renderInlineMarkdown(item)}</li>
              ))}
            </Box>
          );
        }
        return (
          <Typography key={key} variant="body2" className="chat-md-paragraph">
            {renderMarkdownLineBreaks(block.text)}
          </Typography>
        );
      })}
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
  if (error instanceof Error) return error.message;
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
  status?: string;
  result?: SkillImportResponse;
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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [force, setForce] = useState(false);
  const [model, setModel] = useState("");

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

  useEffect(() => {
    if (!open) {
      setError(null);
      setLoading(false);
      setImporting(false);
      return;
    }
    setUrlsText("");
    setItems([]);
  }, [open]);

  const parseUrls = () => {
    const uniq = parseUrlsFromText(urlsText);
    setItems(uniq.map((url) => ({ url, selected: true })));
  };

  const handleImportSelected = async () => {
    // If the user never clicked "Preview list", build the list automatically.
    const effectiveItems =
      items.length > 0 ? items : parseUrlsFromText(urlsText).map((url) => ({ url, selected: true } as BulkImportItem));

    if (items.length === 0 && effectiveItems.length > 0) {
      setItems(effectiveItems);
    }

    const toImport = effectiveItems.filter((item) => item.selected);
    if (!toImport.length) return;
    setImporting(true);
    for (const item of toImport) {
      setItems((prev) =>
        prev.map((x) => (x.url === item.url ? { ...x, status: "Importing..." } : x))
      );
      try {
        const result = await api.importSkill({ url: item.url, force, model: model.trim() || undefined });
        let statusMessage = result.message || `Imported ${result.name}`;
        if (result.status === "blocked") {
          statusMessage = result.message || "Blocked by security verification (enable override and retry).";
        } else if (result.status === "needs_secrets") {
          statusMessage = result.message || `Imported ${result.name} (disabled until secrets are configured)`;
        }
        setItems((prev) =>
          prev.map((x) => (x.url === item.url ? { ...x, status: statusMessage, result } : x))
        );
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
          await onImported?.({ result, message: statusMessage });
        }
      } catch (err) {
        const message = `Error: ${errMessage(err)}`;
        setItems((prev) =>
          prev.map((x) => (x.url === item.url ? { ...x, status: message } : x))
        );
      }
    }
    setImporting(false);
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Bulk Import</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1.25}>
          {error ? <Alert severity="error">{error}</Alert> : null}
          <Typography variant="body2" color="text.secondary">
            Paste one or more skill URLs (one per line). Use <code>/tree/</code> for GitHub folders, not <code>/blob/</code>.
          </Typography>
          <Alert severity="info" variant="outlined" sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem" } }}>
            Getting 403 errors? GitHub rate-limits unauthenticated requests. Go to Settings &gt; Integrations &gt; GitHub and add a Personal Access Token for higher limits.
          </Alert>
          <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
            {`Examples:
https://github.com/org/repo/tree/main/skills
https://raw.githubusercontent.com/org/repo/main/skills/my-skill/SKILL.md`}
          </Typography>
          <TextField
            fullWidth
            multiline
            minRows={3}
            maxRows={8}
            label="Import URLs"
            value={urlsText}
            onChange={(e) => setUrlsText(e.target.value)}
            placeholder={"https://github.com/openclaw/skills/tree/main/skills"}
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
          {items.length > 0 ? (
            <Stack spacing={0.5}>
              {items.map((it) => (
                <Box key={it.url} className="console-line">
                  <Typography variant="body2" noWrap title={it.url}>
                    {it.url}
                  </Typography>
                  <Typography variant="caption" color={it.status?.startsWith("Error") ? "error" : "text.secondary"}>
                    {it.status || "Pending"}
                  </Typography>
                </Box>
              ))}
            </Stack>
          ) : null}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
        <Button
          variant="contained"
          disabled={importing || loading || !urlsText.trim()}
          onClick={handleImportSelected}
        >
          {importing ? "Importing..." : "Import"}
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
            Supports direct SKILL.md links and GitHub folder/repo URLs.
          </Typography>
          <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>
            {`Examples:
1. https://github.com/org/repo/tree/main/skills/market-analysis
2. https://raw.githubusercontent.com/org/repo/main/skills/market-analysis/SKILL.md`}
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
          {importRequiresForce ? (
            <Alert severity="warning">
              Risk score {importRisk.score10.toFixed(1)}/10 is in the risky range (threshold {IMPORT_SECURITY_FORCE_RISK_THRESHOLD}/10). Enable override to continue.
            </Alert>
          ) : null}
          {importResult?.security?.warnings?.length ? (
            <Alert severity={importResult.security.blocked ? "warning" : "info"}>
              {importResult.security.warnings.join("\n")}
            </Alert>
          ) : null}
          {importResult?.security ? (
            <Box sx={{ mt: 1 }}>
              <Typography variant="subtitle2" mb={1}>
                Security scan
              </Typography>
              <Stack spacing={1}>
                <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                  <Chip
                    size="small"
                    color={importRisk.chipColor}
                    label={`Risk ${importRisk.score10.toFixed(1)}/10`}
                  />
                  <Chip size="small" variant="outlined" color={importRisk.chipColor} label={importRisk.bandLabel} />
                  <Typography variant="caption" color="text.secondary">
                    Scale: &lt;5 secure, 5-8 needs review, 8-10 risky
                  </Typography>
                </Stack>
                <Typography variant="body2" color="text.secondary">
                  Threat level: {str(importResult.security.threat_level, "-")} | Blocked: {boolText(importResult.security.blocked)} | Raw severity: {importRisk.rawSeverity}
                </Typography>
                {importRisk.totalFindings > 0 ? (
                  <Typography variant="caption" color="text.secondary">
                    Signals: {importRisk.totalFindings} total, {importRisk.contextualFindings} contextual (common for integration templates: API key refs, env lookups, curl/wget).
                  </Typography>
                ) : null}
                {Array.isArray(importResult.security.findings) && importResult.security.findings.length > 0 ? (
                  <TableContainer className="table-shell">
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Severity</TableCell>
                          <TableCell>Category</TableCell>
                          <TableCell>Line</TableCell>
                          <TableCell>Description</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {(importResult.security.findings as unknown[]).slice(0, 30).map((rawFinding, idx) => {
                          const f = asRecord(rawFinding);
                          return (
                            <TableRow key={`${idx}-${str(f.category, "")}`}>
                              <TableCell sx={{ whiteSpace: "nowrap" }}>{str(f.severity, "-")}</TableCell>
                              <TableCell sx={{ whiteSpace: "nowrap" }}>{str(f.category, "-")}</TableCell>
                              <TableCell sx={{ whiteSpace: "nowrap" }}>{num(f.line, -1) >= 0 ? num(f.line) : "-"}</TableCell>
                              <TableCell>
                                <Typography variant="body2">{str(f.description, "-")}</Typography>
                                {str(f.matched_text, "").trim() ? (
                                  <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
                                    Match: {str(f.matched_text).slice(0, 120)}
                                  </Typography>
                                ) : null}
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                ) : (
                  <Typography variant="body2" color="text.secondary">
                    No findings.
                  </Typography>
                )}
              </Stack>
            </Box>
          ) : null}
          {Array.isArray(importResult?.imported) && importResult.imported.length > 0 ? (
            <Box sx={{ mt: 1 }}>
              <Typography variant="subtitle2" mb={1}>
                Per-skill security
              </Typography>
              <TableContainer className="table-shell">
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Skill</TableCell>
                          <TableCell>Status</TableCell>
                          <TableCell>Risk</TableCell>
                          <TableCell>Threat</TableCell>
                          <TableCell>Blocked</TableCell>
                          <TableCell>Warnings</TableCell>
                          <TableCell>Findings</TableCell>
                        </TableRow>
                  </TableHead>
                  <TableBody>
                    {importResult.imported.map((entry, idx) => {
                      const child = entry?.result;
                      const sec = child?.security;
                      const warningsCount = Array.isArray(sec?.warnings) ? sec?.warnings.length : 0;
                      const findingsCount = Array.isArray(sec?.findings) ? sec?.findings.length : 0;
                      const childRisk = computeImportRiskSummary(sec);
                      return (
                        <TableRow key={`${entry?.url || child?.name || idx}-${idx}`}>
                          <TableCell sx={{ wordBreak: "break-word" }}>{child?.name || "-"}</TableCell>
                          <TableCell>{child?.status || "-"}</TableCell>
                          <TableCell sx={{ whiteSpace: "nowrap" }}>
                            <Chip
                              size="small"
                              color={childRisk.chipColor}
                              label={`${childRisk.score10.toFixed(1)}/10`}
                            />
                          </TableCell>
                          <TableCell>{str(sec?.threat_level, "-")}</TableCell>
                          <TableCell>{boolText(sec?.blocked)}</TableCell>
                          <TableCell>{warningsCount}</TableCell>
                          <TableCell>{findingsCount}</TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
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
                <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>
                  Credentials can be edited now, but are saved only after Import Template completes.
                </Typography>
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
                  {savingSecrets ? "Saving..." : "Save secrets"}
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

function ChatManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [draftProjectId, setDraftProjectId] = useState("");
  const [prompt, setPrompt] = useState("");
  const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatNotice, setChatNotice] = useState<string | null>(null);
  const [isStreaming, setIsStreaming] = useState(false);
  const [pendingUserMessage, setPendingUserMessage] = useState<string | null>(null);
  const [failedUserMessage, setFailedUserMessage] = useState<string | null>(null);
  const [streamingResponse, setStreamingResponse] = useState("");
  const [streamingSteps, setStreamingSteps] = useState<JsonRecord[]>([]);
  const [streamTraceOpen, setStreamTraceOpen] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
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
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const dragDepthRef = useRef(0);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const streamLockRef = useRef(false);
  const recentSendRef = useRef<{ fingerprint: string; at: number } | null>(null);
  const streamingStepsRef = useRef<JsonRecord[]>([]);
  const workspaceActivityRef = useRef<HTMLDivElement | null>(null);

  const convQ = useQuery({
    queryKey: ["chat-conversations"],
    queryFn: () => api.rawGet("/conversations?limit=30"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const projectsQ = useQuery({
    queryKey: ["chat-projects"],
    queryFn: () => api.rawGet("/projects"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const messagesQ = useQuery({
    queryKey: ["chat-messages", conversationId],
    queryFn: () => api.rawGet(`/conversations/${encodeURIComponent(conversationId || "")}/messages?limit=100`),
    enabled: !!conversationId,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const conversations = pickRecords(convQ.data, "conversations");
  const projects = pickRecords(projectsQ.data, "projects");
  const messages = conversationId ? pickRecords(messagesQ.data, "messages") : [];
  const selectedConversation = useMemo(
    () => conversations.find((conv) => str(conv.id, "") === conversationId) ?? null,
    [conversations, conversationId]
  );
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
    if (/proof id:/i.test(text)) return "Saved an execution proof.";
    if (/running in sandboxed environment/i.test(text)) return "Running this action in a safe workspace.";
    if (/install(ing)? dependencies|npm install|pnpm install|yarn install|cargo fetch/i.test(text)) {
      return "Installing dependencies.";
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
      if (detail && detail.length > 2) {
        return {
          label: detail,
          detail: "",
          kind: "Done",
          tone: "tone-success"
        };
      }
      return {
        label: `${toHumanToolName(rawName)} completed`,
        detail: "",
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

  const buildStepCard = (step: JsonRecord, index: number) => {
    const stepType = str(step.step_type, str(step.type, "step")).toLowerCase();
    const title = str(step.title, "").trim();
    const fullDetail = extractStepDetailText(step, 2800);
    const rawDetail = fullDetail.slice(0, 900);
    const human = humanizeStep(title, rawDetail, stepType);
    const humanDetailRaw = str(human.detail, "").trim();
    let detail = humanDetailRaw ? simplifyConsoleDetail(humanDetailRaw) : "";
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
    } else if (stepType.includes("tool_result") || stepType.includes("result") || stepType.includes("complete")) {
      tone = "tone-success";
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
    if (human.tone) tone = human.tone;
    if (human.kind) kind = human.kind;
    let detailFull = humanDetailRaw ? (fullDetail || rawDetail) : "";
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
    return {
      id: `${time || "live"}-${index}-${label}`,
      index,
      tone,
      kind,
      label,
      detail,
      detailFull,
      time
    };
  };

  const streamingTraceCards = useMemo(
    () => streamingSteps.map((step, idx) => buildStepCard(step, idx)).slice(-24),
    [streamingSteps]
  );
  const streamingActivity = useMemo(() => {
    if (streamingTraceCards.length === 0) return "Getting ready to start...";
    const last = streamingTraceCards[streamingTraceCards.length - 1];
    const detail = (last.detail || "").trim();
    if (detail) return `${last.label}: ${detail}`;
    return `Now: ${last.label}`;
  }, [streamingTraceCards]);

  const traceSummaryText = (
    cards: Array<ReturnType<typeof buildStepCard>>,
    opts?: { loading?: boolean; streaming?: boolean; error?: string }
  ) => {
    if (opts?.error) return "Activity details unavailable.";
    if (opts?.loading) return "Loading activity...";
    if (cards.length === 0) return opts?.streaming ? "Waiting for first activity update..." : "No activity yet.";
    const last = cards[cards.length - 1];
    return `${cards.length} update${cards.length === 1 ? "" : "s"} • Now: ${last.label}`;
  };

  const parseTraceSteps = (payload: unknown): JsonRecord[] => {
    const rec = asRecord(payload);
    const raw = Array.isArray(rec.steps) ? rec.steps : Array.isArray(rec.trace) ? rec.trace : [];
    return raw.filter((x) => x && typeof x === "object") as JsonRecord[];
  };

  const loadTraceForId = async (traceId: string) => {
    if (!traceId) return;
    if (traceStepsById[traceId] || traceLoadingById[traceId]) return;
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
    dragDepthRef.current = 0;
    setIsDragOverChat(false);
    setConversationId(null);
    setDraftProjectId("");
    setPrompt("");
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
    if (!id || isStreaming) return;
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
      const blob = new Blob([lines.join("\n")], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${safe}-${stamp}.txt`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      setChatNotice("Chat exported.");
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
      const incomingHeartbeat = isHeartbeatStreamingStep(step);
      let next: JsonRecord[];
      if (incomingHeartbeat) {
        const existingIndex = prev.findIndex((row) => isHeartbeatStreamingStep(row));
        if (existingIndex >= 0) {
          next = [...prev];
          next[existingIndex] = step;
        } else {
          next = [...prev, step];
        }
      } else {
        next = [...prev.filter((row) => !isHeartbeatStreamingStep(row))];
        const incomingKey = streamingStepDedupKey(step);
        const incomingDisplayKey = streamingStepDisplayKey(step);
        const lastIdx = next.length - 1;
        if (
          lastIdx >= 0 &&
          (streamingStepDedupKey(next[lastIdx]) === incomingKey ||
            (incomingDisplayKey &&
              incomingDisplayKey !== "|" &&
              streamingStepDisplayKey(next[lastIdx]) === incomingDisplayKey))
        ) {
          next[lastIdx] = step;
        } else {
          next.push(step);
        }
      }
      if (next.length > 32) next.splice(0, next.length - 32);
      streamingStepsRef.current = next;
      return next;
    });
  };

  const runStreamingChat = async (
    message: string,
    files: File[] = [],
    opts?: { sensitive?: boolean }
  ): Promise<boolean> => {
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
    const fingerprint = `${conversationId || "__new__"}::${activeProjectId || "__no_project__"}::${activeMessage
      .toLowerCase()
      .replace(/\s+/g, " ")
      .trim()}`;
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
    setLiveFileWrites({});
    setDeployedFiles([]);
    setCodeViewerFileIdx(0);
    setStreamTraceOpen(false);
    setIsStreaming(true);

    const requestedConversationId = conversationId || generateConversationId();
    let resolvedConversationId = conversationId || "";
    let payloadMessage = activeMessage;
    let streamError: string | null = null;
    const absorbConversationId = (payload: unknown) => {
      const obj = asRecord(payload);
      const cid = str(obj.conversation_id, str(obj.cid, str(obj.conversationId, "")));
      if (cid) resolvedConversationId = cid;
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
          conversation_id: requestedConversationId,
          project_id: activeProjectId || undefined
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
              if (files && typeof files === "object") {
                const captured = Object.entries(files)
                  .filter(([, v]) => typeof v === "string")
                  .map(([k, v]) => ({ name: k, content: v as string }));
                if (captured.length > 0) {
                  setDeployedFiles(captured);
                  setLiveFileWrites((prev) => {
                    const next = { ...prev };
                    for (const file of captured) {
                      if (!next[file.name]) {
                        const totalLines =
                          file.content.length > 0
                            ? file.content.split(/\r?\n/).length
                            : 0;
                        next[file.name] = {
                          content: "",
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
            }
          },
          onToolResult: (name, content, payload) => {
            const preview = content.trim().slice(0, 1600);
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
              detail: preview,
              data: payload || preview
            });
          },
          onToolProgress: (name, content, payload) => {
            const preview = content.trim().slice(0, 1600);
            const payloadObj = asRecord(payload);
            if (name === "app_deploy" && str(payloadObj.kind, "") === "file_write") {
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
              detail: preview,
              data: Object.keys(payloadObj).length > 0 ? payloadObj : preview
            });
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
      if (!streamError) {
        setFailedUserMessage(null);
        const candidateConversationId = resolvedConversationId || requestedConversationId;
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
        setLastRunSteps(streamingStepsRef.current.slice(-64));
      }
      setPendingUserMessage(null);
      setIsStreaming(false);
      setStreamingSteps([]);
      streamingStepsRef.current = [];
      setStreamingResponse("");
      streamLockRef.current = false;
    }
    return !streamError;
  };

  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    thread.scrollTop = thread.scrollHeight;
  }, [messages.length, pendingUserMessage, streamingResponse, isStreaming]);

  useEffect(() => {
    if (!pendingUserMessage) return;
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
  }, [messages, pendingUserMessage]);

  useEffect(() => {
    if (!chatNotice) return;
    const timer = window.setTimeout(() => setChatNotice(null), 2200);
    return () => window.clearTimeout(timer);
  }, [chatNotice]);

  const hasLiveThreadActivity = Boolean(pendingUserMessage || isStreaming || streamingResponse.trim());
  const hasRenderableThread = messages.length > 0 || hasLiveThreadActivity;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const latestAssistantMessageText = str(
    [...messages].reverse().find((m) => str(m.role, "").toLowerCase() === "assistant")?.content,
    ""
  );

  const workspaceSteps = isStreaming && streamingSteps.length > 0 ? streamingSteps : lastRunSteps;
  const workspaceCards = useMemo(
    () => workspaceSteps.map((step, idx) => buildStepCard(step, idx)).slice(-48),
    [workspaceSteps]
  );
  const latestWorkspaceCard = workspaceCards.length > 0 ? workspaceCards[workspaceCards.length - 1] : null;
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
  const progressSummary = progressRows.length
    ? `${progressDoneCount}/${progressRows.length} steps complete`
    : "No steps yet";

  const codeFromCards = (() => {
    for (let i = workspaceCards.length - 1; i >= 0; i -= 1) {
      const detail = (workspaceCards[i]?.detail || "").trim();
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
    workspaceTunnelBaseUrl && workspaceTunnelBaseUrl !== origin
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
  const runState = apiKeyActionNeeded
    ? ("waiting_input" as const)
    : isStreaming
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
    if (isStreaming) {
      const active = latestRunningCard || latestWorkspaceCard;
      return {
        line1: "Status: Running",
        line2: active?.detail || "Agent is actively running actions.",
        tone: "info"
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
  }, [apiKeyActionNeeded, isStreaming, latestRunningCard, latestWorkspaceCard]);
  const nowDoingLabel = useMemo(() => {
    if (apiKeyActionNeeded) return "Waiting for your approval/input";
    const active = latestRunningCard || latestWorkspaceCard;
    return active?.label || "Waiting for next step";
  }, [apiKeyActionNeeded, latestRunningCard, latestWorkspaceCard]);

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
        height: "100%",
        minHeight: 0,
        display: "grid",
        gridTemplateColumns: {
          xs: "1fr",
          md: "228px minmax(0,1fr)",
          lg: showWorkspacePanel
            ? "228px minmax(0,1fr) clamp(300px, 24vw, 340px)"
            : "228px minmax(0,1fr)"
        },
        gap: 1
      }}
    >
      <Box className="list-shell chat-sidebar" sx={{ minHeight: 0, display: "flex", flexDirection: "column" }}>
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Typography variant="h6">Conversations</Typography>
          <Button size="small" onClick={startNewConversation} disabled={isStreaming}>
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
                      <div className="conversation-card-title" title={title}>
                        {title}
                      </div>
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

      <Box
        className={`list-shell chat-shell chat-density-immersive${isDragOverChat ? " chat-shell-drop-active" : ""}`}
        sx={{ minHeight: 0, display: "flex", flexDirection: "column", position: "relative" }}
        onDragEnter={handleChatDragEnter}
        onDragOver={handleChatDragOver}
        onDragLeave={handleChatDragLeave}
        onDrop={handleChatDrop}
      >
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Stack direction="row" spacing={1} alignItems="center">
            <Avatar src={AgentLogo} variant="rounded" sx={{ width: 28, height: 28, bgcolor: "rgba(12,22,40,0.85)" }} />
            <Typography variant="h6">Chat</Typography>
          </Stack>
          <Stack direction="row" spacing={1} alignItems="center">
            <Tooltip title={showWorkspacePanel ? "Hide agent activity" : "Show agent activity"}>
              <span
                className={`activity-toggle-pill${showWorkspacePanel ? " active" : ""}${isStreaming ? " streaming" : ""}`}
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
        <Stack direction={{ xs: "column", md: "row" }} spacing={1} sx={{ mb: 1 }}>
          <TextField
            fullWidth
            size="small"
            select
            label="Project"
            value={selectedConversationProjectId || draftProjectId}
            onChange={(e) => setDraftProjectId(e.target.value)}
            disabled={Boolean(selectedConversation)}
            helperText={
              selectedConversation
                ? "Project is fixed for this conversation."
                : "Optional. Leave as No project for general chat."
            }
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
          className="chat-thread chat-thread-immersive"
        >
          {conversationId == null && !hasRenderableThread ? (
            <Typography variant="body2" color="text.secondary">
              Start with a message to open a new conversation.
            </Typography>
          ) : !hasRenderableThread ? (
            <Typography variant="body2" color="text.secondary">
              No messages in this conversation yet.
            </Typography>
          ) : (
            <Stack spacing={1.2}>
              {messages.map((message, idx) => {
                const role = str(message.role, "").toLowerCase();
                const isUser = role === "user";
                const messageId = str(message.id, String(idx));
                const ts = str(message.timestamp, "");
                const content = str(message.content);
                const renderedContent = isUser ? stripAttachmentContextMarker(content) : content;
                const traceId = str(message.trace_id, "").trim();
                const hasTrace = !isUser && !!traceId;
                const traceLoading = hasTrace ? Boolean(traceLoadingById[traceId]) : false;
                const traceError = hasTrace ? str(traceErrorById[traceId], "").trim() : "";
                const rawTraceSteps = hasTrace ? traceStepsById[traceId] || [] : [];
                const traceCards = rawTraceSteps.map((step, sIdx) => buildStepCard(step, sIdx)).slice(-24);
                const traceExpanded = Boolean(messageTraceOpen[messageId]);
                const traceSummary = traceSummaryText(traceCards, { loading: traceLoading, error: traceError });
                return (
                  <Box key={messageId} className={isUser ? "chat-row chat-row-user" : "chat-row"}>
                    {!isUser ? (
                      <Avatar
                        src={AgentLogo}
                        variant="rounded"
                        className="chat-avatar"
                        sx={{ width: 30, height: 30, bgcolor: "rgba(12,22,40,0.85)" }}
                      />
                    ) : null}
                    <Box className={isUser ? "chat-bubble chat-bubble-user" : "chat-bubble chat-bubble-assistant"}>
                      <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={0.5}>
                        <Typography variant="caption" color="text.secondary">
                          {isUser ? "You" : "AgentArk"}{ts ? ` | ${ts}` : ""}
                        </Typography>
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
                      {hasTrace ? (
                        <Box className="chat-inline-trace">
                          <Button
                            size="small"
                            className="chat-inline-trace-toggle"
                            onClick={() => {
                              const nextExpanded = !Boolean(messageTraceOpen[messageId]);
                              setMessageTraceOpen((prev) => ({ ...prev, [messageId]: nextExpanded }));
                              if (nextExpanded && traceId) {
                                void loadTraceForId(traceId);
                              }
                            }}
                            endIcon={
                              <ArrowDropDownRoundedIcon
                                sx={{
                                  transform: traceExpanded ? "rotate(180deg)" : "rotate(0deg)",
                                  transition: "transform 160ms ease"
                                }}
                              />
                            }
                          >
                            <span className="chat-inline-trace-summary">{traceSummary}</span>
                          </Button>
                          {traceExpanded ? (
                            <Box className="chat-inline-trace-expanded">
                              {traceError ? (
                                <Typography variant="caption" color="error.main">
                                  {traceError}
                                </Typography>
                              ) : traceCards.length === 0 ? (
                                <Typography variant="caption" color="text.secondary">
                                  {traceLoading ? "Loading activity..." : "No activity updates captured for this message."}
                                </Typography>
                              ) : (
                                <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                                  {traceCards.map((step) => (
                                    <Box key={step.id} className={`chat-inline-step ${step.tone}`} title={step.detail || step.label}>
                                      <span className="chat-inline-step-kind">{step.kind}</span>
                                      <span className="chat-inline-step-label">{step.label}</span>
                                    </Box>
                                  ))}
                                </Stack>
                              )}
                            </Box>
                          ) : null}
                        </Box>
                      ) : null}
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
                        U
                      </Avatar>
                    ) : null}
                  </Box>
                );
              })}

              {pendingUserMessage && isStreaming ? (
                <Box className="chat-row chat-row-user">
                  <Box className="chat-bubble chat-bubble-user">
                    <Typography variant="caption" color="text.secondary">
                      You | sending...
                    </Typography>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {pendingUserMessage}
                    </Typography>
                  </Box>
                  <Avatar className="chat-avatar chat-avatar-user" sx={{ width: 30, height: 30, bgcolor: "rgba(47,212,255,0.18)" }}>
                    U
                  </Avatar>
                </Box>
              ) : null}

              {failedUserMessage && !isStreaming ? (
                <Box className="chat-row chat-row-user">
                  <Box className="chat-bubble chat-bubble-user">
                    <Typography variant="caption" color="warning.main">
                      You | not sent
                    </Typography>
                    <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                      {failedUserMessage}
                    </Typography>
                  </Box>
                  <Avatar className="chat-avatar chat-avatar-user" sx={{ width: 30, height: 30, bgcolor: "rgba(47,212,255,0.18)" }}>
                    U
                  </Avatar>
                </Box>
              ) : null}

              {isStreaming ? (
                <Box className="chat-row">
                  <Avatar
                    src={AgentLogo}
                    variant="rounded"
                    className="chat-avatar"
                    sx={{ width: 30, height: 30, bgcolor: "rgba(12,22,40,0.85)" }}
                  />
                  <Box className="chat-bubble chat-bubble-assistant chat-bubble-streaming">
                    <Box className="chat-inline-trace">
                      <Button
                        size="small"
                        className="chat-inline-trace-toggle"
                        onClick={() => setStreamTraceOpen((prev) => !prev)}
                        endIcon={
                          <ArrowDropDownRoundedIcon
                            sx={{
                              transform: streamTraceOpen ? "rotate(180deg)" : "rotate(0deg)",
                              transition: "transform 160ms ease"
                            }}
                          />
                        }
                      >
                        <span className="chat-inline-trace-summary">
                          {traceSummaryText(streamingTraceCards, { streaming: true })}
                        </span>
                      </Button>
                      {streamTraceOpen ? (
                        <Box className="chat-inline-trace-expanded">
                          {streamingTraceCards.length === 0 ? (
                            <Typography variant="caption" color="text.secondary">
                              Waiting for first execution step...
                            </Typography>
                          ) : (
                            <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                              {streamingTraceCards.map((step) => (
                                <Box key={step.id} className={`chat-inline-step ${step.tone}`} title={step.detail || step.label}>
                                  <span className="chat-inline-step-kind">{step.kind}</span>
                                  <span className="chat-inline-step-label">{step.label}</span>
                                </Box>
                              ))}
                            </Stack>
                          )}
                        </Box>
                      ) : null}
                    </Box>
                    <Typography variant="caption" color="text.secondary">
                      {streamingResponse.trim() ? "AgentArk is streaming..." : "AgentArk is thinking..."}
                    </Typography>
                    <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.25 }}>
                      {streamingActivity}
                    </Typography>
                    {streamingResponse.trim() ? (
                      <>
                        {renderChatMarkdown(streamingResponse)}
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
                void runStreamingChat(msg, attachedFiles);
              }
            }}
            rows={1}
            disabled={false}
          />
          <div className="chat-composer-actions">
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
                  await runStreamingChat(msg, attachedFiles);
                }}
              >
                <ArrowUpwardRoundedIcon fontSize="small" />
              </IconButton>
            )}
          </div>
        </Box>
      </Box>

      {showWorkspacePanel ? (
        <Box
          className="list-shell chat-workspace-shell"
          sx={{ minHeight: 0, display: { xs: "none", lg: "flex" }, flexDirection: "column", p: 1 }}
        >
          <Box className="activity-status-bar">
            <span className={`activity-status-dot${isStreaming ? " running" : " idle"}`} />
            <span className="activity-status-text">
              {isStreaming ? workspaceStatusCopy.line1 || "Processing..." : runStateLabel === "STOPPED" ? "Waiting for activity" : workspaceStatusCopy.line1 || runStateLabel}
            </span>
            <span className="activity-step-count">{workspaceCards.length} step{workspaceCards.length === 1 ? "" : "s"}</span>
          </Box>

          <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }} className="chat-workspace-sections">
              <Box className="term-shell">
                <Box className="term-titlebar">
                  <Typography variant="caption" className="term-titlebar-text">
                    agentark — console
                  </Typography>
                  <Box sx={{ flex: 1 }} />
                  <Typography variant="caption" className="term-titlebar-stats">
                    {progressSummary} | {workspaceCards.length} event{workspaceCards.length === 1 ? "" : "s"}
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
                        <span className="term-text term-dim">Waiting for agent activity...</span>
                      </Box>
                    ) : (
                      workspaceCards.map((row, idx) => {
                        const dot = row.kind === "Done" ? "term-dot-ok" : row.kind === "Issue" ? "term-dot-err" : row.kind === "Running" || row.kind === "Planning" ? "term-dot-run" : "term-dot-info";
                        const isLast = idx === workspaceCards.length - 1;
                        return (
                          <Box key={`activity-${row.id}`} className={`term-line${isLast && isStreaming ? " term-line-latest" : ""}`}>
                            <span className={`term-dot ${dot}`} />
                            <Box className="term-content">
                              <span className={`term-label${isLast && isStreaming ? " term-typing" : ""}`}>{row.label}</span>
                              {row.detailFull ? (
                                <span className="term-detail-full">{row.detailFull.slice(0, 300)}</span>
                              ) : row.detail ? (
                                <span className="term-detail-full">{row.detail}</span>
                              ) : null}
                            </Box>
                          </Box>
                        );
                      })
                    )}
                    {isStreaming && (
                      <Box className="term-line">
                        <span className="term-cursor">_</span>
                      </Box>
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

            {previewUrl ? (
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

  function statusLabel(raw: string): string {
    const s = (raw || "").toLowerCase();
    if (s.includes("awaitingapproval")) return "Needs approval";
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
      setQuickIntent("");
      setFormError(null);
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });

  const tasks = pickRecords(tasksQ.data, "tasks");
  const counts = useMemo(() => {
    const by = { total: tasks.length, queued: 0, running: 0, needs_approval: 0, done: 0 };
    for (const t of tasks) {
      const s = str(t.status, "").toLowerCase();
      if (s.includes("awaitingapproval")) by.needs_approval += 1;
      else if (s.includes("inprogress")) by.running += 1;
      else if (s.includes("pending")) by.queued += 1;
      else if (s.includes("completed")) by.done += 1;
    }
    return by;
  }, [tasks]);

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Typography variant="h6">Tasks</Typography>
        <Typography variant="body2" color="text.secondary">
          Describe what you want in plain English. AgentArk can generate a runnable task for you.
        </Typography>
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
      </Grid2>

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
                    setDescription("");
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
                    <TableCell sx={{ whiteSpace: "nowrap" }}>{str(task.created_at)}</TableCell>
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
              Created: {str(selectedTask?.created_at, "-")}
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
  const systemSkills = skills.filter((a) => str(a.source).toLowerCase() === "system");
  const bundledSkills = skills.filter((a) => str(a.source).toLowerCase() === "bundled");
  const customSkills = skills.filter((a) => str(a.source).toLowerCase() === "custom");
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
    setEditTargetName(name);
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
      const out = (await api.rawGet(`/skills/${encodeURIComponent(name)}`)) as JsonRecord;
      const content = str(out.content, "");
      const parsed = parseSkillEditorForm(content, name);
      setEditContent(content);
      setEditForm({ ...parsed, name });
    } catch (err) {
      setEditError(errMessage(err));
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
        <Typography variant="caption" color="text.secondary" sx={{ mt: 1 }}>
          These are pre-built skills. You can always chat with the agent to build anything custom on your own.
        </Typography>
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
              disabled={wizardStepBlocked}
            >
              {createWizardStep < 2 ? "Next" : "Save"}
            </Button>
          ) : (
            <Button
              variant="contained"
              onClick={saveEditor}
              disabled={
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
  const [tunnelActionError, setTunnelActionError] = useState<string | null>(null);
  const [tunnelActionState, setTunnelActionState] = useState<"idle" | "starting" | "stopping">("idle");

  const opMutation = useMutation({
    mutationFn: ({ path, method }: { path: string; method: "POST" | "DELETE" }) => (method === "DELETE" ? api.rawDelete(path) : api.rawPost(path, {})),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    }
  });
  const tunnelStartMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/start", {}),
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
  const startTunnel = async () => {
    setTunnelActionError(null);
    setTunnelActionState("starting");
    try {
      await tunnelStartMutation.mutateAsync();
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
        <Typography variant="h6" mb={1}>Deployed Apps</Typography>
        {tunnelQ.error ? <Alert severity="error" sx={{ mb: 1 }}>{errMessage(tunnelQ.error)}</Alert> : null}
        {tunnelErrorText ? <Alert severity="error" sx={{ mb: 1 }}>{tunnelErrorText}</Alert> : null}
        {tunnelActionError ? <Alert severity="error" sx={{ mb: 1 }}>{tunnelActionError}</Alert> : null}
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
                  const localUrl = toAbsoluteAppUrl(url, origin);
                  const localAccessUrl = toAbsoluteAppUrl(accessUrl || url, origin);
                  const publicUrl = tunnelBaseUrl ? toAbsoluteAppUrl(url, tunnelBaseUrl) : "";
                  const publicAccessUrl = tunnelBaseUrl ? toAbsoluteAppUrl(accessUrl || url, tunnelBaseUrl) : "";
                  const hasProtectedVariant = !!accessUrl && localAccessUrl !== localUrl;
                  const publicShareUrl = publicAccessUrl || publicUrl;
                  const localShareUrl = localAccessUrl || localUrl;
                  const shareUrl = publicShareUrl || localShareUrl;
                  const openTargets = dedupeLinkTargets([
                    { label: "Open Local", url: localUrl },
                    { label: "Open Local (Key)", url: hasProtectedVariant ? localAccessUrl : "" },
                    { label: "Open Public", url: publicUrl },
                    { label: "Open Public (Key)", url: hasProtectedVariant ? publicAccessUrl : "" }
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
                            <Typography variant="caption" component="div" noWrap title={localAccessUrl}>
                              Local (Key):{" "}
                              <Link href={localAccessUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                                {localAccessUrl}
                              </Link>
                            </Typography>
                          ) : null}
                          {publicUrl ? (
                            <Typography variant="caption" component="div" color="info.main" noWrap title={publicUrl}>
                              Public:{" "}
                              <Link href={publicUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                                {publicUrl}
                              </Link>
                            </Typography>
                          ) : tunnelStarting ? (
                            <Typography variant="caption" component="div" color="info.main">
                              Public: starting tunnel...
                            </Typography>
                          ) : tunnelStopping ? (
                            <Typography variant="caption" component="div" color="text.secondary">
                              Public: stopping tunnel...
                            </Typography>
                          ) : (
                            <Typography variant="caption" component="div" color="text.secondary">
                              Public: tunnel inactive
                            </Typography>
                          )}
                          {hasProtectedVariant && publicAccessUrl && publicAccessUrl !== publicUrl ? (
                            <Typography variant="caption" component="div" color="info.main" noWrap title={publicAccessUrl}>
                              Public (Key):{" "}
                              <Link href={publicAccessUrl} target="_blank" rel="noopener noreferrer" underline="hover">
                                {publicAccessUrl}
                              </Link>
                            </Typography>
                          ) : null}
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
                            {
                              label: tunnelStarting ? "Starting Public Tunnel..." : "Start Public Tunnel",
                              divider: true,
                              disabled: tunnelStarting || tunnelActive || !tunnelAvailable,
                              onClick: startTunnel
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
                              label: "Stop",
                              divider: true,
                              onClick: () => opMutation.mutate({ path: `/api/apps/${encodeURIComponent(id)}/stop`, method: "POST" })
                            },
                            {
                              label: "Restart",
                              onClick: () => opMutation.mutate({ path: `/api/apps/${encodeURIComponent(id)}/restart`, method: "POST" })
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              onClick: () => opMutation.mutate({ path: `/api/apps/${encodeURIComponent(id)}`, method: "DELETE" })
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
                                {str(g.status)}{str(g.due_date) ? ` | due ${str(g.due_date)}` : ""}{str(g.created_at) ? ` | created ${str(g.created_at)}` : ""}
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
                              {str(it.action)} | {str(it.created_at)}
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

  const [delegateTask, setDelegateTask] = useState("");
  const [delegateContext, setDelegateContext] = useState("");
  const [delegateRequireApproval, setDelegateRequireApproval] = useState(false);
  const [delegateResult, setDelegateResult] = useState<JsonRecord | null>(null);

  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [sessionResponse, setSessionResponse] = useState("");
  const [browserRespondResult, setBrowserRespondResult] = useState<JsonRecord | null>(null);

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
  const delegateMutation = useMutation({
    mutationFn: (payload: { task: string; context?: string; require_approval?: boolean }) => api.rawPost("/autonomy/delegate", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["swarm-delegations"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    }
  });
  const browserRespondMutation = useMutation({
    mutationFn: (payload: { id: string; response: string }) =>
      api.rawPost(`/browser/sessions/${encodeURIComponent(payload.id)}/respond`, { response: payload.response }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-browser-sessions"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-browser-session-status", selectedSessionId] });
    }
  });

  const incidents = pickRecords(incidentsQ.data, "incidents");
  const timelineEvents = pickRecords(timelineQ.data, "events");
  const triageRows = pickRecords(triageResult, "triage");
  const browserSessions = pickRecords(browserSessionsQ.data, "sessions");
  const browserStatus = asRecord(browserStatusQ.data);

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
  const modeIndicator = autonomyMode === "auto" ? "Auto" : autonomyMode === "assist" ? "Assist" : "Off";
  const timelineTabIndex = 1;
  const triageTabIndex = 2;
  const delegateTabIndex = SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? 3 : 1;
  const browserTabIndex = SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? 4 : 2;
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
    const maxAllowedTab = SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? 4 : 2;
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
            <Tab label="Delegate" value={delegateTabIndex} />
            <Tab label="Browser Sessions" value={browserTabIndex} />
          </Tabs>
          {!SHOW_EXPERIMENTAL_AUTONOMY_TOOLS ? (
            <Typography variant="caption" color="text.secondary" sx={{ mt: 0.75, display: "block" }}>
              Timeline rollback and inbox triage are hidden by default to keep this view focused.
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
                        <TableCell sx={{ whiteSpace: "nowrap" }}>{str(event.timestamp, "-")}</TableCell>
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

      {showAdvanced && tab === delegateTabIndex ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography variant="h6" mb={1}>One-Click Delegate</Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  label="Task"
                  value={delegateTask}
                  onChange={(e) => setDelegateTask(e.target.value)}
                  placeholder="Example: Analyze top customer complaints and suggest fixes."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  multiline
                  minRows={4}
                  label="Context (optional)"
                  value={delegateContext}
                  onChange={(e) => setDelegateContext(e.target.value)}
                  placeholder="Constraints, links, preferred style, deadlines..."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={delegateRequireApproval}
                      onChange={(e) => setDelegateRequireApproval(e.target.checked)}
                    />
                  }
                  label="Require approval before delegation"
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Button
                  variant="contained"
                  disabled={!delegateTask.trim() || delegateMutation.isPending}
                  onClick={async () => {
                    setError(null);
                    setSuccess(null);
                    setDelegateResult(null);
                    try {
                      const out = asRecord(
                        await delegateMutation.mutateAsync({
                          task: delegateTask.trim(),
                          context: delegateContext.trim() || undefined,
                          require_approval: delegateRequireApproval
                        })
                      );
                      setDelegateResult(out);
                      setSuccess("Delegation submitted.");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {delegateMutation.isPending ? "Submitting..." : "Delegate"}
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          {delegateResult ? (
            <Box className="list-shell">
              <KeyValuePanel title="Delegation result" data={delegateResult} />
              {isRecord(delegateResult.result) ? (
                <Box sx={{ mt: 1 }}>
                  <KeyValuePanel title="Result detail" data={asRecord(delegateResult.result)} />
                </Box>
              ) : null}
            </Box>
          ) : null}
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
                      <TableCell>{str(doc.created_at)}</TableCell>
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
                        <TableCell sx={{ whiteSpace: "nowrap" }}>{str(f.created_at, "-")}</TableCell>
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
                          <TableCell sx={{ whiteSpace: "nowrap" }}>{str(pref.updated_at, "-")}</TableCell>
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
                          <TableCell sx={{ whiteSpace: "nowrap" }}>{str(item.updated_at, "-")}</TableCell>
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
                          <TableCell sx={{ whiteSpace: "nowrap" }}>{str(item.updated_at, "-")}</TableCell>
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
              Confidence: {num(selectedFact?.confidence, 0)} | Created: {str(selectedFact?.created_at, "-")}
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
                        <TableCell>{str(project.updated_at, str(project.created_at))}</TableCell>
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

function SwarmManager({ autoRefresh }: { autoRefresh: boolean }) {
  const statusQ = useQuery({ queryKey: ["swarm-status"], queryFn: () => api.rawGet("/swarm/status"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const agentsQ = useQuery({ queryKey: ["swarm-agents"], queryFn: () => api.rawGet("/swarm/agents"), refetchInterval: autoRefresh ? REFRESH_MS : false });

  const status = asRecord(statusQ.data);

  return (
    <Stack spacing={2}>
      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 120 }}><Typography variant="caption" color="text.secondary">Swarm Enabled</Typography><Typography variant="h5">{boolText(status.enabled)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 120 }}><Typography variant="caption" color="text.secondary">Total Agents</Typography><Typography variant="h5">{num(status.total_agents)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 120 }}><Typography variant="caption" color="text.secondary">Active Agents</Typography><Typography variant="h5">{num(status.active_agents)}</Typography></Box></Grid2>
        <Grid2 size={{ xs: 12, md: 3 }}><Box className="list-shell" sx={{ minHeight: 120 }}><Typography variant="caption" color="text.secondary">Delegations</Typography><Typography variant="h5">{num(asRecord(agentsQ.data).total, pickRecords(agentsQ.data, "agents").length)}</Typography></Box></Grid2>
      </Grid2>

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 6 }}><QueryTable title="Agents" path="/swarm/agents" arrayKey="agents" columns={["name", "agent_type", "status", "enabled", "capabilities"]} autoRefresh={autoRefresh} emptyLabel="No swarm agents configured." queryKey="swarm-agents-table" /></Grid2>
        <Grid2 size={{ xs: 12, lg: 6 }}><QueryTable title="Delegations" path="/swarm/delegations?limit=30" arrayKey="delegations" columns={["task", "agent_id", "success", "confidence", "execution_time_ms", "created_at"]} autoRefresh={autoRefresh} emptyLabel="No delegations yet." queryKey="swarm-delegations-table" /></Grid2>
      </Grid2>

      {statusQ.error || agentsQ.error ? <Alert severity="error">{errMessage(statusQ.error || agentsQ.error)}</Alert> : null}
    </Stack>
  );
}
function TraceManager({ autoRefresh }: { autoRefresh: boolean }) {
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);

  const traceQ = useQuery({ queryKey: ["trace-manager"], queryFn: () => api.rawGet("/trace?limit=40"), refetchInterval: autoRefresh ? REFRESH_MS : false });
  const traceDetailQ = useQuery({ queryKey: ["trace-detail", selectedTraceId], queryFn: () => api.rawGet(`/trace/${encodeURIComponent(selectedTraceId || "")}`), enabled: !!selectedTraceId });
  const approvalsQ = useQuery({
    queryKey: ["approvals-log"],
    queryFn: () => api.rawGet("/approvals/log?limit=40"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const history = pickRecords(traceQ.data, "history");
  const selectedTrace = asRecord(traceDetailQ.data);
  const steps = pickRecords(traceDetailQ.data, "steps");
  const approvals = pickRecords(approvalsQ.data, "approvals");

  return (
    <Stack spacing={2}>
      <Box className="list-shell">
        <Typography variant="h6" mb={1}>Trace History</Typography>
        <TableContainer className="table-shell">
          <Table size="small">
            <TableHead><TableRow><TableCell>Message</TableCell><TableCell>Channel</TableCell><TableCell>Status</TableCell><TableCell>Duration</TableCell><TableCell>Started</TableCell><TableCell>Ops</TableCell></TableRow></TableHead>
            <TableBody>
              {history.map((item) => {
                const id = str(item.id, "");
                return (
                  <TableRow key={id}>
                    <TableCell>{str(item.message_preview)}</TableCell>
                    <TableCell>{str(item.channel)}</TableCell>
                    <TableCell>{str(item.status)}</TableCell>
                    <TableCell>{str(item.duration_ms)}</TableCell>
                    <TableCell>{str(item.started_at)}</TableCell>
                    <TableCell align="right">
                      <RowOpsMenu
                        actions={[
                          {
                            label: "View",
                            onClick: () => setSelectedTraceId(id)
                          }
                        ]}
                        ariaLabel="Trace options"
                      />
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>

      <Box className="list-shell">
        <Typography variant="h6" mb={1}>Approval History</Typography>
        {approvals.length === 0 ? (
          <Typography variant="body2" color="text.secondary">No approval events yet.</Typography>
        ) : (
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Action</TableCell>
                  <TableCell>Rule</TableCell>
                  <TableCell>Status</TableCell>
                  <TableCell>Requested</TableCell>
                  <TableCell>Resolved By</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {approvals.map((item, idx) => (
                  <TableRow key={str(item.id, `approval-${idx}`)}>
                    <TableCell sx={{ maxWidth: 280 }}>
                      <Typography variant="body2" noWrap title={str(item.action_name, "-")}>
                        {str(item.action_name, "-")}
                      </Typography>
                    </TableCell>
                    <TableCell>{str(item.rule_name, "-")}</TableCell>
                    <TableCell>{str(item.status, "-")}</TableCell>
                    <TableCell>{str(item.requested_at, "-")}</TableCell>
                    <TableCell>{str(item.resolved_by, "-")}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </Box>

      {traceQ.error || traceDetailQ.error || approvalsQ.error ? (
        <Alert severity="error">{errMessage(traceQ.error || traceDetailQ.error || approvalsQ.error)}</Alert>
      ) : null}

      <Dialog open={selectedTraceId != null} onClose={() => setSelectedTraceId(null)} maxWidth="md" fullWidth>
        <DialogTitle>Trace Detail</DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Typography variant="caption" color="text.secondary">{str(selectedTrace.started_at)} | {str(selectedTrace.channel)}</Typography>
            <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>{str(selectedTrace.message)}</Typography>
            <Box className="metadata-box" sx={{ maxHeight: 340 }}>
              {steps.length === 0 ? (
                <Typography variant="body2" color="text.secondary">No steps.</Typography>
              ) : (
                <Stack spacing={1}>
                  {steps.map((step, idx) => (
                    <Box key={str(step.id, `step-${idx}`)} className="console-line">
                      <Typography variant="caption" color="text.secondary">{str(step.time)} | {str(step.type)}</Typography>
                      <Typography variant="body2">{str(step.title)}</Typography>
                      <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "pre-wrap" }}>{str(step.detail)}</Typography>
                    </Box>
                  ))}
                </Stack>
              )}
            </Box>
            {selectedTrace.response ? (
              <>
                <Typography variant="subtitle2">Response</Typography>
                <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>{str(selectedTrace.response)}</Typography>
              </>
            ) : null}
          </Stack>
        </DialogContent>
      </Dialog>
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
                  return (
                    <Box key={id} className="action-row">
                      <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={1}>
                        <Stack>
                          <Typography variant="body2">{str(w.description)}</Typography>
                          <Typography variant="caption" color="text.secondary">{str(w.status)} | {str(w.interval_secs)}s</Typography>
                        </Stack>
                        <Button size="small" color="warning" onClick={async () => { setError(null); try { await cancelMutation.mutateAsync(id); } catch (e) { setError(errMessage(e)); } }}>Cancel</Button>
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

function SettingsManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState(() => {
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
  const [vaultRevealedValues, setVaultRevealedValues] = useState<Record<string, string>>({});
  const [vaultEditorOpen, setVaultEditorOpen] = useState(false);
  const [vaultEditorMode, setVaultEditorMode] = useState<VaultEditorMode>("add");
  const [vaultEditorKey, setVaultEditorKey] = useState("");
  const [vaultEditorValue, setVaultEditorValue] = useState("");
  const [showVaultSecretValue, setShowVaultSecretValue] = useState(false);
  const [securityLogsDialogOpen, setSecurityLogsDialogOpen] = useState(false);
  const [selectedSecurityLog, setSelectedSecurityLog] = useState<JsonRecord | null>(null);
  const [selectedPulseEvent, setSelectedPulseEvent] = useState<JsonRecord | null>(null);
  const [selectedMoltbookEvent, setSelectedMoltbookEvent] = useState<JsonRecord | null>(null);
  const [pulsePollState, setPulsePollState] = useState<{ baselineEventId: string; deadlineAt: number } | null>(null);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(getDeveloperModeEnabled);
  const [trustPresetId, setTrustPresetId] = useState(TRUST_APPROVAL_PRESETS[0]?.id ?? "run_terminal_command");
  const [trustPresetDetail, setTrustPresetDetail] = useState("ls -la");
  const [trustUseAdvancedInput, setTrustUseAdvancedInput] = useState(false);
  const [trustActionKind, setTrustActionKind] = useState("shell");
  const [trustPayloadJson, setTrustPayloadJson] = useState("{}");
  const [trustResult, setTrustResult] = useState<JsonRecord | null>(null);

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
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const moltbookLogQ = useQuery({ 
    queryKey: ["moltbook-log"], 
    queryFn: () => api.rawGet("/moltbook/log?limit=40"), 
    refetchInterval: autoRefresh ? REFRESH_MS : false 
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
  const media = asRecord(mediaQ.data);
  const modelsPayload = asRecord(modelsQ.data);
  const evolution = asRecord(evolutionQ.data);
  const evolutionCanary = asRecord(evolution.canary);
  const evolutionDev = asRecord(evolutionDevQ.data);
  const evolutionPolicyMetrics = pickRecords(evolutionDev, "policy_metrics");
  const evolutionStrategyMetrics = pickRecords(evolutionDev, "strategy_metrics");
  const evolutionLineage = pickRecords(evolutionDev, "lineage_recent");

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

    search_primary: "playwright",
    search_fallback1: "duckduckgo",
    search_fallback2: "none",
    search_serper_key: "",
    search_searxng_url: "",
    search_brave_key: "",

    moltbook_api_key: "",
    moltbook_enabled: false,
    moltbook_mode: "read_only",
    moltbook_sync_frequency: "twice_daily",
    moltbook_write_enabled: false,
    moltbook_defer_when_busy: true
  });

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

    setForm((prev) => ({
      ...prev,
      bot_name: str(settings.bot_name, prev.bot_name),
      personality: str(settings.personality, prev.personality),
      timezone: str(settings.timezone, ""),
      language: str(settings.language, prev.language),
      tone: str(settings.tone, prev.tone),
      email_format: str(settings.email_format, prev.email_format),
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

      search_primary: str(settings.search_primary, "playwright"),
      search_fallback1: str(settings.search_fallback1, "duckduckgo"),
      search_fallback2: str(settings.search_fallback2, "none"),
      search_serper_key: "",
      search_searxng_url: str(settings.search_searxng_url, ""),
      search_brave_key: "",

      moltbook_api_key: "",
      moltbook_enabled: toBool(settings.moltbook_enabled),
      moltbook_mode: str(settings.moltbook_mode, "read_only"),
      moltbook_sync_frequency: str(settings.moltbook_sync_frequency, "twice_daily"),
      moltbook_write_enabled: toBool(settings.moltbook_write_enabled),
      moltbook_defer_when_busy: toBool(settings.moltbook_defer_when_busy)
    }));

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
  }, [initialized, settingsQ.data, mediaQ.data]); // eslint-disable-line react-hooks/exhaustive-deps

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
        moltbook_defer_when_busy: form.moltbook_defer_when_busy
      };

      return await api.rawPost("/settings", payload);
    },
    onSuccess: async () => {
      setError(null);
      setSuccess("Saved settings.");
      setDirty(false);
      setForm((prev) => ({
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
        search_brave_key: ""
      }));
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-media"] });
      await queryClient.invalidateQueries({ queryKey: ["models"] });
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
  const updateEvolutionSettingsMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) =>
      api.rawPost("/settings/evolution", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution-dev"] });
    },
    onError: (e) => setError(errMessage(e))
  });
  const runEvolutionDevActionMutation = useMutation({
    mutationFn: (action: string) =>
      api.rawPost("/settings/evolution/dev/action", { action }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution-dev"] });
    },
    onError: (e) => setError(errMessage(e))
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
  const hasWhatsAppToken = toBool(settings.has_whatsapp_token);
  const hasPrimaryApiKey = toBool(settings.has_api_key);
  const hasFallbackApiKey = toBool(settings.has_fallback_api_key);
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
  const sec = asRecord(securityStatusQ.data);
  const securityLogs = pickRecords(securityLogsQ.data, "logs");
  const hasCustomMasterPassword = toBool(sec.master_password_set) && !toBool(sec.using_default);
  const vaultSecrets = pickRecords(vaultSecretsQ.data, "entries");
  const pulseEvents = pickRecords(pulseQ.data, "events");
  const pulseMeta = asRecord(pulseQ.data);
  const pulseRunning = toBool(pulseMeta.running);
  const latestPulseEventId = str(asRecord(pulseEvents[0]).id, "");
  const moltbookStatus = asRecord(moltbookStatusQ.data);
  const moltbookRunning = toBool(moltbookStatus.running);
  const moltbookLastStatus = str(moltbookStatus.last_status, "").toLowerCase();
  const moltbookNeedsConnection =
    moltbookLastStatus === "not_connected" ||
    moltbookLastStatus === "not_configured" ||
    moltbookLastStatus === "error";

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
        detail: "Use the fix command under each issue, then run ArkPulse again."
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
    if (a === "feed_fetched" || a === "feed_read") return "Feed fetched";
    if (a === "post_created") return "Post created";
    if (a.startsWith("error_")) return `Error: ${action}`;
    // Fall back to the raw action code.
    return action || "-";
  }

  function moltbookReason(action: string, details: JsonRecord): string | null {
    const explicit = str(details.reason, "").trim();
    if (explicit) return explicit;
    const a = (action || "").toLowerCase();
    if (a === "skipped_disabled") return "Moltbook is disabled in Settings.";
    if (a === "skipped_off_mode") return "Moltbook mode is set to off.";
    if (a === "deferred_busy") return "Deferred because the server was busy.";
    if (a === "skipped_busy_max_defers") return "Skipped because the server stayed busy after multiple defers.";
    if (a === "not_connected") {
      const status = str(details.status, "").toLowerCase();
      const err = str(details.error, "").trim();
      if (status === "not_configured") {
        return "Moltbook API key is not configured. Enter it in the Moltbook settings tab.";
      }
      if (status === "error") {
        return err
          ? `Moltbook authentication failed: ${err}`
          : "Moltbook authentication failed (invalid API key or unclaimed agent).";
      }
      return "Could not connect to Moltbook.";
    }
    return null;
  }

  const regenerateApiKeyMutation = useMutation({
    mutationFn: () => api.rawPost("/settings/api-key/regenerate", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-api-key"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const tunnelStartMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/start", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
    },
    onError: (e) => setError(errMessage(e))
  });

  const tunnelStopMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/stop", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
    },
    onError: (e) => setError(errMessage(e))
  });

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
      setSuccess("Master password set. Server will restart.");
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
    },
    onError: (e) => setError(errMessage(e))
  });

  const changePasswordMutation = useMutation({
    mutationFn: (payload: { current_password: string; new_password: string }) =>
      api.rawPost("/security/change-password", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      setSuccess("Master password changed. Server will restart.");
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
    },
    onError: (e) => setError(errMessage(e))
  });

  const removePasswordMutation = useMutation({
    mutationFn: (password: string) => api.rawPost("/security/remove-password", { password }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      setSuccess("Master password removed. Server will restart.");
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
    },
    onError: (e) => setError(errMessage(e))
  });

  const passwordMutationPending =
    setPasswordMutation.isPending || changePasswordMutation.isPending || removePasswordMutation.isPending;

  const revealVaultSecretsMutation = useMutation({
    mutationFn: (payload: { password?: string; keys?: string[] }) =>
      api.rawPost("/settings/secrets/reveal", payload),
    onSuccess: (raw) => {
      const entries = pickRecords(raw, "entries");
      if (entries.length === 0) return;
      setVaultRevealedValues((prev) => {
        const next = { ...prev };
        for (const row of entries) {
          const key = str(row.key, "").trim();
          if (!key) continue;
          next[key] = str(row.value, "");
        }
        return next;
      });
    },
    onError: (e) => setError(errMessage(e))
  });

  const upsertVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; value: string; password?: string }) =>
      api.rawPost("/settings/secrets/upsert", payload),
    onSuccess: async (_, vars) => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setSuccess("Secret saved.");
      setVaultRevealedValues((prev) => (Object.prototype.hasOwnProperty.call(prev, vars.key) ? { ...prev, [vars.key]: vars.value } : prev));
    },
    onError: (e) => setError(errMessage(e))
  });

  const deleteVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; password?: string }) =>
      api.rawPost("/settings/secrets/delete", payload),
    onSuccess: async (_, vars) => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setVaultRevealedValues((prev) => {
        const next = { ...prev };
        delete next[vars.key];
        return next;
      });
      setSuccess("Secret deleted.");
    },
    onError: (e) => setError(errMessage(e))
  });

  function resolveVaultPasswordForSensitiveOps(): string | null | undefined {
    if (!hasCustomMasterPassword) return undefined;
    const pw = vaultPassword.trim();
    if (!pw) {
      setError("Master password is required for secret reveal/edit operations.");
      return null;
    }
    return pw;
  }

  function openVaultEditor(mode: VaultEditorMode, key?: string) {
    setError(null);
    setSuccess(null);
    setVaultEditorMode(mode);
    setVaultEditorKey(key || "");
    setVaultEditorValue(mode === "edit" && key ? (vaultRevealedValues[key] || "") : "");
    setShowVaultSecretValue(false);
    setVaultEditorOpen(true);
  }

  function closeVaultEditor() {
    if (upsertVaultSecretMutation.isPending) return;
    setVaultEditorOpen(false);
    setVaultEditorMode("add");
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
      items: [
        { value: 11, label: "Trace" },
        { value: 9, label: "ArkPulse" },
        { value: 13, label: "Evolution" }
      ]
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
  const selectedSettingsNav = settingsNavActual.find((item) => item.value === tab) || settingsNavActual[0];

  return (
    <Stack spacing={2}>
      {showSetupRequired ? (
        <Alert severity="warning">
          Setup required: configure at least one model in the Models tab, then Save Settings.
        </Alert>
      ) : null}

      <Box className="settings-shell-layout">
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
        <Box className="settings-main">
          <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mb: 1.5 }}>
            <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "1rem" }}>{selectedSettingsNav?.label || "Settings"}</Typography>
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
                disabled={saveMutation.isPending || !dirty}
              >
                Save
              </Button>
            </Stack>
          </Stack>

      {tab === 0 ? (
        <Stack spacing={2.5}>
          {/* ── Status Overview ── */}
          <Box>
            <Typography className="settings-section-label">Status</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr 1fr", md: "repeat(4, 1fr)" }, gap: 1.5 }}>
              {[
                { label: "Primary API Key", ok: hasPrimaryApiKey },
                { label: "Fallback API Key", ok: hasFallbackApiKey },
                { label: "Telegram", ok: hasTelegramToken },
                { label: "WhatsApp", ok: hasWhatsAppToken },
              ].map((s) => (
                <Box
                  key={s.label}
                  sx={{
                    p: 1.5,
                    borderRadius: 2,
                    border: "1px solid",
                    borderColor: s.ok ? "rgba(20,241,149,0.18)" : "rgba(255,255,255,0.06)",
                    background: s.ok ? "rgba(20,241,149,0.04)" : "rgba(255,255,255,0.02)",
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
                      background: s.ok ? "#14f195" : "rgba(255,255,255,0.15)",
                      boxShadow: s.ok ? "0 0 6px rgba(20,241,149,0.4)" : "none",
                    }}
                  />
                  <Stack spacing={0}>
                    <Typography variant="caption" sx={{ color: "rgba(180,200,225,0.55)", fontSize: "0.68rem", lineHeight: 1.2 }}>{s.label}</Typography>
                    <Typography variant="body2" sx={{ fontWeight: 500, fontSize: "0.8rem", color: s.ok ? "rgba(225,242,255,0.9)" : "rgba(180,200,225,0.45)" }}>
                      {s.ok ? "Connected" : "Not configured"}
                    </Typography>
                  </Stack>
                </Box>
              ))}
            </Box>
            <Box sx={{ display: "flex", gap: 2, mt: 1.5, flexWrap: "wrap" }}>
              <Chip size="small" variant="outlined" label={`${modelSlots.length} model${modelSlots.length !== 1 ? "s" : ""}`} sx={{ borderColor: "rgba(47,212,255,0.25)", color: "rgba(47,212,255,0.85)", fontSize: "0.72rem" }} />
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
          <Box>
            <Typography className="settings-section-label">Identity</Typography>
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
          </Box>

          <hr className="settings-divider" />

          {/* ── Preferences ── */}
          <Box>
            <Typography className="settings-section-label">Preferences</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr 1fr" }, gap: 1.5 }}>
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
              <TextField
                label="Daily Brief Channel"
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
                        Password setup opens a secure dialog. Changes apply immediately and restart the server.
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
                {tunnelQ.isLoading ? (
                  <Typography variant="body2" color="text.secondary">
                    Loading tunnel status...
                  </Typography>
                ) : tunnelQ.error ? (
                  <Alert severity="error">{errMessage(tunnelQ.error)}</Alert>
                ) : (
                  <Stack spacing={1}>
                    <Typography variant="caption" color="text.secondary">
                      Active: {boolText(tunnel.active)} | Available: {boolText(tunnel.available)}
                    </Typography>
                    {str(tunnel.url, "").trim() ? (
                      <TextField
                        label="Public URL"
                        value={str(tunnel.url)}
                        fullWidth
                        size="small"
                        InputProps={{ readOnly: true }}
                      />
                    ) : null}
                    {str(tunnel.error, "").trim() ? <Alert severity="error">{str(tunnel.error)}</Alert> : null}
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        variant="contained"
                        onClick={async () => {
                          setError(null);
                          try {
                            await tunnelStartMutation.mutateAsync();
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                        disabled={tunnelStartMutation.isPending || toBool(tunnel.active) || !toBool(tunnel.available)}
                      >
                        Start Tunnel
                      </Button>
                      <Button
                        size="small"
                        onClick={async () => {
                          setError(null);
                          try {
                            await tunnelStopMutation.mutateAsync();
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                        disabled={tunnelStopMutation.isPending || !toBool(tunnel.active)}
                      >
                        Stop Tunnel
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
                        Copy URL
                      </Button>
                    </Stack>
                    <Alert
                      severity="warning"
                      sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem", lineHeight: 1.35 } }}
                    >
                      Anyone with the URL can control your agent. Use an API key and stop the tunnel when not needed.
                    </Alert>
                  </Stack>
                )}
              </Box>

              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Stack spacing={1}>
                  <Typography variant="h6">Secrets Vault</Typography>
                  <Typography variant="caption" color="text.secondary">
                    Manage encrypted custom secrets used by skills, integrations, and agent workflows.
                  </Typography>
                  {hasCustomMasterPassword ? (
                    <TextField
                      label="Master password (required for reveal/edit/delete)"
                      value={vaultPassword}
                      onChange={(e) => setVaultPassword(e.target.value)}
                      fullWidth
                      size="small"
                      type="password"
                    />
                  ) : (
                    <Alert severity="info">
                      No custom master password is set. Secrets are still encrypted at rest, and reveal is available in this session.
                    </Alert>
                  )}
                  <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                    <Button
                      size="small"
                      variant="contained"
                      onClick={async () => {
                        const pw = resolveVaultPasswordForSensitiveOps();
                        if (pw === null) return;
                        setError(null);
                        try {
                          await revealVaultSecretsMutation.mutateAsync({ password: pw || undefined });
                          setSuccess("Secrets revealed.");
                        } catch {
                          // handled by mutation onError
                        }
                      }}
                      disabled={revealVaultSecretsMutation.isPending || vaultSecrets.length === 0}
                    >
                      Reveal all
                    </Button>
                    <Button
                      size="small"
                      onClick={() => setVaultRevealedValues({})}
                      disabled={Object.keys(vaultRevealedValues).length === 0}
                    >
                      Hide all
                    </Button>
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
                      onClick={() => openVaultEditor("add")}
                    >
                      Add Secret
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
                      No custom secrets yet.
                    </Typography>
                  ) : (
                    <TableContainer className="table-shell">
                      <Table size="small">
                        <TableHead>
                          <TableRow>
                            <TableCell>Key</TableCell>
                            <TableCell>Value</TableCell>
                            <TableCell align="right">Ops</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {vaultSecrets.map((row, idx) => {
                            const key = str(row.key, "");
                            const revealed = Object.prototype.hasOwnProperty.call(vaultRevealedValues, key);
                            const shownValue = revealed ? vaultRevealedValues[key] : str(row.masked, "");
                            return (
                              <TableRow key={`${key}-${idx}`}>
                                <TableCell sx={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}>{key}</TableCell>
                                <TableCell sx={{ maxWidth: 360 }}>
                                  <Typography
                                    variant="body2"
                                    title={shownValue}
                                    sx={{
                                      whiteSpace: "nowrap",
                                      overflow: "hidden",
                                      textOverflow: "ellipsis",
                                      fontFamily: revealed ? "ui-monospace, SFMono-Regular, Menlo, monospace" : "inherit"
                                    }}
                                  >
                                    {shownValue || "-"}
                                  </Typography>
                                </TableCell>
                                <TableCell align="right">
                                  <Stack direction="row" spacing={0.5} justifyContent="flex-end">
                                    <Button
                                      size="small"
                                      onClick={async () => {
                                        if (revealed) {
                                          setVaultRevealedValues((prev) => {
                                            const next = { ...prev };
                                            delete next[key];
                                            return next;
                                          });
                                          return;
                                        }
                                        const pw = resolveVaultPasswordForSensitiveOps();
                                        if (pw === null) return;
                                        setError(null);
                                        try {
                                          await revealVaultSecretsMutation.mutateAsync({
                                            password: pw || undefined,
                                            keys: [key]
                                          });
                                        } catch {
                                          // handled by mutation onError
                                        }
                                      }}
                                      disabled={revealVaultSecretsMutation.isPending}
                                    >
                                      {revealed ? "Hide" : "Reveal"}
                                    </Button>
                                    <Button
                                      size="small"
                                      onClick={() => openVaultEditor("edit", key)}
                                    >
                                      Edit
                                    </Button>
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
        <Grid2 container spacing={2}>
          <Grid2 size={{ xs: 12 }}>
            <Alert severity="warning">
              Advanced settings are powerful and can impact stability or security. Change only if you understand the effect.
            </Alert>
          </Grid2>
          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={2}>
                <Stack spacing={0.35}>
                  <Typography variant="h6">Restart Bot</Typography>
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
                >
                  Restart Bot
                </Button>
              </Stack>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={2}>
                <Stack spacing={0.35}>
                  <Typography variant="h6">Developer Mode</Typography>
                  <Typography variant="caption" color="text.secondary">
                    Enables raw SKILL.md editing in Skills. Keep this off for beginner-friendly forms.
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
                  label={developerModeEnabled ? "Enabled" : "Disabled"}
                />
              </Stack>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={2}>
                <Stack spacing={0.35}>
                  <Typography variant="h6">Guided Tour</Typography>
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
                >
                  Restart Tour
                </Button>
              </Stack>
            </Box>
          </Grid2>

          <Grid2 size={{ xs: 12 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Typography variant="h6" mb={1}>Will This Need Approval?</Typography>
              <Typography variant="caption" color="text.secondary">
                Pick what you want to do and check the likely approval requirement. This is only a preview and does not run anything.
              </Typography>
              <Grid2 container spacing={1} sx={{ mt: 0.75 }}>
                <Grid2 size={{ xs: 12, md: 5 }}>
                  <TextField
                    fullWidth
                    size="small"
                    select
                    label="What do you want to do?"
                    value={trustPresetId}
                    onChange={(e) => {
                      const nextId = e.target.value;
                      setTrustPresetId(nextId);
                      const nextPreset =
                        TRUST_APPROVAL_PRESETS.find((item) => item.id === nextId) ?? TRUST_APPROVAL_PRESETS[0];
                      if (nextPreset) {
                        setTrustActionKind(nextPreset.actionKind);
                      }
                    }}
                  >
                    {TRUST_APPROVAL_PRESETS.map((preset) => (
                      <MenuItem key={preset.id} value={preset.id}>
                        {preset.label}
                      </MenuItem>
                    ))}
                  </TextField>
                </Grid2>
                <Grid2 size={{ xs: 12, md: 7 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label={selectedTrustPreset ? selectedTrustPreset.detailLabel : "Details"}
                    value={trustPresetDetail}
                    onChange={(e) => setTrustPresetDetail(e.target.value)}
                    placeholder={selectedTrustPreset ? selectedTrustPreset.detailPlaceholder : "Add a short detail"}
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={trustUseAdvancedInput}
                        onChange={(e) => setTrustUseAdvancedInput(e.target.checked)}
                      />
                    }
                    label="Use advanced input (action name + JSON)"
                  />
                </Grid2>
                {trustUseAdvancedInput ? (
                  <>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Technical action name"
                        value={trustActionKind}
                        onChange={(e) => setTrustActionKind(e.target.value)}
                        placeholder="shell"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 8 }}>
                      <TextField
                        fullWidth
                        size="small"
                        multiline
                        minRows={3}
                        label="Technical payload (JSON)"
                        value={trustPayloadJson}
                        onChange={(e) => setTrustPayloadJson(e.target.value)}
                        placeholder='{"command":"ls -la"}'
                      />
                    </Grid2>
                  </>
                ) : null}
                <Grid2 size={{ xs: 12 }}>
                  <Button
                    variant="contained"
                    disabled={
                      trustEvaluateMutation.isPending ||
                      (trustUseAdvancedInput ? !trustActionKind.trim() : !trustPresetDetail.trim())
                    }
                    onClick={async () => {
                      setError(null);
                      setSuccess(null);
                      setTrustResult(null);
                      let actionKind = "";
                      let parsedPayload: unknown = {};
                      if (trustUseAdvancedInput) {
                        actionKind = trustActionKind.trim();
                        const raw = trustPayloadJson.trim();
                        if (raw) {
                          try {
                            parsedPayload = JSON.parse(raw);
                          } catch {
                            setError("Technical payload JSON is invalid.");
                            return;
                          }
                        }
                      } else {
                        const preset = selectedTrustPreset;
                        const detail = trustPresetDetail.trim();
                        if (!preset) {
                          setError("Select an action first.");
                          return;
                        }
                        if (!detail) {
                          setError("Add a short detail so risk can be estimated.");
                          return;
                        }
                        actionKind = preset.actionKind;
                        parsedPayload = preset.buildPayload(detail);
                        setTrustActionKind(actionKind);
                        setTrustPayloadJson(JSON.stringify(parsedPayload, null, 2));
                      }
                      try {
                        const out = asRecord(
                          await trustEvaluateMutation.mutateAsync({
                            action_kind: actionKind,
                            payload: parsedPayload
                          })
                        );
                        setTrustResult(asRecord(out.risk));
                      } catch (e) {
                        setError(errMessage(e));
                      }
                    }}
                  >
                    {trustEvaluateMutation.isPending ? "Checking..." : "Check Approval"}
                  </Button>
                </Grid2>
                {trustResult ? (
                  <Grid2 size={{ xs: 12 }}>
                    <Stack spacing={1}>
                      <Alert severity={toBool(trustResult.requires_approval) ? "warning" : "success"}>
                        {toBool(trustResult.requires_approval)
                          ? "This will likely require your approval before running."
                          : "This is likely safe to run without manual approval."}
                      </Alert>
                      <KeyValuePanel title="Risk details" data={trustResult} />
                    </Stack>
                  </Grid2>
                ) : null}
              </Grid2>
            </Box>
          </Grid2>

          <Grid2 size={{ xs: 12, lg: 6 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Typography variant="h6" mb={1}>
                Auto-Approve Skills
              </Typography>
              <Typography variant="caption" color="text.secondary">
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
                  <Grid2 container spacing={1} sx={{ mt: 1 }}>
                    {items.map((name) => (
                      <Grid2 key={name} size={{ xs: 12, md: 6 }}>
                        <FormControlLabel
                          control={<Switch checked={set.has(name)} onChange={(e) => update(name, e.target.checked)} />}
                          label={name}
                        />
                      </Grid2>
                    ))}
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        label="Auto-Approve (manual CSV)"
                        value={form.auto_approve_csv}
                        onChange={(e) => setField("auto_approve_csv", e.target.value)}
                        fullWidth
                        size="small"
                        placeholder="comma separated action names"
                      />
                    </Grid2>
                  </Grid2>
                );
              })()}
            </Box>
          </Grid2>

          <Grid2 size={{ xs: 12, lg: 6 }}>
            <Box className="list-shell" sx={{ minHeight: 0 }}>
              <Typography variant="h6" mb={1}>
                API Key (HTTP)
              </Typography>
              {apiKeyQ.isLoading ? (
                <Typography variant="body2" color="text.secondary">
                  Loading API key...
                </Typography>
              ) : apiKeyQ.error ? (
                <Alert severity="error">{errMessage(apiKeyQ.error)}</Alert>
              ) : (
                <Stack spacing={1}>
                  <Typography variant="caption" color="text.secondary">
                    Used as `Authorization: Bearer &lt;key&gt;` for all HTTP API requests.
                  </Typography>
                  <Typography variant="caption" color={apiKeyRemainingSeconds > 0 ? "text.secondary" : "warning.main"}>
                    Rotates in {formatDurationClock(apiKeyRemainingSeconds)}
                    {apiKeyExpiresAtUnix > 0
                      ? ` (next: ${new Date(apiKeyExpiresAtUnix * 1000).toLocaleString()})`
                      : ""}
                  </Typography>
                  {apiKeyRotated ? (
                    <Typography variant="caption" color="info.main">
                      API key rotated automatically.
                    </Typography>
                  ) : null}
                  <TextField
                    label="Key"
                    value={apiKeyRevealed ? str(apiKeyPayload.key, "") : str(apiKeyPayload.masked, "")}
                    fullWidth
                    size="small"
                    InputProps={{ readOnly: true }}
                  />
                  {apiKeyIssuedAtUnix > 0 ? (
                    <Typography variant="caption" color="text.secondary">
                      Issued: {new Date(apiKeyIssuedAtUnix * 1000).toLocaleString()}
                    </Typography>
                  ) : null}
                  <Stack direction="row" spacing={1}>
                    <Button size="small" onClick={() => setApiKeyRevealed((v) => !v)}>
                      {apiKeyRevealed ? "Hide" : "Reveal"}
                    </Button>
                    <Button
                      size="small"
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
          </Grid2>

        </Grid2>
      ) : null}

      {tab === 7 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Stack spacing={0.6}>
              <Stack direction="row" justifyContent="space-between" alignItems="center">
                <Typography variant="h6">Moltbook</Typography>
                <FormControlLabel
                  control={
                    <Switch
                      checked={form.moltbook_enabled}
                      onChange={(e) => setField("moltbook_enabled", e.target.checked)}
                    />
                  }
                  label="Enabled"
                />
              </Stack>
              <Typography variant="body2" color="text.secondary">
                Moltbook is a decentralized social network for autonomous AI agents. When enabled, AgentArk
                can discover and collaborate with other agents, negotiate task delegation, share capabilities,
                and participate in multi-agent workflows across the network. All communication is
                zero-knowledge, and no user data, secrets, PII, or conversation content ever leaves your instance.
                Only capability metadata, anonymized skill signatures, and agent availability are shared.
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Disabled by default. Your agent joins the network as a peer, and all inbound requests
                go through the same approval and action-guard rules as any other task.
              </Typography>
              {form.moltbook_enabled ? (
                <TextField
                  label="Moltbook API Key"
                  type="password"
                  value={form.moltbook_api_key}
                  onChange={(e) => setField("moltbook_api_key", e.target.value)}
                  fullWidth
                  size="small"
                  placeholder="Enter your Moltbook API key"
                  helperText="Required to connect to the Moltbook network. Get your key at moltbook.com"
                  sx={{ mt: 1 }}
                />
              ) : (
                <Typography variant="caption" color="text.secondary" sx={{ mt: 1 }}>
                  Turn on Moltbook to add your API key.
                </Typography>
              )}
            </Stack>
          </Box>

          <Grid2 container spacing={2}>
            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
                  <Stack spacing={0.2}>
                    <Typography variant="h6">Sync Settings</Typography>
                    <Typography variant="caption" color="text.secondary">
                      Controls background cadence and write behavior.
                    </Typography>
                  </Stack>
                </Stack>

                <Grid2 container spacing={1}>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.moltbook_defer_when_busy}
                          onChange={(e) => setField("moltbook_defer_when_busy", e.target.checked)}
                        />
                      }
                      label="Defer When Busy"
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.moltbook_write_enabled}
                          onChange={(e) => setField("moltbook_write_enabled", e.target.checked)}
                        />
                      }
                      label="Write Enabled"
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      label="Mode"
                      select
                      value={form.moltbook_mode}
                      onChange={(e) => setField("moltbook_mode", e.target.value)}
                      fullWidth
                      size="small"
                    >
                      <MenuItem value="off">off</MenuItem>
                      <MenuItem value="read_only">read_only</MenuItem>
                      <MenuItem value="assist">assist</MenuItem>
                      <MenuItem value="autopost">autopost</MenuItem>
                    </TextField>
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      label="Sync Frequency"
                      select
                      value={form.moltbook_sync_frequency}
                      onChange={(e) => setField("moltbook_sync_frequency", e.target.value)}
                      fullWidth
                      size="small"
                    >
                      <MenuItem value="twice_daily">twice_daily</MenuItem>
                      <MenuItem value="daily">daily</MenuItem>
                    </TextField>
                  </Grid2>
                </Grid2>
              </Box>
            </Grid2>

            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
                  <Stack spacing={0.2}>
                    <Typography variant="h6">Connector Status</Typography>
                    <Typography variant="caption" color="text.secondary">
                      Registration and recent runs.
                    </Typography>
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
                          setSuccess(`Moltbook run completed. Read ${readCount} post${readCount === 1 ? "" : "s"}.`);
                        } else if (status === "running") {
                          setSuccess(str(out.message, "Moltbook run is already in progress."));
                        } else if (status === "not_connected") {
                          setError(str(out.reason, "Moltbook is not connected. Enter your API key above, save settings, then run again."));
                        } else if (status === "disabled") {
                          setError("Moltbook is disabled in Settings.");
                        } else if (status === "off_mode") {
                          setError("Moltbook mode is off.");
                        } else if (status === "deferred_busy" || status === "skipped_busy") {
                          setError("Moltbook run deferred because the system is busy.");
                        } else if (status === "not_due") {
                          setError("Moltbook run skipped because next scheduled run is not due yet.");
                        } else {
                          setSuccess(`Moltbook run returned status: ${status}.`);
                        }
                      } catch (e) {
                        setError(errMessage(e));
                      }
                    }}
                    disabled={runMoltbookMutation.isPending || moltbookRunning}
                  >
                    {runMoltbookMutation.isPending || moltbookRunning ? "Running..." : "Run now"}
                  </Button>
                </Stack>

                {moltbookStatusQ.error ? <Alert severity="error">{errMessage(moltbookStatusQ.error)}</Alert> : null}
                {moltbookNeedsConnection ? (
                  <Alert
                    severity="warning"
                    sx={{ mb: 1 }}
                    action={
                      <Button size="small" variant="outlined" onClick={() => setTab(2)}>
                        Open Integrations
                      </Button>
                    }
                  >
                    Moltbook is not connected. Enter your API key above, save settings, then run again.
                  </Alert>
                ) : null}
                <Grid2 container spacing={1}>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Typography variant="body2">Connector: {boolText(moltbookStatus.connector_registered)}</Typography>
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Typography variant="body2">Last status: {str(moltbookStatus.last_status, "-")}</Typography>
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Typography variant="body2">Last run: {str(moltbookStatus.last_run_at, "-")}</Typography>
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <Typography variant="body2">Next run: {str(moltbookStatus.next_run_at, "-")}</Typography>
                  </Grid2>
                </Grid2>
              </Box>
            </Grid2>
          </Grid2>

          <Box className="list-shell" sx={{ minHeight: 0 }}>
            <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
              <Typography variant="h6">Moltbook Activity</Typography>
              <Typography variant="caption" color="text.secondary">
                Recent sync runs.
              </Typography>
            </Stack>
            {moltbookLogQ.error ? <Alert severity="error">{errMessage(moltbookLogQ.error)}</Alert> : null}
            {moltbookEvents.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No Moltbook events yet.
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
                    {moltbookEvents.slice(0, 40).map((ev, idx) => {
                      const details = asRecord(ev.details);
                      const rawAction = str(ev.action, "-");
                      const label = moltbookActionLabel(rawAction, details);
                      const reason = moltbookReason(rawAction, details);
                      const trigger = str(details.trigger, "");
                      const triggerLabel = trigger ? moltbookTriggerLabel(trigger) : "";
                      const hover = [label, triggerLabel ? `Trigger: ${triggerLabel}` : "", reason ? `Reason: ${reason}` : ""]
                        .filter(Boolean)
                        .join("\n");
                      return (
                      <TableRow key={str(ev.id, String(idx))}>
                        <TableCell sx={{ whiteSpace: "nowrap" }}>{str(ev.timestamp, "-")}</TableCell>
                        <TableCell>
                          <Chip size="small" label={str(ev.level, "-")} color={severityChipColor(str(ev.level, ""))} />
                        </TableCell>
                        <TableCell sx={{ maxWidth: 420 }}>
                          <Stack spacing={0.25}>
                            <Typography variant="body2" noWrap title={hover}>
                              {label}
                            </Typography>
                            {triggerLabel || reason ? (
                              <Typography variant="caption" color="text.secondary" noWrap title={hover}>
                                {triggerLabel ? triggerLabel : ""}{triggerLabel && reason ? " | " : ""}{reason ? reason : ""}
                              </Typography>
                            ) : null}
                          </Stack>
                        </TableCell>
                        <TableCell sx={{ maxWidth: 260 }}>
                          <Typography variant="body2" noWrap title={str(ev.run_id, "-")}>
                            {str(ev.run_id, "-")}
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
                              <TableCell sx={{ whiteSpace: "nowrap" }}>{str(ev.timestamp, "-")}</TableCell>
                              <TableCell>
                                <Chip
                                  size="small"
                                  label={ok ? "OK" : status || "check"}
                                  color={ok ? "success" : "warning"}
                                  variant={ok ? "filled" : "outlined"}
                                />
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
          <Grid2 container spacing={2}>
            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Typography variant="h6" mb={1}>Evolution Status</Typography>
                {evolutionQ.isLoading ? (
                  <Typography variant="body2" color="text.secondary">Loading evolution status...</Typography>
                ) : evolutionQ.error ? (
                  <Alert severity="error">{errMessage(evolutionQ.error)}</Alert>
                ) : (
                  <Stack spacing={1}>
                    <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                      <Typography variant="body2">Self-evolve:</Typography>
                      <Chip size="small" color={toBool(evolution.self_evolve_enabled) ? "success" : "default"} label={toBool(evolution.self_evolve_enabled) ? "On" : "Off"} />
                    </Stack>
                    <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap">
                      <Typography variant="body2">Canary:</Typography>
                      <Chip size="small" color={toBool(evolutionCanary.enabled) ? "warning" : "default"} label={toBool(evolutionCanary.enabled) ? "On" : "Off"} />
                      <Typography variant="caption" color="text.secondary">
                        Rollout: {num(evolutionCanary.rollout_percent, 0)}%
                      </Typography>
                    </Stack>
                    <Typography variant="body2">
                      Baseline: {str(evolutionCanary.baseline_version, "routing-policy-default-v1")}
                    </Typography>
                    <Typography variant="body2">
                      Candidate: {str(evolutionCanary.candidate_version, "-")}
                    </Typography>
                    <Typography variant="body2">
                      Last promotion result: {str(evolution.last_promotion_result, "No evolution runs yet")}
                    </Typography>
                    <Typography variant="body2">
                      Promotion mode: {str(evolution.promotion_mode, "none")}
                    </Typography>
                    <Typography variant="body2">
                      Replay gate: {str(evolution.replay_gate_result, "-")}
                    </Typography>
                  </Stack>
                )}
              </Box>
            </Grid2>
            <Grid2 size={{ xs: 12, lg: 6 }}>
              <Box className="list-shell" sx={{ minHeight: 0 }}>
                <Typography variant="h6" mb={1}>Deploy Guard Default</Typography>
                <Typography variant="body2" color="text.secondary" mb={1}>
                  Default remains OFF unless changed. This controls app deploy default when a request does not specify `access_guard`.
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
                    <Typography variant="h6">Developer Controls</Typography>
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        onClick={async () => {
                          setError(null);
                          setSuccess(null);
                          try {
                            await runEvolutionDevActionMutation.mutateAsync("disable_canary");
                            setSuccess("Canary disabled.");
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                        disabled={runEvolutionDevActionMutation.isPending}
                      >
                        Disable Canary
                      </Button>
                      <Button
                        size="small"
                        variant="contained"
                        onClick={async () => {
                          const ok = window.confirm("Promote candidate policy to baseline now?");
                          if (!ok) return;
                          setError(null);
                          setSuccess(null);
                          try {
                            await runEvolutionDevActionMutation.mutateAsync("promote_candidate");
                            setSuccess("Candidate promoted to baseline.");
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                        disabled={runEvolutionDevActionMutation.isPending}
                      >
                        Promote Candidate
                      </Button>
                      <Button
                        size="small"
                        color="warning"
                        onClick={async () => {
                          const ok = window.confirm("Rollback baseline policy to stored snapshot?");
                          if (!ok) return;
                          setError(null);
                          setSuccess(null);
                          try {
                            await runEvolutionDevActionMutation.mutateAsync("rollback_baseline");
                            setSuccess("Rolled back to baseline snapshot.");
                          } catch (e) {
                            setError(errMessage(e));
                          }
                        }}
                        disabled={runEvolutionDevActionMutation.isPending}
                      >
                        Rollback Baseline
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>

              <Grid2 size={{ xs: 12, lg: 6 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Typography variant="h6" mb={1}>Policy Metrics</Typography>
                  {evolutionDevQ.isLoading ? (
                    <Typography variant="body2" color="text.secondary">Loading developer metrics...</Typography>
                  ) : evolutionDevQ.error ? (
                    <Alert severity="error">{errMessage(evolutionDevQ.error)}</Alert>
                  ) : evolutionPolicyMetrics.length === 0 ? (
                    <Typography variant="body2" color="text.secondary">No policy metrics yet.</Typography>
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
                          {evolutionPolicyMetrics.map((row, idx) => (
                            <TableRow key={`${str(row.version, "policy")}-${idx}`}>
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

              <Grid2 size={{ xs: 12, lg: 6 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Typography variant="h6" mb={1}>Strategy Metrics</Typography>
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
                  <Typography variant="h6" mb={1}>Lineage</Typography>
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
                              <TableCell sx={{ whiteSpace: "nowrap" }}>{str(row.timestamp_utc, "-")}</TableCell>
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
                    source: {str(selectedSecurityLog.source, "-")} | created_at: {str(selectedSecurityLog.created_at, "-")} | count: {str(selectedSecurityLog.count, "-")}
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
                  const fix = str(fr.fix_command, "-");
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
                            Recommended fix command
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
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>
          {moltbookActionLabel(str(selectedMoltbookEvent?.action, ""), asRecord(selectedMoltbookEvent?.details))}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Typography variant="caption" color="text.secondary">
              {str(selectedMoltbookEvent?.timestamp)} | Level: {str(selectedMoltbookEvent?.level)} | Run:{" "}
              {str(selectedMoltbookEvent?.run_id, "-")}
            </Typography>
            {(() => {
              const details = asRecord(selectedMoltbookEvent?.details);
              const action = str(selectedMoltbookEvent?.action, "");
              const reason = moltbookReason(action, details);
              const trigger = str(details.trigger, "");
              const apiUrl = str(details.api_url, "");
              const postApiUrl = str(details.post_api_url, "");
              const extraUrls = [apiUrl, postApiUrl].filter((u) => !!u.trim());
              return (
                <Stack spacing={0.75}>
                  {trigger ? (
                    <Typography variant="body2" color="text.secondary">
                      Trigger: {moltbookTriggerLabel(trigger)}
                    </Typography>
                  ) : null}
                  {reason ? <Alert severity="info">Reason: {reason}</Alert> : null}
                  {extraUrls.length ? (
                    <Box className="metadata-box">
                      <Typography variant="caption" color="text.secondary">
                        URLs
                      </Typography>
                      <Stack spacing={0.4} sx={{ mt: 0.6 }}>
                        {extraUrls.map((u) => (
                          <Typography key={u} variant="body2" sx={{ wordBreak: "break-all" }}>
                            <a href={u} target="_blank" rel="noreferrer" style={{ color: "inherit" }}>
                              {u}
                            </a>
                          </Typography>
                        ))}
                      </Stack>
                    </Box>
                  ) : null}
                </Stack>
              );
            })()}
            <Divider />
            <KeyValuePanel
              title="Details"
              data={asRecord(selectedMoltbookEvent?.details)}
              emptyLabel="No extra details."
              maxRows={18}
            />
          </Stack>
        </DialogContent>
      </Dialog>
      <Dialog
        open={vaultEditorOpen}
        onClose={closeVaultEditor}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>{vaultEditorMode === "edit" ? "Edit Secret" : "Add New Secret"}</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <TextField
              label="Secret key"
              value={vaultEditorKey}
              onChange={(e) => setVaultEditorKey(e.target.value)}
              fullWidth
              size="small"
              disabled={vaultEditorMode === "edit"}
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
              placeholder={vaultEditorMode === "edit" ? "Enter new value" : "Paste secret value"}
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
            {upsertVaultSecretMutation.isPending
              ? "Saving..."
              : vaultEditorMode === "edit"
                ? "Update Secret"
                : "Save Secret"}
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
              Saving this will restart the server and reconnect active sessions.
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
  type AnalyticsRange = "24h" | "30d" | "90d" | "custom";
  type BreakdownView = "model" | "channel" | "purpose";

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

  const [activeRange, setActiveRange] = useState<AnalyticsRange>("24h");
  const [breakdownView, setBreakdownView] = useState<BreakdownView>("model");
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
  const customBucket = useMemo(() => {
    if (!appliedFromDate || !appliedToDate) return "day";
    const diffMs = appliedToDate.getTime() - appliedFromDate.getTime();
    const diffHours = diffMs / (1000 * 60 * 60);
    if (diffHours <= 72) return "hour";
    if (diffHours <= 24 * 120) return "day";
    return "week";
  }, [appliedFromDate, appliedToDate]);

  const hourQ = useQuery({
    queryKey: ["llm-analytics", "24h", "hour"],
    queryFn: () => api.getLlmAnalytics({ range: "24h", bucket: "hour" }),
    refetchInterval: autoRefresh ? 30000 : false
  });
  const dayQ = useQuery({
    queryKey: ["llm-analytics", "30d", "day"],
    queryFn: () => api.getLlmAnalytics({ range: "30d", bucket: "day" }),
    refetchInterval: autoRefresh ? 120000 : false
  });
  const weekQ = useQuery({
    queryKey: ["llm-analytics", "90d", "week"],
    queryFn: () => api.getLlmAnalytics({ range: "90d", bucket: "week" }),
    refetchInterval: autoRefresh ? 300000 : false
  });
  const customQ = useQuery({
    queryKey: ["llm-analytics", "custom", appliedCustomFrom, appliedCustomTo, customBucket],
    queryFn: async () => {
      if (!appliedFromDate || !appliedToDate) {
        throw new Error("Custom date range is invalid.");
      }
      return api.getLlmAnalytics({
        range: "24h",
        bucket: customBucket,
        from: appliedFromDate.toISOString(),
        to: appliedToDate.toISOString()
      });
    },
    enabled: activeRange === "custom" && Boolean(appliedFromDate && appliedToDate),
    refetchInterval: autoRefresh ? 120000 : false
  });

  const dataMap: Record<AnalyticsRange, LlmAnalyticsResponse | undefined> = {
    "24h": hourQ.data as LlmAnalyticsResponse | undefined,
    "30d": dayQ.data as LlmAnalyticsResponse | undefined,
    "90d": weekQ.data as LlmAnalyticsResponse | undefined,
    custom: customQ.data as LlmAnalyticsResponse | undefined
  };
  const errorMap: Record<AnalyticsRange, unknown> = {
    "24h": hourQ.error,
    "30d": dayQ.error,
    "90d": weekQ.error,
    custom: customQ.error
  };
  const resp = dataMap[activeRange];
  const activeError = errorMap[activeRange];
  const totals = resp?.totals;
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
      grid: { left: 6, right: 6, top: 8, bottom: 12, containLabel: false },
      tooltip: {
        trigger: "axis",
        backgroundColor: "rgba(6,14,28,0.95)",
        borderColor: "rgba(84,198,255,0.25)",
        textStyle: { color: "#d8edff" }
      },
      xAxis: {
        type: "category",
        data: seriesNames,
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false }
      },
      yAxis: {
        type: "value",
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
              borderRadius: [6, 6, 0, 0]
            }
          })),
          barWidth: "40%"
        }
      ]
    };
  }

  const spendValue = typeof totals?.cost_usd === "number" ? `$${totals.cost_usd.toFixed(4)}` : "n/a";
  const requestsValue = compactNumber(num(totals?.request_count, 0));
  const tokensValue = compactNumber(num(totals?.total_tokens, 0));

  return (
    <Stack spacing={1.5} sx={{ pb: 3 }}>
      <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "stretch", md: "center" }} spacing={1}>
        <Typography variant="h4" sx={{ fontWeight: 700, letterSpacing: -0.6, color: "#ecf5ff" }}>
          Activity
        </Typography>
        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
          <Button
            size="small"
            variant="outlined"
            startIcon={<FilterListRoundedIcon fontSize="small" />}
            sx={{ textTransform: "none" }}
          >
            Filters
          </Button>
          <TextField
            select
            size="small"
            value={activeRange}
            onChange={(e) => setActiveRange((e.target.value as AnalyticsRange) || "24h")}
            sx={{ minWidth: 120 }}
          >
            <MenuItem value="24h">24 h</MenuItem>
            <MenuItem value="30d">30 d</MenuItem>
            <MenuItem value="90d">90 d</MenuItem>
            <MenuItem value="custom">Custom</MenuItem>
          </TextField>
          <TextField
            select
            size="small"
            value={breakdownView}
            onChange={(e) => setBreakdownView((e.target.value as BreakdownView) || "model")}
            sx={{ minWidth: 140 }}
          >
            <MenuItem value="model">By Model</MenuItem>
            <MenuItem value="channel">By Channel</MenuItem>
            <MenuItem value="purpose">By Purpose</MenuItem>
          </TextField>
          <IconButton size="small" sx={{ border: "1px solid rgba(108,156,212,0.25)" }}>
            <SettingsRoundedIcon fontSize="small" />
          </IconButton>
        </Stack>
      </Stack>

      {activeRange === "custom" ? (
        <Stack
          direction={{ xs: "column", md: "row" }}
          spacing={1}
          alignItems={{ xs: "stretch", md: "center" }}
          className="list-shell"
          sx={{ p: 1.2 }}
        >
          <TextField
            size="small"
            label="From"
            type="datetime-local"
            value={customFrom}
            onChange={(e) => setCustomFrom(e.target.value)}
            InputLabelProps={{ shrink: true }}
            sx={{ minWidth: 220 }}
          />
          <TextField
            size="small"
            label="To"
            type="datetime-local"
            value={customTo}
            onChange={(e) => setCustomTo(e.target.value)}
            InputLabelProps={{ shrink: true }}
            sx={{ minWidth: 220 }}
            error={customRangeInvalid}
            helperText={customRangeInvalid ? "To must be later than From." : "Choose any custom date/time range."}
          />
          <Button
            variant="contained"
            onClick={() => {
              if (customRangeInvalid) return;
              setAppliedCustomFrom(customFrom);
              setAppliedCustomTo(customTo);
            }}
            disabled={customRangeInvalid || customQ.isFetching}
          >
            {customQ.isFetching ? "Applying..." : "Apply"}
          </Button>
        </Stack>
      ) : null}

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
            <ReactECharts option={miniBarsOption(card.values)} style={{ height: 120 }} />
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

      <Box className="list-shell">
        <Stack direction={{ xs: "column", md: "row" }} justifyContent="space-between" alignItems={{ xs: "flex-start", md: "center" }} spacing={1} sx={{ mb: 1 }}>
          <Typography variant="h6">
            {breakdownView === "model" ? "By Model" : breakdownView === "channel" ? "By Channel" : "By Purpose"}
          </Typography>
          <Typography variant="caption" color="text.secondary">
            Range: {str(asRecord(resp?.range).since, "-")} to {str(asRecord(resp?.range).until, "-")}
          </Typography>
        </Stack>
        {breakdownRows.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No analytics data yet for the selected range.
          </Typography>
        ) : (
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>{breakdownView === "model" ? "Model" : breakdownView === "channel" ? "Channel" : "Purpose"}</TableCell>
                  <TableCell align="right">Requests</TableCell>
                  <TableCell align="right">Tokens</TableCell>
                  <TableCell align="right">Cost</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {breakdownRows.slice(0, 30).map((row, idx) => {
                  const label =
                    breakdownView === "model"
                      ? `${str(row.provider, "-")} / ${str(row.model, "-")}`
                      : breakdownView === "channel"
                        ? str((row as Record<string, unknown>).channel, "-")
                        : str((row as Record<string, unknown>).purpose, "-");
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
    </Stack>
  );
}

export function NativeWorkspace({
  view,
  autoRefresh,
  showAdvanced
}: {
  view: WorkspaceView;
  autoRefresh: boolean;
  showAdvanced: boolean;
}) {
  const isChat = view === "chat";
  return (
    <Box
      sx={{
        p: 0.75,
        height: "calc(100vh - var(--appbar-height) - 8px)",
        overflow: isChat ? "hidden" : "auto",
        display: "flex",
        flexDirection: "column",
        minHeight: 0
      }}
    >
      {view === "chat" ? <ChatManager autoRefresh={autoRefresh} /> : null}
      {view === "tasks" ? <TasksManager autoRefresh={autoRefresh} /> : null}
      {view === "skills" ? <SkillsManager autoRefresh={autoRefresh} /> : null}
      {view === "apps" ? <AppsManager autoRefresh={autoRefresh} /> : null}
      {view === "goals" ? <GoalsManager autoRefresh={autoRefresh} /> : null}
      {view === "autonomy" ? <AutonomyManager autoRefresh={autoRefresh} /> : null}
      {view === "documents" ? <DocumentsManager autoRefresh={autoRefresh} /> : null}
      {view === "projects" ? <ProjectsManager autoRefresh={autoRefresh} /> : null}
      {view === "swarm" ? <SwarmManager autoRefresh={autoRefresh} /> : null}
      {view === "trace" ? <TraceManager autoRefresh={autoRefresh} /> : null}
      {view === "status" ? <StatusManager autoRefresh={autoRefresh} /> : null}
      {view === "analytics" ? <AnalyticsManager autoRefresh={autoRefresh} /> : null}
      {view === "settings" ? <SettingsManager autoRefresh={autoRefresh} /> : null}
      {["tasks", "skills", "apps"].includes(view) ? <Divider sx={{ mt: 2 }} /> : null}
    </Box>
  );
}

