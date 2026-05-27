import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Autocomplete,
  Box,
  Button,
  ButtonBase,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import ErrorOutlineRoundedIcon from "@mui/icons-material/ErrorOutlineRounded";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { api, apiUrl } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  getTunnelAccessMeta,
  getTunnelPanelPasswordPrompt,
  getTunnelPanelResumeMessage,
  getTunnelPanelStartMessage,
  getTunnelPanelStartingMessage,
  getTunnelProviderHelp,
} from "../../lib/tunnelAccess";
import {
  detectLocalTimeZone,
  formatUiDateTime,
  getSupportedUiTimeZones,
  setUiTimeZoneOverride,
} from "../../lib/dateFormat";
import type {
  PulseRemediationSpec,
  PulseRunFixRequest,
} from "../../types";
import {
  asRecord,
  errMessage,
  isRecord,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import {
  humanTs,
  KeyValuePanel,
} from "./workspaceUiBits";
import {
  DEVELOPER_MODE_EVENT,
  getDeveloperModeEnabled,
  OLLAMA_DEFAULT_BASE_URL,
  OPENROUTER_DEFAULT_BASE_URL,
  REFRESH_MS,
  setDeveloperModeEnabled,
} from "./workspaceCore";
import {
  arkPulseManualFollowupText,
  arkPulseRemediationFootnote,
  arkPulseRunActionLabel,
  collapseInlineWhitespace,
  describeArkPulseRemediation,
  formatDurationFromSeconds,
  formatTimestampForHumans,
  formatTraceDuration,
  getArkPulseFixText,
  getRunnableArkPulseRemediation,
  isUserActionableDoctorFinding,
  looksLikeIsoTimestamp,
  MODEL_FALLBACKS_BY_PROVIDER,
  parseArkPulseRemediationSpec,
  SEARCH_API_PROVIDER_OPTIONS,
  SEARCH_PROVIDER_OPTIONS,
  titleCaseLabel,
  truncateUiText,
} from "./settingsPageHelpers";
import {
  IntegrationQuickstartPanel,
  IntegrationsPanel,
  MemoryPage,
  TracePage,
} from "./settingsLazyPanels";
// Static imports for the small settings panels — bundling them with
// SettingsPageFull eliminates the inner Suspense roundtrip when switching
// tabs inside the Settings dialog. Tabs are then driven only by their
// data queries, which are prefetched in parallel via prefetchSettingsTabData.
import { CompanionDevicesPanel } from "../CompanionDevicesPanel";
import { MediaSettingsPanel } from "./MediaSettingsPanel";
import { ObservabilityPanel } from "../ObservabilityPanel";
import { PluginSdkPanel } from "../PluginSdkPanel";
import { SettingsAdvancedPanel } from "./SettingsAdvancedPanel";
import { SettingsDataLifecyclePanel } from "./SettingsDataLifecyclePanel";
import { SettingsModelsPanel } from "./SettingsModelsPanel";
import { SettingsSecurityPanel } from "./SettingsSecurityPanel";
import { SettingsUpdatesPanel } from "./SettingsUpdatesPanel";
import { WebhooksPanel } from "../WebhooksPanel";
import {
  SettingsInlineCard,
  SettingsSectionIntro,
  WorkspaceLazyPanel,
  type SettingsInlineCardProps,
  type SettingsSectionIntroArgs,
} from "./settingsLayout";
import {
  getSelectedSettingsNav,
  SettingsNavigation,
} from "./settingsNavigation";
import {
  CORE_SETTINGS_STALE_TIME_MS,
  SETTINGS_BACKGROUND_STALE_TIME_MS,
  fetchArkPulseLog,
  fetchAvailableMessagingChannels,
  fetchSecurityAbuseReviews,
  fetchSecurityStatus,
  fetchSettings,
  fetchSettingsApiKey,
  fetchSettingsAutonomy,
  fetchSettingsEvolution,
  fetchSettingsMedia,
  fetchSettingsObservabilityLogs,
  fetchSettingsSecrets,
  fetchSettingsSentinel,
  fetchSettingsUpdateStatus,
  fetchTunnelProviders,
  fetchTunnelStatus,
  modelsPayloadFromSettings,
  prefetchSettingsTabData,
  SETTINGS_CACHE_GC_TIME_MS,
  SETTINGS_QUERY_KEYS,
} from "./settingsData";
import {
  getSettingsPageMeta,
  getSettingsTabLoadingMessage,
  normalizeSettingsTab,
  resolveInitialSettingsTab,
  settingsTabSupportsSave,
  type SettingsPageProps,
} from "./settingsMeta";
import { preloadSettingsTab } from "./workspacePreload";

const RESTART_NOTICE_DURATION_MS = 10_000;
const UPDATE_NOTICE_DURATION_MS = 120_000;
const DEFAULT_READINESS_POLICY_DRAFT: Record<string, string> = {
  min_review_samples: "3",
  min_auto_samples: "8",
  min_review_success_rate_pct: "66",
  min_auto_success_rate_pct: "85",
  max_review_correction_rate_pct: "34",
  max_auto_correction_rate_pct: "10",
  min_candidate_review_confidence_pct: "70",
  max_review_trust_score: "50",
  max_auto_trust_score: "25",
};

function percentDraft(value: unknown, fallback: string): string {
  const raw = typeof value === "number" && Number.isFinite(value) ? value : NaN;
  if (!Number.isFinite(raw)) return fallback;
  return String(Math.round(raw * 100));
}

function readinessPolicyToDraft(policy: JsonRecord): Record<string, string> {
  return {
    min_review_samples: String(
      Math.round(
        num(
          policy.min_review_samples,
          Number(DEFAULT_READINESS_POLICY_DRAFT.min_review_samples),
        ),
      ),
    ),
    min_auto_samples: String(
      Math.round(
        num(
          policy.min_auto_samples,
          Number(DEFAULT_READINESS_POLICY_DRAFT.min_auto_samples),
        ),
      ),
    ),
    min_review_success_rate_pct: percentDraft(
      policy.min_review_success_rate,
      DEFAULT_READINESS_POLICY_DRAFT.min_review_success_rate_pct,
    ),
    min_auto_success_rate_pct: percentDraft(
      policy.min_auto_success_rate,
      DEFAULT_READINESS_POLICY_DRAFT.min_auto_success_rate_pct,
    ),
    max_review_correction_rate_pct: percentDraft(
      policy.max_review_correction_rate,
      DEFAULT_READINESS_POLICY_DRAFT.max_review_correction_rate_pct,
    ),
    max_auto_correction_rate_pct: percentDraft(
      policy.max_auto_correction_rate,
      DEFAULT_READINESS_POLICY_DRAFT.max_auto_correction_rate_pct,
    ),
    min_candidate_review_confidence_pct: percentDraft(
      policy.min_candidate_review_confidence,
      DEFAULT_READINESS_POLICY_DRAFT.min_candidate_review_confidence_pct,
    ),
    max_review_trust_score: String(
      Math.round(
        num(
          policy.max_review_trust_score,
          Number(DEFAULT_READINESS_POLICY_DRAFT.max_review_trust_score),
        ),
      ),
    ),
    max_auto_trust_score: String(
      Math.round(
        num(
          policy.max_auto_trust_score,
          Number(DEFAULT_READINESS_POLICY_DRAFT.max_auto_trust_score),
        ),
      ),
    ),
  };
}

function parseReadinessPolicyDraft(draft: Record<string, string>): JsonRecord {
  const parseWhole = (key: string, min: number, max: number) => {
    const parsed = Number(draft[key]);
    if (!Number.isFinite(parsed)) {
      throw new Error("Readiness thresholds must be numbers.");
    }
    return Math.round(Math.min(max, Math.max(min, parsed)));
  };
  const parsePercent = (key: string) => parseWhole(key, 0, 100) / 100;
  return {
    version: "readiness-policy-v1",
    min_review_samples: parseWhole("min_review_samples", 1, 10000),
    min_auto_samples: parseWhole("min_auto_samples", 1, 50000),
    min_review_success_rate: parsePercent("min_review_success_rate_pct"),
    min_auto_success_rate: parsePercent("min_auto_success_rate_pct"),
    max_review_correction_rate: parsePercent("max_review_correction_rate_pct"),
    max_auto_correction_rate: parsePercent("max_auto_correction_rate_pct"),
    min_candidate_review_confidence: parsePercent(
      "min_candidate_review_confidence_pct",
    ),
    max_review_trust_score: parseWhole("max_review_trust_score", 0, 100),
    max_auto_trust_score: parseWhole("max_auto_trust_score", 0, 100),
  };
}
const AUTO_APPROVE_BLOCKED_ACTIONS = [
  "shell",
  "bash",
  "code_execute",
  "file_write",
  "file_delete",
  "file_move",
] as const;

type RestartNoticeState = {
  text: string;
  durationMs: number;
  etaLabel: string;
};

type PulseInlineResult = {
  severity: "success" | "error";
  message: string;
  output: string;
  timestamp: string;
};

type PasswordDialogMode = "set" | "change" | "remove";

function settingsUiEmbeddingsProvider(value: string): "local-hf" | "disabled" {
  const normalized = value.trim().toLowerCase();
  return normalized === "disabled" || normalized === "none" || normalized === "off"
    ? "disabled"
    : "local-hf";
}

function embeddingsProviderHiddenFromSettingsUi(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return (
    normalized.length > 0 &&
    normalized !== "local-hf" &&
    normalized !== "local_hf" &&
    normalized !== "disabled" &&
    normalized !== "none" &&
    normalized !== "off"
  );
}

export default function SettingsPage({
  autoRefresh,
  initialTab,
  hideSettingsNav,
  standaloneSurface,
}: SettingsPageProps) {
  const LOCAL_EMBEDDINGS_MODEL = "BAAI/bge-small-en-v1.5";
  const queryClient = useQueryClient();
  const initialResolvedTab = resolveInitialSettingsTab(initialTab);
  const [tab, setTab] = useState(() => initialResolvedTab);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [restartNotice, setRestartNotice] = useState<RestartNoticeState | null>(
    null,
  );
  const [restartDialogOpen, setRestartDialogOpen] = useState(false);
  const [autonomyPauseDialogOpen, setAutonomyPauseDialogOpen] = useState(false);
  const [selfEvolveDisableDialogOpen, setSelfEvolveDisableDialogOpen] =
    useState(false);
  const [readinessPolicyDraft, setReadinessPolicyDraft] = useState<
    Record<string, string>
  >(DEFAULT_READINESS_POLICY_DRAFT);
  const [sentinelDisableDialogOpen, setSentinelDisableDialogOpen] =
    useState(false);
  const [sentinelInAppDisableDialogOpen, setSentinelInAppDisableDialogOpen] =
    useState(false);
  const [
    rotateInternalCredentialsDialogOpen,
    setRotateInternalCredentialsDialogOpen,
  ] = useState(false);
  const [
    rotateInternalCredentialsAccepted,
    setRotateInternalCredentialsAccepted,
  ] = useState(false);
  const [modelConnectivityWarning, setModelConnectivityWarning] = useState<
    string | null
  >(null);
  const [initialized, setInitialized] = useState(false);
  const [apiKeyRevealed, setApiKeyRevealed] = useState(false);
  const [apiKeyNowMs, setApiKeyNowMs] = useState(() => Date.now());
  const [secCurrentPassword, setSecCurrentPassword] = useState("");
  const [secNewPassword, setSecNewPassword] = useState("");
  const [secConfirmPassword, setSecConfirmPassword] = useState("");
  const [showPasswordInputs, setShowPasswordInputs] = useState(false);
  const [passwordDialogMode, setPasswordDialogMode] =
    useState<PasswordDialogMode | null>(null);
  const [vaultPassword, setVaultPassword] = useState("");
  const [vaultEditorOpen, setVaultEditorOpen] = useState(false);
  const [vaultEditorKey, setVaultEditorKey] = useState("");
  const [vaultEditorValue, setVaultEditorValue] = useState("");
  const [showVaultSecretValue, setShowVaultSecretValue] = useState(false);
  const [selectedPulseEvent, setSelectedPulseEvent] =
    useState<JsonRecord | null>(null);
  const [activePulseFixId, setActivePulseFixId] = useState<string | null>(null);
  const [pulseFixResultsById, setPulseFixResultsById] = useState<
    Record<string, PulseInlineResult>
  >({});
  const [pulsePollState, setPulsePollState] = useState<{
    baselineEventId: string;
    deadlineAt: number;
  } | null>(null);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(
    getDeveloperModeEnabled,
  );
  const [savedDeveloperModeEnabled, setSavedDeveloperModeEnabledState] =
    useState(getDeveloperModeEnabled);
  const [tunnelSelectedProviderId, setTunnelSelectedProviderId] = useState("");
  const [tunnelDraftValues, setTunnelDraftValues] = useState<
    Record<string, string>
  >({});
  const [showTunnelAdvanced, setShowTunnelAdvanced] = useState(false);
  const [securityVaultRequested, setSecurityVaultRequested] = useState(false);
  const [tunnelSetupChecks, setTunnelSetupChecks] = useState<JsonRecord[]>([]);
  const [tunnelPanelNotice, setTunnelPanelNotice] = useState<{
    severity: "success" | "info" | "warning";
    text: string;
  } | null>(null);
  const [resumeTunnelStartAfterPassword, setResumeTunnelStartAfterPassword] =
    useState(false);
  const modelTabActive = tab === 1;
  const mediaTabActive = tab === 3;
  const securityTabActive = tab === 4;
  const advancedTabActive = tab === 5;
  const observabilityTabActive = tab === 6;
  const standalonePulse = standaloneSurface === "arkpulse";
  const pulseTabActive = standalonePulse || tab === 9;
  const updatesTabActive = tab === 25;
  const setupTabActive = tab === 0;
  const channelsTabActive = tab === 20;
  const needsAvailableChannels = setupTabActive || channelsTabActive;
  const needsMediaSettings = setupTabActive || mediaTabActive;
  const needsModelSettings = setupTabActive || modelTabActive;

  const changeSettingsTab = (nextTabRaw: number) => {
    const nextTab = normalizeSettingsTab(nextTabRaw);
    preloadSettingsTab(nextTab);
    if (!standalonePulse) {
      prefetchSettingsTabData(queryClient, nextTab);
    }
    setTab((current) => (current === nextTab ? current : nextTab));
  };

  useEffect(() => {
    const nextTab = resolveInitialSettingsTab(initialTab);
    setTab((current) => (current === nextTab ? current : nextTab));
  }, [initialTab]);

  useEffect(() => {
    preloadSettingsTab(tab);
    if (!standalonePulse) {
      prefetchSettingsTabData(queryClient, tab);
    }
  }, [queryClient, standalonePulse, tab]);

  useEffect(() => {
    const refreshDeveloperMode = () => {
      const next = getDeveloperModeEnabled();
      setDeveloperModeEnabledState(next);
      setSavedDeveloperModeEnabledState(next);
    };
    window.addEventListener(
      DEVELOPER_MODE_EVENT,
      refreshDeveloperMode as EventListener,
    );
    window.addEventListener("storage", refreshDeveloperMode);
    return () => {
      window.removeEventListener(
        DEVELOPER_MODE_EVENT,
        refreshDeveloperMode as EventListener,
      );
      window.removeEventListener("storage", refreshDeveloperMode);
    };
  }, []);

  useEffect(() => {
    if (!success) return;
    const timer = window.setTimeout(() => setSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [success]);

  useEffect(() => {
    if (!restartNotice) return;
    const timer = window.setTimeout(
      () => setRestartNotice(null),
      restartNotice.durationMs,
    );
    return () => window.clearTimeout(timer);
  }, [restartNotice]);

  async function monitorRestartRecovery(
    timeoutMs = RESTART_NOTICE_DURATION_MS,
  ) {
    const deadlineAt = Date.now() + timeoutMs;
    const minimumVisibleUntil = Date.now() + 2000;
    let sawUnavailable = false;
    while (Date.now() < deadlineAt) {
      try {
        const response = await fetch(apiUrl("/health"), { cache: "no-store" });
        if (response.ok) {
          if (sawUnavailable || Date.now() >= minimumVisibleUntil) {
            window.location.reload();
            return;
          }
        } else {
          sawUnavailable = true;
        }
      } catch {
        sawUnavailable = true;
      }
      await new Promise<void>((resolve) => window.setTimeout(resolve, 1000));
    }
  }

  useEffect(() => {
    if (!advancedTabActive) return;
    const timer = window.setInterval(() => setApiKeyNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [advancedTabActive]);

  const settingsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.settings,
    queryFn: fetchSettings,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: false,
    refetchOnWindowFocus: false,
  });
  const availableChannelsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.availableMessagingChannels,
    queryFn: fetchAvailableMessagingChannels,
    enabled: needsAvailableChannels,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: needsAvailableChannels && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const mediaQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.media,
    queryFn: fetchSettingsMedia,
    enabled: needsMediaSettings,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: needsMediaSettings && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const updateStatusQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.updateStatus,
    queryFn: fetchSettingsUpdateStatus,
    enabled: updatesTabActive,
    staleTime: 60_000,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: false,
    refetchOnWindowFocus: false,
  });
  const apiKeyQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.apiKey,
    queryFn: fetchSettingsApiKey,
    enabled: advancedTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive ? 10000 : false,
    refetchIntervalInBackground: advancedTabActive,
  });
  const settingsAutonomyQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.autonomySettings,
    queryFn: fetchSettingsAutonomy,
    enabled: advancedTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const settingsEvolutionQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.evolution,
    queryFn: fetchSettingsEvolution,
    enabled: advancedTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const settingsSentinelQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.sentinel,
    queryFn: fetchSettingsSentinel,
    enabled: advancedTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const tunnelQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.tunnelStatus,
    queryFn: fetchTunnelStatus,
    enabled: securityTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const tunnelProvidersQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.tunnelProviders,
    queryFn: fetchTunnelProviders,
    enabled: securityTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const securityStatusQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.securityStatus,
    queryFn: fetchSecurityStatus,
    enabled: securityTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const abuseReviewsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.securityAbuseReviews,
    queryFn: fetchSecurityAbuseReviews,
    enabled: securityTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const abuseReviews = pickRecords(abuseReviewsQ.data, "reviews");
  const observabilityLogsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.observabilityLogs,
    queryFn: fetchSettingsObservabilityLogs,
    enabled: observabilityTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: observabilityTabActive && autoRefresh ? REFRESH_MS : false,
    refetchOnWindowFocus: false,
  });
  const vaultSecretsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.secrets,
    queryFn: fetchSettingsSecrets,
    enabled: securityTabActive && securityVaultRequested,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: false,
  });
  const pulseQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.arkPulseLog,
    queryFn: fetchArkPulseLog,
    enabled: pulseTabActive,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: pulseTabActive
      ? pulsePollState
        ? 2000
        : autoRefresh
          ? REFRESH_MS
          : false
      : false,
    refetchOnWindowFocus: false,
  });
  const settings = asRecord(settingsQ.data);
  const settingsModelsPayload = useMemo(
    () => modelsPayloadFromSettings(settings),
    [settings],
  );
  useEffect(() => {
    if (!settingsQ.isSuccess) return;
    queryClient.setQueryData(SETTINGS_QUERY_KEYS.models, settingsModelsPayload);
  }, [queryClient, settingsModelsPayload, settingsQ.isSuccess]);
  const modelsQ = {
    data: settingsModelsPayload,
    error: settingsQ.error,
    isError: settingsQ.isError,
    isFetching: settingsQ.isFetching,
    isLoading: settingsQ.isLoading,
    isSuccess: settingsQ.isSuccess,
  };
  const detectedTimezone = useMemo(() => detectLocalTimeZone(), []);
  const availableDeliveryChannels = useMemo(() => {
    const channels = pickRecords(asRecord(availableChannelsQ.data).channels);
    const rows = channels
      .filter((channel) => toBool(channel.configured))
      .map((channel) => ({
        id: str(channel.id, "").trim(),
        name: str(channel.display_name, "").trim(),
        sourceKind: str(asRecord(channel.source).kind, "").trim(),
      }))
      .filter((channel) => channel.id.length > 0);
    const seen = new Set<string>();
    return rows.filter((channel) => {
      if (seen.has(channel.id)) return false;
      seen.add(channel.id);
      return true;
    });
  }, [availableChannelsQ.data]);
  const deliveryChannelMenuLabel = (channel: {
    id: string;
    name: string;
    sourceKind: string;
  }) => {
    const label = channel.name || channel.id;
    if (channel.sourceKind === "custom_messaging_channel") {
      return `Custom: ${label}`;
    }
    if (channel.sourceKind === "extension_pack") {
      return `Extension: ${label}`;
    }
    return label;
  };
  const observabilitySettings = asRecord(settings.observability);
  const dataLifecycleSettings = asRecord(settings.data_lifecycle);
  const media = asRecord(mediaQ.data);
  const modelsPayload = settingsModelsPayload;
  const observabilityLogsPayload = asRecord(observabilityLogsQ.data);
  const observabilityLogs = pickRecords(observabilityLogsPayload, "logs");
  const observabilityIssues = Array.isArray(observabilityLogsPayload.issues)
    ? observabilityLogsPayload.issues.filter(
        (value): value is string =>
          typeof value === "string" && value.trim().length > 0,
      )
    : [];
  const configuredProviders = useMemo(() => {
    const raw = media.configured;
    if (!Array.isArray(raw)) return [];
    return raw.filter((x) => typeof x === "string") as string[];
  }, [media.configured]);

  const [form, setForm] = useState({
    bot_name: "AgentArk",
    personality: "friendly",
    timezone: "",
    language: "English",
    tone: "",
    email_format: "",
    daily_brief_enabled: false,
    daily_brief_time: "09:00",
    daily_brief_channel: "telegram",
    arkreflect_daily_digest_enabled: false,
    smart_routing: true,
    embeddings_provider: "local-hf",
    embeddings_model: LOCAL_EMBEDDINGS_MODEL,
    embeddings_base_url: "",
    embeddings_api_key: "",

    llm_provider: "",
    llm_model: "",
    llm_base_url: "",
    llm_api_key: "",

    llm_fallback_provider: "",
    llm_fallback_model: "",
    llm_fallback_base_url: "",
    llm_fallback_api_key: "",

    telegram_enabled: false,
    telegram_bot_token: "",
    telegram_allowed_users_csv: "",

    slack_enabled: false,
    slack_bot_token: "",
    slack_signing_secret: "",
    slack_api_base_url: "https://slack.com/api",
    slack_default_channel_id: "",
    slack_default_thread_ts: "",
    slack_workspace_id: "",
    slack_workspace_name: "",

    discord_enabled: false,
    discord_bot_token: "",
    discord_api_base_url: "https://discord.com/api/v10",
    discord_default_channel_id: "",
    discord_default_thread_id: "",
    discord_guild_id: "",
    discord_application_id: "",
    discord_webhook_url: "",

    matrix_enabled: false,
    matrix_homeserver_url: "",
    matrix_access_token: "",
    matrix_user_id: "",
    matrix_device_id: "",
    matrix_account_id: "",
    matrix_default_room_id: "",
    matrix_sync_timeout_ms: "30000",
    matrix_limit: "100",
    matrix_user_agent: "",

    teams_enabled: false,
    teams_service_url: "",
    teams_access_token: "",
    teams_bot_app_id: "",
    teams_bot_name: "",
    teams_tenant_id: "",
    teams_team_id: "",
    teams_channel_id: "",
    teams_chat_id: "",
    teams_graph_base_url: "https://graph.microsoft.com/v1.0",
    teams_delivery_mode: "auto",
    teams_timeout_secs: "15",
    teams_user_agent: "",

    whatsapp_enabled: false,
    whatsapp_mode: "baileys",
    whatsapp_access_token: "",
    whatsapp_app_secret: "",
    whatsapp_phone_number_id: "",
    whatsapp_verify_token: "agentark_verify",
    whatsapp_bridge_url: "http://127.0.0.1:8999",
    whatsapp_dm_policy: "pairing",
    whatsapp_allowed_numbers_csv: "",

    auto_approve_csv: "",
    model_privacy_default_mode: "default_redact",
    model_privacy_current_chat_pii_policy: "mask_chat_pii",
    model_privacy_request_scoped_sensitive_approval_enabled: true,

    default_image_provider: "",
    image_model: "",
    fallback_image_provider: "",
    default_video_provider: "",
    fallback_video_provider: "",
    media_provider_keys_json: "",
    media_key_replicate: "",
    media_key_fal: "",
    media_key_stability_ai: "",
    media_key_together: "",
    media_key_openai_dalle: "",
    media_key_google_gemini: "",
    media_key_runway: "",
    media_key_luma: "",
    media_base_url_replicate: "",
    media_base_url_fal: "",
    media_base_url_stability_ai: "",
    media_base_url_together: "",
    media_base_url_openai_dalle: "",
    media_base_url_openai_sora: "",
    media_base_url_google_gemini: "",
    media_base_url_google_veo: "",
    media_base_url_runway: "",
    media_base_url_luma: "",

    search_provider_order: [] as string[],
    search_serper_key: "",
    search_serper_editing: false,
    search_serper_clear: false,
    search_brave_key: "",
    search_brave_editing: false,
    search_brave_clear: false,
    search_exa_key: "",
    search_exa_editing: false,
    search_exa_clear: false,
    search_tavily_key: "",
    search_tavily_editing: false,
    search_tavily_clear: false,
    search_perplexity_key: "",
    search_perplexity_editing: false,
    search_perplexity_clear: false,
    search_firecrawl_key: "",
    search_firecrawl_editing: false,
    search_firecrawl_clear: false,
    search_searxng_base_url: "",

    data_lifecycle_cleanup_enabled: true,
    data_lifecycle_notifications_cleanup_enabled: true,
    data_lifecycle_logs_cleanup_enabled: true,
    data_lifecycle_notifications_retention_days: "7",
    data_lifecycle_notification_cleanup_interval_secs: "3600",
    data_lifecycle_execution_trace_retention_days: "30",
    data_lifecycle_execution_proof_retention_days: "30",
    data_lifecycle_operational_log_retention_days: "30",
    data_lifecycle_security_log_retention_days: "30",
    data_lifecycle_approval_log_retention_days: "30",
    data_lifecycle_swarm_delegation_retention_days: "30",
    data_lifecycle_llm_usage_retention_days: "30",
    data_lifecycle_terminal_task_retention_days: "90",
    data_lifecycle_execution_run_retention_days: "90",
    data_lifecycle_background_session_retention_days: "90",
    data_lifecycle_browser_session_retention_days: "30",
    data_lifecycle_automation_run_retention_days: "90",
    data_lifecycle_message_retention_days: "365",
    data_lifecycle_experience_run_retention_days: "90",
    data_lifecycle_experience_edge_retention_days: "90",
    data_lifecycle_learning_candidate_retention_days: "30",
    data_lifecycle_experience_item_retention_days: "0",
    data_lifecycle_procedural_pattern_retention_days: "0",
    data_lifecycle_recall_event_retention_days: "365",
    data_lifecycle_recall_test_retention_days: "365",
    data_lifecycle_housekeeping_interval_secs: "3600",
    data_lifecycle_security_cleanup_interval_days: "15",
    data_lifecycle_security_cleanup_idle_threshold_secs: "300",

    observability_enabled: false,
    observability_provider: "langtrace",
    observability_endpoint: "",
    observability_service_name: "agentark",
    observability_header_name: "x-api-key",
    observability_privacy_mode: "metadata_only",
    observability_auth_token: "",
  });
  const timezoneOptions = useMemo(() => {
    const zones = new Set(getSupportedUiTimeZones());
    if (form.timezone.trim()) zones.add(form.timezone.trim());
    if (detectedTimezone) zones.add(detectedTimezone);
    return Array.from(zones).sort((left, right) => {
      if (left === "UTC") return -1;
      if (right === "UTC") return 1;
      return left.localeCompare(right);
    });
  }, [detectedTimezone, form.timezone]);
  const timezoneHelperText = (() => {
    const saved = form.timezone.trim();
    if (!saved) {
      return detectedTimezone
        ? `Detected timezone: ${detectedTimezone}. Not correct? Choose one manually.`
        : "Choose an IANA timezone such as America/New_York.";
    }
    if (detectedTimezone && saved !== detectedTimezone) {
      return `Manual timezone override. This browser detected ${detectedTimezone}.`;
    }
    return detectedTimezone
      ? `Using detected timezone ${detectedTimezone}.`
      : "Saved timezone override.";
  })();
  const selectedDailyBriefDeliveryChannel = availableDeliveryChannels.find(
    (channel) => channel.id === form.daily_brief_channel,
  );
  const dailyBriefUsesUserDefinedExternalChannel =
    selectedDailyBriefDeliveryChannel?.sourceKind === "custom_messaging_channel" ||
    selectedDailyBriefDeliveryChannel?.sourceKind === "extension_pack" ||
    form.daily_brief_channel.startsWith("custom.") ||
    form.daily_brief_channel.startsWith("ext.");
  const [savedFormSnapshot, setSavedFormSnapshot] = useState("");

  function snapshotSettingsForm(value: typeof form): string {
    return JSON.stringify(value);
  }

  function snapshotObservabilityForm(value: typeof form): string {
    return JSON.stringify({
      observability_enabled: value.observability_enabled,
      observability_provider: value.observability_provider,
      observability_endpoint: value.observability_endpoint,
      observability_service_name: value.observability_service_name,
      observability_header_name: value.observability_header_name,
      observability_privacy_mode: value.observability_privacy_mode,
      observability_auth_token: value.observability_auth_token,
    });
  }

  function parseSavedSettingsSnapshot(): typeof form | null {
    if (!savedFormSnapshot.trim()) return null;
    try {
      return JSON.parse(savedFormSnapshot) as typeof form;
    } catch {
      return null;
    }
  }

  const settingsFormDirty =
    dirty && snapshotSettingsForm(form) !== savedFormSnapshot;
  const developerModeDirty =
    developerModeEnabled !== savedDeveloperModeEnabled;
  const effectiveDirty = settingsFormDirty || developerModeDirty;

  function setField<K extends keyof typeof form>(
    key: K,
    value: (typeof form)[K],
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
    setDirty(true);
    setSuccess(null);
  }

  function setSearchProviderDraft(
    provider: (typeof SEARCH_API_PROVIDER_OPTIONS)[number],
    updates: {
      key?: string;
      editing?: boolean;
      clear?: boolean;
    },
  ) {
    setForm((prev) => ({
      ...prev,
      ...(updates.key !== undefined
        ? { [provider.keyField]: updates.key }
        : {}),
      ...(updates.editing !== undefined
        ? { [provider.editingField]: updates.editing }
        : {}),
      ...(updates.clear !== undefined
        ? { [provider.clearField]: updates.clear }
        : {}),
    }));
    setDirty(true);
    setSuccess(null);
  }

  function parseCsvList(csv: string): string[] {
    return csv
      .split(/[,\\n]/g)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }

  function sanitizeAutoApproveList(items: string[]): string[] {
    const blocked = new Set<string>(AUTO_APPROVE_BLOCKED_ACTIONS);
    const seen = new Set<string>();
    const sanitized: string[] = [];
    for (const item of items) {
      const trimmed = item.trim();
      if (!trimmed || blocked.has(trimmed) || seen.has(trimmed)) continue;
      seen.add(trimmed);
      sanitized.push(trimmed);
    }
    return sanitized;
  }

  function findBlockedAutoApproveEntries(csv: string): string[] {
    const blocked = new Set<string>(AUTO_APPROVE_BLOCKED_ACTIONS);
    const seen = new Set<string>();
    const blockedEntries: string[] = [];
    for (const item of parseCsvList(csv)) {
      if (!blocked.has(item) || seen.has(item)) continue;
      seen.add(item);
      blockedEntries.push(item);
    }
    return blockedEntries;
  }

  function parseTelegramUsers(csv: string): number[] {
    const parts = parseCsvList(csv);
    const out: number[] = [];
    for (const p of parts) {
      const n = Number(p);
      if (!Number.isFinite(n))
        throw new Error(`Invalid Telegram user id: '${p}'`);
      out.push(n);
    }
    return out;
  }

  function parseMediaProvidersJson(input: string): Record<string, string> {
    const trimmed = input.trim();
    if (!trimmed) return {};
    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      throw new Error(
        "Media provider keys must be valid JSON (object mapping provider -> api_key).",
      );
    }
    if (!isRecord(parsed))
      throw new Error("Media provider keys must be a JSON object.");
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof v !== "string")
        throw new Error(`Media provider key for '${k}' must be a string.`);
      out[k] = v;
    }
    return out;
  }

  function parseNonNegativeInt(value: string, label: string): number {
    const trimmed = value.trim();
    if (!trimmed) throw new Error(`${label} is required.`);
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed) || parsed < 0 || !Number.isInteger(parsed)) {
      throw new Error(`${label} must be a whole number 0 or greater.`);
    }
    return Math.trunc(parsed);
  }

  function parsePositiveInt(value: string, label: string, minimum = 1): number {
    const parsed = parseNonNegativeInt(value, label);
    if (parsed < minimum) {
      throw new Error(`${label} must be at least ${minimum}.`);
    }
    return parsed;
  }

  function recordToStringMap(value: unknown): Record<string, string> {
    const raw = asRecord(value);
    const out: Record<string, string> = {};
    for (const [key, entry] of Object.entries(raw)) {
      if (entry === null || entry === undefined) continue;
      out[key] = String(entry);
    }
    return out;
  }

  function pickRecommendedTunnelProvider(
    providers: JsonRecord[],
  ): JsonRecord | null {
    const publicProviders = providers.filter(
      (provider) => !getTunnelAccessMeta(provider).isPrivate,
    );
    return (
      publicProviders.find(
        (provider) => toBool(provider.available) && toBool(provider.configured),
      ) ||
      publicProviders.find((provider) => toBool(provider.available)) ||
      providers.find(
        (provider) => toBool(provider.available) && toBool(provider.configured),
      ) ||
      providers.find((provider) => toBool(provider.available)) ||
      providers[0] ||
      null
    );
  }

  function syncTunnelDraftFromPayload(
    payloadLike: unknown,
    preferredProviderId?: string,
  ) {
    const payload = asRecord(payloadLike);
    const providers = pickRecords(payload, "providers");
    if (providers.length === 0) return;
    const explicitPreferred = preferredProviderId?.trim() || "";
    const runtimeActive = toBool(payload.active);
    const activeProviderId = str(payload.active_provider, "").trim();
    const selectedProviderId = str(payload.selected_provider, "").trim();
    const explicitlySelected = explicitPreferred
      ? providers.find(
          (provider) => str(provider.id, "").trim() === explicitPreferred,
        )
      : null;
    const activeProvider = activeProviderId
      ? providers.find(
          (provider) => str(provider.id, "").trim() === activeProviderId,
        )
      : null;
    const serverSelected = selectedProviderId
      ? providers.find(
          (provider) => str(provider.id, "").trim() === selectedProviderId,
        )
      : null;
    const serverSelectedAvailable =
      serverSelected && toBool(serverSelected.available)
        ? serverSelected
        : null;
    const selected =
      explicitlySelected ||
      (runtimeActive ? activeProvider : null) ||
      serverSelectedAvailable ||
      pickRecommendedTunnelProvider(providers);
    if (!selected) return;
    const nextId = str(selected.id, "").trim();
    if (!nextId) return;
    setTunnelSelectedProviderId(nextId);
    setTunnelDraftValues(recordToStringMap(asRecord(selected.config_values)));
  }

  function hydrateFromServer() {
    const serverEmbeddingsProvider = str(settings.embeddings_provider, "local-hf");
    const uiEmbeddingsProvider =
      settingsUiEmbeddingsProvider(serverEmbeddingsProvider);
    const hiddenExternalEmbeddingsProvider =
      embeddingsProviderHiddenFromSettingsUi(serverEmbeddingsProvider);
    const tgUsers = Array.isArray(settings.telegram_allowed_users)
      ? (settings.telegram_allowed_users as unknown[])
      : [];
    const waNums = Array.isArray(settings.whatsapp_allowed_numbers)
      ? (settings.whatsapp_allowed_numbers as unknown[])
      : [];
    const autoApprove = Array.isArray(settings.auto_approve)
      ? (settings.auto_approve as unknown[])
      : [];
    const nextForm = {
      ...form,
      bot_name: str(settings.bot_name, form.bot_name),
      personality: str(settings.personality, form.personality),
      timezone: str(settings.timezone, ""),
      language: str(settings.language, form.language),
      tone: str(settings.tone, form.tone),
      email_format: str(settings.email_format, form.email_format),
      daily_brief_enabled: toBool(settings.daily_brief_enabled),
      daily_brief_time: str(settings.daily_brief_time, "09:00"),
      daily_brief_channel: str(settings.daily_brief_channel, "telegram"),
      arkreflect_daily_digest_enabled: toBool(
        settings.arkreflect_daily_digest_enabled,
      ),
      smart_routing: toBool(settings.smart_routing),
      embeddings_provider: uiEmbeddingsProvider,
      embeddings_model: LOCAL_EMBEDDINGS_MODEL,
      embeddings_base_url: "",
      embeddings_api_key: "",

      llm_provider: str(settings.llm_provider, ""),
      llm_model: str(settings.llm_model, ""),
      llm_base_url: str(settings.llm_base_url, ""),
      llm_api_key: "",

      llm_fallback_provider: str(settings.llm_fallback_provider, ""),
      llm_fallback_model: str(settings.llm_fallback_model, ""),
      llm_fallback_base_url: str(settings.llm_fallback_base_url, ""),
      llm_fallback_api_key: "",

      telegram_enabled: toBool(settings.telegram_enabled),
      telegram_bot_token: "",
      telegram_allowed_users_csv: tgUsers
        .map((v) =>
          typeof v === "number" ? String(v) : typeof v === "string" ? v : "",
        )
        .filter((v) => v.trim().length > 0)
        .join(", "),

      slack_enabled: toBool(settings.slack_enabled),
      slack_bot_token: "",
      slack_signing_secret: "",
      slack_api_base_url: str(
        settings.slack_api_base_url,
        "https://slack.com/api",
      ),
      slack_default_channel_id: str(settings.slack_default_channel_id, ""),
      slack_default_thread_ts: str(settings.slack_default_thread_ts, ""),
      slack_workspace_id: str(settings.slack_workspace_id, ""),
      slack_workspace_name: str(settings.slack_workspace_name, ""),

      discord_enabled: toBool(settings.discord_enabled),
      discord_bot_token: "",
      discord_api_base_url: str(
        settings.discord_api_base_url,
        "https://discord.com/api/v10",
      ),
      discord_default_channel_id: str(settings.discord_default_channel_id, ""),
      discord_default_thread_id: str(settings.discord_default_thread_id, ""),
      discord_guild_id: str(settings.discord_guild_id, ""),
      discord_application_id: str(settings.discord_application_id, ""),
      discord_webhook_url: str(settings.discord_webhook_url, ""),

      matrix_enabled: toBool(settings.matrix_enabled),
      matrix_homeserver_url: str(settings.matrix_homeserver_url, ""),
      matrix_access_token: "",
      matrix_user_id: str(settings.matrix_user_id, ""),
      matrix_device_id: str(settings.matrix_device_id, ""),
      matrix_account_id: str(settings.matrix_account_id, ""),
      matrix_default_room_id: str(settings.matrix_default_room_id, ""),
      matrix_sync_timeout_ms: str(settings.matrix_sync_timeout_ms, "30000"),
      matrix_limit: str(settings.matrix_limit, "100"),
      matrix_user_agent: str(settings.matrix_user_agent, ""),

      teams_enabled: toBool(settings.teams_enabled),
      teams_service_url: str(settings.teams_service_url, ""),
      teams_access_token: "",
      teams_bot_app_id: str(settings.teams_bot_app_id, ""),
      teams_bot_name: str(settings.teams_bot_name, ""),
      teams_tenant_id: str(settings.teams_tenant_id, ""),
      teams_team_id: str(settings.teams_team_id, ""),
      teams_channel_id: str(settings.teams_channel_id, ""),
      teams_chat_id: str(settings.teams_chat_id, ""),
      teams_graph_base_url: str(
        settings.teams_graph_base_url,
        "https://graph.microsoft.com/v1.0",
      ),
      teams_delivery_mode: str(settings.teams_delivery_mode, "auto"),
      teams_timeout_secs: str(settings.teams_timeout_secs, "15"),
      teams_user_agent: str(settings.teams_user_agent, ""),

      whatsapp_enabled: toBool(settings.whatsapp_enabled),
      whatsapp_mode: str(settings.whatsapp_mode, "baileys"),
      whatsapp_access_token: "",
      whatsapp_app_secret: "",
      whatsapp_phone_number_id: str(settings.whatsapp_phone_number_id, ""),
      whatsapp_verify_token: str(
        settings.whatsapp_verify_token,
        "agentark_verify",
      ),
      whatsapp_bridge_url: str(
        settings.whatsapp_bridge_url,
        "http://127.0.0.1:8999",
      ),
      whatsapp_dm_policy: str(settings.whatsapp_dm_policy, "pairing"),
      whatsapp_allowed_numbers_csv: waNums
        .map((v) => (typeof v === "string" ? v : ""))
        .filter((v) => v.trim().length > 0)
        .join(", "),

      auto_approve_csv: autoApprove
        .map((v) => (typeof v === "string" ? v : ""))
        .filter((v) => v.trim().length > 0)
        .join(", "),
      model_privacy_default_mode: str(
        settings.default_model_input_mode,
        "default_redact",
      ),
      model_privacy_current_chat_pii_policy: str(
        settings.current_chat_pii_policy,
        "mask_chat_pii",
      ),
      model_privacy_request_scoped_sensitive_approval_enabled:
        settings.request_scoped_sensitive_approval_enabled == null
          ? true
          : toBool(settings.request_scoped_sensitive_approval_enabled),

      default_image_provider: str(
        media.default_image_provider ?? settings.default_image_provider,
        "",
      ),
      image_model: str(media.image_model ?? settings.image_model, ""),
      fallback_image_provider: str(
        media.fallback_image_provider ?? settings.fallback_image_provider,
        "",
      ),
      default_video_provider: str(
        media.default_video_provider ?? settings.default_video_provider,
        "",
      ),
      fallback_video_provider: str(
        media.fallback_video_provider ?? settings.fallback_video_provider,
        "",
      ),
      media_provider_keys_json: "",
      media_key_replicate: "",
      media_key_fal: "",
      media_key_stability_ai: "",
      media_key_together: "",
      media_key_openai_dalle: "",
      media_key_google_gemini: "",
      media_key_runway: "",
      media_key_luma: "",
      media_base_url_replicate: str(asRecord(media.provider_base_urls).replicate, ""),
      media_base_url_fal: str(asRecord(media.provider_base_urls).fal, ""),
      media_base_url_stability_ai: str(
        asRecord(media.provider_base_urls).stability_ai,
        "",
      ),
      media_base_url_together: str(asRecord(media.provider_base_urls).together, ""),
      media_base_url_openai_dalle: str(
        asRecord(media.provider_base_urls).openai_dalle,
        "",
      ),
      media_base_url_openai_sora: str(
        asRecord(media.provider_base_urls).openai_sora,
        "",
      ),
      media_base_url_google_gemini: str(
        asRecord(media.provider_base_urls).google_gemini,
        "",
      ),
      media_base_url_google_veo: str(
        asRecord(media.provider_base_urls).google_veo,
        "",
      ),
      media_base_url_runway: str(asRecord(media.provider_base_urls).runway, ""),
      media_base_url_luma: str(asRecord(media.provider_base_urls).luma, ""),

      search_provider_order: Array.isArray(settings.search_provider_order)
        ? settings.search_provider_order
            .filter((value): value is string => typeof value === "string")
            .map((value) => value.trim().toLowerCase())
            .filter((value) => value.length > 0)
        : [],
      search_serper_key: "",
      search_serper_editing: false,
      search_serper_clear: false,
      search_brave_key: "",
      search_brave_editing: false,
      search_brave_clear: false,
      search_exa_key: "",
      search_exa_editing: false,
      search_exa_clear: false,
      search_tavily_key: "",
      search_tavily_editing: false,
      search_tavily_clear: false,
      search_perplexity_key: "",
      search_perplexity_editing: false,
      search_perplexity_clear: false,
      search_firecrawl_key: "",
      search_firecrawl_editing: false,
      search_firecrawl_clear: false,
      search_searxng_base_url: str(settings.search_searxng_base_url, ""),

      data_lifecycle_cleanup_enabled:
        dataLifecycleSettings.cleanup_enabled == null
          ? true
          : toBool(dataLifecycleSettings.cleanup_enabled),
      data_lifecycle_notifications_cleanup_enabled:
        dataLifecycleSettings.notifications_cleanup_enabled == null
          ? true
          : toBool(dataLifecycleSettings.notifications_cleanup_enabled),
      data_lifecycle_logs_cleanup_enabled:
        dataLifecycleSettings.logs_cleanup_enabled == null
          ? true
          : toBool(dataLifecycleSettings.logs_cleanup_enabled),
      data_lifecycle_notifications_retention_days: str(
        dataLifecycleSettings.notifications_retention_days,
        "7",
      ),
      data_lifecycle_notification_cleanup_interval_secs: str(
        dataLifecycleSettings.notification_cleanup_interval_secs,
        "3600",
      ),
      data_lifecycle_execution_trace_retention_days: str(
        dataLifecycleSettings.execution_trace_retention_days,
        "30",
      ),
      data_lifecycle_execution_proof_retention_days: str(
        dataLifecycleSettings.execution_proof_retention_days,
        "30",
      ),
      data_lifecycle_operational_log_retention_days: str(
        dataLifecycleSettings.operational_log_retention_days,
        "30",
      ),
      data_lifecycle_security_log_retention_days: str(
        dataLifecycleSettings.security_log_retention_days,
        "30",
      ),
      data_lifecycle_approval_log_retention_days: str(
        dataLifecycleSettings.approval_log_retention_days,
        "30",
      ),
      data_lifecycle_swarm_delegation_retention_days: str(
        dataLifecycleSettings.swarm_delegation_retention_days,
        "30",
      ),
      data_lifecycle_llm_usage_retention_days: str(
        dataLifecycleSettings.llm_usage_retention_days,
        "30",
      ),
      data_lifecycle_terminal_task_retention_days: str(
        dataLifecycleSettings.terminal_task_retention_days,
        "90",
      ),
      data_lifecycle_execution_run_retention_days: str(
        dataLifecycleSettings.execution_run_retention_days,
        "90",
      ),
      data_lifecycle_background_session_retention_days: str(
        dataLifecycleSettings.background_session_retention_days,
        "90",
      ),
      data_lifecycle_browser_session_retention_days: str(
        dataLifecycleSettings.browser_session_retention_days,
        "30",
      ),
      data_lifecycle_automation_run_retention_days: str(
        dataLifecycleSettings.automation_run_retention_days,
        "90",
      ),
      data_lifecycle_message_retention_days: str(
        dataLifecycleSettings.message_retention_days,
        "365",
      ),
      data_lifecycle_experience_run_retention_days: str(
        dataLifecycleSettings.experience_run_retention_days,
        "90",
      ),
      data_lifecycle_experience_edge_retention_days: str(
        dataLifecycleSettings.experience_edge_retention_days,
        "90",
      ),
      data_lifecycle_learning_candidate_retention_days: str(
        dataLifecycleSettings.learning_candidate_retention_days,
        "30",
      ),
      data_lifecycle_experience_item_retention_days: str(
        dataLifecycleSettings.experience_item_retention_days,
        "0",
      ),
      data_lifecycle_procedural_pattern_retention_days: str(
        dataLifecycleSettings.procedural_pattern_retention_days,
        "0",
      ),
      data_lifecycle_recall_event_retention_days: str(
        dataLifecycleSettings.recall_event_retention_days,
        "365",
      ),
      data_lifecycle_recall_test_retention_days: str(
        dataLifecycleSettings.recall_test_retention_days,
        "365",
      ),
      data_lifecycle_housekeeping_interval_secs: str(
        dataLifecycleSettings.housekeeping_interval_secs,
        "3600",
      ),
      data_lifecycle_security_cleanup_interval_days: str(
        dataLifecycleSettings.security_cleanup_interval_days,
        "15",
      ),
      data_lifecycle_security_cleanup_idle_threshold_secs: str(
        dataLifecycleSettings.security_cleanup_idle_threshold_secs,
        "300",
      ),

      observability_enabled: toBool(observabilitySettings.enabled),
      observability_provider: str(observabilitySettings.provider, "langtrace"),
      observability_endpoint: str(observabilitySettings.endpoint, ""),
      observability_service_name: str(
        observabilitySettings.service_name,
        "agentark",
      ),
      observability_header_name: str(
        observabilitySettings.header_name,
        "x-api-key",
      ),
      observability_privacy_mode: str(
        observabilitySettings.privacy_mode,
        "metadata_only",
      ),
      observability_auth_token: "",
    };

    setForm(nextForm);
    setSavedFormSnapshot(
      snapshotSettingsForm(
        hiddenExternalEmbeddingsProvider
          ? {
              ...nextForm,
              embeddings_provider: serverEmbeddingsProvider,
              embeddings_model: str(
                settings.embeddings_model,
                LOCAL_EMBEDDINGS_MODEL,
              ),
              embeddings_base_url: str(settings.embeddings_base_url, ""),
            }
          : nextForm,
      ),
    );

    setDirty(hiddenExternalEmbeddingsProvider);
    setError(null);
    setSuccess(null);
    return hiddenExternalEmbeddingsProvider;
  }

  // Initialize form from backend once; keep defaults if backend is down.
  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.isSuccess) return;
    const dirtyAfterHydrate = hydrateFromServer();
    setInitialized(true);
    setDirty(dirtyAfterHydrate);
  }, [initialized, settingsQ.isSuccess, settingsQ.dataUpdatedAt]);

  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.data || !mediaQ.data) return;
    const dirtyAfterHydrate = hydrateFromServer();
    setInitialized(true);
    setDirty(dirtyAfterHydrate);
  }, [initialized, settingsQ.data, mediaQ.data]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!settingsQ.isSuccess) return;
    setUiTimeZoneOverride(str(settings.timezone, "").trim() || null);
  }, [settingsQ.isSuccess, settings.timezone]);

  // Safety: clear dirty once after hydration settles (handles race between effects)
  const hydrationDirtyCleared = useRef(false);
  useEffect(() => {
    if (initialized && !hydrationDirtyCleared.current) {
      hydrationDirtyCleared.current = true;
      setDirty(
        embeddingsProviderHiddenFromSettingsUi(
          str(settings.embeddings_provider, ""),
        ),
      );
    }
  }, [initialized, settings.embeddings_provider]);

  const saveMutation = useMutation({
    mutationFn: async () => {
      const mediaKeys = parseMediaProvidersJson(form.media_provider_keys_json);
      const mediaProviders: Record<string, string> = { ...mediaKeys };
      const savedSnapshot = parseSavedSettingsSnapshot();
      const fieldChanged = <K extends keyof typeof form>(key: K): boolean =>
        !savedSnapshot || savedSnapshot[key] !== form[key];
      const includeTelegramSettings =
        form.telegram_bot_token.trim().length > 0 ||
        fieldChanged("telegram_enabled") ||
        fieldChanged("telegram_allowed_users_csv");
      const includeSlackSettings =
        form.slack_bot_token.trim().length > 0 ||
        form.slack_signing_secret.trim().length > 0 ||
        fieldChanged("slack_enabled") ||
        fieldChanged("slack_api_base_url") ||
        fieldChanged("slack_default_channel_id") ||
        fieldChanged("slack_default_thread_ts") ||
        fieldChanged("slack_workspace_id") ||
        fieldChanged("slack_workspace_name");
      const includeDiscordSettings =
        form.discord_bot_token.trim().length > 0 ||
        fieldChanged("discord_enabled") ||
        fieldChanged("discord_api_base_url") ||
        fieldChanged("discord_default_channel_id") ||
        fieldChanged("discord_default_thread_id") ||
        fieldChanged("discord_guild_id") ||
        fieldChanged("discord_application_id") ||
        fieldChanged("discord_webhook_url");
      const includeMatrixSettings =
        form.matrix_access_token.trim().length > 0 ||
        fieldChanged("matrix_enabled") ||
        fieldChanged("matrix_homeserver_url") ||
        fieldChanged("matrix_user_id") ||
        fieldChanged("matrix_device_id") ||
        fieldChanged("matrix_account_id") ||
        fieldChanged("matrix_default_room_id") ||
        fieldChanged("matrix_sync_timeout_ms") ||
        fieldChanged("matrix_limit") ||
        fieldChanged("matrix_user_agent");
      const includeTeamsSettings =
        form.teams_access_token.trim().length > 0 ||
        fieldChanged("teams_enabled") ||
        fieldChanged("teams_service_url") ||
        fieldChanged("teams_bot_app_id") ||
        fieldChanged("teams_bot_name") ||
        fieldChanged("teams_tenant_id") ||
        fieldChanged("teams_team_id") ||
        fieldChanged("teams_channel_id") ||
        fieldChanged("teams_chat_id") ||
        fieldChanged("teams_graph_base_url") ||
        fieldChanged("teams_delivery_mode") ||
        fieldChanged("teams_timeout_secs") ||
        fieldChanged("teams_user_agent");
      const includeWhatsappSettings =
        form.whatsapp_access_token.trim().length > 0 ||
        form.whatsapp_app_secret.trim().length > 0 ||
        fieldChanged("whatsapp_enabled") ||
        fieldChanged("whatsapp_mode") ||
        fieldChanged("whatsapp_phone_number_id") ||
        fieldChanged("whatsapp_verify_token") ||
        fieldChanged("whatsapp_bridge_url") ||
        fieldChanged("whatsapp_dm_policy") ||
        fieldChanged("whatsapp_allowed_numbers_csv");
      const mediaFieldKeys: Array<[string, string]> = [
        ["replicate", form.media_key_replicate],
        ["fal", form.media_key_fal],
        ["stability_ai", form.media_key_stability_ai],
        ["together", form.media_key_together],
        ["openai_dalle", form.media_key_openai_dalle],
        ["google_gemini", form.media_key_google_gemini],
        ["runway", form.media_key_runway],
        ["luma", form.media_key_luma],
      ];
      for (const [k, v] of mediaFieldKeys) {
        const trimmed = (v || "").trim();
        if (trimmed) {
          mediaProviders[k] = trimmed;
          if (k === "openai_dalle") mediaProviders["openai_sora"] = trimmed;
          if (k === "google_gemini") mediaProviders["google_veo"] = trimmed;
        }
      }
      const mediaProviderBaseUrls: Record<string, string> = {
        replicate: form.media_base_url_replicate.trim(),
        fal: form.media_base_url_fal.trim(),
        stability_ai: form.media_base_url_stability_ai.trim(),
        together: form.media_base_url_together.trim(),
        openai_dalle: form.media_base_url_openai_dalle.trim(),
        openai_sora: form.media_base_url_openai_sora.trim(),
        google_gemini: form.media_base_url_google_gemini.trim(),
        google_veo: form.media_base_url_google_veo.trim(),
        runway: form.media_base_url_runway.trim(),
        luma: form.media_base_url_luma.trim(),
      };
      const dataLifecycle = {
        cleanup_enabled: form.data_lifecycle_cleanup_enabled,
        notifications_cleanup_enabled:
          form.data_lifecycle_notifications_cleanup_enabled,
        logs_cleanup_enabled: form.data_lifecycle_logs_cleanup_enabled,
        notifications_retention_days: parseNonNegativeInt(
          form.data_lifecycle_notifications_retention_days,
          "Notification retention (days)",
        ),
        notification_cleanup_interval_secs: parsePositiveInt(
          form.data_lifecycle_notification_cleanup_interval_secs,
          "Notification cleanup cadence (seconds)",
          300,
        ),
        execution_trace_retention_days: parseNonNegativeInt(
          form.data_lifecycle_execution_trace_retention_days,
          "Execution trace retention (days)",
        ),
        execution_proof_retention_days: parseNonNegativeInt(
          form.data_lifecycle_execution_proof_retention_days,
          "Execution proof retention (days)",
        ),
        operational_log_retention_days: parseNonNegativeInt(
          form.data_lifecycle_operational_log_retention_days,
          "Operational log retention (days)",
        ),
        security_log_retention_days: parseNonNegativeInt(
          form.data_lifecycle_security_log_retention_days,
          "Security log retention (days)",
        ),
        approval_log_retention_days: parseNonNegativeInt(
          form.data_lifecycle_approval_log_retention_days,
          "Approval log retention (days)",
        ),
        swarm_delegation_retention_days: parseNonNegativeInt(
          form.data_lifecycle_swarm_delegation_retention_days,
          "Delegation retention (days)",
        ),
        llm_usage_retention_days: parseNonNegativeInt(
          form.data_lifecycle_llm_usage_retention_days,
          "LLM usage retention (days)",
        ),
        terminal_task_retention_days: parseNonNegativeInt(
          form.data_lifecycle_terminal_task_retention_days,
          "Completed task retention (days)",
        ),
        execution_run_retention_days: parseNonNegativeInt(
          form.data_lifecycle_execution_run_retention_days,
          "Execution run retention (days)",
        ),
        background_session_retention_days: parseNonNegativeInt(
          form.data_lifecycle_background_session_retention_days,
          "Background session retention (days)",
        ),
        browser_session_retention_days: parseNonNegativeInt(
          form.data_lifecycle_browser_session_retention_days,
          "Browser session retention (days)",
        ),
        automation_run_retention_days: parseNonNegativeInt(
          form.data_lifecycle_automation_run_retention_days,
          "Automation run retention (days)",
        ),
        message_retention_days: parseNonNegativeInt(
          form.data_lifecycle_message_retention_days,
          "Conversation retention (days)",
        ),
        experience_run_retention_days: parseNonNegativeInt(
          form.data_lifecycle_experience_run_retention_days,
          "Experience run retention (days)",
        ),
        experience_edge_retention_days: parseNonNegativeInt(
          form.data_lifecycle_experience_edge_retention_days,
          "Experience edge retention (days)",
        ),
        learning_candidate_retention_days: parseNonNegativeInt(
          form.data_lifecycle_learning_candidate_retention_days,
          "Learning candidate retention (days)",
        ),
        experience_item_retention_days: parseNonNegativeInt(
          form.data_lifecycle_experience_item_retention_days,
          "Inactive memory retention (days)",
        ),
        procedural_pattern_retention_days: parseNonNegativeInt(
          form.data_lifecycle_procedural_pattern_retention_days,
          "Inactive pattern retention (days)",
        ),
        recall_event_retention_days: parseNonNegativeInt(
          form.data_lifecycle_recall_event_retention_days,
          "Memory ledger retention (days)",
        ),
        recall_test_retention_days: parseNonNegativeInt(
          form.data_lifecycle_recall_test_retention_days,
          "Memory check retention (days)",
        ),
        housekeeping_interval_secs: parsePositiveInt(
          form.data_lifecycle_housekeeping_interval_secs,
          "Housekeeping cleanup cadence (seconds)",
          300,
        ),
        security_cleanup_interval_days: parsePositiveInt(
          form.data_lifecycle_security_cleanup_interval_days,
          "Security cleanup cadence (days)",
          1,
        ),
        security_cleanup_idle_threshold_secs: parsePositiveInt(
          form.data_lifecycle_security_cleanup_idle_threshold_secs,
          "Security cleanup idle threshold (seconds)",
          60,
        ),
      };
      const embeddingsProviderForSave = settingsUiEmbeddingsProvider(
        form.embeddings_provider,
      );
      const payload: Record<string, unknown> = {
        bot_name: form.bot_name || "AgentArk",
        personality: form.personality || "friendly",
        // Send empty strings to clear fields (null means "skip update" on backend).
        timezone: form.timezone,
        language: form.language,
        tone: form.tone,
        email_format: form.email_format,
        daily_brief_enabled: form.daily_brief_enabled,
        daily_brief_time: form.daily_brief_time || "09:00",
        daily_brief_channel: form.daily_brief_channel || "telegram",
        arkreflect_daily_digest_enabled: form.arkreflect_daily_digest_enabled,
        smart_routing: form.smart_routing,
        embeddings_provider: embeddingsProviderForSave,
        embeddings_model: LOCAL_EMBEDDINGS_MODEL,
        embeddings_base_url: null,
        embeddings_api_key: null,

        llm_provider: form.llm_provider,
        llm_model: form.llm_model,
        llm_base_url: form.llm_base_url || null,
        llm_api_key: form.llm_api_key || null,

        llm_fallback_provider: form.llm_fallback_provider || null,
        llm_fallback_model: form.llm_fallback_model || null,
        llm_fallback_base_url: form.llm_fallback_base_url || null,
        llm_fallback_api_key: form.llm_fallback_api_key || null,
        default_model_input_mode:
          form.model_privacy_default_mode || "default_redact",
        current_chat_pii_policy:
          form.model_privacy_current_chat_pii_policy || "mask_chat_pii",
        request_scoped_sensitive_approval_enabled:
          form.model_privacy_request_scoped_sensitive_approval_enabled,

        auto_approve: sanitizeAutoApproveList(
          parseCsvList(form.auto_approve_csv),
        ),

        media_providers: mediaProviders,
        media_provider_base_urls: mediaProviderBaseUrls,
        default_image_provider: form.default_image_provider || null,
        image_model: form.image_model || null,
        fallback_image_provider: form.fallback_image_provider || null,
        default_video_provider: form.default_video_provider || null,
        fallback_video_provider: form.fallback_video_provider || null,

        search_serper_key: form.search_serper_key || null,
        clear_search_serper_key:
          form.search_serper_clear && !form.search_serper_key.trim(),
        search_brave_key: form.search_brave_key || null,
        clear_search_brave_key:
          form.search_brave_clear && !form.search_brave_key.trim(),
        search_exa_key: form.search_exa_key || null,
        clear_search_exa_key:
          form.search_exa_clear && !form.search_exa_key.trim(),
        search_tavily_key: form.search_tavily_key || null,
        clear_search_tavily_key:
          form.search_tavily_clear && !form.search_tavily_key.trim(),
        search_perplexity_key: form.search_perplexity_key || null,
        clear_search_perplexity_key:
          form.search_perplexity_clear && !form.search_perplexity_key.trim(),
        search_firecrawl_key: form.search_firecrawl_key || null,
        clear_search_firecrawl_key:
          form.search_firecrawl_clear && !form.search_firecrawl_key.trim(),
        search_searxng_base_url: form.search_searxng_base_url,

        data_lifecycle: dataLifecycle,

        observability: {
          enabled: form.observability_enabled,
          provider: form.observability_provider || "langtrace",
          endpoint: form.observability_endpoint || "",
          service_name: form.observability_service_name || "agentark",
          header_name: form.observability_header_name || "x-api-key",
          privacy_mode: form.observability_privacy_mode || "metadata_only",
          // Only send auth_token when user entered a new value - blank means "keep existing"
          ...(form.observability_auth_token.trim()
            ? { auth_token: form.observability_auth_token }
            : {}),
        },
      };

      if (includeTelegramSettings) {
        payload.telegram_enabled = !!form.telegram_enabled;
        payload.telegram_bot_token = form.telegram_bot_token || null;
        payload.telegram_allowed_users = parseTelegramUsers(
          form.telegram_allowed_users_csv,
        );
      }

      if (includeSlackSettings) {
        payload.slack_enabled = !!form.slack_enabled;
        payload.slack_bot_token = form.slack_bot_token || null;
        payload.slack_signing_secret = form.slack_signing_secret || null;
        payload.slack_api_base_url = form.slack_api_base_url || null;
        payload.slack_default_channel_id =
          form.slack_default_channel_id || null;
        payload.slack_default_thread_ts = form.slack_default_thread_ts || null;
        payload.slack_workspace_id = form.slack_workspace_id || null;
        payload.slack_workspace_name = form.slack_workspace_name || null;
      }

      if (includeDiscordSettings) {
        payload.discord_enabled = !!form.discord_enabled;
        payload.discord_bot_token = form.discord_bot_token || null;
        payload.discord_api_base_url = form.discord_api_base_url || null;
        payload.discord_default_channel_id =
          form.discord_default_channel_id || null;
        payload.discord_default_thread_id =
          form.discord_default_thread_id || null;
        payload.discord_guild_id = form.discord_guild_id || null;
        payload.discord_application_id = form.discord_application_id || null;
        payload.discord_webhook_url = form.discord_webhook_url || null;
      }

      if (includeMatrixSettings) {
        payload.matrix_enabled = !!form.matrix_enabled;
        payload.matrix_homeserver_url = form.matrix_homeserver_url || null;
        payload.matrix_access_token = form.matrix_access_token || null;
        payload.matrix_user_id = form.matrix_user_id || null;
        payload.matrix_device_id = form.matrix_device_id || null;
        payload.matrix_account_id = form.matrix_account_id || null;
        payload.matrix_default_room_id = form.matrix_default_room_id || null;
        payload.matrix_sync_timeout_ms =
          Number.parseInt(form.matrix_sync_timeout_ms || "0", 10) || 0;
        payload.matrix_limit =
          Number.parseInt(form.matrix_limit || "0", 10) || 100;
        payload.matrix_user_agent = form.matrix_user_agent || null;
      }

      if (includeTeamsSettings) {
        payload.teams_enabled = !!form.teams_enabled;
        payload.teams_service_url = form.teams_service_url || null;
        payload.teams_access_token = form.teams_access_token || null;
        payload.teams_bot_app_id = form.teams_bot_app_id || null;
        payload.teams_bot_name = form.teams_bot_name || null;
        payload.teams_tenant_id = form.teams_tenant_id || null;
        payload.teams_team_id = form.teams_team_id || null;
        payload.teams_channel_id = form.teams_channel_id || null;
        payload.teams_chat_id = form.teams_chat_id || null;
        payload.teams_graph_base_url = form.teams_graph_base_url || null;
        payload.teams_delivery_mode = form.teams_delivery_mode || null;
        payload.teams_timeout_secs =
          Number.parseInt(form.teams_timeout_secs || "0", 10) || 15;
        payload.teams_user_agent = form.teams_user_agent || null;
      }

      if (includeWhatsappSettings) {
        payload.whatsapp_enabled = !!form.whatsapp_enabled;
        payload.whatsapp_mode = form.whatsapp_mode || null;
        payload.whatsapp_access_token = form.whatsapp_access_token || null;
        payload.whatsapp_app_secret = form.whatsapp_app_secret || null;
        payload.whatsapp_phone_number_id =
          form.whatsapp_phone_number_id || null;
        payload.whatsapp_verify_token = form.whatsapp_verify_token || null;
        payload.whatsapp_bridge_url = form.whatsapp_bridge_url || null;
        payload.whatsapp_dm_policy = form.whatsapp_dm_policy || null;
        payload.whatsapp_allowed_numbers = parseCsvList(
          form.whatsapp_allowed_numbers_csv,
        );
      }

      return await api.rawPost("/settings", payload);
    },
    onSuccess: async () => {
      setError(null);
      setSuccess("Saved settings.");
      setDirty(false);
      setUiTimeZoneOverride(form.timezone.trim() || null);
      const savedSnapshot = parseSavedSettingsSnapshot();
      const observabilityChanged =
        !savedSnapshot ||
        snapshotObservabilityForm(form) !==
          snapshotObservabilityForm(savedSnapshot);
      const shouldTestObservability =
        observabilityChanged &&
        form.observability_enabled &&
        form.observability_endpoint.trim().length > 0 &&
        (form.observability_auth_token.trim().length > 0 ||
          toBool(observabilitySettings.auth_token_configured));
      setForm((prev) => {
        const nextForm = {
          ...prev,
          embeddings_api_key: "",
          llm_api_key: "",
          llm_fallback_api_key: "",
          telegram_bot_token: "",
          whatsapp_access_token: "",
          whatsapp_app_secret: "",
          media_provider_keys_json: "",
          media_key_replicate: "",
          media_key_fal: "",
          media_key_stability_ai: "",
          media_key_together: "",
          media_key_openai_dalle: "",
          media_key_google_gemini: "",
          media_key_runway: "",
          media_key_luma: "",
          media_base_url_replicate: str(asRecord(media.provider_base_urls).replicate, ""),
          media_base_url_fal: str(asRecord(media.provider_base_urls).fal, ""),
          media_base_url_stability_ai: str(
            asRecord(media.provider_base_urls).stability_ai,
            "",
          ),
          media_base_url_together: str(asRecord(media.provider_base_urls).together, ""),
          media_base_url_openai_dalle: str(
            asRecord(media.provider_base_urls).openai_dalle,
            "",
          ),
          media_base_url_openai_sora: str(
            asRecord(media.provider_base_urls).openai_sora,
            "",
          ),
          media_base_url_google_gemini: str(
            asRecord(media.provider_base_urls).google_gemini,
            "",
          ),
          media_base_url_google_veo: str(
            asRecord(media.provider_base_urls).google_veo,
            "",
          ),
          media_base_url_runway: str(asRecord(media.provider_base_urls).runway, ""),
          media_base_url_luma: str(asRecord(media.provider_base_urls).luma, ""),
          search_serper_key: "",
          search_serper_editing: false,
          search_serper_clear: false,
          search_brave_key: "",
          search_brave_editing: false,
          search_brave_clear: false,
          search_exa_key: "",
          search_exa_editing: false,
          search_exa_clear: false,
          search_tavily_key: "",
          search_tavily_editing: false,
          search_tavily_clear: false,
          search_perplexity_key: "",
          search_perplexity_editing: false,
          search_perplexity_clear: false,
          search_firecrawl_key: "",
          search_firecrawl_editing: false,
          search_firecrawl_clear: false,
          observability_auth_token: "",
        };
        setSavedFormSnapshot(snapshotSettingsForm(nextForm));
        return nextForm;
      });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({ queryKey: ["profile"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-media"] });
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-observability-logs"],
      });
      if (shouldTestObservability) {
        try {
          await api.rawPost("/settings/observability/test", {});
          await queryClient.invalidateQueries({
            queryKey: ["settings-observability-logs"],
          });
          setSuccess("Saved settings. Sent a test observability trace.");
        } catch (e) {
          await queryClient.invalidateQueries({
            queryKey: ["settings-observability-logs"],
          });
          setSuccess("Saved settings.");
          setError(`Observability test failed after save: ${errMessage(e)}`);
        }
      }
    },
    onError: (e) => {
      setSuccess(null);
      setError(errMessage(e));
    },
  });

  async function handleSaveSettings() {
    setError(null);
    setSuccess(null);
    try {
      if (settingsFormDirty) {
        await saveMutation.mutateAsync();
      }
      if (developerModeDirty) {
        setDeveloperModeEnabled(developerModeEnabled);
        setSavedDeveloperModeEnabledState(developerModeEnabled);
        if (!settingsFormDirty) {
          setSuccess("Saved settings.");
        }
      }
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const testObservabilityMutation = useMutation({
    mutationFn: () => api.rawPost("/settings/observability/test", {}),
    onSuccess: async () => {
      setError(null);
      setSuccess("Sent a test observability trace.");
      await queryClient.invalidateQueries({
        queryKey: ["settings-observability-logs"],
      });
    },
    onError: (e) => {
      setSuccess(null);
      setError(errMessage(e));
    },
  });

  const modelSlotsLive = useMemo(
    () => pickRecords(modelsPayload, "models"),
    [modelsPayload],
  );
  const settingsPayloadError = str(settings.error, "").trim();
  const modelsPayloadError = str(modelsPayload.error, "").trim();
  const [stableModelSlots, setStableModelSlots] = useState<JsonRecord[]>([]);
  const [stableSettingsComplete, setStableSettingsComplete] = useState(false);
  const consecutiveEmptyModelSnapshotsRef = useRef(0);
  const consecutiveIncompleteSettingsRef = useRef(0);

  useEffect(() => {
    if (modelSlotsLive.length > 0) {
      consecutiveEmptyModelSnapshotsRef.current = 0;
      setStableModelSlots(modelSlotsLive);
      return;
    }
    if (modelsQ.isFetching || modelsQ.isError || modelsPayloadError) return;
    if (!modelsQ.isSuccess) return;
    consecutiveEmptyModelSnapshotsRef.current += 1;
    if (consecutiveEmptyModelSnapshotsRef.current >= 2) {
      setStableModelSlots([]);
    }
  }, [
    modelSlotsLive,
    modelsQ.isFetching,
    modelsQ.isError,
    modelsQ.isSuccess,
    modelsPayloadError,
  ]);

  const modelSlots = useMemo(() => {
    if (modelSlotsLive.length > 0) return modelSlotsLive;
    if (stableModelSlots.length > 0) return stableModelSlots;
    return modelSlotsLive;
  }, [modelSlotsLive, stableModelSlots]);

  useEffect(() => {
    const hasSnapshotIssue =
      !!settingsPayloadError ||
      !!modelsPayloadError ||
      settingsQ.isError ||
      modelsQ.isError;
    const computedComplete =
      toBool(settings.settings_complete) ||
      modelSlotsLive.length > 0 ||
      stableModelSlots.length > 0;
    if (computedComplete) {
      consecutiveIncompleteSettingsRef.current = 0;
      if (!stableSettingsComplete) setStableSettingsComplete(true);
      return;
    }
    if (
      hasSnapshotIssue ||
      settingsQ.isFetching ||
      modelsQ.isFetching ||
      !settingsQ.isSuccess ||
      !modelsQ.isSuccess
    ) {
      return;
    }
    consecutiveIncompleteSettingsRef.current += 1;
    if (
      consecutiveIncompleteSettingsRef.current >= 2 &&
      stableSettingsComplete
    ) {
      setStableSettingsComplete(false);
    }
  }, [
    settings.settings_complete,
    modelSlotsLive.length,
    stableModelSlots.length,
    settingsPayloadError,
    modelsPayloadError,
    settingsQ.isError,
    modelsQ.isError,
    settingsQ.isFetching,
    modelsQ.isFetching,
    settingsQ.isSuccess,
    modelsQ.isSuccess,
    stableSettingsComplete,
  ]);

  const shouldDerivePulseData =
    pulseTabActive || Boolean(pulsePollState) || selectedPulseEvent != null;

  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [searchProviderDialog, setSearchProviderDialog] = useState<{
    providerId: string;
    value: string;
    showValue: boolean;
  } | null>(null);
  const [searxngDialog, setSearxngDialog] = useState<{
    value: string;
  } | null>(null);
  const [modelsSectionTab, setModelsSectionTab] = useState<
    "pool" | "embeddings"
  >("pool");
  const [modelEditingId, setModelEditingId] = useState<string | null>(null);
  const [modelAdvancedOpen, setModelAdvancedOpen] = useState(false);
  const [modelEditingHasApiKey, setModelEditingHasApiKey] = useState(false);
  const [modelEditingOriginalScope, setModelEditingOriginalScope] = useState({
    provider: "",
    base_url: "",
  });
  const [modelClearApiKey, setModelClearApiKey] = useState(false);
  const [openaiSubAuth, setOpenaiSubAuth] = useState<{
    message: string;
    authUrl: string;
    deviceCode: string;
    running: boolean;
    openedBrowser: boolean;
  } | null>(null);
  const [codexAuthBusy, setCodexAuthBusy] = useState(false);
  const [modelConnectionTestResult, setModelConnectionTestResult] = useState<{
    ok: boolean;
    message: string;
  } | null>(null);
  const [modelForm, setModelForm] = useState({
    label: "",
    role: "primary",
    provider: "",
    model: "",
    base_url: "",
    api_key: "",
    enabled: true,
  });
  const previousModelProviderRef = useRef(modelForm.provider);
  const normalizeModelCredentialScope = (
    providerValue: string,
    baseUrlValue: string,
  ) => {
    const provider = str(providerValue, "").trim().toLowerCase();
    const normalizedBaseUrl = str(baseUrlValue, "").trim().replace(/\/+$/, "");
    if (!provider) return { provider: "", baseUrl: "" };
    if (provider === "openrouter") {
      return {
        provider,
        baseUrl: normalizedBaseUrl || OPENROUTER_DEFAULT_BASE_URL,
      };
    }
    if (
      provider === "openai" ||
      provider === "anthropic" ||
      provider === "openai-subscription"
    ) {
      return { provider, baseUrl: "" };
    }
    return { provider, baseUrl: normalizedBaseUrl };
  };
  const modelCanReuseExistingKey = useMemo(() => {
    if (!modelEditingId) return false;
    const currentScope = normalizeModelCredentialScope(
      modelForm.provider,
      modelForm.base_url,
    );
    const originalScope = normalizeModelCredentialScope(
      modelEditingOriginalScope.provider,
      modelEditingOriginalScope.base_url,
    );
    return (
      !!currentScope.provider &&
      currentScope.provider === originalScope.provider &&
      currentScope.baseUrl === originalScope.baseUrl
    );
  }, [
    modelEditingId,
    modelForm.provider,
    modelForm.base_url,
    modelEditingOriginalScope.provider,
    modelEditingOriginalScope.base_url,
  ]);
  const modelPendingApiKey = modelForm.api_key.trim();
  const modelNeedsReplacementKeyWarning =
    !!modelEditingId &&
    modelEditingHasApiKey &&
    !modelCanReuseExistingKey &&
    !modelPendingApiKey &&
    modelForm.provider !== "ollama" &&
    modelForm.provider !== "openai-subscription";
  const modelClearSavedKeyPending =
    !!modelEditingId &&
    modelEditingHasApiKey &&
    modelCanReuseExistingKey &&
    modelClearApiKey &&
    !modelPendingApiKey;
  const showClearSavedKeyAction =
    !!modelEditingId &&
    modelEditingHasApiKey &&
    modelCanReuseExistingKey &&
    modelForm.provider !== "ollama" &&
    modelForm.provider !== "openai-subscription";
  const modelCanReuseSavedKeyForTest =
    !!modelEditingId &&
    modelEditingHasApiKey &&
    modelCanReuseExistingKey &&
    !modelClearSavedKeyPending;
  const modelProviderRequiresBaseUrl =
    modelForm.provider === "ollama" ||
    modelForm.provider === "openai-compatible";
  const modelProviderRequiresApiKey = [
    "anthropic",
    "openai",
    "openrouter",
    "huggingface",
  ].includes(modelForm.provider);
  const canTestModelConnection =
    !!modelForm.label.trim() &&
    !!modelForm.provider.trim() &&
    !!modelForm.model.trim() &&
    (!modelProviderRequiresBaseUrl || !!modelForm.base_url.trim()) &&
    (!modelProviderRequiresApiKey ||
      !!modelPendingApiKey ||
      modelCanReuseSavedKeyForTest);
  const modelTestConnectionHint = !modelForm.label.trim()
    ? "Enter a label before testing."
    : !modelForm.provider.trim()
    ? "Choose a provider to test."
    : !modelForm.model.trim()
      ? "Enter a model ID to test."
      : modelProviderRequiresBaseUrl && !modelForm.base_url.trim()
        ? modelForm.provider === "ollama"
          ? "Enter the Ollama base URL before testing."
          : "Enter the base URL before testing."
        : modelProviderRequiresApiKey &&
            !modelPendingApiKey &&
            !modelCanReuseSavedKeyForTest
          ? modelEditingId
            ? "Enter a replacement API key before testing this draft."
            : "Enter an API key before testing."
          : "";

  useEffect(() => {
    const prevProvider = previousModelProviderRef.current;
    if (prevProvider === modelForm.provider) return;
    previousModelProviderRef.current = modelForm.provider;
    setOpenaiSubAuth(null);

    setModelForm((p) => {
      const current = p.base_url.trim();
      let next = p.base_url;
      let nextModel = p.model;
      if (p.provider === "openrouter") {
        if (current === OLLAMA_DEFAULT_BASE_URL) next = "";
      } else if (p.provider === "ollama") {
        if (current === OPENROUTER_DEFAULT_BASE_URL) next = "";
      } else if (
        (p.provider === "openai" ||
          p.provider === "anthropic" ||
          p.provider === "openai-subscription") &&
        (current === OLLAMA_DEFAULT_BASE_URL ||
          current === OPENROUTER_DEFAULT_BASE_URL)
      ) {
        next = "";
      }
      const providerFallback =
        MODEL_FALLBACKS_BY_PROVIDER[p.provider]?.[0] || "";
      const previousProviderFallbacks =
        MODEL_FALLBACKS_BY_PROVIDER[prevProvider] || [];
      const providerChanged = !!prevProvider && prevProvider !== p.provider;
      if (
        providerFallback &&
        (!p.model.trim() ||
          providerChanged ||
          previousProviderFallbacks.includes(p.model.trim()))
      ) {
        nextModel = providerFallback;
      } else if (providerChanged) {
        nextModel = "";
      }
      return next === p.base_url && nextModel === p.model
        ? p
        : { ...p, base_url: next, model: nextModel };
    });
  }, [modelForm.provider]);

  useEffect(() => {
    setModelConnectionTestResult(null);
  }, [
    modelEditingId,
    modelForm.label,
    modelForm.role,
    modelForm.provider,
    modelForm.model,
    modelForm.base_url,
    modelForm.api_key,
    modelForm.enabled,
    modelClearApiKey,
  ]);

  const discoverModelsQ = useQuery({
    queryKey: [
      "discover-models",
      modelForm.provider,
      modelForm.api_key,
      modelForm.base_url,
    ],
    queryFn: async () => {
      const p = modelForm.provider;
      if (!p) return [] as Array<{ id: string; name?: string }>;
      if (p === "openai-compatible" && !modelForm.base_url.trim())
        return [] as Array<{ id: string; name?: string }>;
      const params = new URLSearchParams();
      if (modelForm.api_key.trim())
        params.set("api_key", modelForm.api_key.trim());
      if (modelForm.base_url.trim())
        params.set("base_url", modelForm.base_url.trim());
      try {
        const resp = asRecord(
          await api.rawGet(
            `/models/discover/${encodeURIComponent(p)}?${params.toString()}`,
          ),
        );
        const models = resp.models;
        if (Array.isArray(models)) {
          return models.reduce<Array<{ id: string; name?: string }>>(
            (acc, m: unknown) => {
              const row = asRecord(m);
              const id = str(row.id, "").trim();
              if (!id) return acc;
              const name = str(row.name, "").trim();
              acc.push({ id, name: name || undefined });
              return acc;
            },
            [],
          );
        }
      } catch {
        /* ignore */
      }
      return [] as Array<{ id: string; name?: string }>;
    },
    enabled:
      modelDialogOpen &&
      !!modelForm.provider &&
      (modelForm.provider !== "openai-compatible" ||
        !!modelForm.base_url.trim()),
    staleTime: 60_000,
    retry: false,
  });
  const modelOptionNames = useMemo(() => {
    const names = new Map<string, string>();
    for (const model of discoverModelsQ.data || []) {
      if (model.name && model.name !== model.id)
        names.set(model.id, model.name);
    }
    return names;
  }, [discoverModelsQ.data]);
  const discoveredModelIds = useMemo(
    () => (discoverModelsQ.data || []).map((model) => model.id).filter(Boolean),
    [discoverModelsQ.data],
  );
  useEffect(() => {
    if (!modelDialogOpen || discoverModelsQ.isFetching) return;
    const firstDiscoveredModel = discoveredModelIds[0];
    if (!firstDiscoveredModel) return;
    setModelForm((p) => {
      if (p.provider !== modelForm.provider) return p;
      const currentModel = p.model.trim();
      const fallbackModels = MODEL_FALLBACKS_BY_PROVIDER[p.provider] || [];
      if (currentModel && !fallbackModels.includes(currentModel)) return p;
      return currentModel === firstDiscoveredModel
        ? p
        : { ...p, model: firstDiscoveredModel };
    });
  }, [
    discoveredModelIds,
    discoverModelsQ.isFetching,
    modelDialogOpen,
    modelForm.provider,
  ]);
  const modelOptions = useMemo(() => {
    const merged: string[] = [];
    for (const candidate of [
      ...discoveredModelIds,
      ...(MODEL_FALLBACKS_BY_PROVIDER[modelForm.provider] || []),
      modelForm.model.trim(),
    ]) {
      const value = String(candidate || "").trim();
      if (!value || merged.includes(value)) continue;
      merged.push(value);
    }
    return merged;
  }, [discoveredModelIds, modelForm.model, modelForm.provider]);

  function openAddModel() {
    setModelEditingId(null);
    setModelAdvancedOpen(false);
    setModelConnectivityWarning(null);
    setModelConnectionTestResult(null);
    setModelEditingHasApiKey(false);
    setModelEditingOriginalScope({ provider: "", base_url: "" });
    setModelClearApiKey(false);
    setOpenaiSubAuth(null);
    previousModelProviderRef.current = "";
    setModelForm({
      label: "",
      role: "primary",
      provider: "",
      model: "",
      base_url: "",
      api_key: "",
      enabled: true,
    });
    setModelDialogOpen(true);
  }

  function openEditModel(slot: JsonRecord) {
    const provider = str(slot.provider, "");
    const baseUrl = str(slot.base_url, "");
    setModelEditingId(str(slot.id, ""));
    setModelAdvancedOpen(false);
    setModelConnectivityWarning(null);
    setModelConnectionTestResult(null);
    setModelEditingHasApiKey(toBool(slot.has_api_key));
    setModelEditingOriginalScope({
      provider,
      base_url: baseUrl,
    });
    setModelClearApiKey(false);
    setOpenaiSubAuth(null);
    previousModelProviderRef.current = provider;
    setModelForm({
      label: str(slot.label, ""),
      role: str(slot.role, "primary"),
      provider,
      model: str(slot.model, ""),
      base_url: baseUrl,
      api_key: "",
      enabled: toBool(slot.enabled),
    });
    setModelDialogOpen(true);
  }

  async function startOpenaiSubscriptionOAuth() {
    if (codexAuthBusy) return;
    setCodexAuthBusy(true);
    setError(null);
    try {
      const response = asRecord(
        await api.rawPost("/models/openai-subscription/oauth/start", {}),
      );
      const message =
        str(response.message, "").trim() ||
        "OpenAI Subscription sign-in started.";
      const authUrl = str(response.auth_url, "").trim();
      const deviceCode = str(response.device_code, "").trim();
      const running = toBool(response.running);
      let openedInBrowser = false;
      if (authUrl) {
        const tab = window.open(authUrl, "_blank", "noopener,noreferrer");
        openedInBrowser = !!tab;
      }
      const openedBrowser = toBool(response.opened_browser) || openedInBrowser;
      setOpenaiSubAuth({
        message,
        authUrl,
        deviceCode,
        running,
        openedBrowser,
      });
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setCodexAuthBusy(false);
    }
  }

  async function checkOpenaiSubscriptionOAuthStatus() {
    if (codexAuthBusy) return;
    setCodexAuthBusy(true);
    setOpenaiSubAuth(null);
    setError(null);
    try {
      const response = asRecord(
        await api.rawGet("/models/openai-subscription/oauth/status"),
      );
      const connected = toBool(response.connected);
      const message = str(response.message, "").trim();
      const authUrl = str(response.auth_url, "").trim();
      const deviceCode = str(response.device_code, "").trim();
      const running = toBool(response.running);
      const openedBrowser = false;
      if (connected) {
        setOpenaiSubAuth({
          message: message || "OpenAI Subscription login is connected.",
          authUrl,
          deviceCode,
          running,
          openedBrowser,
        });
      } else {
        setOpenaiSubAuth({
          message: message || "OpenAI Subscription login is not connected yet.",
          authUrl,
          deviceCode,
          running,
          openedBrowser,
        });
      }
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setCodexAuthBusy(false);
    }
  }

  function buildModelRequestPayload() {
    const provider = modelForm.provider;
    const baseUrl = modelForm.base_url.trim();
    if (!provider.trim()) throw new Error("Provider is required.");
    const normalizedBaseUrl =
      provider === "openai-subscription" ? "" : baseUrl;
    const payload: Record<string, unknown> = {
      label: modelForm.label.trim(),
      role: modelForm.role,
      provider,
      model: modelForm.model.trim(),
      base_url: normalizedBaseUrl || null,
      api_key: modelPendingApiKey || null,
      clear_api_key:
        !!modelEditingId && modelClearApiKey && !modelPendingApiKey,
      enabled: modelForm.enabled,
    };

    if (!payload.label || !payload.model)
      throw new Error("Label and model are required.");

    return payload;
  }

  const testModelConnectionMutation = useMutation({
    mutationFn: async () => {
      const payload = buildModelRequestPayload();
      const response = asRecord(
        await api.rawPost(
          "/models/test",
          {
            id: modelEditingId,
            ...payload,
          },
          { timeoutMs: 30000 },
        ),
      );
      return {
        ok: toBool(response.ok),
        error: str(response.error, "").trim(),
      };
    },
    onSuccess: (result: { ok: boolean; error: string }) => {
      setModelConnectionTestResult(
        result.ok
          ? {
              ok: true,
              message: "Connection check passed.",
            }
          : {
              ok: false,
              message: result.error || "Connection check failed.",
            },
      );
    },
    onError: (e) => {
      setModelConnectionTestResult({
        ok: false,
        message: errMessage(e),
      });
    },
  });

  const saveModelMutation = useMutation({
    mutationFn: async () => {
      const payload = buildModelRequestPayload();

      if (modelEditingId) {
        await api.rawPut(`/models/${encodeURIComponent(modelEditingId)}`, payload, {
          timeoutMs: 15000,
        });
        return;
      }
      await api.rawPost("/models", payload, { timeoutMs: 15000 });
    },
    onSuccess: async () => {
      const wasEdit = !!modelEditingId;
      setModelDialogOpen(false);
      setModelConnectionTestResult(null);
      setModelConnectivityWarning(null);
      setSuccess(wasEdit ? "Model updated." : "Model added.");
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (e) => setError(errMessage(e)),
  });

  const removeModelSlotLocally = (slotId: string) => {
    const normalizedId = slotId.trim();
    if (!normalizedId) return;
    setStableModelSlots((prev) =>
      prev.filter((slot) => str(slot.id, "").trim() !== normalizedId),
    );
    queryClient.setQueryData(["models"], (current: unknown) => {
      const payload = asRecord(current);
      const nextModels = pickRecords(payload, "models").filter(
        (slot) => str(slot.id, "").trim() !== normalizedId,
      );
      return {
        ...payload,
        models: nextModels,
      };
    });
  };

  const deleteModelMutation = useMutation({
    mutationFn: async (slot: JsonRecord) => {
      const id = str(slot.id, "").trim();
      try {
        await api.rawDelete(`/models/${encodeURIComponent(id)}`, {
          timeoutMs: 15000,
        });
        return { alreadyRemoved: false };
      } catch (e) {
        const message = errMessage(e);
        if (/model slot not found/i.test(message)) {
          return { alreadyRemoved: true };
        }
        throw e;
      }
    },
    onSuccess: async (result, slot) => {
      removeModelSlotLocally(str(slot.id, ""));
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      setSuccess(
        result.alreadyRemoved ? "Model was already removed." : "Model removed.",
      );
    },
    onError: (e) => setError(errMessage(e)),
  });

  const toggleModelEnabledMutation = useMutation({
    mutationFn: async (slot: JsonRecord) => {
      const id = str(slot.id, "");
      const payload: Record<string, unknown> = {
        label: str(slot.label, ""),
        role: str(slot.role, "primary"),
        provider: str(slot.provider, ""),
        model: str(slot.model, ""),
        base_url: str(slot.base_url, "") || null,
        enabled: !toBool(slot.enabled),
      };
      return await api.rawPut(`/models/${encodeURIComponent(id)}`, payload, {
        timeoutMs: 15000,
      });
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["models"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (e) => setError(errMessage(e)),
  });

  const hasTelegramToken = toBool(settings.has_telegram_token);
  const telegramDeliveryReady = toBool(settings.telegram_delivery_ready);
  const hasSlackBotToken = toBool(settings.has_slack_bot_token);
  const hasSlackSigningSecret = toBool(settings.has_slack_signing_secret);
  const slackDeliveryReady = toBool(settings.slack_delivery_ready);
  const hasDiscordBotToken = toBool(settings.has_discord_bot_token);
  const discordDeliveryReady = toBool(settings.discord_delivery_ready);
  const hasMatrixAccessToken = toBool(settings.has_matrix_access_token);
  const matrixDeliveryReady = toBool(settings.matrix_delivery_ready);
  const hasTeamsAccessToken = toBool(settings.has_teams_access_token);
  const teamsDeliveryReady = toBool(settings.teams_delivery_ready);
  const hasWhatsAppToken = toBool(settings.has_whatsapp_token);
  const whatsappDeliveryReady = toBool(settings.whatsapp_delivery_ready);
  const hasPrimaryApiKey = toBool(settings.has_api_key);
  const hasFallbackApiKey = toBool(settings.has_fallback_api_key);
  const embeddingsHasApiKey = toBool(settings.embeddings_has_api_key);
  const embeddingsStatus = str(settings.embeddings_status, "");
  const hiddenExternalEmbeddingsProvider = embeddingsProviderHiddenFromSettingsUi(
    str(settings.embeddings_provider, ""),
  );
  const embeddingsProvider = form.embeddings_provider || "local-hf";
  const telegramAllowedUsers = parseTelegramUsers(
    form.telegram_allowed_users_csv,
  );
  const whatsappAllowedNumbers = parseCsvList(
    form.whatsapp_allowed_numbers_csv,
  );
  const whatsappConfigReady =
    form.whatsapp_enabled &&
    (form.whatsapp_mode === "cloud_api"
      ? hasWhatsAppToken && !!form.whatsapp_phone_number_id.trim()
      : !!form.whatsapp_bridge_url.trim());
  const embeddingsIsLocal = embeddingsProvider === "local-hf";
  const embeddingsDisabled = embeddingsProvider === "disabled";
  const embeddingsIsOllama = embeddingsProvider === "ollama";
  const embeddingsIsExternal =
    embeddingsProvider === "openai-compatible" || embeddingsIsOllama;
  const dailyBriefDeliveryWarning = !form.daily_brief_enabled
    ? ""
    : form.daily_brief_channel === "telegram"
      ? !hasTelegramToken
        ? "Telegram is not configured yet."
        : !telegramDeliveryReady
          ? telegramAllowedUsers.length !== 1
            ? "Telegram proactive notifications are fail-closed until exactly one allowed user ID is configured."
            : "Telegram is connected, but proactive delivery is not ready yet."
          : ""
      : form.daily_brief_channel === "whatsapp"
        ? !whatsappConfigReady
          ? "WhatsApp is not configured yet."
          : !whatsappDeliveryReady
            ? whatsappAllowedNumbers.length !== 1
              ? "WhatsApp proactive notifications are fail-closed until exactly one allowed number is configured."
              : "WhatsApp is configured, but proactive delivery is not ready yet."
            : ""
        : form.daily_brief_channel === "slack"
          ? !hasSlackBotToken || !hasSlackSigningSecret
            ? "Slack is not configured yet."
            : !slackDeliveryReady
              ? "Slack is configured, but it still needs a signed webhook and delivery target."
              : ""
          : form.daily_brief_channel === "discord"
            ? !hasDiscordBotToken
              ? "Discord is not configured yet."
              : !discordDeliveryReady
                ? "Discord is configured, but it still needs a guild, channel, or thread scope."
                : ""
            : form.daily_brief_channel === "matrix"
              ? !hasMatrixAccessToken
                ? "Matrix is not configured yet."
                : !matrixDeliveryReady
                  ? "Matrix is configured, but it still needs a room binding."
                  : ""
              : form.daily_brief_channel === "teams"
                ? !hasTeamsAccessToken
                  ? "Teams is not configured yet."
                  : !teamsDeliveryReady
                    ? "Teams is configured, but it still needs a reply destination."
                    : ""
                : form.daily_brief_channel.startsWith("custom.") ||
                    form.daily_brief_channel.startsWith("ext.")
                  ? availableDeliveryChannels.some(
                      (channel) => channel.id === form.daily_brief_channel,
                    )
                    ? ""
                    : "The selected custom delivery channel is not ready yet."
                : "";
  const settingsComplete =
    stableSettingsComplete ||
    toBool(settings.settings_complete) ||
    modelSlots.length > 0;
  const showSetupRequired =
    !settingsComplete &&
    !settingsPayloadError &&
    !modelsPayloadError &&
    settingsQ.isSuccess &&
    modelsQ.isSuccess &&
    !settingsQ.isFetching &&
    !modelsQ.isFetching;
  const modelsRefreshIssue =
    modelsPayloadError || (modelsQ.isError ? errMessage(modelsQ.error) : "");
  const showingModelFallback =
    modelSlotsLive.length === 0 &&
    stableModelSlots.length > 0 &&
    (modelsQ.isFetching || !!modelsRefreshIssue);
  const activeSettingsDataRefreshing =
    !standalonePulse &&
    !settingsQ.isLoading &&
    !(needsMediaSettings && mediaQ.isLoading && mediaQ.data == null) &&
    !(needsModelSettings && modelsQ.isLoading && modelSlots.length === 0) &&
    (settingsQ.isFetching ||
      (needsMediaSettings && mediaQ.isFetching) ||
      (needsModelSettings && modelsQ.isFetching));
  const activeSettingsDataError =
    settingsQ.error ||
    (needsMediaSettings ? mediaQ.error : null) ||
    (needsModelSettings ? modelsQ.error : null);

  const apiKeyPayload = asRecord(apiKeyQ.data);
  const settingsAutonomyPayload = asRecord(settingsAutonomyQ.data);
  const settingsAutonomy = asRecord(settingsAutonomyPayload.settings);
  const settingsEvolution = asRecord(settingsEvolutionQ.data);
  const settingsReadinessPolicy = asRecord(settingsEvolution.readiness_policy);
  const settingsSentinelPayload = asRecord(settingsSentinelQ.data);
  const settingsSentinel = asRecord(settingsSentinelPayload.settings);
  const settingsAutonomyModeRaw = str(
    settingsAutonomy.autonomy_mode,
    "assist",
  ).toLowerCase();
  const settingsSentinelEnabled =
    settingsSentinel.enabled == null ? true : toBool(settingsSentinel.enabled);
  const settingsAutonomyMode =
    settingsAutonomyModeRaw === "auto" || settingsAutonomyModeRaw === "assist"
      ? settingsAutonomyModeRaw
      : "assist";
  const settingsAutonomyPaused =
    Boolean(settingsAutonomy.agent_paused ?? false) ||
    settingsAutonomyModeRaw === "off";
  const settingsAutonomyModeLabel = settingsAutonomyPaused
    ? "Paused"
    : settingsAutonomyMode === "auto"
      ? "Auto"
      : "Assist";
  const settingsSelfEvolveEnabled =
    settingsEvolution.self_evolve_enabled == null
      ? true
      : toBool(settingsEvolution.self_evolve_enabled);
  const settingsDefaultGuardEnabled =
    settingsEvolution.deploy_guard_default == null
      ? true
      : toBool(settingsEvolution.deploy_guard_default);
  const settingsReadinessPolicySignature = JSON.stringify(
    settingsReadinessPolicy,
  );
  useEffect(() => {
    setReadinessPolicyDraft(readinessPolicyToDraft(settingsReadinessPolicy));
  }, [settingsReadinessPolicySignature]);
  const apiKeyIssuedAtUnix = num(apiKeyPayload.issued_at_unix, 0);
  const apiKeyExpiresAtUnix = num(apiKeyPayload.expires_at_unix, 0);
  const apiKeyRemainingFromServer = num(apiKeyPayload.remaining_seconds, 0);
  const apiKeyRemainingSeconds = useMemo(() => {
    if (apiKeyExpiresAtUnix > 0) {
      return Math.max(0, apiKeyExpiresAtUnix - Math.floor(apiKeyNowMs / 1000));
    }
    return Math.max(0, apiKeyRemainingFromServer);
  }, [apiKeyExpiresAtUnix, apiKeyNowMs, apiKeyRemainingFromServer]);
  const apiKeyRotated = toBool(apiKeyPayload.rotated);
  const tunnel = asRecord(tunnelQ.data);
  const tunnelProvidersPayload = asRecord(tunnelProvidersQ.data);
  const tunnelProviders = pickRecords(tunnelProvidersPayload, "providers");
  const serverSelectedTunnelProviderId = str(
    tunnelProvidersPayload.selected_provider,
    str(tunnel.provider, "cloudflare"),
  );
  const activeTunnelProviderId = str(
    tunnelProvidersPayload.active_provider,
    str(tunnel.provider, "cloudflare"),
  ).trim();
  const selectedTunnelProviderRecord =
    tunnelProviders.find(
      (provider) => str(provider.id, "") === tunnelSelectedProviderId,
    ) ||
    tunnelProviders.find(
      (provider) => str(provider.id, "") === serverSelectedTunnelProviderId,
    ) ||
    tunnelProviders[0] ||
    null;
  const selectedTunnelConfigFields = selectedTunnelProviderRecord
    ? pickRecords(selectedTunnelProviderRecord, "config_fields")
    : [];
  const selectedTunnelStoredSecretFields = Array.isArray(
    selectedTunnelProviderRecord?.stored_secret_fields,
  )
    ? (selectedTunnelProviderRecord?.stored_secret_fields as unknown[]).filter(
        (value): value is string =>
          typeof value === "string" && value.trim().length > 0,
      )
    : [];
  const selectedTunnelAvailable = toBool(
    selectedTunnelProviderRecord?.available,
  );
  const selectedTunnelConfigured = toBool(
    selectedTunnelProviderRecord?.configured,
  );
  const selectedTunnelMeta = getTunnelAccessMeta(selectedTunnelProviderRecord);
  const activeTunnelMeta = getTunnelAccessMeta(tunnel);
  const tunnelProviderOptions = useMemo(() => {
    if (showTunnelAdvanced) return tunnelProviders;
    const keepIds = new Set(
      [
        tunnelSelectedProviderId,
        serverSelectedTunnelProviderId,
        activeTunnelProviderId,
      ]
        .map((value) => value.trim())
        .filter((value) => value.length > 0),
    );
    const preferred = tunnelProviders.filter((provider) => {
      const id = str(provider.id, "").trim();
      return keepIds.has(id) || toBool(provider.available);
    });
    return preferred.length > 0 ? preferred : tunnelProviders;
  }, [
    activeTunnelProviderId,
    serverSelectedTunnelProviderId,
    showTunnelAdvanced,
    tunnelProviders,
    tunnelSelectedProviderId,
  ]);
  const selectedTunnelHelp =
    str(selectedTunnelProviderRecord?.config_help, "").trim() ||
    getTunnelProviderHelp(selectedTunnelMeta);
  const basicTunnelConfigFields = selectedTunnelConfigFields.filter(
    (field) => str(field.key, "").trim() !== "binary_path",
  );
  const advancedTunnelConfigFields = selectedTunnelConfigFields.filter(
    (field) => str(field.key, "").trim() === "binary_path",
  );
  const selectedTunnelId = str(selectedTunnelProviderRecord?.id, "").trim();
  const selectedTunnelHasAuthKey =
    selectedTunnelStoredSecretFields.includes("auth_key") ||
    Boolean(str(tunnelDraftValues.auth_key, "").trim());
  const selectedTunnelNeedsAuthKeyHint =
    (selectedTunnelId === "tailscale_private" ||
      selectedTunnelId === "tailscale_funnel") &&
    !selectedTunnelHasAuthKey;
  const selectedTunnelLabel = str(
    selectedTunnelProviderRecord?.label,
    str(tunnel.provider_label, "Tunnel"),
  );
  const tunnelSummaryTone: "success" | "warning" | "info" = toBool(
    tunnel.active,
  )
    ? "success"
    : !selectedTunnelAvailable || !selectedTunnelConfigured
      ? "warning"
      : "info";
  const tunnelGuidanceText = !selectedTunnelAvailable
    ? selectedTunnelMeta.isPrivate
      ? "Private tailnet access needs Tailscale on the same runtime as AgentArk. In Docker, the container needs the Tailscale CLI plus a running tailscaled or TS_SOCKET mount."
      : selectedTunnelId === "tailscale_funnel"
        ? "Tailscale Funnel needs Tailscale on the same runtime as AgentArk. In Docker, the container needs the Tailscale CLI plus a running tailscaled or TS_SOCKET mount."
        : "This tunnel provider is not available on this runtime yet."
    : selectedTunnelMeta.isPrivate
      ? "Private access only works from devices already connected to your tailnet. It does not create a public internet URL."
      : str(selectedTunnelProviderRecord?.id, "") === "bore"
        ? "Bore is a raw TCP tunnel. It is fine for app sharing, but the main AgentArk app works best with an HTTPS provider such as Cloudflare, ngrok, or Tailscale Funnel."
        : selectedTunnelHelp;
  const sec = asRecord(securityStatusQ.data);

  useEffect(() => {
    if (tunnelProviders.length === 0) return;
    const currentValid = tunnelProviders.some(
      (provider) => str(provider.id, "").trim() === tunnelSelectedProviderId,
    );
    if (!currentValid) {
      syncTunnelDraftFromPayload(tunnelProvidersPayload);
    }
  }, [tunnelProviders, tunnelProvidersPayload, tunnelSelectedProviderId]);

  useEffect(() => {
    setShowTunnelAdvanced(false);
    setTunnelSetupChecks([]);
    setTunnelPanelNotice(null);
  }, [tunnelSelectedProviderId, serverSelectedTunnelProviderId]);

  const usingDefaultMasterPassword = toBool(sec.using_default);
  const hasCustomMasterPassword =
    toBool(sec.master_password_set) && !usingDefaultMasterPassword;
  const internalServiceTokens = pickRecords(sec, "internal_service_tokens");
  const internalServiceRotationSupported =
    toBool(sec.internal_service_rotation_supported) &&
    internalServiceTokens.length > 0;
  const internalServiceEnvManaged = internalServiceTokens.some((row) =>
    toBool(row.managed_by_env),
  );
  const showInternalServiceSection =
    internalServiceTokens.length > 0 && !internalServiceEnvManaged;
  const internalServiceDescription = internalServiceRotationSupported
    ? "Rotate internal credentials when needed."
    : "Service-to-service credentials.";
  const vaultSecrets = pickRecords(vaultSecretsQ.data, "entries");
  const tunnelNeedsPassword = !hasCustomMasterPassword;
  const tunnelAccessLabel = selectedTunnelMeta.isPrivate
    ? "Private access"
    : "Public access";
  const tunnelStateLabel = toBool(tunnel.active) ? "Live" : "Off";
  const tunnelPrimaryText = toBool(tunnel.active)
    ? `${selectedTunnelLabel} is live and ready.`
    : !selectedTunnelAvailable
      ? `${selectedTunnelLabel} is not available on this AgentArk runtime yet.`
      : !selectedTunnelConfigured
        ? `${selectedTunnelLabel} needs setup before it can start.`
        : tunnelNeedsPassword
          ? "Set a custom password first to enable remote access."
          : `${selectedTunnelLabel} is ready to start.`;
  const tunnelPrimaryDetail = !selectedTunnelAvailable
    ? tunnelGuidanceText
    : !selectedTunnelConfigured
      ? selectedTunnelHelp
      : toBool(tunnel.active)
        ? selectedTunnelMeta.isPrivate
          ? "Only your connected devices can access this link."
          : "Anyone with the link can see the sign-in page. They still need your password to log in."
        : tunnelNeedsPassword
          ? ""
          : selectedTunnelNeedsAuthKeyHint
            ? "Save a Tailscale auth key first if this runtime isn't signed in yet."
            : selectedTunnelMeta.isPrivate
              ? "Only your connected devices will be able to reach it."
              : "Password-protected sign-in page will be available at the public link.";
  const vaultSummaryText = hasCustomMasterPassword
    ? `${vaultSecrets.length} stored secret${vaultSecrets.length === 1 ? "" : "s"}. Password is required only for protected add or delete actions.`
    : `${vaultSecrets.length} stored secret${vaultSecrets.length === 1 ? "" : "s"}. Secrets stay encrypted even without a custom password.`;
  const pulseEvents = useMemo(() => {
    if (!shouldDerivePulseData) return [];
    return pickRecords(pulseQ.data, "events").sort((a, b) => {
      const aTs = Date.parse(str(a.timestamp, ""));
      const bTs = Date.parse(str(b.timestamp, ""));
      return (Number.isFinite(bTs) ? bTs : 0) - (Number.isFinite(aTs) ? aTs : 0);
    });
  }, [pulseQ.data, shouldDerivePulseData]);
  const pulseMeta = useMemo(
    () => (shouldDerivePulseData ? asRecord(pulseQ.data) : {}),
    [pulseQ.data, shouldDerivePulseData],
  );
  const pulseRunning = toBool(pulseMeta.running);
  const pulseHistoryUnavailable = toBool(pulseMeta.history_unavailable);
  const pulseHistoryUnavailableReason = str(
    pulseMeta.history_unavailable_reason,
    "",
  ).trim();
  const latestPulseEventId = useMemo(
    () => str(asRecord(pulseEvents[0]).id, ""),
    [pulseEvents],
  );

  const selectedPulseDetails = useMemo(
    () => asRecord(selectedPulseEvent?.details),
    [selectedPulseEvent],
  );
  const selectedPulseFindings = useMemo(
    () =>
      pickRecords(selectedPulseDetails, "doctor_findings")
        .map((row, findingIndex) => ({ row, findingIndex }))
        .filter((finding) => isUserActionableDoctorFinding(finding.row)),
    [selectedPulseDetails],
  );
  const selectedPulseScore = num(selectedPulseDetails.doctor_score, -1);
  const selectedPulseStatus = str(selectedPulseEvent?.status, "-");
  const selectedPulseStatusOk = selectedPulseStatus.toLowerCase() === "ok";
  const selectedPulseTimestampRaw = str(selectedPulseEvent?.timestamp, "-");
  const selectedPulseCaptured = looksLikeIsoTimestamp(selectedPulseTimestampRaw)
    ? formatTimestampForHumans(selectedPulseTimestampRaw)
    : { label: selectedPulseTimestampRaw, tooltip: selectedPulseTimestampRaw };
  const selectedPulseGuidance = (() => {
    if (
      selectedPulseFindings.length === 0 &&
      (selectedPulseStatusOk || selectedPulseScore >= 90)
    ) {
      return {
        severity: "success" as const,
        title: "System health looks good.",
        detail: "No active issues were detected in this run.",
      };
    }
    if (selectedPulseFindings.length > 0) {
      const issueLabel =
        selectedPulseFindings.length === 1 ? "issue" : "issues";
      return {
        severity: "warning" as const,
        title: `${selectedPulseFindings.length} ${issueLabel} need attention.`,
        detail:
          "Run only verified Pulse actions; findings without a runnable remediation are manual follow-up.",
      };
    }
    return {
      severity: "info" as const,
      title: "No direct findings were returned.",
      detail:
        "Review the snapshot for context and run another check after changes.",
    };
  })();
  const selectedPulseSnapshot: { label: string; value: string }[] = [
    {
      label: "Pending tasks",
      value: String(num(selectedPulseDetails.pending_tasks, 0)),
    },
    {
      label: "Running tasks",
      value: String(num(selectedPulseDetails.running_tasks, 0)),
    },
    {
      label: "Completed tasks",
      value: String(num(selectedPulseDetails.completed_tasks, 0)),
    },
    {
      label: "Deployed apps",
      value: String(
        Array.isArray(selectedPulseDetails.deployed_apps)
          ? selectedPulseDetails.deployed_apps.length
          : 0,
      ),
    },
    {
      label: "Health checks",
      value: String(
        Array.isArray(selectedPulseDetails.health_checks)
          ? selectedPulseDetails.health_checks.length
          : 0,
      ),
    },
    {
      label: "Memories",
      value: String(num(selectedPulseDetails.total_memories, 0)),
    },
    {
      label: "Watchers",
      value: String(num(selectedPulseDetails.active_watchers, 0)),
    },
    {
      label: "Uptime",
      value: formatDurationFromSeconds(selectedPulseDetails.uptime_secs),
    },
  ];
  const selectedPulseScanLog = pickRecords(selectedPulseDetails, "scan_log");
  const selectedPulseScanStarted = str(
    selectedPulseDetails.scan_started_at,
    "",
  ).trim();
  const selectedPulseScanFinished = str(
    selectedPulseDetails.scan_finished_at,
    "",
  ).trim();
  const selectedPulseScanDurationMs = num(
    selectedPulseDetails.scan_duration_ms,
    0,
  );
  const selectedPulseNotificationOutcome = str(
    selectedPulseDetails.notification_outcome,
    "",
  ).trim();
  const selectedPulseHeroIcon =
    selectedPulseGuidance.severity === "success" ? (
      <CheckCircleRoundedIcon sx={{ fontSize: 22 }} />
    ) : selectedPulseGuidance.severity === "warning" ? (
      <ErrorOutlineRoundedIcon sx={{ fontSize: 22 }} />
    ) : (
      <InfoOutlinedIcon sx={{ fontSize: 22 }} />
    );
  const selectedPulsePrimaryStats = [
    {
      label: "Health score",
      value: selectedPulseScore >= 0 ? String(selectedPulseScore) : "-",
      helper:
        selectedPulseScore >= 90
          ? "Healthy run"
          : selectedPulseFindings.length > 0
            ? "Needs follow-up"
            : "Score unavailable",
    },
    {
      label: "Findings",
      value: String(selectedPulseFindings.length),
      helper:
        selectedPulseFindings.length === 0
          ? "Nothing urgent"
          : `${selectedPulseFindings.length} item${selectedPulseFindings.length === 1 ? "" : "s"} to review`,
    },
    {
      label: "Watchers",
      value: String(num(selectedPulseDetails.active_watchers, 0)),
      helper: "Active background monitors",
    },
  ];
  const latestPulseEvent = asRecord(pulseEvents[0]);
  const latestPulseDetails = asRecord(latestPulseEvent.details);
  const latestPulseFindingsCount = pickRecords(
    latestPulseDetails,
    "doctor_findings",
  ).filter((f) => isUserActionableDoctorFinding(f)).length;
  const latestPulseScore = num(latestPulseDetails.doctor_score, -1);
  const latestPulseStatus = str(latestPulseEvent.status, "").toLowerCase();
  const latestPulseHeadline = pulseRunning
    ? "Pulse is currently running."
    : pulseEvents.length === 0
      ? pulseHistoryUnavailable
        ? "Earlier Pulse history is unavailable."
        : "No health checks yet."
      : latestPulseFindingsCount > 0
        ? `${latestPulseFindingsCount} issue${latestPulseFindingsCount === 1 ? "" : "s"} need attention.`
        : latestPulseStatus === "ok" || latestPulseScore >= 90
          ? "System health looks good."
          : "Health check completed.";
  const latestPulseSubtitle = pulseRunning
    ? "Please wait for this run to finish before starting another."
    : pulseEvents.length === 0
      ? pulseHistoryUnavailable
        ? pulseHistoryUnavailableReason ||
          "A previous Pulse payload exists, but this runtime could not decrypt it. New runs will appear normally."
        : "Click Run now to generate your first diagnostics report."
      : latestPulseFindingsCount > 0
        ? "Open the latest report and start with Fix #1."
        : "No urgent action needed right now.";
  const latestPulseNavCount =
    latestPulseFindingsCount > 0
      ? latestPulseFindingsCount
      : latestPulseStatus === "alert"
        ? 1
        : 0;

  useEffect(() => {
    if (!pulsePollState) return;
    if (Date.now() >= pulsePollState.deadlineAt) {
      setPulsePollState(null);
      return;
    }
    if (
      !pulseRunning &&
      latestPulseEventId &&
      latestPulseEventId !== pulsePollState.baselineEventId
    ) {
      setPulsePollState(null);
    }
  }, [pulsePollState, pulseRunning, latestPulseEventId]);

  function severityChipColor(
    sev: string,
  ): "error" | "warning" | "info" | "success" | "default" {
    const s = (sev || "").toLowerCase();
    if (s === "critical" || s === "high" || s === "error") return "error";
    if (s === "medium" || s === "warn" || s === "warning") return "warning";
    if (s === "low") return "info";
    if (s === "ok" || s === "info") return "success";
    return "default";
  }

  function pulseScanStatusColor(
    status: string,
  ): "error" | "warning" | "info" | "success" | "default" {
    const normalized = (status || "").trim().toLowerCase();
    if (
      normalized === "error" ||
      normalized === "critical" ||
      normalized === "high"
    )
      return "error";
    if (
      normalized === "warning" ||
      normalized === "warn" ||
      normalized === "medium"
    )
      return "warning";
    if (
      normalized === "ok" ||
      normalized === "success" ||
      normalized === "completed"
    )
      return "success";
    if (normalized === "running" || normalized === "info") return "info";
    return "default";
  }

  function pulseScanStatusLabel(status: string): string {
    const normalized = (status || "").trim().toLowerCase();
    if (!normalized) return "Unknown";
    if (normalized === "ok") return "OK";
    return normalized
      .replace(/_/g, " ")
      .replace(/\b\w/g, (m) => m.toUpperCase());
  }

  function pulseScanAlertReason(row: JsonRecord): string {
    const normalizedStatus = str(row.status, "").trim().toLowerCase();
    if (
      !["warning", "warn", "error", "critical", "high", "medium"].includes(
        normalizedStatus,
      )
    ) {
      return "";
    }
    const metricReasons = pickRecords(row, "metrics")
      .map((metric) => {
        const metricRow = asRecord(metric);
        const label = collapseInlineWhitespace(
          str(metricRow.label, "").replace(/:$/, ""),
        );
        const value = collapseInlineWhitespace(str(metricRow.value, ""));
        if (!label || !value) return "";
        const normalizedLabel = label.toLowerCase();
        const numericMatch = value.match(/-?\d+(?:\.\d+)?/);
        const numericValue = numericMatch
          ? Number(numericMatch[0])
          : Number.NaN;
        const isPositiveCount =
          Number.isFinite(numericValue) && numericValue > 0;
        if (
          [
            "injections",
            "auth failures",
            "rate limits",
            "unauthorized channels",
            "overdue",
            "failed",
          ].includes(normalizedLabel) &&
          isPositiveCount
        ) {
          return `${titleCaseLabel(label)}: ${value}`;
        }
        if (
          normalizedLabel === "highest severity" &&
          !["none", "info", "ok", "success"].includes(value.toLowerCase())
        ) {
          return `${titleCaseLabel(label)}: ${value}`;
        }
        if (
          ["findings", "actionable"].includes(normalizedLabel) &&
          isPositiveCount
        ) {
          return `${titleCaseLabel(label)}: ${value}`;
        }
        return "";
      })
      .filter(Boolean);
    if (metricReasons.length > 0) {
      return metricReasons.slice(0, 2).join(" | ");
    }
    const detail = collapseInlineWhitespace(str(row.detail, ""));
    if (detail) return truncateUiText(detail, 150);
    const summary = collapseInlineWhitespace(str(row.summary, ""));
    return summary ? truncateUiText(summary, 150) : "";
  }

  async function copyClipboardText(value: string): Promise<void> {
    const text = value.trim();
    if (!text) throw new Error("Nothing to copy.");
    const nav = typeof navigator !== "undefined" ? navigator : null;
    if (nav?.clipboard?.writeText) {
      await nav.clipboard.writeText(text);
      return;
    }
    const doc = typeof document !== "undefined" ? document : null;
    if (!doc) throw new Error("Clipboard is not available.");
    const ta = doc.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.left = "-9999px";
    doc.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = doc.execCommand("copy");
    doc.body.removeChild(ta);
    if (!ok) throw new Error("Copy failed.");
  }

  const regenerateApiKeyMutation = useMutation({
    mutationFn: () => api.rawPost("/settings/api-key/regenerate", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-api-key"] });
    },
    onError: (e) => setError(errMessage(e)),
  });

  async function refreshTunnelQueries() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["tunnel-status"] }),
      queryClient.invalidateQueries({ queryKey: ["tunnel-providers"] }),
      queryClient.invalidateQueries({
        queryKey: ["apps-manager-tunnel-status"],
      }),
      queryClient.invalidateQueries({ queryKey: ["chat-workspace-tunnel"] }),
    ]);
  }

  function scheduleTunnelRefreshBurst() {
    for (const delayMs of [1500, 4000, 8000]) {
      window.setTimeout(() => {
        void refreshTunnelQueries();
      }, delayMs);
    }
  }

  const tunnelSaveMutation = useMutation({
    mutationFn: (payload: {
      provider: string;
      values: Record<string, string>;
    }) => api.rawPost("/tunnel/configure", payload),
    onSuccess: async (raw, variables) => {
      const response = asRecord(raw);
      await refreshTunnelQueries();
      syncTunnelDraftFromPayload(response.settings, variables.provider);
    },
    onError: (e) => setError(errMessage(e)),
  });

  const tunnelTestMutation = useMutation({
    mutationFn: (payload: { provider: string }) =>
      api.rawPost("/tunnel/test", payload),
    onError: (e) => setError(errMessage(e)),
  });

  const tunnelStartMutation = useMutation({
    mutationFn: (payload: { provider?: string }) =>
      api.rawPost("/tunnel/start", payload),
    onSuccess: async () => {
      await refreshTunnelQueries();
    },
    onError: (e) => setError(errMessage(e)),
  });

  const tunnelStopMutation = useMutation({
    mutationFn: () => api.rawPost("/tunnel/stop", {}),
    onSuccess: async () => {
      await refreshTunnelQueries();
    },
    onError: (e) => setError(errMessage(e)),
  });

  async function saveSelectedTunnelProviderSettings() {
    const provider =
      tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId;
    if (!provider) throw new Error("Choose a tunnel provider first.");
    const response = asRecord(
      await tunnelSaveMutation.mutateAsync({
        provider,
        values: tunnelDraftValues,
      }),
    );
    return response;
  }

  async function performTunnelStart() {
    await saveSelectedTunnelProviderSettings();
    const response = asRecord(
      await tunnelStartMutation.mutateAsync({
        provider:
          tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId,
      }),
    );
    const startedUrl = str(response.url, "").trim();
    const startMessage = startedUrl
      ? getTunnelPanelStartMessage(selectedTunnelMeta, startedUrl)
      : getTunnelPanelStartingMessage(selectedTunnelMeta);
    setTunnelPanelNotice({
      severity: startedUrl ? "success" : "info",
      text: startMessage,
    });
    if (!startedUrl) {
      scheduleTunnelRefreshBurst();
    }
    setSuccess(str(response.message, "Tunnel start requested."));
  }

  async function maybeResumeTunnelStartAfterPassword() {
    if (!resumeTunnelStartAfterPassword) return;
    setResumeTunnelStartAfterPassword(false);
    setTunnelPanelNotice({
      severity: "info",
      text: getTunnelPanelResumeMessage(selectedTunnelMeta),
    });
    try {
      await performTunnelStart();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelProviderSave() {
    setError(null);
    try {
      const response = await saveSelectedTunnelProviderSettings();
      setTunnelPanelNotice({
        severity: "success",
        text: str(response.message, "Tunnel settings saved."),
      });
      setSuccess(str(response.message, "Tunnel settings saved."));
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelProviderTest() {
    setError(null);
    try {
      await saveSelectedTunnelProviderSettings();
      const response = asRecord(
        await tunnelTestMutation.mutateAsync({
          provider:
            tunnelSelectedProviderId.trim() || serverSelectedTunnelProviderId,
        }),
      );
      const ok = toBool(response.ok);
      const checks = pickRecords(response, "checks");
      setTunnelSetupChecks(checks);
      const message = str(
        response.message,
        ok
          ? "Tunnel provider test passed."
          : "Tunnel provider still needs setup.",
      ).trim();
      const detail = str(response.detail, "").trim();
      setTunnelPanelNotice({
        severity: ok ? "success" : "warning",
        text: detail ? `${message} ${detail}` : message,
      });
      if (ok) {
        setSuccess(detail ? `${message} ${detail}` : message);
      }
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelStart() {
    setError(null);
    if (!hasCustomMasterPassword) {
      setResumeTunnelStartAfterPassword(true);
      setTunnelPanelNotice({
        severity: "info",
        text: getTunnelPanelPasswordPrompt(selectedTunnelMeta),
      });
      openPasswordDialog("set");
      return;
    }
    try {
      await performTunnelStart();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTunnelStop() {
    setError(null);
    try {
      const response = asRecord(await tunnelStopMutation.mutateAsync());
      setTunnelPanelNotice({
        severity: "info",
        text: str(
          response.message,
          activeTunnelMeta.isPrivate
            ? "Private access stopped."
            : "Public link stopped.",
        ),
      });
      setSuccess(str(response.message, "Tunnel stopped."));
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const updateStatus = updateStatusQ.data?.update ?? null;
  const updateCheckedAtLabel = updateStatus?.checked_at
    ? formatUiDateTime(updateStatus.checked_at)
    : null;

  const restartMutation = useMutation({
    mutationFn: () => api.rawPost("/restart", {}),
    onSuccess: () => {
      setSuccess(null);
      setRestartNotice({
        text: "AgentArk is restarting. Give it up to 10 seconds. The page will refresh automatically when it is ready.",
        durationMs: RESTART_NOTICE_DURATION_MS,
        etaLabel: "Up to 10 seconds",
      });
    },
    onError: (e) => {
      setRestartNotice(null);
      setError(errMessage(e));
    },
  });
  const settingsAutonomyMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/autonomy/settings", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings-dashboard"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
    onError: (e) => setError(errMessage(e)),
  });
  const settingsEvolutionMutation = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/settings/evolution", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-evolution"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-evolution-dev"],
      });
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
    },
    onError: (e) => setError(errMessage(e)),
  });
  const settingsSentinelMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/autonomy/sentinel/settings", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["sentinel-settings"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-autonomy-settings"],
      });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["sentinel-feed"] });
    },
    onError: (e) => setError(errMessage(e)),
  });

  const updateAgentArkMutation = useMutation({
    mutationFn: () => api.rawPost("/update", {}),
    onSuccess: async (raw) => {
      const response = asRecord(raw);
      setError(null);
      setSuccess(null);
      setRestartNotice({
        text: str(
          response.message,
          "AgentArk is updating and restarting. Pending work can be interrupted during the restart.",
        ),
        durationMs: UPDATE_NOTICE_DURATION_MS,
        etaLabel: "Up to 2 minutes",
      });
      await queryClient.invalidateQueries({ queryKey: ["server-ping"] });
    },
    onError: (e) => {
      setRestartNotice(null);
      setError(errMessage(e));
    },
  });

  const triggerPulseMutation = useMutation({
    mutationFn: () => api.rawPost("/arkpulse/trigger", {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["arkpulse-log"] });
    },
    onError: (e) => setError(errMessage(e)),
  });

  const runPulseFixMutation = useMutation({
    mutationFn: async (payload: {
      fixCommand: string;
      remediation?: PulseRemediationSpec | null;
      issueTitle: string;
      target: string;
      eventTimestamp?: string;
      findingIndex?: number;
    }) => {
      const body: PulseRunFixRequest = {
        issue_title: payload.issueTitle,
        target: payload.target,
        event_timestamp: payload.eventTimestamp || undefined,
        finding_index: payload.findingIndex,
      };
      if (!payload.eventTimestamp || !Number.isFinite(payload.findingIndex)) {
        const fixCommand = payload.fixCommand.trim();
        if (fixCommand) {
          body.fix_command = fixCommand;
        }
        if (payload.remediation) {
          body.remediation = payload.remediation;
        }
      }
      const out = asRecord(await api.rawPost("/arkpulse/fix", body));
      const status = str(out.status, "").toLowerCase();
      if (status === "error") {
        const errorText =
          str(out.error, "").trim() ||
          str(out.message, "").trim() ||
          "Pulse fix failed.";
        throw new Error(errorText);
      }
      return out;
    },
    onSuccess: async (raw) => {
      const message = str(raw.message, "").trim();
      const output = str(raw.output, "").trim();
      if (message && output) {
        setSuccess(`${message}\n\n${output}`);
      } else if (message) {
        setSuccess(message);
      } else {
        setSuccess("Pulse fix completed.");
      }
      setSelectedPulseEvent(null);
      await queryClient.invalidateQueries({ queryKey: ["arkpulse-log"] });
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
      await queryClient.invalidateQueries({
        queryKey: ["chat-workspace-tunnel"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["chat-workspace-apps"],
      });
    },
    onError: (e) => setError(errMessage(e)),
  });
  const decideAbuseReviewMutation = useMutation({
    mutationFn: (payload: { sourceKeyHash: string; decision: "approve" | "reject" }) =>
      api.rawPost(
        `/security/abuse-reviews/${encodeURIComponent(payload.sourceKeyHash)}/${payload.decision}`,
        {},
      ),
    onSuccess: async (_raw, payload) => {
      await queryClient.invalidateQueries({
        queryKey: ["security-abuse-reviews"],
      });
      await queryClient.invalidateQueries({ queryKey: ["security-logs"] });
      await queryClient.invalidateQueries({
        queryKey: ["settings-security-logs-dialog"],
      });
      setSuccess(
        payload.decision === "approve"
          ? "Security review resumed that source."
          : "Security review paused that source.",
      );
    },
    onError: (e) => setError(errMessage(e)),
  });

  const setPasswordMutation = useMutation({
    mutationFn: (password: string) =>
      api.rawPost("/security/set-password", { password }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      if (resumeTunnelStartAfterPassword) {
        await maybeResumeTunnelStartAfterPassword();
      } else {
        setSuccess("Custom password saved.");
      }
    },
    onError: (e) => setError(errMessage(e)),
  });

  const changePasswordMutation = useMutation({
    mutationFn: (payload: { current_password: string; new_password: string }) =>
      api.rawPost("/security/change-password", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      if (resumeTunnelStartAfterPassword) {
        await maybeResumeTunnelStartAfterPassword();
      } else {
        setSuccess("Custom password updated.");
      }
    },
    onError: (e) => setError(errMessage(e)),
  });

  const removePasswordMutation = useMutation({
    mutationFn: (password: string) =>
      api.rawPost("/security/remove-password", { password }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      await refreshTunnelQueries();
      setSuccess("Custom password removed.");
      setSecCurrentPassword("");
      setSecNewPassword("");
      setSecConfirmPassword("");
      setResumeTunnelStartAfterPassword(false);
    },
    onError: (e) => setError(errMessage(e)),
  });

  const rotateInternalServiceTokensMutation = useMutation({
    mutationFn: () =>
      api.rawPost("/security/internal-service-tokens/rotate", {}),
    onSuccess: async (raw) => {
      const response = asRecord(raw);
      await queryClient.invalidateQueries({ queryKey: ["security-status"] });
      setSuccess(null);
      setRestartNotice({
        text: str(
          response.message,
          "Internal service credentials rotated. AgentArk is restarting. The page will refresh automatically when it is ready.",
        ),
        durationMs: RESTART_NOTICE_DURATION_MS,
        etaLabel: "Up to 10 seconds",
      });
    },
    onError: (e) => setError(errMessage(e)),
  });

  const passwordMutationPending =
    setPasswordMutation.isPending ||
    changePasswordMutation.isPending ||
    removePasswordMutation.isPending;

  const upsertVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; value: string; password?: string }) =>
      api.rawPost("/settings/secrets/upsert", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setSuccess("Secret saved.");
    },
    onError: (e) => setError(errMessage(e)),
  });

  const deleteVaultSecretMutation = useMutation({
    mutationFn: (payload: { key: string; password?: string }) =>
      api.rawPost("/settings/secrets/delete", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-secrets"] });
      setSuccess("Secret deleted.");
    },
    onError: (e) => setError(errMessage(e)),
  });

  function resolveVaultPasswordForSensitiveOps(): string | null | undefined {
    if (!hasCustomMasterPassword) return undefined;
    const pw = vaultPassword.trim();
    if (!pw) {
      setError("Master password is required for secret changes.");
      return null;
    }
    return pw;
  }

  function openVaultEditor() {
    setError(null);
    setSuccess(null);
    setVaultEditorKey("");
    setVaultEditorValue("");
    setShowVaultSecretValue(false);
    setVaultEditorOpen(true);
  }

  function closeVaultEditor() {
    if (upsertVaultSecretMutation.isPending) return;
    setVaultEditorOpen(false);
    setVaultEditorKey("");
    setVaultEditorValue("");
    setShowVaultSecretValue(false);
  }

  async function submitVaultEditor() {
    const key = vaultEditorKey.trim();
    const value = vaultEditorValue;
    if (!key) {
      setError("Secret key is required.");
      return;
    }
    if (!value.trim()) {
      setError("Secret value is required.");
      return;
    }
    const pw = resolveVaultPasswordForSensitiveOps();
    if (pw === null) return;
    setError(null);
    try {
      await upsertVaultSecretMutation.mutateAsync({
        key,
        value,
        password: pw || undefined,
      });
      closeVaultEditor();
    } catch {
      // handled by mutation onError
    }
  }

  function resetPasswordInputs() {
    setSecCurrentPassword("");
    setSecNewPassword("");
    setSecConfirmPassword("");
    setShowPasswordInputs(false);
  }

  function openPasswordDialog(mode: PasswordDialogMode) {
    setError(null);
    setSuccess(null);
    resetPasswordInputs();
    setPasswordDialogMode(mode);
  }

  function closePasswordDialog() {
    if (passwordMutationPending) return;
    setPasswordDialogMode(null);
    setResumeTunnelStartAfterPassword(false);
    resetPasswordInputs();
  }

  function openRestartDialog() {
    setError(null);
    setSuccess(null);
    setRestartDialogOpen(true);
  }

  function closeRestartDialog() {
    if (restartMutation.isPending) return;
    setRestartDialogOpen(false);
  }

  function closeAutonomyPauseDialog() {
    if (settingsAutonomyMutation.isPending) return;
    setAutonomyPauseDialogOpen(false);
  }

  async function submitAutonomyPauseDialog() {
    setError(null);
    setSuccess(null);
    try {
      await settingsAutonomyMutation.mutateAsync({
        agent_paused: true,
        pause_mode: "autonomous_only",
      });
      setAutonomyPauseDialogOpen(false);
      setSuccess("Autonomy paused. Scheduled reminders still fire.");
    } catch {
      // handled by mutation onError
    }
  }

  async function handleResumeAutonomy() {
    setError(null);
    setSuccess(null);
    const nextMode =
      settingsAutonomyModeRaw === "auto" || settingsAutonomyModeRaw === "assist"
        ? settingsAutonomyModeRaw
        : "assist";
    try {
      await settingsAutonomyMutation.mutateAsync({
        autonomy_mode: nextMode,
        agent_paused: false,
        pause_mode: "autonomous_only",
      });
      setSuccess(`Autonomy resumed in ${nextMode} mode.`);
    } catch {
      // handled by mutation onError
    }
  }

  function closeSelfEvolveDisableDialog() {
    if (settingsEvolutionMutation.isPending) return;
    setSelfEvolveDisableDialogOpen(false);
  }

  function closeSentinelDisableDialog() {
    if (settingsSentinelMutation.isPending) return;
    setSentinelDisableDialogOpen(false);
  }

  function closeSentinelInAppDisableDialog() {
    if (settingsSentinelMutation.isPending) return;
    setSentinelInAppDisableDialogOpen(false);
  }

  async function updateSettingsEvolution(
    payload: JsonRecord,
    message: string,
  ): Promise<boolean> {
    setError(null);
    setSuccess(null);
    try {
      await settingsEvolutionMutation.mutateAsync(payload);
      setSuccess(message);
      return true;
    } catch {
      // handled by mutation onError
      return false;
    }
  }

  async function submitReadinessPolicyDraft() {
    let readinessPolicy: JsonRecord;
    try {
      readinessPolicy = parseReadinessPolicyDraft(readinessPolicyDraft);
    } catch (e) {
      setError(errMessage(e));
      return;
    }
    await updateSettingsEvolution(
      { readiness_policy: readinessPolicy },
      "Evolve readiness thresholds saved.",
    );
  }

  async function updateSettingsSentinel(
    payload: JsonRecord,
    message: string,
  ): Promise<boolean> {
    setError(null);
    setSuccess(null);
    try {
      await settingsSentinelMutation.mutateAsync(payload);
      setSuccess(message);
      return true;
    } catch {
      // handled by mutation onError
      return false;
    }
  }

  async function submitSentinelDisableDialog() {
    const changed = await updateSettingsSentinel(
      {
        enabled: false,
        watch_in_app: false,
        watch_connected_services: false,
        infer_new_automations: false,
      },
      settingsAutonomyPaused
        ? "Sentinel turned off. Its signal switches will stay off after autonomy resumes until you turn them back on."
        : "Sentinel turned off. Its signal switches are off until you turn them back on.",
    );
    if (changed) {
      setSentinelDisableDialogOpen(false);
    }
  }

  async function submitSentinelInAppDisableDialog() {
    const changed = await updateSettingsSentinel(
      { watch_in_app: false },
      "Sentinel will ignore in-app AgentArk activity until you turn it back on.",
    );
    if (changed) {
      setSentinelInAppDisableDialogOpen(false);
    }
  }

  async function handleEnableSelfEvolve() {
    await updateSettingsEvolution(
      { self_evolve_enabled: true },
      settingsAutonomyPaused
        ? "Self-evolve enabled. Background learning will resume when autonomy is active again."
        : "Self-evolve enabled. Background learning and reviewed improvements will resume.",
    );
  }

  async function submitSelfEvolveDisableDialog() {
    const changed = await updateSettingsEvolution(
      {
        self_evolve_enabled: false,
      },
      "Self-evolve turned off. Background learning and active canary experiments are paused.",
    );
    if (changed) {
      setSelfEvolveDisableDialogOpen(false);
    }
  }

  async function submitRestartDialog() {
    setError(null);
    setSuccess(null);
    setRestartNotice(null);
    try {
      await restartMutation.mutateAsync();
      closeRestartDialog();
      void monitorRestartRecovery();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  function openRotateInternalCredentialsDialog() {
    setError(null);
    setSuccess(null);
    setRotateInternalCredentialsAccepted(false);
    setRotateInternalCredentialsDialogOpen(true);
  }

  function closeRotateInternalCredentialsDialog() {
    if (rotateInternalServiceTokensMutation.isPending) return;
    setRotateInternalCredentialsDialogOpen(false);
    setRotateInternalCredentialsAccepted(false);
  }

  async function submitRotateInternalCredentials() {
    if (!rotateInternalCredentialsAccepted) return;
    setError(null);
    setSuccess(null);
    try {
      await rotateInternalServiceTokensMutation.mutateAsync();
      closeRotateInternalCredentialsDialog();
      void monitorRestartRecovery();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function submitPasswordDialog() {
    if (!passwordDialogMode) return;
    setError(null);
    setSuccess(null);
    try {
      if (passwordDialogMode === "set") {
        const pw = secNewPassword;
        if (pw.length < 8) {
          setError("Password must be at least 8 characters.");
          return;
        }
        if (pw !== secConfirmPassword) {
          setError("Passwords do not match.");
          return;
        }
        await setPasswordMutation.mutateAsync(pw);
      } else if (passwordDialogMode === "change") {
        const pw = secNewPassword;
        if (pw.length < 8) {
          setError("New password must be at least 8 characters.");
          return;
        }
        if (pw !== secConfirmPassword) {
          setError("Passwords do not match.");
          return;
        }
        await changePasswordMutation.mutateAsync({
          current_password: secCurrentPassword,
          new_password: pw,
        });
      } else if (passwordDialogMode === "remove") {
        await removePasswordMutation.mutateAsync(secCurrentPassword);
      }
      setPasswordDialogMode(null);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const selectedSettingsNav = getSelectedSettingsNav(tab, latestPulseNavCount);
  useEffect(() => {
    if (tab === 2 || tab === 10 || tab === 15) {
      setTab(20);
    }
  }, [tab]);

  const tabSupportsSave = settingsTabSupportsSave(tab);
  const selectedSettingsMeta = getSettingsPageMeta(tab);
  const settingsLoadingMessage = getSettingsTabLoadingMessage(tab);
  const selectedSettingsHeaderTitle =
    selectedSettingsMeta.title || selectedSettingsNav?.label || "Settings";
  const arkPulseHeader = (
    <WorkspacePageHeader
      eyebrow="Ark Core"
      title="Pulse"
      description="Setup health, integration checks, runtime drift, and repair actions."
      actions={
        <Button
          size="small"
          variant="contained"
          onClick={runArkPulseCheck}
          disabled={triggerPulseMutation.isPending || pulseRunning}
        >
          {triggerPulseMutation.isPending || pulseRunning
            ? "Running..."
            : "Run now"}
        </Button>
      }
    />
  );
  const arkPulsePageContent = (
    <Stack spacing={2}>
      {standalonePulse ? arkPulseHeader : null}
      <Grid2
        container
        spacing={2}
        sx={{
          alignItems: "stretch",
        }}
      >
        <Grid2 size={{ xs: 12 }}>
          <Box
            className="list-shell"
            sx={{
              minHeight: 0,
              height: "100%",
              display: "flex",
              flexDirection: "column",
            }}
          >
            {pulseQ.error ? (
              <Alert severity="error">{errMessage(pulseQ.error)}</Alert>
            ) : null}
            {!pulseQ.error ? (
              <Alert
                severity={
                  pulseRunning
                    ? "info"
                    : pulseHistoryUnavailable || latestPulseFindingsCount > 0
                      ? "warning"
                      : "success"
                }
                sx={{ mb: 1 }}
              >
                <Typography variant="subtitle2">{latestPulseHeadline}</Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  {latestPulseSubtitle}
                </Typography>
              </Alert>
            ) : null}
            {pulseEvents.length === 0 ? (
              <Stack spacing={1} sx={{ flex: 1 }}>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  {pulseHistoryUnavailable
                    ? "Stored Pulse history could not be loaded in this runtime."
                    : "No Pulse events yet."}
                </Typography>
                {renderSettingsInlineCard({
                  eyebrow: "Pulse",
                  title: "How this helps",
                  description:
                    "Pulse runs a health check for setup, integrations, safety, and runtime drift.",
                  tone: "info",
                  children: (
                    <Stack spacing={0.6}>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Run it after changing models, adding integrations, or
                        when something stops working.
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Example: if notifications stop arriving, Pulse can
                        point you to the broken setup step.
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Every run appears here with findings, suggested fixes,
                        and a health score.
                      </Typography>
                    </Stack>
                  ),
                })}
                <Box sx={{ flex: 1 }} />
              </Stack>
            ) : (
              <Stack
                spacing={0}
                sx={{
                  flex: 1,
                  minHeight: 0,
                  borderTop: "1px solid",
                  borderColor: "divider",
                }}
              >
                {pulseEvents.slice(0, 40).map((ev, idx) => {
                  const details = asRecord(ev.details);
                  const findings = pickRecords(
                    details,
                    "doctor_findings",
                  ).filter((f) => isUserActionableDoctorFinding(f));
                  const score = num(details.doctor_score, -1);
                  const status = str(ev.status, "-");
                  const ok = status.toLowerCase() === "ok";
                  const findingCount = Array.isArray(findings)
                    ? findings.length
                    : 0;
                  const summary = str(ev.summary, "").trim();
                  const message = str(ev.message, "").trim();
                  const overdue = num(ev.overdue_tasks, 0);
                  const failed = num(ev.failed_tasks, 0);
                  const headline =
                    summary || message || (ok ? "All systems healthy" : "Issues detected");
                  const metaParts: string[] = [];
                  if (score >= 0) metaParts.push(`Score ${score}`);
                  if (findingCount > 0)
                    metaParts.push(
                      `${findingCount} finding${findingCount === 1 ? "" : "s"}`,
                    );
                  if (overdue > 0) metaParts.push(`${overdue} overdue`);
                  if (failed > 0) metaParts.push(`${failed} failed`);
                  if (metaParts.length === 0) metaParts.push("No issues");
                  return (
                    <ButtonBase
                      key={str(ev.id, String(idx))}
                      onClick={() => setSelectedPulseEvent(ev)}
                      sx={{
                        width: "100%",
                        textAlign: "left",
                        justifyContent: "flex-start",
                        px: 0,
                        py: 0.85,
                        borderBottom: "1px solid",
                        borderColor: "divider",
                        transition: "background 0.15s ease",
                        "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
                        display: "block",
                      }}
                    >
                      <Stack
                        direction="row"
                        spacing={0.75}
                        useFlexGap
                        sx={{ alignItems: "center", justifyContent: "space-between" }}
                      >
                        <Stack
                          direction="row"
                          spacing={0.75}
                          useFlexGap
                          sx={{ alignItems: "center", minWidth: 0, flex: 1 }}
                        >
                          <Box
                            component="span"
                            sx={{
                              width: 7,
                              height: 7,
                              borderRadius: "50%",
                              flexShrink: 0,
                              bgcolor: ok
                                ? "var(--ui-rgba-74-210-157-850)"
                                : "var(--ui-rgba-255-180-60-850)",
                            }}
                          />
                          <Typography variant="subtitle2" noWrap sx={{ fontWeight: 600 }}>
                            {headline}
                          </Typography>
                        </Stack>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            whiteSpace: "nowrap",
                            flexShrink: 0,
                          }}
                          title={humanTs(str(ev.timestamp, "-")).tip}
                        >
                          {humanTs(str(ev.timestamp, "-")).label}
                        </Typography>
                      </Stack>
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary", pl: "15px", display: "block" }}
                      >
                        {metaParts.join(" / ")}
                      </Typography>
                    </ButtonBase>
                  );
                })}
              </Stack>
            )}
          </Box>
        </Grid2>
      </Grid2>
    </Stack>
  );
  async function runArkPulseCheck() {
    setError(null);
    const baselineEventId = latestPulseEventId;
    setPulsePollState({
      baselineEventId,
      deadlineAt: Date.now() + 2 * 60 * 1000,
    });
    try {
      const out = asRecord(await triggerPulseMutation.mutateAsync());
      const status = str(out.status, "").toLowerCase();
      if (status === "running") {
        setSuccess(str(out.message, "Pulse is already running."));
      } else {
        setSuccess(str(out.message, "Pulse check started."));
      }
    } catch (e) {
      setPulsePollState(null);
      setError(errMessage(e));
    }
  }

  function renderSettingsSectionIntro(props: SettingsSectionIntroArgs) {
    return (
      <SettingsSectionIntro
        {...props}
        selectedHeaderTitle={selectedSettingsHeaderTitle}
      />
    );
  }

  function renderSettingsInlineCard(props: SettingsInlineCardProps) {
    return <SettingsInlineCard {...props} />;
  }

  const foreverLifecycleRules = [
    {
      label: "Notifications",
      value: form.data_lifecycle_notifications_retention_days,
    },
    {
      label: "Execution traces",
      value: form.data_lifecycle_execution_trace_retention_days,
    },
    {
      label: "Execution proofs",
      value: form.data_lifecycle_execution_proof_retention_days,
    },
    {
      label: "Operational logs",
      value: form.data_lifecycle_operational_log_retention_days,
    },
    {
      label: "Security logs",
      value: form.data_lifecycle_security_log_retention_days,
    },
    {
      label: "Approval logs",
      value: form.data_lifecycle_approval_log_retention_days,
    },
    {
      label: "Delegations",
      value: form.data_lifecycle_swarm_delegation_retention_days,
    },
    { label: "LLM usage", value: form.data_lifecycle_llm_usage_retention_days },
    {
      label: "Completed tasks",
      value: form.data_lifecycle_terminal_task_retention_days,
    },
    {
      label: "Execution runs",
      value: form.data_lifecycle_execution_run_retention_days,
    },
    {
      label: "Background sessions",
      value: form.data_lifecycle_background_session_retention_days,
    },
    {
      label: "Browser sessions",
      value: form.data_lifecycle_browser_session_retention_days,
    },
    {
      label: "Automation runs",
      value: form.data_lifecycle_automation_run_retention_days,
    },
    {
      label: "Conversations",
      value: form.data_lifecycle_message_retention_days,
    },
    {
      label: "Experience runs",
      value: form.data_lifecycle_experience_run_retention_days,
    },
    {
      label: "Experience edges",
      value: form.data_lifecycle_experience_edge_retention_days,
    },
    {
      label: "Staged candidates",
      value: form.data_lifecycle_learning_candidate_retention_days,
    },
    {
      label: "Inactive memory rows",
      value: form.data_lifecycle_experience_item_retention_days,
    },
    {
      label: "Inactive patterns",
      value: form.data_lifecycle_procedural_pattern_retention_days,
    },
    {
      label: "Memory ledger",
      value: form.data_lifecycle_recall_event_retention_days,
    },
    {
      label: "Memory checks",
      value: form.data_lifecycle_recall_test_retention_days,
    },
  ].filter((rule) => {
    const parsed = Number(rule.value.trim());
    return Number.isFinite(parsed) && parsed === 0;
  });
  const foreverLifecycleSummary = foreverLifecycleRules
    .map((rule) => rule.label)
    .join(", ");
  const dataCleanupEnabled = form.data_lifecycle_cleanup_enabled;
  const notificationsCleanupInputsEnabled =
    dataCleanupEnabled && form.data_lifecycle_notifications_cleanup_enabled;
  const logsCleanupInputsEnabled =
    dataCleanupEnabled && form.data_lifecycle_logs_cleanup_enabled;

  const openSearchProviderDialog = (
    provider: (typeof SEARCH_API_PROVIDER_OPTIONS)[number],
  ) => {
    setSearchProviderDialog({
      providerId: provider.id,
      value: str(form[provider.keyField], ""),
      showValue: false,
    });
  };

  const closeSearchProviderDialog = () => setSearchProviderDialog(null);

  const submitSearchProviderDialog = () => {
    if (!searchProviderDialog) return;
    const provider = SEARCH_API_PROVIDER_OPTIONS.find(
      (entry) => entry.id === searchProviderDialog.providerId,
    );
    if (!provider) {
      setSearchProviderDialog(null);
      return;
    }
    const trimmed = searchProviderDialog.value.trim();
    if (!trimmed) return;
    setSearchProviderDraft(provider, {
      key: trimmed,
      editing: true,
      clear: false,
    });
    setSearchProviderDialog(null);
  };

  const renderSearchProviderCredentialField = (
    provider: (typeof SEARCH_API_PROVIDER_OPTIONS)[number],
  ) => {
    const configured = toBool(settings[provider.configuredField]);
    const editing = Boolean(form[provider.editingField]);
    const clearPending = Boolean(form[provider.clearField]);
    const pendingValue = str(form[provider.keyField], "");
    const hasPendingNewKey = editing && pendingValue.trim().length > 0;
    const enabled = (configured && !clearPending) || hasPendingNewKey;
    const transientLabel: string | null = clearPending
      ? "Delete pending"
      : hasPendingNewKey
        ? configured
          ? "Replacement pending"
          : "Pending save"
        : null;

    const handleToggle = (checked: boolean) => {
      if (checked) {
        if (clearPending) {
          setSearchProviderDraft(provider, {
            key: "",
            editing: false,
            clear: false,
          });
          return;
        }
        if (configured) return;
        openSearchProviderDialog(provider);
        return;
      }
      if (configured) {
        setSearchProviderDraft(provider, {
          key: "",
          editing: false,
          clear: true,
        });
        return;
      }
      setSearchProviderDraft(provider, {
        key: "",
        editing: false,
        clear: false,
      });
    };

    return (
      <Box
        key={provider.id}
        sx={{
          display: "flex",
          alignItems: "center",
          gap: 1.25,
          flexWrap: "wrap",
          padding: "10px 12px",
          borderRadius: "8px",
          border: enabled
            ? "1px solid var(--green, #78f2b0)"
            : "1px solid var(--surface-border)",
          background: enabled
            ? "rgba(120, 242, 176, 0.08)"
            : "var(--ui-rgba-255-255-255-020)",
          boxShadow: enabled
            ? "0 0 0 1px rgba(120, 242, 176, 0.10) inset"
            : "none",
          opacity: enabled ? 1 : 0.62,
          transition:
            "opacity 120ms ease, background-color 120ms ease, border-color 120ms ease",
        }}
      >
        <Stack spacing={0.25} sx={{ flex: 1, minWidth: 200 }}>
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ alignItems: "center", flexWrap: "wrap" }}
          >
            <Typography
              variant="body2"
              sx={{
                fontWeight: 600,
                color: enabled ? "text.primary" : "text.secondary",
              }}
            >
              {provider.label}
            </Typography>
            {transientLabel ? (
              <Chip
                size="small"
                variant="outlined"
                label={transientLabel}
                color="warning"
              />
            ) : null}
          </Stack>
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            {clearPending
              ? `${provider.label} will be disabled and the saved key removed on Save.`
              : configured
                ? hasPendingNewKey
                  ? `Replacement key entered. Save settings to apply.`
                  : `Saved key on file.`
                : hasPendingNewKey
                  ? `Key entered. Save settings to enable ${provider.label}.`
                  : `Toggle on to add a ${provider.label} API key.`}
          </Typography>
        </Stack>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{ alignItems: "center", flexWrap: "wrap" }}
        >
          {(configured || hasPendingNewKey) && !clearPending ? (
            <Button
              size="small"
              variant="outlined"
              onClick={() => openSearchProviderDialog(provider)}
            >
              {hasPendingNewKey ? "Change" : "Edit"}
            </Button>
          ) : null}
          {clearPending ? (
            <Button
              size="small"
              variant="text"
              onClick={() =>
                setSearchProviderDraft(provider, {
                  key: "",
                  editing: false,
                  clear: false,
                })
              }
            >
              Undo delete
            </Button>
          ) : null}
          {configured && !clearPending ? (
            <Button
              size="small"
              variant="text"
              color="error"
              onClick={() => {
                if (
                  typeof window !== "undefined" &&
                  !window.confirm(
                    `Delete the saved ${provider.label} API key? This disables ${provider.label} search until you save a new key.`,
                  )
                ) {
                  return;
                }
                setSearchProviderDraft(provider, {
                  key: "",
                  editing: false,
                  clear: true,
                });
              }}
            >
              Delete
            </Button>
          ) : null}
          <Switch
            checked={enabled}
            onChange={(e) => handleToggle(e.target.checked)}
            slotProps={{
              input: {
                "aria-label": `Enable ${provider.label} search provider`,
              },
            }}
          />
        </Stack>
      </Box>
    );
  };

  const searchDialogProvider = searchProviderDialog
    ? SEARCH_API_PROVIDER_OPTIONS.find(
        (entry) => entry.id === searchProviderDialog.providerId,
      ) ?? null
    : null;
  const searchDialogConfigured = searchDialogProvider
    ? toBool(settings[searchDialogProvider.configuredField])
    : false;

  const renderSearxngCredentialRow = () => {
    const savedUrl = str(settings.search_searxng_base_url, "").trim();
    const pendingUrl = str(form.search_searxng_base_url, "").trim();
    const configured = savedUrl.length > 0;
    const enabled = pendingUrl.length > 0;
    const clearPending = configured && pendingUrl.length === 0;
    const hasPendingNewUrl =
      pendingUrl.length > 0 && pendingUrl !== savedUrl;
    const transientLabel: string | null = clearPending
      ? "Delete pending"
      : hasPendingNewUrl
        ? configured
          ? "Replacement pending"
          : "Pending save"
        : null;

    const openDialog = () =>
      setSearxngDialog({ value: pendingUrl || savedUrl });

    const handleToggle = (checked: boolean) => {
      if (checked) {
        if (clearPending) {
          setField("search_searxng_base_url", savedUrl);
          return;
        }
        if (configured) return;
        openDialog();
        return;
      }
      setField("search_searxng_base_url", "");
    };

    return (
      <Box
        sx={{
          display: "flex",
          alignItems: "center",
          gap: 1.25,
          flexWrap: "wrap",
          padding: "10px 12px",
          borderRadius: "8px",
          border: enabled
            ? "1px solid var(--green, #78f2b0)"
            : "1px solid var(--surface-border)",
          background: enabled
            ? "rgba(120, 242, 176, 0.08)"
            : "var(--ui-rgba-255-255-255-020)",
          boxShadow: enabled
            ? "0 0 0 1px rgba(120, 242, 176, 0.10) inset"
            : "none",
          opacity: enabled ? 1 : 0.62,
          transition:
            "opacity 120ms ease, background-color 120ms ease, border-color 120ms ease",
        }}
      >
        <Stack spacing={0.25} sx={{ flex: 1, minWidth: 200 }}>
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ alignItems: "center", flexWrap: "wrap" }}
          >
            <Typography
              variant="body2"
              sx={{
                fontWeight: 600,
                color: enabled ? "text.primary" : "text.secondary",
              }}
            >
              SearXNG (self-hosted)
            </Typography>
            {transientLabel ? (
              <Chip
                size="small"
                variant="outlined"
                label={transientLabel}
                color="warning"
              />
            ) : null}
          </Stack>
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            {clearPending
              ? "SearXNG will be disabled and the saved URL removed on Save."
              : configured
                ? hasPendingNewUrl
                  ? `Replacement URL entered (${pendingUrl}). Save settings to apply.`
                  : `Saved: ${savedUrl}`
                : hasPendingNewUrl
                  ? `URL entered. Save settings to enable SearXNG.`
                  : "Toggle on to point AgentArk at your own SearXNG instance. AgentArk will call /search?format=json against this URL."}
          </Typography>
        </Stack>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{ alignItems: "center", flexWrap: "wrap" }}
        >
          {(configured || hasPendingNewUrl) && !clearPending ? (
            <Button size="small" variant="outlined" onClick={openDialog}>
              {hasPendingNewUrl ? "Change" : "Edit"}
            </Button>
          ) : null}
          {clearPending ? (
            <Button
              size="small"
              variant="text"
              onClick={() => setField("search_searxng_base_url", savedUrl)}
            >
              Undo delete
            </Button>
          ) : null}
          {configured && !clearPending ? (
            <Button
              size="small"
              variant="text"
              color="error"
              onClick={() => {
                if (
                  typeof window !== "undefined" &&
                  !window.confirm(
                    "Delete the saved SearXNG URL? This disables SearXNG search until you save a new URL.",
                  )
                ) {
                  return;
                }
                setField("search_searxng_base_url", "");
              }}
            >
              Delete
            </Button>
          ) : null}
          <Switch
            checked={enabled}
            onChange={(e) => handleToggle(e.target.checked)}
            slotProps={{
              input: {
                "aria-label": "Enable SearXNG search provider",
              },
            }}
          />
        </Stack>
      </Box>
    );
  };

  const searxngDialogConfigured =
    str(settings.search_searxng_base_url, "").trim().length > 0;

  return (
    <Stack spacing={2}>
      {standalonePulse ? (
        <WorkspacePageShell spacing={1.5}>
          {success ? <Alert severity="success">{success}</Alert> : null}
          {error ? <Alert severity="error">{error}</Alert> : null}
          {arkPulsePageContent}
        </WorkspacePageShell>
      ) : (
        <>
      {showSetupRequired ? (
        <Alert severity="warning">
          Setup required: configure at least one model in the Models tab, then
          Save Settings.
        </Alert>
      ) : null}
      <Box
        className="settings-shell-layout"
        sx={{
          flex: 1,
          minHeight: 0,
          ...(hideSettingsNav
            ? { gridTemplateColumns: "1fr !important" }
            : undefined),
        }}
      >
        {!hideSettingsNav ? (
          <SettingsNavigation tab={tab} onTabChange={changeSettingsTab} />
        ) : null}
        <Box
          className={`settings-main${hideSettingsNav ? " settings-main-standalone" : ""}`}
        >
          <Stack
            spacing={2}
            className="workspace-page-shell settings-page-shell"
          >
            {!pulseTabActive ? (
              <WorkspacePageHeader
                eyebrow={selectedSettingsMeta.kicker}
                title={selectedSettingsHeaderTitle}
                description={selectedSettingsMeta.description}
                className="settings-page-header"
                actions={
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      alignItems: "center",
                      justifyContent: { xs: "flex-start", md: "flex-end" },
                      flexWrap: "wrap",
                    }}
                  >
                    {activeSettingsDataRefreshing ? (
                      <Chip size="small" variant="outlined" label="Updating..." />
                    ) : null}
                    {modelsQ.isFetching && showingModelFallback ? (
                      <Chip
                        size="small"
                        color="warning"
                        variant="outlined"
                        label="Reconnecting..."
                      />
                    ) : null}
                    {tabSupportsSave ? (
                      <Button
                        size="small"
                        variant="contained"
                        onClick={() => {
                          void handleSaveSettings();
                        }}
                        disabled={saveMutation.isPending || !effectiveDirty}
                      >
                        Save
                      </Button>
                    ) : null}
                  </Stack>
                }
              />
            ) : null}

            {tab === 0 ? (
              <Stack spacing={2.5}>
                {/* -- Status Overview -- */}
                <Box>
                  {renderSettingsSectionIntro({
                    eyebrow: "General",
                    title: "Status",
                    description:
                      "Quick readiness snapshot for models, delivery channels, and overall setup completeness.",
                  })}
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: {
                        xs: "1fr 1fr",
                        md: "repeat(4, 1fr)",
                      },
                      gap: 1.5,
                    }}
                  >
                    {[
                      {
                        label: "Primary API Key",
                        tone: hasPrimaryApiKey ? "success" : "muted",
                        status: hasPrimaryApiKey
                          ? "Connected"
                          : "Not configured",
                      },
                      {
                        label: "Fallback API Key",
                        tone: hasFallbackApiKey ? "success" : "muted",
                        status: hasFallbackApiKey
                          ? "Connected"
                          : "Not configured",
                      },
                      {
                        label: "Telegram",
                        tone: !hasTelegramToken
                          ? "muted"
                          : telegramDeliveryReady
                            ? "success"
                            : "warning",
                        status: !hasTelegramToken
                          ? "Not configured"
                          : telegramDeliveryReady
                            ? "Ready to deliver"
                            : "Needs bound recipient",
                      },
                      {
                        label: "Slack",
                        tone:
                          !hasSlackBotToken || !hasSlackSigningSecret
                            ? "muted"
                            : slackDeliveryReady
                              ? "success"
                              : "warning",
                        status:
                          !hasSlackBotToken || !hasSlackSigningSecret
                            ? "Not configured"
                            : slackDeliveryReady
                              ? "Ready to deliver"
                              : "Needs target",
                      },
                      {
                        label: "Discord",
                        tone: !hasDiscordBotToken
                          ? "muted"
                          : discordDeliveryReady
                            ? "success"
                            : "warning",
                        status: !hasDiscordBotToken
                          ? "Not configured"
                          : discordDeliveryReady
                            ? "Ready to deliver"
                            : "Needs scope",
                      },
                      {
                        label: "Matrix",
                        tone: !hasMatrixAccessToken
                          ? "muted"
                          : matrixDeliveryReady
                            ? "success"
                            : "warning",
                        status: !hasMatrixAccessToken
                          ? "Not configured"
                          : matrixDeliveryReady
                            ? "Ready to deliver"
                            : "Needs room",
                      },
                      {
                        label: "Teams",
                        tone: !hasTeamsAccessToken
                          ? "muted"
                          : teamsDeliveryReady
                            ? "success"
                            : "warning",
                        status: !hasTeamsAccessToken
                          ? "Not configured"
                          : teamsDeliveryReady
                            ? "Ready to deliver"
                            : "Needs target",
                      },
                      {
                        label: "WhatsApp",
                        tone: !whatsappConfigReady
                          ? "muted"
                          : whatsappDeliveryReady
                            ? "success"
                            : "warning",
                        status: !whatsappConfigReady
                          ? "Not configured"
                          : whatsappDeliveryReady
                            ? "Ready to deliver"
                            : "Needs bound recipient",
                      },
                    ].map((s) => (
                      <Box
                        key={s.label}
                        sx={{
                          p: 1.5,
                          borderRadius: 2,
                          border: "1px solid",
                          borderColor:
                            s.tone === "success"
                              ? "var(--ui-rgba-57-208-255-220)"
                              : s.tone === "warning"
                                ? "var(--ui-rgba-255-180-50-240)"
                                : "var(--ui-rgba-255-255-255-080)",
                          background:
                            s.tone === "success"
                              ? "var(--ui-rgba-57-208-255-060)"
                              : s.tone === "warning"
                                ? "var(--ui-rgba-255-180-50-080)"
                                : "var(--ui-rgba-255-255-255-030)",
                          display: "flex",
                          alignItems: "center",
                          gap: 1,
                        }}
                      >
                        <Box
                          sx={{
                            width: 8,
                            height: 8,
                            borderRadius: "50%",
                            flexShrink: 0,
                            background:
                              s.tone === "success"
                                ? "var(--ui-rgba-57-208-255-850)"
                                : s.tone === "warning"
                                  ? "var(--ui-rgba-255-180-50-900)"
                                  : "var(--ui-rgba-255-255-255-180)",
                            boxShadow:
                              s.tone === "success"
                                ? "0 0 6px var(--ui-rgba-57-208-255-300)"
                                : s.tone === "warning"
                                  ? "0 0 6px var(--ui-rgba-255-180-50-350)"
                                  : "none",
                          }}
                        />
                        <Stack spacing={0}>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "var(--ui-rgba-171-176-184-620)",
                              fontSize: "0.68rem",
                              lineHeight: 1.2,
                            }}
                          >
                            {s.label}
                          </Typography>
                          <Typography
                            variant="body2"
                            sx={{
                              fontWeight: 500,
                              fontSize: "0.8rem",
                              color:
                                s.tone === "muted"
                                  ? "var(--ui-rgba-155-159-169-720)"
                                  : "var(--ui-rgba-244-245-247-920)",
                            }}
                          >
                            {s.status}
                          </Typography>
                        </Stack>
                      </Box>
                    ))}
                  </Box>
                  <Box
                    sx={{ display: "flex", gap: 2, mt: 1.5, flexWrap: "wrap" }}
                  >
                    <Chip
                      size="small"
                      variant="outlined"
                      label={
                        modelsQ.isLoading && modelSlots.length === 0
                          ? "Loading models..."
                          : `${modelSlots.length} model${modelSlots.length !== 1 ? "s" : ""}`
                      }
                      sx={{
                        borderColor: "var(--ui-rgba-255-255-255-080)",
                        color: "var(--ui-rgba-244-245-247-820)",
                        fontSize: "0.72rem",
                      }}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label={
                        configuredProviders.length
                          ? configuredProviders.join(", ")
                          : "No media providers"
                      }
                      sx={{
                        borderColor: "var(--ui-rgba-255-255-255-080)",
                        color: "var(--ui-rgba-171-176-184-720)",
                        fontSize: "0.72rem",
                      }}
                    />
                    {settingsComplete ? (
                      <Chip
                        size="small"
                        variant="outlined"
                        label="Setup complete"
                        sx={{
                          borderColor: "var(--ui-rgba-57-208-255-220)",
                          color: "var(--ui-rgba-57-208-255-850)",
                          fontSize: "0.72rem",
                        }}
                      />
                    ) : (
                      <Chip
                        size="small"
                        variant="outlined"
                        label="Setup incomplete"
                        sx={{
                          borderColor: "var(--ui-rgba-255-180-50-300)",
                          color: "var(--ui-rgba-255-180-50-850)",
                          fontSize: "0.72rem",
                        }}
                      />
                    )}
                  </Box>
                </Box>

                <hr className="settings-divider" />

                {/* -- Identity -- */}
                <Stack spacing={2}>
                  {renderSettingsSectionIntro({
                    eyebrow: "General",
                    title: "Identity",
                    description:
                      "Core operator-facing defaults for how this AgentArk instance presents itself in conversations and automation.",
                  })}
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                      gap: 1.5,
                    }}
                  >
                    <TextField
                      label="Bot Name"
                      value={form.bot_name}
                      onChange={(e) => setField("bot_name", e.target.value)}
                      fullWidth
                      size="small"
                    />
                    <TextField
                      label="Personality"
                      select
                      value={form.personality}
                      onChange={(e) => setField("personality", e.target.value)}
                      fullWidth
                      size="small"
                    >
                      <MenuItem value="friendly">Friendly</MenuItem>
                      <MenuItem value="professional">Professional</MenuItem>
                      <MenuItem value="casual">Casual</MenuItem>
                      <MenuItem value="technical">Technical</MenuItem>
                      <MenuItem value="creative">Creative</MenuItem>
                      <MenuItem value="concise">Concise</MenuItem>
                    </TextField>
                    <TextField
                      label="Language"
                      value={form.language}
                      onChange={(e) => setField("language", e.target.value)}
                      fullWidth
                      size="small"
                      placeholder="e.g. English"
                    />
                    <TextField
                      label="Tone"
                      select
                      value={form.tone}
                      onChange={(e) => setField("tone", e.target.value)}
                      fullWidth
                      size="small"
                      slotProps={{
                        select: { displayEmpty: true },
                        inputLabel: { shrink: true },
                      }}
                    >
                      <MenuItem value="">Default</MenuItem>
                      <MenuItem value="concise">Concise</MenuItem>
                      <MenuItem value="friendly">Friendly</MenuItem>
                      <MenuItem value="professional">Professional</MenuItem>
                      <MenuItem value="casual">Casual</MenuItem>
                      <MenuItem value="technical">Technical</MenuItem>
                      <MenuItem value="creative">Creative</MenuItem>
                    </TextField>
                  </Box>
                </Stack>

                <hr className="settings-divider" />

                {/* -- Preferences -- */}
                <Stack spacing={2}>
                  {renderSettingsSectionIntro({
                    eyebrow: "General",
                    title: "Preferences",
                    description:
                      "Timezone, formatting, and operator defaults used across briefs, reminders, and generated communication.",
                  })}
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                      gap: 1.5,
                    }}
                  >
                    <Stack spacing={0.75}>
                      <Autocomplete
                        freeSolo
                        options={timezoneOptions}
                        value={form.timezone || ""}
                        onChange={(_, v) => setField("timezone", String(v ?? ""))}
                        inputValue={form.timezone || ""}
                        onInputChange={(_, v) => setField("timezone", v)}
                        renderInput={(params) => (
                          <TextField
                            {...params}
                            label="Timezone"
                            placeholder={detectedTimezone || "e.g. America/New_York"}
                            helperText={timezoneHelperText}
                            fullWidth
                            size="small"
                          />
                        )}
                      />
                      {detectedTimezone ? (
                        <Box>
                          <Button
                            size="small"
                            variant="text"
                            disabled={form.timezone.trim() === detectedTimezone}
                            onClick={() => setField("timezone", detectedTimezone)}
                          >
                            Use detected timezone
                          </Button>
                        </Box>
                      ) : null}
                    </Stack>
                    <TextField
                      label="Email Format"
                      select
                      value={form.email_format}
                      onChange={(e) => setField("email_format", e.target.value)}
                      fullWidth
                      size="small"
                      slotProps={{
                        select: { displayEmpty: true },
                        inputLabel: { shrink: true },
                      }}
                    >
                      <MenuItem value="">Default</MenuItem>
                      <MenuItem value="bullets">Bullets</MenuItem>
                      <MenuItem value="sections">Sections</MenuItem>
                      <MenuItem value="narrative">Narrative</MenuItem>
                    </TextField>
                  </Box>
                </Stack>

                <hr className="settings-divider" />

                <Box>
                  {renderSettingsSectionIntro({
                    eyebrow: "General",
                    title: "Daily Brief",
                    description:
                      "Schedule the recurring summary and choose how AgentArk delivers it each day.",
                  })}
                  <Stack spacing={1.25}>
                    {renderSettingsInlineCard({
                      eyebrow: "Daily brief",
                      title: "Delivery status",
                      description:
                        "Send a recurring summary using your chosen time and timezone.",
                      action: (
                        <FormControlLabel
                          control={
                            <Switch
                              checked={form.daily_brief_enabled}
                              onChange={(e) =>
                                setField(
                                  "daily_brief_enabled",
                                  e.target.checked,
                                )
                              }
                            />
                          }
                          label={
                            form.daily_brief_enabled ? "Enabled" : "Disabled"
                          }
                        />
                      ),
                    })}
                    {renderSettingsInlineCard({
                      eyebrow: "Reflect",
                      title: "Daily digest",
                      description:
                        "Send one end-of-day reflection to the same notification channel, only when AgentArk found meaningful activity.",
                      action: (
                        <FormControlLabel
                          control={
                            <Switch
                              checked={form.arkreflect_daily_digest_enabled}
                              onChange={(e) =>
                                setField(
                                  "arkreflect_daily_digest_enabled",
                                  e.target.checked,
                                )
                              }
                            />
                          }
                          label={
                            form.arkreflect_daily_digest_enabled
                              ? "Enabled"
                              : "Disabled"
                          }
                        />
                      ),
                    })}
                    <Box
                      sx={{
                        display: "grid",
                        gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                        gap: 1.5,
                      }}
                    >
                      <TextField
                        label="Preferred Delivery Time"
                        type="time"
                        value={form.daily_brief_time}
                        onChange={(e) =>
                          setField("daily_brief_time", e.target.value)
                        }
                        fullWidth
                        size="small"
                        helperText="24-hour time. The brief follows the timezone above."
                        slotProps={{
                          htmlInput: { step: 60 },
                          inputLabel: { shrink: true },
                        }}
                      />
                      <TextField
                        label="Delivery Channel"
                        select
                        value={form.daily_brief_channel}
                        onChange={(e) =>
                          setField("daily_brief_channel", e.target.value)
                        }
                        fullWidth
                        size="small"
                        slotProps={{
                          inputLabel: { shrink: true },
                        }}
                        helperText={
                          availableDeliveryChannels.length === 0
                            ? "No delivery channels are configured yet. Connect one in Integrations to enable daily brief delivery."
                            : undefined
                        }
                      >
                        {availableDeliveryChannels.map((channel) => (
                          <MenuItem key={channel.id} value={channel.id}>
                            {deliveryChannelMenuLabel(channel)}
                          </MenuItem>
                        ))}
                        {form.daily_brief_channel &&
                        !availableDeliveryChannels.some(
                          (channel) => channel.id === form.daily_brief_channel,
                        ) ? (
                          <MenuItem value={form.daily_brief_channel} disabled>
                            Not connected: {form.daily_brief_channel}
                          </MenuItem>
                        ) : null}
                      </TextField>
                    </Box>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Turning this off pauses the scheduled brief but keeps your
                      preferred time saved.
                    </Typography>
                    {dailyBriefDeliveryWarning ? (
                      <Alert severity="warning">
                        {dailyBriefDeliveryWarning}
                      </Alert>
                    ) : null}
                    {dailyBriefUsesUserDefinedExternalChannel ? (
                      <Alert severity="info">
                        AgentArk sends the full daily brief to this user-defined
                        external endpoint. Detected secrets are redacted before
                        delivery.
                      </Alert>
                    ) : null}
                  </Stack>
                </Box>
              </Stack>
            ) : null}

            {tab === 1 ? (
              <SettingsModelsPanel
                modelsSectionTab={modelsSectionTab}
                setModelsSectionTab={setModelsSectionTab}
                renderSettingsSectionIntro={renderSettingsSectionIntro}
                  openAddModel={openAddModel}
                  form={form}
                  setField={setField}
                  modelsQ={modelsQ}
                  modelSlots={modelSlots}
                  modelsRefreshIssue={modelsRefreshIssue}
                  showingModelFallback={showingModelFallback}
                  toggleModelEnabledMutation={toggleModelEnabledMutation}
                  setError={setError}
                  deleteModelMutation={deleteModelMutation}
                  openEditModel={openEditModel}
                  embeddingsProvider={embeddingsProvider}
                  embeddingsDisabled={embeddingsDisabled}
                  embeddingsHasApiKey={embeddingsHasApiKey}
                  embeddingsStatus={embeddingsStatus}
                  embeddingsIsLocal={embeddingsIsLocal}
                  embeddingsIsOllama={embeddingsIsOllama}
                  embeddingsIsExternal={embeddingsIsExternal}
                  hiddenExternalEmbeddingsProvider={
                    hiddenExternalEmbeddingsProvider
                  }
                  modelDialogOpen={modelDialogOpen}
                  setModelDialogOpen={setModelDialogOpen}
                  modelForm={modelForm}
                  setModelForm={setModelForm}
                  modelEditingId={modelEditingId}
                  modelCanReuseExistingKey={modelCanReuseExistingKey}
                  showClearSavedKeyAction={showClearSavedKeyAction}
                  modelClearSavedKeyPending={modelClearSavedKeyPending}
                  setModelClearApiKey={setModelClearApiKey}
                  modelNeedsReplacementKeyWarning={modelNeedsReplacementKeyWarning}
                  modelAdvancedOpen={modelAdvancedOpen}
                  setModelAdvancedOpen={setModelAdvancedOpen}
                  modelConnectionTestResult={modelConnectionTestResult}
                  setModelConnectionTestResult={setModelConnectionTestResult}
                  modelTestConnectionHint={modelTestConnectionHint}
                  testModelConnectionMutation={testModelConnectionMutation}
                  canTestModelConnection={canTestModelConnection}
                  saveModelMutation={saveModelMutation}
                  setModelConnectivityWarning={setModelConnectivityWarning}
                  openaiSubAuth={openaiSubAuth}
                  codexAuthBusy={codexAuthBusy}
                  startOpenaiSubscriptionOAuth={startOpenaiSubscriptionOAuth}
                  checkOpenaiSubscriptionOAuthStatus={
                    checkOpenaiSubscriptionOAuthStatus
                  }
                discoverModelsQ={discoverModelsQ}
                modelOptions={modelOptions}
                modelOptionNames={modelOptionNames}
                setSuccess={setSuccess}
              />
            ) : null}
            {tab === 3 ? (
              <MediaSettingsPanel
                form={form}
                setField={setField}
                configuredProviders={configuredProviders}
                renderSettingsSectionIntro={renderSettingsSectionIntro}
              />
            ) : null}

            {tab === 24 ? (
              <Grid2
                container
                spacing={1.5}
                sx={{
                  alignItems: "stretch",
                }}
              >
                <Grid2 size={{ xs: 12 }} sx={{ display: "flex" }}>
                  <Box
                    className="list-shell"
                    sx={{ minHeight: 0, width: "100%" }}
                  >
                    {renderSettingsSectionIntro({
                      eyebrow: "Search",
                      title: "Provider Credentials",
                      description:
                        "Paid and self-hosted providers are ignored until configured. Configured providers are then used automatically in a fixed order.",
                    })}
                    <Stack spacing={1.2} sx={{ mt: 1 }}>
                      <Alert severity="info">
                        Configured provider order:{" "}
                        {SEARCH_PROVIDER_OPTIONS.map(
                          (provider) => provider.label,
                        ).join(" -> ")}{" "}
                        {"- free fallback: DuckDuckGo to Lightpanda to Bing RSS."}
                      </Alert>
                      {!toBool(settings.search_lightpanda_available) ? (
                        <Alert severity="warning">
                          Lightpanda is missing from this runtime, so the
                          bundled free search fallback skips it until the
                          AgentArk runtime is updated or rebuilt.
                        </Alert>
                      ) : null}
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Anonymous HTML backends that return challenge pages are
                        cooled down for{" "}
                        {str(settings.search_builtin_cooldown_hours, "24")}{" "}
                        hours. Configured API providers and SearXNG are never
                        auto-cooled down.
                      </Typography>
                      {SEARCH_API_PROVIDER_OPTIONS.map((provider) =>
                        renderSearchProviderCredentialField(provider),
                      )}
                      {renderSearxngCredentialRow()}
                    </Stack>
                  </Box>
                </Grid2>
              </Grid2>
            ) : null}

            {tab === 4 ? (
              <SettingsSecurityPanel
                  renderSettingsSectionIntro={renderSettingsSectionIntro}
                  securityStatusQ={securityStatusQ}
                  hasCustomMasterPassword={hasCustomMasterPassword}
                  passwordMutationPending={passwordMutationPending}
                  openPasswordDialog={openPasswordDialog}
                  abuseReviews={abuseReviews}
                  abuseReviewsQ={abuseReviewsQ}
                  decideAbuseReviewMutation={decideAbuseReviewMutation}
                  showInternalServiceSection={showInternalServiceSection}
                  internalServiceDescription={internalServiceDescription}
                  internalServiceRotationSupported={
                    internalServiceRotationSupported
                  }
                  internalServiceTokens={internalServiceTokens}
                  openRotateInternalCredentialsDialog={
                    openRotateInternalCredentialsDialog
                  }
                  tunnelQ={tunnelQ}
                  tunnelProvidersQ={tunnelProvidersQ}
                  tunnel={tunnel}
                  tunnelProvidersPayload={tunnelProvidersPayload}
                  selectedTunnelMeta={selectedTunnelMeta}
                  selectedTunnelStoredSecretFields={
                    selectedTunnelStoredSecretFields
                  }
                  showTunnelAdvanced={showTunnelAdvanced}
                  setShowTunnelAdvanced={setShowTunnelAdvanced}
                  tunnelDraftValues={tunnelDraftValues}
                  setTunnelDraftValues={setTunnelDraftValues}
                  tunnelSelectedProviderId={tunnelSelectedProviderId}
                  tunnelPanelNotice={tunnelPanelNotice}
                  serverSelectedTunnelProviderId={serverSelectedTunnelProviderId}
                  tunnelSummaryTone={tunnelSummaryTone}
                  tunnelStateLabel={tunnelStateLabel}
                  tunnelAccessLabel={tunnelAccessLabel}
                  tunnelPrimaryText={tunnelPrimaryText}
                  tunnelPrimaryDetail={tunnelPrimaryDetail}
                  tunnelProviderOptions={tunnelProviderOptions}
                  basicTunnelConfigFields={basicTunnelConfigFields}
                  advancedTunnelConfigFields={advancedTunnelConfigFields}
                  selectedTunnelAvailable={selectedTunnelAvailable}
                  tunnelSetupChecks={tunnelSetupChecks}
                  renderSettingsInlineCard={renderSettingsInlineCard}
                  tunnelSaveMutation={tunnelSaveMutation}
                  tunnelTestMutation={tunnelTestMutation}
                  tunnelStartMutation={tunnelStartMutation}
                  tunnelStopMutation={tunnelStopMutation}
                  handleTunnelStart={handleTunnelStart}
                  handleTunnelStop={handleTunnelStop}
                  syncTunnelDraftFromPayload={syncTunnelDraftFromPayload}
                  handleTunnelProviderSave={handleTunnelProviderSave}
                  handleTunnelProviderTest={handleTunnelProviderTest}
                  rotateInternalServiceTokensMutation={
                    rotateInternalServiceTokensMutation
                  }
                  restartNotice={restartNotice}
                  vaultSummaryText={vaultSummaryText}
                  vaultSecrets={vaultSecrets}
                  vaultPassword={vaultPassword}
                  setVaultPassword={setVaultPassword}
                  vaultSecretsQ={vaultSecretsQ}
                  vaultSecretsRequested={securityVaultRequested}
                  requestVaultSecrets={() => setSecurityVaultRequested(true)}
                  queryClient={queryClient}
                  openVaultEditor={openVaultEditor}
                  deleteVaultSecretMutation={deleteVaultSecretMutation}
                  resolveVaultPasswordForSensitiveOps={
                    resolveVaultPasswordForSensitiveOps
                  }
                  autoRefresh={autoRefresh}
                form={form}
                setField={setField}
                setError={setError}
                setSuccess={setSuccess}
              />
            ) : null}
            {tab === 5 ? (
              <SettingsAdvancedPanel
                  restartNotice={restartNotice}
                  renderSettingsInlineCard={renderSettingsInlineCard}
                  settingsAutonomyQ={settingsAutonomyQ}
                  settingsAutonomyPaused={settingsAutonomyPaused}
                  settingsAutonomyModeLabel={settingsAutonomyModeLabel}
                  handleResumeAutonomy={handleResumeAutonomy}
                  setAutonomyPauseDialogOpen={setAutonomyPauseDialogOpen}
                  settingsAutonomyMutation={settingsAutonomyMutation}
                  openRestartDialog={openRestartDialog}
                  restartMutation={restartMutation}
                  developerModeEnabled={developerModeEnabled}
                  setDeveloperModeEnabledState={setDeveloperModeEnabledState}
                  setError={setError}
                  setSuccess={setSuccess}
                  settingsSentinelQ={settingsSentinelQ}
                  settingsSentinel={settingsSentinel}
                  settingsSentinelEnabled={settingsSentinelEnabled}
                  setSentinelDisableDialogOpen={setSentinelDisableDialogOpen}
                  setSentinelInAppDisableDialogOpen={
                    setSentinelInAppDisableDialogOpen
                  }
                  updateSettingsSentinel={updateSettingsSentinel}
                  settingsSentinelMutation={settingsSentinelMutation}
                  settingsEvolutionQ={settingsEvolutionQ}
                  settingsSelfEvolveEnabled={settingsSelfEvolveEnabled}
                  handleEnableSelfEvolve={handleEnableSelfEvolve}
                  setSelfEvolveDisableDialogOpen={setSelfEvolveDisableDialogOpen}
                  settingsEvolutionMutation={settingsEvolutionMutation}
                  readinessPolicyDraft={readinessPolicyDraft}
                  setReadinessPolicyDraft={setReadinessPolicyDraft}
                  readinessPolicyToDraft={readinessPolicyToDraft}
                  settingsReadinessPolicy={settingsReadinessPolicy}
                  submitReadinessPolicyDraft={submitReadinessPolicyDraft}
                  settingsDefaultGuardEnabled={settingsDefaultGuardEnabled}
                  updateSettingsEvolution={updateSettingsEvolution}
                  findBlockedAutoApproveEntries={findBlockedAutoApproveEntries}
                  parseCsvList={parseCsvList}
                  sanitizeAutoApproveList={sanitizeAutoApproveList}
                  form={form}
                  setField={setField}
                  apiKeyQ={apiKeyQ}
                  apiKeyRemainingSeconds={apiKeyRemainingSeconds}
                  apiKeyRotated={apiKeyRotated}
                  apiKeyRevealed={apiKeyRevealed}
                  setApiKeyRevealed={setApiKeyRevealed}
                  apiKeyPayload={apiKeyPayload}
                  apiKeyIssuedAtUnix={apiKeyIssuedAtUnix}
                  apiKeyExpiresAtUnix={apiKeyExpiresAtUnix}
                regenerateApiKeyMutation={regenerateApiKeyMutation}
              />
            ) : null}
            {tab === 14 ? (
              <SettingsDataLifecyclePanel
                form={form}
                setField={(key, value) => setField(key, value)}
                foreverLifecycleRules={foreverLifecycleRules}
                foreverLifecycleSummary={foreverLifecycleSummary}
                dataCleanupEnabled={dataCleanupEnabled}
                notificationsCleanupInputsEnabled={
                  notificationsCleanupInputsEnabled
                }
                logsCleanupInputsEnabled={logsCleanupInputsEnabled}
                renderSettingsSectionIntro={renderSettingsSectionIntro}
              />
            ) : null}
            {tab === 25 ? (
              <SettingsUpdatesPanel
                  restartNotice={restartNotice}
                  renderSettingsInlineCard={renderSettingsInlineCard}
                  renderSettingsSectionIntro={renderSettingsSectionIntro}
                  updateStatus={updateStatus}
                  updateCheckedAtLabel={updateCheckedAtLabel}
                  updateStatusQ={updateStatusQ}
                  updateAgentArkPending={updateAgentArkMutation.isPending}
                  onUpdateAndRestart={async () => {
                    const ok = window.confirm(
                      "Update AgentArk and restart now? Pending chats, running jobs, and in-flight approvals can be interrupted.",
                    );
                    if (!ok) return;
                    setError(null);
                    setSuccess(null);
                    setRestartNotice(null);
                    try {
                      await updateAgentArkMutation.mutateAsync();
                      void monitorRestartRecovery(UPDATE_NOTICE_DURATION_MS);
                    } catch (e) {
                      setError(errMessage(e));
                    }
                }}
              />
            ) : null}

            {tab === 6 ? (
              <Stack spacing={2.5}>
                <ObservabilityPanel
                    values={{
                      enabled: form.observability_enabled,
                      provider: form.observability_provider,
                      endpoint: form.observability_endpoint,
                      serviceName: form.observability_service_name,
                      headerName: form.observability_header_name,
                      privacyMode: form.observability_privacy_mode,
                      authToken: form.observability_auth_token,
                      authTokenConfigured: toBool(
                        observabilitySettings.auth_token_configured,
                      ),
                    }}
                    logs={observabilityLogs}
                    issues={observabilityIssues}
                    logsLoading={observabilityLogsQ.isLoading}
                    logsError={
                      observabilityLogsQ.error
                        ? errMessage(observabilityLogsQ.error)
                        : null
                    }
                    testing={testObservabilityMutation.isPending}
                    embedded
                    onValueChange={(next) => {
                      if (
                        Object.prototype.hasOwnProperty.call(next, "enabled")
                      ) {
                        setField("observability_enabled", !!next.enabled);
                      }
                      if (typeof next.provider === "string") {
                        setField("observability_provider", next.provider);
                      }
                      if (typeof next.endpoint === "string") {
                        setField("observability_endpoint", next.endpoint);
                      }
                      if (typeof next.serviceName === "string") {
                        setField(
                          "observability_service_name",
                          next.serviceName,
                        );
                      }
                      if (typeof next.headerName === "string") {
                        setField("observability_header_name", next.headerName);
                      }
                      if (typeof next.privacyMode === "string") {
                        setField(
                          "observability_privacy_mode",
                          next.privacyMode,
                        );
                      }
                      if (typeof next.authToken === "string") {
                        setField("observability_auth_token", next.authToken);
                      }
                    }}
                    onTest={async () => {
                      setError(null);
                      setSuccess(null);
                      try {
                        await testObservabilityMutation.mutateAsync();
                      } catch {
                        // handled by mutation onError
                      }
                  }}
                />
              </Stack>
            ) : null}

            {tab === 20 ? (
              <WorkspaceLazyPanel message={settingsLoadingMessage}>
                <IntegrationsPanel
                  autoRefresh={autoRefresh}
                  embedded
                  mode="integrations"
                />
              </WorkspaceLazyPanel>
            ) : null}

            {tab === 21 ? (
              <Box className="list-shell">
                <WorkspaceLazyPanel message={settingsLoadingMessage}>
                  <IntegrationsPanel
                    autoRefresh={autoRefresh}
                    embedded
                    mode="connectors"
                  />
                </WorkspaceLazyPanel>
              </Box>
            ) : null}

            {tab === 22 ? (
              <Stack spacing={2}>
                <Box className="list-shell">
                  <WebhooksPanel autoRefresh={autoRefresh} />
                </Box>
                <Box className="list-shell">
                  <WorkspaceLazyPanel message={settingsLoadingMessage}>
                    <IntegrationQuickstartPanel
                      integrations={[]}
                      autoRefresh={autoRefresh}
                      embedded
                      onConfigureIntegration={() => {}}
                      mode="custom-apis-only"
                    />
                  </WorkspaceLazyPanel>
                </Box>
              </Stack>
            ) : null}

            {tab === 23 ? (
              <Box className="list-shell">
                <PluginSdkPanel autoRefresh={autoRefresh} embedded />
              </Box>
            ) : null}

            {tab === 26 ? (
              <CompanionDevicesPanel autoRefresh={autoRefresh} />
            ) : null}

            {tab === 11 ? (
              <WorkspaceLazyPanel message={settingsLoadingMessage}>
                <TracePage autoRefresh={autoRefresh} />
              </WorkspaceLazyPanel>
            ) : null}

            {tab === 8 ? (
              <Box className="list-shell">
                <WorkspaceLazyPanel message={settingsLoadingMessage}>
                  <IntegrationsPanel
                    autoRefresh={autoRefresh}
                    embedded
                    mode="mcp"
                  />
                </WorkspaceLazyPanel>
              </Box>
            ) : null}

            {tab === 12 ? (
              <WorkspaceLazyPanel message={settingsLoadingMessage}>
                <MemoryPage
                  autoRefresh={autoRefresh}
                  showHeader={false}
                />
              </WorkspaceLazyPanel>
            ) : null}

            {tab === 9 ? (
              <Stack spacing={2}>
                {arkPulseHeader}
                <Grid2
                  container
                  spacing={2}
                  sx={{
                    alignItems: "stretch",
                  }}
                >
                  <Grid2 size={{ xs: 12 }}>
                    <Box
                      className="list-shell"
                      sx={{
                        minHeight: 0,
                        height: "100%",
                        display: "flex",
                        flexDirection: "column",
                      }}
                    >
                      {pulseQ.error ? (
                        <Alert severity="error">
                          {errMessage(pulseQ.error)}
                        </Alert>
                      ) : null}
                      {!pulseQ.error ? (
                        <Alert
                          severity={
                            pulseRunning
                              ? "info"
                              : pulseHistoryUnavailable ||
                                  latestPulseFindingsCount > 0
                                ? "warning"
                                : "success"
                          }
                          sx={{ mb: 1 }}
                        >
                          <Typography variant="subtitle2">
                            {latestPulseHeadline}
                          </Typography>
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {latestPulseSubtitle}
                          </Typography>
                        </Alert>
                      ) : null}
                      {pulseEvents.length === 0 ? (
                        <Stack spacing={1} sx={{ flex: 1 }}>
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {pulseHistoryUnavailable
                              ? "Stored Pulse history could not be loaded in this runtime."
                              : "No Pulse events yet."}
                          </Typography>
                          {renderSettingsInlineCard({
                            eyebrow: "Pulse",
                            title: "How this helps",
                            description:
                              "Pulse runs a health check for setup, integrations, safety, and runtime drift.",
                            tone: "info",
                            children: (
                              <Stack spacing={0.6}>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  Run it after changing models, adding
                                  integrations, or when something stops working.
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  Example: if notifications stop arriving,
                                  Pulse can point you to the broken setup
                                  step.
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  Every run appears here with findings,
                                  suggested fixes, and a health score.
                                </Typography>
                              </Stack>
                            ),
                          })}
                          <Box sx={{ flex: 1 }} />
                        </Stack>
                      ) : (
                        <Stack spacing={0} sx={{ flex: 1, minHeight: 0, borderTop: "1px solid", borderColor: "divider" }}>
                          {pulseEvents.slice(0, 40).map((ev, idx) => {
                            const details = asRecord(ev.details);
                            const findings = pickRecords(
                              details,
                              "doctor_findings",
                            ).filter((f) =>
                              isUserActionableDoctorFinding(f),
                            );
                            const score = num(details.doctor_score, -1);
                            const status = str(ev.status, "-");
                            const ok = status.toLowerCase() === "ok";
                            const findingCount = Array.isArray(findings) ? findings.length : 0;
                            const summary = str(ev.summary, "").trim();
                            const message = str(ev.message, "").trim();
                            const overdue = num(ev.overdue_tasks, 0);
                            const failed = num(ev.failed_tasks, 0);
                            const headline = summary || message || (ok ? "All systems healthy" : "Issues detected");
                            const metaParts: string[] = [];
                            if (score >= 0) metaParts.push(`Score ${score}`);
                            if (findingCount > 0) metaParts.push(`${findingCount} finding${findingCount === 1 ? "" : "s"}`);
                            if (overdue > 0) metaParts.push(`${overdue} overdue`);
                            if (failed > 0) metaParts.push(`${failed} failed`);
                            if (metaParts.length === 0) metaParts.push("No issues");
                            return (
                              <ButtonBase
                                key={str(ev.id, String(idx))}
                                onClick={() => setSelectedPulseEvent(ev)}
                                sx={{ width: "100%", textAlign: "left", justifyContent: "flex-start", px: 0, py: 0.85, borderBottom: "1px solid", borderColor: "divider", transition: "background 0.15s ease", "&:hover": { background: "var(--ui-rgba-57-208-255-040)" }, display: "block" }}
                              >
                                <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", justifyContent: "space-between" }}>
                                  <Stack direction="row" spacing={0.75} useFlexGap sx={{ alignItems: "center", minWidth: 0, flex: 1 }}>
                                    <Box component="span" sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, bgcolor: ok ? "var(--ui-rgba-74-210-157-850)" : "var(--ui-rgba-255-180-60-850)" }} />
                                    <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>{headline}</Typography>
                                  </Stack>
                                  <Typography variant="caption" sx={{ color: "text.secondary", whiteSpace: "nowrap", flexShrink: 0 }} title={humanTs(str(ev.timestamp, "-")).tip}>{humanTs(str(ev.timestamp, "-")).label}</Typography>
                                </Stack>
                                <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px", display: "block" }}>
                                  {metaParts.join(" / ")}
                                </Typography>
                              </ButtonBase>
                            );
                          })}
                        </Stack>
                      )}
                    </Box>
                  </Grid2>
                </Grid2>
              </Stack>
            ) : null}
          </Stack>
        </Box>
      </Box>
        </>
      )}
      <Dialog
        open={selectedPulseEvent != null}
        onClose={() => setSelectedPulseEvent(null)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-bg-elevated)",
              boxShadow: "0 30px 96px var(--ui-rgba-0-0-0-500)",
            },
          },
        }}
      >
        <DialogTitle
          sx={{ pb: 1.2, borderBottom: "1px solid var(--ui-rgba-255-255-255-060)" }}
        >
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1.25}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", sm: "center" },
            }}
          >
            <Box>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Pulse Run
              </Typography>
              <Typography
                variant="body2"
                sx={{ color: "text.secondary", mt: 0.35, maxWidth: 720 }}
              >
                {str(
                  selectedPulseEvent?.summary,
                  "Health check details, findings, and scan ledger.",
                )}
              </Typography>
            </Box>
            <Chip
              size="small"
              label={selectedPulseStatus}
              color={selectedPulseStatusOk ? "success" : "warning"}
              variant="outlined"
            />
          </Stack>
        </DialogTitle>
        <DialogContent sx={{ pt: 2 }}>
          <Stack spacing={1.25}>
            <Box
              sx={{
                borderRadius: "8px",
                border: "1px solid var(--ui-rgba-255-255-255-080)",
                background: "var(--ui-rgba-255-255-255-020)",
                p: { xs: 1.5, sm: 1.75 },
                boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)",
              }}
            >
              <Stack spacing={1.5}>
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={1}
                  useFlexGap
                  sx={{
                    alignItems: { xs: "flex-start", sm: "center" },
                    flexWrap: "wrap",
                  }}
                >
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`Captured: ${selectedPulseCaptured.label}`}
                    title={selectedPulseCaptured.tooltip}
                    sx={{
                      borderColor: "var(--ui-rgba-255-255-255-140)",
                      background: "var(--ui-rgba-255-255-255-030)",
                    }}
                  />
                  <Chip
                    size="small"
                    label={`Status: ${selectedPulseStatus}`}
                    color={selectedPulseStatusOk ? "success" : "warning"}
                    variant="outlined"
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`${selectedPulseFindings.length} priority item${selectedPulseFindings.length === 1 ? "" : "s"}`}
                    sx={{
                      borderColor: "var(--ui-rgba-255-255-255-140)",
                      background: "var(--ui-rgba-255-255-255-030)",
                    }}
                  />
                </Stack>

                <Grid2 container spacing={1.25} sx={{ alignItems: "stretch" }}>
                  <Grid2 size={{ xs: 12, lg: 7 }}>
                    <Stack
                      direction="row"
                      spacing={1.25}
                      sx={{ alignItems: "flex-start" }}
                    >
                      <Box
                        sx={{
                          width: 42,
                          height: 42,
                          borderRadius: "8px",
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          color: "var(--ui-rgba-243-246-250-920)",
                          background: "var(--ui-rgba-255-255-255-050)",
                          border: "1px solid var(--ui-rgba-255-255-255-080)",
                          flex: "0 0 auto",
                        }}
                      >
                        {selectedPulseHeroIcon}
                      </Box>
                      <Stack spacing={0.65} sx={{ minWidth: 0 }}>
                        <Typography
                          variant="h6"
                          sx={{ fontWeight: 700, lineHeight: 1.15 }}
                        >
                          {selectedPulseGuidance.title}
                        </Typography>
                        <Typography
                          variant="body2"
                          sx={{
                            color: "text.secondary",
                            maxWidth: 560,
                            lineHeight: 1.55,
                          }}
                        >
                          {selectedPulseGuidance.detail}
                        </Typography>
                      </Stack>
                    </Stack>
                  </Grid2>
                  <Grid2 size={{ xs: 12, lg: 5 }}>
                    <Box
                      sx={{
                        display: "grid",
                        gridTemplateColumns: {
                          xs: "1fr",
                          sm: "repeat(3, minmax(0, 1fr))",
                        },
                        gap: 1,
                        height: "100%",
                      }}
                    >
                      {selectedPulsePrimaryStats.map((item) => (
                        <Box
                          key={item.label}
                          sx={{
                            minWidth: 0,
                            p: 1.2,
                            borderRadius: "8px",
                            border: "1px solid var(--ui-rgba-255-255-255-080)",
                            background: "var(--ui-rgba-255-255-255-030)",
                          }}
                        >
                          <Typography
                            variant="caption"
                            sx={{
                              display: "block",
                              color: "var(--ui-rgba-188-198-212-700)",
                            }}
                          >
                            {item.label}
                          </Typography>
                          <Typography
                            variant="h5"
                            sx={{
                              mt: 0.35,
                              fontWeight: 700,
                              fontVariantNumeric: "tabular-nums",
                            }}
                          >
                            {item.value}
                          </Typography>
                          <Typography
                            variant="caption"
                            sx={{
                              display: "block",
                              mt: 0.4,
                              color: "text.secondary",
                            }}
                          >
                            {item.helper}
                          </Typography>
                        </Box>
                      ))}
                    </Box>
                  </Grid2>
                </Grid2>
              </Stack>
            </Box>

            <Stack spacing={0.3} sx={{ pt: 0.35 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                Priority actions
              </Typography>
              <Typography
                variant="body2"
                sx={{ color: "var(--ui-rgba-188-198-212-720)" }}
              >
                {selectedPulseFindings.length === 0
                  ? "This run did not return any actionable issues."
                  : "Work from top to bottom. Each item includes the cause, the recommended next step, and the safest action available here."}
              </Typography>
            </Stack>
            {selectedPulseFindings.length === 0 ? (
              <Box
                sx={{
                  borderRadius: "8px",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "var(--ui-rgba-255-255-255-020)",
                  px: 1.4,
                  py: 1.25,
                }}
              >
                <Stack
                  direction="row"
                  spacing={1.1}
                  sx={{ alignItems: "flex-start" }}
                >
                  <Box
                    sx={{
                      width: 34,
                      height: 34,
                      borderRadius: "8px",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      color: "var(--ui-rgba-243-246-250-920)",
                      background: "var(--ui-rgba-255-255-255-050)",
                      border: "1px solid var(--ui-rgba-255-255-255-080)",
                      flex: "0 0 auto",
                    }}
                  >
                    <CheckCircleRoundedIcon sx={{ fontSize: 20 }} />
                  </Box>
                  <Stack spacing={0.35}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      Nothing urgent in this run
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{ color: "text.secondary" }}
                    >
                      The system snapshot below is still useful for context, but
                      there is no direct remediation queued from this report.
                    </Typography>
                  </Stack>
                </Stack>
              </Box>
            ) : (
              <Grid2 container spacing={1.25}>
                {selectedPulseFindings.slice(0, 20).map((finding, idx) => {
                  const fr = finding.row;
                  const sev = str(fr.severity, "");
                  const title = str(fr.title, "Issue");
                  const target = str(fr.target, "-");
                  const cause = str(fr.root_cause, "-");
                  const typedRemediation = parseArkPulseRemediationSpec(
                    fr.remediation,
                  );
                  const runnableRemediation =
                    typedRemediation ?? getRunnableArkPulseRemediation(fr);
                  const rawFixCommand = str(fr.fix_command, "").trim();
                  const displayRemediation =
                    typedRemediation ?? runnableRemediation;
                  const fix = displayRemediation
                    ? describeArkPulseRemediation(displayRemediation)
                    : getArkPulseFixText(fr);
                  const useMonospaceFix =
                    displayRemediation?.kind === "shell_command" ||
                    (!displayRemediation && rawFixCommand.length > 0);
                  const canCopyFix =
                    fix.trim().length > 0 && fix.trim() !== "-";
                  const canRunFix = runnableRemediation != null;
                  const fixActionId = `${title}:${target}:${idx}`;
                  const fixBusy =
                    runPulseFixMutation.isPending &&
                    activePulseFixId === fixActionId;
                  const inlineFixResult = pulseFixResultsById[fixActionId];
                  return (
                    <Grid2 key={`${title}-${idx}`} size={{ xs: 12, xl: 6 }}>
                      <Box
                        sx={{
                          height: "100%",
                          borderRadius: "8px",
                          border: "1px solid var(--ui-rgba-255-255-255-080)",
                          background: "var(--ui-rgba-255-255-255-020)",
                          p: 1.35,
                        }}
                      >
                        <Stack spacing={0.75}>
                          <Stack
                            direction="row"
                            spacing={1}
                            useFlexGap
                            sx={{
                              alignItems: "flex-start",
                              flexWrap: "wrap",
                            }}
                          >
                            <Box
                              sx={{
                                minWidth: 28,
                                height: 28,
                                borderRadius: "8px",
                                display: "flex",
                                alignItems: "center",
                                justifyContent: "center",
                                background: "var(--ui-rgba-255-255-255-060)",
                                color: "var(--ui-rgba-243-246-250-920)",
                                fontSize: "0.8rem",
                                fontWeight: 700,
                              }}
                            >
                              {idx + 1}
                            </Box>
                            <Stack spacing={0.15} sx={{ minWidth: 0, flex: 1 }}>
                              <Typography
                                variant="subtitle2"
                                sx={{ fontWeight: 700 }}
                              >
                                {title}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{ color: "var(--ui-rgba-188-198-212-700)" }}
                              >
                                Target: {target}
                              </Typography>
                            </Stack>
                            <Chip
                              size="small"
                              label={sev || "-"}
                              color={severityChipColor(sev)}
                            />
                          </Stack>
                          <Typography
                            variant="body2"
                            sx={{
                              color: "var(--ui-rgba-231-236-243-720)",
                              lineHeight: 1.55,
                            }}
                          >
                            {cause === "-"
                              ? "The run flagged this issue but did not include a detailed cause."
                              : cause}
                          </Typography>
                          <Box
                            sx={{
                              border: "1px solid var(--ui-rgba-255-255-255-060)",
                              borderRadius: "8px",
                              p: 1.05,
                              background: "var(--ui-rgba-255-255-255-030)",
                            }}
                          >
                            <Typography
                              variant="caption"
                              sx={{
                                color: "var(--ui-rgba-188-198-212-700)",
                              }}
                            >
                              Recommended next step
                            </Typography>
                            <Typography
                              variant="body2"
                              sx={{
                                mt: 0.6,
                                ...(useMonospaceFix
                                  ? {
                                      fontFamily:
                                        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                                    }
                                  : {}),
                                whiteSpace: "pre-wrap",
                                overflowWrap: "anywhere",
                                color: "var(--ui-rgba-245-247-250-920)",
                              }}
                            >
                              {fix}
                            </Typography>
                          </Box>
                          {!canRunFix ? (
                            <Alert
                              severity="warning"
                              icon={<ErrorOutlineRoundedIcon />}
                            >
                              <Typography variant="body2">
                                {arkPulseManualFollowupText()}
                              </Typography>
                            </Alert>
                          ) : (
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                              }}
                            >
                              <Button
                                size="small"
                                variant="outlined"
                                startIcon={
                                  <ContentCopyRoundedIcon fontSize="small" />
                                }
                                sx={{
                                  borderRadius: "8px",
                                  textTransform: "none",
                                }}
                                disabled={!canCopyFix}
                                onClick={async () => {
                                  setError(null);
                                  setSuccess(null);
                                  try {
                                    await copyClipboardText(fix);
                                    setSuccess("Remediation copied.");
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                }}
                              >
                                Copy next step
                              </Button>
                              <Button
                                size="small"
                                variant="contained"
                                sx={{
                                  borderRadius: "8px",
                                  textTransform: "none",
                                }}
                                disabled={runPulseFixMutation.isPending}
                                onClick={async () => {
                                  setError(null);
                                  setSuccess(null);
                                  setActivePulseFixId(fixActionId);
                                  setPulseFixResultsById((prev) => {
                                    if (!prev[fixActionId]) return prev;
                                    const next = { ...prev };
                                    delete next[fixActionId];
                                    return next;
                                  });
                                  try {
                                    const result =
                                      await runPulseFixMutation.mutateAsync({
                                        fixCommand: rawFixCommand,
                                        remediation: typedRemediation,
                                        issueTitle: title,
                                        target,
                                        eventTimestamp: str(
                                          selectedPulseEvent?.timestamp,
                                          "",
                                        ),
                                        findingIndex: finding.findingIndex,
                                      });
                                    setPulseFixResultsById((prev) => ({
                                      ...prev,
                                      [fixActionId]: {
                                        severity: "success",
                                        message:
                                          str(
                                            result.message,
                                            "Pulse diagnostic completed.",
                                          ).trim() ||
                                          "Pulse diagnostic completed.",
                                        output: str(result.output, "").trim(),
                                        timestamp: new Date().toISOString(),
                                      },
                                    }));
                                  } catch (e) {
                                    setPulseFixResultsById((prev) => ({
                                      ...prev,
                                      [fixActionId]: {
                                        severity: "error",
                                        message: errMessage(e),
                                        output: "",
                                        timestamp: new Date().toISOString(),
                                      },
                                    }));
                                  } finally {
                                    setActivePulseFixId((prev) =>
                                      prev === fixActionId ? null : prev,
                                    );
                                  }
                                }}
                              >
                                {fixBusy
                                  ? "Running..."
                                  : arkPulseRunActionLabel(displayRemediation)}
                              </Button>
                            </Stack>
                          )}
                          {inlineFixResult ? (
                            <Alert
                              severity={inlineFixResult.severity}
                              className="arkpulse-inline-result"
                            >
                              <Typography
                                variant="body2"
                                className="arkpulse-inline-result-message"
                              >
                                {inlineFixResult.message}
                              </Typography>
                              {inlineFixResult.output ? (
                                <Box
                                  component="pre"
                                  className="arkpulse-inline-result-output"
                                >
                                  {inlineFixResult.output}
                                </Box>
                              ) : null}
                            </Alert>
                          ) : null}
                          <Typography
                            variant="caption"
                            sx={{
                              color: "var(--ui-rgba-188-198-212-660)",
                              lineHeight: 1.45,
                            }}
                          >
                            {arkPulseRemediationFootnote(
                              typedRemediation,
                              canRunFix,
                            )}
                          </Typography>
                        </Stack>
                      </Box>
                    </Grid2>
                  );
                })}
              </Grid2>
            )}

            <Stack spacing={0.3} sx={{ pt: 0.25 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                Run ledger
              </Typography>
              <Typography
                variant="body2"
                sx={{ color: "var(--ui-rgba-188-198-212-720)" }}
              >
                This shows exactly what Pulse scanned, how long each phase
                took, and what happened with notifications. Sections stay
                collapsed until you open them.
              </Typography>
            </Stack>
            <Box
              sx={{
                borderRadius: "8px",
                border: "1px solid var(--ui-rgba-255-255-255-080)",
                background: "var(--ui-rgba-255-255-255-020)",
                p: 1.2,
              }}
            >
              <Stack
                direction="row"
                spacing={1}
                useFlexGap
                sx={{ flexWrap: "wrap" }}
              >
                <Chip
                  size="small"
                  variant="outlined"
                  label={`Scans: ${selectedPulseScanLog.length}`}
                  sx={{ borderColor: "var(--ui-rgba-255-255-255-120)" }}
                />
                {selectedPulseScanDurationMs > 0 ? (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`Duration: ${formatTraceDuration(selectedPulseScanDurationMs)}`}
                    sx={{ borderColor: "var(--ui-rgba-255-255-255-120)" }}
                  />
                ) : null}
                {selectedPulseNotificationOutcome ? (
                  <Chip
                    size="small"
                    color={pulseScanStatusColor(
                      selectedPulseNotificationOutcome,
                    )}
                    variant="outlined"
                    label={`Notify: ${pulseScanStatusLabel(selectedPulseNotificationOutcome)}`}
                  />
                ) : null}
                {selectedPulseScanStarted ? (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`Started: ${formatTimestampForHumans(selectedPulseScanStarted).label}`}
                    title={
                      formatTimestampForHumans(selectedPulseScanStarted).tooltip
                    }
                    sx={{ borderColor: "var(--ui-rgba-255-255-255-120)" }}
                  />
                ) : null}
                {selectedPulseScanFinished ? (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={`Finished: ${formatTimestampForHumans(selectedPulseScanFinished).label}`}
                    title={
                      formatTimestampForHumans(selectedPulseScanFinished)
                        .tooltip
                    }
                    sx={{ borderColor: "var(--ui-rgba-255-255-255-120)" }}
                  />
                ) : null}
              </Stack>
            </Box>
            {selectedPulseScanLog.length === 0 ? (
              <Box
                sx={{
                  borderRadius: "8px",
                  border: "1px dashed var(--ui-rgba-255-255-255-100)",
                  background: "var(--ui-rgba-255-255-255-020)",
                  px: 1.4,
                  py: 1.25,
                }}
              >
                <Typography
                  variant="body2"
                  sx={{ color: "var(--ui-rgba-188-198-212-720)" }}
                >
                  This run does not include a detailed scan ledger yet.
                </Typography>
              </Box>
            ) : (
              <Stack spacing={1}>
                {selectedPulseScanLog.map((section, idx) => {
                  const row = asRecord(section);
                  const metrics = pickRecords(row, "metrics");
                  const status = str(row.status, "ok");
                  const durationMs = num(row.duration_ms, 0);
                  const summary = str(row.summary, "").trim();
                  const detail = str(row.detail, "").trim();
                  const alertReason = pulseScanAlertReason(row);
                  const alertReasonColor =
                    pulseScanStatusColor(status) === "error"
                      ? "error.main"
                      : "warning.main";
                  return (
                    <Accordion
                      key={`${str(row.id, `scan-${idx}`)}-${idx}`}
                      disableGutters
                      sx={{
                        background: "var(--ui-rgba-255-255-255-020)",
                        boxShadow: "none",
                        border: "1px solid var(--ui-rgba-255-255-255-080)",
                        borderRadius: "8px",
                        overflow: "hidden",
                        "&:before": { display: "none" },
                      }}
                    >
                      <AccordionSummary
                        expandIcon={
                          <ExpandMoreIcon
                            sx={{ color: "var(--ui-rgba-196-223-255-820)" }}
                          />
                        }
                        sx={{
                          minHeight: 54,
                          "& .MuiAccordionSummary-content": {
                            my: 1,
                            alignItems: "center",
                            gap: 1,
                            flexWrap: "wrap",
                          },
                        }}
                      >
                        <Box
                          sx={{
                            minWidth: 28,
                            height: 28,
                            borderRadius: "8px",
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "center",
                            background: "var(--ui-rgba-255-255-255-060)",
                            color: "var(--ui-rgba-243-246-250-920)",
                            fontSize: "0.8rem",
                            fontWeight: 700,
                          }}
                        >
                          {idx + 1}
                        </Box>
                        <Stack spacing={0.15} sx={{ minWidth: 0, flex: 1 }}>
                          <Typography
                            variant="subtitle2"
                            sx={{ fontWeight: 700 }}
                          >
                            {str(row.title, `Scan ${idx + 1}`)}
                          </Typography>
                          <Typography
                            variant="body2"
                            sx={{ color: "var(--ui-rgba-188-198-212-720)" }}
                            noWrap
                          >
                            {summary || "No summary recorded."}
                          </Typography>
                          {alertReason ? (
                            <Typography
                              variant="caption"
                              sx={{ color: alertReasonColor }}
                              noWrap
                              title={alertReason}
                            >
                              Why: {alertReason}
                            </Typography>
                          ) : null}
                        </Stack>
                        {durationMs > 0 ? (
                          <Chip
                            size="small"
                            variant="outlined"
                            label={formatTraceDuration(durationMs)}
                          />
                        ) : null}
                        <Chip
                          size="small"
                          color={pulseScanStatusColor(status)}
                          label={pulseScanStatusLabel(status)}
                        />
                      </AccordionSummary>
                      <AccordionDetails sx={{ pt: 0, pb: 1.2 }}>
                        <Stack spacing={1}>
                          {detail ? (
                            <Typography
                              variant="body2"
                              sx={{
                                color: "var(--ui-rgba-231-236-243-780)",
                                lineHeight: 1.6,
                              }}
                            >
                              {detail}
                            </Typography>
                          ) : null}
                          {metrics.length > 0 ? (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{ flexWrap: "wrap" }}
                            >
                              {metrics.map((metric, metricIdx) => {
                                const metricRow = asRecord(metric);
                                const metricLabel = str(
                                  metricRow.label,
                                  "",
                                ).trim();
                                const metricValue = str(
                                  metricRow.value,
                                  "",
                                ).trim();
                                if (!metricLabel && !metricValue) return null;
                                return (
                                  <Chip
                                    key={`${metricLabel}-${metricValue}-${metricIdx}`}
                                    size="small"
                                    variant="outlined"
                                    label={
                                      metricLabel
                                        ? `${metricLabel}: ${metricValue || "-"}`
                                        : metricValue || "-"
                                    }
                                    sx={{
                                      borderColor: "var(--ui-rgba-255-255-255-120)",
                                    }}
                                  />
                                );
                              })}
                            </Stack>
                          ) : null}
                        </Stack>
                      </AccordionDetails>
                    </Accordion>
                  );
                })}
              </Stack>
            )}

            <Stack spacing={0.3} sx={{ pt: 0.25 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                System snapshot
              </Typography>
              <Typography
                variant="body2"
                sx={{ color: "var(--ui-rgba-188-198-212-720)" }}
              >
                Runtime state captured with this health check.
              </Typography>
            </Stack>
            <Grid2 container spacing={1}>
              {selectedPulseSnapshot.map((item) => (
                <Grid2 key={item.label} size={{ xs: 6, md: 3 }}>
                  <Box
                    sx={{
                      height: "100%",
                      borderRadius: "8px",
                      border: "1px solid var(--ui-rgba-255-255-255-070)",
                      background: "var(--ui-rgba-255-255-255-020)",
                      p: 1.2,
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{
                        display: "block",
                        color: "var(--ui-rgba-188-198-212-680)",
                      }}
                    >
                      {item.label}
                    </Typography>
                    <Typography
                      variant="h6"
                      sx={{
                        mt: 0.35,
                        fontWeight: 700,
                        fontVariantNumeric: "tabular-nums",
                        fontFamily:
                          item.label === "Uptime"
                            ? "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
                            : undefined,
                      }}
                    >
                      {item.value}
                    </Typography>
                  </Box>
                </Grid2>
              ))}
            </Grid2>

            {savedDeveloperModeEnabled ? (
              <Accordion
                disableGutters
                sx={{
                  background: "var(--ui-rgba-255-255-255-020)",
                  boxShadow: "none",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  borderRadius: "8px",
                  "&:before": { display: "none" },
                }}
              >
                <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                  <Typography variant="subtitle2">
                    Technical signals (developer mode)
                  </Typography>
                </AccordionSummary>
                <AccordionDetails sx={{ pt: 0 }}>
                  <KeyValuePanel
                    title="Raw signals"
                    data={asRecord(selectedPulseEvent?.details)}
                    emptyLabel="No extra signals."
                    maxRows={24}
                  />
                </AccordionDetails>
              </Accordion>
            ) : null}
          </Stack>
        </DialogContent>
      </Dialog>
      <Dialog
        open={vaultEditorOpen}
        onClose={closeVaultEditor}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Add Secret</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <TextField
              label="Secret key"
              value={vaultEditorKey}
              onChange={(e) => setVaultEditorKey(e.target.value)}
              fullWidth
              size="small"
              helperText="Allowed: letters, numbers, _, -, :, ."
            />
            <TextField
              label="Secret value"
              value={vaultEditorValue}
              onChange={(e) => setVaultEditorValue(e.target.value)}
              fullWidth
              size="small"
              multiline
              minRows={3}
              type={showVaultSecretValue ? "text" : "password"}
              placeholder="Paste secret value"
            />
            <FormControlLabel
              control={
                <Switch
                  checked={showVaultSecretValue}
                  onChange={(e) => setShowVaultSecretValue(e.target.checked)}
                />
              }
              label="Show secret value"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeVaultEditor}
            disabled={upsertVaultSecretMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            onClick={submitVaultEditor}
            disabled={upsertVaultSecretMutation.isPending}
          >
            {upsertVaultSecretMutation.isPending ? "Saving..." : "Save Secret"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={searchProviderDialog != null}
        onClose={closeSearchProviderDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {searchDialogProvider
            ? searchDialogConfigured
              ? `Replace ${searchDialogProvider.label} API key`
              : `Enable ${searchDialogProvider.label}`
            : "Search provider"}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              {searchDialogProvider
                ? searchDialogConfigured
                  ? `Paste a new ${searchDialogProvider.label} API key. The previous key will be overwritten when you save settings.`
                  : `Paste your ${searchDialogProvider.label} API key. AgentArk stores it encrypted and uses it in the configured provider order.`
                : ""}
            </Typography>
            <TextField
              label="API key"
              value={searchProviderDialog?.value ?? ""}
              onChange={(e) =>
                setSearchProviderDialog((prev) =>
                  prev ? { ...prev, value: e.target.value } : prev,
                )
              }
              onKeyDown={(e) => {
                if (
                  e.key === "Enter" &&
                  (searchProviderDialog?.value ?? "").trim().length > 0
                ) {
                  e.preventDefault();
                  submitSearchProviderDialog();
                }
              }}
              fullWidth
              autoFocus
              size="small"
              type={searchProviderDialog?.showValue ? "text" : "password"}
              placeholder={
                searchDialogProvider
                  ? `Enter ${searchDialogProvider.label} API key`
                  : ""
              }
            />
            <FormControlLabel
              control={
                <Switch
                  checked={Boolean(searchProviderDialog?.showValue)}
                  onChange={(e) =>
                    setSearchProviderDialog((prev) =>
                      prev ? { ...prev, showValue: e.target.checked } : prev,
                    )
                  }
                />
              }
              label="Show key"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeSearchProviderDialog}>Cancel</Button>
          <Button
            variant="contained"
            onClick={submitSearchProviderDialog}
            disabled={
              !searchProviderDialog ||
              searchProviderDialog.value.trim().length === 0
            }
          >
            {searchDialogConfigured ? "Replace key" : "Save key"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={searxngDialog != null}
        onClose={() => setSearxngDialog(null)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {searxngDialogConfigured
            ? "Replace SearXNG URL"
            : "Enable SearXNG"}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Enter the base URL of your SearXNG instance. AgentArk will call{" "}
              <code>/search?format=json</code> against this URL.
            </Typography>
            <TextField
              label="SearXNG base URL"
              value={searxngDialog?.value ?? ""}
              onChange={(e) =>
                setSearxngDialog((prev) =>
                  prev ? { ...prev, value: e.target.value } : prev,
                )
              }
              onKeyDown={(e) => {
                if (
                  e.key === "Enter" &&
                  (searxngDialog?.value ?? "").trim().length > 0
                ) {
                  e.preventDefault();
                  if (searxngDialog) {
                    setField(
                      "search_searxng_base_url",
                      searxngDialog.value.trim(),
                    );
                    setSearxngDialog(null);
                  }
                }
              }}
              fullWidth
              autoFocus
              size="small"
              placeholder="https://search.example.com"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSearxngDialog(null)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => {
              if (!searxngDialog) return;
              const trimmed = searxngDialog.value.trim();
              if (!trimmed) return;
              setField("search_searxng_base_url", trimmed);
              setSearxngDialog(null);
            }}
            disabled={
              !searxngDialog || searxngDialog.value.trim().length === 0
            }
          >
            {searxngDialogConfigured ? "Replace URL" : "Save URL"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={passwordDialogMode != null}
        onClose={closePasswordDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {passwordDialogMode === "set"
            ? "Set Master Password"
            : passwordDialogMode === "change"
              ? "Change Master Password"
              : "Remove Master Password"}
        </DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              {resumeTunnelStartAfterPassword
                ? selectedTunnelMeta.isPrivate
                  ? "Save a custom AgentArk password to finish creating the private access URL."
                  : "Save a custom AgentArk password to finish creating the public link."
                : "Password changes apply immediately to this running AgentArk session."}
            </Alert>
            <FormControlLabel
              control={
                <Switch
                  checked={showPasswordInputs}
                  onChange={(e) => setShowPasswordInputs(e.target.checked)}
                />
              }
              label="Show password text"
            />
            {passwordDialogMode === "set" ? (
              <>
                <TextField
                  label="New password (min 8 chars)"
                  value={secNewPassword}
                  onChange={(e) => setSecNewPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="Confirm new password"
                  value={secConfirmPassword}
                  onChange={(e) => setSecConfirmPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
            {passwordDialogMode === "change" ? (
              <>
                <TextField
                  label="Current password (blank uses default, if applicable)"
                  value={secCurrentPassword}
                  onChange={(e) => setSecCurrentPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="New password (min 8 chars)"
                  value={secNewPassword}
                  onChange={(e) => setSecNewPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
                <TextField
                  label="Confirm new password"
                  value={secConfirmPassword}
                  onChange={(e) => setSecConfirmPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
            {passwordDialogMode === "remove" ? (
              <>
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  Removes the master password and returns to keyfile-based
                  encryption.
                </Typography>
                <TextField
                  label="Current password"
                  value={secCurrentPassword}
                  onChange={(e) => setSecCurrentPassword(e.target.value)}
                  fullWidth
                  type={showPasswordInputs ? "text" : "password"}
                  size="small"
                />
              </>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closePasswordDialog}
            disabled={passwordMutationPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color={passwordDialogMode === "remove" ? "error" : "primary"}
            onClick={submitPasswordDialog}
            disabled={passwordMutationPending}
          >
            {passwordMutationPending
              ? "Saving..."
              : passwordDialogMode === "set"
                ? "Set Password"
                : passwordDialogMode === "change"
                  ? "Change Password"
                  : "Remove Password"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={restartDialogOpen}
        onClose={closeRestartDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Restart AgentArk</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              This restarts control, executor, and workspace. Postgres stays
              running.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Active chats, running jobs, and in-flight approvals can be
              interrupted while AgentArk comes back online.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeRestartDialog}
            disabled={restartMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitRestartDialog}
            disabled={restartMutation.isPending}
          >
            {restartMutation.isPending ? "Restarting..." : "Restart AgentArk"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={autonomyPauseDialogOpen}
        onClose={closeAutonomyPauseDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Pause autonomy?</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              This pauses autonomous background work. Scheduled reminders still
              fire.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              While autonomy is paused:
            </Typography>
            <Typography variant="body2">
              1. New scheduled non-reminder automations and routine runs stay
              paused.
            </Typography>
            <Typography variant="body2">
              2. Watchers, external polling triggers, and Pulse health checks
              stop running.
            </Typography>
            <Typography variant="body2">
              3. Background learning, suggestion scans, self-evolve work, and
              proactive optimizations pause.
            </Typography>
            <Typography variant="body2">
              4. Proactive outbound notifications from those systems pause.
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Existing tasks, watchers, and history stay saved. AgentArk will
              remind you every 7 days while autonomy remains paused.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeAutonomyPauseDialog}
            disabled={settingsAutonomyMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitAutonomyPauseDialog}
            disabled={settingsAutonomyMutation.isPending}
          >
            {settingsAutonomyMutation.isPending ? "Pausing..." : "Pause autonomy"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={selfEvolveDisableDialogOpen}
        onClose={closeSelfEvolveDisableDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Turn off Self-evolve?</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              This stops background learning and turns off active evolution
              canaries.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              While Self-evolve is off:
            </Typography>
            <Typography variant="body2">
              1. Heuristic reflection, experience consolidation, pattern
              induction, and candidate generation stop processing recent work.
            </Typography>
            <Typography variant="body2">
              2. New draft improvements stop appearing for review.
            </Typography>
            <Typography variant="body2">
              3. Active prompt and routing canary experiments are disabled.
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Existing learned items, approved procedures, and past review
              history stay saved.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeSelfEvolveDisableDialog}
            disabled={settingsEvolutionMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitSelfEvolveDisableDialog}
            disabled={settingsEvolutionMutation.isPending}
          >
            {settingsEvolutionMutation.isPending
              ? "Turning off..."
              : "Turn off Self-evolve"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={sentinelDisableDialogOpen}
        onClose={closeSentinelDisableDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Turn off Sentinel?</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              This stops Sentinel follow-up scanning in the background.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              While Sentinel is off:
            </Typography>
            <Typography variant="body2">
              1. New follow-up suggestions from in-app and connected-app activity
              stop.
            </Typography>
            <Typography variant="body2">
              2. Routine-detection proposals stop appearing.
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              3. Existing preferences stay saved, but Sentinel stays off until
              you turn it back on.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeSentinelDisableDialog}
            disabled={settingsSentinelMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitSentinelDisableDialog}
            disabled={settingsSentinelMutation.isPending}
          >
            {settingsSentinelMutation.isPending
              ? "Turning off..."
              : "Turn off Sentinel"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={sentinelInAppDisableDialogOpen}
        onClose={closeSentinelInAppDisableDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Stop watching AgentArk activity?</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              Sentinel will stop surfacing follow-ups from in-app chat and
              execution runs.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Connected-app signals can still appear if that source remains on.
              In-app signals such as failed, blocked, degraded, stalled, or
              needs-input runs will stay hidden until you turn this back on.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeSentinelInAppDisableDialog}
            disabled={settingsSentinelMutation.isPending}
          >
            Keep watching
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitSentinelInAppDisableDialog}
            disabled={settingsSentinelMutation.isPending}
          >
            {settingsSentinelMutation.isPending
              ? "Turning off..."
              : "Stop watching AgentArk"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={rotateInternalCredentialsDialogOpen}
        onClose={closeRotateInternalCredentialsDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Rotate Internal Credentials</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              AgentArk will replace the executor and workspace trust
              credentials, then restart the stack immediately.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Active tasks, tool runs, and internal requests can be interrupted
              while control, executor, and workspace restart.
            </Typography>
            <FormControlLabel
              control={
                <Checkbox
                  checked={rotateInternalCredentialsAccepted}
                  onChange={(e) =>
                    setRotateInternalCredentialsAccepted(e.target.checked)
                  }
                />
              }
              label="I accept the credential swap and restart."
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={closeRotateInternalCredentialsDialog}
            disabled={rotateInternalServiceTokensMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            color="warning"
            onClick={submitRotateInternalCredentials}
            disabled={
              rotateInternalServiceTokensMutation.isPending ||
              !rotateInternalCredentialsAccepted
            }
          >
            {rotateInternalServiceTokensMutation.isPending
              ? "Rotating..."
              : "I Accept and Rotate"}
          </Button>
        </DialogActions>
      </Dialog>
      {activeSettingsDataError ? (
        <Alert severity="error">
          {errMessage(activeSettingsDataError)}
        </Alert>
      ) : null}
      {error ? <Alert severity="error">{error}</Alert> : null}
      {modelConnectivityWarning ? (
        <Alert severity="warning">{modelConnectivityWarning}</Alert>
      ) : null}
      {success ? <Alert severity="success">{success}</Alert> : null}
    </Stack>
  );
}
