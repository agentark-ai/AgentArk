import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import MoreVertIcon from "@mui/icons-material/MoreVert";
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
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import {
  formatUiDateOnly,
  formatUiDateTime,
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import {
  isBackgroundSessionVisibleInUi,
  isOneShotReminderTask,
  taskActionDisplay,
  taskKind,
  taskKindLabel,
} from "../../lib/backgroundSessions";
import {
  TASK_CANCEL_CONTROLS_ENABLED,
  TASK_PAUSE_CONTROLS_ENABLED,
  TASK_RETRY_CONTROLS_ENABLED,
} from "../../lib/featureFlags";
import type { BackgroundSessionSummary, Task } from "../../types";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  type JsonRecord,
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
} from "./pageHelpers";

const REFRESH_MS = 8000;
const TASKS_QUERY_LIMIT = 300;
const MAX_VISIBLE_TASK_ROWS = 80;
const RECENT_DONE_LIMIT = 24;
const TASK_INPUT_NEEDED_MARKER = "__INPUT_NEEDED__:";
const CHAT_PENDING_LAUNCH_STORAGE_KEY = "agentark.chat.pendingLaunch";

type TaskFilter = "open" | "scheduled" | "recent" | "all";

type ChatPendingLaunch = {
  createdAt: number;
  launchMode: "message" | "resume_task";
  message?: string;
  conversationId?: string;
  taskId?: string;
  source?: string;
};

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

function looksLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  return trimmed.startsWith("http://") || trimmed.startsWith("https://");
}

function looksLikeUuid(value: string): boolean {
  const trimmed = value.trim();
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    trimmed,
  );
}

function looksLikeIsoTimestamp(value: string): boolean {
  const trimmed = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}T/.test(trimmed)) return false;
  const date = new Date(trimmed);
  return !Number.isNaN(date.getTime());
}

function looksLikeIsoDateOnly(value: string): boolean {
  const trimmed = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) return false;
  const date = new Date(`${trimmed}T00:00:00`);
  return !Number.isNaN(date.getTime());
}

function formatTimestampForHumans(value: string): {
  label: string;
  tooltip: string;
} {
  const meta = formatUiDateTimeMeta(value, { fallback: value || "-" });
  return { label: meta.label, tooltip: meta.tip };
}

function humanTs(raw: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(raw, { fallback: "-" });
}

function boolLabelForKey(
  key: string,
  value: boolean,
): { label: string; color: "success" | "warning" | "default" } {
  const normalized = key.trim().toLowerCase();
  if (normalized.includes("enabled")) {
    return {
      label: value ? "Enabled" : "Disabled",
      color: value ? "success" : "warning",
    };
  }
  if (normalized.includes("active")) {
    return {
      label: value ? "Active" : "Inactive",
      color: value ? "success" : "warning",
    };
  }
  if (normalized.includes("connected")) {
    return {
      label: value ? "Connected" : "Not connected",
      color: value ? "success" : "warning",
    };
  }
  return { label: value ? "Yes" : "No", color: value ? "success" : "default" };
}

function formatCompactValue(value: unknown): { text: string; tooltip?: string } {
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
  if (typeof value === "number") {
    return { text: Number.isFinite(value) ? String(value) : "-" };
  }
  if (typeof value === "boolean") {
    return { text: value ? "true" : "false" };
  }
  if (Array.isArray(value)) {
    const items = value
      .slice(0, 5)
      .map((entry) =>
        typeof entry === "string"
          ? entry
          : typeof entry === "number"
            ? String(entry)
            : typeof entry === "boolean"
              ? entry
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
    const record = asRecord(value);
    const title =
      str(record.title, "") ||
      str(record.name, "") ||
      str(record.label, "") ||
      str(record.description, "");
    const id = str(record.id, "");
    if (title) return { text: title, tooltip: id ? `ID: ${id}` : undefined };
    const scalars = Object.entries(record)
      .filter(
        ([, entry]) =>
          typeof entry === "string" ||
          typeof entry === "number" ||
          typeof entry === "boolean",
      )
      .slice(0, 4)
      .map(([recordKey, entry]) => {
        const text =
          typeof entry === "string" && entry.length > 30
            ? `${entry.slice(0, 30)}...`
            : String(entry);
        return `${recordKey}: ${text}`;
      });
    if (scalars.length > 0) {
      const keys = Object.keys(record);
      const more =
        keys.length > scalars.length
          ? ` (+${keys.length - scalars.length} fields)`
          : "";
      return {
        text: scalars.join(", ") + more,
        tooltip: `Fields: ${keys.join(", ")}`,
      };
    }
    const keys = Object.keys(record);
    return {
      text: keys.length ? `${keys.length} fields` : "-",
      tooltip: keys.length ? `Fields: ${keys.join(", ")}` : undefined,
    };
  }
  return { text: String(value) };
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
          <Typography variant="body2" sx={{ color: "text.secondary" }}>
            {emptyLabel || "No details available."}
          </Typography>
        ) : (
          shown.map(([key, value], index) => {
            const compactValue = formatCompactValue(value);
            const keyLower = key.toLowerCase();
            const renderValue = () => {
              if (typeof value === "string" && looksLikeUrl(value)) {
                const trimmed = value.trim();
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
                typeof value === "string" &&
                (looksLikeIsoTimestamp(value) ||
                  looksLikeIsoDateOnly(value) ||
                  keyLower.endsWith("_at") ||
                  keyLower.endsWith("_date") ||
                  keyLower.includes("timestamp"))
              ) {
                const timestamp =
                  looksLikeIsoDateOnly(value) || keyLower.endsWith("_date")
                    ? {
                        label: formatUiDateOnly(value, { fallback: "-" }),
                        tooltip: formatUiDateOnly(value, {
                          fallback: "-",
                          includeYear: true,
                        }),
                      }
                    : formatTimestampForHumans(value);
                return (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={timestamp.label}
                    title={timestamp.tooltip}
                  />
                );
              }
              if (typeof value === "boolean") {
                const boolLabel = boolLabelForKey(key, value);
                return (
                  <Chip
                    size="small"
                    label={boolLabel.label}
                    color={boolLabel.color}
                    variant={value ? "filled" : "outlined"}
                  />
                );
              }
              if (typeof value === "number" && Number.isFinite(value)) {
                if (keyLower.includes("ms") || keyLower.includes("duration")) {
                  return (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`${Math.round(value)} ms`}
                    />
                  );
                }
                if (
                  keyLower.includes("count") ||
                  keyLower.includes("total") ||
                  keyLower.includes("remaining")
                ) {
                  return (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={String(value)}
                    />
                  );
                }
              }
              if (
                typeof value === "string" &&
                (looksLikeUuid(value) ||
                  keyLower.endsWith("_id") ||
                  keyLower === "id")
              ) {
                const trimmed = value.trim();
                const label =
                  trimmed.length > 22
                    ? `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`
                    : trimmed;
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
                        // ignore clipboard failures
                      }
                    }}
                    sx={{ cursor: "pointer" }}
                  />
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
                  title={compactValue.tooltip || ""}
                >
                  {compactValue.text}
                </Typography>
              );
            };
            return (
              <Box
                key={key}
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
                  {key}
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

type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

function RowOpsMenu({
  actions,
  ariaLabel = "Row actions",
}: {
  actions: RowMenuAction[];
  ariaLabel?: string;
}) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton
        size="small"
        aria-label={ariaLabel}
        onClick={(event) => setAnchorEl(event.currentTarget)}
      >
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu anchorEl={anchorEl} open={open} onClose={closeMenu}>
        {actions.map((action, index) => (
          <MenuItem
            key={`${action.label}-${index}`}
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

type TasksPageProps = {
  autoRefresh: boolean;
};

export default function TasksPage({ autoRefresh }: TasksPageProps) {
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
  const [taskFilter, setTaskFilter] = useState<TaskFilter>("open");
  const [editTaskInputsOpen, setEditTaskInputsOpen] = useState(false);
  const [editTaskInputsJson, setEditTaskInputsJson] = useState("{}");
  const [editTaskInputsError, setEditTaskInputsError] = useState<string | null>(
    null,
  );

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

  function parseTaskResultPayload(raw: unknown): JsonRecord | null {
    if (raw && typeof raw === "object" && !Array.isArray(raw)) {
      return asRecord(raw);
    }
    if (typeof raw !== "string") return null;
    const trimmed = raw.trim();
    if (!trimmed) return null;
    const normalized = trimmed.startsWith(TASK_INPUT_NEEDED_MARKER)
      ? trimmed.slice(TASK_INPUT_NEEDED_MARKER.length).trim()
      : trimmed;
    try {
      const parsed = JSON.parse(normalized);
      return parsed && typeof parsed === "object"
        ? (parsed as JsonRecord)
        : null;
    } catch {
      return null;
    }
  }

  function normalizeStringList(raw: unknown): string[] {
    if (Array.isArray(raw)) {
      return raw
        .map((value) => String(value).trim())
        .filter((value) => !!value);
    }
    if (typeof raw === "string") {
      const trimmed = raw.trim();
      return trimmed ? [trimmed] : [];
    }
    return [];
  }

  function inputNeededResult(task: JsonRecord): JsonRecord | null {
    const payload = parseTaskResultPayload(task.result);
    if (!payload) return null;
    const kind = str(payload.kind, "").toLowerCase();
    if (
      kind === "input_needed" ||
      kind === "input-needed" ||
      kind === "workflow_inputs"
    ) {
      return payload;
    }
    return null;
  }

  function isSensitiveTaskInputKey(raw: string): boolean {
    const lower = raw.trim().toLowerCase();
    if (!lower) return false;
    return [
      "key",
      "token",
      "secret",
      "password",
      "passwd",
      "credential",
      "auth",
      "api_key",
      "client_secret",
      "webhook_secret",
    ].some((token) => lower.includes(token));
  }

  function statusLabel(raw: string, result?: unknown): string {
    const needed = result && inputNeededResult(asRecord({ result }));
    if (needed) return "Input needed";
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

  function statusColor(
    raw: string,
    result?: unknown,
  ): "success" | "warning" | "error" | "default" | "info" {
    const needed = result && inputNeededResult(asRecord({ result }));
    if (needed) return "warning";
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
    queryFn: () => api.rawGet(`/tasks?limit=${TASKS_QUERY_LIMIT}&sort=ops`),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const sessionsQ = useQuery({
    queryKey: ["background-sessions-task-links"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    staleTime: 10_000,
  });

  const opMutation = useMutation({
    mutationFn: ({
      path,
      method,
      payload,
    }: {
      path: string;
      method: "POST" | "DELETE";
      payload?: unknown;
    }) =>
      method === "DELETE"
        ? api.rawDelete(path)
        : api.rawPost(path, payload ?? {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    },
  });
  const updateTaskInputsMutation = useMutation({
    mutationFn: async ({
      id,
      argumentsJson,
    }: {
      id: string;
      argumentsJson: string;
    }) => {
      const trimmed = argumentsJson.trim() || "{}";
      let parsed: unknown;
      try {
        parsed = JSON.parse(trimmed);
      } catch {
        throw new Error("Arguments JSON must be valid JSON.");
      }
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("Arguments JSON must be a JSON object.");
      }
      await api.rawPost(`/tasks/${encodeURIComponent(id)}`, {
        arguments: parsed,
      });
      return parsed as JsonRecord;
    },
    onSuccess: async (parsed, variables) => {
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      setSelectedTask((current) =>
        current && str(current.id, "").trim() === variables.id
          ? { ...current, arguments: parsed }
          : current,
      );
      setEditTaskInputsOpen(false);
      setEditTaskInputsError(null);
    },
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
          arguments: asRecord(step.arguments),
        }))
        .filter((step) => !!step.action);

      if (steps.length === 0) {
        throw new Error(
          "AI planner returned no runnable steps. Try a more specific request.",
        );
      }

      let cronValue: string | null = null;
      if (schedulePreset === "every_15") cronValue = "*/15 * * * *";
      else if (schedulePreset === "hourly") cronValue = "0 * * * *";
      else if (schedulePreset === "daily_9") cronValue = "0 9 * * *";
      else if (schedulePreset === "weekday_9") cronValue = "0 9 * * 1-5";
      else if (schedulePreset === "custom")
        cronValue = customCron.trim() || null;

      const summary = str(plan.summary, "").trim();
      await opMutation.mutateAsync({
        path: "/tasks",
        method: "POST",
        payload: {
          description: summary || intent,
          action: "plan",
          arguments: { steps },
          cron: cronValue,
          approval: requireApproval ? "require" : "auto",
        },
      });
    },
    onSuccess: async () => {
      resetTaskCreateForm();
      setCreateTaskOpen(false);
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    },
  });

  const tasks = pickRecords(tasksQ.data, "tasks");
  const sessionsById = useMemo(() => {
    const map = new Map<string, JsonRecord>();
    for (const session of pickRecords(sessionsQ.data, "sessions")) {
      const id = str(session.id, "").trim();
      if (!id) continue;
      map.set(id, session);
    }
    return map;
  }, [sessionsQ.data]);
  const taskBackgroundSessionId = (task: JsonRecord): string =>
    str(
      asRecord(asRecord(task.arguments)._automation).background_session_id,
      "",
    ).trim();
  const taskBackgroundSessionVisible = (task: JsonRecord): boolean => {
    const id = taskBackgroundSessionId(task);
    if (!id) return false;
    const session = sessionsById.get(id);
    if (session) {
      return isBackgroundSessionVisibleInUi(
        session as unknown as BackgroundSessionSummary,
      );
    }
    return !isOneShotReminderTask(task as unknown as Task);
  };
  const taskBackgroundSessionTitle = (task: JsonRecord): string => {
    const id = taskBackgroundSessionId(task);
    if (!id) return "";
    return str(sessionsById.get(id)?.title, "").trim();
  };
  const isWebChatRequestTask = (task: JsonRecord): boolean => {
    const argumentsObj = asRecord(task.arguments);
    return (
      str(task.action, "").trim() === "chat_request" &&
      str(argumentsObj._origin, "").trim() === "chat" &&
      str(argumentsObj.channel, "").trim() === "web"
    );
  };
  const taskCreatedMs = (task: JsonRecord): number => {
    const value = Date.parse(str(task.created_at, ""));
    return Number.isFinite(value) ? value : 0;
  };
  const taskHasSchedule = (task: JsonRecord): boolean =>
    !!str(task.cron, "").trim() || !!str(task.scheduled_for, "").trim();
  const taskIsTerminal = (task: JsonRecord): boolean => {
    const status = str(task.status, "").toLowerCase();
    return status.includes("completed") || status.includes("cancelled");
  };
  const taskIsOpen = (task: JsonRecord): boolean => {
    if (inputNeededResult(task)) return true;
    const status = str(task.status, "").toLowerCase();
    return ["awaitingapproval", "paused", "inprogress", "pending", "failed"].some((token) =>
      status.includes(token),
    );
  };
  const taskIsManaged = (task: JsonRecord): boolean => {
    if (!isWebChatRequestTask(task)) return true;
    return taskBackgroundSessionVisible(task);
  };
  const taskSortRank = (task: JsonRecord): number => {
    if (inputNeededResult(task)) return 0;
    const status = str(task.status, "").toLowerCase();
    if (status.includes("awaitingapproval")) return 1;
    if (status.includes("failed")) return 2;
    if (status.includes("paused")) return 3;
    if (status.includes("inprogress")) return 4;
    if (status.includes("pending")) return 5;
    if (taskHasSchedule(task) && !taskIsTerminal(task)) return 6;
    if (status.includes("cancelled")) return 8;
    if (status.includes("completed")) return 9;
    return 7;
  };
  const sortTaskRows = (rows: JsonRecord[]): JsonRecord[] =>
    [...rows].sort((left, right) => {
      const rankDelta = taskSortRank(left) - taskSortRank(right);
      if (rankDelta !== 0) return rankDelta;
      return taskCreatedMs(right) - taskCreatedMs(left);
    });
  const launchChatResumeForTask = (task: JsonRecord) => {
    const argumentsObj = asRecord(task.arguments);
    const taskId = str(task.id, "").trim();
    const conversationId = str(argumentsObj.conversation_id, "").trim();
    if (!taskId || !conversationId) {
      window.alert(
        "This chat task is missing its conversation metadata and cannot be resumed in chat.",
      );
      return;
    }
    storeChatPendingLaunch({
      createdAt: Date.now(),
      launchMode: "resume_task",
      conversationId,
      taskId,
    });
    if (typeof window !== "undefined") {
      const nextUrl = `/ui/chat${window.location.search || ""}`;
      const current = `${window.location.pathname}${window.location.search}`;
      if (current !== nextUrl) {
        window.history.pushState(null, "", nextUrl);
      }
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  };
  const managedTasks = sortTaskRows(tasks.filter((task) => taskIsManaged(task)));
  const openTasks = managedTasks.filter((task) => taskIsOpen(task));
  const scheduledTasks = managedTasks.filter(
    (task) => taskHasSchedule(task) && !taskIsTerminal(task),
  );
  const recentDoneTasks = sortTaskRows(
    managedTasks.filter((task) => taskIsTerminal(task)),
  ).slice(0, RECENT_DONE_LIMIT);
  const filteredTasks =
    taskFilter === "open"
      ? openTasks
      : taskFilter === "scheduled"
        ? scheduledTasks
        : taskFilter === "recent"
          ? recentDoneTasks
          : managedTasks;
  const visibleTasks = filteredTasks.slice(0, MAX_VISIBLE_TASK_ROWS);
  const hiddenTaskCount = Math.max(0, filteredTasks.length - visibleTasks.length);
  const internalChatTaskCount = tasks.filter((task) => isWebChatRequestTask(task)).length;
  const counts = {
    open: openTasks.length,
    running: managedTasks.filter((task) =>
      str(task.status, "").toLowerCase().includes("inprogress"),
    ).length,
    waiting: managedTasks.filter((task) => {
      const status = str(task.status, "").toLowerCase();
      return (
        inputNeededResult(task) ||
        status.includes("awaitingapproval") ||
        status.includes("paused")
      );
    }).length,
    scheduled: scheduledTasks.length,
    failed: managedTasks.filter((task) =>
      str(task.status, "").toLowerCase().includes("failed"),
    ).length,
    done: recentDoneTasks.length,
    managed: managedTasks.length,
  };
  const filterOptions: Array<{ key: TaskFilter; label: string; value: number }> = [
    { key: "open", label: "Open", value: counts.open },
    { key: "scheduled", label: "Scheduled", value: counts.scheduled },
    { key: "recent", label: "Recent done", value: counts.done },
    { key: "all", label: "All managed", value: counts.managed },
  ];

  return (
    <WorkspacePageShell
      spacing={1.5}
      sx={{ flex: 1, minHeight: 0, height: "100%" }}
    >
        <WorkspacePageHeader
          eyebrow="Operations"
          title="Tasks"
          description="Runnable automations, scheduled work, approvals, and paused jobs."
          className="tasks-page-header"
          actions={
          <Button
            variant="contained"
            onClick={() => {
              setFormError(null);
              setCreateTaskOpen(true);
            }}
          >
            Create Task
          </Button>
        }
      />
      <Box className="list-shell stat-strip">
        {[
          { label: "Open", value: counts.open, tone: counts.open > 0 ? "warn" : undefined },
          { label: "Running", value: counts.running, tone: counts.running > 0 ? "info" : undefined },
          { label: "Waiting", value: counts.waiting, tone: counts.waiting > 0 ? "warn" : undefined },
          { label: "Scheduled", value: counts.scheduled, tone: counts.scheduled > 0 ? "info" : undefined },
          { label: "Failed", value: counts.failed, tone: counts.failed > 0 ? "warn" : undefined },
          { label: "Recent Done", value: counts.done, tone: counts.done > 0 ? "good" : undefined },
        ].map((s) => (
          <div key={s.label} className="stat-strip-item" data-tone={s.tone}>
            <span className="stat-strip-label">{s.label}</span>
            <span className="stat-strip-value">{s.value}</span>
          </div>
        ))}
      </Box>
      <Box className="list-shell" data-tour-target="tasks-work-queue">
        <Typography
          variant="h6"
          sx={{
            mb: 1,
          }}
        >
          Work Queue
        </Typography>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{ flexWrap: "wrap", alignItems: "center", mb: 1 }}
        >
          {filterOptions.map((option) => (
            <Button
              key={option.key}
              size="small"
              variant={taskFilter === option.key ? "contained" : "outlined"}
              onClick={() => setTaskFilter(option.key)}
              sx={{ textTransform: "none" }}
            >
              {option.label} {option.value}
            </Button>
          ))}
          {internalChatTaskCount > 0 ? (
            <Chip size="small" variant="outlined" label={`${internalChatTaskCount} chat traces hidden`} />
          ) : null}
        </Stack>
        <Box sx={{ width: "100%" }}>
              {visibleTasks.length === 0 ? (
                <Box
                  className="empty-state"
                  sx={{
                    minHeight: 140,
                    display: "grid",
                    placeItems: "center",
                    borderTop: "1px solid",
                    borderColor: "divider",
                  }}
                >
                  <Typography variant="body2" sx={{ color: "text.secondary", fontWeight: 600 }}>
                    No tasks in this queue.
                  </Typography>
                </Box>
              ) : visibleTasks.map((task) => {
                const id = str(task.id, "");
                const cronExpr = str(task.cron, "");
                const scheduledFor = str(task.scheduled_for, "");
                const schedule = cronExpr
                  ? `cron: ${cronExpr}`
                  : scheduledFor
                    ? `at ${formatUiDateTime(scheduledFor, { fallback: scheduledFor })}`
                    : "manual";
                const rawStatus = str(task.status, "-");
                const rawStatusLower = rawStatus.toLowerCase();
                const taskDisplay = taskActionDisplay(task as unknown as Task);
                const backgroundSessionVisible =
                  taskBackgroundSessionVisible(task);
                const backgroundSessionId = backgroundSessionVisible
                  ? taskBackgroundSessionId(task)
                  : "";
                const backgroundSessionTitle = backgroundSessionVisible
                  ? taskBackgroundSessionTitle(task)
                  : "";
                const isChatRequestTask = isWebChatRequestTask(task);
                const rowActions: RowMenuAction[] = [
                  {
                    label: "View",
                    onClick: () => setSelectedTask(asRecord(task)),
                  },
                  {
                    label: "Approve",
                    disabled: !rawStatusLower.includes("awaitingapproval"),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/approve`,
                        method: "POST",
                      }),
                  },
                  {
                    label: "Reject",
                    tone: "warning",
                    disabled: !rawStatusLower.includes("awaitingapproval"),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/reject`,
                        method: "POST",
                      }),
                  },
                ];
                if (TASK_PAUSE_CONTROLS_ENABLED) {
                  rowActions.push({
                    label: "Pause",
                    disabled: !["pending", "awaitingapproval"].some((token) =>
                      rawStatusLower.includes(token),
                    ),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/pause`,
                        method: "POST",
                      }),
                  });
                }
                if (isChatRequestTask) {
                  if (TASK_RETRY_CONTROLS_ENABLED) {
                    if (rawStatusLower.includes("cancelled")) {
                      rowActions.push({
                        label: "Resume in chat",
                        onClick: () => launchChatResumeForTask(task),
                      });
                    } else if (rawStatusLower.includes("failed")) {
                      rowActions.push({
                        label: "Retry in chat",
                        onClick: () => launchChatResumeForTask(task),
                      });
                    }
                  }
                } else if (TASK_RETRY_CONTROLS_ENABLED) {
                  rowActions.push({
                    label: "Resume",
                    disabled: !rawStatusLower.includes("paused"),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/resume`,
                        method: "POST",
                      }),
                  });
                }
                if (TASK_CANCEL_CONTROLS_ENABLED) {
                  rowActions.push({
                    label: "Stop",
                    tone: "warning",
                    disabled: ![
                      "pending",
                      "awaitingapproval",
                      "paused",
                      "inprogress",
                    ].some((token) => rawStatusLower.includes(token)),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/cancel`,
                        method: "POST",
                      }),
                  });
                }
                if (TASK_RETRY_CONTROLS_ENABLED && !isChatRequestTask) {
                  rowActions.push({
                    label: "Retry",
                    disabled: !["failed", "cancelled"].some((token) =>
                      rawStatusLower.includes(token),
                    ),
                    onClick: () =>
                      opMutation.mutate({
                        path: `/tasks/${encodeURIComponent(id)}/retry`,
                        method: "POST",
                      }),
                  });
                }
                rowActions.push({
                  label: "Delete",
                  tone: "error",
                  divider: true,
                  onClick: async () => {
                    const ok = window.confirm(
                      "Delete this task? This cannot be undone.",
                    );
                    if (!ok) return;
                    opMutation.mutate({
                      path: `/tasks/${encodeURIComponent(id)}`,
                      method: "DELETE",
                    });
                  },
                });
                const dotColor = rawStatusLower.includes("completed")
                  ? "var(--ui-rgba-74-210-157-850)"
                  : rawStatusLower.includes("failed")
                    ? "var(--ui-rgba-255-100-100-850)"
                    : rawStatusLower.includes("inprogress")
                      ? "var(--ui-rgba-57-208-255-850)"
                      : rawStatusLower.includes("awaitingapproval")
                        ? "var(--ui-rgba-255-191-130-850)"
                        : rawStatusLower.includes("paused") || rawStatusLower.includes("cancelled")
                          ? "var(--ui-rgba-255-191-130-850)"
                          : rawStatusLower.includes("pending")
                            ? "var(--ui-rgba-180-200-220-500)"
                            : "var(--ui-rgba-180-200-220-500)";

                return (
                  <ButtonBase
                    key={id}
                    onClick={() => setSelectedTask(asRecord(task))}
                    sx={{
                      width: "100%",
                      textAlign: "left",
                      justifyContent: "flex-start",
                      px: 0,
                      py: 1.15,
                      borderBottom: "1px solid",
                      borderColor: "divider",
                      transition: "background 0.15s ease",
                      "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
                    }}
                  >
                    <Box sx={{ flex: 1, minWidth: 0 }}>
                      {/* Line 1: dot + title ... timestamp + ops menu */}
                      <Stack
                        direction="row"
                        sx={{ alignItems: "center", gap: 1 }}
                      >
                        <Box
                          component="span"
                          sx={{
                            width: 7,
                            height: 7,
                            borderRadius: "50%",
                            flexShrink: 0,
                            bgcolor: dotColor,
                          }}
                        />
                        <Typography
                          variant="body2"
                          noWrap
                          sx={{ fontWeight: 600, flex: 1, minWidth: 0 }}
                          title={str(task.description)}
                        >
                          {str(task.description)}
                        </Typography>
                        <Typography
                          variant="caption"
                          noWrap
                          sx={{ color: "text.secondary", flexShrink: 0 }}
                          title={humanTs(str(task.created_at)).tip}
                        >
                          {humanTs(str(task.created_at)).label}
                        </Typography>
                        <Box
                          onClick={(e) => e.stopPropagation()}
                          sx={{ flexShrink: 0 }}
                        >
                          <RowOpsMenu
                            actions={rowActions}
                            ariaLabel="Task options"
                          />
                        </Box>
                      </Stack>
                      {/* Line 2: background session info */}
                      {backgroundSessionVisible ? (
                        <Typography
                          variant="caption"
                          noWrap
                          sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                          title={backgroundSessionTitle || backgroundSessionId}
                        >
                          {backgroundSessionTitle
                            ? `Background session: ${backgroundSessionTitle}`
                            : "Background session linked"}
                        </Typography>
                      ) : null}
                      {/* Line 3: metadata - status, type, schedule */}
                      <Typography
                        variant="caption"
                        noWrap
                        sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                      >
                        {statusLabel(rawStatus, task.result)}
                        {" \u00B7 "}
                        {taskDisplay}
                        {" \u00B7 "}
                        {schedule}
                      </Typography>
                    </Box>
                  </ButtonBase>
                );
              })}
              {hiddenTaskCount > 0 ? (
                <Typography
                  variant="caption"
                  sx={{ color: "text.secondary", display: "block", pt: 1 }}
                >
                  Showing {visibleTasks.length} of {filteredTasks.length} tasks in this view.
                </Typography>
              ) : null}
        </Box>
      </Box>
      <Dialog
        open={selectedTask != null}
        onClose={() => setSelectedTask(null)}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-bg-elevated)",
              boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)",
            },
          },
        }}
      >
        <DialogTitle
          sx={{
            pb: 0.5,
            display: "flex",
            alignItems: "center",
            gap: 1.5,
            borderBottom: "1px solid",
            borderColor: "divider",
          }}
        >
          <Typography
            variant="h6"
            noWrap
            sx={{ fontWeight: 700, flex: 1, minWidth: 0 }}
            title={str(selectedTask?.description, "Task")}
          >
            {str(selectedTask?.description, "Task")}
          </Typography>
          <Chip
            size="small"
            label={statusLabel(
              str(selectedTask?.status, ""),
              selectedTask?.result,
            )}
            color={statusColor(
              str(selectedTask?.status, ""),
              selectedTask?.result,
            )}
            sx={{ flexShrink: 0 }}
          />
        </DialogTitle>
        <DialogContent sx={{ pt: 2 }}>
          <Stack spacing={1.25}>
            <Box
              className="micro-surface"
              sx={{
                borderRadius: "8px",
                border: "1px solid var(--surface-border)",
                background: "var(--micro-surface-bg)",
                p: 1.4,
                boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-040)",
              }}
            >
              <Stack spacing={1.15}>
                <Stack
                  direction="row"
                  spacing={1}
                  useFlexGap
                  sx={{
                    flexWrap: "wrap",
                    alignItems: "center",
                  }}
                >
                  <Chip
                    size="small"
                    label={statusLabel(
                      str(selectedTask?.status, ""),
                      selectedTask?.result,
                    )}
                    color={statusColor(
                      str(selectedTask?.status, ""),
                      selectedTask?.result,
                    )}
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      str(selectedTask?.cron, "") ||
                      str(selectedTask?.scheduled_for, "")
                        ? "Scheduled"
                        : "Manual"
                    }
                    sx={{
                      borderColor: "var(--ui-rgba-255-255-255-140)",
                      background: "var(--ui-rgba-255-255-255-030)",
                    }}
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      taskKind(selectedTask as Task | null | undefined) ===
                      "reminder"
                        ? `Type: ${taskKindLabel(selectedTask as Task | null | undefined)}`
                        : `Action: ${taskActionDisplay(selectedTask as Task | null | undefined)}`
                    }
                    sx={{
                      borderColor: "var(--ui-rgba-255-255-255-140)",
                      background: "var(--ui-rgba-255-255-255-030)",
                    }}
                  />
                  {selectedTask &&
                  taskBackgroundSessionVisible(selectedTask) ? (
                    <Chip
                      size="small"
                      variant="outlined"
                      sx={{
                        borderColor: "var(--ui-rgba-255-255-255-140)",
                        background: "var(--ui-rgba-255-255-255-030)",
                      }}
                      label={
                        taskBackgroundSessionTitle(selectedTask)
                          ? `Session: ${taskBackgroundSessionTitle(selectedTask)}`
                          : "Background session linked"
                      }
                    />
                  ) : null}
                </Stack>

                <Grid2 container spacing={1}>
                  <Grid2 size={{ xs: 12, sm: 6 }}>
                    <Box
                      sx={{
                        height: "100%",
                        borderRadius: "8px",
                        border: "1px solid var(--ui-rgba-255-255-255-080)",
                        background: "var(--ui-rgba-255-255-255-030)",
                        p: 1.1,
                      }}
                    >
                      <Typography
                        variant="caption"
                        sx={{ color: "var(--ui-rgba-188-198-212-680)" }}
                      >
                        Created
                      </Typography>
                      <Typography variant="body2" sx={{ mt: 0.35 }}>
                        <span
                          title={
                            humanTs(str(selectedTask?.created_at, "-")).tip
                          }
                        >
                          {humanTs(str(selectedTask?.created_at, "-")).label}
                        </span>
                      </Typography>
                    </Box>
                  </Grid2>
                  <Grid2 size={{ xs: 12, sm: 6 }}>
                    <Box
                      sx={{
                        height: "100%",
                        borderRadius: "8px",
                        border: "1px solid var(--ui-rgba-255-255-255-080)",
                        background: "var(--ui-rgba-255-255-255-030)",
                        p: 1.1,
                      }}
                    >
                      <Typography
                        variant="caption"
                        sx={{ color: "var(--ui-rgba-188-198-212-680)" }}
                      >
                        Execution
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{ mt: 0.35, color: "var(--ui-rgba-231-236-243-760)" }}
                      >
                        {str(selectedTask?.cron, "")
                          ? "Runs on a schedule and stays queued between executions."
                          : str(selectedTask?.scheduled_for, "")
                            ? "Runs once at the scheduled time."
                            : "Runs once when triggered or approved."}
                      </Typography>
                    </Box>
                  </Grid2>
                </Grid2>

                {str(selectedTask?.cron, "") ||
                str(selectedTask?.scheduled_for, "") ? (
                  <Box
                    sx={{
                      borderRadius: "8px",
                      border: "1px solid var(--ui-rgba-255-255-255-080)",
                      background: "var(--ui-rgba-255-255-255-030)",
                      p: 1.1,
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{ color: "var(--ui-rgba-188-198-212-680)" }}
                    >
                      Schedule
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{ mt: 0.45, whiteSpace: "pre-wrap" }}
                    >
                      {str(selectedTask?.cron, "")
                        ? str(selectedTask?.cron)
                        : formatUiDateTime(
                            str(selectedTask?.scheduled_for, ""),
                            {
                              fallback: str(selectedTask?.scheduled_for, "-"),
                            },
                          )}
                    </Typography>
                  </Box>
                ) : null}
              </Stack>
            </Box>

            {selectedTask && inputNeededResult(selectedTask) ? (
              (() => {
                const payload = inputNeededResult(selectedTask);
                const missing = normalizeStringList(payload?.missing);
                const required = normalizeStringList(payload?.required);
                const provided = normalizeStringList(payload?.provided);
                const summary = str(
                  payload?.summary,
                  "This task is paused until the missing inputs are provided.",
                );
                const canEditTaskInputs = missing.some(
                  (item) => !isSensitiveTaskInputKey(item),
                );
                const fixHint = str(
                  payload?.resolution_hint,
                  str(
                    payload?.fix_hint,
                    "Update the task inputs or required secrets, then resume it.",
                  ),
                );

                return (
                  <Stack spacing={1}>
                    <Alert severity="warning">
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Input needed
                      </Typography>
                      <Typography variant="body2">{summary}</Typography>
                    </Alert>
                    <Box className="metadata-box">
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Missing fields
                      </Typography>
                      {missing.length === 0 ? (
                        <Typography variant="body2">
                          Required inputs were not specified.
                        </Typography>
                      ) : (
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                            mt: 0.75,
                          }}
                        >
                          {missing.map((item) => (
                            <Chip
                              key={item}
                              size="small"
                              label={item}
                              color="warning"
                              variant="outlined"
                            />
                          ))}
                        </Stack>
                      )}
                    </Box>
                    {required.length > 0 ? (
                      <Box className="metadata-box">
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          Required inputs
                        </Typography>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                            mt: 0.75,
                          }}
                        >
                          {required.map((item) => (
                            <Chip
                              key={item}
                              size="small"
                              label={item}
                              variant="outlined"
                            />
                          ))}
                        </Stack>
                      </Box>
                    ) : null}
                    {provided.length > 0 ? (
                      <Box className="metadata-box">
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          Already provided
                        </Typography>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                            mt: 0.75,
                          }}
                        >
                          {provided.map((item) => (
                            <Chip
                              key={item}
                              size="small"
                              label={item}
                              variant="outlined"
                            />
                          ))}
                        </Stack>
                      </Box>
                    ) : null}
                    <Box className="metadata-box">
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Fix guidance
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{ whiteSpace: "pre-wrap" }}
                      >
                        {fixHint}
                      </Typography>
                    </Box>
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{
                        alignItems: { xs: "stretch", sm: "center" },
                      }}
                    >
                      {canEditTaskInputs ? (
                        <Button
                          variant="outlined"
                          size="small"
                          onClick={() => {
                            setEditTaskInputsError(null);
                            setEditTaskInputsJson(
                              JSON.stringify(
                                asRecord(selectedTask?.arguments),
                                null,
                                2,
                              ),
                            );
                            setEditTaskInputsOpen(true);
                          }}
                        >
                          Edit task inputs
                        </Button>
                      ) : null}
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Save the missing values, then resume the task from the
                        task list.
                      </Typography>
                    </Stack>
                  </Stack>
                );
              })()
            ) : str(selectedTask?.result, "") ? (
              <Box
                className="metadata-box"
                sx={{
                  borderRadius: "8px",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "var(--ui-rgba-255-255-255-025)",
                  p: 1.25,
                }}
              >
                <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                  Last Result
                </Typography>
                <Typography
                  variant="body2"
                  sx={{ mt: 0.8, whiteSpace: "pre-wrap" }}
                >
                  {str(selectedTask?.result)}
                </Typography>
              </Box>
            ) : (
              <Box
                className="metadata-box"
                sx={{
                  borderRadius: "8px",
                  border: "1px dashed var(--ui-rgba-255-255-255-140)",
                  background: "var(--ui-rgba-255-255-255-018)",
                  p: 1.2,
                }}
              >
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  No result yet.
                </Typography>
              </Box>
            )}

            <Grid2 container spacing={1.25}>
              <Grid2 size={{ xs: 12, lg: 6 }}>
                <KeyValuePanel
                  title="Arguments"
                  data={asRecord(selectedTask?.arguments)}
                  emptyLabel="No arguments."
                  maxRows={18}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, lg: 6 }}>
                <KeyValuePanel
                  title="System fields"
                  data={asRecord(selectedTask)}
                  emptyLabel="No extra fields."
                  maxRows={10}
                />
              </Grid2>
            </Grid2>
          </Stack>
        </DialogContent>
      </Dialog>
      <Dialog
        open={editTaskInputsOpen}
        onClose={() => {
          if (!updateTaskInputsMutation.isPending) {
            setEditTaskInputsOpen(false);
            setEditTaskInputsError(null);
          }
        }}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Edit Task Inputs</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              Update the arguments JSON with the missing fields, save it, then
              resume the task.
            </Typography>
            <TextField
              fullWidth
              multiline
              minRows={10}
              label="Arguments JSON"
              value={editTaskInputsJson}
              onChange={(event) => setEditTaskInputsJson(event.target.value)}
            />
            {editTaskInputsError ? (
              <Alert severity="error">{editTaskInputsError}</Alert>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setEditTaskInputsOpen(false);
              setEditTaskInputsError(null);
            }}
            disabled={updateTaskInputsMutation.isPending}
          >
            Close
          </Button>
          <Button
            variant="contained"
            disabled={updateTaskInputsMutation.isPending || !selectedTask}
            onClick={async () => {
              const taskId = str(selectedTask?.id, "").trim();
              if (!taskId) {
                setEditTaskInputsError("Task id is missing.");
                return;
              }
              setEditTaskInputsError(null);
              try {
                await updateTaskInputsMutation.mutateAsync({
                  id: taskId,
                  argumentsJson: editTaskInputsJson,
                });
              } catch (error) {
                setEditTaskInputsError(errMessage(error));
              }
            }}
          >
            {updateTaskInputsMutation.isPending ? "Saving..." : "Save inputs"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={createTaskOpen}
        onClose={closeCreateTaskDialog}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Create Task</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 0.5 }}>
            <Box className="list-shell">
              <Typography
                variant="h6"
                sx={{
                  mb: 1,
                }}
              >
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
                    control={
                      <Switch
                        checked={requireApproval}
                        onChange={(e) => setRequireApproval(e.target.checked)}
                      />
                    }
                    label="Require approval before execution"
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <Button
                    variant="contained"
                    disabled={
                      aiCreateMutation.isPending ||
                      opMutation.isPending ||
                      !quickIntent.trim()
                    }
                    onClick={async () => {
                      setFormError(null);
                      try {
                        await aiCreateMutation.mutateAsync();
                      } catch (e) {
                        const msg = errMessage(e);
                        if (msg.toLowerCase().includes("llm planning failed")) {
                          setFormError(
                            "AI planner needs an active LLM model. Configure one in Settings > Models, or use Manual mode below.",
                          );
                        } else {
                          setFormError(msg);
                        }
                      }
                    }}
                  >
                    {aiCreateMutation.isPending
                      ? "Creating..."
                      : "Create with AI"}
                  </Button>
                </Grid2>
              </Grid2>
              {formError ? (
                <Alert severity="error" sx={{ mt: 1 }}>
                  {formError}
                </Alert>
              ) : null}
            </Box>

            <Accordion
              expanded={manualOpen}
              onChange={() => setManualOpen((p) => !p)}
              className="accordion-shell"
            >
              <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>
                  Manual Mode (Optional)
                </Typography>
              </AccordionSummary>
              <AccordionDetails>
                <Grid2 container spacing={1}>
                  <Grid2 size={{ xs: 12, md: 4 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Description"
                      value={description}
                      onChange={(e) => setDescription(e.target.value)}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 2 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Action"
                      value={action}
                      onChange={(e) => setAction(e.target.value)}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 3 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Cron"
                      value={cron}
                      onChange={(e) => setCron(e.target.value)}
                      placeholder="*/10 * * * *"
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 3 }}>
                    <TextField
                      fullWidth
                      size="small"
                      select
                      label="Approval"
                      value={approval}
                      onChange={(e) => setApproval(e.target.value)}
                    >
                      <MenuItem value="auto">auto</MenuItem>
                      <MenuItem value="require">require</MenuItem>
                    </TextField>
                  </Grid2>
                  <Grid2 size={{ xs: 12 }}>
                    <TextField
                      fullWidth
                      multiline
                      minRows={2}
                      label="Arguments JSON"
                      value={argumentsJson}
                      onChange={(e) => setArgumentsJson(e.target.value)}
                    />
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
                            payload: {
                              description: description.trim(),
                              action: action.trim(),
                              arguments: parsed,
                              cron: cron.trim() || null,
                              approval,
                            },
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
    </WorkspacePageShell>
  );
}
