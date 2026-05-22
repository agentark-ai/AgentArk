import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  FormControlLabel,
  IconButton,
  Link,
  Menu,
  MenuItem,
  Switch,
  TextField,
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

type QualityDialogTarget = {
  id: string;
  title: string;
  appItem: JsonRecord;
};

type VercelPublishTarget = {
  id: string;
  title: string;
  appItem: JsonRecord;
  mode: "vercel_direct" | "vercel_git";
};

type VercelProjectMode = "auto" | "existing" | "create";

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

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => str(item, "").trim())
    .filter((item) => item.length > 0);
}

function normalizeVercelProjectMode(value: unknown): VercelProjectMode {
  const normalized = str(value, "").trim().toLowerCase();
  if (normalized === "existing") {
    return "existing";
  }
  if (normalized === "create") {
    return "create";
  }
  return "auto";
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

type AppQualityChip = {
  label: string;
  color: "default" | "success" | "warning" | "info" | "error";
  title: string;
};

function appQualityChip(appItem: JsonRecord): AppQualityChip | null {
  const report = asRecord(appItem.quality_report);
  const status = str(report.status, str(appItem.quality_report_status, ""))
    .trim()
    .toLowerCase();
  if (!status || status === "unavailable") return null;
  const coverage = asRecord(report.coverage);
  const total = num(coverage.total, 0);
  const covered = num(coverage.covered, 0);
  const missing = num(coverage.missing, Math.max(0, total - covered));
  const concerns = Array.isArray(report.judge_concerns)
    ? report.judge_concerns.length
    : 0;
  if (status === "pending") {
    return {
      label: "Review pending",
      color: "info",
      title: "Automated app review is queued.",
    };
  }
  if (status === "error") {
    return {
      label: "Review unavailable",
      color: "error",
      title: str(report.detail, "Automated app review did not complete."),
    };
  }
  if (total > 0) {
    const passed = missing === 0 && concerns === 0;
    return {
      label: passed ? "Review OK" : "Review needs attention",
      color: passed ? "success" : "warning",
      title: passed
        ? `Automated review found all ${total} requested item${total === 1 ? "" : "s"}.`
        : `Automated review found ${covered} of ${total} requested item${total === 1 ? "" : "s"}.`,
    };
  }
  if (status === "passed") {
    return {
      label: "Review OK",
      color: "success",
      title: "Automated app review found no advisory concerns.",
    };
  }
  if (status === "concerns") {
    return {
      label: "Review notes",
      color: "warning",
      title:
        concerns > 0
          ? `Automated app review has ${concerns} note${concerns === 1 ? "" : "s"}.`
          : "Automated app review has advisory notes.",
    };
  }
  return null;
}

function qualityDialogDetails(appItem: JsonRecord) {
  const report = asRecord(appItem.quality_report);
  const coverage = asRecord(report.coverage);
  const items = Array.isArray(coverage.items)
    ? coverage.items.filter(isRecord)
    : [];
  return {
    report,
    coverage,
    total: num(coverage.total, 0),
    covered: num(coverage.covered, 0),
    missing: num(
      coverage.missing,
      Math.max(0, num(coverage.total, 0) - num(coverage.covered, 0)),
    ),
    concerns: stringList(report.judge_concerns),
    items,
    status: str(report.status, str(appItem.quality_report_status, ""))
      .trim()
      .toLowerCase(),
  };
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
  const integrationsQ = useQuery({
    queryKey: ["apps-manager-integrations"],
    queryFn: api.getIntegrations,
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
  const [deleteTarget, setDeleteTarget] = useState<{
    id: string;
    title: string;
  } | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [qualityTarget, setQualityTarget] =
    useState<QualityDialogTarget | null>(null);
  const [vercelTarget, setVercelTarget] = useState<VercelPublishTarget | null>(
    null,
  );
  const [vercelToken, setVercelToken] = useState("");
  const [vercelTeamId, setVercelTeamId] = useState("");
  const [vercelProjectMode, setVercelProjectMode] =
    useState<VercelProjectMode>("auto");
  const [vercelProjectId, setVercelProjectId] = useState("");
  const [vercelProduction, setVercelProduction] = useState(false);
  const [vercelBusy, setVercelBusy] = useState(false);
  const [vercelError, setVercelError] = useState<string | null>(null);

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
    mutationFn: (payload: { app_id?: string }) =>
      api.rawPost("/tunnel/stop", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      });
      await queryClient.invalidateQueries({ queryKey: ["apps-manager"] });
    },
  });

  const appsPayload = asRecord(appsQ.data);
  const apps = pickRecords(appsPayload, "apps");
  const integrations = Array.isArray(integrationsQ.data?.integrations)
    ? integrationsQ.data.integrations
    : [];
  const vercelIntegration = integrations.find(
    (item) => str((item as JsonRecord).id, "") === "vercel",
  ) as JsonRecord | undefined;
  const vercelConfigValues = asRecord(vercelIntegration?.config_values);
  const vercelConnected =
    str(vercelIntegration?.status, "").trim().toLowerCase() === "connected";
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
  const isCloudflareQuickTunnel =
    !tunnelMeta.isPrivate &&
    str(tunnel.provider, "").trim().toLowerCase() === "cloudflare";
  const selectedPublicAppId = str(tunnel.selected_app_id, "").trim();
  const exposedPublicAppIds = new Set(stringList(tunnel.exposed_app_ids));
  if (selectedPublicAppId) exposedPublicAppIds.add(selectedPublicAppId);
  const exposedPublicAppIdsKey = Array.from(exposedPublicAppIds)
    .sort()
    .join("|");
  const tunnelControlPlaneEnabled = toBool(tunnel.control_plane_enabled);
  const tunnelExposureActive =
    tunnelActive &&
    (exposedPublicAppIds.size > 0 || tunnelControlPlaneEnabled);
  const tunnelStarting =
    tunnelActionState === "starting" || tunnelStartMutation.isPending;
  const tunnelStopping =
    tunnelActionState === "stopping" || tunnelStopMutation.isPending;
  const showQuickTunnelWarning =
    isCloudflareQuickTunnel && tunnelActive && !!tunnelBaseUrl;

  useEffect(() => {
    if (tunnelActionState === "starting") {
      if (tunnelBaseUrl || tunnelErrorText) {
        setTunnelActionState("idle");
      }
      return;
    }
    if (tunnelActionState === "stopping") {
      if (tunnelActionAppId) {
        if (!exposedPublicAppIds.has(tunnelActionAppId)) {
          setTunnelActionState("idle");
        }
      } else if (!tunnelExposureActive) {
        setTunnelActionState("idle");
      }
    }
  }, [
    tunnelActionState,
    tunnelActionAppId,
    tunnelBaseUrl,
    tunnelErrorText,
    exposedPublicAppIdsKey,
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
      queryClient.invalidateQueries({
        queryKey: ["apps-manager-integrations"],
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

  const confirmDeleteApp = async () => {
    if (!deleteTarget || deleteBusy) return;
    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await opMutation.mutateAsync({
        path: `/api/apps/${encodeURIComponent(deleteTarget.id)}`,
        method: "DELETE",
      });
      await refreshAppState();
      setAppsActionSuccess(`Delete App completed.`);
      setDeleteTarget(null);
    } catch (error) {
      setDeleteError(errMessage(error));
    } finally {
      setDeleteBusy(false);
    }
  };

  const openVercelDialog = (
    appItem: JsonRecord,
    mode: "vercel_direct" | "vercel_git",
  ) => {
    const external = asRecord(appItem.external_deployments);
    const vercel = asRecord(external.vercel || appItem.vercel_deployment);
    setVercelTarget({
      id: str(appItem.id, ""),
      title: str(appItem.title, str(appItem.id, "")),
      appItem,
      mode,
    });
    setVercelProjectMode(
      mode === "vercel_git"
        ? "existing"
        : normalizeVercelProjectMode(vercel.project_mode),
    );
    setVercelProjectId(
      str(vercel.project_id, str(vercelConfigValues.project_id, "")).trim(),
    );
    setVercelTeamId(
      str(vercel.team_id, str(vercelConfigValues.team_id, "")).trim(),
    );
    setVercelProduction(str(vercel.target, "").trim() === "production");
    setVercelToken("");
    setVercelError(null);
  };

  const publishVercelTarget = async () => {
    if (!vercelTarget || vercelBusy) return;
    setVercelBusy(true);
    setVercelError(null);
    setAppsActionError(null);
    setAppsActionSuccess(null);
    try {
      if (!vercelConnected) {
        if (!vercelToken.trim()) {
          setVercelError("Vercel access token is required.");
          return;
        }
        await api.configureIntegration("vercel", {
          token: vercelToken.trim(),
          team_id: vercelTeamId.trim() || undefined,
          project_id: vercelProjectId.trim() || undefined,
        });
        setVercelToken("");
        await queryClient.invalidateQueries({
          queryKey: ["apps-manager-integrations"],
        });
      }
      const payload = await api.rawPost(
        `/api/apps/${encodeURIComponent(vercelTarget.id)}/publish`,
        {
          deploy_target: vercelTarget.mode,
          production: vercelProduction,
          vercel_project_mode: vercelProjectMode,
          vercel_team_id: vercelTeamId.trim() || undefined,
          vercel_project_id: vercelProjectId.trim() || undefined,
        },
      );
      const deployment = asRecord(asRecord(payload).external_deployment);
      const status = str(deployment.status, str(asRecord(payload).status, ""));
      if (status === "needs_auth") {
        setVercelError("Connect Vercel before publishing this app.");
        return;
      }
      if (status === "needs_project") {
        setVercelError(str(deployment.message, "Select a Vercel project."));
        return;
      }
      if (
        status === "needs_git" ||
        status === "needs_git_push" ||
        status === "vercel_project_linked"
      ) {
        setAppsActionSuccess(str(deployment.message, "Vercel Git setup is needed."));
        setVercelTarget(null);
        await refreshAppState();
        return;
      }
      if (status === "error") {
        setVercelError(str(deployment.message, "Vercel deployment failed."));
        await refreshAppState();
        return;
      }
      if (status === "building") {
        setAppsActionSuccess(
          str(
            deployment.message,
            "Vercel accepted the deployment and it is still building.",
          ),
        );
        setVercelToken("");
        setVercelTarget(null);
        await refreshAppState();
        return;
      }
      const url = str(deployment.url, "");
      setAppsActionSuccess(
        url ? `Published to Vercel: ${url}` : "Published to Vercel.",
      );
      setVercelToken("");
      setVercelTarget(null);
      await refreshAppState();
    } catch (error) {
      setVercelError(errMessage(error));
    } finally {
      setVercelBusy(false);
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

  const stopTunnel = async (appId?: string) => {
    setTunnelActionError(null);
    setTunnelActionState("stopping");
    setTunnelActionAppId(appId || "");
    try {
      await tunnelStopMutation.mutateAsync(appId ? { app_id: appId } : {});
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
      {showQuickTunnelWarning ? (
        <Alert severity="info" variant="outlined">
          Public links from Cloudflare Quick Tunnel are temporary. They are good
          for sharing or testing now, but the address can change or stop after a
          session restart. For a permanent address, choose a production tunnel in
          Settings.
        </Alert>
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
      <Box className="list-shell" data-tour-target="apps-registry">
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
                  const isPublicAppExposed = exposedPublicAppIds.has(id);
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
                  const qualityChip = appQualityChip(appItem);
                  const canStopApp = isEnabled && !isRestoring;
                  const canRestartApp =
                    !isRestoring && (!isEnabled || !isStaticApp || isRunning);
                  const appTunnelActive =
                    tunnelActive && !!tunnelBaseUrl && isPublicAppExposed;
                  const publicUrl = appTunnelActive
                    ? toAbsoluteAppUrl(url, tunnelBaseUrl)
                    : "";
                  const hasProtectedVariant =
                    !!accessUrl && localAccessUrl !== localUrl;
                  const controlPlaneTunnelOnly =
                    tunnelActive &&
                    !!tunnelBaseUrl &&
                    exposedPublicAppIds.size === 0 &&
                    tunnelControlPlaneEnabled;
                  const publicTunnelReadyOnly =
                    tunnelActive &&
                    !!tunnelBaseUrl &&
                    exposedPublicAppIds.size === 0 &&
                    !tunnelControlPlaneEnabled;
                  const publicShareUrl = publicUrl;
                  const localShareUrl = localUrl;
                  const shareUrl = publicShareUrl || localShareUrl;
                  const externalDeployments = asRecord(appItem.external_deployments);
                  const vercelDeployment = asRecord(
                    externalDeployments.vercel || appItem.vercel_deployment,
                  );
                  const vercelUrl = str(vercelDeployment.url, "").trim();
                  const vercelStatus = str(vercelDeployment.status, "").trim();
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
                        ? tunnelActionAppId === id
                          ? tunnelMeta.isPrivate
                            ? "Starting Private Access..."
                            : "Starting Public Access..."
                          : tunnelMeta.isPrivate
                            ? "Start Private Access"
                            : "Start Public Access"
                        : isPublicAppExposed
                          ? tunnelMeta.isPrivate
                            ? "Refresh Private Exposure"
                            : "Refresh Public Exposure"
                          : tunnelMeta.isPrivate
                            ? "Start Private Access"
                            : "Start Public Access",
                      divider: true,
                      disabled: tunnelStarting || !tunnelAvailable,
                      onClick: () => startTunnel(id),
                    },
                    {
                      label: tunnelStopping
                        ? tunnelActionAppId === id
                          ? "Stopping Exposure..."
                          : tunnelMeta.isPrivate
                            ? "Stop Private Exposure"
                            : "Stop Public Exposure"
                        : tunnelMeta.isPrivate
                          ? "Stop Private Exposure"
                          : "Stop Public Exposure",
                      disabled: tunnelStopping || !isPublicAppExposed,
                      onClick: () => stopTunnel(id),
                    },
                    {
                      label: tunnelMeta.isPrivate
                        ? "Refresh Private URL"
                        : "Refresh Public Link",
                      onClick: refreshLinks,
                    },
                    {
                      label: vercelConnected
                        ? "Publish to Vercel"
                        : "Connect Vercel + Publish",
                      divider: true,
                      disabled: appsActionBusy != null || isRestoring,
                      onClick: () => openVercelDialog(appItem, "vercel_direct"),
                    },
                    {
                      label: "Vercel via Git",
                      disabled: appsActionBusy != null || isRestoring,
                      onClick: () => openVercelDialog(appItem, "vercel_git"),
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
                      onClick: () => {
                        setDeleteError(null);
                        setDeleteTarget({
                          id,
                          title: str(appItem.title, id) || id,
                        });
                      },
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
                          {qualityChip ? (
                            <Chip
                              size="small"
                              color={qualityChip.color}
                              variant={
                                qualityChip.color === "success"
                                  ? "filled"
                                  : "outlined"
                              }
                              label={qualityChip.label}
                              title={qualityChip.title}
                              onClick={() => {
                                if (!id) return;
                                setQualityTarget({
                                  id,
                                  title: str(appItem.title, id) || id,
                                  appItem,
                                });
                              }}
                            />
                          ) : null}
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
                            <>
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
                            </>
                          ) : tunnelStarting && tunnelActionAppId === id ? (
                            <Typography
                              variant="caption"
                              component="div"
                              sx={{ color: "info.main" }}
                            >
                              {shareCaptionLabel} starting tunnel...
                            </Typography>
                          ) : tunnelStopping && tunnelActionAppId === id ? (
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
                          {vercelUrl ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              title={vercelUrl}
                              sx={{ color: "success.main" }}
                            >
                              Vercel:{" "}
                              <Link
                                href={vercelUrl}
                                target="_blank"
                                rel="noopener noreferrer"
                                underline="hover"
                              >
                                {vercelUrl}
                              </Link>
                            </Typography>
                          ) : vercelStatus ? (
                            <Typography
                              variant="caption"
                              component="div"
                              noWrap
                              sx={{ color: "text.secondary" }}
                            >
                              Vercel: {vercelStatus}
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
      {qualityTarget
        ? (() => {
            const details = qualityDialogDetails(qualityTarget.appItem);
            const hasChecklist = details.total > 0 || details.items.length > 0;
            const hasConcerns = details.concerns.length > 0;
            return (
              <Dialog
                open
                onClose={() => setQualityTarget(null)}
                aria-labelledby="app-quality-dialog-title"
                maxWidth="sm"
                fullWidth
              >
                <DialogTitle id="app-quality-dialog-title">
                  Automated review
                </DialogTitle>
                <DialogContent>
                  <DialogContentText>
                    {`AgentArk checked '${qualityTarget.title}' after it was created. This helps spot missing requested pieces, but it is advisory and does not prove the app is perfect.`}
                  </DialogContentText>
                  <DialogContentText sx={{ mt: 1.25 }}>
                    Found means the live page appeared to include that requested
                    item when AgentArk opened it in a browser.
                  </DialogContentText>
                  {hasChecklist ? (
                    <Box sx={{ mt: 2 }}>
                      <Stack
                        direction="row"
                        spacing={1}
                        sx={{ alignItems: "center", flexWrap: "wrap" }}
                      >
                        <Chip
                          size="small"
                          color={details.missing > 0 ? "warning" : "success"}
                          label={`Found ${details.covered} of ${details.total}`}
                        />
                        {details.missing > 0 ? (
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            Review the items marked Needs review.
                          </Typography>
                        ) : null}
                      </Stack>
                      {details.items.length > 0 ? (
                        <Stack spacing={1} sx={{ mt: 1.5 }}>
                          {details.items.map((item, index) => {
                            const covered = item.covered;
                            const label =
                              covered === true
                                ? "Found"
                                : covered === false
                                  ? "Needs review"
                                  : "Not checked";
                            const color =
                              covered === true
                                ? "success"
                                : covered === false
                                  ? "warning"
                                  : "default";
                            return (
                              <Box
                                key={`${str(item.id, "item")}-${index}`}
                                sx={{
                                  display: "flex",
                                  gap: 1,
                                  alignItems: "flex-start",
                                }}
                              >
                                <Chip
                                  size="small"
                                  color={color}
                                  variant={
                                    covered === true ? "filled" : "outlined"
                                  }
                                  label={label}
                                  sx={{ flexShrink: 0 }}
                                />
                                <Typography variant="body2">
                                  {str(item.summary, "Requested item")}
                                </Typography>
                              </Box>
                            );
                          })}
                        </Stack>
                      ) : null}
                    </Box>
                  ) : null}
                  {hasConcerns ? (
                    <Box sx={{ mt: 2 }}>
                      <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                        Notes
                      </Typography>
                      <Stack spacing={0.75}>
                        {details.concerns.map((concern, index) => (
                          <Typography
                            key={`${concern}-${index}`}
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            {concern}
                          </Typography>
                        ))}
                      </Stack>
                    </Box>
                  ) : null}
                  {!hasChecklist && !hasConcerns ? (
                    <DialogContentText sx={{ mt: 2 }}>
                      No detailed review notes are available yet.
                    </DialogContentText>
                  ) : null}
                </DialogContent>
                <DialogActions>
                  <Button onClick={() => setQualityTarget(null)}>Close</Button>
                </DialogActions>
              </Dialog>
            );
          })()
        : null}
      <Dialog
        open={vercelTarget != null}
        onClose={() => {
          if (vercelBusy) return;
          setVercelTarget(null);
          setVercelToken("");
          setVercelError(null);
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {vercelTarget?.mode === "vercel_git"
            ? "Publish with Vercel Git"
            : "Publish to Vercel"}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1.5} sx={{ mt: 1 }}>
            <DialogContentText>
              {vercelTarget?.mode === "vercel_git"
                ? "Use the app's Git repository and connected Vercel project for deployment. If Git or project setup is missing, AgentArk will return a setup nudge."
                : "Deploy the current app bundle to Vercel through the REST API. The token is stored encrypted and is never written into app files or metadata."}
            </DialogContentText>
            {vercelError ? <Alert severity="error">{vercelError}</Alert> : null}
            {!vercelConnected ? (
              <TextField
                label="Vercel Access Token"
                type="password"
                size="small"
                value={vercelToken}
                onChange={(event) => setVercelToken(event.target.value)}
                autoComplete="off"
                fullWidth
              />
            ) : (
              <Alert severity="success" variant="outlined">
                Vercel is connected.
              </Alert>
            )}
            <TextField
              label="Team ID"
              size="small"
              value={vercelTeamId}
              onChange={(event) => setVercelTeamId(event.target.value)}
              placeholder="team_..."
              fullWidth
            />
            <TextField
              select
              label="Project Handling"
              size="small"
              value={vercelProjectMode}
              onChange={(event) =>
                setVercelProjectMode(event.target.value as VercelProjectMode)
              }
              disabled={vercelTarget?.mode === "vercel_git"}
              fullWidth
            >
              <MenuItem value="auto">Auto</MenuItem>
              <MenuItem value="existing">Existing project</MenuItem>
              <MenuItem value="create">Create project</MenuItem>
            </TextField>
            <TextField
              label={
                vercelProjectMode === "create"
                  ? "Project Name"
                  : "Project ID or Name"
              }
              size="small"
              value={vercelProjectId}
              onChange={(event) => setVercelProjectId(event.target.value)}
              placeholder={
                vercelProjectMode === "create"
                  ? "agentark-app"
                  : "my-vercel-project"
              }
              helperText={
                vercelProjectMode === "auto"
                  ? "Leave empty to let AgentArk derive a Vercel project name from the app."
                  : vercelProjectMode === "create"
                    ? "Leave empty to create a project from the app name."
                    : "Required unless a default project is saved in Vercel integration settings."
              }
              fullWidth
            />
            <FormControlLabel
              control={
                <Switch
                  checked={vercelProduction}
                  onChange={(event) => setVercelProduction(event.target.checked)}
                />
              }
              label="Production deployment"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            disabled={vercelBusy}
            onClick={() => {
              setVercelTarget(null);
              setVercelToken("");
            }}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={vercelBusy || !vercelTarget}
            onClick={publishVercelTarget}
          >
            {vercelBusy ? (
              <CircularProgress size={18} color="inherit" />
            ) : vercelTarget?.mode === "vercel_git" ? (
              "Check Git Deploy"
            ) : (
              "Publish"
            )}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={deleteTarget != null}
        onClose={() => {
          if (deleteBusy) return;
          setDeleteTarget(null);
          setDeleteError(null);
        }}
        aria-labelledby="app-delete-confirm-title"
        className="app-delete-confirm-dialog"
      >
        <DialogTitle id="app-delete-confirm-title">Delete app?</DialogTitle>
        <DialogContent className="app-delete-confirm-content">
          <DialogContentText>
            {`Delete '${deleteTarget?.title ?? ""}'?`}
          </DialogContentText>
          <DialogContentText sx={{ mt: 1 }}>
            All files, virtual environments, install caches, and run logs for
            this app will be permanently deleted. This cannot be undone.
          </DialogContentText>
          {deleteError ? (
            <Alert
              severity="error"
              sx={{ mt: 2 }}
              className="app-delete-confirm-error"
            >
              {deleteError}
            </Alert>
          ) : null}
        </DialogContent>
        <DialogActions className="app-delete-confirm-actions">
          <Button
            color="inherit"
            disabled={deleteBusy}
            onClick={() => {
              setDeleteTarget(null);
              setDeleteError(null);
            }}
          >
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={deleteBusy}
            onClick={() => {
              void confirmDeleteApp();
            }}
            startIcon={
              deleteBusy ? (
                <CircularProgress size={16} color="inherit" />
              ) : undefined
            }
          >
            Delete
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
