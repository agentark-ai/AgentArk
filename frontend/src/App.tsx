import {
  Alert,
  AppBar,
  Badge,
  Box,
  Button,
  Dialog,
  DialogContent,
  DialogTitle,
  Drawer,
  Divider,
  IconButton,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Popover,
  Stack,
  Toolbar,
  Tooltip,
  Typography
} from "@mui/material";
import AppsRoundedIcon from "@mui/icons-material/AppsRounded";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import BoltRoundedIcon from "@mui/icons-material/BoltRounded";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import DescriptionRoundedIcon from "@mui/icons-material/DescriptionRounded";
import FlagRoundedIcon from "@mui/icons-material/FlagRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import NotificationsNoneRoundedIcon from "@mui/icons-material/NotificationsNoneRounded";
import FolderRoundedIcon from "@mui/icons-material/FolderRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import MonitorHeartRoundedIcon from "@mui/icons-material/MonitorHeartRounded";
import SpaceDashboardRoundedIcon from "@mui/icons-material/SpaceDashboardRounded";
import QueryStatsRoundedIcon from "@mui/icons-material/QueryStatsRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "./api/client";
import { GuidedTour } from "./components/GuidedTour";
import { NativeWorkspace } from "./components/NativeWorkspace";
import { OverviewPane } from "./components/OverviewPane";
import { useUiStore } from "./store/uiStore";

const REFRESH_MS = 8000;
const PING_STALE_MS = 30_000;
const SIDEBAR_COLLAPSED_KEY = "agentark.sidebar.collapsed";
const NOTIFICATIONS_MUTE_UNTIL_KEY = "agentark.notifications.mute_until_v1";

type ViewKey =
  | "overview"
  | "chat"
  | "skills"
  | "tasks"
  | "apps"
  | "memory"
  | "goals"
  | "autonomy"
  | "trace"
  | "status"
  | "swarm"
  | "projects"
  | "documents"
  | "analytics"
  | "settings";

type NavItem = { key: ViewKey; label: string; icon: ReactNode };
type NavGroup = { id: string; label: string; items: NavItem[] };

const NAV_GROUPS: NavGroup[] = [
  {
    id: "core",
    label: "Core",
    items: [
      { key: "overview", label: "Mission Control", icon: <SpaceDashboardRoundedIcon fontSize="small" /> },
      { key: "chat", label: "Chat", icon: <ChatRoundedIcon fontSize="small" /> }
    ]
  },
  {
    id: "agent",
    label: "Agent",
    items: [
      { key: "skills", label: "Skills", icon: <BoltRoundedIcon fontSize="small" /> },
      { key: "apps", label: "Apps", icon: <AppsRoundedIcon fontSize="small" /> },
      { key: "goals", label: "Goals", icon: <FlagRoundedIcon fontSize="small" /> },
      { key: "autonomy", label: "Autonomy", icon: <AutoAwesomeRoundedIcon fontSize="small" /> }
    ]
  },
  {
    id: "data",
    label: "Data",
    items: [
      { key: "documents", label: "Documents", icon: <DescriptionRoundedIcon fontSize="small" /> },
      { key: "analytics", label: "Analytics", icon: <QueryStatsRoundedIcon fontSize="small" /> }
    ]
  }
  // { key: "swarm", label: "Swarm", icon: <HubRoundedIcon fontSize="small" /> },
  // { key: "status", label: "Status", icon: <MonitorHeartRoundedIcon fontSize="small" /> },
];

const VIEW_PATH_SEGMENTS: Record<ViewKey, string> = {
  overview: "overview",
  chat: "chat",
  skills: "skills",
  tasks: "tasks",
  apps: "apps",
  memory: "memory",
  goals: "goals",
  autonomy: "autonomy",
  trace: "trace",
  status: "status",
  swarm: "swarm",
  projects: "projects",
  documents: "documents",
  analytics: "analytics",
  settings: "settings"
};

const PATH_SEGMENT_TO_VIEW: Record<string, ViewKey> = Object.entries(VIEW_PATH_SEGMENTS).reduce(
  (acc, [view, segment]) => {
    acc[segment] = view as ViewKey;
    return acc;
  },
  {} as Record<string, ViewKey>
);

function viewPath(view: ViewKey): string {
  return `/ui/${VIEW_PATH_SEGMENTS[view]}`;
}

function resolveViewFromPath(pathname: string): { view: ViewKey; matched: boolean } {
  const normalized = pathname.replace(/\/+$/, "");
  if (
    normalized === "" ||
    normalized === "/" ||
    normalized === "/ui" ||
    normalized === "/ui/v2"
  ) {
    return { view: "overview", matched: true };
  }

  if (normalized.startsWith("/ui/")) {
    const segment = normalized.slice("/ui/".length).split("/")[0]?.toLowerCase() || "";
    if (segment === "actions") return { view: "skills", matched: true };
    if (segment === "integrations") return { view: "settings", matched: true };
    if (segment === "trace") return { view: "settings", matched: true };
    if (segment === "memory") return { view: "settings", matched: true };
    const view = PATH_SEGMENT_TO_VIEW[segment];
    if (view) {
      if (view === "tasks") return { view: "skills", matched: true };
      return { view, matched: true };
    }
  }

  return { view: "overview", matched: false };
}

function formatMetaValue(value: unknown): { text: string; href?: string } {
  if (value == null) return { text: "-" };
  if (typeof value === "string") {
    const v = value.trim();
    if (v.startsWith("http://") || v.startsWith("https://")) return { text: v, href: v };
    return { text: v };
  }
  if (typeof value === "number") return { text: Number.isFinite(value) ? String(value) : "-" };
  if (typeof value === "boolean") return { text: value ? "true" : "false" };
  if (Array.isArray(value)) return { text: `List (${value.length})` };
  if (typeof value === "object") {
    const rec = value as Record<string, unknown>;
    const keys = Object.keys(rec || {});
    const keyHint = keys.slice(0, 4).join(", ");
    const more = keys.length > 4 ? `, +${keys.length - 4}` : "";
    return { text: keys.length ? `Object(${keyHint}${more})` : "Object" };
  }
  return { text: String(value) };
}

function isAutomationFailureNotification(notification: {
  title?: string;
  body?: string;
  source?: string;
  level?: string;
}): boolean {
  const title = (notification.title || "").toLowerCase();
  const body = (notification.body || "").toLowerCase();
  const source = (notification.source || "").toLowerCase();
  const level = (notification.level || "").toLowerCase();
  const text = `${title} ${body} ${source}`;
  const failureSignal =
    level === "error" ||
    level === "critical" ||
    text.includes("failed") ||
    text.includes("failure") ||
    text.includes("error");
  const automationSignal =
    source.includes("automation") ||
    source.includes("task") ||
    source.includes("watcher") ||
    source.includes("hook") ||
    text.includes("automation") ||
    text.includes("scheduled") ||
    text.includes("watcher") ||
    text.includes("task");
  return failureSignal && automationSignal;
}

export default function App() {
  const queryClient = useQueryClient();
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const selectedNotificationId = useUiStore((s) => s.selectedNotificationId);
  const openNotification = useUiStore((s) => s.openNotification);
  const closeNotification = useUiStore((s) => s.closeNotification);
  const [view, setViewState] = useState<ViewKey>(() => resolveViewFromPath(window.location.pathname).view);
  const [lastNonSettingsView, setLastNonSettingsView] = useState<ViewKey>("overview");
  const [sidebarCollapsed, setSidebarCollapsed] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const showAdvanced = useUiStore((s) => s.showAdvancedByView[view] ?? false);
  const tourActive = useUiStore((s) => s.tourActive);
  const tourCompleted = useUiStore((s) => s.tourCompleted);
  const startTour = useUiStore((s) => s.startTour);

  // Auto-start guided tour on first launch
  useEffect(() => {
    if (!tourCompleted && !tourActive) {
      const timer = setTimeout(() => startTour(), 800);
      return () => clearTimeout(timer);
    }
  }, []);

  // Expand sidebar when tour is active
  useEffect(() => {
    if (tourActive) setSidebarCollapsed(false);
  }, [tourActive]);

  const [notifAnchorEl, setNotifAnchorEl] = useState<HTMLElement | null>(null);
  const notifListOpen = Boolean(notifAnchorEl);
  const [notifFilter, setNotifFilter] = useState<"all" | "unread" | "errors" | "automation_failures">("all");
  const [notificationControlNotice, setNotificationControlNotice] = useState<string | null>(null);
  const [notificationsMuteUntilMs, setNotificationsMuteUntilMs] = useState<number>(() => {
    try {
      const raw = window.localStorage.getItem(NOTIFICATIONS_MUTE_UNTIL_KEY);
      const parsed = raw ? Number(raw) : Number.NaN;
      return Number.isFinite(parsed) ? parsed : 0;
    } catch {
      return 0;
    }
  });

  const navigateToView = (nextView: ViewKey, replace = false) => {
    const nextPath = viewPath(nextView);
    if (window.location.pathname !== nextPath) {
      const nextUrl = `${nextPath}${window.location.search}`;
      if (replace) {
        window.history.replaceState(null, "", nextUrl);
      } else {
        window.history.pushState(null, "", nextUrl);
      }
    }
    setViewState(nextView);
  };

  useEffect(() => {
    const syncFromLocation = (replaceInvalid: boolean) => {
      const normalizedPath = window.location.pathname.replace(/\/+$/, "");
      if (replaceInvalid && normalizedPath.startsWith("/ui/memory")) {
        const params = new URLSearchParams(window.location.search);
        if (!params.get("settings_tab")) params.set("settings_tab", "memory");
        const nextUrl = `/ui/settings?${params.toString()}`;
        window.history.replaceState(null, "", nextUrl);
        setViewState("settings");
        return;
      }
      const resolved = resolveViewFromPath(window.location.pathname);
      setViewState(resolved.view);
      const canonicalPath = viewPath(resolved.view);
      const shouldNormalizePath =
        window.location.pathname.startsWith("/ui") &&
        window.location.pathname !== canonicalPath;
      if (replaceInvalid && (shouldNormalizePath || !resolved.matched)) {
        const nextUrl = `${canonicalPath}${window.location.search}`;
        window.history.replaceState(null, "", nextUrl);
      }
    };

    syncFromLocation(true);
    const onPopState = () => syncFromLocation(false);
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  useEffect(() => {
    if (view !== "settings") {
      setLastNonSettingsView(view);
    }
  }, [view]);

  useEffect(() => {
    try {
      window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, sidebarCollapsed ? "1" : "0");
    } catch {
      // ignore storage failures
    }
  }, [sidebarCollapsed]);

  const serverQ = useQuery({
    queryKey: ["server-ping"],
    queryFn: async () => {
      const t0 = typeof performance !== "undefined" ? performance.now() : Date.now();
      const status = await api.getStatus();
      const t1 = typeof performance !== "undefined" ? performance.now() : Date.now();
      return { at: Date.now(), rtt_ms: Math.max(0, Math.round(t1 - t0)), status };
    },
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    retry: 0
  });

  const notificationsQ = useQuery({
    queryKey: ["notifications"],
    queryFn: api.getNotifications,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const notificationsCountQ = useQuery({
    queryKey: ["notifications-count"],
    queryFn: () => api.rawGet("/notifications/count"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const notifications = Array.isArray(notificationsQ.data) ? notificationsQ.data : [];
  const unreadCountFromEndpointRaw =
    notificationsCountQ.data && typeof notificationsCountQ.data === "object"
      ? (notificationsCountQ.data as Record<string, unknown>).unread
      : null;
  const unreadCountFromEndpoint =
    typeof unreadCountFromEndpointRaw === "number"
      ? unreadCountFromEndpointRaw
      : typeof unreadCountFromEndpointRaw === "string"
        ? Number(unreadCountFromEndpointRaw)
        : Number.NaN;
  const unreadCount = Number.isFinite(unreadCountFromEndpoint)
    ? Math.max(0, Math.round(unreadCountFromEndpoint))
    : notifications.filter((n) => !n.read).length;
  const filteredNotifications = useMemo(() => {
    if (notifFilter === "all") return notifications;
    if (notifFilter === "unread") return notifications.filter((n) => !n.read);
    if (notifFilter === "errors") {
      return notifications.filter((n) => {
        const level = (n.level || "").toLowerCase();
        return level === "error" || level === "critical";
      });
    }
    return notifications.filter((n) => isAutomationFailureNotification(n));
  }, [notifications, notifFilter]);
  const notificationsMuted = notificationsMuteUntilMs > Date.now();
  let selectedNotification: (typeof notifications)[number] | null = null;
  for (const n of notifications) {
    if (n.id === selectedNotificationId) {
      selectedNotification = n;
      break;
    }
  }

  useEffect(() => {
    try {
      if (notificationsMuteUntilMs > Date.now()) {
        window.localStorage.setItem(
          NOTIFICATIONS_MUTE_UNTIL_KEY,
          String(notificationsMuteUntilMs)
        );
      } else {
        window.localStorage.removeItem(NOTIFICATIONS_MUTE_UNTIL_KEY);
      }
    } catch {
      // ignore storage failures
    }
  }, [notificationsMuteUntilMs]);

  useEffect(() => {
    if (!notificationControlNotice) return;
    const t = window.setTimeout(() => setNotificationControlNotice(null), 3500);
    return () => window.clearTimeout(t);
  }, [notificationControlNotice]);

  const now = Date.now();
  const lastPingAt = serverQ.data?.at ?? 0;
  const pingAge = lastPingAt ? now - lastPingAt : Number.POSITIVE_INFINITY;
  const pingStale = pingAge > PING_STALE_MS;
  const serverTooltip = serverQ.isError
    ? "Server Offline"
    : serverQ.isLoading && !serverQ.data
      ? "Connecting..."
      : serverQ.data && !pingStale
        ? `Server Online \u2022 ${serverQ.data.rtt_ms}ms`
        : "Server Status Unknown";
  const serverDotColor =
    serverQ.isError ? "#f44336" : serverQ.data && !pingStale ? "#4caf50" : "#ff9800";
  const serverPulse = !serverQ.isError && serverQ.data && !pingStale;

  const markReadMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/notifications/${encodeURIComponent(id)}/read`, {}),
    onMutate: async (id: string) => {
      await queryClient.cancelQueries({ queryKey: ["notifications"] });
      const previous = queryClient.getQueryData(["notifications"]);
      queryClient.setQueryData(["notifications"], (old: unknown) => {
        if (!Array.isArray(old)) return old;
        return old.map((row) => {
          const item = row as Record<string, unknown>;
          return String(item.id || "") === id ? { ...item, read: true } : item;
        });
      });
      return { previous };
    },
    onError: (_error, _id, context) => {
      if (context?.previous !== undefined) {
        queryClient.setQueryData(["notifications"], context.previous);
      }
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
    }
  });

  const markAllMutation = useMutation({
    mutationFn: () => api.rawPost("/notifications/read-all", {}),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["notifications"] });
      const previous = queryClient.getQueryData(["notifications"]);
      queryClient.setQueryData(["notifications"], (old: unknown) => {
        if (!Array.isArray(old)) return old;
        return old.map((row) => ({ ...(row as Record<string, unknown>), read: true }));
      });
      return { previous };
    },
    onError: (_error, _vars, context) => {
      if (context?.previous !== undefined) {
        queryClient.setQueryData(["notifications"], context.previous);
      }
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
    }
  });

  const notificationControlMutation = useMutation({
    mutationFn: async (command: "stop notifications" | "resume notifications") => {
      const out = await api.chat({ message: command, channel: "web" });
      const rec = out as unknown as Record<string, unknown>;
      const text =
        (typeof rec.response === "string" ? rec.response : "") ||
        (typeof rec.message === "string" ? rec.message : "");
      return { text: text.trim(), command };
    },
    onSuccess: async ({ text, command }) => {
      if (command === "stop notifications") {
        setNotificationsMuteUntilMs(Date.now() + 24 * 60 * 60 * 1000);
      } else {
        setNotificationsMuteUntilMs(0);
      }
      setNotificationControlNotice(
        text || (command === "stop notifications" ? "Alerts paused for 24h." : "Alerts resumed.")
      );
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
    },
    onError: (err) => {
      setNotificationControlNotice(
        `Alert setting failed: ${err instanceof Error ? err.message : "unknown error"}`
      );
    }
  });

  const closeSettingsModal = () => {
    const fallback = lastNonSettingsView === "settings" ? "overview" : lastNonSettingsView;
    navigateToView(fallback, true);
  };

  const activeView: ViewKey = view === "settings" ? lastNonSettingsView : view;

  return (
      <Box className="agi-shell">
        <Box className="bg-orb orb-a" />
        <Box className="bg-orb orb-b" />
      <AppBar position="sticky" elevation={0} color="transparent" className="glass-appbar">
        <Toolbar sx={{ minHeight: "var(--appbar-height)", px: 1.25 }}>
          <Stack direction="row" alignItems="center" spacing={0.75} sx={{ flexGrow: 1 }}>
            <img src="/logo.svg" alt="AgentArk" width={42} height={42} />
            <Typography variant="h6">AgentArk Console</Typography>
            <Tooltip title={serverTooltip} arrow>
              <Box
                onClick={() => serverQ.refetch()}
                sx={{
                  width: 10,
                  height: 10,
                  borderRadius: "50%",
                  backgroundColor: serverDotColor,
                  cursor: "pointer",
                  ml: 0.5,
                  boxShadow: serverPulse ? `0 0 6px 2px ${serverDotColor}` : "none",
                  animation: serverPulse ? "pulse-dot 2s ease-in-out infinite" : "none",
                  "@keyframes pulse-dot": {
                    "0%, 100%": { boxShadow: `0 0 4px 1px ${serverDotColor}` },
                    "50%": { boxShadow: `0 0 8px 3px ${serverDotColor}` },
                  },
                }}
              />
            </Tooltip>
          </Stack>
          <Tooltip title="Notifications">
            <IconButton
              color="primary"
              onClick={(e) => setNotifAnchorEl(e.currentTarget)}
              aria-label="Open notifications"
            >
              <Badge badgeContent={unreadCount} color="warning" max={99}>
                <NotificationsNoneRoundedIcon />
              </Badge>
            </IconButton>
          </Tooltip>
          <Tooltip title="Projects">
            <IconButton color="primary" onClick={() => navigateToView("projects")}>
              <FolderRoundedIcon />
            </IconButton>
          </Tooltip>
          <Tooltip title="Settings">
            <IconButton color="primary" onClick={() => navigateToView("settings")}>
              <SettingsRoundedIcon />
            </IconButton>
          </Tooltip>
        </Toolbar>
      </AppBar>

      <Box className={`main-grid${sidebarCollapsed ? " nav-collapsed" : ""}`}>
        <Box className={`side-nav${sidebarCollapsed ? " collapsed" : ""}`}>
          <Stack direction="row" alignItems="center" justifyContent={sidebarCollapsed ? "center" : "space-between"} sx={{ px: 0.5, mb: 1 }}>
            {!sidebarCollapsed ? (
              <Typography variant="caption" className="nav-label">
                Navigation
              </Typography>
            ) : null}
            <Tooltip title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}>
              <IconButton
                size="small"
                className="nav-collapse-btn"
                onClick={() => setSidebarCollapsed((prev) => !prev)}
                aria-label={sidebarCollapsed ? "Expand navigation sidebar" : "Collapse navigation sidebar"}
              >
                {sidebarCollapsed ? <ChevronRightRoundedIcon fontSize="small" /> : <ChevronLeftRoundedIcon fontSize="small" />}
              </IconButton>
            </Tooltip>
          </Stack>
          <List dense>
            {NAV_GROUPS.map((group, groupIdx) => (
              <Box key={group.id} className="nav-group">
                {!sidebarCollapsed ? (
                  <Typography variant="overline" className="nav-group-label">
                    {group.label}
                  </Typography>
                ) : null}
                {group.items.map((item) => (
                  <Tooltip
                    key={item.key}
                    title={item.label}
                    placement="right"
                    disableHoverListener={!sidebarCollapsed}
                  >
                    <ListItemButton
                      selected={view === item.key}
                      onClick={() => navigateToView(item.key)}
                      className={`nav-item${sidebarCollapsed ? " collapsed" : ""}`}
                      data-tour-target={`nav-${item.key}`}
                    >
                      <ListItemIcon className="nav-item-icon">{item.icon}</ListItemIcon>
                      <ListItemText
                        className={`nav-item-text${sidebarCollapsed ? " collapsed" : ""}`}
                        primary={item.label}
                        primaryTypographyProps={{ noWrap: true }}
                      />
                    </ListItemButton>
                  </Tooltip>
                ))}
                {groupIdx < NAV_GROUPS.length - 1 ? (
                  <Divider className="nav-group-divider" />
                ) : null}
              </Box>
            ))}
          </List>
        </Box>

        <Box className="main-pane">
            {activeView === "overview" ? (
              <OverviewPane
                navigateToView={navigateToView as (view: string, replace?: boolean) => void}
                serverStatus={serverQ.data}
                serverError={serverQ.isError}
                serverLoading={serverQ.isLoading && !serverQ.data}
              />
            ) : (
            <NativeWorkspace
              view={activeView as Exclude<ViewKey, "overview">}
              autoRefresh={autoRefresh}
              showAdvanced={showAdvanced}
            />
          )}
        </Box>
      </Box>

      <Dialog
        open={view === "settings"}
        onClose={closeSettingsModal}
        fullWidth
        maxWidth={false}
        PaperProps={{
          sx: {
            width: { xs: "96vw", md: "82vw", lg: 1120 },
            maxWidth: 1120,
            height: { xs: "92vh", md: "84vh" },
            maxHeight: "92vh",
            borderRadius: 3,
            border: "1px solid rgba(108, 156, 212, 0.16)",
            background: "linear-gradient(160deg, rgba(9, 21, 39, 0.96), rgba(9, 21, 39, 0.78))",
            backdropFilter: "blur(18px)",
            WebkitBackdropFilter: "blur(18px)",
            overflow: "hidden"
          }
        }}
      >
        <DialogTitle
          sx={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            py: 1.2,
            px: 2,
            borderBottom: "1px solid rgba(108, 156, 212, 0.16)"
          }}
        >
          <Typography variant="h6">Settings</Typography>
          <IconButton size="small" onClick={closeSettingsModal} aria-label="Close settings">
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent sx={{ p: 0, height: "100%", overflow: "hidden" }}>
          <NativeWorkspace view="settings" autoRefresh={autoRefresh} showAdvanced={showAdvanced} />
        </DialogContent>
      </Dialog>

      <Popover
        open={notifListOpen}
        anchorEl={notifAnchorEl}
        onClose={() => setNotifAnchorEl(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
        slotProps={{
          paper: {
            sx: {
              width: 420,
              maxWidth: "calc(100vw - 24px)",
              borderRadius: 2.5,
              overflow: "hidden",
              border: "1px solid rgba(108, 156, 212, 0.12)",
              background: "rgba(9, 21, 39, 0.85)",
              boxShadow: "0 16px 48px rgba(0, 0, 0, 0.5)",
              backdropFilter: "blur(24px)",
              WebkitBackdropFilter: "blur(24px)"
            }
          }
        }}
      >
        <Box sx={{ px: 1.5, pt: 1.25, pb: 1, borderBottom: "1px solid rgba(108, 156, 212, 0.08)" }}>
          <Stack direction="row" justifyContent="space-between" alignItems="center">
            <Typography variant="subtitle1" fontWeight={600} sx={{ color: "rgba(195, 221, 252, 0.9)" }}>Notifications</Typography>
            <Button
              size="small"
              onClick={() => markAllMutation.mutate()}
              disabled={markAllMutation.isPending || notifications.length === 0}
              sx={{
                textTransform: "none",
                fontSize: "0.75rem",
                color: "rgba(195, 221, 252, 0.5)",
                "&:hover": {
                  color: "rgba(195, 221, 252, 0.8)",
                  background: "rgba(108, 156, 212, 0.08)"
                }
              }}
            >
              Mark all read
            </Button>
          </Stack>
          <Stack direction="row" spacing={0.75} sx={{ mt: 0.75, flexWrap: "wrap" }} useFlexGap>
            <Button
              size="small"
              variant={notificationsMuted ? "contained" : "outlined"}
              disabled={notificationControlMutation.isPending}
              onClick={() => notificationControlMutation.mutate("stop notifications")}
            >
              Snooze 24h
            </Button>
            <Button
              size="small"
              variant={!notificationsMuted ? "contained" : "outlined"}
              disabled={notificationControlMutation.isPending}
              onClick={() => notificationControlMutation.mutate("resume notifications")}
            >
              Resume
            </Button>
          </Stack>
          {notificationControlNotice ? (
            <Typography variant="caption" sx={{ display: "block", mt: 0.6, color: "rgba(195, 221, 252, 0.7)" }}>
              {notificationControlNotice}
            </Typography>
          ) : notificationsMuted ? (
            <Typography variant="caption" sx={{ display: "block", mt: 0.6, color: "rgba(195, 221, 252, 0.7)" }}>
              Notifications paused until {new Date(notificationsMuteUntilMs).toLocaleString()}
            </Typography>
          ) : null}
          <Stack direction="row" spacing={0.75} sx={{ mt: 0.75, flexWrap: "wrap" }} useFlexGap>
            <Button
              size="small"
              variant={notifFilter === "all" ? "contained" : "outlined"}
              onClick={() => setNotifFilter("all")}
            >
              All
            </Button>
            <Button
              size="small"
              variant={notifFilter === "unread" ? "contained" : "outlined"}
              onClick={() => setNotifFilter("unread")}
            >
              Unread
            </Button>
            <Button
              size="small"
              variant={notifFilter === "errors" ? "contained" : "outlined"}
              onClick={() => setNotifFilter("errors")}
            >
              Errors Only
            </Button>
            <Button
              size="small"
              variant={notifFilter === "automation_failures" ? "contained" : "outlined"}
              onClick={() => setNotifFilter("automation_failures")}
            >
              Automation Failures
            </Button>
          </Stack>
        </Box>
        <Box sx={{ maxHeight: 520, overflow: "auto", p: 1.25 }}>
          {notificationsQ.error ? <Alert severity="error">Failed to load notifications</Alert> : null}
          {filteredNotifications.length === 0 ? (
            <Box sx={{ p: 1.25 }}>
              <Typography variant="body2" color="text.secondary">
                {notifications.length === 0 ? "No notifications yet." : "No notifications match this filter."}
              </Typography>
            </Box>
          ) : (
            <List dense disablePadding sx={{ display: "flex", flexDirection: "column", gap: 0.25 }}>
              {filteredNotifications.slice(0, 40).map((n) => (
                <ListItemButton
                  key={n.id}
                  sx={{
                    alignItems: "flex-start",
                    position: "relative",
                    overflow: "hidden",
                    borderRadius: 1.5,
                    px: 1.25,
                    py: 0.85,
                    border: "none",
                    background: "transparent",
                    transition: "background 140ms ease",
                    "&:hover": {
                      background: "rgba(108, 156, 212, 0.08)"
                    },
                    "&:not(:last-child)": {
                      borderBottom: "1px solid rgba(108, 156, 212, 0.08)"
                    }
                  }}
                  onClick={async () => {
                    openNotification(n.id);
                    setNotifAnchorEl(null);
                    if (!n.read) {
                      markReadMutation.mutate(n.id);
                    }
                  }}
                >
                  {!n.read ? (
                    <Box sx={{
                      width: 6,
                      height: 6,
                      borderRadius: "50%",
                      background: "rgba(47, 212, 255, 0.9)",
                      boxShadow: "0 0 6px rgba(47, 212, 255, 0.5)",
                      flexShrink: 0,
                      mt: 0.8,
                      mr: 1
                    }} />
                  ) : (
                    <Box sx={{ width: 6, flexShrink: 0, mr: 1 }} />
                  )}
                  <ListItemText
                    sx={{ my: 0, minWidth: 0 }}
                    primary={
                      <Stack direction="row" justifyContent="space-between" spacing={2} sx={{ minWidth: 0 }}>
                        <Typography
                          variant="body2"
                          fontWeight={n.read ? 400 : 600}
                          noWrap
                          sx={{ minWidth: 0, flex: 1, color: n.read ? "rgba(195, 221, 252, 0.6)" : "rgba(195, 221, 252, 0.95)" }}
                        >
                          {n.title || "Notification"}
                        </Typography>
                        <Typography variant="caption" noWrap sx={{ flexShrink: 0, color: "rgba(195, 221, 252, 0.35)" }}>
                          {n.created_at?.slice(0, 19) || ""}
                        </Typography>
                      </Stack>
                    }
                    secondary={
                      <Typography
                        variant="caption"
                        sx={{
                          display: "-webkit-box",
                          WebkitBoxOrient: "vertical",
                          WebkitLineClamp: 2,
                          overflow: "hidden",
                          wordBreak: "break-word",
                          color: n.read ? "rgba(195, 221, 252, 0.35)" : "rgba(195, 221, 252, 0.55)"
                        }}
                      >
                        {n.body}
                      </Typography>
                    }
                  />
                </ListItemButton>
              ))}
            </List>
          )}
        </Box>
      </Popover>

      <Drawer
        anchor="right"
        open={!!selectedNotification}
        onClose={closeNotification}
        PaperProps={{
          sx: {
            width: 520,
            maxWidth: "calc(100vw - 24px)",
            borderLeft: "1px solid rgba(108, 156, 212, 0.18)",
            background: "linear-gradient(160deg, rgba(9, 21, 39, 0.96), rgba(9, 21, 39, 0.78))"
          }
        }}
      >
        <Box sx={{ p: 2, height: "100%", display: "flex", flexDirection: "column", gap: 1.25 }}>
          <Stack direction="row" spacing={1} alignItems="center" justifyContent="space-between">
            <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
              <NotificationsActiveRoundedIcon color="warning" />
              <Typography variant="h6" noWrap>
                {selectedNotification?.title || "Notification detail"}
              </Typography>
            </Stack>
            {!selectedNotification?.read ? (
              <Button
                size="small"
                onClick={() => selectedNotification?.id && markReadMutation.mutate(selectedNotification.id)}
                disabled={markReadMutation.isPending}
              >
                Mark read
              </Button>
            ) : null}
          </Stack>
          <Typography variant="caption" color="text.secondary">
            {selectedNotification?.created_at?.slice(0, 19) || ""}
          </Typography>
          <Divider />
          <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }}>
            {selectedNotification ? (
              <Stack spacing={1}>
                <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                  {selectedNotification.body}
                </Typography>
                {selectedNotification.metadata ? (
                  <>
                    <Typography variant="subtitle2">Metadata</Typography>
                    <Box className="metadata-box">
                      {(() => {
                        const meta = selectedNotification.metadata as any;
                        const entries: Array<[string, unknown]> =
                          meta && typeof meta === "object" && !Array.isArray(meta) ? Object.entries(meta) : [];
                        const shown = entries.slice(0, 14);
                        return (
                          <Stack spacing={0.65}>
                            {shown.length === 0 ? (
                              <Typography variant="body2" color="text.secondary">
                                (No top-level metadata fields)
                              </Typography>
                            ) : (
                              shown.map(([k, v]) => {
                                const out = formatMetaValue(v);
                                return (
                                  <Stack key={k} direction="row" spacing={1} alignItems="baseline">
                                    <Typography
                                      variant="caption"
                                      color="text.secondary"
                                      sx={{ width: 140, flex: "0 0 auto" }}
                                    >
                                      {k}
                                    </Typography>
                                    {out.href ? (
                                      <Typography
                                        variant="body2"
                                        sx={{ wordBreak: "break-all", flex: "1 1 auto" }}
                                      >
                                        <a
                                          href={out.href}
                                          target="_blank"
                                          rel="noreferrer"
                                          style={{ color: "inherit" }}
                                        >
                                          {out.text}
                                        </a>
                                      </Typography>
                                    ) : (
                                      <Typography variant="body2" sx={{ wordBreak: "break-word", flex: "1 1 auto" }}>
                                        {out.text}
                                      </Typography>
                                    )}
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
                        );
                      })()}
                    </Box>
                  </>
                ) : null}
              </Stack>
            ) : null}
          </Box>
        </Box>
      </Drawer>
      <GuidedTour navigateToView={navigateToView as (view: string, replace?: boolean) => void} currentView={view} />
    </Box>
  );
}
