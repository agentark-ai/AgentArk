import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  Divider,
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
import DonutLargeRoundedIcon from "@mui/icons-material/DonutLargeRounded";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import InsightsRoundedIcon from "@mui/icons-material/InsightsRounded";
import MemoryRoundedIcon from "@mui/icons-material/MemoryRounded";
import MonitorHeartRoundedIcon from "@mui/icons-material/MonitorHeartRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
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
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { asRecord, errMessage, num, pickRecords, str } from "./pageHelpers";

type ArkReflectPageProps = {
  autoRefresh: boolean;
};

type ReflectPeriod = "daily" | "weekly" | "monthly";

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
  clusters: ReflectCluster[];
  unclustered_units: ReflectUnit[];
};

const PERIOD_OPTIONS: { value: ReflectPeriod; label: string }[] = [
  { value: "daily", label: "Day" },
  { value: "weekly", label: "Week" },
  { value: "monthly", label: "Month" },
];

const SOURCE_DISPLAY: Record<string, { label: string; group: string; color: string }> = {
  conversation: { label: "Chat", group: "Conversation work", color: "#4E8DFF" },
  orbit_chat: { label: "ArkOrbit", group: "Orbit conversations", color: "#7C5CFF" },
  experience_item: { label: "Memory", group: "What AgentArk learned", color: "#21B573" },
  procedural_pattern: { label: "Workflows", group: "Working patterns", color: "#E6A93D" },
  app: { label: "Apps", group: "Apps built", color: "#00A8A8" },
  goal: { label: "Goals", group: "Goals and progress", color: "#FF7A45" },
  watcher: { label: "Watchers", group: "Background watchers", color: "#D94F70" },
  sentinel: { label: "Sentinel", group: "Safety and checks", color: "#A96DFF" },
  arkpulse: { label: "ArkPulse", group: "System health", color: "#00B8D9" },
  arkevolve: { label: "ArkEvolve", group: "Agent improvements", color: "#C58A00" },
  llm_usage: { label: "Usage", group: "Agent usage", color: "#8FA3BF" },
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
    color: str(raw.color, "#2F80ED"),
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
    clusters: pickRecords(raw, "clusters")
      .map(asReflectCluster)
      .filter((cluster): cluster is ReflectCluster => cluster !== null),
    unclustered_units: pickRecords(raw, "unclustered_units")
      .map(asReflectUnit)
      .filter((unit): unit is ReflectUnit => unit !== null),
  };
}

function sourceIcon(label: string) {
  const lower = label.toLowerCase();
  if (lower.includes("orbit")) return <HubRoundedIcon fontSize="small" />;
  if (lower.includes("memory")) return <MemoryRoundedIcon fontSize="small" />;
  return <ChatRoundedIcon fontSize="small" />;
}

function relatedHistoryLabel(history: ReflectRelatedHistory): string {
  if (history.mode === "recurring") return "Recurring theme";
  if (history.mode === "new") return "New this period";
  return "History pending";
}

function relatedHistoryColor(history: ReflectRelatedHistory): "default" | "primary" | "success" {
  if (history.mode === "recurring") return "primary";
  if (history.mode === "new") return "success";
  return "default";
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
  if (unit.source_kind === "llm_usage") return "Usage summary";
  if (title.length < 8) return sourceMeta(unit.source_kind).group;
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
  return [
    `I saw ${totalUnits} reflected item${totalUnits === 1 ? "" : "s"} in this range, with ${focusLabel.toLowerCase()} as the clearest focus.`,
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
  return SOURCE_DISPLAY[source] ?? { label: "Work", group: "Mixed work", color: "#8FA3BF" };
}

function dominantSource(cluster: ReflectCluster): string {
  const counts = new Map<string, number>();
  for (const unit of cluster.units) {
    counts.set(unit.source_kind, (counts.get(unit.source_kind) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "work";
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
  if (!hsl) return hex;
  return hslToHex(hsl.h, Math.min(0.7, hsl.s * 0.78), Math.min(0.78, hsl.l * 0.95 + 0.18));
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
      ? "Loading today's ArkReflect status."
      : "Today status appears here after ArkReflect has cached activity.";
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
    return "ArkReflect checked the day and found nothing worth notifying you about.";
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

function quietStatus(
  response: ReflectResponse | undefined,
  fetching: boolean,
  refreshing: boolean,
): { title: string; detail: string; active: boolean } {
  if (!response) {
    return {
      title: fetching ? "Loading your recap" : "Recap is ready when activity is available",
      detail: "ArkReflect reads cached reflection data first, then updates quietly in the background.",
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
      title: "No recap for this range yet",
      detail: "Choose another range or refresh when AgentArk is idle.",
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

export default function ArkReflectPage({ autoRefresh }: ArkReflectPageProps) {
  const queryClient = useQueryClient();
  const [period, setPeriod] = useState<ReflectPeriod>("weekly");
  const [anchor, setAnchor] = useState(() => toDateInputValue(new Date()));
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
      await api.rawPost(
        `/reflect/refresh?period=${encodeURIComponent(period)}&from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
      void queryClient.invalidateQueries({ queryKey: todayQueryKey });
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

  useEffect(() => {
    if (!response?.refresh_status.running && !refreshMutation.isPending) return undefined;
    const id = window.setInterval(() => {
      void queryClient.invalidateQueries({ queryKey: reflectQueryKey });
    }, 5000);
    return () => window.clearInterval(id);
  }, [queryClient, reflectQueryKey, refreshMutation.isPending, response?.refresh_status.running]);

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

  const totalUnits = allUnits.length;
  const strongestCluster = clusters[0] ?? null;
  const embeddingCoverage =
    response && response.embedding_status.total_units > 0
      ? response.embedding_status.embedded_units / response.embedding_status.total_units
      : 0;

  const rangeLabel = formatUiDateRange(response?.from || fromIso, response?.to || toIso);
  const status = quietStatus(response, reflectQ.isFetching, refreshMutation.isPending);
  const todayDigestTitle = digestStatusTitle(todayResponse);
  const todayDigestDetail = digestStatusDetail(todayResponse, todayQ.isFetching);
  const todayMeaningful = meaningfulForSourceCounts(todayResponse?.source_counts);
  const todayTotal = totalForSourceCounts(todayResponse?.source_counts);
  const focusLabel = strongestCluster
    ? (clusterLabelById[strongestCluster.id] ?? clusterDisplayLabel(strongestCluster))
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
  const styleSignals = useMemo(() => workingStyleSignals(response), [response]);
  const narrative = useMemo(
    () => narrativeLines(response, focusLabel, totalUnits, learnedCount, backgroundCount, recurringCount),
    [backgroundCount, focusLabel, learnedCount, recurringCount, response, totalUnits],
  );

  const SPARKLINE_BUCKETS = 7;
  const backgroundSparklines = useMemo(() => {
    const sources = ["app", "goal", "watcher", "sentinel", "arkpulse", "arkevolve"] as const;
    const fromTs = response?.from ? Date.parse(response.from) : NaN;
    const toTs = response?.to ? Date.parse(response.to) : NaN;
    const haveBounds =
      Number.isFinite(fromTs) && Number.isFinite(toTs) && toTs > fromTs;
    const span = haveBounds ? toTs - fromTs : 1;
    const result: Record<string, number[]> = {};
    for (const source of sources) {
      result[source] = new Array(SPARKLINE_BUCKETS).fill(0);
    }
    if (!haveBounds) return result;
    for (const unit of allUnits) {
      const bucket = result[unit.source_kind as keyof typeof result];
      if (!bucket) continue;
      const ts = Date.parse(unit.occurred_at);
      if (!Number.isFinite(ts)) continue;
      const ratio = (ts - fromTs) / span;
      const idx = Math.min(
        SPARKLINE_BUCKETS - 1,
        Math.max(0, Math.floor(ratio * SPARKLINE_BUCKETS)),
      );
      bucket[idx] += 1;
    }
    return result;
  }, [allUnits, response?.from, response?.to]);

  const constellationOption = useMemo(() => {
    const nodes: Array<Record<string, unknown>> = [];
    const links: Array<Record<string, unknown>> = [];
    const seen = new Set<string>();
    const clusterNodeIds: string[] = [];
    clusters.forEach((cluster, index) => {
      const source = dominantSource(cluster);
      const meta = sourceMeta(source);
      const clusterName = clusterLabelById[cluster.id] ?? clusterDisplayLabel(cluster);
      const nodeId = `cluster-${cluster.id}`;
      seen.add(nodeId);
      clusterNodeIds.push(nodeId);
      const nodeSize = Math.max(14, Math.min(28, 12 + cluster.unit_count * 3));
      const stroke = tacticalAccent(meta.color);
      const code = tacticalCode(source);
      const idx = String(index + 1).padStart(2, "0");
      const truncated = clusterName.length > 38 ? `${clusterName.slice(0, 36)}…` : clusterName;
      nodes.push({
        id: nodeId,
        name: clusterName,
        value: cluster.unit_count,
        symbol: tacticalSymbol(source),
        symbolSize: nodeSize,
        category: 0,
        itemStyle: {
          color: "rgba(0,0,0,0)",
          borderColor: stroke,
          borderWidth: 1,
          shadowBlur: 6,
          shadowColor: stroke,
        },
        label: {
          show: true,
          position: "right",
          distance: 8,
          formatter: `{code|${idx}·${code}}  {name|${truncated.toUpperCase()}}`,
          rich: {
            code: {
              color: stroke,
              fontSize: 8.5,
              fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
              fontWeight: 500,
              letterSpacing: 1,
              backgroundColor: "rgba(0,0,0,0.35)",
              padding: [2, 4, 2, 4],
              borderRadius: 1,
            },
            name: {
              color: "rgba(210, 226, 238, 0.78)",
              fontSize: 9.5,
              fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
              fontWeight: 400,
              letterSpacing: 0.6,
            },
          },
        },
        emphasis: {
          scale: 1.4,
          itemStyle: {
            borderColor: stroke,
            borderWidth: 1.4,
            shadowBlur: 14,
            shadowColor: stroke,
          },
          label: {
            rich: {
              name: { color: "#f4fbff" },
              code: { color: stroke },
            },
          },
        },
        x: Math.cos((index / Math.max(clusters.length, 1)) * Math.PI * 2 - Math.PI / 2) * 240,
        y: Math.sin((index / Math.max(clusters.length, 1)) * Math.PI * 2 - Math.PI / 2) * 150,
      });
      const angle = (index / Math.max(clusters.length, 1)) * Math.PI * 2 - Math.PI / 2;
      cluster.related_history.items.slice(0, 2).forEach((item, itemIndex) => {
        const historyId = `history-${item.id}`;
        const satOffset = 36 + itemIndex * 18;
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
            symbol: "path://M50,8 L92,50 L50,92 L8,50 Z",
            symbolSize: 6,
            category: 1,
            itemStyle: {
              color: "rgba(0,0,0,0)",
              borderColor: "rgba(170, 200, 220, 0.4)",
              borderWidth: 0.8,
            },
            label: { show: false },
            x: Math.cos(satAngle) * (240 + satOffset),
            y: Math.sin(satAngle) * (150 + satOffset * 0.6),
          });
        }
        links.push({
          source: nodeId,
          target: historyId,
          value: item.similarity,
          lineStyle: {
            width: 0.8 + item.similarity * 1.4,
            color: stroke,
            opacity: 0.42,
            curveness: 0.14 + itemIndex * 0.06,
            type: "solid",
          },
        });
      });
    });
    if (links.length === 0 && clusterNodeIds.length >= 2) {
      for (let i = 0; i < clusterNodeIds.length; i += 1) {
        for (let j = i + 1; j < clusterNodeIds.length; j += 1) {
          links.push({
            source: clusterNodeIds[i],
            target: clusterNodeIds[j],
            lineStyle: {
              width: 0.6,
              color: "rgba(140, 200, 220, 0.16)",
              curveness: 0.18,
              type: [3, 5],
              dashOffset: 0,
            },
          });
        }
      }
    }
    return {
      backgroundColor: "transparent",
      tooltip: {
        backgroundColor: "rgba(6, 11, 16, 0.96)",
        borderColor: "rgba(120, 200, 220, 0.4)",
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
            ? `<span style="opacity:0.6">› TRACE</span> ${name}<br/><span style="opacity:0.6">› UNITS</span> ${v}`
            : `<span style="opacity:0.6">› NODE</span> ${name}`;
        },
      },
      graphic: {
        elements: [
          {
            type: "group",
            left: "center",
            top: "middle",
            children: [
              { type: "circle", shape: { cx: 0, cy: 0, r: 3 }, style: { fill: "transparent", stroke: "rgba(120,200,220,0.55)", lineWidth: 1 } },
              { type: "circle", shape: { cx: 0, cy: 0, r: 1 }, style: { fill: "rgba(120,200,220,0.7)" } },
              { type: "line", shape: { x1: -10, y1: 0, x2: -5, y2: 0 }, style: { stroke: "rgba(120,200,220,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 5, y1: 0, x2: 10, y2: 0 }, style: { stroke: "rgba(120,200,220,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 0, y1: -10, x2: 0, y2: -5 }, style: { stroke: "rgba(120,200,220,0.45)", lineWidth: 1 } },
              { type: "line", shape: { x1: 0, y1: 5, x2: 0, y2: 10 }, style: { stroke: "rgba(120,200,220,0.45)", lineWidth: 1 } },
            ],
          },
          {
            type: "text",
            left: 14,
            top: 12,
            style: {
              text: `◢ PANORAMA · ${clusters.length.toString().padStart(2, "0")} TRACES`,
              fill: "rgba(120, 200, 220, 0.55)",
              font: "500 9.5px 'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
            },
          },
          {
            type: "text",
            right: 14,
            bottom: 12,
            style: {
              text: "◣ FOCUS·MAP",
              fill: "rgba(120, 200, 220, 0.45)",
              font: "500 9.5px 'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
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
          categories: [{ name: "Active" }, { name: "Bridge" }],
          data: nodes,
          links,
          edgeSymbol: ["none", "none"],
          lineStyle: { opacity: 0.4, curveness: 0.08 },
          zlevel: 2,
        },
      ],
    };
  }, [clusters, clusterLabelById]);

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
        color: count === 0 ? "rgba(120, 200, 220, 0.10)" : "rgba(120, 200, 220, 0.78)",
        borderColor: count === peak ? "rgba(180, 230, 250, 0.95)" : "transparent",
        borderWidth: count === peak ? 0.6 : 0,
      },
    }));
    return {
      backgroundColor: "transparent",
      tooltip: {
        trigger: "axis",
        backgroundColor: "rgba(6, 11, 16, 0.96)",
        borderColor: "rgba(120, 200, 220, 0.4)",
        borderWidth: 1,
        padding: [6, 10],
        textStyle: {
          color: "#dceaf2",
          fontSize: 11,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
        },
        axisPointer: { type: "shadow", shadowStyle: { color: "rgba(120, 200, 220, 0.06)" } },
        formatter: (params: Array<{ dataIndex: number; value: number }>) => {
          const p = params?.[0];
          if (!p) return "";
          const i = p.dataIndex;
          const tBucket = haveBounds ? new Date(fromTs + ((i + 0.5) / TIMELINE_BUCKETS) * span) : null;
          const stamp = tBucket ? tBucket.toISOString().slice(0, 16).replace("T", " ") : `BIN ${i + 1}`;
          return `<span style="opacity:0.55">› T</span> ${stamp}<br/><span style="opacity:0.55">› N</span> ${p.value}`;
        },
      },
      grid: { left: 28, right: 12, top: 14, bottom: 22, containLabel: false },
      xAxis: {
        type: "category",
        data: buckets.map((_, i) => i),
        boundaryGap: true,
        axisTick: { show: false },
        axisLine: { lineStyle: { color: "rgba(120, 200, 220, 0.18)" } },
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
        minInterval: 1,
        max: peak,
        axisTick: { show: false },
        axisLine: { show: false },
        axisLabel: {
          color: "rgba(180, 210, 225, 0.45)",
          fontSize: 9,
          fontFamily: "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace",
          showMinLabel: false,
          formatter: (val: number) => (val === peak || val === 0 ? String(val) : ""),
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

  const sourceDonutOption = useMemo(
    () => ({
      tooltip: { trigger: "item" },
      legend: {
        bottom: 0,
        textStyle: { color: "rgba(255,255,255,0.7)" },
      },
      series: [
        {
          type: "pie",
          radius: ["48%", "72%"],
          center: ["50%", "43%"],
          avoidLabelOverlap: true,
          label: {
            color: "rgba(255,255,255,0.86)",
            formatter: "{b}",
          },
          labelLine: { lineStyle: { color: "rgba(255,255,255,0.28)" } },
          data: sourceRows.map((item) => ({
            name: item.label,
            value: item.count,
            itemStyle: { color: item.color },
          })),
        },
      ],
    }),
    [sourceRows],
  );

  const radarOption = useMemo(
    () => ({
      tooltip: {
        formatter: () =>
          "Working style is shown as change versus your recent baseline, not raw counts.",
      },
      radar: {
        radius: "68%",
        indicator: styleSignals.map((signal) => ({
          name: signal.label,
          max: 100,
        })),
        splitNumber: 4,
        axisName: { color: "rgba(255,255,255,0.72)", fontSize: 11 },
        splitLine: { lineStyle: { color: "rgba(255,255,255,0.13)" } },
        splitArea: { areaStyle: { color: ["rgba(255,255,255,0.02)", "rgba(255,255,255,0.05)"] } },
        axisLine: { lineStyle: { color: "rgba(255,255,255,0.13)" } },
      },
      series: [
        {
          type: "radar",
          data: [
            {
              name: "Change",
              value: styleSignals.map((signal) => Math.max(0, Math.min(100, 50 + signal.delta * 160))),
              areaStyle: { color: "rgba(78,141,255,0.22)" },
              lineStyle: { color: "#4E8DFF", width: 2 },
              itemStyle: { color: "#4E8DFF" },
            },
            {
              name: "Baseline",
              value: styleSignals.map(() => 50),
              areaStyle: { color: "rgba(255,255,255,0.04)" },
              lineStyle: { color: "rgba(255,255,255,0.38)", width: 1, type: "dashed" },
              itemStyle: { color: "rgba(255,255,255,0.55)" },
            },
          ],
        },
      ],
    }),
    [styleSignals],
  );

  return (
    <WorkspacePageShell spacing={1.4}>
      <WorkspacePageHeader
        eyebrow="ArkReflect"
        title="Your work, clustered into a clear recap"
        description={
          <span>
            See where chat, ArkOrbit, apps, goals, watchers, Sentinel, ArkPulse,
            ArkEvolve, usage, memory, and learned workflows concentrated.
          </span>
        }
        actions={
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{ minWidth: { xs: "100%", md: 460 } }}
          >
            <ToggleButtonGroup
              exclusive
              size="small"
              value={period}
              onChange={(_, value) => value && setPeriod(value)}
              aria-label="Reflection period"
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
                  color: "primary.contrastText",
                  bgcolor: "primary.main",
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
              sx={{ minWidth: 164 }}
              slotProps={{
                input: {
                  startAdornment: <CalendarMonthRoundedIcon fontSize="small" />,
                },
              }}
            />
            <Tooltip title="Refresh recap in the background">
              <Button
                variant="outlined"
                onClick={() => refreshMutation.mutate()}
                disabled={refreshMutation.isPending || response?.refresh_status.running}
                startIcon={<RefreshRoundedIcon />}
                sx={{ minHeight: 40 }}
              >
                {response?.refresh_status.running || refreshMutation.isPending ? "Refreshing" : "Refresh"}
              </Button>
            </Tooltip>
          </Stack>
        }
      />

      {reflectQ.error ? <Alert severity="error">{errMessage(reflectQ.error)}</Alert> : null}
      {refreshMutation.error ? <Alert severity="error">{errMessage(refreshMutation.error)}</Alert> : null}
      <Box
        className="list-shell arkreflect-status"
        sx={{
          p: 1.25,
          borderColor: status.active ? "rgba(0,168,168,0.34)" : "rgba(255,255,255,0.1)",
        }}
      >
        <Stack direction="row" spacing={1.1} sx={{ alignItems: "center" }}>
          <InsightsRoundedIcon color={status.active ? "primary" : "disabled"} fontSize="small" />
          <Box sx={{ minWidth: 0, flex: 1 }}>
            <span className="arkreflect-section-eyebrow">Reflect Runtime</span>
            <Typography variant="body2" sx={{ fontWeight: 800 }}>
              {status.title}
            </Typography>
            <Typography variant="caption" className="arkreflect-section-subtitle">
              {status.detail}
            </Typography>
          </Box>
          <Chip
            size="small"
            label={
              response?.embedding_status.mode === "semantic"
                ? `${Math.round(embeddingCoverage * 100)}% grouped`
                : "Preparing"
            }
            variant="outlined"
          />
        </Stack>
        {status.active || reflectQ.isFetching ? <LinearProgress sx={{ mt: 1.1, borderRadius: 999 }} /> : null}
      </Box>

      <Box
        className="list-shell arkreflect-status"
        sx={{
          p: 1.25,
          borderColor:
            todayResponse?.daily_digest_status.enabled && todayMeaningful > 0
              ? "rgba(78,141,255,0.34)"
              : "rgba(255,255,255,0.1)",
        }}
      >
        <Stack direction={{ xs: "column", md: "row" }} spacing={1.2} sx={{ alignItems: { xs: "flex-start", md: "center" } }}>
          <Stack direction="row" spacing={1.1} sx={{ alignItems: "center", minWidth: 0, flex: 1 }}>
            <NotificationsActiveRoundedIcon
              color={todayResponse?.daily_digest_status.enabled ? "primary" : "disabled"}
              fontSize="small"
            />
            <Box sx={{ minWidth: 0 }}>
              <span className="arkreflect-section-eyebrow">Today Status</span>
              <Typography variant="body2" sx={{ fontWeight: 850 }}>
                {todayDigestTitle}
              </Typography>
              <Typography variant="caption" className="arkreflect-section-subtitle">
                {todayDigestDetail}
              </Typography>
            </Box>
          </Stack>
          <Stack direction="row" spacing={0.75} sx={{ flexWrap: "wrap", rowGap: 0.75 }}>
            <Chip size="small" label={`${todayTotal} cached`} variant="outlined" />
            <Chip size="small" label={`${todayMeaningful} meaningful`} color={todayMeaningful > 0 ? "primary" : "default"} variant="outlined" />
            {todayResponse?.daily_digest_status.summary &&
            (!todayResponse.daily_digest_status.target_date ||
              todayResponse.daily_digest_status.target_date ===
                todayResponse.daily_digest_status.today_date) ? (
              <Chip size="small" label="Summary prepared" color="success" variant="outlined" />
            ) : null}
          </Stack>
        </Stack>
        {todayResponse?.daily_digest_status.summary &&
        (!todayResponse.daily_digest_status.target_date ||
          todayResponse.daily_digest_status.target_date ===
            todayResponse.daily_digest_status.today_date) ? (
          <Typography
            variant="body2"
            sx={{
              mt: 1,
              whiteSpace: "pre-line",
              color: "text.primary",
              lineHeight: 1.55,
            }}
          >
            {todayResponse.daily_digest_status.summary}
          </Typography>
        ) : null}
        {todayQ.isFetching ? <LinearProgress sx={{ mt: 1.1, borderRadius: 999 }} /> : null}
      </Box>

      <Box className="list-shell arkreflect-narrative" sx={{ p: { xs: 1.4, md: 2 } }}>
        <Stack spacing={1.3}>
          <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
            <InsightsRoundedIcon color="primary" />
            <Box>
              <span className="arkreflect-section-eyebrow">What I noticed</span>
              <Typography variant="h6" sx={{ fontWeight: 850, lineHeight: 1.2 }}>
                A plain-language read of this period
              </Typography>
              <Typography variant="body2" className="arkreflect-section-subtitle">
                Before the charts. Numbers below.
              </Typography>
            </Box>
          </Stack>
          <Stack spacing={0.8}>
            {narrative.map((line) => (
              <Typography key={line} className="arkreflect-narrative-line" variant="body1">
                {line}
              </Typography>
            ))}
          </Stack>
        </Stack>
      </Box>

      <Box
        className="list-shell arkreflect-panorama"
        sx={{
          p: { xs: 1.2, md: 1.6 },
        }}
      >
        <Box className="arkreflect-panorama-backdrop" />
        <Box className="arkreflect-panorama-grid" />
        <Stack
          direction="row"
          sx={{ justifyContent: "space-between", alignItems: "flex-start", mb: 1, position: "relative", zIndex: 3 }}
        >
          <Box>
            <span className="arkreflect-section-eyebrow">Panorama</span>
            <Typography variant="h6" sx={{ fontWeight: 850, lineHeight: 1.2 }}>
              Focus areas across this period
            </Typography>
            <Typography variant="body2" className="arkreflect-section-subtitle">
              Islands are focus areas. Bridges connect this period to similar history.
            </Typography>
          </Box>
          <Stack direction="row" spacing={0.7} sx={{ flexWrap: "wrap", justifyContent: "flex-end", gap: 0.7 }}>
            <Chip
              className="arkreflect-pill"
              size="small"
              icon={<BubbleChartRoundedIcon />}
              label={`${clusters.length} focus areas`}
            />
            <Chip
              className="arkreflect-pill"
              size="small"
              icon={<WorkHistoryRoundedIcon />}
              label={`${recurringCount} recurring`}
            />
          </Stack>
        </Stack>
        {clusters.length > 0 ? (
          <Box className="arkreflect-panorama-canvas">
            <ReactECharts option={constellationOption} style={{ height: 460, width: "100%" }} />
          </Box>
        ) : (
          <Box className="arkreflect-panorama-empty" sx={{ height: 420, display: "grid", placeItems: "center", textAlign: "center" }}>
            <Stack spacing={0.8} sx={{ alignItems: "center" }}>
              <BubbleChartRoundedIcon color="disabled" />
              <Typography color="text.secondary">
                {status.active ? "Preparing the first panorama." : "No activity found in this range."}
              </Typography>
            </Stack>
          </Box>
        )}
      </Box>

      <Grid2 container spacing={1.2}>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <Box className="list-shell arkreflect-grid-pane" sx={{ p: 1.2, minHeight: 360 }}>
            <Stack direction="row" spacing={1} sx={{ alignItems: "center", px: 0.4 }}>
              <AutoGraphRoundedIcon color="success" />
              <Box>
                <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                  Working style
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  Change versus your recent baseline.
                </Typography>
              </Box>
            </Stack>
            <ReactECharts option={radarOption} style={{ height: 292, width: "100%" }} />
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell arkreflect-grid-pane" sx={{ p: 1.2, minHeight: 360 }}>
            <Stack direction="row" spacing={1} sx={{ alignItems: "center", px: 0.4 }}>
              <MonitorHeartRoundedIcon color="info" />
              <Box>
                <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                  Background agent lane
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  Apps, goals, watchers, Sentinel, ArkPulse, and ArkEvolve.
                </Typography>
              </Box>
            </Stack>
            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: {
                  xs: "repeat(2, minmax(0, 1fr))",
                  md: "repeat(3, minmax(0, 1fr))",
                },
                gap: 1,
                mt: 1.2,
              }}
            >
              {(["app", "goal", "watcher", "sentinel", "arkpulse", "arkevolve"] as const).map(
                (source) => {
                  const meta = sourceMeta(source);
                  const count = countForSource(response, source);
                  const active = count > 0;
                  const buckets = backgroundSparklines[source] ?? [];
                  const bucketMax = buckets.reduce((m, v) => (v > m ? v : m), 0);
                  const showSparkline = active && bucketMax > 0;
                  return (
                    <Box
                      key={source}
                      sx={{
                        p: 1.2,
                        borderRadius: "8px",
                        border: `1px solid ${active ? `${meta.color}55` : "rgba(255,255,255,0.08)"}`,
                        background: active
                          ? `linear-gradient(180deg, ${meta.color}1f, rgba(7,13,18,0.4))`
                          : "rgba(255,255,255,0.02)",
                        minWidth: 0,
                        position: "relative",
                        overflow: "hidden",
                      }}
                    >
                      <Stack
                        direction="row"
                        spacing={0.8}
                        sx={{ alignItems: "center", mb: 0.6 }}
                      >
                        <Box
                          sx={{
                            width: 10,
                            height: 10,
                            borderRadius: "50%",
                            flex: "0 0 auto",
                            background: active ? meta.color : "rgba(255,255,255,0.2)",
                            boxShadow: active ? `0 0 10px ${meta.color}` : "none",
                          }}
                        />
                        <Typography
                          variant="caption"
                          sx={{
                            color: active ? "#edf7f4" : "rgba(255,255,255,0.42)",
                            textTransform: "uppercase",
                            letterSpacing: "0.08em",
                            fontWeight: 700,
                            fontSize: "0.68rem",
                            whiteSpace: "nowrap",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                          }}
                        >
                          {meta.label}
                        </Typography>
                      </Stack>
                      <Stack
                        direction="row"
                        spacing={1}
                        sx={{ alignItems: "flex-end", justifyContent: "space-between" }}
                      >
                        <Typography
                          variant="h5"
                          sx={{
                            fontWeight: 700,
                            color: active ? meta.color : "rgba(255,255,255,0.3)",
                            fontVariantNumeric: "tabular-nums",
                            lineHeight: 1,
                            fontFamily: "var(--font-display)",
                          }}
                        >
                          {count}
                        </Typography>
                        {showSparkline ? (
                          <Box
                            sx={{
                              display: "flex",
                              alignItems: "flex-end",
                              gap: "3px",
                              height: 20,
                              flex: "0 1 auto",
                              minWidth: 56,
                            }}
                          >
                            {buckets.map((value, idx) => {
                              const heightPct =
                                value > 0
                                  ? Math.max(18, Math.round((value / bucketMax) * 100))
                                  : 10;
                              return (
                                <Box
                                  key={idx}
                                  sx={{
                                    width: 4,
                                    height: `${heightPct}%`,
                                    borderRadius: "1px",
                                    background:
                                      value > 0 ? meta.color : `${meta.color}33`,
                                    opacity: value > 0 ? 0.92 : 0.35,
                                    boxShadow:
                                      value > 0 ? `0 0 4px ${meta.color}` : "none",
                                    transition:
                                      "height 240ms ease, opacity 240ms ease",
                                  }}
                                  title={`Bucket ${idx + 1}: ${value} event${value === 1 ? "" : "s"}`}
                                />
                              );
                            })}
                          </Box>
                        ) : null}
                      </Stack>
                    </Box>
                  );
                },
              )}
            </Box>
          </Box>
        </Grid2>
      </Grid2>

      <Grid2 container spacing={1.2}>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell arkreflect-grid-pane" sx={{ p: 1.2, minHeight: 168 }}>
            <Typography variant="subtitle1" sx={{ fontWeight: 800, px: 0.4 }}>
              Timeline ribbon
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ px: 0.4 }}>
              The rhythm of this period.
            </Typography>
            {allUnits.length > 0 ? (
              <ReactECharts option={activityOption} style={{ height: 110, width: "100%" }} />
            ) : (
              <Box sx={{ height: 110, display: "grid", placeItems: "center", textAlign: "center" }}>
                <Typography color="text.secondary">No rhythm to show yet.</Typography>
              </Box>
            )}
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <Box className="list-shell arkreflect-grid-pane" sx={{ p: 1.2, minHeight: 330 }}>
            <Stack direction="row" spacing={1} sx={{ alignItems: "center", px: 0.4 }}>
              <DonutLargeRoundedIcon color="warning" />
              <Box>
                <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                  What contributed
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  The sources behind the recap.
                </Typography>
              </Box>
            </Stack>
            {sourceRows.length > 0 ? (
              <ReactECharts option={sourceDonutOption} style={{ height: 265, width: "100%" }} />
            ) : (
              <Box sx={{ height: 265, display: "grid", placeItems: "center", textAlign: "center" }}>
                <Typography color="text.secondary">Sources will appear after the recap is prepared.</Typography>
              </Box>
            )}
          </Box>
        </Grid2>
      </Grid2>

      <Accordion
        disableGutters
        sx={{
          bgcolor: "rgba(255,255,255,0.035)",
          border: "1px solid rgba(255,255,255,0.1)",
          borderRadius: "8px !important",
          color: "text.primary",
          boxShadow: "none",
          "&:before": { display: "none" },
        }}
      >
        <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
          <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
            <WorkHistoryRoundedIcon color="info" fontSize="small" />
            <Typography sx={{ fontWeight: 800 }}>Examples and evidence</Typography>
            <Chip size="small" label={`${totalUnits} item${totalUnits === 1 ? "" : "s"}`} />
          </Stack>
        </AccordionSummary>
        <AccordionDetails sx={{ pt: 0.5, pb: 1 }}>
          {(() => {
            const MAX_CLUSTERS = 6;
            const ranked = [...clusters].sort((a, b) => b.unit_count - a.unit_count).slice(0, MAX_CLUSTERS);
            const hiddenClusters = clusters.length - ranked.length;
            const monoFont = "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace";
            const headerCellSx = {
              fontSize: 8.5,
              fontFamily: monoFont,
              color: "rgba(180,210,225,0.42)",
              letterSpacing: 1.2,
              fontWeight: 600,
              textTransform: "uppercase" as const,
              py: 0.5,
            };
            return (
              <Box>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: "28px 44px 1fr 56px 88px 96px",
                    columnGap: 1.2,
                    px: 0.8,
                    borderBottom: "1px solid rgba(120,200,220,0.14)",
                    alignItems: "center",
                  }}
                >
                  <Box sx={headerCellSx}>idx</Box>
                  <Box sx={headerCellSx}>code</Box>
                  <Box sx={headerCellSx}>cluster</Box>
                  <Box sx={{ ...headerCellSx, textAlign: "right" }}>units</Box>
                  <Box sx={headerCellSx}>recent</Box>
                  <Box sx={headerCellSx}>signal</Box>
                </Box>
                {ranked.map((cluster, i) => {
                  const source = dominantSource(cluster);
                  const meta = sourceMeta(source);
                  const stroke = tacticalAccent(meta.color);
                  const code = tacticalCode(source);
                  const idx = String(i + 1).padStart(2, "0");
                  const dedup = new Set<string>();
                  let uniqueCount = 0;
                  let mostRecent = 0;
                  for (const u of cluster.units) {
                    const k = unitDisplayTitle(u).trim().toLowerCase();
                    if (!dedup.has(k)) {
                      dedup.add(k);
                      uniqueCount += 1;
                    }
                    const ts = Date.parse(u.occurred_at);
                    if (Number.isFinite(ts) && ts > mostRecent) mostRecent = ts;
                  }
                  const recentLabel = mostRecent > 0
                    ? formatUiDateTime(new Date(mostRecent).toISOString(), { fallback: "—" })
                    : "—";
                  const recurring = cluster.related_history.mode === "recurring";
                  const title = (clusterLabelById[cluster.id] ?? clusterDisplayLabel(cluster)).trim();
                  return (
                    <Box
                      key={cluster.id}
                      sx={{
                        display: "grid",
                        gridTemplateColumns: "28px 44px 1fr 56px 88px 96px",
                        columnGap: 1.2,
                        px: 0.8,
                        py: 0.7,
                        alignItems: "center",
                        borderBottom: "1px solid rgba(120,200,220,0.06)",
                        transition: "background 160ms ease",
                        "&:hover": { background: "rgba(120,200,220,0.04)" },
                      }}
                    >
                      <Box sx={{ fontSize: 9.5, fontFamily: monoFont, color: "rgba(180,210,225,0.5)", letterSpacing: 0.6 }}>
                        {idx}
                      </Box>
                      <Box
                        sx={{
                          fontSize: 9.5,
                          fontFamily: monoFont,
                          color: stroke,
                          letterSpacing: 0.8,
                          fontWeight: 600,
                          border: `1px solid ${stroke}`,
                          borderRadius: 0.5,
                          px: 0.5,
                          py: 0.1,
                          textAlign: "center",
                          width: "fit-content",
                          opacity: 0.92,
                        }}
                      >
                        {code}
                      </Box>
                      <Box sx={{ minWidth: 0 }}>
                        <Typography
                          sx={{
                            fontSize: 12,
                            fontFamily: monoFont,
                            fontWeight: 600,
                            color: "rgba(232,242,250,0.92)",
                            letterSpacing: 0.3,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {title.toUpperCase()}
                        </Typography>
                        {uniqueCount < cluster.unit_count ? (
                          <Typography
                            sx={{
                              fontSize: 9,
                              fontFamily: monoFont,
                              color: "rgba(180,210,225,0.4)",
                              letterSpacing: 0.4,
                              mt: 0.1,
                            }}
                          >
                            {uniqueCount} unique · {cluster.unit_count - uniqueCount} repeat{cluster.unit_count - uniqueCount === 1 ? "" : "s"}
                          </Typography>
                        ) : null}
                      </Box>
                      <Box
                        sx={{
                          fontSize: 11,
                          fontFamily: monoFont,
                          color: stroke,
                          fontWeight: 700,
                          textAlign: "right",
                          letterSpacing: 0.4,
                        }}
                      >
                        {String(cluster.unit_count).padStart(3, "0")}
                      </Box>
                      <Box
                        sx={{
                          fontSize: 9.5,
                          fontFamily: monoFont,
                          color: "rgba(180,210,225,0.55)",
                          letterSpacing: 0.4,
                          whiteSpace: "nowrap",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                        }}
                      >
                        {recentLabel}
                      </Box>
                      <Box
                        sx={{
                          fontSize: 9,
                          fontFamily: monoFont,
                          color: recurring ? stroke : "rgba(180,210,225,0.45)",
                          letterSpacing: 0.6,
                          fontWeight: 500,
                        }}
                      >
                        {recurring ? "◆ RECURRING" : "◇ NEW"}
                      </Box>
                    </Box>
                  );
                })}
                <Box
                  sx={{
                    display: "flex",
                    justifyContent: "space-between",
                    px: 0.8,
                    pt: 1,
                    fontSize: 8.5,
                    fontFamily: monoFont,
                    color: "rgba(180,210,225,0.42)",
                    letterSpacing: 0.8,
                  }}
                >
                  <span>◢ TOP {ranked.length} BY ACTIVITY</span>
                  <span>
                    {hiddenClusters > 0
                      ? `${hiddenClusters} CLUSTER${hiddenClusters === 1 ? "" : "S"} OMITTED · ${totalUnits} TOTAL UNITS`
                      : `${totalUnits} TOTAL UNITS`}
                  </span>
                </Box>
              </Box>
            );
          })()}
        </AccordionDetails>
      </Accordion>

      {response?.generated_at ? (
        <Typography variant="caption" color="text.secondary" sx={{ px: 0.5 }}>
          Cached view generated {formatUiDateTime(response.generated_at, { fallback: response.generated_at })}
          {response.refresh_status.completed_at
            ? ` - Last background refresh ${formatUiDateTime(response.refresh_status.completed_at, { fallback: response.refresh_status.completed_at })}`
            : ""}
        </Typography>
      ) : null}
    </WorkspacePageShell>
  );
}
