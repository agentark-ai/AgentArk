export function normalizeSettingsPreloadTab(
  rawTab?: number | null,
): number | null {
  if (typeof rawTab !== "number" || !Number.isFinite(rawTab)) return null;
  const tab = Math.trunc(rawTab);
  if (tab === 2 || tab === 10 || tab === 15) return 20;
  return tab;
}

const loadedPreloads = new Set<string>();
const COMMON_SETTINGS_TABS = [3, 6, 8, 11, 12, 20, 21, 22, 23, 26] as const;
let commonSettingsPreloadScheduled = false;

function preloadOnce(key: string, loader: () => Promise<unknown>): void {
  if (loadedPreloads.has(key)) return;
  loadedPreloads.add(key);
  void loader().catch(() => {
    loadedPreloads.delete(key);
  });
}

function schedulePreload(task: () => void, delayMs: number): void {
  if (typeof window === "undefined") {
    task();
    return;
  }
  window.setTimeout(task, delayMs);
}

export function preloadSettingsShell(): void {
  preloadOnce("settings-shell", () => import("./SettingsPage"));
}

export function preloadSettingsFull(): void {
  preloadOnce("settings-full", () => import("./SettingsPageFull"));
}

export function preloadSettingsTab(rawTab?: number | null): void {
  preloadSettingsShell();
  const tab = normalizeSettingsPreloadTab(rawTab);
  if (tab == null) return;
  if (tab !== 0) {
    preloadSettingsFull();
  }
  switch (tab) {
    case 3:
      preloadOnce("settings-media", () => import("./MediaSettingsPanel"));
      break;
    case 6:
      preloadOnce("settings-observability", () => import("../ObservabilityPanel"));
      break;
    case 8:
    case 20:
    case 21:
      preloadOnce("settings-integrations", () => import("../IntegrationsPanel"));
      break;
    case 11:
      preloadOnce("settings-trace", () => import("./TracePage"));
      break;
    case 12:
      preloadOnce("settings-memory", () => import("./MemoryPage"));
      break;
    case 22:
      preloadOnce("settings-webhooks", () => import("../WebhooksPanel"));
      preloadOnce("settings-quickstart", () => import("../IntegrationQuickstartPanel"));
      break;
    case 23:
      preloadOnce("settings-plugins", () => import("../PluginSdkPanel"));
      break;
    case 26:
      preloadOnce("settings-devices", () => import("../CompanionDevicesPanel"));
      break;
    default:
      break;
  }
}

export function preloadCommonSettingsPanels(): void {
  preloadSettingsShell();
  preloadSettingsFull();
  if (commonSettingsPreloadScheduled) return;
  commonSettingsPreloadScheduled = true;
  COMMON_SETTINGS_TABS.forEach((tab, index) => {
    schedulePreload(() => preloadSettingsTab(tab), 40 + index * 50);
  });
}
