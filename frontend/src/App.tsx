import {
  Alert,
  AppBar,
  Badge,
  Box,
  Chip,
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
  Typography,
} from "@mui/material";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import DescriptionRoundedIcon from "@mui/icons-material/DescriptionRounded";
import ExtensionRoundedIcon from "@mui/icons-material/ExtensionRounded";
import AppsRoundedIcon from "@mui/icons-material/AppsRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import HistoryRoundedIcon from "@mui/icons-material/HistoryRounded";
import FlagRoundedIcon from "@mui/icons-material/FlagRounded";
import TaskRoundedIcon from "@mui/icons-material/TaskRounded";
import VisibilityRoundedIcon from "@mui/icons-material/VisibilityRounded";
import TimelineRoundedIcon from "@mui/icons-material/TimelineRounded";
import AutoGraphRoundedIcon from "@mui/icons-material/AutoGraphRounded";
import AnalyticsRoundedIcon from "@mui/icons-material/AnalyticsRounded";
import BubbleChartRoundedIcon from "@mui/icons-material/BubbleChartRounded";
import MemoryRoundedIcon from "@mui/icons-material/MemoryRounded";
import MonitorHeartRoundedIcon from "@mui/icons-material/MonitorHeartRounded";
import MenuRoundedIcon from "@mui/icons-material/MenuRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import NotificationsNoneRoundedIcon from "@mui/icons-material/NotificationsNoneRounded";
import SpaceDashboardRoundedIcon from "@mui/icons-material/SpaceDashboardRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { Orbit as OrbitIcon } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import useMediaQuery from "@mui/material/useMediaQuery";
import { api, ApiRequestError } from "./api/client";
import {
  formatUiRelativeDateTimeMeta,
  setUiTimeZoneOverride,
} from "./lib/dateFormat";
import { recordRuntimeMetricSample } from "./lib/runtimeMetricHistory";
import { AmberCascadesBackground } from "./components/AmberCascadesBackground";
import {
  NativeWorkspace,
  preloadSettingsTab,
  preloadWorkspaceSurface,
  type WorkspaceView,
} from "./components/NativeWorkspace";
import SettingsPage from "./components/pages/SettingsPage";
import {
  prefetchSettingsTabData,
} from "./components/pages/settingsData";
import { OverviewPane } from "./components/OverviewPane";
import { LibraryPane } from "./components/LibraryPane";
import { useUiStore } from "./store/uiStore";
import type { ApprovalLogEntry, Task } from "./types";
import { PRODUCT_CATEGORY, PRODUCT_NAME } from "./brand";

const REFRESH_MS = 8000;
const PING_STALE_MS = 30_000;
const APPROVAL_FALLBACK_POLL_MS = 2500;
const AUTO_REFRESH_IDLE_PAUSE_MS = 5 * 60 * 1000;
const NAV_HIDDEN_STORAGE_KEY = "agentark.ui.navHidden";

function memoizeModuleLoader<T>(
  loader: () => Promise<T>,
): () => Promise<T> {
  let pending: Promise<T> | null = null;
  return () => {
    if (!pending) {
      pending = loader().catch((error) => {
        pending = null;
        throw error;
      });
    }
    return pending;
  };
}

function useAutoRefreshWhileActive(enabled: boolean): boolean {
  const [idle, setIdle] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined" || !enabled) {
      setIdle(false);
      return undefined;
    }

    let idleTimer: number | null = null;
    const clearIdleTimer = () => {
      if (idleTimer !== null) {
        window.clearTimeout(idleTimer);
        idleTimer = null;
      }
    };
    const scheduleIdle = () => {
      clearIdleTimer();
      idleTimer = window.setTimeout(() => {
        setIdle(true);
      }, AUTO_REFRESH_IDLE_PAUSE_MS);
    };
    const markActive = () => {
      setIdle(false);
      scheduleIdle();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        clearIdleTimer();
        setIdle(true);
        return;
      }
      markActive();
    };
    const events: Array<keyof WindowEventMap> = [
      "pointerdown",
      "keydown",
      "wheel",
      "touchstart",
    ];

    markActive();
    for (const eventName of events) {
      window.addEventListener(eventName, markActive, { passive: true });
    }
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      clearIdleTimer();
      for (const eventName of events) {
        window.removeEventListener(eventName, markActive);
      }
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [enabled]);

  return enabled && !idle;
}

const loadApprovalPromptOverlayModule = memoizeModuleLoader(() =>
  import("./components/ApprovalPromptOverlay"),
);
const loadGuidedTourModule = memoizeModuleLoader(() =>
  import("./components/GuidedTour"),
);
const loadBrowserHandoffPageModule = memoizeModuleLoader(() =>
  import("./components/BrowserHandoffPage"),
);

const loadApprovalPromptOverlayLazy = memoizeModuleLoader(() =>
  loadApprovalPromptOverlayModule().then((module) => ({
    default: module.ApprovalPromptOverlay,
  })),
);
const loadGuidedTourLazy = memoizeModuleLoader(() =>
  loadGuidedTourModule().then((module) => ({
    default: module.GuidedTour,
  })),
);
const loadBrowserHandoffPageLazy = memoizeModuleLoader(() =>
  loadBrowserHandoffPageModule().then((module) => ({
    default: module.BrowserHandoffPage,
  })),
);
const ApprovalPromptOverlay = lazy(loadApprovalPromptOverlayLazy);
const GuidedTour = lazy(loadGuidedTourLazy);
const BrowserHandoffPage = lazy(loadBrowserHandoffPageLazy);

function scheduleWarmup(task: () => void, delayMs = 900): () => void {
  if (typeof window === "undefined") return () => {};
  const timer = window.setTimeout(task, delayMs);
  return () => window.clearTimeout(timer);
}

function WorkspacePaneFallback() {
  return (
    <Box className="list-shell" sx={{ minHeight: 180, p: 1.5 }}>
      <Typography
        variant="body2"
        sx={{
          color: "text.secondary",
        }}
      >
        Loading workspace...
      </Typography>
    </Box>
  );
}

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
  | "arkpulse"
  | "arkorbit"
  | "arkmemory"
  | "arkreflect"
  | "goals"
  | "autonomy"
  | "evolution"
  | "sentinel"
  | "trace"
  | "status"
  | "swarm"
  | "documents"
  | "analytics"
  | "search"
  | "settings";

type NavItem = {
  key: ViewKey;
  label: string;
  icon: ReactNode;
  tooltip: string;
};
type NavGroup = { id: string; label: string; items: NavItem[] };
type NotificationStreamPayload = {
  kind?: string;
  source?: string;
  title?: string;
  body?: string;
};

function defaultSettingsTabForView(
  view: ViewKey,
  explicitTab?: number | null,
): number | null {
  if (typeof explicitTab === "number") return explicitTab;
  switch (view) {
    case "connections":
    case "channels":
    case "routing":
    case "browser":
      return 20;
    case "devices":
      return 26;
    case "failover":
      return 1;
    case "search":
      return 24;
    default:
      return null;
  }
}

function settingsSearchForTab(tab?: number | null): string {
  let tabName = "";
  switch (tab) {
    case 1:
      tabName = "models";
      break;
    case 5:
      tabName = "advanced";
      break;
    case 6:
      tabName = "observability";
      break;
    case 8:
      tabName = "mcp";
      break;
    case 14:
      tabName = "data-lifecycle";
      break;
    case 16:
      tabName = "security";
      break;
    case 20:
      tabName = "integrations";
      break;
    case 21:
      tabName = "connectors";
      break;
    case 22:
      tabName = "webhooks";
      break;
    case 23:
      tabName = "plugins";
      break;
    case 25:
      tabName = "updates";
      break;
    default:
      break;
  }
  return tabName ? `?settings_tab=${encodeURIComponent(tabName)}` : "";
}

const UNAVAILABLE_APPROVAL_DESCRIPTION = "Older task details unavailable";

const VIEW_ALIASES: Record<string, ViewKey> = {
  home: "overview",
  overview: "overview",
  workspace: "chat",
  chat: "chat",
  inbox: "overview",
  task: "tasks",
  tasks: "tasks",
  session: "sessions",
  sessions: "sessions",
  "browser-session": "sessions",
  "browser-sessions": "sessions",
  app: "apps",
  apps: "apps",
  skill: "skills",
  skills: "skills",
  goal: "goals",
  goals: "goals",
  evolution: "evolution",
  evolutions: "evolution",
  sentinel: "sentinel",
  agent: "swarm",
  agents: "swarm",
  swarm: "swarm",
  document: "documents",
  documents: "documents",
  file: "documents",
  files: "documents",
  library: "library",
  connections: "connections",
  channels: "channels",
  routing: "routing",
  devices: "devices",
  browser: "browser",
  "gateway-ops": "arkpulse",
  gatewayops: "arkpulse",
  failover: "settings",
  watchers: "status",
  watcher: "status",
  "background-work": "status",
  background: "status",
  status: "status",
  integration: "settings",
  integrations: "settings",
  search: "search",
  ambient: "sentinel",
  arkorbit: "arkorbit",
  orbit: "arkorbit",
  orbits: "arkorbit",
  arkmemory: "arkmemory",
  arkreflect: "arkreflect",
  reflect: "arkreflect",
  arkrecall: "arkmemory",
  memory: "arkmemory",
  setting: "settings",
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
  "arkpulse",
  "arkorbit",
  "arkmemory",
  "arkreflect",
  "goals",
  "autonomy",
  "evolution",
  "sentinel",
  "trace",
  "status",
  "swarm",
  "documents",
  "analytics",
  "search",
  "settings",
]);

const NAV_GROUPS: NavGroup[] = [
  {
    id: "core",
    label: "Home",
    items: [
      {
        key: "overview",
        label: "Mission Control",
        icon: <SpaceDashboardRoundedIcon fontSize="small" />,
        tooltip: "Home overview, alerts, and recent activity.",
      },
      {
        key: "chat",
        label: "Chat",
        icon: <ChatRoundedIcon fontSize="small" />,
        tooltip: "Talk with AgentArk and run work.",
      },
      {
        key: "arkorbit",
        label: "Orbit",
        icon: <OrbitIcon size={18} strokeWidth={2.2} aria-hidden="true" />,
        tooltip: "Your canvases, projects, and working spaces.",
      },
    ],
  },
  {
    id: "agent",
    label: "Agent",
    items: [
      {
        key: "skills",
        label: "Skills",
        icon: <ExtensionRoundedIcon fontSize="small" />,
        tooltip: "Manage agent abilities and tools.",
      },
      {
        key: "apps",
        label: "Apps",
        icon: <AppsRoundedIcon fontSize="small" />,
        tooltip: "Open and manage generated apps.",
      },
      {
        key: "swarm",
        label: "Agents",
        icon: <HubRoundedIcon fontSize="small" />,
        tooltip: "View and coordinate available agents.",
      },
      {
        key: "goals",
        label: "Goals",
        icon: <FlagRoundedIcon fontSize="small" />,
        tooltip: "Track objectives and progress.",
      },
    ],
  },
  {
    id: "ark_core",
    label: "Ark Core",
    items: [
      {
        key: "sentinel",
        label: "Sentinel",
        icon: <NotificationsActiveRoundedIcon fontSize="small" />,
        tooltip: "Review automation checks and suggestions.",
      },
      {
        key: "evolution",
        label: "Evolve",
        icon: <AutoGraphRoundedIcon fontSize="small" />,
        tooltip: "Inspect experiments that improve AgentArk.",
      },
      {
        key: "arkmemory",
        label: "Memory",
        icon: <MemoryRoundedIcon fontSize="small" />,
        tooltip: "Review stored knowledge and preferences.",
      },
      {
        key: "arkreflect",
        label: "Reflect",
        icon: <BubbleChartRoundedIcon fontSize="small" />,
        tooltip: "Explore patterns and self-review insights.",
      },
      {
        key: "arkpulse",
        label: "Pulse",
        icon: <MonitorHeartRoundedIcon fontSize="small" />,
        tooltip: "Monitor system health and cleanup findings.",
      },
    ],
  },
  {
    id: "operations",
    label: "Operations",
    items: [
      {
        key: "tasks",
        label: "Tasks",
        icon: <TaskRoundedIcon fontSize="small" />,
        tooltip: "See scheduled, running, and completed tasks.",
      },
      {
        key: "browser",
        label: "Browser",
        icon: <HistoryRoundedIcon fontSize="small" />,
        tooltip: "Manage browser profiles and live handoff sessions.",
      },
      {
        key: "status",
        label: "Background Work",
        icon: <VisibilityRoundedIcon fontSize="small" />,
        tooltip: "Follow ongoing background work.",
      },
      {
        key: "trace",
        label: "Trace",
        icon: <TimelineRoundedIcon fontSize="small" />,
        tooltip: "Inspect execution steps and diagnostics.",
      },
    ],
  },
  {
    id: "data",
    label: "Data",
    items: [
      {
        key: "documents",
        label: "Documents",
        icon: <DescriptionRoundedIcon fontSize="small" />,
        tooltip: "Browse files and document context.",
      },
      {
        key: "analytics",
        label: "Analytics",
        icon: <AnalyticsRoundedIcon fontSize="small" />,
        tooltip: "View usage and performance metrics.",
      },
    ],
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
  arkpulse: "arkpulse",
  arkorbit: "arkorbit",
  arkmemory: "arkmemory",
  arkreflect: "arkreflect",
  goals: "goals",
  autonomy: "autonomy",
  evolution: "evolution",
  sentinel: "sentinel",
  trace: "trace",
  status: "background-work",
  swarm: "swarm",
  documents: "documents",
  analytics: "analytics",
  search: "search",
  settings: "settings",
};

const PATH_SEGMENT_TO_VIEW: Record<string, ViewKey> = (() => {
  const base = Object.entries(VIEW_PATH_SEGMENTS).reduce(
    (acc, [view, segment]) => {
      acc[segment] = view as ViewKey;
      return acc;
    },
    {} as Record<string, ViewKey>,
  );
  base.overview = "overview";
  base.chat = "chat";
  base.workspace = "chat";
  base.connections = "connections";
  base.arkrecall = "arkmemory";
  return base;
})();

function isNavItemActive(itemKey: ViewKey, activeView: ViewKey): boolean {
  return activeView === itemKey;
}

function viewPath(view: ViewKey): string {
  return `/ui/${VIEW_PATH_SEGMENTS[view]}`;
}

function normalizeViewKey(rawView: string): ViewKey {
  const raw = rawView.trim().toLowerCase();
  if (!raw) return "chat";

  const withoutOrigin = raw.replace(/^https?:\/\/[^/]+/, "");
  const withoutHash = withoutOrigin.startsWith("#")
    ? withoutOrigin.slice(1)
    : withoutOrigin;
  const routeRef = withoutHash.split(/[?#]/, 1)[0]?.replace(/\/+$/, "") || "";
  if (
    routeRef === "/ui" ||
    routeRef === "/ui/v2" ||
    routeRef.startsWith("/ui/") ||
    routeRef.startsWith("ui/")
  ) {
    const resolved = resolveViewFromPath(
      routeRef.startsWith("/") ? routeRef : `/${routeRef}`,
    );
    if (resolved.matched) {
      return resolved.view;
    }
  }

  const normalized =
    withoutHash
      .split(/[?#]/, 1)[0]
      ?.replace(/^\/+/, "")
      .replace(/^ui\/v2\/?/, "")
      .replace(/^ui\/?/, "")
      .replace(/\/+$/, "") || "";
  const alias = VIEW_ALIASES[normalized];
  if (alias) {
    return alias;
  }
  if (VIEW_KEYS.has(normalized as ViewKey)) {
    return normalized as ViewKey;
  }
  return "chat";
}

function resolveViewFromPath(pathname: string): {
  view: ViewKey;
  matched: boolean;
} {
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
    const segment =
      normalized.slice("/ui/".length).split("/")[0]?.toLowerCase() || "";
    if (segment === "inbox") return { view: "overview", matched: true };
    if (segment === "actions") return { view: "skills", matched: true };
    if (segment === "integrations") return { view: "settings", matched: true };
    if (segment === "memory") return { view: "arkmemory", matched: true };
    if (segment === "gateway-ops" || segment === "gatewayops")
      return { view: "arkpulse", matched: true };
    if (segment === "failover") return { view: "settings", matched: true };
    if (segment === "status") return { view: "status", matched: true };
    const view = PATH_SEGMENT_TO_VIEW[segment];
    if (view) {
      return { view, matched: true };
    }
    const alias = VIEW_ALIASES[segment];
    if (alias) {
      return { view: alias, matched: true };
    }
  }

  return { view: "overview", matched: false };
}

function resolveBrowserHandoffPath(pathname: string): string | null {
  const normalized = pathname.replace(/\/+$/, "");
  const uiPrefix = "/ui/browser-handoff/";
  if (normalized.startsWith(uiPrefix)) {
    const sessionId =
      normalized.slice(uiPrefix.length).split("/")[0]?.trim() || "";
    return sessionId || null;
  }
  return null;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

function pickTasks(value: unknown): Task[] {
  if (Array.isArray(value)) return value as Task[];
  const record = asRecord(value);
  return Array.isArray(record.tasks) ? (record.tasks as Task[]) : [];
}

function pickApprovalLogEntries(value: unknown): ApprovalLogEntry[] {
  if (Array.isArray(value)) return value as ApprovalLogEntry[];
  const record = asRecord(value);
  return Array.isArray(record.approvals)
    ? (record.approvals as ApprovalLogEntry[])
    : [];
}

function notifTimeAgo(raw?: string | null): { label: string; tip: string } {
  if (!raw) return { label: "", tip: "" };
  return formatUiRelativeDateTimeMeta(raw, { fallback: raw });
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

function isInputNeededNotification(notification: {
  title?: string;
  body?: string;
  source?: string;
  kind?: string;
}): boolean {
  const title = (notification.title || "").toLowerCase();
  const body = (notification.body || "").toLowerCase();
  const source = (notification.source || "").toLowerCase();
  const kind = (notification.kind || "").toLowerCase();
  const text = `${title} ${body} ${source} ${kind}`;
  return (
    source.includes("workflow_inputs") ||
    kind.includes("input_needed") ||
    kind.includes("input-needed") ||
    text.includes("missing input") ||
    text.includes("required input") ||
    text.includes("input needed")
  );
}

function normalizeTaskStatus(status: unknown): string {
  const compact = String(status || "")
    .toLowerCase()
    .replace(/[^a-z]/g, "");
  if (compact.includes("awaitingapproval")) return "awaiting_approval";
  if (compact.includes("expiredneedsreapproval")) return "expired";
  return compact;
}

function hasRenderableApprovalTask(task: Task): boolean {
  const approvalTask = task as Task & { arguments?: Record<string, unknown> };
  const status = normalizeTaskStatus(approvalTask.status);
  if (status !== "awaiting_approval" && status !== "expired")
    return false;
  const description = String(approvalTask.description || "").trim();
  if (description && description !== UNAVAILABLE_APPROVAL_DESCRIPTION)
    return true;
  const approval = asRecord(asRecord(approvalTask.arguments)._approval);
  return (
    Boolean(String(approval.title || "").trim()) ||
    Boolean(String(approval.summary || "").trim()) ||
    Boolean(String(approval.reason || "").trim()) ||
    Boolean(String(approval.risk_level || "").trim()) ||
    Boolean(String(approval.risk_score || "").trim()) ||
    Boolean(String(approval.source || "").trim())
  );
}

function parseJsonRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "string") return asRecord(value);
  try {
    return asRecord(JSON.parse(value));
  } catch {
    return {};
  }
}

function hasRenderableApprovalLogEntry(entry: ApprovalLogEntry): boolean {
  const status = String(entry.status || "").trim().toLowerCase();
  if (status !== "pending" && status !== "expired") return false;
  const payload = parseJsonRecord(entry.arguments);
  if (Array.isArray(payload.calls) && payload.calls.length > 0) return true;
  return Boolean(
    String(entry.action_name || "").trim() ||
      String(entry.rule_name || "").trim(),
  );
}

function isApprovalPopupDuplicateNotification(
  notification: {
    title?: string;
    body?: string;
    source?: string;
  },
  approvalPopupVisible: boolean,
): boolean {
  const title = (notification.title || "").toLowerCase();
  const body = (notification.body || "").toLowerCase();
  const source = (notification.source || "").toLowerCase();
  if (source.includes("approval") || title.includes("approval needed")) {
    return approvalPopupVisible;
  }
  if (source === "autonomy_attention") {
    const mentionsApprovals =
      body.includes(" approval") || body.includes(" approvals");
    const approvalOnlyAttention =
      mentionsApprovals &&
      (body.includes("0 missing input") || body.includes("0 missing inputs"));
    return approvalOnlyAttention || (approvalPopupVisible && mentionsApprovals);
  }
  return false;
}

function notificationDisplayTitle(notification: {
  title?: string;
  body?: string;
  source?: string;
  kind?: string;
}): string {
  if (isInputNeededNotification(notification)) return "Input needed";
  return notification.title || "Notification";
}

function notificationDisplaySummary(notification: {
  title?: string;
  body?: string;
  source?: string;
  kind?: string;
}): string {
  if (isInputNeededNotification(notification)) {
    return (
      notification.body ||
      "Waiting on you to provide the missing inputs and resume the task."
    );
  }
  return notification.body || notification.source || "Open to view details.";
}

type NotificationActionTarget = {
  view: ViewKey;
  label: string;
  settingsTab?: number | null;
  search?: string;
};

type ApprovalDecisionTarget = {
  kind: "task" | "direct_chat";
  id: string;
};

function approvalDecisionTargetKey(target: ApprovalDecisionTarget): string {
  return `${target.kind}:${target.id}`;
}

type TraceSectionTarget = "history" | "agentark" | "sync" | "exports" | "security";

const TRACE_SECTION_LABELS: Record<TraceSectionTarget, string> = {
  history: "Runs",
  agentark: "Runtime",
  sync: "Sync",
  exports: "Exports",
  security: "Security",
};

const TRACE_SECTION_TARGETS = new Set<TraceSectionTarget>([
  "history",
  "agentark",
  "sync",
  "exports",
  "security",
]);

function traceSectionFromStructuredValue(value: unknown): TraceSectionTarget | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase().replace(/[\s_]+/g, "-");
  if (!normalized) return null;
  const compact = normalized.replace(/-/g, "");
  if (TRACE_SECTION_TARGETS.has(normalized as TraceSectionTarget)) {
    return normalized as TraceSectionTarget;
  }
  if (TRACE_SECTION_TARGETS.has(compact as TraceSectionTarget)) {
    return compact as TraceSectionTarget;
  }
  return null;
}

function traceSearchForSection(section: TraceSectionTarget): string {
  return section === "history"
    ? ""
    : `?section=${encodeURIComponent(section)}`;
}

function traceNotificationTarget(section: TraceSectionTarget): NotificationActionTarget {
  const label = TRACE_SECTION_LABELS[section];
  return {
    view: "trace",
    label: section === "history" ? "Open Trace" : `Open Trace ${label}`,
    search: traceSearchForSection(section),
  };
}

const NOTIFICATION_SOURCE_TARGETS: Record<string, NotificationActionTarget> = {
  // `source` is a typed notification source emitted by the integration sync
  // subsystem, not user-facing wording. Route it to the matching Trace tab.
  integration_sync: traceNotificationTarget("sync"),
};

function viewFromStructuredValue(value: unknown): ViewKey | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase().replace(/[\s_]+/g, "-");
  if (!normalized) return null;
  if (VIEW_KEYS.has(normalized as ViewKey)) return normalized as ViewKey;
  const directAlias = VIEW_ALIASES[normalized];
  if (directAlias) return directAlias;
  const compact = normalized.replace(/-/g, "");
  if (VIEW_KEYS.has(compact as ViewKey)) return compact as ViewKey;
  return VIEW_ALIASES[compact] || null;
}

function notificationMetadataRecord(notification: {
  metadata?: Record<string, unknown> | null;
}): Record<string, unknown> {
  return notification.metadata && typeof notification.metadata === "object"
    ? notification.metadata
    : {};
}

function viewLabel(view: ViewKey): string {
  for (const group of NAV_GROUPS) {
    for (const item of group.items) {
      if (item.key === view) return item.label;
    }
  }
  if (view === "overview") return "Mission Control";
  return view;
}

function notificationActionTarget(notification: {
  title?: string;
  body?: string;
  source?: string;
  level?: string;
  kind?: string;
  metadata?: Record<string, unknown> | null;
}): NotificationActionTarget {
  const metadata = notificationMetadataRecord(notification);
  const action =
    metadata.action && typeof metadata.action === "object"
      ? (metadata.action as Record<string, unknown>)
      : {};
  const candidates = [
    metadata.target_view,
    metadata.targetView,
    metadata.route,
    action.target_view,
    action.targetView,
    action.view,
    notification.source,
  ];
  for (const candidate of candidates) {
    const view = viewFromStructuredValue(candidate);
    if (!view) continue;
    if (view === "trace") {
      const sectionCandidates = [
        metadata.target_section,
        metadata.targetSection,
        metadata.section,
        action.target_section,
        action.targetSection,
        action.section,
      ];
      for (const sectionCandidate of sectionCandidates) {
        const section = traceSectionFromStructuredValue(sectionCandidate);
        if (section) return traceNotificationTarget(section);
      }
    }
    return { view, label: `Open ${viewLabel(view)}` };
  }
  const sourceTarget =
    NOTIFICATION_SOURCE_TARGETS[
      (notification.source || "").trim().toLowerCase()
    ];
  if (sourceTarget) return sourceTarget;
  return { view: "overview", label: "Open Mission Control" };
}

function isRoutineSecurityGuardNotification(notification: {
  title?: string;
  source?: string;
}): boolean {
  const source = (notification.source || "").trim().toLowerCase();
  const title = (notification.title || "").trim().toLowerCase();
  return source === "security" && title === "security alert";
}

function shouldSurfaceNotification(notification: {
  title?: string;
  body?: string;
  source?: string;
  level?: string;
}): boolean {
  if (isRoutineSecurityGuardNotification(notification)) return false;
  const source = (notification.source || "").toLowerCase();
  const title = (notification.title || "").toLowerCase();
  if (source.includes("watcher") || title.includes("watcher triggered"))
    return false;
  if (source.includes("arkpulse")) return false;
  if (source.includes("predictive_nudge")) return false;
  if (title.includes("what to improve now")) return false;
  return true;
}

function notificationEventAffectsApprovals(
  payload: NotificationStreamPayload,
): boolean {
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

function notificationEventAffectsChat(
  payload: NotificationStreamPayload,
): boolean {
  const kind = (payload.kind || "").toLowerCase();
  const source = (payload.source || "").toLowerCase();
  return (
    kind.includes("conversation") ||
    source.includes("browser") ||
    source.includes("chat")
  );
}

export default function App() {
  const queryClient = useQueryClient();
  const profileQ = useQuery({
    queryKey: ["profile"],
    queryFn: () => api.rawGet("/profile"),
    staleTime: 30_000,
    refetchInterval: false,
  });
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const activeAutoRefresh = useAutoRefreshWhileActive(autoRefresh);
  const selectedNotificationId = useUiStore((s) => s.selectedNotificationId);
  const openNotification = useUiStore((s) => s.openNotification);
  const closeNotification = useUiStore((s) => s.closeNotification);
  const [view, setViewState] = useState<ViewKey>(
    () => resolveViewFromPath(window.location.pathname).view,
  );
  const [browserHandoffSessionId, setBrowserHandoffSessionId] = useState<
    string | null
  >(() => resolveBrowserHandoffPath(window.location.pathname));
  const [lastNonSettingsView, setLastNonSettingsView] =
    useState<ViewKey>("overview");
  const [settingsInitialTab, setSettingsInitialTab] = useState<number | null>(
    null,
  );
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
  }, [startTour, tourActive, tourCompleted]);

  useEffect(() => {
    if (!profileQ.isSuccess) return;
    const timezone = asRecord(profileQ.data).timezone;
    setUiTimeZoneOverride(
      typeof timezone === "string" ? timezone.trim() || null : null,
    );
  }, [profileQ.data, profileQ.isSuccess]);

  const [notifAnchorEl, setNotifAnchorEl] = useState<HTMLElement | null>(null);
  const [navHidden, setNavHidden] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.localStorage.getItem(NAV_HIDDEN_STORAGE_KEY) === "1";
  });
  const isMobileShell = useMediaQuery("(max-width:980px)");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const notifListOpen = Boolean(notifAnchorEl);
  const [notifFilter, setNotifFilter] = useState<
    "all" | "unread" | "input_needed" | "errors" | "automation_failures"
  >("all");
  const [notificationsStreamConnected, setNotificationsStreamConnected] =
    useState(false);
  const [approvalBusyTargetKey, setApprovalBusyTargetKey] = useState<string | null>(
    null,
  );
  const [approvalPopupError, setApprovalPopupError] = useState<string | null>(
    null,
  );
  const [dismissedApprovalTargetKeys, setDismissedApprovalTargetKeys] = useState<
    string[]
  >([]);
  const desktopNavCollapsed = !isMobileShell && navHidden;
  const preloadAppView = useCallback(
    (
      nextViewRaw: ViewKey | string,
      options?: { settingsTab?: number | null },
    ) => {
      const nextView = normalizeViewKey(nextViewRaw);
      if (nextView === "overview" || nextView === "library") return;
      const settingsTab = defaultSettingsTabForView(
        nextView,
        options?.settingsTab,
      );
      preloadWorkspaceSurface(nextView as WorkspaceView, settingsTab);
      if (nextView === "settings" || settingsTab != null) {
        preloadSettingsTab(settingsTab);
        prefetchSettingsTabData(queryClient, settingsTab);
      }
    },
    [queryClient],
  );
  const navigateToView = useCallback(
    (
      nextViewRaw: ViewKey | string,
      replace = false,
      searchOverride = "",
    ) => {
      const nextView = normalizeViewKey(nextViewRaw);
      const nextPath = viewPath(nextView);
      const nextSearch = searchOverride;
      preloadAppView(nextView);
      setBrowserHandoffSessionId(null);
      if (
        window.location.pathname !== nextPath ||
        window.location.search !== nextSearch
      ) {
        const nextUrl = `${nextPath}${nextSearch}`;
        if (replace) {
          window.history.replaceState(null, "", nextUrl);
        } else {
          window.history.pushState(null, "", nextUrl);
        }
      }
      if (isMobileShell) {
        setMobileNavOpen(false);
      }
      setViewState(nextView);
      window.dispatchEvent(new Event("agentark:navigation"));
    },
    [isMobileShell, preloadAppView],
  );

  useEffect(() => {
    const cancelSettingsWarmup = scheduleWarmup(() => {
      // Settings has a large editor surface. Warm it shortly after first paint
      // so opening the dialog or jumping to a tab does not pay the full
      // download/parse cost on the click path.
      preloadAppView("settings", { settingsTab: 1 });
    }, 120);
    const cancelCoreWarmup = scheduleWarmup(() => {
      preloadAppView("chat");
      preloadAppView("sentinel");
      preloadAppView("swarm");
      preloadAppView("sessions");
      preloadAppView("trace");
      void loadApprovalPromptOverlayModule();
    }, 900);
    const cancelGuidedTourWarmup = scheduleWarmup(() => {
      void loadGuidedTourModule();
    }, 1800);
    return () => {
      cancelSettingsWarmup();
      cancelCoreWarmup();
      cancelGuidedTourWarmup();
    };
  }, [preloadAppView]);

  useEffect(() => {
    const syncFromLocation = (replaceInvalid: boolean) => {
      const handoffSessionId = resolveBrowserHandoffPath(
        window.location.pathname,
      );
      setBrowserHandoffSessionId(handoffSessionId);
      if (handoffSessionId) {
        return;
      }
      const normalizedPath = window.location.pathname.replace(/\/+$/, "");
      if (
        replaceInvalid &&
        (normalizedPath.startsWith("/ui/memory") ||
          normalizedPath.startsWith("/ui/arkrecall"))
      ) {
        const nextUrl = `/ui/arkmemory${window.location.search}`;
        window.history.replaceState(null, "", nextUrl);
        setViewState("arkmemory");
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
    if (typeof window === "undefined") return;
    window.localStorage.setItem(NAV_HIDDEN_STORAGE_KEY, navHidden ? "1" : "0");
  }, [navHidden]);

  useEffect(() => {
    if (!isMobileShell) {
      setMobileNavOpen(false);
    }
  }, [isMobileShell]);

  const serverQ = useQuery({
    queryKey: ["server-ping"],
    queryFn: async () => {
      const t0 =
        typeof performance !== "undefined" ? performance.now() : Date.now();
      const status = await api.getStatus();
      const t1 =
        typeof performance !== "undefined" ? performance.now() : Date.now();
      const rttMs = Math.max(0, Math.round(t1 - t0));
      const at = Date.now();
      recordRuntimeMetricSample({ at, latencyMs: rttMs, status });
      return {
        at,
        rtt_ms: rttMs,
        status,
      };
    },
    refetchInterval: activeAutoRefresh ? REFRESH_MS : false,
    retry: 0,
  });
  const approvalTasksQ = useQuery({
    queryKey: ["approval-popup-tasks"],
    queryFn: () => api.rawGet("/tasks?limit=200"),
    refetchInterval: notificationsStreamConnected
      ? false
      : activeAutoRefresh
        ? APPROVAL_FALLBACK_POLL_MS
        : false,
    refetchIntervalInBackground:
      !notificationsStreamConnected && activeAutoRefresh,
  });
  const approvalLogQ = useQuery({
    queryKey: ["approval-popup-log"],
    queryFn: () => api.getApprovalLog(80),
    refetchInterval: notificationsStreamConnected
      ? false
      : activeAutoRefresh
        ? APPROVAL_FALLBACK_POLL_MS
        : false,
    refetchIntervalInBackground:
      !notificationsStreamConnected && activeAutoRefresh,
  });

  const notificationsQ = useQuery({
    queryKey: ["notifications"],
    queryFn: api.getNotifications,
    refetchInterval:
      activeAutoRefresh && !notificationsStreamConnected ? REFRESH_MS : false,
  });
  const notificationsCountQ = useQuery({
    queryKey: ["notifications-count"],
    queryFn: () => api.rawGet("/notifications/count"),
    refetchInterval:
      activeAutoRefresh && !notificationsStreamConnected ? REFRESH_MS : false,
  });
  const notifications = Array.isArray(notificationsQ.data)
    ? notificationsQ.data
    : [];
  const approvalTasks = useMemo(
    () => pickTasks(approvalTasksQ.data),
    [approvalTasksQ.data],
  );
  const approvalLogEntries = useMemo(
    () => pickApprovalLogEntries(approvalLogQ.data),
    [approvalLogQ.data],
  );
  const approvalPopupVisible = useMemo(
    () =>
      approvalTasks.some((task) => hasRenderableApprovalTask(task)) ||
      approvalLogEntries.some((entry) => hasRenderableApprovalLogEntry(entry)),
    [approvalTasks, approvalLogEntries],
  );
  const visibleNotifications = useMemo(
    () =>
      notifications.filter(
        (n) =>
          shouldSurfaceNotification(n) &&
          !isApprovalPopupDuplicateNotification(n, approvalPopupVisible),
      ),
    [notifications, approvalPopupVisible],
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
    ? Math.max(
        0,
        Math.min(Math.round(unreadCountFromEndpoint), visibleUnreadCount),
      )
    : visibleUnreadCount;
  const filteredNotifications = useMemo(() => {
    if (notifFilter === "all") return visibleNotifications;
    if (notifFilter === "unread")
      return visibleNotifications.filter((n) => !n.read);
    if (notifFilter === "input_needed")
      return visibleNotifications.filter((n) => isInputNeededNotification(n));
    if (notifFilter === "errors") {
      return visibleNotifications.filter((n) => {
        const level = (n.level || "").toLowerCase();
        return level === "error" || level === "critical";
      });
    }
    return visibleNotifications.filter(
      (n) =>
        isAutomationFailureNotification(n) && !isInputNeededNotification(n),
    );
  }, [visibleNotifications, notifFilter]);

  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    let invalidationTimer: number | null = null;
    let pendingApprovalInvalidation = false;
    let pendingChatInvalidation = false;

    const invalidateNotificationViews = (includeApprovalTasks: boolean) => {
      void queryClient.invalidateQueries({ queryKey: ["notifications"] });
      void queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
      void queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
      if (includeApprovalTasks) {
        void queryClient.invalidateQueries({
          queryKey: ["approval-popup-tasks"],
        });
        void queryClient.invalidateQueries({
          queryKey: ["approval-popup-log"],
        });
        void queryClient.invalidateQueries({ queryKey: ["tasks"] });
        void queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      }
    };
    const invalidateChatViews = () => {
      void queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      void queryClient.invalidateQueries({ queryKey: ["chat-conversation"] });
      void queryClient.invalidateQueries({ queryKey: ["chat-messages"] });
      void queryClient.invalidateQueries({
        queryKey: ["chat-background-sessions"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["autonomy-browser-sessions"],
      });
    };
    const flushPendingInvalidations = () => {
      invalidationTimer = null;
      const includeApprovalTasks = pendingApprovalInvalidation;
      const includeChatViews = pendingChatInvalidation;
      pendingApprovalInvalidation = false;
      pendingChatInvalidation = false;
      invalidateNotificationViews(includeApprovalTasks);
      if (includeChatViews) {
        invalidateChatViews();
      }
    };
    const queueStreamInvalidation = (
      includeApprovalTasks: boolean,
      includeChatViews: boolean,
    ) => {
      pendingApprovalInvalidation ||= includeApprovalTasks;
      pendingChatInvalidation ||= includeChatViews;
      if (invalidationTimer !== null) return;
      invalidationTimer = window.setTimeout(flushPendingInvalidations, 450);
    };

    const stream = new EventSource("/notifications/stream", {
      withCredentials: true,
    });

    const handleConnected = () => {
      setNotificationsStreamConnected(true);
      queueStreamInvalidation(true, true);
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
      queueStreamInvalidation(
        notificationEventAffectsApprovals(payload),
        notificationEventAffectsChat(payload),
      );
    };

    const handleResync = () => {
      setNotificationsStreamConnected(true);
      queueStreamInvalidation(true, true);
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
      if (invalidationTimer !== null) {
        window.clearTimeout(invalidationTimer);
      }
      stream.close();
      stream.removeEventListener("connected", handleConnected);
      stream.removeEventListener("notification", handleNotification);
      stream.removeEventListener("resync", handleResync);
      stream.removeEventListener("closed", handleClosed);
    };
  }, [queryClient]);

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
  const serverDotColor = serverQ.isError
    ? "#f44336"
    : serverQ.data && !pingStale
      ? "#4caf50"
      : "#ff9800";
  const serverPulse = !serverQ.isError && serverQ.data && !pingStale;
  const updateStatus = serverQ.data?.status.update;
  const updateAvailable = updateStatus?.state === "available";
  const updateChipLabel = updateStatus?.latest_version
    ? `Update ${updateStatus.latest_version}`
    : "Update available";

  const markReadMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/notifications/${encodeURIComponent(id)}/read`, {}),
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
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
    },
  });

  const markAllMutation = useMutation({
    mutationFn: () => api.rawPost("/notifications/read-all", {}),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["notifications"] });
      const previous = queryClient.getQueryData(["notifications"]);
      queryClient.setQueryData(["notifications"], (old: unknown) => {
        if (!Array.isArray(old)) return old;
        return old.map((row) => ({
          ...(row as Record<string, unknown>),
          read: true,
        }));
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
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
    },
  });

  const approvalDecisionMutation = useMutation({
    mutationFn: async (payload: {
      id: string;
      decision: "approve" | "reject";
      comment?: string;
    }) => {
      if (payload.decision === "approve")
        return api.approveTask(payload.id, payload.comment);
      return api.rejectTask(payload.id, payload.comment);
    },
    onSuccess: async () => {
      setApprovalPopupError(null);
      await queryClient.invalidateQueries({
        queryKey: ["approval-popup-tasks"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["approval-popup-log"],
      });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
    },
    onError: (error, payload) => {
      const staleApproval =
        error instanceof ApiRequestError &&
        (error.status === 410 || error.code === "approval_stale");
      if (staleApproval) {
        const key = approvalDecisionTargetKey({ kind: "task", id: payload.id });
        setDismissedApprovalTargetKeys((current) =>
          current.includes(key) ? current : [...current, key],
        );
        setApprovalPopupError(null);
      } else {
        setApprovalPopupError(
          error instanceof Error ? error.message : "Failed to update approval.",
        );
      }
      void queryClient.invalidateQueries({
        queryKey: ["approval-popup-tasks"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["approval-popup-log"],
      });
    },
    onSettled: () => {
      setApprovalBusyTargetKey(null);
    },
  });
  const directChatApprovalDecisionMutation = useMutation({
    mutationFn: async (payload: {
      id: string;
      decision: "approve" | "reject";
    }) =>
      api.rawPost(
        `/chat/tool-approvals/${encodeURIComponent(payload.id)}/decision`,
        { decision: payload.decision },
      ),
    onSuccess: async () => {
      setApprovalPopupError(null);
      await queryClient.invalidateQueries({
        queryKey: ["approval-popup-log"],
      });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-conversation"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-messages"] });
    },
    onError: (error) => {
      setApprovalPopupError(
        error instanceof Error ? error.message : "Failed to update approval.",
      );
      void queryClient.invalidateQueries({
        queryKey: ["approval-popup-log"],
      });
    },
    onSettled: () => {
      setApprovalBusyTargetKey(null);
    },
  });
  const approvalDismissMutation = useMutation({
    mutationFn: async (payload: {
      target: ApprovalDecisionTarget;
      comment?: string;
    }) => api.dismissApproval(payload.target.id, payload.comment),
    onMutate: async (payload) => {
      const key = approvalDecisionTargetKey(payload.target);
      setDismissedApprovalTargetKeys((current) =>
        current.includes(key) ? current : [...current, key],
      );
      return { key };
    },
    onSuccess: async () => {
      setApprovalPopupError(null);
      await queryClient.invalidateQueries({
        queryKey: ["approval-popup-tasks"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["approval-popup-log"],
      });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({
        queryKey: ["notifications-count"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["autonomy-unread-notifications"],
      });
    },
    onError: (error, _payload, context) => {
      if (context?.key) {
        setDismissedApprovalTargetKeys((current) =>
          current.filter((key) => key !== context.key),
        );
      }
      setApprovalPopupError(
        error instanceof Error ? error.message : "Failed to dismiss approval.",
      );
    },
    onSettled: () => {
      setApprovalBusyTargetKey(null);
    },
  });

  const handleApprovalDecision = (
    id: string,
    decision: "approve" | "reject",
    comment?: string,
  ) => {
    setApprovalPopupError(null);
    setApprovalBusyTargetKey(approvalDecisionTargetKey({ kind: "task", id }));
    approvalDecisionMutation.mutate({ id, decision, comment });
  };

  const handleApprovalPopupDecision = (
    target: ApprovalDecisionTarget,
    decision: "approve" | "reject",
    comment?: string,
  ) => {
    setApprovalPopupError(null);
    setApprovalBusyTargetKey(approvalDecisionTargetKey(target));
    if (target.kind === "direct_chat") {
      directChatApprovalDecisionMutation.mutate({ id: target.id, decision });
      return;
    }
    approvalDecisionMutation.mutate({ id: target.id, decision, comment });
  };

  const handleApprovalPopupDismiss = (
    target: ApprovalDecisionTarget,
    comment?: string,
  ) => {
    setApprovalPopupError(null);
    setApprovalBusyTargetKey(approvalDecisionTargetKey(target));
    approvalDismissMutation.mutate({ target, comment });
  };

  const openSettingsView = useCallback(
    (route: "settings" | "arkpulse", initialTab: number | null = null) => {
      preloadAppView(route, { settingsTab: initialTab });
      setSettingsInitialTab(initialTab);
      navigateToView(
        route,
        false,
        route === "settings" ? settingsSearchForTab(initialTab) : "",
      );
    },
    [navigateToView, preloadAppView],
  );

  const handleNotificationPrimaryAction = useCallback(
    (notification: {
      id: string;
      read?: boolean;
      title?: string;
      body?: string;
      source?: string;
      level?: string;
      metadata?: Record<string, unknown> | null;
    }) => {
      const target = notificationActionTarget(notification);
      if (!notification.read) {
        markReadMutation.mutate(notification.id);
      }
      setNotifAnchorEl(null);
      closeNotification();
      if (target.view === "settings") {
        openSettingsView("settings", target.settingsTab ?? null);
        return;
      }
      if (target.view === "arkpulse") {
        openSettingsView("arkpulse");
        return;
      }
      navigateToView(target.view, false, target.search || "");
    },
    [closeNotification, markReadMutation, navigateToView, openSettingsView],
  );

  const openGuidedTourStep = useCallback(
    (targetView: string, options?: { settingsInitialTab?: number }) => {
      if (targetView === "settings") {
        openSettingsView(
          "settings",
          typeof options?.settingsInitialTab === "number"
            ? options.settingsInitialTab
            : null,
        );
        return;
      }
      navigateToView(targetView);
    },
    [navigateToView, openSettingsView],
  );

  const closeSettingsModal = useCallback(() => {
    setSettingsInitialTab(null);
    const fallback =
      lastNonSettingsView === "settings" ? "overview" : lastNonSettingsView;
    navigateToView(fallback, true);
  }, [lastNonSettingsView, navigateToView]);

  const settingsModalOpen = view === "settings";
  const activeView: ViewKey = settingsModalOpen ? lastNonSettingsView : view;
  const workspaceView = activeView as Exclude<
    ViewKey,
    "overview" | "settings" | "library"
  >;
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

  const renderSideNav = ({
    collapsed,
    mobile = false,
  }: {
    collapsed: boolean;
    mobile?: boolean;
  }) => (
    <Box
      className={`side-nav${collapsed ? " collapsed" : ""}${mobile ? " side-nav-mobile" : ""}`}
    >
      <Stack
        direction="row"
        sx={{
          alignItems: "center",
          justifyContent: collapsed ? "center" : "space-between",
          px: collapsed ? 0 : 0.5,
          mb: collapsed ? 0.4 : 1,
        }}
      >
        {!collapsed ? (
          <Typography variant="caption" className="nav-label">
            {mobile ? "Navigation" : "Navigate"}
          </Typography>
        ) : null}
        {mobile ? (
          <Tooltip title="Close navigation">
            <IconButton
              size="small"
              className="nav-collapse-btn"
              onClick={() => setMobileNavOpen(false)}
            >
              <CloseRoundedIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        ) : (
          <Tooltip
            title={collapsed ? "Expand navigation" : "Collapse navigation"}
          >
            <IconButton
              size="small"
              className="nav-collapse-btn"
              onClick={() => setNavHidden((prev) => !prev)}
            >
              {collapsed ? (
                <ChevronRightRoundedIcon fontSize="small" />
              ) : (
                <ChevronLeftRoundedIcon fontSize="small" />
              )}
            </IconButton>
          </Tooltip>
        )}
      </Stack>
      <List disablePadding>
        {NAV_GROUPS.map((group, groupIdx) => (
          <Box key={group.id} className="nav-group">
            {!collapsed ? (
              <Stack
                direction="row"
                sx={{
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
              >
                <Typography variant="overline" className="nav-group-label">
                  {group.label}
                </Typography>
              </Stack>
            ) : null}
            {group.items.map((item) => (
              <Tooltip
                key={item.key}
                title={`${item.label}: ${item.tooltip}`}
                placement="right"
                arrow
              >
                <ListItemButton
                  selected={isNavItemActive(item.key, activeView)}
                  onClick={() => navigateToView(item.key)}
                  onMouseEnter={() => preloadAppView(item.key)}
                  onFocus={() => preloadAppView(item.key)}
                  onTouchStart={() => preloadAppView(item.key)}
                  className={`nav-item${collapsed ? " collapsed" : ""}`}
                  data-tour-target={`nav-${item.key}`}
                >
                  <ListItemIcon className="nav-item-icon">
                    {item.icon}
                  </ListItemIcon>
                  <ListItemText
                    className={`nav-item-text${collapsed ? " collapsed" : ""}`}
                    primary={item.label}
                    slotProps={{
                      primary: { noWrap: true },
                    }}
                  />
                </ListItemButton>
              </Tooltip>
            ))}
            {!collapsed && groupIdx < NAV_GROUPS.length - 1 ? (
              <Divider className="nav-group-divider" />
            ) : null}
          </Box>
        ))}
      </List>
    </Box>
  );

  if (browserHandoffSessionId) {
    return (
      <Suspense fallback={<WorkspacePaneFallback />}>
        <BrowserHandoffPage
          sessionId={browserHandoffSessionId}
          onBack={() => navigateToView("chat", true)}
        />
      </Suspense>
    );
  }

  return (
    <Box className="agi-shell">
      <AmberCascadesBackground />
      <Box className="bg-orb orb-a" />
      <Box className="bg-orb orb-b" />
      <Box className="app-frame">
        <AppBar
          position="static"
          elevation={0}
          color="transparent"
          className="glass-appbar shell-appbar"
        >
          <Toolbar
            className="shell-toolbar"
            sx={{
              minHeight: "var(--appbar-height)",
              px: { xs: 1.25, md: 1.5 },
            }}
          >
            <Stack
              direction="row"
              spacing={1}
              sx={{
                alignItems: "center",
                flexGrow: 1,
                minWidth: 0,
              }}
            >
              {isMobileShell ? (
                <Tooltip title="Open navigation">
                  <IconButton
                    color="primary"
                    className="mobile-nav-trigger"
                    onClick={() => setMobileNavOpen(true)}
                    aria-label="Open navigation"
                  >
                    <MenuRoundedIcon />
                  </IconButton>
                </Tooltip>
              ) : null}
              <Box className="shell-brand-mark">
                <img src="/logo.svg" alt="AgentArk" width={28} height={28} />
              </Box>
              <Box className="shell-brand-copy">
                <Typography variant="caption" className="shell-kicker">
                  {PRODUCT_NAME}
                </Typography>
                <Box className="shell-title-row">
                  <Typography variant="subtitle1" className="shell-title" noWrap>
                    {PRODUCT_CATEGORY}
                  </Typography>
                  <Tooltip title={serverTooltip} arrow>
                    <Box
                      onClick={() => serverQ.refetch()}
                      className={serverPulse ? "status-dot status-dot--pulse" : "status-dot"}
                      style={{
                        cursor: "pointer",
                        backgroundColor: serverDotColor,
                        boxShadow: serverPulse ? `0 0 6px 1px ${serverDotColor}` : "none",
                      }}
                    />
                  </Tooltip>
                </Box>
              </Box>
            </Stack>
            <Stack
              direction="row"
              spacing={0.5}
              className="shell-actions"
              sx={{
                alignItems: "center",
              }}
            >
              {updateAvailable ? (
                <Chip
                  size="small"
                  color="warning"
                  variant="outlined"
                  label={updateChipLabel}
                  onClick={() => openSettingsView("settings", 25)}
                  clickable
                />
              ) : null}
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
                <IconButton
                  color="primary"
                  onClick={() => openSettingsView("settings")}
                  onMouseEnter={() => preloadAppView("settings")}
                  onFocus={() => preloadAppView("settings")}
                  onTouchStart={() => preloadAppView("settings")}
                  aria-label="Open settings"
                  data-tour-target="settings-trigger"
                >
                  <SettingsRoundedIcon />
                </IconButton>
              </Tooltip>
            </Stack>
          </Toolbar>
        </AppBar>

        <Box
          className={`main-grid${desktopNavCollapsed ? " nav-collapsed" : ""}${isMobileShell ? " is-mobile-shell" : ""}`}
        >
          {!isMobileShell
            ? renderSideNav({ collapsed: desktopNavCollapsed })
            : null}

          <Box className={mainPaneClassName}>
            <Box className={stageClassName}>
              <Suspense fallback={<WorkspacePaneFallback />}>
                {activeView === "overview" ? (
                  <OverviewPane
                    navigateToView={
                      navigateToView as (
                        view: string,
                        replace?: boolean,
                      ) => void
                    }
                    serverStatus={serverQ.data}
                    serverError={serverQ.isError}
                    serverLoading={serverQ.isLoading && !serverQ.data}
                  />
                ) : activeView === "library" ? (
                  <LibraryPane
                    autoRefresh={settingsModalOpen ? false : activeAutoRefresh}
                    showAdvanced={showAdvanced}
                    onNavigateToView={
                      navigateToView as (
                        view: string,
                        replace?: boolean,
                      ) => void
                    }
                  />
                ) : (
                  <NativeWorkspace
                    view={
                      activeView === "chat"
                        ? "chat"
                        : (workspaceView as WorkspaceView)
                    }
                    autoRefresh={settingsModalOpen ? false : activeAutoRefresh}
                    showAdvanced={showAdvanced}
                    onNavigateToView={
                      navigateToView as (
                        view: string,
                        replace?: boolean,
                      ) => void
                    }
                  />
                )}
              </Suspense>
            </Box>
          </Box>
        </Box>

        <Drawer
          anchor="left"
          open={isMobileShell && mobileNavOpen}
          onClose={() => setMobileNavOpen(false)}
          ModalProps={{ keepMounted: true }}
          slotProps={{
            paper: { className: "side-nav-mobile-paper" },
          }}
        >
          {renderSideNav({ collapsed: false, mobile: true })}
        </Drawer>
      </Box>
      <Suspense fallback={null}>
        <ApprovalPromptOverlay
          tasks={approvalTasks}
          approvalLogs={approvalLogEntries}
          busyTargetKey={approvalBusyTargetKey}
          errorMessage={approvalPopupError}
          onDecide={handleApprovalPopupDecision}
          onDismiss={handleApprovalPopupDismiss}
          onOpenTasks={() => navigateToView("tasks")}
          onOpenChat={() => navigateToView("chat")}
          hiddenTargetKeys={dismissedApprovalTargetKeys}
        />
      </Suspense>
      <Dialog
        open={settingsModalOpen}
        onClose={closeSettingsModal}
        fullWidth
        maxWidth={false}
        slotProps={{
          paper: {
            sx: {
              width: { xs: "96vw", md: "82vw", lg: 1120 },
              maxWidth: 1120,
              height: { xs: "92vh", md: "84vh" },
              maxHeight: "92vh",
              borderRadius: 2.25,
              border: "1px solid var(--ui-rgba-255-255-255-080)",
              background:
                "linear-gradient(160deg, var(--ui-rgba-24-24-28-980), var(--ui-rgba-15-15-18-950))",
              backdropFilter: "blur(18px)",
              WebkitBackdropFilter: "blur(18px)",
              overflow: "hidden",
            },
          },
        }}
      >
        <DialogTitle
          sx={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            py: 1.25,
            px: 2,
            minHeight: 48,
            borderBottom: "1px solid var(--ui-rgba-255-255-255-080)",
          }}
        >
          <Typography variant="h6" sx={{ lineHeight: 1 }}>
            Settings
          </Typography>
          <IconButton
            size="small"
            onClick={closeSettingsModal}
            aria-label="Close settings"
          >
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent sx={{ p: 0, height: "100%", overflow: "hidden" }}>
          <SettingsPage autoRefresh={false} initialTab={settingsInitialTab} />
        </DialogContent>
      </Dialog>
      <Popover
        open={notifListOpen}
        anchorEl={notifAnchorEl}
        onClose={() => {
          setNotifAnchorEl(null);
          closeNotification();
        }}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
        slotProps={{
          paper: {
            className: "notification-popover-paper",
            sx: {
              width: 460,
              maxWidth: "calc(100vw - 24px)",
              maxHeight: "min(640px, calc(100vh - 88px))",
              display: "flex",
              flexDirection: "column",
              borderRadius: 2,
              overflow: "hidden",
              border: "1px solid #2a3038",
              background: "#111317 !important",
              backgroundImage: "none !important",
              boxShadow: "0 18px 54px var(--ui-rgba-0-0-0-500)",
              backdropFilter: "none !important",
              WebkitBackdropFilter: "none !important",
              color: "#f3f6f8",
            },
          },
        }}
      >
        <Box
          sx={{
            px: 1.5,
            pt: 1.25,
            pb: 1,
            background: "#111317",
            borderBottom: "1px solid #2a3038",
          }}
        >
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
            }}
          >
            <Typography
              variant="subtitle1"
              sx={{
                fontWeight: 600,
                color: "#f3f6f8",
              }}
            >
              Notifications
            </Typography>
            <Button
              size="small"
              onClick={() => markAllMutation.mutate()}
              disabled={
                markAllMutation.isPending || visibleNotifications.length === 0
              }
              sx={{
                textTransform: "none",
                fontSize: "0.75rem",
                color: "#c8d0d8",
                "&:hover": {
                  color: "#f3f6f8",
                  background: "#222831",
                },
              }}
            >
              Mark all read
            </Button>
          </Stack>
          <Stack
            direction="row"
            spacing={0.75}
            sx={{ mt: 0.75, flexWrap: "wrap" }}
            useFlexGap
          >
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
              variant={
                notifFilter === "input_needed" ? "contained" : "outlined"
              }
              onClick={() => setNotifFilter("input_needed")}
            >
              Input
            </Button>
            <Button
              size="small"
              variant={notifFilter === "errors" ? "contained" : "outlined"}
              onClick={() => setNotifFilter("errors")}
            >
              Errors
            </Button>
            <Button
              size="small"
              variant={
                notifFilter === "automation_failures" ? "contained" : "outlined"
              }
              onClick={() => setNotifFilter("automation_failures")}
            >
              Failures
            </Button>
          </Stack>
        </Box>
        <Box
          sx={{
            flex: 1,
            minHeight: 0,
            overflow: "auto",
            p: 1.25,
            background: "#0f1115",
          }}
        >
          {notificationsQ.error ? (
            <Alert severity="error">Failed to load notifications</Alert>
          ) : null}
          {filteredNotifications.length === 0 ? (
            <Box sx={{ p: 1.25 }}>
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                {visibleNotifications.length === 0
                  ? "No notifications yet."
                  : "No notifications match this filter."}
              </Typography>
            </Box>
          ) : (
            <List
              dense
              disablePadding
              sx={{ display: "flex", flexDirection: "column", gap: 0.25 }}
            >
              {filteredNotifications.slice(0, 40).map((n) => {
                const inputNeeded = isInputNeededNotification(n);
                const automationFailure =
                  isAutomationFailureNotification(n) && !inputNeeded;
                const displayTitle = notificationDisplayTitle(n);
                const displaySummary = notificationDisplaySummary(n);
                const selected = selectedNotificationId === n.id;
                const actionTarget = notificationActionTarget(n);
                return (
                  <ListItemButton
                    key={n.id}
                    sx={{
                      alignItems: "flex-start",
                      position: "relative",
                      overflow: "hidden",
                      borderRadius: 1.5,
                      px: 1.25,
                      py: 1,
                      border: selected
                        ? "1px solid #3d4652"
                        : "1px solid #232933",
                      background: selected
                        ? "#20252c"
                        : inputNeeded
                          ? "#211b10"
                          : "#171a1f",
                      transition: "background 140ms ease",
                      "&:hover": {
                        background: selected
                          ? "#252b34"
                          : inputNeeded
                            ? "#2a2112"
                            : "#20242b",
                      },
                      "&:not(:last-child)": {
                        borderBottom: "1px solid #232933",
                      },
                    }}
                    onClick={async () => {
                      openNotification(n.id);
                      if (!n.read) {
                        markReadMutation.mutate(n.id);
                      }
                    }}
                  >
                    {!n.read ? (
                      <Box
                        sx={{
                          width: 6,
                          height: 6,
                          borderRadius: "50%",
                          background: inputNeeded
                            ? "var(--ui-rgba-255-193-7-950)"
                            : "var(--ui-rgba-244-245-247-880)",
                          boxShadow: inputNeeded
                            ? "0 0 6px var(--ui-rgba-255-193-7-450)"
                            : "0 0 6px var(--ui-rgba-255-255-255-180)",
                          flexShrink: 0,
                          mt: 0.8,
                          mr: 1,
                        }}
                      />
                    ) : (
                      <Box sx={{ width: 6, flexShrink: 0, mr: 1 }} />
                    )}
                    <ListItemText
                      sx={{ my: 0, minWidth: 0 }}
                      primary={
                        <Stack
                          direction="row"
                          spacing={2}
                          sx={{
                            justifyContent: "space-between",
                            minWidth: 0,
                          }}
                        >
                          <Typography
                            variant="body2"
                            noWrap
                            title={n.title || displayTitle}
                            sx={{
                              fontWeight: n.read ? 400 : 600,
                              minWidth: 0,
                              flex: 1,
                              color: n.read
                                ? "#b5bdc8"
                                : "#f3f6f8",
                            }}
                          >
                            {displayTitle}
                          </Typography>
                          <Typography
                            variant="caption"
                            noWrap
                            sx={{
                              flexShrink: 0,
                              color: "#9aa5b1",
                            }}
                            title={notifTimeAgo(n.created_at).tip}
                          >
                            {notifTimeAgo(n.created_at).label}
                          </Typography>
                        </Stack>
                      }
                      secondary={
                        <Stack spacing={0.5} sx={{ mt: 0.35 }}>
                          <Typography
                            variant="caption"
                            sx={{
                              display: "block",
                              color: n.read
                                ? "#9aa5b1"
                                : "#d4d9df",
                              lineHeight: 1.45,
                              whiteSpace: selected ? "pre-wrap" : "nowrap",
                            }}
                            noWrap={!selected}
                            title={displaySummary}
                          >
                            {displaySummary}
                          </Typography>
                          <Stack
                            direction="row"
                            spacing={0.5}
                            useFlexGap
                            sx={{
                              flexWrap: "wrap",
                            }}
                          >
                            {inputNeeded ? (
                              <Chip
                                size="small"
                                label="Waiting on you"
                                color="warning"
                                variant="outlined"
                                sx={{ height: 22 }}
                              />
                            ) : null}
                            {automationFailure ? (
                              <Chip
                                size="small"
                                label="Automation failure"
                                color="error"
                                variant="outlined"
                                sx={{ height: 22 }}
                              />
                            ) : null}
                            {n.source ? (
                              <Chip
                                size="small"
                                label={n.source}
                                variant="outlined"
                                sx={{
                                  height: 22,
                                  color: "var(--ui-rgba-187-191-199-760)",
                                  borderColor: "var(--ui-rgba-255-255-255-080)",
                                }}
                              />
                            ) : null}
                          </Stack>
                          {selected ? (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{
                                alignItems: "center",
                                flexWrap: "wrap",
                                pt: 0.35,
                              }}
                            >
                              <Button
                                size="small"
                                variant="contained"
                                endIcon={
                                  <ChevronRightRoundedIcon fontSize="small" />
                                }
                                onClick={(event) => {
                                  event.stopPropagation();
                                  handleNotificationPrimaryAction(n);
                                }}
                              >
                                {actionTarget.label}
                              </Button>
                              <Button
                                size="small"
                                variant="text"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  closeNotification();
                                }}
                              >
                                Collapse
                              </Button>
                            </Stack>
                          ) : null}
                        </Stack>
                      }
                    />
                  </ListItemButton>
                );
              })}
            </List>
          )}
        </Box>
      </Popover>
      <Suspense fallback={null}>
        <GuidedTour openTourStep={openGuidedTourStep} currentView={view} />
      </Suspense>
    </Box>
  );
}
