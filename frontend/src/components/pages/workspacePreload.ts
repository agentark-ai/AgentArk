export function normalizeSettingsPreloadTab(
  rawTab?: number | null,
): number | null {
  if (typeof rawTab !== "number" || !Number.isFinite(rawTab)) return null;
  const tab = Math.trunc(rawTab);
  if (tab === 2 || tab === 10 || tab === 15) return 20;
  if (tab === 21 || tab === 22 || tab === 23) return 20;
  if (tab === 16) return 4;
  if (tab === 9 || tab === 13 || tab === 17) return 0;
  return tab;
}

const loadedPreloads = new Set<string>();

function preloadOnce(key: string, loader: () => Promise<unknown>): void {
  if (loadedPreloads.has(key)) return;
  loadedPreloads.add(key);
  void loader().catch(() => {
    loadedPreloads.delete(key);
  });
}

export function preloadSettingsShell(): void {
  // SettingsPage is statically imported by the app shell; keep this hook for
  // callers that warm settings data/panels without triggering a redundant
  // dynamic import.
}

export function preloadSettingsFull(): void {
  preloadOnce("settings-full", () => import("./SettingsPageFull"));
}

export function preloadSettingsTab(rawTab?: number | null): void {
  preloadSettingsShell();
  const tab = normalizeSettingsPreloadTab(rawTab);
  if (tab == null) {
    preloadSettingsFull();
    return;
  }
  preloadSettingsFull();
  switch (tab) {
    case 1:
      // Models panel is now statically imported by SettingsPageFull, so no
      // separate preload is needed. Falls through to settings-full preload.
      break;
    case 8:
    case 20:
      preloadOnce("settings-integrations", () => import("../IntegrationsPanel"));
      break;
    case 11:
      preloadOnce("settings-trace", () => import("./TracePage"));
      break;
    case 12:
      preloadOnce("settings-memory", () => import("./MemoryPage"));
      break;
    default:
      break;
  }
}
