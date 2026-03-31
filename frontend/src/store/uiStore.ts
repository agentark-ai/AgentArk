import { create } from "zustand";

const TOUR_COMPLETED_KEY = "agentark.tour.completed";
const ACTIVE_PROJECT_ID_KEY = "agentark.workspace.activeProjectId";

function isTourCompleted(): boolean {
  try {
    return window.localStorage.getItem(TOUR_COMPLETED_KEY) === "1";
  } catch {
    return false;
  }
}

function persistTourCompleted(done: boolean): void {
  try {
    window.localStorage.setItem(TOUR_COMPLETED_KEY, done ? "1" : "0");
  } catch {
    /* ignore storage failures */
  }
}

function loadActiveProjectId(): string {
  try {
    return (window.localStorage.getItem(ACTIVE_PROJECT_ID_KEY) || "").trim();
  } catch {
    return "";
  }
}

function persistActiveProjectId(projectId: string): void {
  const normalized = projectId.trim();
  try {
    if (normalized) {
      window.localStorage.setItem(ACTIVE_PROJECT_ID_KEY, normalized);
    } else {
      window.localStorage.removeItem(ACTIVE_PROJECT_ID_KEY);
    }
  } catch {
    /* ignore storage failures */
  }
}

type UiState = {
  autoRefresh: boolean;
  activeProjectId: string;
  showAdvancedByView: Record<string, boolean>;
  selectedNotificationId: string | null;
  tourActive: boolean;
  tourStep: number;
  tourCompleted: boolean;
  setActiveProjectId: (projectId: string) => void;
  toggleAdvanced: (viewKey: string) => void;
  openNotification: (id: string) => void;
  closeNotification: () => void;
  startTour: () => void;
  nextTourStep: () => void;
  prevTourStep: () => void;
  skipTour: () => void;
  completeTour: () => void;
};

export const useUiStore = create<UiState>((set) => ({
  autoRefresh: true,
  activeProjectId: loadActiveProjectId(),
  showAdvancedByView: {},
  selectedNotificationId: null,
  tourActive: false,
  tourStep: 0,
  tourCompleted: isTourCompleted(),
  setActiveProjectId: (projectId) => {
    const normalized = projectId.trim();
    persistActiveProjectId(normalized);
    set({ activeProjectId: normalized });
  },
  toggleAdvanced: (viewKey) =>
    set((s) => ({
      showAdvancedByView: {
        ...s.showAdvancedByView,
        [viewKey]: !(s.showAdvancedByView[viewKey] ?? false)
      }
    })),
  openNotification: (id) => set({ selectedNotificationId: id }),
  closeNotification: () => set({ selectedNotificationId: null }),
  startTour: () => set({ tourActive: true, tourStep: 0 }),
  nextTourStep: () => set((s) => ({ tourStep: s.tourStep + 1 })),
  prevTourStep: () => set((s) => ({ tourStep: Math.max(0, s.tourStep - 1) })),
  skipTour: () => {
    persistTourCompleted(true);
    set({ tourActive: false, tourStep: 0, tourCompleted: true });
  },
  completeTour: () => {
    persistTourCompleted(true);
    set({ tourActive: false, tourStep: 0, tourCompleted: true });
  },
}));
