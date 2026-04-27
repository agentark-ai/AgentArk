import { Alert, Box, Divider, Typography } from "@mui/material";
import {
  useEffect,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import SettingsPage from "./pages/SettingsPage";
import type { JsonRecord } from "./pages/pageHelpers";

type WorkspaceViewOutletProps = {
  view: string;
  autoRefresh: boolean;
  settingsInitialTab?: number | null;
  projects: JsonRecord[];
  activeProjectId: string;
  onNavigateToView?: (view: string, replace?: boolean) => void;
  onOpenProjectWorkspace: (projectId: string) => void;
};

type LoadedPage = ComponentType<any>;
type PageLoader = () => Promise<{ default: LoadedPage }>;

type WorkspaceViewRouteConfig = {
  componentKey: string;
  message: string;
  load: PageLoader;
  render: (Page: LoadedPage, props: WorkspaceViewOutletProps) => ReactNode;
};

const DIVIDER_VIEWS = new Set(["tasks", "sessions", "skills", "apps"]);
const pageCache = new Map<string, LoadedPage>();
const pageLoads = new Map<string, Promise<LoadedPage>>();
pageCache.set("settings", SettingsPage as LoadedPage);

const loadAppsPage: PageLoader = () => import("./pages/AppsPage");
const loadAnalyticsPage: PageLoader = () => import("./pages/AnalyticsPage");
const loadArkPulsePage: PageLoader = () => import("./pages/ArkPulsePage");
const loadArkMemoryPage: PageLoader = () => import("./pages/ArkMemoryPage");
const loadAutonomyPage: PageLoader = () => import("./pages/AutonomyPage");
const loadDocumentsPage: PageLoader = () => import("./pages/DocumentsPage");
const loadEvolutionPage: PageLoader = () => import("./pages/EvolutionPage");
const loadGoalsPage: PageLoader = () => import("./pages/GoalsPage");
const loadProjectsPage: PageLoader = () => import("./pages/ProjectsPage");
const loadSkillsPage: PageLoader = () => import("./pages/SkillsPage");
const loadTasksPage: PageLoader = () => import("./pages/TasksPage");
const loadTracePage: PageLoader = () => import("./pages/TracePage");
const loadWatchersPage: PageLoader = () => import("./pages/WatchersPage");
const loadBackgroundSessionsManager: PageLoader = () =>
  import("./BackgroundSessionsManager").then((module) => ({
    default: module.BackgroundSessionsManager as LoadedPage,
  }));
const loadSentinelPanel: PageLoader = () =>
  import("./SentinelPanel").then((module) => ({
    default: module.SentinelPanel as LoadedPage,
  }));
const loadSwarmManager: PageLoader = () =>
  import("./SwarmManager").then((module) => ({
    default: module.SwarmManager as LoadedPage,
  }));

function loadPage(componentKey: string, loader: PageLoader) {
  const cached = pageCache.get(componentKey);
  if (cached) return Promise.resolve(cached);

  let pending = pageLoads.get(componentKey);
  if (!pending) {
    pending = loader().then((module) => {
      pageCache.set(componentKey, module.default);
      pageLoads.delete(componentKey);
      return module.default;
    });
    pageLoads.set(componentKey, pending);
  }
  return pending;
}

export function preloadWorkspaceRoute(view: string): void {
  const route = VIEW_ROUTES[view];
  if (!route) return;
  void loadPage(route.componentKey, route.load);
}

function useLoadedRouteComponent(route: WorkspaceViewRouteConfig | undefined) {
  const [loaded, setLoaded] = useState<{
    componentKey: string;
    Page: LoadedPage;
  } | null>(() =>
    route
      ? (() => {
          const Page = pageCache.get(route.componentKey);
          return Page ? { componentKey: route.componentKey, Page } : null;
        })()
      : null,
  );
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    if (!route) {
      setLoaded(null);
      setError(null);
      return;
    }

    const cached = pageCache.get(route.componentKey);
    if (cached) {
      setLoaded({ componentKey: route.componentKey, Page: cached });
      setError(null);
      return;
    }

    let cancelled = false;
    setLoaded(null);
    setError(null);
    loadPage(route.componentKey, route.load)
      .then((Loaded) => {
        if (!cancelled) {
          setLoaded({ componentKey: route.componentKey, Page: Loaded });
        }
      })
      .catch((loadError) => {
        if (!cancelled) {
          setError(loadError);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [route]);

  return {
    Page:
      route && loaded?.componentKey === route.componentKey ? loaded.Page : null,
    error,
  };
}

function WorkspaceLoadingPanel({
  message = "Loading panel...",
}: {
  message?: string;
}) {
  return (
    <Box className="list-shell" sx={{ minHeight: 180, p: 1.5 }}>
      <Typography
        variant="body2"
        sx={{
          color: "text.secondary",
        }}
      >
        {message}
      </Typography>
    </Box>
  );
}

function settingsRoute(
  message: string,
  initialTab: number,
): WorkspaceViewRouteConfig {
  return {
    componentKey: "settings",
    message,
    load: () => Promise.resolve({ default: SettingsPage as LoadedPage }),
    render: (SettingsPage, { autoRefresh }) => (
      <SettingsPage autoRefresh={autoRefresh} initialTab={initialTab} />
    ),
  };
}

const VIEW_ROUTES: Record<string, WorkspaceViewRouteConfig> = {
  connections: settingsRoute("Loading connections...", 2),
  channels: settingsRoute("Loading channels...", 2),
  routing: settingsRoute("Loading routing...", 2),
  webhooks: settingsRoute("Loading webhooks...", 2),
  devices: settingsRoute("Loading devices...", 26),
  // Keep the legacy browser route alive, but redirect it to Integrations
  // while the browser-profile UI is hidden.
  browser: settingsRoute("Loading browser integrations...", 2),
  gatewayops: {
    componentKey: "arkpulse",
    message: "Loading gateway ops...",
    load: loadArkPulsePage,
    render: (ArkPulsePage, { autoRefresh }) => (
      <ArkPulsePage autoRefresh={autoRefresh} />
    ),
  },
  failover: settingsRoute("Loading failover...", 1),
  tasks: {
    componentKey: "tasks",
    message: "Loading tasks...",
    load: loadTasksPage,
    render: (TasksPage, { autoRefresh }) => (
      <TasksPage autoRefresh={autoRefresh} />
    ),
  },
  sessions: {
    componentKey: "sessions",
    message: "Loading sessions...",
    load: loadBackgroundSessionsManager,
    render: (BackgroundSessionsManager, { autoRefresh }) => (
      <BackgroundSessionsManager autoRefresh={autoRefresh} />
    ),
  },
  skills: {
    componentKey: "skills",
    message: "Loading skills...",
    load: loadSkillsPage,
    render: (SkillsPage, { autoRefresh }) => (
      <SkillsPage autoRefresh={autoRefresh} />
    ),
  },
  apps: {
    componentKey: "apps",
    message: "Loading apps...",
    load: loadAppsPage,
    render: (AppsPage, { autoRefresh }) => <AppsPage autoRefresh={autoRefresh} />,
  },
  goals: {
    componentKey: "goals",
    message: "Loading goals...",
    load: loadGoalsPage,
    render: (GoalsPage, { autoRefresh }) => (
      <GoalsPage autoRefresh={autoRefresh} />
    ),
  },
  autonomy: {
    componentKey: "autonomy",
    message: "Loading autonomy...",
    load: loadAutonomyPage,
    render: (AutonomyPage, { autoRefresh }) => (
      <AutonomyPage autoRefresh={autoRefresh} />
    ),
  },
  evolution: {
    componentKey: "evolution",
    message: "Loading evolution...",
    load: loadEvolutionPage,
    render: (EvolutionPage, { autoRefresh }) => (
      <EvolutionPage autoRefresh={autoRefresh} />
    ),
  },
  sentinel: {
    componentKey: "sentinel",
    message: "Loading sentinel...",
    load: loadSentinelPanel,
    render: (SentinelPanel, { autoRefresh, onNavigateToView }) => (
      <SentinelPanel
        autoRefresh={autoRefresh}
        navigateToView={(nextView: string, replace?: boolean) =>
          onNavigateToView?.(nextView, replace)
        }
      />
    ),
  },
  documents: {
    componentKey: "documents",
    message: "Loading documents...",
    load: loadDocumentsPage,
    render: (
      DocumentsPage,
      {
      autoRefresh,
      projects,
      activeProjectId,
      onNavigateToView,
      },
    ) => (
      <DocumentsPage
        autoRefresh={autoRefresh}
        projects={projects}
        activeProjectId={activeProjectId}
        onNavigateToView={onNavigateToView}
      />
    ),
  },
  projects: {
    componentKey: "projects",
    message: "Loading projects...",
    load: loadProjectsPage,
    render: (
      ProjectsPage,
      {
      autoRefresh,
      projects,
      activeProjectId,
      onOpenProjectWorkspace,
      },
    ) => (
      <ProjectsPage
        autoRefresh={autoRefresh}
        projects={projects}
        activeProjectId={activeProjectId}
        onOpenProjectWorkspace={onOpenProjectWorkspace}
      />
    ),
  },
  swarm: {
    componentKey: "swarm",
    message: "Loading swarm...",
    load: loadSwarmManager,
    render: (SwarmManager, { autoRefresh }) => (
      <SwarmManager autoRefresh={autoRefresh} />
    ),
  },
  trace: {
    componentKey: "trace",
    message: "Loading trace...",
    load: loadTracePage,
    render: (TracePage, { autoRefresh }) => (
      <TracePage autoRefresh={autoRefresh} />
    ),
  },
  status: {
    componentKey: "status",
    message: "Loading watchers...",
    load: loadWatchersPage,
    render: (WatchersPage, { autoRefresh }) => (
      <WatchersPage autoRefresh={autoRefresh} />
    ),
  },
  analytics: {
    componentKey: "analytics",
    message: "Loading analytics...",
    load: loadAnalyticsPage,
    render: (AnalyticsPage, { autoRefresh }) => (
      <AnalyticsPage autoRefresh={autoRefresh} />
    ),
  },
  arkpulse: {
    componentKey: "arkpulse",
    message: "Loading ArkPulse...",
    load: loadArkPulsePage,
    render: (ArkPulsePage, { autoRefresh }) => (
      <ArkPulsePage autoRefresh={autoRefresh} />
    ),
  },
  search: settingsRoute("Loading search settings...", 24),
  settings: {
    componentKey: "settings",
    message: "Loading settings...",
    load: () => Promise.resolve({ default: SettingsPage as LoadedPage }),
    render: (SettingsPage, { autoRefresh, settingsInitialTab }) => (
      <SettingsPage autoRefresh={autoRefresh} initialTab={settingsInitialTab} />
    ),
  },
  arkmemory: {
    componentKey: "arkmemory",
    message: "Loading memory...",
    load: loadArkMemoryPage,
    render: (
      ArkMemoryPage,
      {
      autoRefresh,
      projects,
      activeProjectId,
      onNavigateToView,
      },
    ) => (
      <ArkMemoryPage
        autoRefresh={autoRefresh}
        projects={projects}
        activeProjectId={activeProjectId}
        onNavigateToView={onNavigateToView}
      />
    ),
  },
};

export function WorkspaceViewOutlet(props: WorkspaceViewOutletProps) {
  const route = VIEW_ROUTES[props.view];
  const { Page, error } = useLoadedRouteComponent(route);
  if (!route) return null;
  return (
    <>
      {error ? (
        <Alert severity="error" sx={{ mb: 1.25 }}>
          Failed to load this workspace page. Try refreshing the app.
        </Alert>
      ) : Page ? (
        route.render(Page, props)
      ) : (
        <WorkspaceLoadingPanel message={route.message} />
      )}
      {DIVIDER_VIEWS.has(props.view) ? <Divider sx={{ mt: 2 }} /> : null}
    </>
  );
}
