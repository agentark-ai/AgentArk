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
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import DescriptionRoundedIcon from "@mui/icons-material/DescriptionRounded";
import ExtensionRoundedIcon from "@mui/icons-material/ExtensionRounded";
import AppsRoundedIcon from "@mui/icons-material/AppsRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import FlagRoundedIcon from "@mui/icons-material/FlagRounded";
import TaskRoundedIcon from "@mui/icons-material/TaskRounded";
import VisibilityRoundedIcon from "@mui/icons-material/VisibilityRounded";
import TimelineRoundedIcon from "@mui/icons-material/TimelineRounded";
import AutoStoriesRoundedIcon from "@mui/icons-material/AutoStoriesRounded";
import AnalyticsRoundedIcon from "@mui/icons-material/AnalyticsRounded";
import MonitorHeartRoundedIcon from "@mui/icons-material/MonitorHeartRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import NotificationsNoneRoundedIcon from "@mui/icons-material/NotificationsNoneRounded";
import SpaceDashboardRoundedIcon from "@mui/icons-material/SpaceDashboardRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "./api/client";
import { GuidedTour } from "./components/GuidedTour";
import { NativeWorkspace, type WorkspaceView } from "./components/NativeWorkspace";
import { OverviewPane } from "./components/OverviewPane";
import { ApprovalPromptOverlay } from "./components/ApprovalPromptOverlay";
import { LibraryPane } from "./components/LibraryPane";
import { useUiStore } from "./store/uiStore";
import type { Task } from "./types";

const REFRESH_MS = 8000;
const PING_STALE_MS = 30_000;
const APPROVAL_FALLBACK_POLL_MS = 2500;
type ViewKey =
  | "overview"
  | "chat"
  | "library"
  | "connections"
  | "channels"
  | "routing"
  | "devices"
  | "browser"
  | "gatewayops"
  | "failover"
  | "skills"
  | "tasks"
  | "sessions"
  | "apps"
  | "moltbook"
  | "arkpulse"
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
type NotificationStreamPayload = {
  kind?: string;
  source?: string;
  title?: string;
};

const VIEW_ALIASES: Record<string, ViewKey> = {
  home: "overview",
  overview: "overview",
  workspace: "chat",
  chat: "chat",
  inbox: "overview",
  project: "projects",
  projects: "projects",
  library: "library",
  connections: "connections",
  channels: "channels",
  routing: "routing",
  devices: "devices",
  browser: "browser",
  gatewayops: "arkpulse",
  failover: "settings",
  watchers: "status",
  watcher: "status",
  sessions: "sessions",
  session: "sessions",
  status: "status",
  memory: "settings",
  integrations: "settings",
  settings: "settings",
};

const VIEW_KEYS: ReadonlySet<ViewKey> = new Set<ViewKey>([
  "overview",
  "chat",
  "library",
  "connections",
  "channels",
  "routing",
  "devices",
  "browser",
  "gatewayops",
  "failover",
  "skills",
  "tasks",
  "sessions",
  "apps",
  "moltbook",
  "arkpulse",
  "memory",
  "goals",
  "autonomy",
  "trace",
  "status",
  "swarm",
  "projects",
  "documents",
  "analytics",
  "settings",
]);

const NAV_GROUPS: NavGroup[] = [
  {
    id: "core",
    label: "Core",
    items: [
      { key: "overview", label: "Mission Control", icon: <SpaceDashboardRoundedIcon fontSize="small" /> },
      { key: "chat", label: "Chat", icon: <ChatRoundedIcon fontSize="small" /> },
    ]
  },
  {
    id: "agent",
    label: "Agent",
    items: [
      { key: "skills", label: "Skills", icon: <ExtensionRoundedIcon fontSize="small" /> },
      { key: "apps", label: "Apps", icon: <AppsRoundedIcon fontSize="small" /> },
      { key: "swarm", label: "Agents", icon: <HubRoundedIcon fontSize="small" /> },
      { key: "goals", label: "Goals", icon: <FlagRoundedIcon fontSize="small" /> },
      { key: "moltbook", label: "Moltbook", icon: <AutoStoriesRoundedIcon fontSize="small" /> },
    ]
  },
  {
    id: "operations",
    label: "Operations",
    items: [
      { key: "tasks", label: "Tasks", icon: <TaskRoundedIcon fontSize="small" /> },
      { key: "sessions", label: "Sessions", icon: <HubRoundedIcon fontSize="small" /> },
      { key: "status", label: "Watchers", icon: <VisibilityRoundedIcon fontSize="small" /> },
      { key: "arkpulse", label: "ArkPulse", icon: <MonitorHeartRoundedIcon fontSize="small" /> },
      { key: "trace", label: "Trace", icon: <TimelineRoundedIcon fontSize="small" /> },
    ]
  },
  {
    id: "data",
    label: "Data",
    items: [
      { key: "documents", label: "Documents", icon: <DescriptionRoundedIcon fontSize="small" /> },
      { key: "analytics", label: "Analytics", icon: <AnalyticsRoundedIcon fontSize="small" /> },
    ]
  },
];

const VIEW_PATH_SEGMENTS: Record<ViewKey, string> = {
  overview: "home",
  chat: "chat",
  library: "library",
  connections: "connections",
  channels: "channels",
  routing: "routing",
  devices: "devices",
  browser: "browser",
  gatewayops: "gateway-ops",
  failover: "failover",
  skills: "skills",
  tasks: "tasks",
  sessions: "sessions",
  apps: "apps",
  moltbook: "moltbook",
  arkpulse: "arkpulse",
  memory: "memory",
  goals: "goals",
  autonomy: "autonomy",
  trace: "trace",
  status: "watchers",
  swarm: "swarm",
  projects: "projects",
  documents: "documents",
  analytics: "analytics",
  settings: "settings"
};

const PATH_SEGMENT_TO_VIEW: Record<string, ViewKey> = (() => {
  const base = Object.entries(VIEW_PATH_SEGMENTS).reduce((acc, [view, segment]) => {
    acc[segment] = view as ViewKey;
    return acc;
  }, {} as Record<string, ViewKey>);
  base.overview = "overview";
  base.chat = "chat";
  base.workspace = "chat";
  base.connections = "connections";
  return base;
})();

function isNavItemActive(itemKey: ViewKey, activeView: ViewKey): boolean {
  return activeView === itemKey;
}

function viewPath(view: ViewKey): string {
  return `/ui/${VIEW_PATH_SEGMENTS[view]}`;
}

function normalizeViewKey(rawView: string): ViewKey {
  const normalized = rawView.trim().toLowerCase();
  if (VIEW_KEYS.has(normalized as ViewKey)) {
    return normalized as ViewKey;
  }
  return VIEW_ALIASES[normalized] || "chat";
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
      if (segment === "inbox") return { view: "overview", matched: true };
      if (segment === "actions") return { view: "skills", matched: true };
      if (segment === "integrations") return { view: "settings", matched: true };
      if (segment === "memory") return { view: "settings", matched: true };
      if (segment === "gateway-ops" || segment === "gatewayops") return { view: "arkpulse", matched: true };
      if (segment === "failover") return { view: "settings", matched: true };
      if (segment === "status") return { view: "status", matched: true };
      const view = PATH_SEGMENT_TO_VIEW[segment];
      if (view) {
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

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function pickTasks(value: unknown): Task[] {
  if (Array.isArray(value)) return value as Task[];
  const record = asRecord(value);
  return Array.isArray(record.tasks) ? (record.tasks as Task[]) : [];
}

function notifTimeAgo(raw?: string | null): { label: string; tip: string } {
  if (!raw) return { label: "", tip: "" };
  const dt = new Date(raw);
  if (Number.isNaN(dt.getTime())) return { label: raw, tip: raw };
  const tip = new Intl.DateTimeFormat(undefined, {
    month: "short", day: "2-digit", year: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit", timeZoneName: "short",
  }).format(dt);
  const diffMs = Date.now() - dt.getTime();
  const isPast = diffMs >= 0;
  const absSec = Math.round(Math.abs(diffMs) / 1000);
  if (absSec < 30) return { label: "just now", tip };
  const absMin = Math.round(absSec / 60);
  if (absMin < 60) { const s = absMin === 1 ? "1 min" : `${absMin} mins`; return { label: isPast ? `${s} ago` : `in ${s}`, tip }; }
  const absHours = Math.round(absMin / 60);
  if (absHours < 24) { const s = absHours === 1 ? "1 hour" : `${absHours} hours`; return { label: isPast ? `${s} ago` : `in ${s}`, tip }; }
  const absDays = Math.round(absHours / 24);
  if (absDays < 7) { const s = absDays === 1 ? "1 day" : `${absDays} days`; return { label: isPast ? `${s} ago` : `in ${s}`, tip }; }
  const absWeeks = Math.round(absDays / 7);
  if (absWeeks < 5) { const s = absWeeks === 1 ? "1 week" : `${absWeeks} weeks`; return { label: isPast ? `${s} ago` : `in ${s}`, tip }; }
  return { label: tip, tip };
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

function shouldSurfaceNotification(notification: {
  title?: string;
  body?: string;
  source?: string;
  level?: string;
}): boolean {
  const source = (notification.source || "").toLowerCase();
  const title = (notification.title || "").toLowerCase();
  if (source.includes("watcher") || title.includes("watcher triggered")) return false;
  if (source.includes("arkpulse")) return false;
  return true;
}

function notificationEventAffectsApprovals(payload: NotificationStreamPayload): boolean {
  const kind = (payload.kind || "").toLowerCase();
  const source = (payload.source || "").toLowerCase();
  const title = (payload.title || "").toLowerCase();
  return (
    kind.includes("resync") ||
    source.includes("approval") ||
    source.includes("task") ||
    title.includes("approval")
  );
}

export default function App() {
  const queryClient = useQueryClient();
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const selectedNotificationId = useUiStore((s) => s.selectedNotificationId);
  const openNotification = useUiStore((s) => s.openNotification);
  const closeNotification = useUiStore((s) => s.closeNotification);
  const [view, setViewState] = useState<ViewKey>(() => resolveViewFromPath(window.location.pathname).view);
  const [lastNonSettingsView, setLastNonSettingsView] = useState<ViewKey>("overview");
  const [settingsInitialTab, setSettingsInitialTab] = useState<number | null>(null);
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

  const [notifAnchorEl, setNotifAnchorEl] = useState<HTMLElement | null>(null);
  const notifListOpen = Boolean(notifAnchorEl);
  const [notifFilter, setNotifFilter] = useState<"all" | "unread" | "errors" | "automation_failures">("all");
  const [notificationsStreamConnected, setNotificationsStreamConnected] = useState(false);
  const [approvalBusyTaskId, setApprovalBusyTaskId] = useState<string | null>(null);
  const [approvalPopupError, setApprovalPopupError] = useState<string | null>(null);
  const navigateToView = (nextViewRaw: ViewKey | string, replace = false) => {
    const nextView = normalizeViewKey(nextViewRaw);
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
  const approvalTasksQ = useQuery({
    queryKey: ["approval-popup-tasks"],
    queryFn: () => api.rawGet("/tasks?limit=200"),
    refetchInterval: notificationsStreamConnected ? false : APPROVAL_FALLBACK_POLL_MS,
    refetchIntervalInBackground: !notificationsStreamConnected
  });

  const notificationsQ = useQuery({
    queryKey: ["notifications"],
    queryFn: api.getNotifications,
    refetchInterval: autoRefresh && !notificationsStreamConnected ? REFRESH_MS : false
  });
  const notificationsCountQ = useQuery({
    queryKey: ["notifications-count"],
    queryFn: () => api.rawGet("/notifications/count"),
    refetchInterval: autoRefresh && !notificationsStreamConnected ? REFRESH_MS : false
  });
  const notifications = Array.isArray(notificationsQ.data) ? notificationsQ.data : [];
  const visibleNotifications = useMemo(
    () => notifications.filter((n) => shouldSurfaceNotification(n)),
    [notifications]
  );
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
  const visibleUnreadCount = visibleNotifications.filter((n) => !n.read).length;
  const unreadCount = Number.isFinite(unreadCountFromEndpoint)
    ? Math.max(0, Math.min(Math.round(unreadCountFromEndpoint), visibleUnreadCount))
    : visibleUnreadCount;
  const filteredNotifications = useMemo(() => {
    if (notifFilter === "all") return visibleNotifications;
    if (notifFilter === "unread") return visibleNotifications.filter((n) => !n.read);
    if (notifFilter === "errors") {
      return visibleNotifications.filter((n) => {
        const level = (n.level || "").toLowerCase();
        return level === "error" || level === "critical";
      });
    }
    return visibleNotifications.filter((n) => isAutomationFailureNotification(n));
  }, [visibleNotifications, notifFilter]);
  const approvalTasks = useMemo(() => pickTasks(approvalTasksQ.data), [approvalTasksQ.data]);

  useEffect(() => {
    if (typeof window === "undefined") return undefined;

    const invalidateNotificationViews = (includeApprovalTasks: boolean) => {
      void queryClient.invalidateQueries({ queryKey: ["notifications"] });
      void queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      void queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
      if (includeApprovalTasks) {
        void queryClient.invalidateQueries({ queryKey: ["approval-popup-tasks"] });
        void queryClient.invalidateQueries({ queryKey: ["tasks"] });
        void queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      }
    };

    const stream = new EventSource("/notifications/stream", { withCredentials: true });

    const handleConnected = () => {
      setNotificationsStreamConnected(true);
      invalidateNotificationViews(true);
    };

    const handleNotification = (event: Event) => {
      setNotificationsStreamConnected(true);
      let payload: NotificationStreamPayload = {};
      const raw = (event as MessageEvent<string>).data;
      if (typeof raw === "string" && raw.trim()) {
        try {
          payload = JSON.parse(raw) as NotificationStreamPayload;
        } catch {
          payload = {};
        }
      }
      invalidateNotificationViews(notificationEventAffectsApprovals(payload));
    };

    const handleResync = () => {
      setNotificationsStreamConnected(true);
      invalidateNotificationViews(true);
    };

    const handleClosed = () => {
      setNotificationsStreamConnected(false);
    };

    stream.onopen = handleConnected;
    stream.onerror = () => {
      setNotificationsStreamConnected(false);
    };
    stream.addEventListener("connected", handleConnected);
    stream.addEventListener("notification", handleNotification);
    stream.addEventListener("resync", handleResync);
    stream.addEventListener("closed", handleClosed);

    return () => {
      stream.close();
      stream.removeEventListener("connected", handleConnected);
      stream.removeEventListener("notification", handleNotification);
      stream.removeEventListener("resync", handleResync);
      stream.removeEventListener("closed", handleClosed);
    };
  }, [queryClient]);

  let selectedNotification: (typeof visibleNotifications)[number] | null = null;
  for (const n of visibleNotifications) {
    if (n.id === selectedNotificationId) {
      selectedNotification = n;
      break;
    }
  }

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

  const approvalDecisionMutation = useMutation({
    mutationFn: async (payload: { id: string; decision: "approve" | "reject" }) => {
      if (payload.decision === "approve") return api.approveTask(payload.id);
      return api.rejectTask(payload.id);
    },
    onSuccess: async () => {
      setApprovalPopupError(null);
      await queryClient.invalidateQueries({ queryKey: ["approval-popup-tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-unread-notifications"] });
    },
    onError: (error) => {
      setApprovalPopupError(error instanceof Error ? error.message : "Failed to update approval.");
    },
    onSettled: () => {
      setApprovalBusyTaskId(null);
    }
  });

  const handleApprovalDecision = (id: string, decision: "approve" | "reject") => {
    setApprovalPopupError(null);
    setApprovalBusyTaskId(id);
    approvalDecisionMutation.mutate({ id, decision });
  };

  const openSettingsView = (route: "settings" | "arkpulse", initialTab: number | null = null) => {
    setSettingsInitialTab(initialTab);
    navigateToView(route);
  };

  const openGuidedTourStep = (
    targetView: string,
    options?: { settingsInitialTab?: number }
  ) => {
    if (targetView === "settings") {
      openSettingsView("settings", typeof options?.settingsInitialTab === "number" ? options.settingsInitialTab : null);
      return;
    }
    navigateToView(targetView);
  };

  const closeSettingsModal = () => {
    setSettingsInitialTab(null);
    const fallback = lastNonSettingsView === "settings" ? "overview" : lastNonSettingsView;
    navigateToView(fallback, true);
  };

  const settingsModalOpen = view === "settings";
  const activeView: ViewKey = settingsModalOpen ? lastNonSettingsView : view;
  const workspaceView = activeView as Exclude<ViewKey, "overview" | "settings" | "library">;
  const stageClassName = [
    "workspace-stage",
    activeView === "overview"
      ? "workspace-stage-overview"
      : activeView === "chat"
        ? "workspace-stage-chat"
        : "",
  ]
    .filter(Boolean)
    .join(" ");
  const mainPaneClassName = `main-pane main-pane-${activeView}`;

  return (
    <Box className="agi-shell">
      <Box className="bg-orb orb-a" />
      <Box className="bg-orb orb-b" />
      <Box className="app-frame">
        <AppBar position="static" elevation={0} color="transparent" className="glass-appbar shell-appbar">
          <Toolbar
            className="shell-toolbar"
            sx={{ minHeight: "var(--appbar-height)", px: { xs: 1.25, md: 1.5 } }}
          >
            <Stack direction="row" alignItems="center" spacing={1} sx={{ flexGrow: 1, minWidth: 0 }}>
              <Box className="shell-brand-mark">
                <img src="/logo.svg" alt="AgentArk" width={36} height={36} />
              </Box>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="caption" className="shell-kicker">
                  AgentArk
                </Typography>
                <Typography variant="subtitle1" className="shell-title" noWrap>
                  Operator Console
                </Typography>
              </Box>
              <Tooltip title={serverTooltip} arrow>
                <Box
                  onClick={() => serverQ.refetch()}
                  sx={{
                    width: 10,
                    height: 10,
                    borderRadius: "50%",
                    backgroundColor: serverDotColor,
                    cursor: "pointer",
                    ml: 0.75,
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
            <Stack direction="row" spacing={0.5} alignItems="center" className="shell-actions">
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
              <Tooltip title="Settings">
                <IconButton color="primary" onClick={() => openSettingsView("settings")}>
                  <SettingsRoundedIcon />
                </IconButton>
              </Tooltip>
            </Stack>
          </Toolbar>
        </AppBar>

        <Box className="main-grid">
          <Box className="side-nav">
            <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ px: 0.5, mb: 1 }}>
              <Typography variant="caption" className="nav-label">
                Navigate
              </Typography>
            </Stack>
            <List dense>
              {NAV_GROUPS.map((group, groupIdx) => (
                <Box key={group.id} className="nav-group">
                  <Stack direction="row" alignItems="center" justifyContent="space-between">
                    <Typography variant="overline" className="nav-group-label">
                      {group.label}
                    </Typography>
                  </Stack>
                  {group.items.map((item) => (
                    <Tooltip
                      key={item.key}
                      title={item.label}
                      placement="right"
                      disableHoverListener
                    >
                      <ListItemButton
                        selected={isNavItemActive(item.key, activeView)}
                        onClick={() => navigateToView(item.key)}
                        className="nav-item"
                        data-tour-target={`nav-${item.key}`}
                      >
                        <ListItemIcon className="nav-item-icon">{item.icon}</ListItemIcon>
                        <ListItemText
                          className="nav-item-text"
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

            <Box className={mainPaneClassName}>
              <Box className={stageClassName}>
                {activeView === "overview" ? (
                  <OverviewPane
                    navigateToView={navigateToView as (view: string, replace?: boolean) => void}
                    serverStatus={serverQ.data}
                    serverError={serverQ.isError}
                    serverLoading={serverQ.isLoading && !serverQ.data}
                  />
                ) : activeView === "chat" ? (
                  <NativeWorkspace
                    view="chat"
                    autoRefresh={settingsModalOpen ? false : autoRefresh}
                    showAdvanced={showAdvanced}
                    onNavigateToView={navigateToView as (view: string, replace?: boolean) => void}
                  />
                ) : activeView === "library" ? (
                  <LibraryPane
                    autoRefresh={settingsModalOpen ? false : autoRefresh}
                    showAdvanced={showAdvanced}
                    onNavigateToView={navigateToView as (view: string, replace?: boolean) => void}
                  />
                ) : (
                  <NativeWorkspace
                    view={workspaceView as WorkspaceView}
                    autoRefresh={settingsModalOpen ? false : autoRefresh}
                    showAdvanced={showAdvanced}
                    onNavigateToView={navigateToView as (view: string, replace?: boolean) => void}
                  />
              )}
            </Box>
          </Box>
        </Box>
      </Box>

      <ApprovalPromptOverlay
        tasks={approvalTasks}
        busyTaskId={approvalBusyTaskId}
        errorMessage={approvalPopupError}
        onApprove={(id) => handleApprovalDecision(id, "approve")}
        onReject={(id) => handleApprovalDecision(id, "reject")}
        onOpenTasks={() => navigateToView("tasks")}
      />

      <Dialog
        open={settingsModalOpen}
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
          <NativeWorkspace
            view="settings"
            autoRefresh={false}
            showAdvanced={showAdvanced}
            settingsInitialTab={settingsInitialTab}
            onNavigateToView={navigateToView as (view: string, replace?: boolean) => void}
          />
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
              disabled={markAllMutation.isPending || visibleNotifications.length === 0}
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
                {visibleNotifications.length === 0 ? "No notifications yet." : "No notifications match this filter."}
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
                        <Typography variant="caption" noWrap sx={{ flexShrink: 0, color: "rgba(195, 221, 252, 0.35)" }} title={notifTimeAgo(n.created_at).tip}>
                          {notifTimeAgo(n.created_at).label}
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
          <Typography variant="caption" color="text.secondary" title={notifTimeAgo(selectedNotification?.created_at).tip}>
            {notifTimeAgo(selectedNotification?.created_at).label}
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
      <GuidedTour openTourStep={openGuidedTourStep} currentView={view} />
    </Box>
  );
}
