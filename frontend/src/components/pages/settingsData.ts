import type { QueryClient } from "@tanstack/react-query";
import { api } from "../../api/client";

export const SETTINGS_QUERY_KEYS = {
  settings: ["settings"] as const,
  availableMessagingChannels: ["available-messaging-channels"] as const,
  media: ["settings-media"] as const,
  models: ["models"] as const,
  updateStatus: ["settings-update-status"] as const,
  apiKey: ["settings-api-key"] as const,
  autonomySettings: ["settings-autonomy-settings"] as const,
  evolution: ["settings-evolution"] as const,
  sentinel: ["sentinel-settings"] as const,
  tunnelStatus: ["tunnel-status"] as const,
  tunnelProviders: ["tunnel-providers"] as const,
  securityStatus: ["security-status"] as const,
  securityAbuseReviews: ["security-abuse-reviews"] as const,
  observabilityLogs: ["settings-observability-logs"] as const,
  secrets: ["settings-secrets"] as const,
  arkPulseLog: ["arkpulse-log"] as const,
};

export const CORE_SETTINGS_STALE_TIME_MS = 30_000;
export const SETTINGS_BACKGROUND_STALE_TIME_MS = 60_000;
export const SETTINGS_CACHE_GC_TIME_MS = 15 * 60_000;

export const fetchSettings = () => api.rawGet("/settings");
export const fetchAvailableMessagingChannels = () =>
  api.rawGet("/channels/available");
export const fetchSettingsMedia = () => api.rawGet("/settings/media");
export const fetchModels = () => api.rawGet("/models");
export const fetchSettingsUpdateStatus = () => api.getStatus();
export const fetchSettingsApiKey = () => api.rawGet("/settings/api-key");
export const fetchSettingsAutonomy = () => api.rawGet("/autonomy/settings");
export const fetchSettingsEvolution = () => api.rawGet("/settings/evolution");
export const fetchSettingsSentinel = () =>
  api.rawGet("/autonomy/sentinel/settings");
export const fetchTunnelStatus = () => api.rawGet("/tunnel/status");
export const fetchTunnelProviders = () => api.rawGet("/tunnel/providers");
export const fetchSecurityStatus = () => api.rawGet("/security/status");
export const fetchSecurityAbuseReviews = () =>
  api.rawGet("/security/abuse-reviews");
export const fetchSettingsObservabilityLogs = () =>
  api.rawGet("/settings/observability/logs?limit=40");
export const fetchSettingsSecrets = () => api.rawGet("/settings/secrets");
export const fetchArkPulseLog = () => api.rawGet("/arkpulse?limit=40");

export function prefetchCoreSettingsData(queryClient: QueryClient): void {
  void Promise.allSettled([
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.settings,
      queryFn: fetchSettings,
      staleTime: CORE_SETTINGS_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.availableMessagingChannels,
      queryFn: fetchAvailableMessagingChannels,
      staleTime: CORE_SETTINGS_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.media,
      queryFn: fetchSettingsMedia,
      staleTime: CORE_SETTINGS_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.models,
      queryFn: fetchModels,
      staleTime: CORE_SETTINGS_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
  ]);
}

export function prefetchSettingsPageData(queryClient: QueryClient): void {
  prefetchCoreSettingsData(queryClient);
  void Promise.allSettled([
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.updateStatus,
      queryFn: fetchSettingsUpdateStatus,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.apiKey,
      queryFn: fetchSettingsApiKey,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.autonomySettings,
      queryFn: fetchSettingsAutonomy,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.evolution,
      queryFn: fetchSettingsEvolution,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.sentinel,
      queryFn: fetchSettingsSentinel,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.tunnelStatus,
      queryFn: fetchTunnelStatus,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.tunnelProviders,
      queryFn: fetchTunnelProviders,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.securityStatus,
      queryFn: fetchSecurityStatus,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.securityAbuseReviews,
      queryFn: fetchSecurityAbuseReviews,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.observabilityLogs,
      queryFn: fetchSettingsObservabilityLogs,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.secrets,
      queryFn: fetchSettingsSecrets,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
    queryClient.prefetchQuery({
      queryKey: SETTINGS_QUERY_KEYS.arkPulseLog,
      queryFn: fetchArkPulseLog,
      staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
      gcTime: SETTINGS_CACHE_GC_TIME_MS,
    }),
  ]);
}
