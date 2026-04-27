import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import {
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import type { BackgroundSessionSummary } from "../../types";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  type JsonRecord,
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
} from "./pageHelpers";

const REFRESH_MS = 8000;

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

function asRecords(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.map(asRecord);
}

function formatDurationFromSeconds(value: unknown): string {
  const total = num(value, -1);
  if (total < 0) return "-";
  const seconds = Math.floor(total);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) {
    return remainingSeconds > 0
      ? `${minutes}m ${remainingSeconds}s`
      : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours < 24) {
    return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return remainingHours > 0 ? `${days}d ${remainingHours}h` : `${days}d`;
}

function watcherDotColor(raw: unknown): string {
  const value = str(raw, "").toLowerCase();
  if (value.includes("active")) return "var(--ui-rgba-57-208-255-850)";
  if (value.includes("completed") || value.includes("triggered")) {
    return "var(--ui-rgba-74-210-157-850)";
  }
  if (value.includes("failed")) return "var(--ui-rgba-255-100-100-850)";
  if (value.includes("paused") || value.includes("pending")) {
    return "var(--ui-rgba-255-191-130-850)";
  }
  return "var(--ui-rgba-180-200-220-500)";
}

function watcherStatusLabel(raw: unknown): string {
  const value = str(raw, "").trim();
  if (!value) return "-";
  return value.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
}

function watcherStatusColor(
  raw: unknown,
): "success" | "warning" | "error" | "default" | "info" {
  const value = str(raw, "").toLowerCase();
  if (value.includes("active")) return "success";
  if (value.includes("paused")) return "warning";
  if (value.includes("triggered")) return "info";
  if (
    value.includes("failed") ||
    value.includes("timed") ||
    value.includes("cancelled")
  ) {
    return "error";
  }
  return "default";
}

function watcherConditionSummary(raw: unknown): string {
  const condition = asRecord(raw);
  const entries = Object.entries(condition);
  if (entries.length === 0) return "-";
  const [kind, payload] = entries[0];
  const body = asRecord(payload);
  if (kind === "not_empty") return "Trigger when results are not empty";
  if (kind === "contains") {
    return `Trigger when result contains "${str(body.keyword, "")}"`;
  }
  if (kind === "matches") {
    return `Trigger when result matches ${str(body.pattern, "")}`;
  }
  if (kind === "custom") return str(body.description, "Custom condition");
  return kind.replace(/_/g, " ");
}

function watcherPollOutcomeLabel(raw: unknown): string {
  const value = str(raw, "").trim();
  if (!value) return "Unknown";
  return value.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
}

function watcherPollOutcomeColor(
  raw: unknown,
): "success" | "warning" | "error" | "default" | "info" {
  const value = str(raw, "").trim().toLowerCase();
  if (value === "matched") return "success";
  if (value === "error") return "error";
  if (value === "no_match") return "default";
  return "info";
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

function watcherPayloadText(raw: unknown): string {
  if (typeof raw === "string") return formatTraceData(raw);
  if (raw == null) return "";
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return String(raw);
  }
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

type WatchersPageProps = {
  autoRefresh: boolean;
};

export default function WatchersPage({ autoRefresh }: WatchersPageProps) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedWatcherId, setSelectedWatcherId] = useState<string | null>(
    null,
  );
  const watchersQ = useQuery({
    queryKey: ["watchers-page-watchers"],
    queryFn: () => api.rawGet("/watchers"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const sessionsQ = useQuery({
    queryKey: ["background-sessions-watcher-links"],
    queryFn: api.getBackgroundSessions,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    staleTime: 10_000,
  });

  const pauseMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/watchers/${encodeURIComponent(id)}/pause`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const resumeMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/watchers/${encodeURIComponent(id)}/resume`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const cancelMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/watchers/${encodeURIComponent(id)}/cancel`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/watchers/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const runNowMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/watchers/${encodeURIComponent(id)}/run-now`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const extendMutation = useMutation({
    mutationFn: ({ id, body }: { id: string; body: JsonRecord }) =>
      api.rawPost(`/watchers/${encodeURIComponent(id)}/extend`, body),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["watchers-page-watchers"],
      });
    },
  });
  const watchers = pickRecords(watchersQ.data, "watchers");
  const sessionsById = useMemo(() => {
    const map = new Map<string, string>();
    for (const session of pickRecords(sessionsQ.data, "sessions")) {
      const id = str(session.id, "").trim();
      if (!id) continue;
      map.set(id, str(session.title, "").trim());
    }
    return map;
  }, [sessionsQ.data]);
  const watcherBackgroundSessionId = (
    watcher: JsonRecord | null | undefined,
  ): string =>
    str(
      asRecord(asRecord(watcher?.poll_arguments)._automation)
        .background_session_id,
      "",
    ).trim();
  const watcherBackgroundSessionTitle = (
    watcher: JsonRecord | null | undefined,
  ): string => {
    const id = watcherBackgroundSessionId(watcher);
    return id ? sessionsById.get(id) || "" : "";
  };
  const selectedWatcher = useMemo(
    () =>
      watchers.find((watcher) => str(watcher.id, "") === selectedWatcherId) ??
      null,
    [selectedWatcherId, watchers],
  );
  const activeCount = watchers.filter((w) =>
    str(w.status, "").toLowerCase().includes("active"),
  ).length;
  const pausedCount = watchers.filter((w) =>
    str(w.status, "").toLowerCase().includes("paused"),
  ).length;
  const triggeredCount = watchers.filter((w) =>
    str(w.status, "").toLowerCase().includes("triggered"),
  ).length;
  const failedCount = watchers.filter((w) => {
    const status = str(w.status, "").toLowerCase();
    return (
      status.includes("failed") ||
      status.includes("timed") ||
      status.includes("cancelled")
    );
  }).length;

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Operations"
        title="Watchers"
        description="Monitor conditions over time, then notify a channel or take the next action when something changes."
      />
      <Box className="list-shell stat-strip">
        {[
          { label: "Active", value: activeCount },
          { label: "Paused", value: pausedCount },
          { label: "Triggered", value: triggeredCount },
          { label: "Stopped / Failed", value: failedCount },
        ].map((s) => (
          <div key={s.label} className="stat-strip-item">
            <span className="stat-strip-label">{s.label}</span>
            <span className="stat-strip-value">{s.value}</span>
          </div>
        ))}
      </Box>
      {watchers.length === 0 ? (
        <Box className="list-shell" sx={{ py: 8, textAlign: "center" }}>
          <Typography
            variant="h6"
            sx={{
              color: "text.secondary",
            }}
          >
            No watchers
          </Typography>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              mt: 0.5,
            }}
          >
            Ask AgentArk to watch something until a condition is met, then
            notify a channel or take action.
          </Typography>
        </Box>
      ) : (
        <Box className="list-shell" sx={{ minHeight: 0 }}>
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              mb: 1,
            }}
          >
            <Typography variant="h6">Watchers</Typography>
          </Stack>
          <Box>
            {watchers.map((w, idx) => {
                  const id = str(w.id, String(idx));
                  const rawStatus = str(w.status, "");
                  const statusLower = rawStatus.toLowerCase();
                  const backgroundSessionId = watcherBackgroundSessionId(w);
                  const backgroundSessionTitle = backgroundSessionId
                    ? sessionsById.get(backgroundSessionId) || ""
                    : "";
                  const isActive = statusLower.includes("active");
                  const isPaused = statusLower.includes("paused");
                  const isHistoryOnly = toBool(w.history_only);
                  const lastPollAt = str(w.last_poll_at, "").trim();
                  const createdAt = str(w.created_at, "").trim();
                  const lastPollLabel = lastPollAt
                    ? formatTimestampForHumans(lastPollAt).label
                    : "Never";
                  const createdLabel = createdAt
                    ? formatTimestampForHumans(createdAt).label
                    : "-";
                  /* trimmed unused vars for unified row */
                  const lastOutcome = str(w.last_poll_outcome, "").trim();
                  const intervalLabel = isHistoryOnly
                    ? "-"
                    : formatDurationFromSeconds(num(w.interval_secs, 0));
                  /* trimmed unused vars for unified row */
                  return (
                    <ButtonBase
                      key={id}
                      onClick={() => {
                        setError(null);
                        setSelectedWatcherId(id);
                      }}
                      sx={{
                        width: "100%",
                        textAlign: "left",
                        justifyContent: "flex-start",
                        px: 0,
                        py: 1.15,
                        borderBottom: "1px solid",
                        borderColor: "divider",
                        transition: "background 0.15s ease",
                        "&:hover": {
                          background: "var(--ui-rgba-57-208-255-040)",
                        },
                        display: "block",
                      }}
                    >
                      {/* Line 1: dot + description ... timestamp + ops */}
                      <Stack
                        direction="row"
                        sx={{
                          justifyContent: "space-between",
                          alignItems: "center",
                        }}
                      >
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            alignItems: "center",
                            minWidth: 0,
                            flex: 1,
                          }}
                        >
                          <Box
                            sx={{
                              width: 7,
                              height: 7,
                              borderRadius: "50%",
                              flexShrink: 0,
                              background: watcherDotColor(rawStatus),
                            }}
                          />
                          <Typography
                            variant="body2"
                            noWrap
                            sx={{ fontWeight: 600 }}
                            title={str(w.description, "")}
                          >
                            {str(w.description, "-")}
                          </Typography>
                        </Stack>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            alignItems: "center",
                            flexShrink: 0,
                            ml: 1,
                          }}
                        >
                          <Typography
                            variant="caption"
                            sx={{ color: "text.secondary" }}
                          >
                            {createdLabel}
                          </Typography>
                          <RowOpsMenu
                            ariaLabel="Watcher actions"
                            actions={[
                              {
                                label: "Inspect",
                                onClick: () => {
                                  setError(null);
                                  setSelectedWatcherId(id);
                                },
                              },
                              {
                                label: "Run now",
                                disabled:
                                  isHistoryOnly ||
                                  !isActive ||
                                  runNowMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await runNowMutation.mutateAsync(id);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Pause",
                                disabled:
                                  isHistoryOnly ||
                                  !isActive ||
                                  pauseMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await pauseMutation.mutateAsync(id);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Resume",
                                disabled:
                                  isHistoryOnly ||
                                  !isPaused ||
                                  resumeMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await resumeMutation.mutateAsync(id);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Stop",
                                tone: "warning",
                                disabled:
                                  isHistoryOnly ||
                                  (!isActive && !isPaused) ||
                                  cancelMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await cancelMutation.mutateAsync(id);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Extend +24h",
                                disabled:
                                  isHistoryOnly ||
                                  (!isActive && !isPaused) ||
                                  extendMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await extendMutation.mutateAsync({
                                      id,
                                      body: { extra_hours: 24 },
                                    });
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Until stopped",
                                disabled:
                                  isHistoryOnly ||
                                  (!isActive && !isPaused) ||
                                  extendMutation.isPending,
                                onClick: async () => {
                                  setError(null);
                                  try {
                                    await extendMutation.mutateAsync({
                                      id,
                                      body: { until_stopped: true },
                                    });
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                              {
                                label: "Delete",
                                tone: "error",
                                divider: true,
                                disabled: deleteMutation.isPending,
                                onClick: async () => {
                                  const ok = window.confirm(
                                    "Delete this watcher? This cannot be undone.",
                                  );
                                  if (!ok) return;
                                  setError(null);
                                  try {
                                    await deleteMutation.mutateAsync(id);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                },
                              },
                            ]}
                          />
                        </Stack>
                      </Stack>
                      {/* Line 2: poll action, interval, notify channel */}
                      <Typography
                        variant="caption"
                        noWrap
                        sx={{
                          color: "text.secondary",
                          pl: "15px",
                          display: "block",
                        }}
                      >
                        {str(w.poll_action, "-")} — {intervalLabel}
                        {str(w.notify_channel, "").trim()
                          ? ` — notify ${str(w.notify_channel, "-")}`
                          : ""}
                        {backgroundSessionTitle
                          ? ` — session: ${backgroundSessionTitle}`
                          : backgroundSessionId
                            ? " — background session linked"
                            : ""}
                      </Typography>
                      {/* Line 3: status, polls, last poll */}
                      <Typography
                        variant="caption"
                        noWrap
                        sx={{
                          color: "text.secondary",
                          pl: "15px",
                          display: "block",
                        }}
                      >
                        {watcherStatusLabel(rawStatus)}
                        {isHistoryOnly ? " (history)" : ""}
                        {` — ${num(w.poll_count, 0)} polls`}
                        {lastOutcome
                          ? ` — ${watcherPollOutcomeLabel(lastOutcome)}`
                          : ""}
                        {` — last: ${lastPollLabel}`}
                      </Typography>
                    </ButtonBase>
                  );
                })}
          </Box>
        </Box>
      )}
      <Dialog
        open={selectedWatcher != null}
        onClose={() => setSelectedWatcherId(null)}
        maxWidth="sm"
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
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            borderBottom: "1px solid",
            borderColor: "divider",
            pb: 1.5,
          }}
        >
          <Typography
            variant="h6"
            noWrap
            sx={{ fontWeight: 600, lineHeight: 1.4, flex: 1, minWidth: 0 }}
            title={str(selectedWatcher?.description, "Watcher")}
          >
            {str(selectedWatcher?.description, "Watcher")}
          </Typography>
          <Stack
            direction="row"
            spacing={0.75}
            sx={{
              alignItems: "center",
              flexShrink: 0,
              ml: 1,
            }}
          >
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
                label={watcherPollOutcomeLabel(
                  selectedWatcher?.last_poll_outcome,
                )}
                color={watcherPollOutcomeColor(
                  selectedWatcher?.last_poll_outcome,
                )}
              />
            ) : null}
            {watcherBackgroundSessionId(selectedWatcher) ? (
              <Chip
                size="small"
                variant="outlined"
                label={
                  watcherBackgroundSessionTitle(selectedWatcher)
                    ? `Session: ${watcherBackgroundSessionTitle(selectedWatcher)}`
                    : "Background session linked"
                }
              />
            ) : null}
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                ml: "auto !important",
              }}
            >
              {str(selectedWatcher?.id, "-").slice(0, 12)}
            </Typography>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {/* Config summary */}
            <Stack spacing={0.75}>
              {[
                {
                  label: "Action",
                  value: str(selectedWatcher?.poll_action, "-"),
                },
                {
                  label: "Interval",
                  value: toBool(selectedWatcher?.history_only)
                    ? "-"
                    : formatDurationFromSeconds(
                        num(selectedWatcher?.interval_secs, 0),
                      ),
                },
                {
                  label: "Timeout",
                  value: toBool(selectedWatcher?.history_only)
                    ? "-"
                    : formatDurationFromSeconds(
                        num(selectedWatcher?.timeout_secs, 0),
                      ),
                },
                {
                  label: "Notify",
                  value: str(selectedWatcher?.notify_channel, "-"),
                },
                {
                  label: "Polls",
                  value: String(num(selectedWatcher?.poll_count, 0)),
                },
                {
                  label: "Created",
                  value: humanTs(str(selectedWatcher?.created_at, "-")).label,
                  tip: humanTs(str(selectedWatcher?.created_at, "-")).tip,
                },
                ...(str(selectedWatcher?.last_poll_at, "").trim()
                  ? [
                      {
                        label: "Last poll",
                        value: humanTs(str(selectedWatcher?.last_poll_at, ""))
                          .label,
                        tip: humanTs(str(selectedWatcher?.last_poll_at, ""))
                          .tip,
                      },
                    ]
                  : []),
              ].map((row) => (
                <Stack
                  key={row.label}
                  direction="row"
                  spacing={1.5}
                  sx={{
                    alignItems: "baseline",
                  }}
                >
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      minWidth: 70,
                      flexShrink: 0,
                    }}
                  >
                    {row.label}
                  </Typography>
                  <Typography
                    variant="body2"
                    title={(row as { tip?: string }).tip || ""}
                  >
                    {row.value}
                  </Typography>
                </Stack>
              ))}
            </Stack>

            {/* Condition */}
            {watcherConditionSummary(selectedWatcher?.condition) ? (
              <Box>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Condition
                </Typography>
                <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                  {watcherConditionSummary(selectedWatcher?.condition)}
                </Typography>
              </Box>
            ) : null}

            {/* On trigger */}
            {str(selectedWatcher?.on_trigger, "").trim() ? (
              <Box>
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  On trigger
                </Typography>
                <Typography variant="body2" sx={{ mt: 0.25, lineHeight: 1.5 }}>
                  {str(selectedWatcher?.on_trigger, "-")}
                </Typography>
              </Box>
            ) : null}

            {/* Error (only if present) */}
            {str(selectedWatcher?.last_error, "").trim() ||
            str(selectedWatcher?.status_error, "").trim() ? (
              <Alert severity="error" variant="outlined" sx={{ py: 0.5 }}>
                <Typography
                  variant="body2"
                  sx={{
                    fontFamily: "monospace",
                    fontSize: "0.8rem",
                    wordBreak: "break-word",
                  }}
                >
                  {str(selectedWatcher?.last_error, "").trim() ||
                    str(selectedWatcher?.status_error, "").trim()}
                </Typography>
              </Alert>
            ) : null}

            {/* Latest poll payload (only if present) */}
            {(() => {
              const payloadText = watcherPayloadText(
                selectedWatcher?.last_result,
              ).trim();
              return payloadText ? (
                <Box>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Latest poll result
                  </Typography>
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
                      fontFamily:
                        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                      fontSize: "0.78rem",
                      background: "var(--ui-rgba-0-0-0-300)",
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
              const triggerText = watcherPayloadText(
                selectedWatcher?.trigger_result,
              ).trim();
              return triggerText ? (
                <Box>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Trigger payload
                  </Typography>
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
                      fontFamily:
                        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                      fontSize: "0.78rem",
                      background: "var(--ui-rgba-0-0-0-300)",
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
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                    mb: 0.5,
                    display: "block",
                  }}
                >
                  Notifications (
                  {asRecords(selectedWatcher?.notification_attempts).length})
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
                          sx={{
                            borderBottom: "1px solid var(--ui-rgba-62-143-214-080)",
                            pb: 0.75,
                            mb: 0.25,
                          }}
                        >
                          <Stack
                            direction="row"
                            spacing={1}
                            sx={{
                              alignItems: "center",
                              mb: 0.35,
                            }}
                          >
                            <Chip
                              size="small"
                              label={
                                toBool(attempt.success) ? "sent" : "failed"
                              }
                              color={
                                toBool(attempt.success) ? "success" : "error"
                              }
                              variant="outlined"
                              sx={{ height: 20, fontSize: "0.7rem" }}
                            />
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {str(attempt.attempted_at, "").trim()
                                ? formatTimestampForHumans(
                                    str(attempt.attempted_at, ""),
                                  ).label
                                : "-"}
                            </Typography>
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {str(attempt.channel, "")}
                            </Typography>
                          </Stack>
                          {errorText ? (
                            <Typography
                              variant="caption"
                              color="error"
                              sx={{
                                whiteSpace: "pre-wrap",
                                wordBreak: "break-word",
                              }}
                            >
                              {errorText}
                            </Typography>
                          ) : message ? (
                            <Typography
                              variant="caption"
                              sx={{
                                whiteSpace: "pre-wrap",
                                wordBreak: "break-word",
                                color: "text.secondary",
                              }}
                            >
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
          {!toBool(selectedWatcher?.history_only) ? (
            <Stack direction="row" spacing={1} sx={{ mr: "auto" }}>
              <Button
                size="small"
                variant="outlined"
                disabled={
                  !str(selectedWatcher?.status, "")
                    .toLowerCase()
                    .includes("active") || runNowMutation.isPending
                }
                onClick={async () => {
                  const id = str(selectedWatcher?.id, "").trim();
                  if (!id) return;
                  setError(null);
                  try {
                    await runNowMutation.mutateAsync(id);
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Run now
              </Button>
              <Button
                size="small"
                variant="outlined"
                disabled={
                  !["active", "paused"].some((token) =>
                    str(selectedWatcher?.status, "")
                      .toLowerCase()
                      .includes(token),
                  ) || extendMutation.isPending
                }
                onClick={async () => {
                  const id = str(selectedWatcher?.id, "").trim();
                  if (!id) return;
                  setError(null);
                  try {
                    await extendMutation.mutateAsync({
                      id,
                      body: { extra_hours: 24 },
                    });
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Extend 24h
              </Button>
              <Button
                size="small"
                variant="outlined"
                disabled={
                  !["active", "paused"].some((token) =>
                    str(selectedWatcher?.status, "")
                      .toLowerCase()
                      .includes(token),
                  ) || extendMutation.isPending
                }
                onClick={async () => {
                  const id = str(selectedWatcher?.id, "").trim();
                  if (!id) return;
                  setError(null);
                  try {
                    await extendMutation.mutateAsync({
                      id,
                      body: { until_stopped: true },
                    });
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Until stopped
              </Button>
            </Stack>
          ) : null}
          <Button onClick={() => setSelectedWatcherId(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      {watchersQ.error || error ? (
        <Alert severity="error">{error || errMessage(watchersQ.error)}</Alert>
      ) : null}
    </WorkspacePageShell>
  );
}
