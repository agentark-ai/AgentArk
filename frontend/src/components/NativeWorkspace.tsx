import { Alert, Box } from "@mui/material";
import { useQuery } from "@tanstack/react-query";
import { memo, useCallback, useEffect, useMemo } from "react";
import { api } from "../api/client";
import { useUiStore } from "../store/uiStore";
import ChatPage from "./pages/ChatPage";
import { errMessage, pickRecords } from "./pages/pageHelpers";
import { normalizeProjectId } from "./pages/projectScope";
import { REFRESH_MS } from "./pages/workspaceCore";
import { WorkspaceViewOutlet } from "./WorkspaceViewOutlet";
import { type WorkspaceView } from "./workspaceSurface";

export {
  preloadCommonSettingsPanels,
  preloadSettingsTab,
  preloadWorkspaceSurface,
} from "./workspaceSurface";
export type { WorkspaceView } from "./workspaceSurface";

function NativeWorkspaceInner({
  view,
  autoRefresh,
  showAdvanced: _showAdvanced,
  settingsInitialTab,
  onNavigateToView,
}: {
  view: WorkspaceView;
  autoRefresh: boolean;
  showAdvanced: boolean;
  settingsInitialTab?: number | null;
  onNavigateToView?: (view: string, replace?: boolean) => void;
}) {
  const activeProjectId = useUiStore((state) => state.activeProjectId);
  const setActiveProjectId = useUiStore((state) => state.setActiveProjectId);

  const isChat = view === "chat";
  const isSettingsSurface =
    view === "settings" ||
    [
      "connections",
      "channels",
      "routing",
      "webhooks",
      "devices",
      "browser",
      "gatewayops",
      "failover",
      "search",
    ].includes(view);
  const needsProjects = ["chat", "documents", "arkmemory", "projects"].includes(
    view,
  );
  const showProjectScopeBar = view === "documents";

  const projectsQ = useQuery({
    queryKey: ["workspace-projects"],
    queryFn: () => api.rawGet("/projects"),
    enabled: needsProjects,
    refetchInterval: autoRefresh && needsProjects ? REFRESH_MS : false,
  });
  const projects = useMemo(
    () => pickRecords(projectsQ.data, "projects"),
    [projectsQ.data],
  );
  const handleOpenProjectWorkspace = useCallback(
    (projectId: string) => {
      setActiveProjectId(projectId);
      onNavigateToView?.("chat");
    },
    [onNavigateToView, setActiveProjectId],
  );

  useEffect(() => {
    if (
      !needsProjects ||
      !activeProjectId ||
      !projectsQ.isSuccess ||
      projectsQ.isFetching
    ) {
      return;
    }
    if (
      !projects.some(
        (project) => normalizeProjectId(project.id) === activeProjectId,
      )
    ) {
      setActiveProjectId("");
    }
  }, [
    activeProjectId,
    needsProjects,
    projects,
    projectsQ.isFetching,
    projectsQ.isSuccess,
    setActiveProjectId,
  ]);

  return (
    <Box
      sx={{
        p: isChat
          ? { xs: 0.35, md: 0.45 }
          : isSettingsSurface
            ? { xs: 1, md: 1.25 }
            : { xs: 0.75, md: 1 },
        height: "100%",
        overflow: isChat ? "hidden" : "auto",
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        minWidth: 0,
        width: "100%",
      }}
    >
      {showProjectScopeBar && projectsQ.error ? (
        <Alert severity="error" sx={{ mb: 1.25, flexShrink: 0 }}>
          {errMessage(projectsQ.error)}
        </Alert>
      ) : null}

      <Box
        sx={{
          display: isChat ? "flex" : "none",
          flex: 1,
          minHeight: 0,
          minWidth: 0,
          width: "100%",
        }}
      >
        <ChatPage
          autoRefresh={autoRefresh}
          isActive={isChat}
          projects={projects}
          activeProjectId={activeProjectId}
          onNavigateToView={onNavigateToView}
        />
      </Box>

      <WorkspaceViewOutlet
        view={view}
        autoRefresh={autoRefresh}
        settingsInitialTab={settingsInitialTab}
        projects={projects}
        activeProjectId={activeProjectId}
        onNavigateToView={onNavigateToView}
        onOpenProjectWorkspace={handleOpenProjectWorkspace}
      />
    </Box>
  );
}

export const NativeWorkspace = memo(NativeWorkspaceInner);
NativeWorkspace.displayName = "NativeWorkspace";
