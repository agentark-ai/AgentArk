import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import {
  CORE_SETTINGS_STALE_TIME_MS,
  fetchAvailableMessagingChannels,
  fetchModels,
  fetchSettings,
  fetchSettingsMedia,
  SETTINGS_QUERY_KEYS,
} from "./settingsData";
import { asRecord, errMessage, pickRecords, str, toBool } from "./pageHelpers";

type SettingsOverviewTabProps = {
  autoRefresh: boolean;
  onOpenFullSettings: () => void;
  onNavigateTab: (tab: number) => void;
};

type StatusTone = "success" | "warning" | "muted";

function StatusCard({
  label,
  status,
  detail,
  tone,
}: {
  label: string;
  status: string;
  detail?: string;
  tone: StatusTone;
}) {
  return (
    <Box
      sx={{
        p: 1.45,
        borderRadius: 2,
        border: "1px solid",
        borderColor:
          tone === "success"
            ? "var(--ui-rgba-130-247-193-200)"
            : tone === "warning"
              ? "var(--ui-rgba-255-180-50-240)"
              : "var(--ui-rgba-255-255-255-080)",
        background:
          tone === "success"
            ? "var(--ui-rgba-130-247-193-060)"
            : tone === "warning"
              ? "var(--ui-rgba-255-180-50-080)"
              : "var(--ui-rgba-255-255-255-030)",
        display: "flex",
        alignItems: "flex-start",
        gap: 1,
        minWidth: 0,
      }}
    >
      <Box
        sx={{
          width: 8,
          height: 8,
          mt: 0.5,
          borderRadius: "50%",
          flexShrink: 0,
          background:
            tone === "success"
              ? "#82f7c1"
              : tone === "warning"
                ? "var(--ui-rgba-255-180-50-900)"
                : "var(--ui-rgba-255-255-255-180)",
          boxShadow:
            tone === "success"
              ? "0 0 6px var(--ui-rgba-130-247-193-320)"
              : tone === "warning"
                ? "0 0 6px var(--ui-rgba-255-180-50-350)"
                : "none",
        }}
      />
      <Stack spacing={0.15} sx={{ minWidth: 0 }}>
        <Typography
          variant="caption"
          sx={{
            color: "var(--ui-rgba-171-176-184-620)",
            fontSize: "0.68rem",
            lineHeight: 1.2,
          }}
        >
          {label}
        </Typography>
        <Typography
          variant="body2"
          sx={{
            fontWeight: 600,
            fontSize: "0.8rem",
            color:
              tone === "muted"
                ? "var(--ui-rgba-155-159-169-720)"
                : "var(--ui-rgba-244-245-247-920)",
          }}
        >
          {status}
        </Typography>
        {detail ? (
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              display: "block",
              lineHeight: 1.35,
            }}
          >
            {detail}
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
}

export function SettingsOverviewTab({
  autoRefresh,
  onOpenFullSettings,
  onNavigateTab,
}: SettingsOverviewTabProps) {
  const settingsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.settings,
    queryFn: fetchSettings,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    refetchInterval: false,
    refetchOnWindowFocus: false,
  });
  const availableChannelsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.availableMessagingChannels,
    queryFn: fetchAvailableMessagingChannels,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    refetchInterval: autoRefresh ? 8000 : false,
  });
  const mediaQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.media,
    queryFn: fetchSettingsMedia,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    refetchInterval: autoRefresh ? 8000 : false,
  });
  const modelsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.models,
    queryFn: fetchModels,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    refetchInterval: autoRefresh ? 8000 : false,
  });

  const settings = asRecord(settingsQ.data);
  const models = pickRecords(asRecord(modelsQ.data).models);
  const media = asRecord(mediaQ.data);
  const availableDeliveryChannels = useMemo(() => {
    const channels = pickRecords(asRecord(availableChannelsQ.data).channels);
    const rows = channels
      .filter((channel) => toBool(channel.configured))
      .map((channel) => ({
        id: str(channel.id, "").trim(),
        name: str(channel.display_name, "").trim(),
      }))
      .filter((channel) => channel.id.length > 0);
    const seen = new Set<string>();
    return rows.filter((channel) => {
      if (seen.has(channel.id)) return false;
      seen.add(channel.id);
      return true;
    });
  }, [availableChannelsQ.data]);

  const hasPrimaryApiKey = toBool(settings.has_api_key);
  const hasFallbackApiKey = toBool(settings.has_fallback_api_key);
  const hasTelegramToken = toBool(settings.has_telegram_token);
  const telegramReady = toBool(settings.telegram_delivery_ready);
  const hasSlackBotToken = toBool(settings.has_slack_bot_token);
  const hasSlackSigningSecret = toBool(settings.has_slack_signing_secret);
  const slackReady = toBool(settings.slack_delivery_ready);
  const hasDiscordBotToken = toBool(settings.has_discord_bot_token);
  const discordReady = toBool(settings.discord_delivery_ready);
  const hasMatrixAccessToken = toBool(settings.has_matrix_access_token);
  const matrixReady = toBool(settings.matrix_delivery_ready);
  const hasTeamsAccessToken = toBool(settings.has_teams_access_token);
  const teamsReady = toBool(settings.teams_delivery_ready);
  const whatsappReady =
    toBool(settings.whatsapp_enabled) &&
    str(settings.whatsapp_bridge_url, "").trim().length > 0 &&
    str(settings.whatsapp_default_to, "").trim().length > 0;

  const configuredModels = models.filter((slot) =>
    str(slot.model, "").trim(),
  );
  const enabledModels = configuredModels.filter((slot) => toBool(slot.enabled));
  const searchProviderCount = [
    settings.tavily_api_key_present,
    settings.brave_search_api_key_present,
    settings.serpapi_api_key_present,
  ].filter(toBool).length;
  const mediaConfigured = [
    media.default_image_provider,
    media.default_video_provider,
    media.openai_api_key_present,
    media.replicate_api_key_present,
  ].some((value) => str(value, "").trim() || toBool(value));

  const setupMissing =
    settingsQ.isSuccess && modelsQ.isSuccess && configuredModels.length === 0;
  const loading =
    settingsQ.isLoading ||
    modelsQ.isLoading ||
    mediaQ.isLoading ||
    availableChannelsQ.isLoading;
  const error =
    settingsQ.error || modelsQ.error || mediaQ.error || availableChannelsQ.error;

  const deliveryStatus = [
    {
      label: "Telegram",
      configured: hasTelegramToken,
      ready: telegramReady,
      missing: "Not configured",
      waiting: "Needs bound recipient",
    },
    {
      label: "Slack",
      configured: hasSlackBotToken && hasSlackSigningSecret,
      ready: slackReady,
      missing: "Not configured",
      waiting: "Needs target",
    },
    {
      label: "Discord",
      configured: hasDiscordBotToken,
      ready: discordReady,
      missing: "Not configured",
      waiting: "Needs scope",
    },
    {
      label: "Matrix",
      configured: hasMatrixAccessToken,
      ready: matrixReady,
      missing: "Not configured",
      waiting: "Needs room",
    },
    {
      label: "Teams",
      configured: hasTeamsAccessToken,
      ready: teamsReady,
      missing: "Not configured",
      waiting: "Needs target",
    },
    {
      label: "WhatsApp",
      configured: whatsappReady,
      ready: whatsappReady,
      missing: "Not configured",
      waiting: "Needs recipient",
    },
  ];

  return (
    <Stack spacing={1.5}>
      {setupMissing ? (
        <Alert severity="warning">
          Setup required: configure at least one model in the Models tab, then
          Save Settings.
        </Alert>
      ) : null}
      {error ? <Alert severity="error">{errMessage(error)}</Alert> : null}
      {loading ? (
        <Typography variant="body2" sx={{ color: "text.secondary" }}>
          Loading settings...
        </Typography>
      ) : null}

      <Box
        className="list-shell"
        sx={{
          p: 1.6,
          display: "grid",
          gridTemplateColumns: { xs: "1fr", md: "1.08fr 0.92fr" },
          gap: 1.4,
          alignItems: "stretch",
        }}
      >
        <Stack spacing={1.2}>
          <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
            Readiness
          </Typography>
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: { xs: "1fr", sm: "1fr 1fr" },
              gap: 1,
            }}
          >
            <StatusCard
              label="Primary model"
              tone={hasPrimaryApiKey || enabledModels.length > 0 ? "success" : "warning"}
              status={
                enabledModels.length > 0
                  ? `${enabledModels.length} enabled`
                  : hasPrimaryApiKey
                    ? "Key ready"
                    : "Needs model"
              }
              detail={
                configuredModels.length > 0
                  ? `${configuredModels.length} configured`
                  : "No configured model slots"
              }
            />
            <StatusCard
              label="Fallback API key"
              tone={hasFallbackApiKey ? "success" : "muted"}
              status={hasFallbackApiKey ? "Connected" : "Not configured"}
            />
            <StatusCard
              label="Search"
              tone={searchProviderCount > 0 ? "success" : "muted"}
              status={
                searchProviderCount > 0
                  ? `${searchProviderCount} provider${searchProviderCount === 1 ? "" : "s"}`
                  : "Not configured"
              }
            />
            <StatusCard
              label="Media"
              tone={mediaConfigured ? "success" : "muted"}
              status={mediaConfigured ? "Configured" : "Defaults only"}
            />
          </Box>
        </Stack>

        <Stack spacing={1.2}>
          <Stack
            direction="row"
            sx={{ justifyContent: "space-between", alignItems: "center" }}
          >
            <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
              Delivery
            </Typography>
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              {availableDeliveryChannels.length} active
            </Typography>
          </Stack>
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: { xs: "1fr", sm: "1fr 1fr" },
              gap: 1,
            }}
          >
            {deliveryStatus.map((item) => (
              <StatusCard
                key={item.label}
                label={item.label}
                tone={
                  !item.configured ? "muted" : item.ready ? "success" : "warning"
                }
                status={
                  !item.configured
                    ? item.missing
                    : item.ready
                      ? "Ready"
                      : item.waiting
                }
              />
            ))}
          </Box>
        </Stack>
      </Box>

      <Box
        className="list-shell"
        sx={{
          p: 1.35,
          display: "flex",
          gap: 1,
          flexWrap: "wrap",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Typography variant="subtitle2" sx={{ color: "text.primary" }}>
          Full editor
        </Typography>
        <Stack direction="row" spacing={0.8} useFlexGap sx={{ flexWrap: "wrap" }}>
          <Button size="small" variant="outlined" onClick={() => onNavigateTab(1)}>
            Models
          </Button>
          <Button size="small" variant="outlined" onClick={() => onNavigateTab(20)}>
            Integrations
          </Button>
          <Button size="small" variant="contained" onClick={onOpenFullSettings}>
            Edit setup
          </Button>
        </Stack>
      </Box>
    </Stack>
  );
}
