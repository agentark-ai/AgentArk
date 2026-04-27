import { lazy } from "react";

export const CompanionDevicesPanel = lazy(() =>
  import("../CompanionDevicesPanel").then((module) => ({
    default: module.CompanionDevicesPanel,
  })),
);

export const IntegrationQuickstartPanel = lazy(() =>
  import("../IntegrationQuickstartPanel").then((module) => ({
    default: module.IntegrationQuickstartPanel,
  })),
);

export const IntegrationsPanel = lazy(() =>
  import("../IntegrationsPanel").then((module) => ({
    default: module.IntegrationsPanel,
  })),
);

export const MediaSettingsPanel = lazy(() =>
  import("./MediaSettingsPanel").then((module) => ({
    default: module.MediaSettingsPanel,
  })),
);

export const MemoryPage = lazy(() => import("./MemoryPage"));

export const ObservabilityPanel = lazy(() =>
  import("../ObservabilityPanel").then((module) => ({
    default: module.ObservabilityPanel,
  })),
);

export const PluginSdkPanel = lazy(() =>
  import("../PluginSdkPanel").then((module) => ({
    default: module.PluginSdkPanel,
  })),
);

export const TracePage = lazy(() => import("./TracePage"));

export const WebhooksPanel = lazy(() =>
  import("../WebhooksPanel").then((module) => ({
    default: module.WebhooksPanel,
  })),
);
