import {
  preloadCommonSettingsPanels,
  preloadSettingsTab,
} from "./pages/workspacePreload";
import { preloadWorkspaceRoute } from "./WorkspaceViewOutlet";

export type WorkspaceView =
  | "chat"
  | "connections"
  | "channels"
  | "routing"
  | "webhooks"
  | "devices"
  | "browser"
  | "gatewayops"
  | "failover"
  | "tasks"
  | "sessions"
  | "skills"
  | "apps"
  | "goals"
  | "autonomy"
  | "evolution"
  | "arkmemory"
  | "sentinel"
  | "documents"
  | "projects"
  | "swarm"
  | "trace"
  | "status"
  | "analytics"
  | "arkpulse"
  | "search"
  | "settings";

export { preloadCommonSettingsPanels, preloadSettingsTab };

export function preloadWorkspaceSurface(
  view: WorkspaceView,
  settingsTab?: number | null,
): void {
  preloadWorkspaceRoute(view);
  switch (view) {
    case "settings":
      preloadSettingsTab(settingsTab);
      return;
    case "connections":
    case "channels":
    case "routing":
    case "webhooks":
    case "devices":
    case "browser":
      preloadSettingsTab(view === "devices" ? 26 : settingsTab ?? 20);
      return;
    case "search":
      preloadSettingsTab(settingsTab ?? 24);
      return;
    case "failover":
      preloadSettingsTab(settingsTab ?? 1);
      return;
    default:
      return;
  }
}
