import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Chip,
  IconButton,
  Link,
  Menu,
  MenuItem,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import {
  getAppShareLinkLabel,
  getAppShareOpenLabel,
  getAppSharePublicCaption,
  getTunnelAccessMeta,
} from "../../lib/tunnelAccess";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";

const REFRESH_MS = 8000;
const RESTART_NOTICE_DURATION_MS = 10_000;

type JsonRecord = Record<string, unknown>;

type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

type AppsPageProps = {
  autoRefresh: boolean;
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  const record = asRecord(value);
  const rows = record[key];
  if (!Array.isArray(rows)) return [];
  return rows.filter(isRecord);
}

function str(value: unknown, fallback = "-"): string {
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
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

function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return normalized === "true" || normalized === "1" || normalized === "yes";
  }
  return false;
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

function looksLikeUrl(value: string): boolean {
  const trimmed = (value || "").trim();
  return trimmed.startsWith("http://") || trimmed.startsWith("https://");
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

function generateSuggestedAccessPassword(): string {
  const alphabet =
    "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789_-";
  const length = 20;
  if (globalThis.crypto?.getRandomValues) {
    const bytes = new Uint8Array(length);
    globalThis.crypto.getRandomValues(bytes);
    return Array.from(
      bytes,
      (value) => alphabet[value % alphabet.length],
    ).join("");
  }
  return `ap-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 12)}`;
}

function extractAccessPasswordFromUrl(
  pathOrUrl: string,
  baseOrigin: string,
): string {
  const value = (pathOrUrl || "").trim();
  if (!value) return "";
  try {
    const fallbackBase = (baseOrigin || "http://localhost").trim();
    const parsed = new URL(value, fallbackBase);
    return (
      parsed.searchParams.get("password") || parsed.searchParams.get("key") || ""
    ).trim();
  } catch {
    return "";
  }
}

function getAppAccessPasswordValue(
  appItem: JsonRecord,
  accessUrl: string,
  baseOrigin: string,
): string {
  return (
    str(appItem.access_password, "").trim() ||
    str(appItem.access_key, "").trim() ||
    extractAccessPasswordFromUrl(accessUrl, baseOrigin)
  );
}

function dedupeLinkTargets(
  targets: Array<{ label: string; url: string }>,
): Array<{ label: string; url: string }> {
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

async function copyTextWithPromptFallback(
  text: string,
  promptMessage: string,
): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    window.prompt(promptMessage, text);
  }
}

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

export default function AppsPage({ autoRefresh }: AppsPageProps) {
  const queryClient = useQueryClient();
  const appsQ = useQuery({
    queryKey: ["apps-manager"],
    queryFn: () => api.rawGet("/api/apps"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const tunnelQ = useQuery({
    queryKey: ["apps-manager-tunnel-status"],
    queryFn: () => api.rawGet("/tunnel/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const [tunnelActionError, setTunnelActionError] = useState<string | null>(
    null,
  );
  const [tunnelActionState, setTunnelActionState] = useState<
    "idle" | "starting" | "stopping"
  >("idle");
  const [tunnelActionAppId, setTunnelActionAppId] = useState("");
  const [appsActionError, setAppsActionError] = useState<string | null>(null);
  const [appsActionSuccess, setAppsActionSuccess] = useState<string | null>(
    null,
  );
  const [appsRestartNotice, setAppsRestartNotice] = useState<string | null>(
    null,
  );
  const [appsActionBusy, setAppsActionBusy] = useState<string | null>(null);

  const opMutation = useMutation({
    mutationFn: ({
      path,
      method,
      body,
    }: {
      path: string;
      method: "POST" | "DELETE";
      body?: JsonRecord;
    }) =>
      method === "DELETE" ? api.rawDelete(path) : api.rawPost(path, body ?? {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
      await queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      });
    },
  });

  const tunnelStartMutation = useMutation({
    mutationFn: (payload: { app_id?: string }) =>
      api.rawPost("/tunnel/start", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    },
  });

  const tunnelStopMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/stop", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    },
  });

  const appsPayload = asRecord(appsQ.data);
  const apps = pickRecords(appsPayload, "apps");
  const restoreInfo = asRecord(appsPayload.restore);
  const restoreActive = toBool(restoreInfo.active);
  const restoreTotal = Math.max(0, num(restoreInfo.total, apps.length));
  const restorePending = Math.max(0, num(restoreInfo.pending, 0));
  const restoreReady = Math.max(0, num(restoreInfo.ready, 0));
  const restoreDegraded = Math.max(0, num(restoreInfo.degraded, 0));
  const restoringAppsCount = apps.filter((app) => {
    const status = str(app.restore_status, "").trim().toLowerCase();
    return toBool(app.restoring) || status === "restoring";
  }).length;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const tunnel = asRecord(tunnelQ.data);
  const tunnelMeta = getTunnelAccessMeta(tunnel);
  const tunnelBaseUrl = str(tunnel.url, "").trim().replace(/\/+$/, "");
  const tunnelActive = toBool(tunnel.active);
  const tunnelAvailable = toBool(tunnel.available);
  const tunnelErrorText = str(tunnel.error, "").trim();
  const selectedPublicAppId = str(tunnel.selected_app_id, "").trim();
  const tunnelControlPlaneEnabled = toBool(tunnel.control_plane_enabled);
  const tunnelExposureActive =
    tunnelActive && (!!selectedPublicAppId || tunnelControlPlaneEnabled);
  const tunnelStarting =
    tunnelActionState === "starting" || tunnelStartMutation.isPending;
  const tunnelStopping =
    tunnelActionState === "stopping" || tunnelStopMutation.isPending;

  useEffect(() => {
    if (tunnelActionState === "starting") {
      if (tunnelBaseUrl || tunnelErrorText) {
        setTunnelActionState("idle");
      }
      return;
    }
    if (tunnelActionState === "stopping" && !tunnelExposureActive) {
      setTunnelActionState("idle");
    }
  }, [
    tunnelActionState,
    tunnelBaseUrl,
    tunnelErrorText,
    tunnelExposureActive,
  ]);

  useEffect(() => {
    if (!appsActionSuccess) return;
    const timer = window.setTimeout(() => setAppsActionSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [appsActionSuccess]);

  useEffect(() => {
    if (!appsRestartNotice) return;
    const timer = window.setTimeout(
      () => setAppsRestartNotice(null),
      RESTART_NOTICE_DURATION_MS,
    );
    return () => window.clearTimeout(timer);
  }, [appsRestartNotice]);

  useEffect(() => {
    if (!appsRestartNotice) return;
    const timer = window.setInterval(() => {
      void appsQ.refetch();
      void tunnelQ.refetch();
    }, 1200);
    return () => window.clearInterval(timer);
  }, [appsRestartNotice, appsQ, tunnelQ]);

  useEffect(() => {
    if (autoRefresh || (!restoreActive && restoringAppsCount === 0)) return;
    const timer = window.setInterval(() => {
      void appsQ.refetch();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [autoRefresh, restoreActive, restoringAppsCount, appsQ]);

  useEffect(() => {
    if (tunnelActionState === "idle") return;
    const timer = window.setInterval(() => {
      void tunnelQ.refetch();
      void appsQ.refetch();
    }, 1200);
    return () => window.clearInterval(timer);
  }, [tunnelActionState, tunnelQ, appsQ]);

  const refreshLinks = async () => {
    setTunnelActionError(null);
    await Promise.all([appsQ.refetch(), tunnelQ.refetch()]);
  };

  const refreshAppState = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["apps-manager"] }),
      queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      }),
    ]);
  };

  const runAppOp = async (opts: {
    label: string;
    path: string;
    method: "POST" | "DELETE";
    body?: JsonRecord;
  }) => {
    setAppsActionError(null);
    setAppsActionSuccess(null);
    setAppsRestartNotice(null);
    setAppsActionBusy(opts.label);
    try {
      await opMutation.mutateAsync({
        path: opts.path,
        method: opts.method,
        body: opts.body,
      });
      await refreshAppState();
      if (/\brestart\b|\breload\b/i.test(opts.label)) {
        setAppsRestartNotice(
          /reload/i.test(opts.label)
            ? "App refresh in progress. Give it up to 10 seconds. This card will disappear automatically."
            : "App restart in progress. Give it up to 10 seconds. This card will disappear automatically.",
        );
      } else {
        setAppsActionSuccess(`${opts.label} completed.`);
      }
      return true;
    } catch (error) {
      setAppsRestartNotice(null);
      setAppsActionError(errMessage(error));
      return false;
    } finally {
      setAppsActionBusy(null);
    }
  };

  const promptForAppAccessPassword = (
    message: string,
    initialValue?: string,
  ): string | null => {
    setAppsActionError(null);
    const suggestion =
      (initialValue || "").trim() || generateSuggestedAccessPassword();
    const rawValue = window.prompt(message, suggestion);
    if (rawValue == null) return null;
    const trimmed = rawValue.trim();
    if (!trimmed) {
      setAppsActionError("Access password is required.");
      return null;
    }
    if (trimmed.length > 256) {
      setAppsActionError("Access password must be 256 characters or fewer.");
      return null;
    }
    return trimmed;
  };

  const ensureAppAccessPassword = async (
    appId: string,
    appRecord?: JsonRecord,
    reason?: string,
  ) => {
    if (!appRecord) return true;
    const accessPassword = getAppAccessPasswordValue(
      appRecord,
      str(appRecord.access_url, ""),
      origin,
    );
    if (toBool(appRecord.access_guard_enabled) && accessPassword) return true;
    const appLabel = str(appRecord.title, "").trim() || appId;
    const nextPassword = promptForAppAccessPassword(
      `Set an access password for ${appLabel}${reason ? ` ${reason}` : ""}.`,
    );
    if (!nextPassword) return false;
    return runAppOp({
      label: "Set Access Password",
      path: `/api/apps/${encodeURIComponent(appId)}/access-guard`,
      method: "POST",
      body: {
        enabled: true,
        access_password: nextPassword,
      },
    });
  };

  const startTunnel = async (appId?: string) => {
    setTunnelActionError(null);
    if (appId) {
      const appRecord = apps.find((item) => str(item.id, "").trim() === appId);
      const ensured = await ensureAppAccessPassword(
        appId,
        appRecord,
        tunnelMeta.isPrivate
          ? "before starting private access"
          : "before starting public access",
      );
      if (!ensured) {
        setTunnelActionState("idle");
        return;
      }
    }
    setTunnelActionState("starting");
    setTunnelActionAppId(appId || "");
    try {
      await tunnelStartMutation.mutateAsync(appId ? { app_id: appId } : {});
      await refreshLinks();
    } catch (error) {
      setTunnelActionState("idle");
      setTunnelActionError(errMessage(error));
    }
  };

  const stopTunnel = async () => {
    setTunnelActionError(null);
    setTunnelActionState("stopping");
    try {
      await tunnelStopMutation.mutateAsync();
      await refreshLinks();
    } catch (error) {
      setTunnelActionState("idle");
      setTunnelActionError(errMessage(error));
    }
  };

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Agent"
        title="Apps"
        description="Manage deployed app runtime, health, and local access."
      />
      {tunnelQ.error ? <Alert severity="error">{errMessage(tunnelQ.error)}</Alert> : null}
      {tunnelErrorText ? <Alert severity="error">{tunnelErrorText}</Alert> : null}
      {tunnelActionError ? (
        <Alert severity="error">{tunnelActionError}</Alert>
      ) : null}
      {appsActionError ? <Alert severity="error">{appsActionError}</Alert> : null}
      {appsActionSuccess ? (
        <Alert severity="success">{appsActionSuccess}</Alert>
      ) : null}
      {appsRestartNotice ? (
        <Box className="settings-inline-card tone-info">
          <Stack
            className="settings-inline-card-head"
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", sm: "center" },
            }}
          >
            <Box className="settings-inline-card-copy">
              <Typography className="settings-inline-card-kicker">
                Restarting
              </Typography>
              <Typography className="settings-inline-card-title">
                App changes are being applied
              </Typography>
              <Typography className="settings-inline-card-description">
                {appsRestartNotice}
              </Typography>
            </Box>
            <Chip
              size="small"
              icon={<AutorenewRoundedIcon />}
              label="Up to 10 seconds"
              color="info"
              variant="outlined"
            />
          </Stack>
        </Box>
      ) : null}
      {restoreActive ? (
        <Alert severity="info">
          Restoring {restoreTotal} app{restoreTotal === 1 ? "" : "s"} in the
          background. {restorePending} remaining, {restoreReady} ready
          {restoreDegraded > 0 ? `, ${restoreDegraded} degraded` : ""}.
        </Alert>
      ) : null}
      <Box className="list-shell">
        <TableContainer className="table-shell">
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Title</TableCell>
                <TableCell>ID</TableCell>
                <TableCell>Status</TableCell>
                <TableCell>Links</TableCell>
                <TableCell align="right">Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {apps.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5}>
                    <Typography variant="body2" sx={{ color: "text.secondary" }}>
                      {restoreActive
                        ? "App restore is still running in the background. This list will populate as saved apps are discovered."
                        : "There are no deployed apps at this time. When you create any app with agent, it will show here."}
                    </Typography>
                  </TableCell>
                </TableRow>
              ) : (
                apps.map((appItem) => {
                  const id = str(appItem.id, "");
                  const url = str(appItem.url, "");
                  const accessUrl = str(appItem.access_url, "");
                  const accessPassword = getAppAccessPasswordValue(
                    appItem,
                    accessUrl,
                    origin,
                  );
                  const localUrl = toAbsoluteAppUrl(url, origin);
                  const localAccessUrl = toAbsoluteAppUrl(
                    accessUrl || url,
                    origin,
                  );
                  const isSelectedPublicApp = selectedPublicAppId === id;
                  const runtimeMode = str(appItem.runtime_mode, "")
                    .trim()
                    .toLowerCase();
                  const restoreStatus = str(appItem.restore_status, "")
                    .trim()
                    .toLowerCase();
                  const isRestoring =
                    toBool(appItem.restoring) || restoreStatus === "restoring";
                  const restoreError = str(appItem.restore_error, "").trim();
                  const isStaticApp = runtimeMode === "static";
                  const isEnabled =
                    appItem.enabled === undefined ? true : toBool(appItem.enabled);
                  const isRunning = toBool(appItem.running);
                  const canStopApp = isEnabled && !isRestoring;
                  const canRestartApp =
                    !isRestoring && (!isEnabled || !isStaticApp || isRunning);
                  const appTunnelActive =
                    tunnelActive && !!tunnelBaseUrl && isSelectedPublicApp;
                  const publicUrl = appTunnelActive
                    ? toAbsoluteAppUrl(url, tunnelBaseUrl)
                    : "";
                  const hasProtectedVariant =
                    !!accessUrl && localAccessUrl !== localUrl;
                  const controlPlaneTunnelOnly =
                    tunnelActive &&
                    !!tunnelBaseUrl &&
                    !selectedPublicAppId &&
                    tunnelControlPlaneEnabled;
                  const publicTunnelReadyOnly =
                    tunnelActive &&
                    !!tunnelBaseUrl &&
                    !selectedPublicAppId &&
                    !tunnelControlPlaneEnabled;
                  const publicShareUrl = publicUrl;
                  const localShareUrl = localUrl;
                  const shareUrl = publicShareUrl || localShareUrl;
                  const shareCaptionLabel =
                    getAppSharePublicCaption(tunnelMeta);
                  const shareOpenLabel = getAppShareOpenLabel(tunnelMeta);
                  const shareCopyLabel = getAppShareLinkLabel(tunnelMeta);
                  const openTargets = dedupeLinkTargets([
                    { label: "Open Local", url: localUrl },
                    { label: shareOpenLabel, url: publicUrl },
                  ]);
                  const actions: RowMenuAction[] = [
                    ...openTargets.map((target, index) => ({
                      label: target.label,
                      divider: index === 0 ? false : undefined,
                      onClick: () => {
                        window.open(
                          target.url,
                          "_blank",
                          "noopener,noreferrer",
                        );
                      },
                    })),
                    {
                      label: publicShareUrl
                        ? shareCopyLabel
                        : "Copy Local Link",
                      divider: openTargets.length > 0,
                      disabled: !shareUrl,
                      onClick: async () => {
                        if (!shareUrl) return;
                        await copyTextWithPromptFallback(
                          shareUrl,
                          "Copy this link",
                        );
                      },
                    },
                    ...(accessPassword
                      ? [
                          {
                            label: "Copy Access Password",
                            onClick: async () => {
                              await copyTextWithPromptFallback(
                                accessPassword,
                                "Copy this access password",
                              );
                            },
                          },
                        ]
                      : []),
                    {
                      label: toBool(appItem.access_guard_enabled)
                        ? "Disable App Guard"
                        : "Enable App Guard",
                      disabled: appsActionBusy != null,
                      onClick: () => {
                        if (toBool(appItem.access_guard_enabled)) {
                          void runAppOp({
                            label: "Disable App Guard",
                            path: `/api/apps/${encodeURIComponent(id)}/access-guard`,
                            method: "POST",
                            body: {
                              enabled: false,
                            },
                          });
                          return;
                        }
                        const nextPassword = promptForAppAccessPassword(
                          `Set an access password for ${str(appItem.title, id) || id}.`,
                        );
                        if (!nextPassword) return;
                        void runAppOp({
                          label: "Enable App Guard",
                          path: `/api/apps/${encodeURIComponent(id)}/access-guard`,
                          method: "POST",
                          body: {
                            enabled: true,
                            access_password: nextPassword,
                          },
                        });
                      },
                    },
                    {
                      label: "Change Access Password",
                      disabled:
                        appsActionBusy != null ||
                        !toBool(appItem.access_guard_enabled),
                      onClick: () => {
                        const nextPassword = promptForAppAccessPassword(
                          `Set a new access password for ${str(appItem.title, id) || id}.`,
                        );
                        if (!nextPassword) return;
                        void runAppOp({
                          label: "Change Access Password",
                          path: `/api/apps/${encodeURIComponent(id)}/access-guard`,
                          method: "POST",
                          body: {
                            enabled: true,
                            access_password: nextPassword,
                          },
                        });
                      },
                    },
                    {
                      label: tunnelStarting
                        ? tunnelMeta.isPrivate
                          ? "Starting Private Access..."
                          : "Starting Public Tunnel..."
                        : tunnelActive && selectedPublicAppId === id
                          ? tunnelMeta.isPrivate
                            ? "Refresh Private Exposure"
                            : "Refresh Public Exposure"
                          : tunnelActive &&
                              selectedPublicAppId &&
                              selectedPublicAppId !== id
                            ? tunnelMeta.isPrivate
                              ? "Set as Private Landing App"
                              : "Set as Public Landing App"
                            : tunnelMeta.isPrivate
                              ? "Start Private Access"
                              : "Start Public Tunnel",
                      divider: true,
                      disabled: tunnelStarting || !tunnelAvailable,
                      onClick: () => startTunnel(id),
                    },
                    {
                      label: tunnelStopping
                        ? "Stopping Exposure..."
                        : tunnelMeta.isPrivate
                          ? "Stop Private Exposure"
                          : "Stop Public Exposure",
                      disabled: tunnelStopping || !tunnelExposureActive,
                      onClick: stopTunnel,
                    },
                    {
                      label: tunnelMeta.isPrivate
                        ? "Refresh Private URL"
                        : "Refresh Public Link",
                      onClick: refreshLinks,
                    },
                    {
                      label: !canStopApp ? "Stop Unavailable" : "Stop",
                      divider: true,
                      disabled: !canStopApp || appsActionBusy != null,
                      onClick: () =>
                        void runAppOp({
                          label: "Stop App",
                          path: `/api/apps/${encodeURIComponent(id)}/stop`,
                          method: "POST",
                        }),
                    },
                    {
                      label: !isEnabled
                        ? "Start App"
                        : isStaticApp
                          ? "Reload Metadata"
                          : isRunning
                            ? "Restart"
                            : "Start App",
                      disabled: appsActionBusy != null || !canRestartApp,
                      onClick: () =>
                        void runAppOp({
                          label: !isEnabled
                            ? "Start App"
                            : isStaticApp
                              ? "Reload Metadata"
                              : isRunning
                                ? "Restart App"
                                : "Start App",
                          path: `/api/apps/${encodeURIComponent(id)}/restart`,
                          method: "POST",
                        }),
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
                          method: "DELETE",
                        }),
                    },
                  ];

                  return (
                    <TableRow key={id}>
                      <TableCell>{str(appItem.title)}</TableCell>
                      <TableCell>{id}</TableCell>
                      <TableCell>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{ flexWrap: "wrap" }}
                        >
                          {!isEnabled ? (
                            <Chip
                              size="small"
                              color="default"
                              label="Disabled"
                            />
                          ) : isRestoring ? (
                            <Chip size="small" color="info" label="Restoring" />
                          ) : restoreStatus === "degraded" ? (
                            <Chip
                              size="small"
                              color="warning"
                              label="Degraded"
                            />
                          ) : isRunning ? (
                            <Chip
                              size="small"
                              color="success"
                              label="Running"
                            />
                          ) : (
                            <Chip
                              size="small"
                              variant="outlined"
                              label="Stopped"
                            />
                          )}
                        </Stack>
                      </TableCell>
                      <TableCell sx={{ maxWidth: 420 }}>
                        <Stack spacing={0.2}>
                          {localUrl ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              title={localUrl}
                            >
                              Local:{" "}
                              <Link
                                href={localUrl}
                                target="_blank"
                                rel="noopener noreferrer"
                                underline="hover"
                              >
                                {localUrl}
                              </Link>
                            </Typography>
                          ) : (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              title={url || "-"}
                            >
                              Local: {url || "-"}
                            </Typography>
                          )}
                          {hasProtectedVariant ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              title={accessPassword || localAccessUrl}
                            >
                              Access Password: {accessPassword || "-"}
                            </Typography>
                          ) : null}
                          {toBool(appItem.access_guard_enabled) ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              sx={{ color: "warning.main" }}
                            >
                              Guard enabled
                            </Typography>
                          ) : null}
                          {!isEnabled ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "text.secondary" }}
                            >
                              Disabled until you start it again from this page.
                            </Typography>
                          ) : null}
                          {isRestoring ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "info.main" }}
                            >
                              Restore is still bringing this runtime up in the
                              background.
                            </Typography>
                          ) : null}
                          {restoreError ? (
                            <Typography
                              variant="caption"
                              component="div"
                              title={restoreError}
                              sx={{ color: "warning.main" }}
                            >
                              Restore note: {restoreError}
                            </Typography>
                          ) : null}
                          {publicShareUrl ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              title={publicShareUrl}
                              sx={{ color: "info.main" }}
                            >
                              {shareCaptionLabel}{" "}
                              <Link
                                href={publicShareUrl}
                                target="_blank"
                                rel="noopener noreferrer"
                                underline="hover"
                              >
                                {publicShareUrl}
                              </Link>
                            </Typography>
                          ) : tunnelStarting && tunnelActionAppId === id ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "info.main" }}
                            >
                              {shareCaptionLabel} starting tunnel...
                            </Typography>
                          ) : tunnelStopping && isSelectedPublicApp ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "text.secondary" }}
                            >
                              {shareCaptionLabel} stopping tunnel...
                            </Typography>
                          ) : controlPlaneTunnelOnly ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "text.secondary" }}
                            >
                              {shareCaptionLabel} control-plane access is active.
                              Expose this app to get a working app link.
                            </Typography>
                          ) : publicTunnelReadyOnly ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "text.secondary" }}
                            >
                              {shareCaptionLabel} infrastructure is ready. Expose
                              this app to get a working app link.
                            </Typography>
                          ) : null}
                          {toBool(appItem.access_guard_enabled) &&
                          (publicShareUrl || localShareUrl) ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              sx={{ color: "warning.main" }}
                            >
                              Visitors will be asked for the access password.
                            </Typography>
                          ) : null}
                        </Stack>
                      </TableCell>
                      <TableCell align="right">
                        <RowOpsMenu actions={actions} ariaLabel="App options" />
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>
    </WorkspacePageShell>
  );
}
