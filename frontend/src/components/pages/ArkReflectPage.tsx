import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  IconButton,
  InputAdornment,
  LinearProgress,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import AutoGraphRoundedIcon from "@mui/icons-material/AutoGraphRounded";
import BubbleChartRoundedIcon from "@mui/icons-material/BubbleChartRounded";
import CalendarMonthRoundedIcon from "@mui/icons-material/CalendarMonthRounded";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import FitScreenRoundedIcon from "@mui/icons-material/FitScreenRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import KeyboardArrowLeftRoundedIcon from "@mui/icons-material/KeyboardArrowLeftRounded";
import KeyboardArrowRightRoundedIcon from "@mui/icons-material/KeyboardArrowRightRounded";
import MemoryRoundedIcon from "@mui/icons-material/MemoryRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import ThumbUpAltRoundedIcon from "@mui/icons-material/ThumbUpAltRounded";
import TaskAltRoundedIcon from "@mui/icons-material/TaskAltRounded";
import WorkHistoryRoundedIcon from "@mui/icons-material/WorkHistoryRounded";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import ReactECharts from "echarts-for-react";
import { api } from "../../api/client";
import {
  formatUiDateOnly,
  formatUiDateRange,
  formatUiDateTime,
} from "../../lib/dateFormat";
import { humanizeStatusLabel } from "../../lib/displayLabels";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { asRecord, errMessage, num, pickRecords, str } from "./pageHelpers";
import {
  followupHasSourceEvidence as reflectFollowupHasSourceEvidence,
  isOpportunitySettlementActive,
  isDisplayableOpportunity as reflectIsDisplayableOpportunity,
  latestDevelopmentSummary as reflectLatestDevelopmentSummary,
  latestUpdateTitle as reflectLatestUpdateTitle,
  shouldPollForOpportunitySettlement,
  shouldStartOpportunitySettlementPoll,
} from "./arkReflectOpportunityHelpers";

type ReflectPageProps = {
  autoRefresh: boolean;
  onNavigateToView?: (view: string, replace?: boolean) => void;
};

type ReflectPeriod = "daily" | "weekly" | "monthly";
type ReflectStoryTab = "overview" | "topics" | "latest" | "review";
type ReflectTopicSortMode = "ranked" | "recent";

type ReflectUnit = {
  id: string;
  source_kind: string;
  source_label: string;
  channel: string;
  title: string;
  summary: string;
  content_preview: string;
  occurred_at: string;
  message_count: number;
  has_embedding: boolean;
};

type ReflectRelatedUnit = {
  id: string;
  source_label: string;
  title: string;
  occurred_at: string;
  similarity: number;
};

type ReflectRelatedHistory = {
  mode: string;
  similar_count: number;
  most_recent_at: string;
  top_similarity: number | null;
  detail: string;
  items: ReflectRelatedUnit[];
};

type ReflectCluster = {
  id: string;
  label: string;
  plain_summary: string;
  unit_count: number;
  message_count: number;
  source_mix: Record<string, number>;
  color: string;
  related_history: ReflectRelatedHistory;
  units: ReflectUnit[];
};

type ReflectSourceCounts = {
  main_chat: number;
  orbit_chat: number;
  memory: number;
  procedures: number;
  apps: number;
  goals: number;
  watchers: number;
  sentinel: number;
  arkpulse: number;
  arkevolve: number;
  usage: number;
};

type ReflectSuggestedFollowup = {
  id: string;
  kind: string;
  title: string;
  detail: string;
  prompt: string;
  status: string;
  source_label: string;
  occurred_at: string;
  conversation_id?: string | null;
  source_unit_id?: string | null;
  rank_score: number;
  search_results: ReflectSearchResult[];
  search_checked_at?: string | null;
  search_error?: string | null;
  latest_summary?: string | null;
  latest_summary_generated_at?: string | null;
  latest_summary_error?: string | null;
  latest_summary_evidence_supported?: boolean | null;
  feedback?: ReflectFollowupFeedbackState | null;
  feedback_keys: string[];
};

type ReflectFollowupFeedbackState = {
  useful_count: number;
  dismiss_count: number;
  snooze_count: number;
  last_action?: string | null;
  last_at?: string | null;
  snoozed_until?: string | null;
  renewed_after_feedback: boolean;
};

type ReflectSearchResult = {
  title: string;
  url: string;
  snippet: string;
  source: string;
  published_date?: string | null;
};

type ChatPendingLaunch = {
  createdAt: number;
  launchMode: "message";
  message: string;
  conversationId?: string;
  newConversation?: boolean;
  source?: string;
};

const CHAT_PENDING_LAUNCH_STORAGE_KEY = "agentark.chat.pendingLaunch";
const OPPORTUNITY_PAGE_SIZE = 6;
const OPPORTUNITY_SETTLE_POLL_MS = 5000;
const OPPORTUNITY_SETTLE_WINDOW_MS = 90000;
const TOPIC_PAGE_SIZE = 6;

type ReflectResponse = {
  period: ReflectPeriod;
  from: string;
  to: string;
  generated_at: string;
  source_counts: ReflectSourceCounts;
  baseline_source_counts: ReflectSourceCounts;
  embedding_status: {
    mode: string;
    embedded_units: number;
    total_units: number;
    detail: string;
  };
  refresh_status: {
    running: boolean;
    status: string;
    trigger: string;
    requested_at: string;
    started_at: string;
    completed_at: string;
    last_error: string;
  };
  cache_status: {
    mode: string;
    cached_units: number;
    stale: boolean;
    detail: string;
  };
  daily_digest_status: {
    enabled: boolean;
    status: string;
    target_date: string;
    today_date: string;
    meaningful: boolean;
    unit_count: number;
    cluster_count: number;
    source_counts: ReflectSourceCounts;
    summary: string;
    detail: string;
    last_checked_at: string;
    last_sent_at: string;
    last_skipped_at: string;
    last_error: string;
  };
  suggested_followups: ReflectSuggestedFollowup[];
  clusters: ReflectCluster[];
  unclustered_units: ReflectUnit[];
};

type ReflectRefreshStartResponse = {
  accepted: boolean;
  running: boolean;
  status: string;
  detail: string;
  refresh_status: ReflectResponse["refresh_status"];
};

const PERIOD_OPTIONS: { value: ReflectPeriod; label: string }[] = [
  { value: "daily", label: "Day" },
  { value: "weekly", label: "Week" },
  { value: "monthly", label: "Month" },
];

const SOURCE_DISPLAY: Record<string, { label: string; group: string; color: string }> = {
  conversation: { label: "Chat", group: "Conversation work", color: "#B7BCC2" },
  orbit_chat: { label: "ArkOrbit", group: "Orbit conversations", color: "#AAB0B7" },
  experience_item: { label: "Memory", group: "What AgentArk learned", color: "#9EA5AC" },
  procedural_pattern: { label: "Workflows", group: "Working patterns", color: "#C1C5CA" },
  app: { label: "Apps", group: "Apps built", color: "#949BA3" },
  goal: { label: "Goals", group: "Goals and progress", color: "#A5ABB2" },
  watcher: { label: "Watchers", group: "Background watchers", color: "#868E96" },
  sentinel: { label: "Sentinel", group: "Safety and checks", color: "#9AA1A9" },
  arkpulse: { label: "Pulse", group: "System health", color: "#AEB4BB" },
  arkevolve: { label: "Evolve", group: "Agent improvements", color: "#A1A8AF" },
  llm_usage: { label: "Usage", group: "Agent usage", color: "#C9CDD2" },
};

const SOURCE_ORDER = [
  "conversation",
  "orbit_chat",
  "experience_item",
  "procedural_pattern",
  "app",
  "goal",
  "watcher",
  "sentinel",
  "arkpulse",
  "arkevolve",
  "llm_usage",
] as const;

const USER_FACING_SOURCE_KINDS = new Set([
  "conversation",
  "orbit_chat",
  "experience_item",
  "procedural_pattern",
  "app",
  "goal",
]);

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function toDateInputValue(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function parseDateInput(value: string): Date {
  const [yearRaw, monthRaw, dayRaw] = value.split("-").map((part) => Number(part));
  const year = Number.isFinite(yearRaw) ? yearRaw : new Date().getFullYear();
  const month = Number.isFinite(monthRaw) ? monthRaw - 1 : new Date().getMonth();
  const day = Number.isFinite(dayRaw) ? dayRaw : new Date().getDate();
  return new Date(year, month, day);
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}

function startOfLocalDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function periodBounds(period: ReflectPeriod, anchorValue: string): { from: Date; to: Date } {
  const anchor = startOfLocalDay(parseDateInput(anchorValue));
  if (period === "daily") {
    return { from: anchor, to: addDays(anchor, 1) };
  }
  if (period === "monthly") {
    return {
      from: new Date(anchor.getFullYear(), anchor.getMonth(), 1),
      to: new Date(anchor.getFullYear(), anchor.getMonth() + 1, 1),
    };
  }
  const dayOffset = (anchor.getDay() + 6) % 7;
  const from = addDays(anchor, -dayOffset);
  return { from, to: addDays(from, 7) };
}

function asReflectUnit(value: unknown): ReflectUnit | null {
  const raw = asRecord(value);
  const id = str(raw.id, "");
  if (!id) return null;
  return {
    id,
    source_kind: str(raw.source_kind, "work"),
    source_label: str(raw.source_label, "Work"),
    channel: str(raw.channel, ""),
    title: str(raw.title, "Untitled work"),
    summary: str(raw.summary, ""),
    content_preview: str(raw.content_preview, ""),
    occurred_at: str(raw.occurred_at, ""),
    message_count: num(raw.message_count, 0),
    has_embedding: Boolean(raw.has_embedding),
  };
}

function asRelatedUnit(value: unknown): ReflectRelatedUnit | null {
  const raw = asRecord(value);
  const id = str(raw.id, "");
  if (!id) return null;
  return {
    id,
    source_label: str(raw.source_label, "Work"),
    title: str(raw.title, "Related work"),
    occurred_at: str(raw.occurred_at, ""),
    similarity: num(raw.similarity, 0),
  };
}

function asRelatedHistory(value: unknown): ReflectRelatedHistory {
  const raw = asRecord(value);
  const topSimilarityRaw = raw.top_similarity;
  const topSimilarity = typeof topSimilarityRaw === "number" && Number.isFinite(topSimilarityRaw)
    ? topSimilarityRaw
    : null;
  return {
    mode: str(raw.mode, "unavailable"),
    similar_count: num(raw.similar_count, 0),
    most_recent_at: str(raw.most_recent_at, ""),
    top_similarity: topSimilarity,
    detail: str(raw.detail, ""),
    items: pickRecords(raw, "items")
      .map(asRelatedUnit)
      .filter((item): item is ReflectRelatedUnit => item !== null),
  };
}

function asSearchResult(value: unknown): ReflectSearchResult | null {
  const raw = asRecord(value);
  const title = str(raw.title, "").trim();
  const url = str(raw.url, "").trim();
  if (!title && !url) return null;
  return {
    title: title || url,
    url,
    snippet: str(raw.snippet, ""),
    source: str(raw.source, "Search"),
    published_date: str(raw.published_date, "") || null,
  };
}

function asSuggestedFollowup(value: unknown): ReflectSuggestedFollowup | null {
  const raw = asRecord(value);
  const id = str(raw.id, "");
  if (!id) return null;
  return {
    id,
    kind: str(raw.kind, "followup"),
    title: str(raw.title, "Suggested follow-up"),
    detail: str(raw.detail, ""),
    prompt: str(raw.prompt, ""),
    status: str(raw.status, "ready"),
    source_label: str(raw.source_label, "Reflect"),
    occurred_at: str(raw.occurred_at, ""),
    conversation_id: str(raw.conversation_id, "") || null,
    source_unit_id: str(raw.source_unit_id, "") || null,
    rank_score: num(raw.rank_score, 0),
    search_results: pickRecords(raw, "search_results")
      .map(asSearchResult)
      .filter((item): item is ReflectSearchResult => item !== null),
    search_checked_at: str(raw.search_checked_at, "") || null,
    search_error: str(raw.search_error, "") || null,
    latest_summary: str(raw.latest_summary, "") || null,
    latest_summary_generated_at: str(raw.latest_summary_generated_at, "") || null,
    latest_summary_error: str(raw.latest_summary_error, "") || null,
    latest_summary_evidence_supported:
      typeof raw.latest_summary_evidence_supported === "boolean"
        ? raw.latest_summary_evidence_supported
        : null,
    feedback: asFollowupFeedback(raw.feedback),
    feedback_keys: Array.isArray(raw.feedback_keys)
      ? raw.feedback_keys.map((key) => String(key).trim()).filter(Boolean)
      : [],
  };
}

function asFollowupFeedback(value: unknown): ReflectFollowupFeedbackState | null {
  const raw = asRecord(value);
  if (!Object.keys(raw).length) return null;
  return {
    useful_count: num(raw.useful_count, 0),
    dismiss_count: num(raw.dismiss_count, 0),
    snooze_count: num(raw.snooze_count, 0),
    last_action: str(raw.last_action, "") || null,
    last_at: str(raw.last_at, "") || null,
    snoozed_until: str(raw.snoozed_until, "") || null,
    renewed_after_feedback: Boolean(raw.renewed_after_feedback),
  };
}

function asReflectCluster(value: unknown): ReflectCluster | null {
  const raw = asRecord(value);
  const id = str(raw.id, "");
  if (!id) return null;
  const sourceMixRaw = asRecord(raw.source_mix);
  const source_mix = Object.fromEntries(
    Object.entries(sourceMixRaw).map(([key, value]) => [key, num(value, 0)]),
  );
  return {
    id,
    label: str(raw.label, "Related work"),
    plain_summary: str(raw.plain_summary, ""),
    unit_count: num(raw.unit_count, 0),
    message_count: num(raw.message_count, 0),
    source_mix,
    color: str(raw.color, "#AEB4BB"),
    related_history: asRelatedHistory(raw.related_history),
    units: pickRecords(raw, "units")
      .map(asReflectUnit)
      .filter((unit): unit is ReflectUnit => unit !== null),
  };
}

function parseSourceCounts(value: unknown): ReflectSourceCounts {
  const sourceCounts = asRecord(value);
  return {
    main_chat: num(sourceCounts.main_chat, 0),
    orbit_chat: num(sourceCounts.orbit_chat, 0),
    memory: num(sourceCounts.memory, 0),
    procedures: num(sourceCounts.procedures, 0),
    apps: num(sourceCounts.apps, 0),
    goals: num(sourceCounts.goals, 0),
    watchers: num(sourceCounts.watchers, 0),
    sentinel: num(sourceCounts.sentinel, 0),
    arkpulse: num(sourceCounts.arkpulse, 0),
    arkevolve: num(sourceCounts.arkevolve, 0),
    usage: num(sourceCounts.usage, 0),
  };
}

function parseReflectResponse(value: unknown, period: ReflectPeriod): ReflectResponse {
  const raw = asRecord(value);
  const embedding = asRecord(raw.embedding_status);
  const digest = asRecord(raw.daily_digest_status);
  return {
    period,
    from: str(raw.from, ""),
    to: str(raw.to, ""),
    generated_at: str(raw.generated_at, ""),
    source_counts: parseSourceCounts(raw.source_counts),
    baseline_source_counts: parseSourceCounts(raw.baseline_source_counts),
    embedding_status: {
      mode: str(embedding.mode, "activity"),
      embedded_units: num(embedding.embedded_units, 0),
      total_units: num(embedding.total_units, 0),
      detail: str(embedding.detail, ""),
    },
    refresh_status: {
      running: Boolean(asRecord(raw.refresh_status).running),
      status: str(asRecord(raw.refresh_status).status, "idle"),
      trigger: str(asRecord(raw.refresh_status).trigger, ""),
      requested_at: str(asRecord(raw.refresh_status).requested_at, ""),
      started_at: str(asRecord(raw.refresh_status).started_at, ""),
      completed_at: str(asRecord(raw.refresh_status).completed_at, ""),
      last_error: str(asRecord(raw.refresh_status).last_error, ""),
    },
    cache_status: {
      mode: str(asRecord(raw.cache_status).mode, "empty"),
      cached_units: num(asRecord(raw.cache_status).cached_units, 0),
      stale: Boolean(asRecord(raw.cache_status).stale),
      detail: str(asRecord(raw.cache_status).detail, ""),
    },
    daily_digest_status: {
      enabled: Boolean(digest.enabled),
      status: str(digest.status, "disabled"),
      target_date: str(digest.target_date, ""),
      today_date: str(digest.today_date, ""),
      meaningful: Boolean(digest.meaningful),
      unit_count: num(digest.unit_count, 0),
      cluster_count: num(digest.cluster_count, 0),
      source_counts: parseSourceCounts(digest.source_counts),
      summary: str(digest.summary, ""),
      detail: str(digest.detail, ""),
      last_checked_at: str(digest.last_checked_at, ""),
      last_sent_at: str(digest.last_sent_at, ""),
      last_skipped_at: str(digest.last_skipped_at, ""),
      last_error: str(digest.last_error, ""),
    },
    suggested_followups: pickRecords(raw, "suggested_followups")
      .map(asSuggestedFollowup)
      .filter((item): item is ReflectSuggestedFollowup => item !== null),
    clusters: pickRecords(raw, "clusters")
      .map(asReflectCluster)
      .filter((cluster): cluster is ReflectCluster => cluster !== null),
    unclustered_units: pickRecords(raw, "unclustered_units")
      .map(asReflectUnit)
      .filter((unit): unit is ReflectUnit => unit !== null),
  };
}

function parseReflectRefreshStartResponse(value: unknown): ReflectRefreshStartResponse {
  const raw = asRecord(value);
  const refresh = asRecord(raw.refresh_status);
  return {
    accepted: Boolean(raw.accepted),
    running: Boolean(raw.running),
    status: str(raw.status, ""),
    detail: str(raw.detail, ""),
    refresh_status: {
      running: Boolean(refresh.running),
      status: str(refresh.status, "idle"),
      trigger: str(refresh.trigger, ""),
      requested_at: str(refresh.requested_at, ""),
      started_at: str(refresh.started_at, ""),
      completed_at: str(refresh.completed_at, ""),
      last_error: str(refresh.last_error, ""),
    },
  };
}

function sourceIcon(label: string) {
  const lower = label.toLowerCase();
  if (lower.includes("orbit")) return <HubRoundedIcon fontSize="small" />;
  if (lower.includes("memory")) return <MemoryRoundedIcon fontSize="small" />;
  return <ChatRoundedIcon fontSize="small" />;
}

function relatedHistoryText(history: ReflectRelatedHistory): string {
  if (history.mode === "recurring") {
    const when = history.most_recent_at
      ? `, most recently ${formatUiDateOnly(history.most_recent_at, { fallback: history.most_recent_at })}`
      : "";
    return `Similar work appeared ${history.similar_count} time${history.similar_count === 1 ? "" : "s"} before${when}.`;
  }
  if (history.mode === "new") return "No close match found in reflection history.";
  return "History comparison appears when enough cached data exists.";
}

function unitDisplayTitle(unit: ReflectUnit): string {
  const title = unit.title.trim();
  const meta = sourceMeta(unit.source_kind);
  if (unit.source_kind === "llm_usage") return "Usage summary";
  if (
    (unit.source_kind === "sentinel" || unit.source_kind === "arkpulse" || unit.source_kind === "watcher") &&
    /[.:;]/.test(title)
  ) {
    return meta.group;
  }
  if (title.length < 8) return meta.group;
  return title;
}

type StyleSignal = {
  key: string;
  label: string;
  current: number;
  baseline: number;
  delta: number;
};

function styleBuckets(counts: ReflectSourceCounts | undefined): Record<string, number> {
  return {
    Conversations:
      countForSourceCounts(counts, "conversation") + countForSourceCounts(counts, "orbit_chat"),
    Building: countForSourceCounts(counts, "app") + countForSourceCounts(counts, "goal"),
    Memory:
      countForSourceCounts(counts, "experience_item") +
      countForSourceCounts(counts, "procedural_pattern"),
    Background:
      countForSourceCounts(counts, "watcher") +
      countForSourceCounts(counts, "sentinel") +
      countForSourceCounts(counts, "arkpulse") +
      countForSourceCounts(counts, "arkevolve"),
    Usage: countForSourceCounts(counts, "llm_usage"),
  };
}

function workingStyleSignals(response: ReflectResponse | undefined): StyleSignal[] {
  const current = styleBuckets(response?.source_counts);
  const baseline = styleBuckets(response?.baseline_source_counts);
  const currentTotal = Object.values(current).reduce((sum, value) => sum + value, 0);
  const baselineTotal = Object.values(baseline).reduce((sum, value) => sum + value, 0);
  return Object.keys(current).map((key) => {
    const currentShare = currentTotal > 0 ? current[key] / currentTotal : 0;
    const baselineShare = baselineTotal > 0 ? baseline[key] / baselineTotal : 1 / Object.keys(current).length;
    const delta = currentShare - baselineShare;
    return {
      key,
      label: key,
      current: currentShare,
      baseline: baselineShare,
      delta,
    };
  });
}

function narrativeLines(
  response: ReflectResponse | undefined,
  focusLabel: string,
  totalUnits: number,
  learnedCount: number,
  backgroundCount: number,
  recurringCount: number,
): string[] {
  if (!response || totalUnits === 0) {
    return [
      "I do not have enough cached activity for this range yet.",
      "When the background refresh finishes, I will summarize the main focus areas, working style, background activity, and recurring themes here.",
    ];
  }
  const style = workingStyleSignals(response)
    .slice()
    .sort((left, right) => Math.abs(right.delta) - Math.abs(left.delta))[0];
  const styleText =
    style && Math.abs(style.delta) > 0.08
      ? `${style.label.toLowerCase()} stood out compared with your recent baseline`
      : "your activity stayed close to your recent baseline";
  const hasUserFacingFocus =
    focusLabel !== "No activity yet" && focusLabel !== "No clear user-facing focus yet";
  return [
    hasUserFacingFocus
      ? `Reflect grouped ${totalUnits} reflected item${totalUnits === 1 ? "" : "s"} in this range. The clearest user-facing thread is ${focusLabel.toLowerCase()}.`
      : `Reflect grouped ${totalUnits} reflected item${totalUnits === 1 ? "" : "s"} in this range, but the strongest signals are background or not actionable enough to promote.`,
    `${styleText}.`,
    `AgentArk also captured ${learnedCount} learned signal${learnedCount === 1 ? "" : "s"} and ${backgroundCount} background event${backgroundCount === 1 ? "" : "s"}.`,
    recurringCount > 0
      ? `${recurringCount} theme${recurringCount === 1 ? "" : "s"} connected back to earlier work.`
      : "Most visible themes look new for this cached history window.",
  ];
}

function countForSource(response: ReflectResponse | undefined, source: string): number {
  if (!response) return 0;
  return countForSourceCounts(response.source_counts, source);
}

function countForSourceCounts(counts: ReflectSourceCounts | undefined, source: string): number {
  if (!counts) return 0;
  switch (source) {
    case "conversation":
      return counts.main_chat;
    case "orbit_chat":
      return counts.orbit_chat;
    case "experience_item":
      return counts.memory;
    case "procedural_pattern":
      return counts.procedures;
    case "app":
      return counts.apps;
    case "goal":
      return counts.goals;
    case "watcher":
      return counts.watchers;
    case "sentinel":
      return counts.sentinel;
    case "arkpulse":
      return counts.arkpulse;
    case "arkevolve":
      return counts.arkevolve;
    case "llm_usage":
      return counts.usage;
    default:
      return 0;
  }
}

function totalForSourceCounts(counts: ReflectSourceCounts | undefined): number {
  if (!counts) return 0;
  return SOURCE_ORDER.reduce((sum, source) => sum + countForSourceCounts(counts, source), 0);
}

function meaningfulForSourceCounts(counts: ReflectSourceCounts | undefined): number {
  return Math.max(0, totalForSourceCounts(counts) - countForSourceCounts(counts, "llm_usage"));
}

function sourceMeta(source: string) {
  return SOURCE_DISPLAY[source] ?? { label: "Work", group: "Mixed work", color: "#C8D8C9" };
}

function dominantSource(cluster: ReflectCluster): string {
  const counts = new Map<string, number>();
  for (const unit of cluster.units) {
    counts.set(unit.source_kind, (counts.get(unit.source_kind) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "work";
}

function isUserFacingSource(source: string): boolean {
  return USER_FACING_SOURCE_KINDS.has(source);
}

function clusterHasUserFacingSignal(cluster: ReflectCluster): boolean {
  return cluster.units.some((unit) => isUserFacingSource(unit.source_kind));
}

function followupHasSourceEvidence(item: ReflectSuggestedFollowup): boolean {
  return reflectFollowupHasSourceEvidence(item);
}

function isDisplayableOpportunity(item: ReflectSuggestedFollowup): boolean {
  return reflectIsDisplayableOpportunity(item);
}

function isReviewThreadFollowup(item: ReflectSuggestedFollowup): boolean {
  if (item.kind === "recovery_advice") return true;
  return false;
}

function hexToHsl(hex: string): { h: number; s: number; l: number } | null {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  const r = ((n >> 16) & 0xff) / 255;
  const g = ((n >> 8) & 0xff) / 255;
  const b = (n & 0xff) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return { h: 0, s: 0, l };
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
  else if (max === g) h = ((b - r) / d + 2) / 6;
  else h = ((r - g) / d + 4) / 6;
  return { h, s, l };
}

function hslToHex(h: number, s: number, l: number): string {
  const hue2rgb = (p: number, q: number, t: number) => {
    let tt = t;
    if (tt < 0) tt += 1;
    if (tt > 1) tt -= 1;
    if (tt < 1 / 6) return p + (q - p) * 6 * tt;
    if (tt < 1 / 2) return q;
    if (tt < 2 / 3) return p + (q - p) * (2 / 3 - tt) * 6;
    return p;
  };
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const r = Math.round(hue2rgb(p, q, h + 1 / 3) * 255);
  const g = Math.round(hue2rgb(p, q, h) * 255);
  const b = Math.round(hue2rgb(p, q, h - 1 / 3) * 255);
  const toHex = (x: number) => x.toString(16).padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

function tacticalAccent(hex: string): string {
  const hsl = hexToHsl(hex);
  if (!hsl) return "#AEB4BB";
  return hslToHex(0, 0, Math.min(0.82, Math.max(0.42, hsl.l * 0.9 + 0.14)));
}

function tacticalSymbol(source: string): string {
  const HEXAGON = "path://M50,3 L93,26 L93,74 L50,97 L7,74 L7,26 Z";
  const DIAMOND = "path://M50,3 L97,50 L50,97 L3,50 Z";
  const TRIANGLE = "path://M50,6 L94,88 L6,88 Z";
  const SQUARE = "path://M10,10 L90,10 L90,90 L10,90 Z";
  if (source === "conversation" || source === "orbit_chat") return HEXAGON;
  if (source === "watcher" || source === "sentinel" || source === "arkpulse") return DIAMOND;
  if (source === "experience_item" || source === "procedural_pattern") return TRIANGLE;
  if (source === "app" || source === "goal" || source === "arkevolve") return SQUARE;
  return HEXAGON;
}

function tacticalCode(source: string): string {
  const map: Record<string, string> = {
    conversation: "CHT",
    orbit_chat: "ORB",
    experience_item: "MEM",
    procedural_pattern: "PRC",
    app: "APP",
    goal: "GOL",
    watcher: "WCH",
    sentinel: "SNT",
    arkpulse: "PLS",
    arkevolve: "EVO",
    llm_usage: "USG",
  };
  return map[source] ?? "WRK";
}

function hexToRgba(hex: string, opacity: number): string {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) return `rgba(174, 180, 187, ${opacity})`;
  const n = parseInt(m[1], 16);
  const r = (n >> 16) & 0xff;
  const g = (n >> 8) & 0xff;
  const b = n & 0xff;
  return `rgba(${r}, ${g}, ${b}, ${opacity})`;
}

function clusterMostRecentTimestamp(cluster: ReflectCluster): number {
  const unitTimes = cluster.units
    .map((unit) => Date.parse(unit.occurred_at))
    .filter(Number.isFinite);
  const historyTime = Date.parse(cluster.related_history.most_recent_at);
  if (Number.isFinite(historyTime)) unitTimes.push(historyTime);
  return unitTimes.length > 0 ? Math.max(...unitTimes) : 0;
}

function topicStrength(cluster: ReflectCluster, maxUnits: number): { label: "High" | "Medium" | "Low"; score: number } {
  const volumeScore = maxUnits > 0 ? cluster.unit_count / maxUnits : 0;
  const recurrenceScore =
    cluster.related_history.mode === "recurring"
      ? Math.min(0.24, cluster.related_history.similar_count / Math.max(12, cluster.unit_count * 2))
      : 0;
  const score = Math.min(1, volumeScore * 0.78 + recurrenceScore);
  if (score >= 0.64) return { label: "High", score };
  if (score >= 0.34) return { label: "Medium", score };
  return { label: "Low", score };
}

function topicStrengthColor(label: "High" | "Medium" | "Low"): string {
  if (label === "High") return "#D2D6DB";
  if (label === "Medium") return "#AEB4BB";
  return "#858D96";
}

function clusterDisplayLabel(cluster: ReflectCluster): string {
  const explicit = cluster.label?.trim();
  if (explicit) return explicit;
  const sourceKinds = new Set(cluster.units.map((unit) => unit.source_kind));
  if (sourceKinds.size === 1) return sourceMeta(dominantSource(cluster)).group;
  if (sourceKinds.has("conversation") || sourceKinds.has("orbit_chat")) return "Conversation-led work";
  if (sourceKinds.has("watcher") || sourceKinds.has("sentinel") || sourceKinds.has("arkpulse")) {
    return "Background operations";
  }
  return "Mixed AgentArk activity";
}

function clusterDistinguishingHint(cluster: ReflectCluster): string {
  const firstUnit = cluster.units[0];
  const title = firstUnit?.title?.trim() ?? "";
  if (title) {
    const words = title.split(/\s+/).slice(0, 4).join(" ");
    return words.length > 32 ? `${words.slice(0, 29)}...` : words;
  }
  return cluster.id.slice(0, 6);
}

function buildClusterLabelMap(clusters: ReflectCluster[]): Record<string, string> {
  const counts = new Map<string, number>();
  for (const cluster of clusters) {
    const primary = clusterDisplayLabel(cluster);
    counts.set(primary, (counts.get(primary) ?? 0) + 1);
  }
  const result: Record<string, string> = {};
  for (const cluster of clusters) {
    const primary = clusterDisplayLabel(cluster);
    const collision = (counts.get(primary) ?? 1) > 1;
    if (!collision) {
      result[cluster.id] = primary;
      continue;
    }
    const hint = clusterDistinguishingHint(cluster);
    result[cluster.id] = hint ? `${primary}: ${hint}` : primary;
  }
  return result;
}

function digestStatusTitle(response: ReflectResponse | undefined): string {
  const digest = response?.daily_digest_status;
  if (!digest || !digest.enabled) return "Daily digest is off";
  const appliesToToday = !digest.target_date || digest.target_date === digest.today_date;
  if (!appliesToToday) {
    const meaningful = meaningfulForSourceCounts(response?.source_counts);
    return meaningful > 0 ? "Today has activity to reflect" : "Waiting for today's activity";
  }
  if (digest.status === "sent") return "Daily digest sent";
  if (digest.status === "preparing") return "Preparing today's digest";
  if (digest.status === "skipped_quiet") return "No digest sent for a quiet day";
  if (digest.status === "delivery_failed") return "Digest delivery needs attention";
  const meaningful = meaningfulForSourceCounts(response?.source_counts);
  if (meaningful > 0) return "Today has activity to reflect";
  return "Waiting for meaningful activity";
}

function digestStatusDetail(response: ReflectResponse | undefined, fetching: boolean): string {
  if (!response) {
    return fetching
      ? "Loading today's Reflect status."
      : "Today status appears here after Reflect has cached activity.";
  }
  const digest = response.daily_digest_status;
  const total = totalForSourceCounts(response.source_counts);
  const meaningful = meaningfulForSourceCounts(response.source_counts);
  const appliesToToday = !digest.target_date || digest.target_date === digest.today_date;
  if (!digest.enabled) {
    return total > 0
      ? `${meaningful} meaningful signal${meaningful === 1 ? "" : "s"} cached today. Enable the daily digest in Settings to send recaps.`
      : "Enable the digest in Settings if you want meaningful days sent to your notification channel.";
  }
  if (!appliesToToday) {
    return total > 0
      ? `${meaningful} meaningful signal${meaningful === 1 ? "" : "s"} cached today; today's digest will wait for a quiet end-of-day window.`
      : "No meaningful activity has been cached for today yet.";
  }
  if (digest.status === "sent" && digest.last_sent_at) {
    return `Sent for ${digest.target_date || "the selected day"} at ${formatUiDateTime(digest.last_sent_at)}.`;
  }
  if (digest.status === "skipped_quiet") {
    return "Reflect checked the day and found nothing worth notifying you about.";
  }
  if (digest.status === "preparing") {
    return "AgentArk is refreshing the daily work units in the background.";
  }
  if (digest.status === "delivery_failed") {
    return digest.last_error || "The digest was prepared, but no notification channel accepted it.";
  }
  return total > 0
    ? `${meaningful} meaningful signal${meaningful === 1 ? "" : "s"} cached today; the digest waits for a quiet end-of-day window.`
    : "No meaningful activity has been cached for today yet.";
}

function clusterPlainSummary(cluster: ReflectCluster): string {
  const sources = [...new Set(cluster.units.map((unit) => sourceMeta(unit.source_kind).label))];
  const sourceText = sources.slice(0, 3).join(", ");
  return `${cluster.unit_count} item${cluster.unit_count === 1 ? "" : "s"} from ${sourceText || "AgentArk"}.`;
}

function compactText(value: string, maxChars: number): string {
  const cleaned = value.split(/\s+/).join(" ").trim();
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function stripInlineMarkup(value: string): string {
  return value
    .replace(/\*\*/g, "")
    .replace(/#+\s*/g, "")
    .replace(/`/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function compactMultilineText(value: string, maxChars: number): string {
  const cleaned = value
    .replace(/\*\*/g, "")
    .replace(/#+\s*/g, "")
    .replace(/`/g, "")
    .split(/\r?\n/)
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .join("\n");
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function meaningTokens(value: string): string[] {
  return stripInlineMarkup(value)
    .toLowerCase()
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/[^a-z0-9]+/g, " ")
    .split(/\s+/)
    .filter((token) => token.length >= 3);
}

function meaningSimilarity(left: string, right: string): number {
  const leftTokens = new Set(meaningTokens(left));
  const rightTokens = new Set(meaningTokens(right));
  if (!leftTokens.size || !rightTokens.size) return 0;
  let overlap = 0;
  for (const token of leftTokens) {
    if (rightTokens.has(token)) overlap += 1;
  }
  const union = new Set([...leftTokens, ...rightTokens]).size;
  return union > 0 ? overlap / union : 0;
}

function isNearDuplicateText(left: string, right: string): boolean {
  const leftClean = stripInlineMarkup(left).toLowerCase();
  const rightClean = stripInlineMarkup(right).toLowerCase();
  if (!leftClean || !rightClean) return false;
  return leftClean === rightClean || meaningSimilarity(leftClean, rightClean) >= 0.72;
}

function firstNonDuplicateText(values: string[], reference: string, maxChars: number): string {
  for (const value of values) {
    const cleaned = stripInlineMarkup(value);
    if (!cleaned || isNearDuplicateText(cleaned, reference)) continue;
    return compactText(cleaned, maxChars);
  }
  return "";
}

function uniqueByVisibleMeaning<T>(items: T[], getText: (item: T) => string): T[] {
  const selected: T[] = [];
  for (const item of items) {
    const text = getText(item);
    if (!text.trim()) continue;
    if (selected.some((existing) => isNearDuplicateText(getText(existing), text))) continue;
    selected.push(item);
  }
  return selected;
}

function clusterTopicTitle(cluster: ReflectCluster): string {
  const raw = stripInlineMarkup(cluster.label || clusterDisplayLabel(cluster));
  const tokens = meaningTokens(raw);
  const source = dominantSource(cluster);
  if (
    tokens.length > 9 ||
    raw.length > 86 ||
    ((source === "sentinel" || source === "arkpulse" || source === "watcher") && /[.:;]/.test(raw))
  ) {
    return sourceMeta(source).group;
  }
  return raw || sourceMeta(source).group;
}

function clusterTopicDetail(cluster: ReflectCluster): string {
  const sources = Object.entries(cluster.source_mix)
    .sort((left, right) => right[1] - left[1])
    .slice(0, 2)
    .map(([label, count]) => `${sourceMeta(label).label}${count > 1 ? ` (${count})` : ""}`)
    .join(" / ");
  return `${cluster.unit_count} item${cluster.unit_count === 1 ? "" : "s"}${sources ? ` from ${sources}` : ""}. ${relatedHistoryText(cluster.related_history)}`;
}

function latestUpdateTitle(item: ReflectSuggestedFollowup): string {
  return reflectLatestUpdateTitle(item);
}

function latestDevelopmentSummary(item: ReflectSuggestedFollowup): string {
  return reflectLatestDevelopmentSummary(item);
}

function latestUpdateSummary(item: ReflectSuggestedFollowup): string {
  return latestDevelopmentSummary(item);
}

function latestDevelopmentMeta(item: ReflectSuggestedFollowup): string {
  if (item.latest_summary_generated_at) {
    return `Generated from source check ${formatUiDateTime(item.search_checked_at || item.latest_summary_generated_at, { fallback: "recently" })}`;
  }
  if (item.search_results.length > 0) {
    return `Source check ${formatUiDateTime(item.search_checked_at || item.occurred_at, { fallback: "recently" })}`;
  }
  return `Queued ${formatUiDateTime(item.occurred_at, { fallback: "recently" })}`;
}

function followupWhatThisIs(item: ReflectSuggestedFollowup): string {
  const origin = item.source_label || "Reflect";
  if (item.kind === "latest_developments") {
    if (item.feedback?.renewed_after_feedback) {
      return `Renewed source-backed interest from ${origin}; this reappeared after earlier feedback.`;
    }
    if (item.latest_summary) return `Source-backed insight inferred from ${origin}.`;
    if (item.latest_summary_error && item.search_results.length > 0) {
      return `Current-source check inferred from ${origin}; source documents are available below.`;
    }
    if (item.search_results.length > 0) return `Current-source check inferred from ${origin}; summary worker pending.`;
    if (item.status === "failed") return `Current-source check inferred from ${origin}; source fetch failed.`;
    return `Next step from ${origin}; source check is queued.`;
  }
  if (item.kind === "recovery_advice") {
    return `Recovery item from ${origin}; a prior run needs follow-up.`;
  }
  return `Reflect opportunity from ${origin}.`;
}

function latestReflectedTopic(item: ReflectSuggestedFollowup): string {
  return compactText(stripInlineMarkup(item.title), 120);
}

function followupChatContext(item: ReflectSuggestedFollowup): string {
  const lines = [
    `Reflect opportunity: ${item.title}`,
    `Type: ${followupKindLabel(item.kind)}`,
    `Status: ${followupStatusLabel(item)}`,
    `Origin: ${item.source_label || "Reflect"}`,
    `Why surfaced: ${followupWhatThisIs(item)}`,
    item.detail ? `Reflect detail: ${item.detail}` : "",
    item.latest_summary ? `Source-backed insight:\n${item.latest_summary}` : "",
    item.search_error ? `Source check error: ${item.search_error}` : "",
    item.search_checked_at ? `Source checked at: ${item.search_checked_at}` : "",
    item.feedback?.renewed_after_feedback
      ? "Prior feedback: this area was dismissed or snoozed before, but newer reflected evidence suggests renewed interest."
      : "",
  ].filter(Boolean);
  if (item.search_results.length > 0) {
    lines.push("Cached sources:");
    for (const [index, result] of item.search_results.entries()) {
      lines.push(
        `${index + 1}. ${result.title || result.url}\nSource: ${result.source || "Search"}${result.published_date ? ` (${result.published_date})` : ""}\nURL: ${result.url || "n/a"}\nSnippet: ${result.snippet || "n/a"}`,
      );
    }
  }
  lines.push(`Requested opportunity: ${item.prompt.trim() || item.title.trim()}`);
  return lines.join("\n\n");
}

function followupKindLabel(kind: string): string {
  switch (kind) {
    case "latest_developments":
      return "Source check";
    case "recovery_advice":
      return "Needs review";
    default:
      return "Opportunity";
  }
}

function followupStatusLabel(item: ReflectSuggestedFollowup): string {
  if (item.kind === "latest_developments") {
    if (item.status === "queued") return "Research queued";
    if (item.status === "failed") return "Source check failed";
    if (item.search_results.length > 0 && !item.latest_summary && !item.latest_summary_error) {
      return "Summarizing insight";
    }
    if (item.search_results.length > 0) return `${item.search_results.length} source${item.search_results.length === 1 ? "" : "s"}`;
  }
  return humanizeStatusLabel(item.status, "Ready");
}

function followupActionLabel(kind: string): string {
  switch (kind) {
    case "latest_developments":
      return "Open in Chat";
    case "recovery_advice":
      return "Review in new Chat";
    default:
      return "Start new Chat";
  }
}

function unitReadableSummary(unit: ReflectUnit): string {
  return compactText(unit.summary || unit.content_preview || sourceMeta(unit.source_kind).group, 170);
}

function storeChatPendingLaunch(snapshot: ChatPendingLaunch): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(CHAT_PENDING_LAUNCH_STORAGE_KEY, JSON.stringify(snapshot));
  } catch {
    // Best-effort handoff only.
  }
}

function safeExternalHttpUrl(raw: string | undefined): string | null {
  const value = (raw || "").trim();
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

function quietStatus(
  response: ReflectResponse | undefined,
  fetching: boolean,
  refreshing: boolean,
): { title: string; detail: string; active: boolean } {
  if (!response) {
    return {
      title: fetching ? "Loading your recap" : "Recap is ready when activity is available",
      detail: "Reflect reads cached reflection data first, then updates quietly in the background.",
      active: fetching,
    };
  }
  if (refreshing || response.refresh_status.running) {
    return {
      title: response.cache_status.cached_units > 0 ? "Updating quietly" : "Preparing first recap",
      detail:
        response.cache_status.cached_units > 0
          ? "The current view stays usable while AgentArk refreshes the cached recap."
          : "AgentArk is gathering enough activity to build this view.",
      active: true,
    };
  }
  if (response.cache_status.mode === "empty") {
    return {
      title: "Still collecting data",
      detail:
        "Reflect does not have enough cached work units for this range yet. The recap will appear here once activity is available.",
      active: false,
    };
  }
  if (response.cache_status.mode === "stale") {
    return {
      title: "Showing the latest cached recap",
      detail: "Recent changes may appear after the next background refresh.",
      active: false,
    };
  }
  return {
    title: "Recap ready",
    detail: `${response.cache_status.cached_units} cached item${response.cache_status.cached_units === 1 ? "" : "s"} summarized for this range.`,
    active: false,
  };
}

export default function ReflectPage({ autoRefresh, onNavigateToView }: ReflectPageProps) {
  const queryClient = useQueryClient();
  const [period, setPeriod] = useState<ReflectPeriod>("weekly");
  const [anchor, setAnchor] = useState(() => toDateInputValue(new Date()));
  const [storyTab, setStoryTab] = useState<ReflectStoryTab>("latest");
  const [selectedFollowupId, setSelectedFollowupId] = useState<string | null>(null);
  const [topicPage, setTopicPage] = useState(0);
  const [topicSortMode, setTopicSortMode] = useState<ReflectTopicSortMode>("ranked");
  const [showAllTopics, setShowAllTopics] = useState(false);
  const [opportunityPage, setOpportunityPage] = useState(0);
  const [localRefreshRunning, setLocalRefreshRunning] = useState(false);
  const [refreshNotice, setRefreshNotice] = useState("");
  const [opportunitySettleUntil, setOpportunitySettleUntil] = useState<Record<string, number>>({});
  const bounds = useMemo(() => periodBounds(period, anchor), [period, anchor]);
  const fromIso = bounds.from.toISOString();
  const toIso = bounds.to.toISOString();
  const todayBounds = useMemo(
    () => periodBounds("daily", toDateInputValue(new Date())),
    [],
  );
  const todayFromIso = todayBounds.from.toISOString();
  const todayToIso = todayBounds.to.toISOString();
  const reflectQueryKey = useMemo(
    () => ["arkreflect", period, fromIso, toIso] as const,
    [period, fromIso, toIso],
  );
  const todayQueryKey = useMemo(
    () => ["arkreflect", "today", todayFromIso, todayToIso] as const,
    [todayFromIso, todayToIso],
  );

  const reflectQ = useQuery({
    queryKey: reflectQueryKey,
    queryFn: async () => {
      const raw = await api.rawGet(
        `/reflect?period=${encodeURIComponent(period)}&from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`,
      );
      return parseReflectResponse(raw, period);
    },
    refetchInterval: autoRefresh ? 120000 : false,
  });

  const refreshMutation = useMutation({
    mutationFn: async () => {
      const raw = await api.rawPost(
        `/reflect/refresh?period=${encodeURIComponent(period)}&from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`,
      );
      return parseReflectRefreshStartResponse(raw);
    },
    onMutate: () => {
      setLocalRefreshRunning(true);
      setRefreshNotice(
        "Reflect is running. This page is locked until the current refresh finishes.",
      );
    },
    onSuccess: (result) => {
      if (result.running || result.refresh_status.running) {
        setLocalRefreshRunning(true);
        setRefreshNotice(
          result.detail ||
            "Reflect is running. This page is locked until the current refresh finishes.",
        );
      } else {
        setLocalRefreshRunning(false);
        setRefreshNotice(result.detail);
      }
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
      void queryClient.invalidateQueries({ queryKey: todayQueryKey });
    },
    onError: () => {
      setLocalRefreshRunning(false);
    },
  });

  const response = reflectQ.data;
  const todayQ = useQuery({
    queryKey: todayQueryKey,
    queryFn: async () => {
      const raw = await api.rawGet(
        `/reflect?period=daily&from=${encodeURIComponent(todayFromIso)}&to=${encodeURIComponent(todayToIso)}`,
      );
      return parseReflectResponse(raw, "daily");
    },
    refetchInterval: autoRefresh ? 120000 : false,
  });
  const todayResponse = todayQ.data;
  const backendRefreshRunning = Boolean(response?.refresh_status.running);
  const isReflectRunning =
    refreshMutation.isPending || localRefreshRunning || backendRefreshRunning;
  const refreshStartedAt =
    refreshMutation.data?.refresh_status.started_at ||
    refreshMutation.data?.refresh_status.requested_at ||
    "";
  const runReflectNow = () => {
    if (isReflectRunning) return;
    refreshMutation.mutate();
  };

  useEffect(() => {
    if (backendRefreshRunning) {
      setLocalRefreshRunning(true);
      setRefreshNotice(
        "Reflect is running. This page is locked until the current refresh finishes.",
      );
      return;
    }
    const completedAt = response?.refresh_status.completed_at || "";
    const canClearLocalLock =
      Boolean(completedAt) &&
      (!refreshStartedAt || completedAt >= refreshStartedAt);
    if (!refreshMutation.isPending && canClearLocalLock) {
      setLocalRefreshRunning(false);
      setRefreshNotice("");
    }
  }, [backendRefreshRunning, refreshMutation.isPending, refreshStartedAt, response]);

  useEffect(() => {
    if (!isReflectRunning) return undefined;
    const id = window.setInterval(() => {
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
    }, 5000);
    return () => window.clearInterval(id);
  }, [isReflectRunning, queryClient, reflectQueryKey]);

  const clusters = response?.clusters ?? [];
  const clusterLabelById = useMemo(() => buildClusterLabelMap(clusters), [clusters]);
  const allUnits = useMemo(() => {
    const byId = new Map<string, ReflectUnit>();
    for (const cluster of clusters) {
      for (const unit of cluster.units) byId.set(unit.id, unit);
    }
    for (const unit of response?.unclustered_units ?? []) byId.set(unit.id, unit);
    return [...byId.values()];
  }, [clusters, response?.unclustered_units]);
  const suggestedFollowups = response?.suggested_followups ?? [];
  const openFollowupInChat = (item: ReflectSuggestedFollowup) => {
    const message = followupChatContext(item);
    if (!message) return;
    storeChatPendingLaunch({
      createdAt: Date.now(),
      launchMode: "message",
      message,
      newConversation: true,
      source: "Reflect",
    });
    onNavigateToView?.("chat");
    if (!onNavigateToView && typeof window !== "undefined") {
      window.location.href = "/ui/chat";
    }
  };
  const openSearchResult = (result: ReflectSearchResult) => {
    const safeUrl = safeExternalHttpUrl(result.url);
    if (!safeUrl || typeof window === "undefined") return;
    window.open(safeUrl, "_blank", "noopener,noreferrer");
  };
  const feedbackMutation = useMutation({
    mutationFn: async ({ item, action }: { item: ReflectSuggestedFollowup; action: "useful" | "snooze" | "dismiss" }) => {
      await api.rawPost(`/reflect/followups/${encodeURIComponent(item.id)}/feedback`, {
        action,
        keys: item.feedback_keys,
      });
      return { item, action };
    },
    onSuccess: ({ item, action }) => {
      if (action === "dismiss" || action === "snooze") {
        setSelectedFollowupId((current) => (current === item.id ? null : current));
      }
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
      void queryClient.invalidateQueries({ queryKey: todayQueryKey });
    },
  });
  const submitFollowupFeedback = (
    item: ReflectSuggestedFollowup,
    action: "useful" | "snooze" | "dismiss",
  ) => {
    if (feedbackMutation.isPending) return;
    feedbackMutation.mutate({ item, action });
  };
  const renderFollowupControls = (item: ReflectSuggestedFollowup, includeDetails: boolean) => (
    <Stack direction="row" spacing={0.65} sx={{ alignItems: "center", flexWrap: "wrap", rowGap: 0.65 }}>
      <Button
        size="small"
        variant="outlined"
        startIcon={<ThumbUpAltRoundedIcon />}
        disabled={feedbackMutation.isPending}
        onClick={(event) => {
          event.stopPropagation();
          submitFollowupFeedback(item, "useful");
        }}
        sx={{ borderRadius: "8px" }}
      >
        Useful
      </Button>
      <Button
        size="small"
        variant="outlined"
        disabled={feedbackMutation.isPending}
        onClick={(event) => {
          event.stopPropagation();
          submitFollowupFeedback(item, "snooze");
        }}
        sx={{ borderRadius: "8px" }}
      >
        Snooze
      </Button>
      <Button
        size="small"
        variant="outlined"
        startIcon={<CloseRoundedIcon />}
        disabled={feedbackMutation.isPending}
        onClick={(event) => {
          event.stopPropagation();
          submitFollowupFeedback(item, "dismiss");
        }}
        sx={{ borderRadius: "8px" }}
      >
        Dismiss
      </Button>
      {includeDetails ? (
        <Button
          size="small"
          variant="outlined"
          endIcon={<OpenInNewRoundedIcon />}
          onClick={(event) => {
            event.stopPropagation();
            setSelectedFollowupId(item.id);
          }}
          sx={{ borderRadius: "8px" }}
        >
          Details
        </Button>
      ) : null}
      <Button
        size="small"
        variant="contained"
        startIcon={<PlayArrowRoundedIcon />}
        onClick={(event) => {
          event.stopPropagation();
          openFollowupInChat(item);
        }}
        sx={{ borderRadius: "8px" }}
      >
        Launch
      </Button>
    </Stack>
  );

  useEffect(() => {
    const waitingForDailyLatest = suggestedFollowups.some(
      (item) => item.kind === "latest_developments" && item.status === "queued",
    );
    if (!waitingForDailyLatest) return undefined;
    const id = window.setInterval(() => {
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
    }, 30000);
    return () => window.clearInterval(id);
  }, [queryClient, reflectQueryKey, suggestedFollowups]);

  const totalUnits = allUnits.length;
  const embeddingCoverage =
    response && response.embedding_status.total_units > 0
      ? response.embedding_status.embedded_units / response.embedding_status.total_units
      : 0;
  const strongestUserCluster = useMemo(
    () => clusters.find((cluster) => clusterHasUserFacingSignal(cluster)) ?? null,
    [clusters],
  );

  const rangeLabel = formatUiDateRange(response?.from || fromIso, response?.to || toIso);
  const status = quietStatus(response, reflectQ.isFetching, isReflectRunning);
  const todayDigestTitle = digestStatusTitle(todayResponse);
  const todayDigestDetail = digestStatusDetail(todayResponse, todayQ.isFetching);
  const todayMeaningful = meaningfulForSourceCounts(todayResponse?.source_counts);
  const todayTotal = totalForSourceCounts(todayResponse?.source_counts);
  const focusLabel = strongestUserCluster
    ? clusterTopicTitle(strongestUserCluster)
    : totalUnits > 0
      ? "No clear user-facing focus yet"
      : "No activity yet";
  const recurringCount = clusters.filter((cluster) => cluster.related_history.mode === "recurring").length;
  const sourceRows = useMemo(
    () =>
      SOURCE_ORDER.map((source) => ({
        source,
        ...sourceMeta(source),
        count: countForSource(response, source),
      })).filter((item) => item.count > 0),
    [response],
  );
  const backgroundCount =
    countForSource(response, "app") +
    countForSource(response, "goal") +
    countForSource(response, "watcher") +
    countForSource(response, "sentinel") +
    countForSource(response, "arkpulse") +
    countForSource(response, "arkevolve");
  const learnedCount =
    countForSource(response, "experience_item") + countForSource(response, "procedural_pattern");
  const narrative = useMemo(
    () => narrativeLines(response, focusLabel, totalUnits, learnedCount, backgroundCount, recurringCount),
    [backgroundCount, focusLabel, learnedCount, recurringCount, response, totalUnits],
  );
  const hasReflectContent = Boolean(response) || reflectQ.isFetching || isReflectRunning;
  const selectedRangeLabel = rangeLabel || formatUiDateRange(fromIso, toIso);
  const sourceSignalCount = totalForSourceCounts(response?.source_counts);
  const emptyStateDetail = response
    ? totalUnits > 0
      ? `Reflect has ${totalUnits} reflected work unit${totalUnits === 1 ? "" : "s"} for this range and is still grouping them into focus areas.`
      : sourceSignalCount > 0
      ? `Reflect has ${sourceSignalCount} source signal${sourceSignalCount === 1 ? "" : "s"} in this range and is preparing the reflected work units for the recap.`
      : "No reflected work units are cached for this range yet. Keep working normally; this panel will turn into the recap after chat, ArkOrbit, apps, goals, watchers, or background systems produce activity."
    : status.detail;
  const emptyStateChip =
    reflectQ.isFetching || isReflectRunning
      ? "Collecting"
      : "Waiting for activity";

  const rankedTopicRows = useMemo(
    () =>
      uniqueByVisibleMeaning(
        [...clusters].sort((left, right) => right.unit_count - left.unit_count),
        (cluster) => clusterTopicTitle(cluster),
      ),
    [clusters],
  );
  const recentTopicRows = useMemo(
    () =>
      uniqueByVisibleMeaning(
        [...clusters].sort(
          (left, right) => clusterMostRecentTimestamp(right) - clusterMostRecentTimestamp(left),
        ),
        (cluster) => clusterTopicTitle(cluster),
      ),
    [clusters],
  );
  const sortedClusters = rankedTopicRows;
  const topicRows = topicSortMode === "recent" ? recentTopicRows : rankedTopicRows;
  const topClusters = rankedTopicRows.slice(0, 5);
  const graphTopicRows = useMemo(() => topicRows.slice(0, 8), [topicRows]);
  const maxTopicUnits = Math.max(1, ...topicRows.map((cluster) => cluster.unit_count));
  const topicPageCount = showAllTopics ? 1 : Math.max(1, Math.ceil(topicRows.length / TOPIC_PAGE_SIZE));
  const visibleTopicRows = showAllTopics
    ? topicRows
    : topicRows.slice(
      topicPage * TOPIC_PAGE_SIZE,
      topicPage * TOPIC_PAGE_SIZE + TOPIC_PAGE_SIZE,
    );
  const leadCluster = topicRows[0] ?? sortedClusters[0] ?? null;

  const constellationOption = useMemo(() => {
    const nodes: Array<Record<string, unknown>> = [];
    const links: Array<Record<string, unknown>> = [];
    const seen = new Set<string>();
    const clusterNodeIds: string[] = [];
    const centerName = leadCluster ? clusterTopicTitle(leadCluster) : "Reflect focus";
    nodes.push({
      id: "reflect-center",
      name: centerName,
      value: totalUnits,
      symbol: "path://M50,8 C72,8 88,24 88,46 C88,70 66,86 50,94 C34,86 12,70 12,46 C12,24 28,8 50,8 Z",
      symbolSize: 88,
      category: 0,
      x: 0,
      y: 0,
      itemStyle: {
        color: "rgba(174, 180, 187, 0.12)",
        borderColor: "#B7BCC2",
        borderWidth: 2,
        shadowBlur: 18,
        shadowColor: "rgba(174, 180, 187, 0.38)",
      },
      label: {
        show: true,
        position: "bottom",
        distance: 10,
        formatter: `{center|${compactText(centerName, 28)}}\n{count|${totalUnits} trace${totalUnits === 1 ? "" : "s"}}`,
        rich: {
          center: {
            color: "#F7FEFF",
            fontSize: 15,
            fontWeight: 800,
            fontFamily: "Inter, system-ui, sans-serif",
            lineHeight: 22,
          },
          count: {
            color: "#C6CAD0",
            fontSize: 10,
            fontWeight: 700,
            fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
            lineHeight: 16,
            backgroundColor: "rgba(255, 255, 255, 0.075)",
            padding: [3, 7, 3, 7],
            borderRadius: 4,
          },
        },
      },
      emphasis: {
        scale: 1.08,
        itemStyle: {
          shadowBlur: 24,
          shadowColor: "rgba(174, 180, 187, 0.46)",
        },
      },
    });
    graphTopicRows.forEach((cluster, index) => {
      const source = dominantSource(cluster);
      const meta = sourceMeta(source);
      const clusterName = clusterTopicTitle(cluster) || clusterLabelById[cluster.id] || sourceMeta(source).group;
      const nodeId = `cluster-${cluster.id}`;
      seen.add(nodeId);
      clusterNodeIds.push(nodeId);
      const stroke = tacticalAccent(meta.color);
      const ratio = cluster.unit_count / maxTopicUnits;
      const nodeSize = Math.max(48, Math.min(74, 46 + ratio * 28));
      const code = tacticalCode(source);
      const idx = String(index + 1).padStart(2, "0");
      const truncated = compactText(clusterName, 44);
      const angle = (index / Math.max(graphTopicRows.length, 1)) * Math.PI * 2 - Math.PI / 2;
      const radiusX = 300 + (index % 2) * 32;
      const radiusY = 170 + ((index + 1) % 2) * 26;
      const x = Math.cos(angle) * radiusX;
      const y = Math.sin(angle) * radiusY;
      const labelPosition = x < -24 ? "left" : x > 24 ? "right" : y < 0 ? "top" : "bottom";
      nodes.push({
        id: nodeId,
        name: clusterName,
        value: cluster.unit_count,
        symbol: tacticalSymbol(source),
        symbolSize: nodeSize,
        category: 1,
        itemStyle: {
          color: hexToRgba(stroke, 0.12),
          borderColor: stroke,
          borderWidth: 2,
          shadowBlur: 14,
          shadowColor: hexToRgba(stroke, 0.28),
        },
        label: {
          show: true,
          position: labelPosition,
          distance: 12,
          formatter: `{code|${idx} - ${code}}\n{name|${truncated}}\n{count|${cluster.unit_count} traces}`,
          rich: {
            code: {
              color: stroke,
              fontSize: 11,
              fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
              fontWeight: 800,
              lineHeight: 18,
            },
            name: {
              color: "rgba(246, 251, 255, 0.96)",
              fontSize: 13,
              fontFamily: "Inter, system-ui, sans-serif",
              fontWeight: 750,
              lineHeight: 18,
            },
            count: {
              color: stroke,
              fontSize: 10,
              fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
              fontWeight: 700,
              lineHeight: 17,
              backgroundColor: hexToRgba(stroke, 0.12),
              padding: [2, 6, 2, 6],
              borderRadius: 4,
            },
          },
        },
        emphasis: {
          scale: 1.12,
          itemStyle: {
            borderColor: stroke,
            borderWidth: 2.4,
            shadowBlur: 18,
            shadowColor: hexToRgba(stroke, 0.36),
          },
          label: {
            rich: {
              name: { color: "#f4fbff" },
              code: { color: stroke },
            },
          },
        },
        x,
        y,
      });
      links.push({
        source: "reflect-center",
        target: nodeId,
        value: cluster.unit_count,
        lineStyle: {
          width: 1.3 + ratio * 2.7,
          color: stroke,
          opacity: 0.72,
          curveness: index % 2 === 0 ? 0.18 : -0.18,
          shadowBlur: 12,
          shadowColor: hexToRgba(stroke, 0.28),
        },
      });
      cluster.related_history.items.slice(0, 2).forEach((item, itemIndex) => {
        const historyId = `history-${item.id}`;
        const satOffset = 54 + itemIndex * 22;
        const satAngle = angle + (itemIndex === 0 ? -0.22 : 0.22);
        if (!seen.has(historyId)) {
          seen.add(historyId);
          nodes.push({
            id: historyId,
            name: unitDisplayTitle({
              id: item.id,
              source_kind: "history",
              source_label: item.source_label,
              channel: "",
              title: item.title,
              summary: "",
              content_preview: "",
              occurred_at: item.occurred_at,
              message_count: 1,
              has_embedding: true,
            }),
            value: 1,
            symbol: "circle",
            symbolSize: 8,
            category: 2,
            itemStyle: {
              color: hexToRgba(stroke, 0.18),
              borderColor: hexToRgba(stroke, 0.62),
              borderWidth: 1.2,
              shadowBlur: 4,
              shadowColor: hexToRgba(stroke, 0.24),
            },
            label: { show: false },
            x: x + Math.cos(satAngle) * satOffset,
            y: y + Math.sin(satAngle) * (satOffset * 0.68),
          });
        }
        links.push({
          source: nodeId,
          target: historyId,
          value: item.similarity,
          lineStyle: {
            width: 0.9 + item.similarity * 1.2,
            color: stroke,
            opacity: 0.46,
            curveness: 0.14 + itemIndex * 0.06,
            type: "solid",
          },
        });
      });
    });
    if (clusterNodeIds.length >= 2) {
      for (let i = 0; i < clusterNodeIds.length; i += 1) {
        const next = clusterNodeIds[(i + 1) % clusterNodeIds.length];
        links.push({
          source: clusterNodeIds[i],
          target: next,
          lineStyle: {
            width: 0.7,
            color: "rgba(150, 180, 215, 0.22)",
            opacity: 0.48,
            curveness: i % 2 === 0 ? 0.22 : -0.22,
            type: [3, 6],
            dashOffset: 0,
          },
        });
      }
    }
    return {
      backgroundColor: "transparent",
      tooltip: {
        backgroundColor: "rgba(6, 11, 16, 0.96)",
        borderColor: "rgba(130, 170, 160, 0.4)",
        borderWidth: 1,
        padding: [8, 12],
        textStyle: {
          color: "#dceaf2",
          fontSize: 11.5,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
        },
        formatter: (info: { data?: { name?: string; value?: number } }) => {
          const name = (info.data?.name || "node").toUpperCase();
          const v = info.data?.value;
          return v
            ? `<span style="opacity:0.6">THEME</span> ${name}<br/><span style="opacity:0.6">TRACES</span> ${v}`
            : `<span style="opacity:0.6">NODE</span> ${name}`;
        },
      },
      graphic: {
        elements: [
          {
            type: "group",
            left: "center",
            top: "middle",
            children: [
              { type: "circle", shape: { cx: 0, cy: 0, r: 3 }, style: { fill: "transparent", stroke: "rgba(130,170,160,0.55)", lineWidth: 1 } },
              { type: "circle", shape: { cx: 0, cy: 0, r: 1 }, style: { fill: "rgba(130,170,160,0.7)" } },
              { type: "line", shape: { x1: -10, y1: 0, x2: -5, y2: 0 }, style: { stroke: "rgba(130,170,160,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 5, y1: 0, x2: 10, y2: 0 }, style: { stroke: "rgba(130,170,160,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 0, y1: -10, x2: 0, y2: -5 }, style: { stroke: "rgba(130,170,160,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 0, y1: 5, x2: 0, y2: 10 }, style: { stroke: "rgba(130,170,160,0.45)", lineWidth: 1 } },
            ],
          },
          {
            type: "circle",
            left: "center",
            top: "middle",
            shape: { cx: 0, cy: 0, r: 82 },
            style: { fill: "transparent", stroke: "rgba(174,180,187,0.16)", lineWidth: 1 },
            silent: true,
          },
          {
            type: "circle",
            left: "center",
            top: "middle",
            shape: { cx: 0, cy: 0, r: 132 },
            style: { fill: "transparent", stroke: "rgba(174,180,187,0.11)", lineWidth: 1 },
            silent: true,
          },
          {
            type: "circle",
            left: "center",
            top: "middle",
            shape: { cx: 0, cy: 0, r: 184 },
            style: { fill: "transparent", stroke: "rgba(174,180,187,0.08)", lineWidth: 1 },
            silent: true,
          },
          {
            type: "text",
            left: 14,
            top: 12,
            style: {
              text: `MEMORY GRAPH - ${topicRows.length.toString().padStart(2, "0")} THEMES`,
              fill: "rgba(184, 190, 198, 0.64)",
              font: "700 10px 'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
            },
          },
          {
            type: "text",
            right: 14,
            bottom: 12,
            style: {
              text: `${allUnits.length.toString().padStart(2, "0")} SOURCED TRACES`,
              fill: "rgba(170, 176, 184, 0.54)",
              font: "700 10px 'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
              textAlign: "right",
            },
          },
        ],
      },
      animationDurationUpdate: 900,
      animationEasingUpdate: "cubicInOut",
      series: [
        {
          type: "graph",
          layout: "none",
          roam: false,
          draggable: false,
          categories: [{ name: "Core" }, { name: "Theme" }, { name: "Evidence" }],
          data: nodes,
          links,
          edgeSymbol: ["none", "none"],
          lineStyle: { opacity: 0.4, curveness: 0.08 },
          zlevel: 2,
        },
      ],
    };
  }, [allUnits.length, clusterLabelById, graphTopicRows, leadCluster, maxTopicUnits, topicRows.length, totalUnits]);

  const activityOption = useMemo(() => {
    const TIMELINE_BUCKETS = period === "daily" ? 24 : period === "weekly" ? 28 : 36;
    const fromTs = response?.from ? Date.parse(response.from) : NaN;
    const toTs = response?.to ? Date.parse(response.to) : NaN;
    const haveBounds = Number.isFinite(fromTs) && Number.isFinite(toTs) && toTs > fromTs;
    const span = haveBounds ? toTs - fromTs : 1;
    const buckets = new Array(TIMELINE_BUCKETS).fill(0);
    for (const unit of allUnits) {
      const ts = Date.parse(unit.occurred_at);
      if (!Number.isFinite(ts)) continue;
      if (!haveBounds) continue;
      const ratio = (ts - fromTs) / span;
      const idx = Math.min(TIMELINE_BUCKETS - 1, Math.max(0, Math.floor(ratio * TIMELINE_BUCKETS)));
      buckets[idx] += 1;
    }
    const peak = Math.max(1, ...buckets);
    const startLabel = haveBounds
      ? formatUiDateOnly(new Date(fromTs).toISOString(), { fallback: "start" })
      : "start";
    const endLabel = haveBounds
      ? formatUiDateOnly(new Date(toTs).toISOString(), { fallback: "now" })
      : "now";
    const data = buckets.map((count) => ({
      value: count,
      itemStyle: {
        color: count === 0 ? "rgba(130, 170, 160, 0.10)" : "rgba(130, 170, 160, 0.78)",
        borderColor: count === peak ? "rgba(180, 230, 250, 0.95)" : "transparent",
        borderWidth: count === peak ? 0.6 : 0,
      },
    }));
    return {
      backgroundColor: "transparent",
      tooltip: {
        trigger: "axis",
        backgroundColor: "rgba(6, 11, 16, 0.96)",
        borderColor: "rgba(130, 170, 160, 0.4)",
        borderWidth: 1,
        padding: [6, 10],
        textStyle: {
          color: "#dceaf2",
          fontSize: 11,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
        },
        axisPointer: { type: "shadow", shadowStyle: { color: "rgba(130, 170, 160, 0.06)" } },
        formatter: (params: Array<{ dataIndex: number; value: number }>) => {
          const p = params?.[0];
          if (!p) return "";
          const i = p.dataIndex;
          const tBucket = haveBounds ? new Date(fromTs + ((i + 0.5) / TIMELINE_BUCKETS) * span) : null;
          const stamp = tBucket ? tBucket.toISOString().slice(0, 16).replace("T", " ") : `BIN ${i + 1}`;
          return `<span style="opacity:0.55">T</span> ${stamp}<br/><span style="opacity:0.55">N</span> ${p.value}`;
        },
      },
      grid: { left: 28, right: 12, top: 14, bottom: 22, containLabel: false },
      xAxis: {
        type: "category",
        data: buckets.map((_, i) => i),
        boundaryGap: true,
        axisTick: { show: false },
        axisLine: { lineStyle: { color: "rgba(130, 170, 160, 0.18)" } },
        axisLabel: {
          color: "rgba(180, 210, 225, 0.5)",
          fontSize: 9,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
          letterSpacing: 0.6,
          interval: TIMELINE_BUCKETS - 2,
          formatter: (val: string) => {
            const i = Number(val);
            if (i === 0) return startLabel.toUpperCase();
            if (i === TIMELINE_BUCKETS - 1) return endLabel.toUpperCase();
            return "";
          },
          align: (val: string) => (Number(val) === 0 ? "left" : "right"),
        },
      },
      yAxis: {
        type: "value",
        min: 0,
        max: peak,
        interval: peak,
        axisTick: { show: false },
        axisLine: { show: false },
        axisLabel: {
          color: "rgba(180, 210, 225, 0.45)",
          fontSize: 9,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
          showMinLabel: true,
          showMaxLabel: true,
          formatter: (val: number) => String(val),
        },
        splitLine: { show: false },
      },
      series: [
        {
          type: "bar",
          data,
          barWidth: 2,
          barCategoryGap: "60%",
          silent: false,
          animationDuration: 600,
          animationEasing: "cubicOut",
        },
      ],
    };
  }, [allUnits, period, response?.from, response?.to]);

  const recoveryFollowups = suggestedFollowups.filter((item) => item.kind === "recovery_advice");
  const latestFollowups = suggestedFollowups.filter((item) => item.kind === "latest_developments");
  const sourceBackedLatestFollowups = latestFollowups.filter(followupHasSourceEvidence);
  const opportunityFollowups = suggestedFollowups.filter(isDisplayableOpportunity);
  const reviewThreadFollowups = suggestedFollowups.filter(isReviewThreadFollowup);
  const selectedFollowup =
    [...opportunityFollowups, ...reviewThreadFollowups].find((item) => item.id === selectedFollowupId) ?? null;
  const latestSourceCount = latestFollowups.reduce((sum, item) => sum + item.search_results.length, 0);
  const latestReadyCount = sourceBackedLatestFollowups.filter((item) => item.status === "ready").length;
  const latestQueuedCount = latestFollowups.filter((item) => item.status === "queued").length;
  const latestFailedCount = latestFollowups.filter((item) => item.status === "failed" && followupHasSourceEvidence(item)).length;
  const opportunitySettleRefreshKey =
    response?.refresh_status.completed_at ||
    response?.refresh_status.started_at ||
    response?.refresh_status.requested_at ||
    "initial";
  const opportunitySettleKey = `${period}:${fromIso}:${toIso}:${opportunitySettleRefreshKey}`;
  const shouldSettleOpportunityPoll = shouldPollForOpportunitySettlement({
    sourceCounts: response?.source_counts,
    opportunityCount: opportunityFollowups.length,
    queuedSourceCheckCount: latestQueuedCount,
    refreshRunning: isReflectRunning,
  });
  const isOpportunitySettling = isOpportunitySettlementActive({
    shouldPoll: shouldSettleOpportunityPoll,
    currentUntil: opportunitySettleUntil[opportunitySettleKey],
    now: Date.now(),
  });
  const opportunityPageCount = Math.max(1, Math.ceil(opportunityFollowups.length / OPPORTUNITY_PAGE_SIZE));
  const visibleOpportunityFollowups = opportunityFollowups.slice(
    opportunityPage * OPPORTUNITY_PAGE_SIZE,
    opportunityPage * OPPORTUNITY_PAGE_SIZE + OPPORTUNITY_PAGE_SIZE,
  );

  useEffect(() => {
    if (!shouldSettleOpportunityPoll) return;
    const now = Date.now();
    setOpportunitySettleUntil((current) => {
      if (
        !shouldStartOpportunitySettlementPoll({
          shouldPoll: shouldSettleOpportunityPoll,
          currentUntil: current[opportunitySettleKey],
          now,
        })
      ) {
        return current;
      }
      return {
        ...current,
        [opportunitySettleKey]: now + OPPORTUNITY_SETTLE_WINDOW_MS,
      };
    });
  }, [opportunitySettleKey, shouldSettleOpportunityPoll]);

  useEffect(() => {
    if (!shouldSettleOpportunityPoll) return undefined;
    const until = opportunitySettleUntil[opportunitySettleKey] ?? 0;
    if (until <= Date.now()) return undefined;
    const id = window.setInterval(() => {
      if (Date.now() >= until) {
        setOpportunitySettleUntil((current) => {
          if (current[opportunitySettleKey] === -1) return current;
          return {
            ...current,
            [opportunitySettleKey]: -1,
          };
        });
        return;
      }
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
    }, OPPORTUNITY_SETTLE_POLL_MS);
    return () => window.clearInterval(id);
  }, [
    opportunitySettleKey,
    opportunitySettleUntil,
    queryClient,
    reflectQueryKey,
    shouldSettleOpportunityPoll,
  ]);

  const sourceMixOption = useMemo(
    () => ({
      backgroundColor: "transparent",
      color: sourceRows.map((source) => tacticalAccent(source.color)),
      tooltip: {
        trigger: "item",
        backgroundColor: "rgba(6, 11, 16, 0.96)",
        borderColor: "rgba(130, 170, 160, 0.4)",
        borderWidth: 1,
        textStyle: {
          color: "#dceaf2",
          fontSize: 11,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
        },
      },
      legend: {
        type: "scroll",
        bottom: 0,
        icon: "circle",
        itemWidth: 7,
        itemHeight: 7,
        textStyle: {
          color: "rgba(210, 226, 238, 0.72)",
          fontSize: 10,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
        },
      },
      series: [
        {
          type: "pie",
          radius: ["48%", "74%"],
          center: ["50%", "42%"],
          avoidLabelOverlap: true,
          padAngle: 2,
          itemStyle: { borderColor: "rgba(5, 9, 12, 0.94)", borderWidth: 2 },
          label: {
            color: "rgba(230, 244, 248, 0.86)",
            fontSize: 10,
            fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
            formatter: "{b}\n{c}",
          },
          labelLine: { length: 8, length2: 5, lineStyle: { color: "rgba(130, 170, 160, 0.35)" } },
          data: sourceRows.map((source) => ({ name: source.label, value: source.count })),
          animationDuration: 800,
          animationEasing: "cubicOut",
        },
      ],
    }),
    [sourceRows],
  );
  useEffect(() => {
    if (!selectedFollowupId) return;
    if (
      opportunityFollowups.some((item) => item.id === selectedFollowupId) ||
      reviewThreadFollowups.some((item) => item.id === selectedFollowupId)
    ) {
      return;
    }
    setSelectedFollowupId(null);
  }, [opportunityFollowups, reviewThreadFollowups, selectedFollowupId]);
  useEffect(() => {
    if (topicPage < topicPageCount) return;
    setTopicPage(Math.max(0, topicPageCount - 1));
  }, [topicPage, topicPageCount]);
  useEffect(() => {
    setTopicPage(0);
    setShowAllTopics(false);
  }, [topicSortMode, period, anchor]);
  useEffect(() => {
    if (opportunityPage < opportunityPageCount) return;
    setOpportunityPage(Math.max(0, opportunityPageCount - 1));
  }, [opportunityPage, opportunityPageCount]);
  const hasProblems =
    recoveryFollowups.length > 0 ||
    Boolean(response?.refresh_status.last_error) ||
    (Boolean(response) && response?.embedding_status.mode !== "semantic" && totalUnits > 0);
  const hasTodayStatus =
    todayQ.isFetching ||
    todayTotal > 0 ||
    todayMeaningful > 0 ||
    Boolean(todayResponse?.daily_digest_status.enabled) ||
    Boolean(todayResponse?.daily_digest_status.summary);
  const hasGroupingStatus =
    (response?.embedding_status.total_units ?? 0) > 0 ||
    Boolean(response?.embedding_status.detail);
  const hasStudioSide = opportunityFollowups.length > 0 || reviewThreadFollowups.length > 0;
  const whatWentWrong =
    response?.refresh_status.last_error ||
    recoveryFollowups[0]?.detail ||
    (response?.embedding_status.mode !== "semantic" && totalUnits > 0
      ? "Semantic grouping is still catching up, so some patterns may be grouped by source activity first."
      : "No major failure stood out in the reflected data. The main risk is leaving the opportunity implicit.");
  const overviewStats = [
    {
      label: "Topics found",
      value: `${topicRows.length}`,
      detail: `${totalUnits} reflected item${totalUnits === 1 ? "" : "s"} grouped into evidence-backed work themes.`,
      tone: "rgba(210, 214, 219, 0.82)",
    },
    {
      label: "Source checks",
      value: `${latestSourceCount || latestFollowups.length}`,
      detail:
        latestSourceCount > 0
          ? `${latestSourceCount} current-source result${latestSourceCount === 1 ? "" : "s"} cached for reflected topics.`
          : latestFollowups.length > 0
            ? `${latestFollowups.length} opportunit${latestFollowups.length === 1 ? "y is" : "ies are"} queued for source checking.`
            : "No opportunities are queued yet.",
      tone: "rgba(190, 196, 204, 0.78)",
    },
    ...(hasProblems
      ? [
          {
            label: "Needs attention",
            value: `${recoveryFollowups.length}`,
            detail:
              recoveryFollowups.length > 0
                ? "stalled or corrected run(s) that deserve review."
                : "a system caveat is affecting this recap.",
            tone: "var(--red)",
          },
        ]
      : []),
    ...(recurringCount > 0
      ? [
          {
            label: "Recurring threads",
            value: `${recurringCount}`,
            detail: `theme${recurringCount === 1 ? "" : "s"} connected to earlier similar work.`,
            tone: "var(--orange)",
          },
        ]
      : []),
  ];
  const storyTabs = [
    { value: "latest" as const, label: "Opportunities", short: "Opportunities", count: opportunityFollowups.length },
    { value: "overview" as const, label: "Overview", short: "Overview", count: totalUnits },
    ...(topClusters.length > 0
      ? [{ value: "topics" as const, label: "Topics", short: "Topics", count: topicRows.length }]
      : []),
    ...(reviewThreadFollowups.length > 0
      ? [{ value: "review" as const, label: "Recovery", short: "Recovery", count: reviewThreadFollowups.length }]
      : []),
  ];

  useEffect(() => {
    if (storyTabs.some((tab) => tab.value === storyTab)) return;
    setStoryTab("latest");
  }, [storyTab, storyTabs]);

  const renderStoryView = () => {
    const panelSx = {
      border: "1px solid var(--surface-border)",
      borderRadius: "8px",
      background:
        "radial-gradient(circle at top left, rgba(255, 255, 255, 0.026), transparent 38%), linear-gradient(180deg, var(--cyber-panel-raised), var(--cyber-panel))",
      boxShadow: "var(--surface-shadow-soft)",
    };
    const labelSx = {
      fontFamily: "var(--font-mono)",
      fontSize: "0.68rem",
      letterSpacing: "0.14em",
      textTransform: "uppercase",
      color: "var(--text-dim)",
      lineHeight: 1.35,
    };
    const titleSx = {
      fontFamily: "var(--font-display)",
      fontWeight: 750,
      letterSpacing: 0,
      lineHeight: 1.18,
    };
    const bodySx = {
      color: "var(--text-secondary)",
      lineHeight: 1.55,
      fontSize: "0.9rem",
    };
    const focusTitle =
      focusLabel === "No activity yet"
        ? "Reflect is waiting for a clear focus."
        : focusLabel === "No clear user-facing focus yet"
          ? "No user-facing focus yet"
          : `User-facing focus: ${focusLabel}`;

    return (
      <Stack spacing={1.4}>
        {/* Tabs are the primary Reflect surface: opportunities, overview,
            topics, and recovery threads share the same page-level frame. */}
        <Box
          className="arkreflect-motion-panel"
          sx={{
            ...panelSx,
            p: 0.75,
            display: "flex",
            gap: 0.75,
            flexWrap: "wrap",
            alignItems: "center",
          }}
        >
          {storyTabs.map((tab) => {
            const active = storyTab === tab.value;
            return (
              <Button
                key={tab.value}
                variant={active ? "contained" : "outlined"}
                onClick={() => setStoryTab(tab.value)}
                sx={{
                  minHeight: 34,
                  borderRadius: "8px",
                  color: active ? "var(--text-primary)" : "var(--button-text)",
                  bgcolor: active ? "rgba(255, 255, 255, 0.09)" : "transparent",
                  borderColor: active ? "rgba(255, 255, 255, 0.12)" : "var(--surface-border)",
                  "&:hover": {
                    bgcolor: active ? "rgba(255, 255, 255, 0.11)" : "rgba(255, 255, 255, 0.045)",
                    borderColor: "rgba(255, 255, 255, 0.14)",
                  },
                }}
              >
                {tab.short}
                <Box component="span" sx={{ ml: 0.75, opacity: 0.72, fontFamily: "var(--font-mono)" }}>
                  {tab.count}
                </Box>
              </Button>
            );
          })}
        </Box>

        {storyTab === "overview" ? (
        <Grid2 container spacing={1.4}>
          {sourceRows.length > 0 ? (
          <Grid2 size={{ xs: 12, lg: 3 }}>
            <Box className="arkreflect-motion-panel" sx={{ ...panelSx, p: 1.35, height: "100%" }}>
              <Typography sx={labelSx}>Source mix</Typography>
              <ReactECharts option={sourceMixOption} style={{ height: 230, width: "100%" }} />
              <Stack spacing={0.75} sx={{ mt: 0.5 }}>
                {sourceRows.slice(0, 5).map((source) => {
                  const pct = totalUnits > 0 ? Math.round((source.count / totalUnits) * 100) : 0;
                  return (
                    <Box key={source.source}>
                      <Stack direction="row" sx={{ justifyContent: "space-between", mb: 0.45 }}>
                        <Typography variant="caption">{source.label}</Typography>
                        <Typography variant="caption">{source.count}</Typography>
                      </Stack>
                      <Box sx={{ height: 5, borderRadius: 999, bgcolor: "var(--ui-rgba-255-255-255-040)", overflow: "hidden" }}>
                        <Box sx={{ height: "100%", width: `${Math.max(6, pct)}%`, bgcolor: tacticalAccent(source.color) }} />
                      </Box>
                    </Box>
                  );
                })}
              </Stack>
            </Box>
          </Grid2>
          ) : null}

          <Grid2
            size={{
              xs: 12,
              lg: sourceRows.length > 0 && hasStudioSide ? 6 : sourceRows.length > 0 || hasStudioSide ? 9 : 12,
            }}
          >
            <Box className="arkreflect-motion-panel" sx={{ ...panelSx, p: { xs: 1.4, md: 1.8 }, minHeight: 430 }}>
              {/* "Plain-language recap" sub-header dropped: the narrative
                  hero above already states the focus in plain English, and
                  repeating the user's literal message ("can you check my
                  google drive ?") as a section title felt like the page
                  echoing them. Only the date range stays as a quiet
                  context line at the top of the stats. */}
              <Stack direction="row" sx={{ justifyContent: "flex-end", mb: 1 }}>
                <Typography variant="caption" sx={{ color: "var(--text-secondary)" }}>
                  {rangeLabel}
                </Typography>
              </Stack>
              <Grid2 container spacing={1}>
                {overviewStats.map((item) => (
                  <Grid2 key={item.label} size={{ xs: 12, sm: 6 }}>
                    <Box
                      sx={{
                        p: 1.2,
                        minHeight: 126,
                        border: "1px solid var(--surface-border)",
                        borderRadius: "8px",
                        background: "var(--ui-rgba-255-255-255-020)",
                      }}
                    >
                      <Typography sx={labelSx}>{item.label}</Typography>
                      <Typography sx={{ fontFamily: "var(--font-mono)", fontSize: "1.75rem", fontWeight: 850, color: item.tone, mt: 0.6 }}>
                        {item.value}
                      </Typography>
                      <Typography sx={{ ...bodySx, mt: 0.45 }}>{item.detail}</Typography>
                    </Box>
                  </Grid2>
                ))}
              </Grid2>
              <Box sx={{ mt: 1.4 }}>
                <Stack direction={{ xs: "column", md: "row" }} spacing={1.2} sx={{ alignItems: { xs: "stretch", md: "center" } }}>
                  <Box sx={{ flex: "1 1 280px", minWidth: 0 }}>
                    <Typography sx={labelSx}>Activity rhythm</Typography>
                    <ReactECharts option={activityOption} style={{ height: 136, width: "100%" }} />
                  </Box>
                  <Box sx={{ flex: "1 1 260px", minWidth: 0 }}>
                    <Typography sx={labelSx}>Why this matters</Typography>
                    <Typography sx={{ ...bodySx, mt: 0.75 }}>
                      {hasProblems
                        ? whatWentWrong
                        : leadCluster
                          ? `${leadCluster.unit_count} item${leadCluster.unit_count === 1 ? "" : "s"} support the leading topic. ${latestFollowups.length} possible opportunit${latestFollowups.length === 1 ? "y can" : "ies can"} be checked against current sources.`
                          : "When enough activity exists, this area explains the strongest thread and why it is worth attention."}
                    </Typography>
                  </Box>
                </Stack>
              </Box>
            </Box>
          </Grid2>

          {hasStudioSide ? (
          <Grid2 size={{ xs: 12, lg: 3 }}>
            <Stack spacing={1.4}>
              {/* Today digest status panel removed: misleading "off" flag
                  read from the wrong field and the cached/meaningful chips
                  were operator-only telemetry. The hero now answers
                  "what's going on today" without the jargon side car. */}
              {opportunityFollowups.length > 0 ? (
                <Box sx={{ ...panelSx, p: 1.35 }}>
                  <Typography sx={labelSx}>Top opportunity</Typography>
                  <Typography sx={{ ...titleSx, fontSize: "1.35rem", mt: 0.55 }}>
                    {opportunityFollowups.length}
                  </Typography>
                  <Typography sx={{ ...bodySx, mt: 0.65 }}>
                    {opportunityFollowups[0].title}
                  </Typography>
                  <Button
                    size="small"
                    variant="outlined"
                    startIcon={<PlayArrowRoundedIcon />}
                    onClick={() => openFollowupInChat(opportunityFollowups[0])}
                    sx={{ mt: 1, borderRadius: "8px" }}
                  >
                    {followupActionLabel(opportunityFollowups[0].kind)}
                  </Button>
                </Box>
              ) : null}
              {reviewThreadFollowups.length > 0 ? (
                <Box sx={{ ...panelSx, p: 1.35 }}>
                  <Typography sx={labelSx}>Review threads</Typography>
                  <Typography sx={{ ...titleSx, fontSize: "1.35rem", mt: 0.55 }}>
                    {reviewThreadFollowups.length}
                  </Typography>
                  <Typography sx={{ ...bodySx, mt: 0.65 }}>
                    Failed or stalled reflected work stays separate from source-checked opportunities.
                  </Typography>
                  <Button
                    size="small"
                    variant="outlined"
                    startIcon={<WorkHistoryRoundedIcon />}
                    onClick={() => setStoryTab("review")}
                    sx={{ mt: 1, borderRadius: "8px" }}
                  >
                    Review threads
                  </Button>
                </Box>
              ) : null}
              {/* "Grouping 100%" embedding-coverage panel removed: pure
                  operator telemetry ("Semantic grouping is based on local
                  derived work-unit embeddings…") with no novice value. */}
            </Stack>
          </Grid2>
          ) : null}
        </Grid2>
        ) : null}

        {storyTab === "topics" ? (
        <Grid2 container spacing={1.4}>
          <Grid2 size={{ xs: 12, lg: 8 }}>
            <Box className="arkreflect-panorama arkreflect-memory-graph" sx={{ ...panelSx, p: { xs: 1.25, md: 1.65 }, minHeight: { xs: 560, md: 650 } }}>
              <Box className="arkreflect-panorama-backdrop" />
              <Box className="arkreflect-panorama-grid" />
              <Stack
                direction={{ xs: "column", sm: "row" }}
                spacing={1}
                className="arkreflect-memory-graph-head"
                sx={{ justifyContent: "space-between", alignItems: { xs: "stretch", sm: "flex-start" }, position: "relative", zIndex: 2 }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography sx={labelSx}>Memory graph</Typography>
                  <Typography sx={{ ...titleSx, fontSize: { xs: "1.18rem", md: "1.34rem" }, mt: 0.35 }}>
                    {leadCluster ? clusterTopicTitle(leadCluster) : "Work themes"}
                  </Typography>
                  <Typography sx={{ ...bodySx, mt: 0.45, maxWidth: 580 }}>
                    {topicRows.length} topic{topicRows.length === 1 ? "" : "s"} - {totalUnits} trace{totalUnits === 1 ? "" : "s"} - {sourceRows.length} source{sourceRows.length === 1 ? "" : "s"}
                  </Typography>
                </Box>
                <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", flexWrap: "wrap", rowGap: 0.75 }}>
                  <Chip className="arkreflect-pill" icon={<BubbleChartRoundedIcon />} label={`${graphTopicRows.length} shown`} />
                  <Chip className="arkreflect-pill" icon={<FitScreenRoundedIcon />} label="Fit view" />
                </Stack>
              </Stack>
              <Box className="arkreflect-panorama-canvas" sx={{ mt: 0.5, position: "relative", zIndex: 1 }}>
                <ReactECharts option={constellationOption} style={{ height: 510, width: "100%" }} />
              </Box>
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                className="arkreflect-memory-legend"
                sx={{ position: "relative", zIndex: 2, justifyContent: "space-between", alignItems: { xs: "flex-start", md: "center" } }}
              >
                <Stack direction="row" spacing={1.2} sx={{ flexWrap: "wrap", rowGap: 0.7 }}>
                  {sourceRows.slice(0, 6).map((source) => (
                    <Box key={source.source} className="arkreflect-memory-legend-item">
                      <Box sx={{ bgcolor: tacticalAccent(source.color) }} />
                      <Typography variant="caption">{source.label}</Typography>
                    </Box>
                  ))}
                </Stack>
                <Stack direction="row" spacing={1.2} sx={{ flexWrap: "wrap", rowGap: 0.7 }}>
                  <Box className="arkreflect-memory-legend-item">
                    <Box className="arkreflect-memory-legend-line is-strong" />
                    <Typography variant="caption">Strong link</Typography>
                  </Box>
                  <Box className="arkreflect-memory-legend-item">
                    <Box className="arkreflect-memory-legend-line is-weak" />
                    <Typography variant="caption">Weak link</Typography>
                  </Box>
                </Stack>
              </Stack>
            </Box>
          </Grid2>
          <Grid2 size={{ xs: 12, lg: 4 }}>
            <Box className="arkreflect-motion-panel arkreflect-theme-panel" sx={{ ...panelSx, p: { xs: 1.2, md: 1.45 }, minHeight: { xs: 420, md: 650 } }}>
              <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center", gap: 1 }}>
                <Box sx={{ minWidth: 0 }}>
                  <Typography sx={labelSx}>Deduped themes</Typography>
                  <Typography sx={{ ...titleSx, fontSize: "1.02rem", mt: 0.35 }}>
                    {topicRows.length} theme{topicRows.length === 1 ? "" : "s"}
                  </Typography>
                </Box>
                <ToggleButtonGroup
                  exclusive
                  size="small"
                  value={topicSortMode}
                  onChange={(_, value: ReflectTopicSortMode | null) => {
                    if (value) setTopicSortMode(value);
                  }}
                  sx={{
                    p: 0.25,
                    border: "1px solid var(--surface-border)",
                    borderRadius: "8px",
                    "& .MuiToggleButton-root": {
                      px: 1.15,
                      py: 0.45,
                      border: 0,
                      borderRadius: "7px",
                      color: "var(--text-secondary)",
                      fontSize: "0.72rem",
                      fontWeight: 750,
                      textTransform: "none",
                    },
                    "& .Mui-selected": {
                      color: "var(--text-primary) !important",
                      background: "rgba(255, 255, 255, 0.09) !important",
                      boxShadow: "inset 0 0 0 1px rgba(255, 255, 255, 0.12)",
                    },
                  }}
                >
                  <ToggleButton value="ranked">Ranked</ToggleButton>
                  <ToggleButton value="recent">Recent</ToggleButton>
                </ToggleButtonGroup>
              </Stack>
              <Box className="arkreflect-theme-list">
                {visibleTopicRows.map((cluster, index) => {
                  const name = clusterTopicTitle(cluster);
                  const sourceKey = dominantSource(cluster);
                  const source = sourceMeta(sourceKey);
                  const accent = tacticalAccent(source.color);
                  const displayIndex = topicPage * TOPIC_PAGE_SIZE + index;
                  const strength = topicStrength(cluster, maxTopicUnits);
                  const strengthColor = topicStrengthColor(strength.label);
                  const clusterSourceCount = Math.max(1, Object.keys(cluster.source_mix).length);
                  return (
                    <Box
                      key={cluster.id}
                      className="arkreflect-theme-card"
                      sx={{
                        borderColor: displayIndex === 0
                          ? "rgba(255, 255, 255, 0.16)"
                          : "rgba(255, 255, 255, 0.09)",
                        background:
                          displayIndex === 0
                            ? "linear-gradient(90deg, rgba(255, 255, 255, 0.055), rgba(18, 18, 20, 0.82))"
                            : "linear-gradient(90deg, rgba(255, 255, 255, 0.026), rgba(12, 12, 14, 0.72))",
                        boxShadow: "none",
                      }}
                    >
                      <Typography className="arkreflect-theme-rank" sx={{ color: "rgba(210, 214, 219, 0.92)" }}>
                        {String(displayIndex + 1).padStart(2, "0")}
                      </Typography>
                      <Box
                        className="arkreflect-theme-icon"
                        sx={{
                          color: accent,
                          borderColor: "rgba(255, 255, 255, 0.16)",
                          background: "rgba(255, 255, 255, 0.045)",
                          boxShadow: "none",
                        }}
                      >
                        {sourceIcon(source.label)}
                      </Box>
                      <Box sx={{ minWidth: 0 }}>
                        <Typography className="arkreflect-theme-title">{name}</Typography>
                        <Typography className="arkreflect-theme-meta">
                          {source.label} - {cluster.unit_count} trace{cluster.unit_count === 1 ? "" : "s"} - {clusterSourceCount} source{clusterSourceCount === 1 ? "" : "s"}
                        </Typography>
                        <Typography className="arkreflect-theme-detail">
                          {clusterTopicDetail(cluster)}
                        </Typography>
                      </Box>
                      <Stack className="arkreflect-theme-score" spacing={0.5}>
                        <Stack direction="row" spacing={0.55} sx={{ alignItems: "center", justifyContent: "flex-end" }}>
                          <Box sx={{ width: 7, height: 7, borderRadius: "50%", bgcolor: strengthColor, boxShadow: "none" }} />
                          <Typography variant="caption" sx={{ color: strengthColor, fontWeight: 800 }}>
                            {strength.label}
                          </Typography>
                        </Stack>
                        <Typography>{cluster.unit_count}</Typography>
                      </Stack>
                    </Box>
                  );
                })}
              </Box>
              <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", mt: 1 }}>
                {topicRows.length > TOPIC_PAGE_SIZE ? (
                  <Button
                    size="small"
                    variant="text"
                    startIcon={<OpenInNewRoundedIcon />}
                    onClick={() => {
                      setTopicPage(0);
                      setShowAllTopics((expanded) => !expanded);
                    }}
                    sx={{ borderRadius: "8px", color: "var(--text-secondary)", textTransform: "none" }}
                  >
                    {showAllTopics ? "Show page" : "View all themes"}
                  </Button>
                ) : (
                  <Box />
                )}
                <Stack direction="row" spacing={0.8} sx={{ alignItems: "center" }}>
                  <Typography variant="caption" sx={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)" }}>
                    {showAllTopics ? 1 : topicPage * TOPIC_PAGE_SIZE + 1}-{showAllTopics ? topicRows.length : Math.min(topicRows.length, (topicPage + 1) * TOPIC_PAGE_SIZE)} of {topicRows.length}
                  </Typography>
                  {!showAllTopics ? (
                    <>
                      <Tooltip title="Previous themes">
                        <span>
                          <IconButton
                            size="small"
                            disabled={topicPage === 0}
                            onClick={() => setTopicPage((page) => Math.max(0, page - 1))}
                            className="arkreflect-theme-pager"
                          >
                            <KeyboardArrowLeftRoundedIcon fontSize="small" />
                          </IconButton>
                        </span>
                      </Tooltip>
                      <Tooltip title="Next themes">
                        <span>
                          <IconButton
                            size="small"
                            disabled={topicPage >= topicPageCount - 1}
                            onClick={() => setTopicPage((page) => Math.min(topicPageCount - 1, page + 1))}
                            className="arkreflect-theme-pager"
                          >
                            <KeyboardArrowRightRoundedIcon fontSize="small" />
                          </IconButton>
                        </span>
                      </Tooltip>
                    </>
                  ) : null}
                </Stack>
              </Stack>
            </Box>
          </Grid2>
        </Grid2>
        ) : null}

        {storyTab === "latest" ? (
        <Box className="arkreflect-motion-panel" sx={{ ...panelSx, p: 1.35 }}>
          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", mb: 1.2 }}>
            <Box>
              <Typography sx={labelSx}>Opportunities</Typography>
              <Typography sx={{ ...titleSx, fontSize: "1.2rem", mt: 0.35 }}>
                What to do after this recap
              </Typography>
              <Typography sx={{ ...bodySx, mt: 0.55, maxWidth: 720 }}>
                Actionable next steps Reflect can open in Chat, with source checks where current info helps.
              </Typography>
            </Box>
            <Stack direction="row" spacing={0.7} sx={{ flexWrap: "wrap", rowGap: 0.7 }}>
              <Chip className="arkreflect-pill" icon={<TaskAltRoundedIcon />} label={`${opportunityFollowups.length} opportunit${opportunityFollowups.length === 1 ? "y" : "ies"}`} />
              {sourceBackedLatestFollowups.length > 0 ? (
                <Chip className="arkreflect-pill" icon={<SearchRoundedIcon />} label={`${latestSourceCount || sourceBackedLatestFollowups.length} source-backed`} />
              ) : null}
              {latestQueuedCount > 0 || isOpportunitySettling ? (
                <Chip
                  size="small"
                  variant="outlined"
                  label={latestQueuedCount > 0 ? `${latestQueuedCount} checking` : "checking"}
                />
              ) : null}
              {latestReadyCount > 0 ? <Chip size="small" variant="outlined" label={`${latestReadyCount} ready`} /> : null}
              {latestFailedCount > 0 ? <Chip size="small" variant="outlined" label={`${latestFailedCount} failed`} /> : null}
            </Stack>
          </Stack>
          <Stack spacing={1}>
            {opportunityFollowups.length === 0 ? (
              <Box
                sx={{
                  p: 2.5,
                  minHeight: 176,
                  border: "1px solid var(--surface-border)",
                  borderRadius: "8px",
                  background: "var(--ui-rgba-255-255-255-020)",
                  display: "grid",
                  placeItems: "center",
                  textAlign: "center",
                }}
              >
                <Stack spacing={1.05} sx={{ maxWidth: 400, alignItems: "center" }}>
                  {isReflectRunning || isOpportunitySettling ? (
                    <span className="arkreflect-live-dot" aria-hidden="true" />
                  ) : (
                    <Box sx={{ color: "rgba(210, 214, 219, 0.7)", display: "flex" }}>
                      <SearchRoundedIcon sx={{ fontSize: 26 }} />
                    </Box>
                  )}
                  <Typography sx={{ ...titleSx, fontSize: "1rem" }}>
                    {isReflectRunning || isOpportunitySettling
                      ? "Looking for useful opportunities"
                      : "No opportunities for this range yet"}
                  </Typography>
                  <Typography sx={{ ...bodySx, fontSize: "0.78rem" }}>
                    {isReflectRunning
                      ? "Reflect is refreshing this range — anything worth acting on shows up here."
                      : isOpportunitySettling
                        ? "Waiting for source checks to finish. Updates automatically."
                      : totalUnits > 0
                        ? "There's activity here, but nothing's been promoted yet. Use Run Reflect to re-check."
                        : "No activity cached for this range yet."}
                  </Typography>
                </Stack>
              </Box>
            ) : null}
            {visibleOpportunityFollowups.map((item) => {
              const isLatest = item.kind === "latest_developments";
              const itemIcon = isLatest ? (
                <SearchRoundedIcon fontSize="small" />
              ) : item.kind === "recovery_advice" ? (
                <AutoGraphRoundedIcon fontSize="small" />
              ) : (
                <TaskAltRoundedIcon fontSize="small" />
              );
              const itemSummary = isLatest ? latestUpdateSummary(item) : item.detail || followupStatusLabel(item);
              return (
                <Box
                  key={item.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => setSelectedFollowupId(item.id)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedFollowupId(item.id);
                    }
                  }}
                  sx={{
                    display: "grid",
                    gridTemplateColumns: { xs: "1fr", md: "34px minmax(0, 1fr) auto" },
                    gap: 1,
                    alignItems: "start",
                    p: { xs: 1.05, md: 1.2 },
                    border: "1px solid var(--surface-border)",
                    borderRadius: "8px",
                    cursor: "pointer",
                    background:
                      isLatest && item.search_results.length > 0
                        ? "linear-gradient(90deg, rgba(255, 255, 255, 0.055), rgba(255,255,255,0.02))"
                        : "var(--ui-rgba-255-255-255-020)",
                    transition: "border-color 180ms ease, background 180ms ease, transform 180ms ease",
                    "&:hover": {
                      borderColor: "rgba(255, 255, 255, 0.12)",
                      background: "rgba(255, 255, 255, 0.04)",
                      transform: "translateY(-1px)",
                    },
                  }}
                >
                  <Box sx={{ color: isLatest ? "rgba(210, 214, 219, 0.8)" : item.kind === "recovery_advice" ? "var(--red)" : "rgba(210, 214, 219, 0.72)", pt: 0.25 }}>
                    {itemIcon}
                  </Box>
                  <Box sx={{ minWidth: 0 }}>
                    <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", flexWrap: "wrap", rowGap: 0.5 }}>
                      <Typography sx={{ ...titleSx, fontSize: "1.05rem" }}>
                        {isLatest ? latestUpdateTitle(item) : item.title}
                      </Typography>
                      <Chip size="small" variant="outlined" label={followupKindLabel(item.kind)} />
                      <Chip size="small" variant="outlined" label={followupStatusLabel(item)} />
                    </Stack>
                    <Typography variant="caption" sx={{ display: "block", mt: 0.45, color: "var(--text-dim)" }}>
                      {isLatest ? latestDevelopmentMeta(item) : followupWhatThisIs(item)}
                    </Typography>
                    <Typography sx={{ ...bodySx, mt: 0.45, whiteSpace: isLatest ? "pre-line" : "normal" }}>
                      {itemSummary}
                    </Typography>
                    {isLatest ? (
                      <Typography variant="caption" sx={{ display: "block", mt: 0.55, color: "var(--text-dim)" }}>
                        Reflected topic: {latestReflectedTopic(item)}
                      </Typography>
                    ) : null}
                  </Box>
                  <Box sx={{ justifySelf: { xs: "start", md: "end" } }}>
                    {renderFollowupControls(item, isLatest)}
                  </Box>
                </Box>
              );
            })}
            {opportunityFollowups.length > OPPORTUNITY_PAGE_SIZE ? (
              <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center", pt: 0.25 }}>
                <Typography variant="caption" sx={{ color: "var(--text-dim)" }}>
                  {opportunityPage * OPPORTUNITY_PAGE_SIZE + 1}-{Math.min(opportunityFollowups.length, (opportunityPage + 1) * OPPORTUNITY_PAGE_SIZE)} of {opportunityFollowups.length}
                </Typography>
                <Stack direction="row" spacing={0.75}>
                  <Button
                    size="small"
                    variant="outlined"
                    disabled={opportunityPage === 0}
                    onClick={() => setOpportunityPage((page) => Math.max(0, page - 1))}
                    sx={{ borderRadius: "8px" }}
                  >
                    Previous
                  </Button>
                  <Button
                    size="small"
                    variant="outlined"
                    disabled={opportunityPage >= opportunityPageCount - 1}
                    onClick={() => setOpportunityPage((page) => Math.min(opportunityPageCount - 1, page + 1))}
                    sx={{ borderRadius: "8px" }}
                  >
                    Next
                  </Button>
                </Stack>
              </Stack>
            ) : null}
          </Stack>
        </Box>
        ) : null}

        {storyTab === "review" ? (
          <Box className="arkreflect-motion-panel" sx={{ ...panelSx, p: 1.35 }}>
            <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", mb: 1.2 }}>
              <Box>
                <Typography sx={labelSx}>Review threads</Typography>
                <Typography sx={{ ...titleSx, fontSize: "1.2rem", mt: 0.35 }}>
                  Recovery items that need attention
                </Typography>
              </Box>
              <Stack direction="row" spacing={0.7} sx={{ flexWrap: "wrap", rowGap: 0.7 }}>
                <Chip className="arkreflect-pill" icon={<WorkHistoryRoundedIcon />} label={`${reviewThreadFollowups.length} review item${reviewThreadFollowups.length === 1 ? "" : "s"}`} />
                {recoveryFollowups.length > 0 ? (
                  <Chip size="small" variant="outlined" label={`${recoveryFollowups.length} recovery`} />
                ) : null}
              </Stack>
            </Stack>
            <Stack spacing={1}>
              {reviewThreadFollowups.map((item) => {
                const itemIcon =
                  item.kind === "recovery_advice" ? (
                    <AutoGraphRoundedIcon fontSize="small" />
                  ) : (
                    <TaskAltRoundedIcon fontSize="small" />
                  );
                return (
                  <Box
                    key={item.id}
                    role="button"
                    tabIndex={0}
                    onClick={() => setSelectedFollowupId(item.id)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        setSelectedFollowupId(item.id);
                      }
                    }}
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", md: "34px minmax(0, 1fr) auto" },
                      gap: 1,
                      alignItems: "start",
                      p: { xs: 1.05, md: 1.2 },
                      border: "1px solid var(--surface-border)",
                      borderRadius: "8px",
                      cursor: "pointer",
                      background: "var(--ui-rgba-255-255-255-020)",
                      transition: "border-color 180ms ease, background 180ms ease, transform 180ms ease",
                      "&:hover": {
                        borderColor: "rgba(255, 255, 255, 0.12)",
                        background: "rgba(255, 255, 255, 0.04)",
                        transform: "translateY(-1px)",
                      },
                    }}
                  >
                    <Box sx={{ color: item.kind === "recovery_advice" ? "var(--red)" : "rgba(210, 214, 219, 0.72)", pt: 0.25 }}>
                      {itemIcon}
                    </Box>
                    <Box sx={{ minWidth: 0 }}>
                      <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", flexWrap: "wrap", rowGap: 0.5 }}>
                        <Typography sx={{ ...titleSx, fontSize: "1.05rem" }}>
                          {item.title}
                        </Typography>
                        <Chip size="small" variant="outlined" label={followupKindLabel(item.kind)} />
                        <Chip size="small" variant="outlined" label={followupStatusLabel(item)} />
                      </Stack>
                      <Typography variant="caption" sx={{ display: "block", mt: 0.45, color: "var(--text-dim)" }}>
                        {followupWhatThisIs(item)}
                      </Typography>
                      <Typography sx={{ ...bodySx, mt: 0.45 }}>
                        {item.detail || "Review this failed or stalled reflected item in Chat, decide the recovery path, or dismiss it if it is stale."}
                      </Typography>
                    </Box>
                    <Box sx={{ justifySelf: { xs: "start", md: "end" } }}>
                      {renderFollowupControls(item, false)}
                    </Box>
                  </Box>
                );
              })}
            </Stack>
          </Box>
        ) : null}
      </Stack>
    );
  };

  return (
    <WorkspacePageShell spacing={1.4} className="arkreflect-mono">
      <WorkspacePageHeader
        eyebrow="ARK CORE"
        title="Reflect"
        description="Visual retrospective of recent chats, memory, and background work — semantic clusters, narrative summary, source coverage, and rhythm."
        actions={
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: { xs: "1fr", sm: "auto 164px auto" },
              alignItems: "center",
              justifyContent: { xs: "stretch", sm: "end" },
              gap: 1,
              minWidth: { xs: "100%", md: 460 },
              "& .MuiToggleButtonGroup-root": {
                height: 40,
                justifySelf: { xs: "stretch", sm: "end" },
              },
              "& .MuiToggleButton-root": {
                minHeight: 40,
                height: 40,
              },
              "& .MuiInputBase-root": {
                height: 40,
                borderRadius: "8px",
                alignItems: "center",
              },
              "& .MuiButton-root": {
                height: 40,
                whiteSpace: "nowrap",
              },
            }}
          >
            <ToggleButtonGroup
              exclusive
              size="small"
              value={period}
              onChange={(_, value) => value && setPeriod(value)}
              disabled={isReflectRunning}
              aria-label="Reflect period"
              sx={{
                bgcolor: "rgba(255,255,255,0.06)",
                borderRadius: 2,
                "& .MuiToggleButton-root": {
                  minHeight: 40,
                  px: 1.6,
                  color: "text.secondary",
                  borderColor: "rgba(255,255,255,0.12)",
                },
                "& .Mui-selected": {
                  color: "var(--text-primary)",
                  bgcolor: "rgba(255, 255, 255, 0.09)",
                },
              }}
            >
              {PERIOD_OPTIONS.map((option) => (
                <ToggleButton key={option.value} value={option.value}>
                  {option.label}
                </ToggleButton>
              ))}
            </ToggleButtonGroup>
            <TextField
              size="small"
              type="date"
              value={anchor}
              onChange={(event) => setAnchor(event.target.value)}
              disabled={isReflectRunning}
              sx={{ minWidth: 164 }}
              slotProps={{
                input: {
                  startAdornment: (
                    <InputAdornment position="start" sx={{ mr: 0.4 }}>
                      <CalendarMonthRoundedIcon fontSize="small" />
                    </InputAdornment>
                  ),
                },
              }}
            />
            <Tooltip title="Run Reflect for this date range now">
              <Button
                variant="outlined"
                onClick={runReflectNow}
                disabled={isReflectRunning}
                startIcon={<RefreshRoundedIcon />}
                sx={{ minHeight: 40 }}
              >
                {isReflectRunning ? "Running Reflect" : "Run Reflect now"}
              </Button>
            </Tooltip>
          </Box>
        }
      />

      {reflectQ.error ? <Alert severity="error">{errMessage(reflectQ.error)}</Alert> : null}
      {refreshMutation.error ? <Alert severity="error">{errMessage(refreshMutation.error)}</Alert> : null}
      {isReflectRunning ? (
        <Alert
          severity="info"
          icon={<RefreshRoundedIcon fontSize="inherit" />}
          sx={{
            border: "1px solid var(--surface-border)",
            bgcolor: "rgba(255, 255, 255, 0.045)",
          }}
        >
          <Stack spacing={1}>
            <Box>
              {refreshNotice ||
                "Reflect is running. This page is locked until the current refresh finishes."}
            </Box>
            <LinearProgress sx={{ borderRadius: 999 }} />
          </Stack>
        </Alert>
      ) : null}

      {!hasReflectContent ? (
        <Box
          className="arkreflect-status"
          sx={{
            p: { xs: 2.2, md: 3 },
            border: "1px solid rgba(130, 170, 160, 0.18)",
            borderRadius: "3px",
            background:
              "linear-gradient(180deg, rgba(7, 13, 18, 0.96), rgba(5, 9, 12, 0.94))",
            boxShadow: "0 24px 60px rgba(0, 0, 0, 0.34)",
          }}
        >
          <Stack
            direction={{ xs: "column", md: "row" }}
            spacing={2}
            sx={{ alignItems: { xs: "flex-start", md: "center" } }}
          >
            <Box
              sx={{
                width: 46,
                height: 46,
                borderRadius: "6px",
                border: "1px solid rgba(130, 170, 160, 0.28)",
                color: "var(--cyan-glow)",
                display: "grid",
                placeItems: "center",
                background: "rgba(130, 170, 160, 0.07)",
                flex: "0 0 auto",
              }}
            >
              <AutoGraphRoundedIcon />
            </Box>
            <Box sx={{ flex: 1, minWidth: 0 }}>
              <Typography
                sx={{
                  fontFamily: "var(--font-display)",
                  fontSize: { xs: "1.25rem", md: "1.45rem" },
                  fontWeight: 750,
                  color: "rgba(237,247,244,0.96)",
                  mb: 0.5,
                }}
              >
                {status.active ? status.title : "Still collecting data"}
              </Typography>
              <Typography
                sx={{
                  maxWidth: 820,
                  color: "rgba(213,228,225,0.72)",
                  lineHeight: 1.55,
                }}
              >
                {emptyStateDetail}
              </Typography>
            </Box>
            <Chip
              className="arkreflect-pill"
              label={emptyStateChip}
              icon={status.active ? <RefreshRoundedIcon /> : <WorkHistoryRoundedIcon />}
              sx={{ flex: "0 0 auto" }}
            />
          </Stack>
          <LinearProgress
            variant={status.active ? "indeterminate" : "determinate"}
            value={status.active ? undefined : 0}
            sx={{ mt: 2.4, mb: 2 }}
          />
          <Grid2 container spacing={1.2}>
            {[
              { label: "Range", value: selectedRangeLabel || "Selected period" },
              {
                label: "Cached units",
                value: String(response?.cache_status.cached_units ?? totalUnits),
              },
              {
                label: "Source signals",
                value: String(sourceSignalCount),
              },
            ].map((item) => (
              <Grid2 key={item.label} size={{ xs: 12, sm: 4 }}>
                <Box
                  sx={{
                    p: 1.4,
                    border: "1px solid rgba(130, 170, 160, 0.12)",
                    borderRadius: "3px",
                    background: "rgba(255,255,255,0.025)",
                    minHeight: 78,
                  }}
                >
                  <Typography
                    sx={{
                      fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
                      fontSize: "0.66rem",
                      letterSpacing: "0.16em",
                      textTransform: "uppercase",
                      color: "rgba(180, 210, 225, 0.52)",
                      mb: 0.8,
                    }}
                  >
                    {item.label}
                  </Typography>
                  <Typography
                    sx={{
                      color: "rgba(237,247,244,0.9)",
                      fontWeight: 700,
                      lineHeight: 1.25,
                    }}
                  >
                    {item.value}
                  </Typography>
                </Box>
              </Grid2>
            ))}
          </Grid2>
          {response?.refresh_status.last_error ? (
            <Alert severity="warning" sx={{ mt: 2 }}>
              {response.refresh_status.last_error}
            </Alert>
          ) : null}
        </Box>
      ) : (
        renderStoryView()
      )}

      {response?.generated_at ? (
        <Typography variant="caption" color="text.secondary" sx={{ px: 0.5 }}>
          Cached view generated {formatUiDateTime(response.generated_at, { fallback: response.generated_at })}
          {response.refresh_status.completed_at
            ? ` - Last background refresh ${formatUiDateTime(response.refresh_status.completed_at, { fallback: response.refresh_status.completed_at })}`
            : ""}
        </Typography>
      ) : null}

      <Dialog
        open={Boolean(selectedFollowup)}
        onClose={() => setSelectedFollowupId(null)}
        fullWidth
        maxWidth="md"
        slotProps={{
          paper: {
            sx: {
              border: "1px solid var(--surface-border)",
              borderRadius: "8px",
              background:
                "linear-gradient(180deg, rgba(7, 13, 18, 0.98), rgba(5, 9, 12, 0.98))",
            },
          },
        }}
      >
        {selectedFollowup ? (
          <>
            <DialogTitle sx={{ pb: 1 }}>
              <Typography sx={{ fontFamily: "var(--font-mono)", fontSize: "0.68rem", letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--text-dim)" }}>
                {followupKindLabel(selectedFollowup.kind)}
              </Typography>
              <Typography sx={{ fontFamily: "var(--font-display)", fontWeight: 850, fontSize: "1.15rem", mt: 0.45 }}>
                {selectedFollowup.kind === "latest_developments" ? latestUpdateTitle(selectedFollowup) : selectedFollowup.title}
              </Typography>
              <Typography variant="caption" sx={{ color: "var(--text-dim)", display: "block", mt: 0.45 }}>
                {selectedFollowup.kind === "latest_developments"
                  ? latestDevelopmentMeta(selectedFollowup)
                  : `${selectedFollowup.source_label || "Reflect"} - ${formatUiDateTime(selectedFollowup.occurred_at, { fallback: "recently" })}`}
              </Typography>
            </DialogTitle>
            <DialogContent dividers sx={{ borderColor: "var(--surface-border)" }}>
              <Stack spacing={1.25}>
                <Box>
                  <Typography sx={{ fontFamily: "var(--font-mono)", fontSize: "0.68rem", letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--text-dim)", mb: 0.55 }}>
                    Detail
                  </Typography>
                  <Typography sx={{ color: "var(--text-secondary)", lineHeight: 1.6, whiteSpace: "pre-line" }}>
                    {selectedFollowup.kind === "latest_developments"
                      ? latestUpdateSummary(selectedFollowup)
                      : selectedFollowup.detail || selectedFollowup.prompt || followupStatusLabel(selectedFollowup)}
                  </Typography>
                </Box>
                <Box>
                  <Typography sx={{ fontFamily: "var(--font-mono)", fontSize: "0.68rem", letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--text-dim)", mb: 0.55 }}>
                    Why surfaced
                  </Typography>
                  <Typography sx={{ color: "var(--text-secondary)", lineHeight: 1.6 }}>
                    {followupWhatThisIs(selectedFollowup)}
                  </Typography>
                  {selectedFollowup.kind === "latest_developments" ? (
                    <Typography variant="caption" sx={{ color: "var(--text-dim)", display: "block", mt: 0.45 }}>
                      Reflected topic: {latestReflectedTopic(selectedFollowup)}
                    </Typography>
                  ) : null}
                </Box>
                {selectedFollowup.kind === "latest_developments" || selectedFollowup.search_results.length > 0 || selectedFollowup.search_error ? (
                <Box>
                  <Typography sx={{ fontFamily: "var(--font-mono)", fontSize: "0.68rem", letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--text-dim)", mb: 0.8 }}>
                    Source documents
                  </Typography>
                  <Stack spacing={0.85}>
                    {selectedFollowup.search_results.length > 0 ? (
                      selectedFollowup.search_results.map((result, index) => {
                          const safeUrl = safeExternalHttpUrl(result.url);
                          return (
                            <Box
                              key={`${selectedFollowup.id}-${result.url || index}`}
                              sx={{
                                p: 1,
                                border: "1px solid rgba(130, 170, 160, 0.14)",
                                borderRadius: "8px",
                                background: "rgba(255,255,255,0.025)",
                              }}
                            >
                              <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", gap: 1 }}>
                                <Box sx={{ minWidth: 0 }}>
                                  <Typography sx={{ fontWeight: 850 }}>{compactText(stripInlineMarkup(result.title), 120)}</Typography>
                                  <Typography variant="caption" sx={{ color: "var(--text-dim)" }}>
                                    {result.source || "Source"}{result.published_date ? ` - ${result.published_date}` : ""}
                                  </Typography>
                                  <Typography sx={{ color: "var(--text-secondary)", lineHeight: 1.55, mt: 0.55 }}>
                                    {compactText(stripInlineMarkup(result.snippet || result.url), 220)}
                                  </Typography>
                                </Box>
                                {safeUrl ? (
                                  <Button
                                    size="small"
                                    variant="outlined"
                                    endIcon={<OpenInNewRoundedIcon />}
                                    onClick={() => openSearchResult(result)}
                                    sx={{ borderRadius: "8px", alignSelf: { xs: "flex-start", sm: "center" }, flex: "0 0 auto" }}
                                  >
                                    Open
                                  </Button>
                                ) : null}
                              </Stack>
                            </Box>
                          );
                      })
                    ) : (
                      <Typography sx={{ color: "var(--text-secondary)" }}>
                        {selectedFollowup.search_error || selectedFollowup.detail || "No sources are cached for this insight yet."}
                      </Typography>
                    )}
                  </Stack>
                </Box>
                ) : null}
              </Stack>
            </DialogContent>
            <DialogActions sx={{ px: 2, py: 1.3 }}>
              <Stack direction="row" spacing={0.75} sx={{ flex: 1, flexWrap: "wrap", rowGap: 0.75 }}>
                {renderFollowupControls(selectedFollowup, false)}
              </Stack>
              <Button onClick={() => setSelectedFollowupId(null)}>Close</Button>
            </DialogActions>
          </>
        ) : null}
      </Dialog>
    </WorkspacePageShell>
  );
}
