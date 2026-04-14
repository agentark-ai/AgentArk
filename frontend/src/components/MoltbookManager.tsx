import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Link,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiDateTimeMeta, formatUiRelativeDateTimeMeta } from "../lib/dateFormat";
import { WorkspacePageHeader, WorkspacePageShell } from "./WorkspacePage";

const REFRESH_MS = 8000;

type JsonRecord = Record<string, unknown>;

type MoltbookFormState = {
  moltbook_api_key: string;
  moltbook_enabled: boolean;
  moltbook_mode: string;
  moltbook_sync_frequency: string;
  moltbook_write_enabled: boolean;
  moltbook_defer_when_busy: boolean;
  moltbook_model_slot_id: string;
};

type MoltbookModelOption = {
  value: string;
  label: string;
  helper: string;
};

type RunCounts = {
  readCount: number;
  commentCount: number;
  upvoteCount: number;
  postCount: number;
  stepCount: number;
};

type RunGroup = {
  id: string;
  representative: JsonRecord;
  events: JsonRecord[];
  counts: RunCounts;
  level: "error" | "warning" | "success";
  summary: string;
  trigger: string;
};

type LinkEntry = {
  label: string;
  url: string;
};

type StatusDetailItem = {
  label: string;
  value: string;
  title?: string;
  renderMeta?: () => React.ReactNode;
};

const SCHEDULE_PRESETS = [
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

const SCHEDULE_PRESET_VALUES = new Set<string>(SCHEDULE_PRESETS.map((item) => item.value));

const PARTICIPATION_MODES = [
  {
    value: "autopost",
    label: "Engage",
    shortLabel: "Recommended default",
    description:
      "AgentArk reads the feed, replies when useful, upvotes strong work, and creates new posts when there is something worth contributing."
  },
  {
    value: "assist",
    label: "Assist",
    shortLabel: "Interactive only",
    description:
      "AgentArk reads Moltbook continuously, and on manual runs it can reply, vote, and draft contributions without scheduling those actions automatically."
  },
  {
    value: "read_only",
    label: "Read Only",
    shortLabel: "Observe only",
    description: "AgentArk fetches posts for awareness and internal context, but never replies, votes, or posts."
  },
  {
    value: "off",
    label: "Off",
    shortLabel: "Disabled",
    description: "The connector stays registered but stops sync activity and engagement."
  }
] as const;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function pickRecords(value: unknown, ...keys: string[]): JsonRecord[] {
  if (Array.isArray(value)) return value.filter(isRecord);
  if (!isRecord(value)) return [];
  for (const key of keys) {
    const candidate = value[key];
    if (Array.isArray(candidate)) return candidate.filter(isRecord);
  }
  return [];
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : fallback;
  }
  return fallback;
}

function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return ["1", "true", "yes", "on", "enabled"].includes(normalized);
  }
  return false;
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "Something went wrong.";
}

function humanTs(value: unknown): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(typeof value === "string" ? value : "", { fallback: "-" });
}

function absoluteTs(value: unknown): { label: string; tip: string } {
  return formatUiDateTimeMeta(typeof value === "string" ? value : "", { fallback: "-" });
}

function humanizeKey(value: unknown): string {
  const raw = str(value, "").trim();
  if (!raw) return "-";
  return raw.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
}

function createFormState(settings: JsonRecord): MoltbookFormState {
  return {
    moltbook_api_key: "",
    moltbook_enabled: toBool(settings.moltbook_enabled),
    moltbook_mode: str(settings.moltbook_mode, "autopost"),
    moltbook_sync_frequency: str(settings.moltbook_sync_frequency, "every_12_hours"),
    moltbook_write_enabled:
      settings.moltbook_write_enabled == null ? true : toBool(settings.moltbook_write_enabled),
    moltbook_defer_when_busy: toBool(settings.moltbook_defer_when_busy),
    moltbook_model_slot_id: str(settings.moltbook_model_slot_id, "")
  };
}

function snapshotFormState(value: MoltbookFormState): string {
  return JSON.stringify(value);
}

function eventTsValue(event: JsonRecord): number {
  const raw = str(event.timestamp, "").trim();
  if (!raw) return 0;
  const parsed = Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function triggerLabel(raw: string): string {
  const normalized = raw.trim().toLowerCase();
  if (normalized === "manual") return "Manual";
  if (normalized === "scheduler") return "Scheduled";
  return raw || "-";
}

function toolActionName(raw: string): string {
  const normalized = raw.trim().toLowerCase();
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
  return normalized ? normalized.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase()) : "Tool call";
}

function actionLabel(action: string, details: JsonRecord): string {
  const normalized = action.trim().toLowerCase();
  if (normalized === "skipped_disabled") return "Skipped: Disabled";
  if (normalized === "skipped_off_mode") return "Skipped: Mode off";
  if (normalized === "deferred_busy") return "Deferred: Busy";
  if (normalized === "skipped_busy_max_defers") return "Skipped: Busy (max defers)";
  if (normalized === "not_connected") return "Not connected";
  if (normalized === "run_started") return "Run started";
  if (normalized === "run_completed") return "Run completed";
  if (normalized === "status_checked") return "Status checked";
  if (normalized === "engagement_plan_created") return "Engagement planned";
  if (normalized === "engagement_plan_fallback") return "Fallback plan used";
  if (normalized === "engagement_skipped_mode") return "Engagement skipped";
  if (normalized === "engagement_skipped_disabled") return "Engagement disabled";
  if (normalized === "engagement_skipped_empty_feed") return "No feed items";
  if (normalized === "engagement_skipped_not_needed") return "No action needed";
  if (normalized === "feed_fetched" || normalized === "feed_read") return "Feed fetched";
  if (normalized === "post_created") return "Post created";
  if (normalized === "comment_created") return "Comment created";
  if (normalized === "comment_failed") return "Comment failed";
  if (normalized === "post_upvoted") return "Post upvoted";
  if (normalized === "upvote_failed") return "Upvote failed";
  if (normalized.startsWith("tool_")) {
    return `Tool call: ${toolActionName(str(details.sub_action, normalized.slice(5)))}`;
  }
  if (normalized.startsWith("error_")) return `Error: ${action}`;
  return action || "-";
}

function actionSummary(action: string, details: JsonRecord): string | null {
  const normalized = action.trim().toLowerCase();
  const summaryPreview = str(details.summary_preview, str(details.summary, "")).trim();
  const contentPreview = str(details.content_preview, "").trim();
  const errorPreview = str(details.error, "").trim();
  if (normalized === "run_completed") {
    const readCount = num(details.read_count, 0);
    const commentCount = num(details.comment_count, 0);
    const upvoteCount = num(details.upvote_count, 0);
    const postCount = num(details.post_count, toBool(details.posted) ? 1 : 0);
    const parts = [`Read ${readCount} post${readCount === 1 ? "" : "s"}`];
    if (commentCount > 0) parts.push(`${commentCount} comment${commentCount === 1 ? "" : "s"}`);
    if (upvoteCount > 0) parts.push(`${upvoteCount} upvote${upvoteCount === 1 ? "" : "s"}`);
    if (postCount > 0) parts.push(`${postCount} new post${postCount === 1 ? "" : "s"}`);
    if (commentCount + upvoteCount + postCount === 0) parts.push("no public action taken");
    return parts.join(" | ");
  }
  if (normalized === "engagement_plan_created" || normalized === "engagement_plan_fallback") {
    return summaryPreview || "Prepared an engagement plan.";
  }
  if (
    normalized === "engagement_skipped_mode" ||
    normalized === "engagement_skipped_disabled" ||
    normalized === "engagement_skipped_empty_feed" ||
    normalized === "engagement_skipped_not_needed"
  ) {
    return str(details.reason, "").trim() || "No engagement action was taken.";
  }
  if (normalized === "feed_read" || normalized === "feed_fetched") {
    const readCount = num(details.count, num(details.read_count, 0));
    return readCount > 0
      ? `Fetched ${readCount} recent post${readCount === 1 ? "" : "s"}.`
      : "Fetched the feed but found no posts.";
  }
  if (normalized === "post_created") {
    return str(asRecord(details.request).title, "").trim() || "Published a new Moltbook post.";
  }
  if (normalized === "comment_created") {
    return contentPreview || "Posted a reply on Moltbook.";
  }
  if (normalized === "comment_failed") {
    return errorPreview || "Could not post the Moltbook reply.";
  }
  if (normalized === "post_upvoted") {
    const postId = str(details.post_id, "").trim();
    return postId ? `Upvoted post ${postId}` : "Upvoted a Moltbook post.";
  }
  if (normalized === "upvote_failed") {
    return errorPreview || "Could not upvote the Moltbook post.";
  }
  if (normalized === "memory_saved") {
    return summaryPreview || "Saved a Moltbook feed summary to memory.";
  }
  if (normalized === "memory_save_failed") {
    return errorPreview || "Could not save the Moltbook summary to memory.";
  }
  if (normalized.startsWith("tool_")) {
    return errorPreview || null;
  }
  return null;
}

function actionReason(action: string, details: JsonRecord): string | null {
  const explicit = str(details.reason, "").trim();
  if (explicit) return explicit;
  const normalized = action.trim().toLowerCase();
  if (normalized === "skipped_disabled") return "Moltbook is disabled on this page.";
  if (normalized === "skipped_off_mode") return "Moltbook mode is set to off.";
  if (normalized === "deferred_busy") return "Deferred because the server was busy.";
  if (normalized === "engagement_skipped_empty_feed") return "There was nothing new in the feed to engage with.";
  if (normalized === "engagement_skipped_not_needed") return "The current feed did not justify a public action.";
  if (normalized === "not_connected") {
    const status = str(details.status, "").toLowerCase();
    const error = str(details.error, "").trim();
    if (status === "not_configured") {
      return "Moltbook API key is not configured. Enter it on this page and save.";
    }
    return error || "Could not connect to Moltbook.";
  }
  if (normalized.startsWith("tool_")) {
    const error = str(details.error, "").trim();
    if (error) return `Tool call failed: ${error}`;
  }
  return null;
}

function derivePostUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const raw = value.trim();
  if (!raw) return null;
  if (raw.startsWith("https://www.moltbook.com/post/")) return raw;
  const match = raw.match(/\/api\/v1\/posts\/([0-9a-f-]+)/i);
  return match?.[1] ? `https://www.moltbook.com/post/${match[1]}` : null;
}

function collectLinks(details: JsonRecord): LinkEntry[] {
  const out: LinkEntry[] = [];
  const seen = new Set<string>();
  const push = (label: string, urlLike: unknown) => {
    if (typeof urlLike !== "string") return;
    const url = urlLike.trim();
    if (!url.startsWith("http://") && !url.startsWith("https://")) return;
    if (seen.has(url)) return;
    seen.add(url);
    out.push({ label, url });
  };
  push("Claim URL", details.claim_url);
  push("Article URL", details.article_url);
  push("Post URL", details.post_url);
  push("URL", details.url);
  push("Post URL", derivePostUrl(details.post_api_url));
  push("Post URL", derivePostUrl(details.api_url));
  const readPosts = Array.isArray(details.read_posts) ? details.read_posts : [];
  for (const entry of readPosts.slice(0, 4)) {
    if (!isRecord(entry)) continue;
    const title = str(entry.title, "").trim();
    push(title ? `Post: ${title}` : "Feed post", entry.url);
    push(title ? `Post: ${title}` : "Feed post", derivePostUrl(entry.post_api_url));
  }
  return out;
}

function runCounts(events: JsonRecord[]): RunCounts {
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

function runTrigger(events: JsonRecord[]): string {
  for (const event of events) {
    const trigger = str(asRecord(event.details).trigger, "").trim();
    if (trigger) return trigger;
  }
  return "";
}

function runLevel(events: JsonRecord[]): "error" | "warning" | "success" {
  const levels = events.map((event) => str(event.level, "").toLowerCase());
  if (levels.some((level) => level === "error")) return "error";
  if (levels.some((level) => level === "warning" || level === "warn")) return "warning";
  return "success";
}

function representativeEvent(events: JsonRecord[]): JsonRecord | null {
  if (events.length === 0) return null;
  const completed = events
    .filter((event) => str(event.action, "").toLowerCase() === "run_completed")
    .sort((left, right) => eventTsValue(right) - eventTsValue(left))[0];
  if (completed) return completed;
  return events.slice().sort((left, right) => eventTsValue(right) - eventTsValue(left))[0] ?? null;
}

function runSummary(events: JsonRecord[]): string {
  const completed = events.find((event) => str(event.action, "").toLowerCase() === "run_completed");
  if (completed) {
    return actionSummary("run_completed", asRecord(completed.details)) || "Run completed.";
  }
  const counts = runCounts(events);
  const parts: string[] = [];
  if (counts.readCount > 0) parts.push(`Read ${counts.readCount} post${counts.readCount === 1 ? "" : "s"}`);
  if (counts.commentCount > 0) {
    parts.push(`${counts.commentCount} comment${counts.commentCount === 1 ? "" : "s"}`);
  }
  if (counts.upvoteCount > 0) parts.push(`${counts.upvoteCount} like${counts.upvoteCount === 1 ? "" : "s"}`);
  if (counts.postCount > 0) parts.push(`${counts.postCount} post${counts.postCount === 1 ? "" : "s"} created`);
  if (parts.length > 0) return parts.join(" | ");
  const representative = representativeEvent(events);
  return actionSummary(str(representative?.action, ""), asRecord(representative?.details)) || "Run recorded.";
}

function buildRunGroups(events: JsonRecord[]): RunGroup[] {
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
  return Array.from(grouped.entries())
    .map(([id, group]) => {
      const sorted = group.slice().sort((left, right) => eventTsValue(left) - eventTsValue(right));
      const representative = representativeEvent(sorted);
      if (!representative) return null;
      return {
        id,
        representative,
        events: sorted,
        counts: runCounts(sorted),
        level: runLevel(sorted),
        summary: runSummary(sorted),
        trigger: runTrigger(sorted)
      };
    })
    .filter((group): group is RunGroup => group != null)
    .filter((group) => group.trigger || group.events.some((event) => {
      const action = str(event.action, "").toLowerCase();
      return action === "run_started" || action === "run_completed";
    }))
    .sort((left, right) => eventTsValue(right.representative) - eventTsValue(left.representative));
}

export function MoltbookManager({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<MoltbookFormState>(() => createFormState({}));
  const [savedSnapshot, setSavedSnapshot] = useState(() => snapshotFormState(createFormState({})));
  const [initialized, setInitialized] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<RunGroup | null>(null);
  const [pollState, setPollState] = useState<{ baselineEventId: string; deadlineAt: number } | null>(null);

  const settingsQ = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.rawGet("/settings"),
    refetchInterval: false,
    refetchOnWindowFocus: false
  });
  const statusQ = useQuery({
    queryKey: ["moltbook-status"],
    queryFn: () => api.rawGet("/moltbook/status"),
    refetchInterval: pollState ? 2000 : autoRefresh ? REFRESH_MS : false
  });
  const logQ = useQuery({
    queryKey: ["moltbook-log"],
    queryFn: () => api.rawGet("/moltbook/log?limit=500"),
    refetchInterval: pollState ? 2000 : autoRefresh ? REFRESH_MS : false
  });

  const settings = asRecord(settingsQ.data);
  const status = asRecord(statusQ.data);
  const events = pickRecords(logQ.data, "events");
  const latestEventId = str(asRecord(events[0]).id, "");
  const runs = buildRunGroups(events);
  const lastStatus = str(status.last_status, "").toLowerCase();
  const lastRunStats = asRecord(status.last_run_stats);
  const hasStoredApiKey = toBool(settings.moltbook_has_api_key) || toBool(status.has_api_key);
  const needsConnection = !hasStoredApiKey;
  const hasConnectionIssue = hasStoredApiKey && (lastStatus === "not_connected" || lastStatus === "error");
  const isRunning = toBool(status.running);
  const runBusy = isRunning || Boolean(pollState);
  const scheduleMode = SCHEDULE_PRESET_VALUES.has(form.moltbook_sync_frequency)
    ? form.moltbook_sync_frequency
    : "__custom__";
  const selectedMode =
    PARTICIPATION_MODES.find((option) => option.value === form.moltbook_mode) ?? PARTICIPATION_MODES[0];
  const enabledModelSlots = useMemo(
    () =>
      pickRecords(settings, "model_pool").filter(
        (slot) => toBool(slot.enabled) && str(slot.id, "").trim()
      ),
    [settingsQ.data]
  );
  const modelOptions = useMemo<MoltbookModelOption[]>(() => {
    const options: MoltbookModelOption[] = [
      {
        value: "",
        label: "Use primary model",
        helper: "Follows the primary/default model from the Models tab."
      }
    ];
    for (const slot of enabledModelSlots) {
      const id = str(slot.id, "").trim();
      if (!id) continue;
      const label = str(slot.label, "").trim() || humanizeKey(slot.role);
      const provider = str(slot.provider, "").trim();
      const model = str(slot.model, "").trim();
      const role = humanizeKey(slot.role);
      options.push({
        value: id,
        label,
        helper: [role, provider, model].filter(Boolean).join(" | ")
      });
    }
    return options;
  }, [enabledModelSlots]);
  const showModelSelector = enabledModelSlots.length > 1;
  const normalizedModelSlotId = modelOptions.some((option) => option.value === form.moltbook_model_slot_id)
    ? form.moltbook_model_slot_id
    : "";
  const selectedModelOption =
    modelOptions.find((option) => option.value === normalizedModelSlotId) ?? modelOptions[0];
  const scheduleLabel =
    scheduleMode === "__custom__"
      ? `Custom: ${form.moltbook_sync_frequency || "-"}`
      : SCHEDULE_PRESETS.find((option) => option.value === form.moltbook_sync_frequency)?.label ||
        form.moltbook_sync_frequency;

  useEffect(() => {
    if (!success) return;
    const timer = window.setTimeout(() => setSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [success]);

  useEffect(() => {
    if (!settingsQ.data) return;
    const next = createFormState(asRecord(settingsQ.data));
    const nextSnapshot = snapshotFormState(next);
    if (!initialized || !dirty) {
      setForm(next);
      setSavedSnapshot(nextSnapshot);
      setDirty(false);
      setInitialized(true);
    }
  }, [dirty, initialized, settingsQ.data]);

  useEffect(() => {
    if (!pollState) return;
    if (Date.now() >= pollState.deadlineAt) {
      setPollState(null);
      return;
    }
    if (!isRunning && latestEventId && latestEventId !== pollState.baselineEventId) {
      setPollState(null);
    }
  }, [isRunning, latestEventId, pollState]);

  const saveMutation = useMutation({
    mutationFn: () =>
      api.rawPost("/settings", {
        moltbook_api_key: form.moltbook_api_key || null,
        moltbook_enabled: form.moltbook_enabled,
        moltbook_mode: form.moltbook_mode || null,
        moltbook_sync_frequency: form.moltbook_sync_frequency || null,
        moltbook_write_enabled: form.moltbook_write_enabled,
        moltbook_defer_when_busy: form.moltbook_defer_when_busy,
        moltbook_model_slot_id: normalizedModelSlotId
      }),
    onSuccess: async () => {
      const next = { ...form, moltbook_api_key: "" };
      setForm(next);
      setSavedSnapshot(snapshotFormState(next));
      setDirty(false);
      setError(null);
      setSuccess("Saved Moltbook settings.");
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({ queryKey: ["moltbook-status"] });
      await queryClient.invalidateQueries({ queryKey: ["moltbook-log"] });
    },
    onError: (mutationError) => {
      setSuccess(null);
      setError(errMessage(mutationError));
    }
  });

  const runMutation = useMutation({
    mutationFn: () => api.rawPost("/moltbook/run", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["moltbook-status"] });
      await queryClient.invalidateQueries({ queryKey: ["moltbook-log"] });
    }
  });

  const updateForm = (updater: (current: MoltbookFormState) => MoltbookFormState) => {
    setForm((current) => {
      const next = updater(current);
      setDirty(snapshotFormState(next) !== savedSnapshot);
      return next;
    });
    setError(null);
    setSuccess(null);
  };

  const setField = <K extends keyof MoltbookFormState>(key: K, value: MoltbookFormState[K]) => {
    updateForm((current) => ({ ...current, [key]: value }));
  };

  const resetForm = () => {
    const next = createFormState(settings);
    setForm(next);
    setSavedSnapshot(snapshotFormState(next));
    setDirty(false);
    setError(null);
    setSuccess(null);
  };

  const runNow = async () => {
    setError(null);
    setSuccess(null);
    try {
      const out = asRecord(await runMutation.mutateAsync());
      const runStatus = str(out.status, "ok").toLowerCase();
      if (runStatus === "ok") {
        setSuccess(`Moltbook run completed. ${runSummary([out])}`);
        return;
      }
      if (runStatus === "started" || runStatus === "running") {
        setPollState({
          baselineEventId: latestEventId,
          deadlineAt: Date.now() + 3 * 60 * 1000
        });
        setSuccess(
          runStatus === "started"
            ? "Moltbook run started in the background. Watch recent runs for completion."
            : str(out.message, "Moltbook run is already in progress.")
        );
        return;
      }
      if (runStatus === "not_connected") {
        const detail = str(out.status_detail, "").toLowerCase();
        const reason = str(out.reason, "").trim();
        if (detail === "not_configured" || !hasStoredApiKey) {
          setError(reason || "No Moltbook API key configured. Save your key first, then run.");
        } else {
          setError(reason || "Stored Moltbook key could not connect. Check the key or claim status, then run again.");
        }
        return;
      }
      if (runStatus === "disabled") {
        setError("Moltbook is disabled.");
        return;
      }
      if (runStatus === "off_mode") {
        setError("Moltbook mode is off.");
        return;
      }
      setSuccess(`Status: ${runStatus}`);
    } catch (runError) {
      setError(errMessage(runError));
    }
  };

  const connectionChip = !form.moltbook_enabled
    ? { label: "Disabled", color: "default" as const, helper: "Sync is turned off." }
    : needsConnection
      ? { label: "Not connected", color: "warning" as const, helper: "Save an API key to connect." }
      : hasConnectionIssue
        ? { label: "Check connection", color: "warning" as const, helper: "Stored key could not authenticate." }
        : lastStatus === "ok"
          ? { label: "Connected", color: "success" as const, helper: "Last status check succeeded." }
          : { label: "Configured", color: "success" as const, helper: "Key is saved and ready." };

  const statItems = [
    { label: "Connection", value: connectionChip.label, helper: connectionChip.helper },
    { label: "Mode", value: selectedMode.label, helper: selectedMode.shortLabel },
    {
      label: "Schedule",
      value: form.moltbook_enabled ? scheduleLabel : "Off",
      helper: form.moltbook_enabled ? `Next run ${humanTs(status.next_run_at).label}` : "Enable Moltbook to schedule runs."
    },
    {
      label: "Recorded Runs",
      value: String(runs.length),
      helper:
        runs.length > 0
          ? `Last run ${absoluteTs(runs[0].representative.timestamp).label}`
          : "No Moltbook runs recorded yet."
    }
  ];
  const connectorStatusDetails: StatusDetailItem[] = [
    {
      label: "Last run",
      value: humanTs(status.last_run_at).label,
      title: humanTs(status.last_run_at).tip
    },
    {
      label: "Next run",
      value: humanTs(status.next_run_at).label,
      title: humanTs(status.next_run_at).tip
    },
    {
      label: "Last engagement",
      value: humanTs(status.last_engagement_at).label,
      title: humanTs(status.last_engagement_at).tip
    },
    {
      label: "Latest summary",
      value: "",
      renderMeta: () => (
        <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
          <Chip size="small" variant="outlined" label={`${num(lastRunStats.read_count, 0)} read`} />
          <Chip size="small" variant="outlined" label={`${num(lastRunStats.comment_count, 0)} comments`} />
          <Chip size="small" variant="outlined" label={`${num(lastRunStats.upvote_count, 0)} upvotes`} />
          <Chip size="small" variant="outlined" label={`${num(lastRunStats.post_count, toBool(lastRunStats.posted) ? 1 : 0)} posts`} />
        </Stack>
      )
    }
  ];

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Core"
        title="Moltbook"
        description="Federated agent-to-agent participation, schedule, and run history for Moltbook."
        actions={
          <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
            <Chip size="small" color={connectionChip.color} label={connectionChip.label} />
            <Chip size="small" variant="outlined" color={dirty ? "warning" : "default"} label={dirty ? "Unsaved changes" : "Saved"} />
            <Button size="small" variant="outlined" onClick={runNow} disabled={!form.moltbook_enabled || runBusy || runMutation.isPending}>
              {runBusy || runMutation.isPending ? "Running..." : "Run now"}
            </Button>
            <Button size="small" variant="outlined" onClick={resetForm} disabled={!dirty || saveMutation.isPending}>
              Reset
            </Button>
            <Button size="small" variant="contained" onClick={() => saveMutation.mutate()} disabled={saveMutation.isPending || !dirty}>
              {saveMutation.isPending ? "Saving..." : "Save"}
            </Button>
          </Stack>
        }
      />

      <Box className="list-shell stat-strip">
        {statItems.map((item) => (
          <div key={item.label} className="stat-strip-item">
            <span className="stat-strip-label">{item.label}</span>
            <span className="stat-strip-value">{item.value}</span>
            <span className="stat-strip-helper">{item.helper}</span>
          </div>
        ))}
      </Box>

      {settingsQ.error ? <Alert severity="error">{errMessage(settingsQ.error)}</Alert> : null}
      {error ? <Alert severity="error">{error}</Alert> : null}
      {success ? <Alert severity="success">{success}</Alert> : null}

      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 7 }} sx={{ display: "flex" }}>
          <Box className="list-shell" sx={{ width: "100%" }}>
            <Stack spacing={1.5}>
              <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", alignItems: { xs: "flex-start", sm: "center" } }}>
                <Box>
                  <Typography variant="h6">Participation</Typography>
                  <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.35 }}>
                    Control how AgentArk reads, reacts, and posts on Moltbook.
                  </Typography>
                </Box>
                <FormControlLabel control={<Switch checked={form.moltbook_enabled} onChange={(event) => setField("moltbook_enabled", event.target.checked)} />} label="Enabled" />
              </Stack>

              <TextField
                fullWidth
                size="small"
                type="password"
                label="API key"
                placeholder="mk-..."
                value={form.moltbook_api_key}
                onChange={(event) => setField("moltbook_api_key", event.target.value)}
              />
              <Typography variant="caption" sx={{ color: "text.secondary", mt: -0.5 }}>
                {hasStoredApiKey ? "Stored securely on the server. Leave this blank to keep the existing key." : "Required to connect. Get your key at moltbook.com."}
              </Typography>

              <Grid2 container spacing={1.25}>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField label="Participation mode" select fullWidth size="small" value={form.moltbook_mode} disabled={!form.moltbook_enabled} onChange={(event) => {
                    const next = event.target.value;
                    if (next === "autopost") {
                      updateForm((current) => ({ ...current, moltbook_mode: next, moltbook_write_enabled: true }));
                      return;
                    }
                    setField("moltbook_mode", next);
                  }}>
                    {PARTICIPATION_MODES.map((option) => (
                      <MenuItem key={option.value} value={option.value}>
                        {option.label}{option.value === "autopost" ? " (recommended)" : ""} - {option.shortLabel}
                      </MenuItem>
                    ))}
                  </TextField>
                </Grid2>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField label="Run schedule" select fullWidth size="small" value={scheduleMode} disabled={!form.moltbook_enabled} onChange={(event) => {
                    const next = event.target.value;
                    if (next === "__custom__") {
                      if (SCHEDULE_PRESET_VALUES.has(form.moltbook_sync_frequency)) {
                        setField("moltbook_sync_frequency", "0 0 */12 * * *");
                      }
                      return;
                    }
                    setField("moltbook_sync_frequency", next);
                  }}>
                    {SCHEDULE_PRESETS.map((option) => (
                      <MenuItem key={option.value} value={option.value}>{option.label}</MenuItem>
                    ))}
                    <MenuItem value="__custom__">Custom cron</MenuItem>
                  </TextField>
                </Grid2>
                {showModelSelector ? (
                  <Grid2 size={{ xs: 12 }}>
                    <TextField
                      label="Model for Moltbook"
                      select
                      fullWidth
                      size="small"
                      value={normalizedModelSlotId}
                      disabled={!form.moltbook_enabled}
                      onChange={(event) => setField("moltbook_model_slot_id", event.target.value)}
                      helperText={selectedModelOption.helper}
                    >
                      {modelOptions.map((option) => (
                        <MenuItem key={option.value || "__primary__"} value={option.value}>
                          {option.label}
                        </MenuItem>
                      ))}
                    </TextField>
                  </Grid2>
                ) : null}
                {scheduleMode === "__custom__" ? (
                  <Grid2 size={{ xs: 12 }}>
                    <TextField label="Custom cron" value={form.moltbook_sync_frequency} onChange={(event) => setField("moltbook_sync_frequency", event.target.value)} fullWidth size="small" placeholder="0 0 */6 * * *" disabled={!form.moltbook_enabled} />
                  </Grid2>
                ) : null}
                <Grid2 size={{ xs: 12 }}>
                  <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ alignItems: { xs: "stretch", sm: "center" } }}>
                    <FormControlLabel control={<Switch size="small" checked={form.moltbook_write_enabled} onChange={(event) => setField("moltbook_write_enabled", event.target.checked)} disabled={!form.moltbook_enabled} />} label={<Typography variant="body2">Allow autonomous engagement</Typography>} />
                    <FormControlLabel control={<Switch size="small" checked={form.moltbook_defer_when_busy} onChange={(event) => setField("moltbook_defer_when_busy", event.target.checked)} disabled={!form.moltbook_enabled} />} label={<Typography variant="body2">Defer when busy</Typography>} />
                  </Stack>
                </Grid2>
              </Grid2>

              <Box className="metadata-box">
                <Typography variant="caption" sx={{ color: "text.secondary" }}>Current behavior</Typography>
                <Typography variant="body2" sx={{ mt: 0.55 }}>{selectedMode.description}</Typography>
                {showModelSelector ? (
                  <Typography variant="caption" sx={{ color: "text.secondary", display: "block", mt: 0.75 }}>
                    Model: {selectedModelOption.label}
                  </Typography>
                ) : null}
              </Box>
            </Stack>
          </Box>
        </Grid2>

        <Grid2 size={{ xs: 12, lg: 5 }} sx={{ display: "flex" }}>
          <Box className="list-shell" sx={{ width: "100%" }}>
            <Stack spacing={1.25}>
              <Box>
                <Typography variant="h6">Connector status</Typography>
                <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.35 }}>
                  Health, last activity, and next scheduled run.
                </Typography>
              </Box>
              {statusQ.error ? <Alert severity="error">{errMessage(statusQ.error)}</Alert> : null}
              {form.moltbook_enabled && needsConnection ? <Alert severity="warning" variant="outlined">No Moltbook API key is configured. Add the key, save, then run.</Alert> : null}
              {form.moltbook_enabled && hasConnectionIssue ? <Alert severity="warning" variant="outlined">Stored Moltbook API key found, but the last connection attempt failed. Try <strong>Run now</strong> again or replace the key if it expired.</Alert> : null}
              <Box sx={{ borderTop: "1px solid rgba(255,255,255,0.08)" }}>
                {connectorStatusDetails.map((item, index) => (
                  <Box
                    key={item.label}
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", sm: "132px minmax(0, 1fr)" },
                      gap: { xs: 0.45, sm: 1.25 },
                      py: 1.05,
                      borderTop: index === 0 ? "none" : "1px solid rgba(255,255,255,0.06)",
                      alignItems: "start"
                    }}
                  >
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      {item.label}
                    </Typography>
                    <Box sx={{ minWidth: 0 }}>
                      {item.value ? (
                        <Typography variant="body2" title={item.title}>
                          {item.value}
                        </Typography>
                      ) : null}
                      {item.renderMeta ? (
                        <Box sx={{ mt: item.value ? 0.5 : 0 }}>
                          {item.renderMeta()}
                        </Box>
                      ) : null}
                    </Box>
                  </Box>
                ))}
              </Box>
            </Stack>
          </Box>
        </Grid2>
      </Grid2>

      <Box className="list-shell">
        <Typography variant="h6">Recent runs</Typography>
        <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.35, mb: 1 }}>
          Grouped run history with the action summary, trigger, and step count.
        </Typography>
        {logQ.error ? <Alert severity="error">{errMessage(logQ.error)}</Alert> : null}
        {runs.length === 0 ? (
          <Typography variant="body2" sx={{ color: "text.secondary" }}>No Moltbook runs yet.</Typography>
        ) : (
          <Stack spacing={0} sx={{ borderTop: "1px solid rgba(255,255,255,0.08)" }}>
            {runs.slice(0, 40).map((run, index) => {
              const action = actionLabel(str(run.representative.action, "-"), asRecord(run.representative.details));
              const timestamp = absoluteTs(run.representative.timestamp);
              return (
                <ButtonBase
                  key={run.id}
                  onClick={() => setSelectedRun(run)}
                  sx={{
                    width: "100%",
                    textAlign: "left",
                    justifyContent: "flex-start",
                    alignItems: "stretch",
                    px: 0,
                    py: 1.15,
                    borderTop: index === 0 ? "none" : "1px solid rgba(255,255,255,0.06)"
                  }}
                >
                  <Stack spacing={0.55} sx={{ width: "100%", minWidth: 0 }}>
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{ justifyContent: "space-between", alignItems: { sm: "center" } }}
                    >
                      <Stack direction="row" spacing={0.8} useFlexGap sx={{ flexWrap: "wrap", alignItems: "center", minWidth: 0 }}>
                        <Typography variant="body2" sx={{ fontWeight: 500 }}>
                          {action}
                        </Typography>
                        <Chip
                          size="small"
                          label={run.level}
                          color={run.level === "error" ? "error" : run.level === "warning" ? "warning" : "success"}
                        />
                      </Stack>
                      <Typography variant="caption" sx={{ color: "text.secondary" }} title={timestamp.tip}>
                        {timestamp.label}
                      </Typography>
                    </Stack>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      {run.summary}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      {run.counts.readCount} read | {run.counts.commentCount} commented | {run.counts.upvoteCount} liked | {run.counts.postCount} posted
                    </Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      {run.trigger ? `${triggerLabel(run.trigger)} | ` : ""}{run.counts.stepCount} step{run.counts.stepCount === 1 ? "" : "s"} | Run {run.id.slice(0, 8)}
                    </Typography>
                  </Stack>
                </ButtonBase>
              );
            })}
          </Stack>
        )}
      </Box>

      <Dialog open={selectedRun != null} onClose={() => setSelectedRun(null)} maxWidth="lg" fullWidth>
        <DialogTitle>Moltbook Run</DialogTitle>
        <DialogContent>
          {selectedRun ? (
            <Stack spacing={1.5} sx={{ pt: 0.5 }}>
              <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                <Typography variant="subtitle1">{actionLabel(str(selectedRun.representative.action, ""), asRecord(selectedRun.representative.details))}</Typography>
                <Chip size="small" label={selectedRun.level} color={selectedRun.level === "error" ? "error" : selectedRun.level === "warning" ? "warning" : "success"} />
              </Stack>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                <span title={absoluteTs(selectedRun.representative.timestamp).tip}>{absoluteTs(selectedRun.representative.timestamp).label}</span>
                {" | "}Run: {selectedRun.id}
                {" | "}{selectedRun.counts.stepCount} step{selectedRun.counts.stepCount === 1 ? "" : "s"}
              </Typography>
              <Grid2 container spacing={1}>
                <Grid2 size={{ xs: 6, md: 3 }}><Box className="metadata-box" sx={{ minHeight: 84 }}><Typography variant="caption" sx={{ color: "text.secondary" }}>Read</Typography><Typography variant="h6">{selectedRun.counts.readCount}</Typography></Box></Grid2>
                <Grid2 size={{ xs: 6, md: 3 }}><Box className="metadata-box" sx={{ minHeight: 84 }}><Typography variant="caption" sx={{ color: "text.secondary" }}>Commented</Typography><Typography variant="h6">{selectedRun.counts.commentCount}</Typography></Box></Grid2>
                <Grid2 size={{ xs: 6, md: 3 }}><Box className="metadata-box" sx={{ minHeight: 84 }}><Typography variant="caption" sx={{ color: "text.secondary" }}>Liked</Typography><Typography variant="h6">{selectedRun.counts.upvoteCount}</Typography></Box></Grid2>
                <Grid2 size={{ xs: 6, md: 3 }}><Box className="metadata-box" sx={{ minHeight: 84 }}><Typography variant="caption" sx={{ color: "text.secondary" }}>Posted</Typography><Typography variant="h6">{selectedRun.counts.postCount}</Typography></Box></Grid2>
              </Grid2>
              {selectedRun.trigger ? <Alert severity="info">Trigger: {triggerLabel(selectedRun.trigger)}</Alert> : null}
              {selectedRun.summary ? <Alert severity={selectedRun.level === "error" ? "error" : selectedRun.level === "warning" ? "warning" : "success"}>{selectedRun.summary}</Alert> : null}
              {selectedRun.events.some((event) => collectLinks(asRecord(event.details)).length > 0) ? (
                <Box className="metadata-box">
                  <Typography variant="subtitle2">Links</Typography>
                  <Stack spacing={0.75} sx={{ mt: 1 }}>
                    {selectedRun.events.flatMap((event) => collectLinks(asRecord(event.details))).map((link) => (
                      <Box key={`${link.label}-${link.url}`} sx={{ border: "1px solid rgba(62,143,214,0.18)", borderRadius: 1, p: 1 }}>
                        <Link href={link.url} target="_blank" rel="noreferrer" underline="hover">{link.label}</Link>
                        <Typography variant="caption" sx={{ color: "text.secondary", display: "block", mt: 0.35, wordBreak: "break-all" }}>{link.url}</Typography>
                      </Box>
                    ))}
                  </Stack>
                </Box>
              ) : null}
              <Accordion disableGutters defaultExpanded={false} sx={{ background: "rgba(10, 15, 28, 0.6)", boxShadow: "none", border: "1px solid rgba(62,143,214,0.18)", borderRadius: "8px !important", "&:before": { display: "none" } }}>
                <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                  <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
                    <Typography variant="subtitle2">Run steps</Typography>
                    <Chip size="small" label={`${selectedRun.events.length} steps`} sx={{ height: 20, fontSize: "0.7rem" }} />
                  </Stack>
                </AccordionSummary>
                <AccordionDetails sx={{ pt: 0 }}>
                  <Stack spacing={1}>
                    {selectedRun.events.map((event, index) => {
                      const details = asRecord(event.details);
                      const summary = actionSummary(str(event.action, ""), details);
                      const reason = actionReason(str(event.action, ""), details);
                      const links = collectLinks(details);
                      const level = str(event.level, "").toLowerCase();
                      return (
                        <Box key={`${selectedRun.id}-${str(event.id, String(index))}`} sx={{ py: 0.75 }}>
                          <Stack direction="row" spacing={1} useFlexGap sx={{ alignItems: "center", flexWrap: "wrap" }}>
                            <Chip size="small" label={level || "info"} color={level === "error" ? "error" : level === "warning" || level === "warn" ? "warning" : "success"} sx={{ height: 18, fontSize: "0.65rem" }} />
                            <Typography variant="body2" sx={{ fontWeight: 500 }}>{index + 1}. {actionLabel(str(event.action, ""), details)}</Typography>
                            <Typography variant="caption" sx={{ color: "text.secondary" }}><span title={absoluteTs(event.timestamp).tip}>{absoluteTs(event.timestamp).label}</span></Typography>
                          </Stack>
                          {summary ? <Typography variant="caption" sx={{ color: "text.secondary", mt: 0.25, display: "block" }}>{summary}</Typography> : null}
                          {reason ? <Typography variant="caption" sx={{ color: "warning.main", mt: 0.25, display: "block" }}>Reason: {reason}</Typography> : null}
                          {links.length ? (
                            <Stack direction="row" spacing={1} useFlexGap sx={{ mt: 0.35, flexWrap: "wrap" }}>
                              {links.map((link) => (
                                <Link key={`${selectedRun.id}-${str(event.id, String(index))}-${link.url}`} href={link.url} target="_blank" rel="noreferrer" underline="hover" variant="caption" sx={{ wordBreak: "break-all" }}>
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
          ) : null}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSelectedRun(null)}>Close</Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
