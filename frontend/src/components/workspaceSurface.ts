import { preloadSettingsTab } from "./pages/workspacePreload";
import { preloadWorkspaceRoute } from "./WorkspaceViewOutlet";

export type WorkspaceView =
  | "chat"
  | "voice"
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
  | "arkreflect"
  | "sentinel"
  | "documents"
  | "swarm"
  | "trace"
  | "status"
  | "analytics"
  | "arkpulse"
  | "arkorbit"
  | "search"
  | "settings";

export { preloadSettingsTab };

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
      preloadSettingsTab(settingsTab ?? 20);
      return;
    case "webhooks":
      preloadSettingsTab(settingsTab ?? 22);
      return;
    case "devices":
      preloadSettingsTab(settingsTab ?? 26);
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
