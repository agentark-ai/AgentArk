export type SettingsPageProps = {
  autoRefresh: boolean;
  initialTab?: number | null;
  hideSettingsNav?: boolean;
  standaloneSurface?: "arkpulse";
};

export type SettingsPageMeta = {
  kicker: string;
  title?: string;
  description: string;
};

export function normalizeSettingsTab(rawTab?: number | null): number {
  if (typeof rawTab !== "number" || !Number.isFinite(rawTab)) return 0;
  const tab = Math.max(0, Math.trunc(rawTab));
  if (tab === 2 || tab === 10 || tab === 15) return 20;
  if (tab === 21 || tab === 22 || tab === 23) return 20;
  if (tab === 16) return 4;
  if (tab === 9 || tab === 13 || tab === 17) return 0;
  return tab;
}

export function settingsTabFromLocation(): number {
  if (typeof window === "undefined") return 0;
  const raw = new URLSearchParams(window.location.search).get("settings_tab");
  if (!raw) return 0;
  const normalized = raw.trim().toLowerCase();
  const byName: Record<string, number> = {
    quick: 0,
    setup: 0,
    models: 1,
    channels: 20,
    connections: 20,
    integrations: 20,
    messaging: 20,
    media: 3,
    security: 4,
    verification: 4,
    trust: 4,
    "sender-verification": 4,
    senderverification: 4,
    observability: 6,
    telemetry: 6,
    webhooks: 20,
    plugins: 20,
    plugin: 20,
    sdk: 20,
    ingress: 20,
    events: 20,
    connectors: 20,
    prebuilt: 20,
    companion: 26,
    "companion-devices": 26,
    companiondevices: 26,
    devices: 26,
    advanced: 5,
    lifecycle: 14,
    "data-lifecycle": 14,
    retention: 14,
    update: 25,
    updates: 25,
    upgrade: 25,
    mcp: 8,
    routing: 20,
    browser: 20,
    failover: 1,
    reliability: 1,
    gateway: 0,
    gatewayops: 0,
    system: 0,
    trace: 11,
  };
  if (normalized in byName) return normalizeSettingsTab(byName[normalized]);
  const asNumber = Number(normalized);
  return Number.isFinite(asNumber) ? normalizeSettingsTab(asNumber) : 0;
}

export function resolveInitialSettingsTab(initialTab?: number | null): number {
  if (typeof initialTab === "number") return normalizeSettingsTab(initialTab);
  return settingsTabFromLocation();
}

export function settingsTabSupportsSave(tab: number): boolean {
  return ![8, 9, 11, 13, 17, 20, 21, 22, 23, 25, 26].includes(tab);
}

export function getSettingsTabLoadingMessage(tab: number): string {
  switch (normalizeSettingsTab(tab)) {
    case 1:
      return "Loading models...";
    case 3:
      return "Loading media settings...";
    case 4:
      return "Loading security settings...";
    case 5:
      return "Loading advanced settings...";
    case 6:
      return "Loading observability...";
    case 8:
      return "Loading MCP servers...";
    case 11:
      return "Loading trace...";
    case 12:
      return "Loading memory...";
    case 14:
      return "Loading data cleanup...";
    case 20:
      return "Loading integrations...";
    case 21:
      return "Loading integrations...";
    case 22:
      return "Loading webhooks and APIs...";
    case 23:
      return "Loading plugins...";
    case 24:
      return "Loading search settings...";
    case 25:
      return "Loading updates...";
    case 26:
      return "Loading companion devices...";
    default:
      return "Loading settings...";
  }
}

export function getSettingsPageMeta(tab: number): SettingsPageMeta {
  switch (tab) {
    case 0:
      return {
        kicker: "Setup",
        description:
          "OS identity, defaults, and readiness signals for this AgentArk workspace.",
      };
    case 1:
      return {
        kicker: "Setup",
        description:
          "Model routing, fallbacks, and provider posture for operator and autonomous work.",
      };
    case 3:
      return {
        kicker: "Setup",
        description:
          "Media generation defaults, rendering paths, and asset handling behavior.",
      };
    case 24:
      return {
        kicker: "Setup",
        description:
          "Search providers, precedence, and fallback behavior for web search and deep research.",
      };
    case 4:
      return {
        kicker: "Security",
        description:
          "Instance protection, secrets posture, and security review controls.",
      };
    case 5:
      return {
        kicker: "Security",
        description:
          "Advanced runtime switches and expert-only instance controls.",
      };
    case 6:
      return {
        kicker: "Admin",
        description:
          "External trace export, privacy mode, and observability delivery configuration.",
      };
    case 8:
      return {
        kicker: "Integrations",
        description:
          "MCP server registration, transport, auth, and tool/resource exposure.",
      };
    case 9:
      return {
        kicker: "Ark Core",
        description:
          "Operational health, onboarding readiness, and runtime drift checks in one place.",
      };
    case 11:
      return {
        kicker: "Observability",
        description:
          "Execution history, integration sync runs, and export delivery in one place.",
      };
    case 12:
      return {
        kicker: "Knowledge",
        description:
          "Durable memory, learned facts, and knowledge controls used across conversations.",
      };
    case 13:
      return {
        kicker: "Admin",
        description:
          "Review what AgentArk has learned, what is proposed, and what should be promoted.",
      };
    case 14:
      return {
        kicker: "Admin",
        title: "Data Cleanup",
        description:
          "Retention windows, cleanup cadence, and long-run storage posture.",
      };
    case 25:
      return {
        kicker: "Admin",
        title: "Updates",
        description:
          "Release status, restart-aware update flow, and version pinning for this installation.",
      };
    case 20:
      return {
        kicker: "Integrations",
        description:
          "Messaging channels, built-in connectors, custom APIs, webhooks, plugins, and extension packs in one hub.",
      };
    case 21:
      return {
        kicker: "Integrations",
        description:
          "Built-in connectors and user-added custom integrations for chat, tasks, and automation.",
      };
    case 22:
      return {
        kicker: "Integrations",
        description:
          "Inbound webhooks and imported external APIs that create or extend runtime actions.",
      };
    case 23:
      return {
        kicker: "Integrations",
        description:
          "External plugin endpoints, event subscriptions, and plugin runtime health.",
      };
    case 26:
      return {
        kicker: "Integrations",
        description:
          "Pair phone or desktop companions for AgentArk notifications and approval prompts.",
      };
    default:
      return {
        kicker: "Settings",
        description: "Operator controls and system defaults for this AgentArk OS.",
      };
  }
}
