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
  CircularProgress,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  IconButton,
  Link,
  ListItemText,
  MenuItem,
  Stack,
  Switch,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import ArrowDropDownRoundedIcon from "@mui/icons-material/ArrowDropDownRounded";
import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import CloseIcon from "@mui/icons-material/Close";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import ErrorOutlineRoundedIcon from "@mui/icons-material/ErrorOutlineRounded";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import OpenInFullRoundedIcon from "@mui/icons-material/OpenInFullRounded";
import RadioButtonUncheckedRoundedIcon from "@mui/icons-material/RadioButtonUncheckedRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import StarBorderRoundedIcon from "@mui/icons-material/StarBorderRounded";
import StarRoundedIcon from "@mui/icons-material/StarRounded";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  useCallback,
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
  getTunnelPanelWarning,
  getTunnelProviderHelp,
  getTunnelStartButtonLabel,
  getTunnelStopButtonLabel,
  getTunnelUrlFieldLabel,
} from "../../lib/tunnelAccess";
import {
  formatUiDateOnly,
  formatUiDateRange,
  formatUiDateTime,
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import {
  isBackgroundSessionVisibleInUi,
  isOneShotReminderTask,
  taskActionDisplay,
  taskKind,
  taskKindLabel,
} from "../../lib/backgroundSessions";
import type {
  ArkPulseRemediationSpec,
  ArkPulseRunFixRequest,
  BackgroundSessionSummary,
  Task,
} from "../../types";
import { useUiStore } from "../../store/uiStore";
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
  formatBytes,
  humanTs,
  KeyValuePanel,
  RowOpsMenu,
} from "./workspaceUiBits";
import {
  asRecords,
  boolText,
  DEVELOPER_MODE_EVENT,
  getDeveloperModeEnabled,
  humanizeStatusLabel,
  OLLAMA_DEFAULT_BASE_URL,
  OPENROUTER_DEFAULT_BASE_URL,
  REFRESH_MS,
  setDeveloperModeEnabled,
} from "./workspaceCore";
import {
  ADVANCED_SENTINEL_SIGNAL_OPTIONS,
  MODEL_PROVIDER_OPTIONS,
  TRUST_APPROVAL_PRESETS,
  type TrustApprovalPreset,
} from "./settingsConstants";
import {
  AUTO_APPROVE_ACTION_OPTIONS,
  arkPulseRemediationFootnote,
  arkPulseRunActionLabel,
  collapseInlineWhitespace,
  describeArkPulseRemediation,
  formatDurationClock,
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
  tunnelCheckAlertSeverity,
  tunnelCheckChipColor,
  tunnelCheckLabel,
} from "./settingsPageHelpers";
import {
  CompanionDevicesPanel,
  IntegrationQuickstartPanel,
  IntegrationsPanel,
  MediaSettingsPanel,
  MemoryPage,
  ObservabilityPanel,
  PluginSdkPanel,
  TracePage,
  WebhooksPanel,
} from "./settingsLazyPanels";
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
  fetchModels,
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
  prefetchSettingsPageData,
  SETTINGS_CACHE_GC_TIME_MS,
  SETTINGS_QUERY_KEYS,
} from "./settingsData";
import {
  getSettingsPageMeta,
  resolveInitialSettingsTab,
  settingsTabSupportsSave,
  type SettingsPageProps,
} from "./settingsMeta";
import {
  preloadCommonSettingsPanels,
  preloadSettingsTab,
} from "./workspacePreload";

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

type ArkPulseInlineResult = {
  severity: "success" | "error";
  message: string;
  output: string;
  timestamp: string;
};

type PasswordDialogMode = "set" | "change" | "remove";

export default function SettingsPage({
  autoRefresh,
  initialTab,
  hideSettingsNav,
  standaloneSurface,
}: SettingsPageProps) {
  const LOCAL_EMBEDDINGS_MODEL = "BAAI/bge-small-en-v1.5";
  const queryClient = useQueryClient();
  const [tab, setTab] = useState(() => resolveInitialSettingsTab(initialTab));
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
  const [securityLogsDialogOpen, setSecurityLogsDialogOpen] = useState(false);
  const [selectedPulseEvent, setSelectedPulseEvent] =
    useState<JsonRecord | null>(null);
  const [activePulseFixId, setActivePulseFixId] = useState<string | null>(null);
  const [pulseFixResultsById, setPulseFixResultsById] = useState<
    Record<string, ArkPulseInlineResult>
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
  const [trustPresetId, setTrustPresetId] = useState(
    TRUST_APPROVAL_PRESETS[0]?.id ?? "run_terminal_command",
  );
  const [trustPresetDetail, setTrustPresetDetail] = useState("ls -la");
  const [trustUseAdvancedInput, setTrustUseAdvancedInput] = useState(false);
  const [trustActionKind, setTrustActionKind] = useState("shell");
  const [trustPayloadJson, setTrustPayloadJson] = useState("{}");
  const [trustResult, setTrustResult] = useState<JsonRecord | null>(null);
  const [tunnelSelectedProviderId, setTunnelSelectedProviderId] = useState("");
  const [tunnelDraftValues, setTunnelDraftValues] = useState<
    Record<string, string>
  >({});
  const [showTunnelAdvanced, setShowTunnelAdvanced] = useState(false);
  const [tunnelSetupChecks, setTunnelSetupChecks] = useState<JsonRecord[]>([]);
  const [tunnelPanelNotice, setTunnelPanelNotice] = useState<{
    severity: "success" | "info" | "warning";
    text: string;
  } | null>(null);
  const [resumeTunnelStartAfterPassword, setResumeTunnelStartAfterPassword] =
    useState(false);
  const securityTabActive = tab === 4;
  const advancedTabActive = tab === 5;
  const observabilityTabActive = tab === 6;
  const pulseTabActive = tab === 9;
  const standaloneArkPulse = standaloneSurface === "arkpulse";
  const updatesTabActive = tab === 25;
  const backgroundSettingsDataEnabled = !standaloneArkPulse;

  useEffect(() => {
    const nextTab = resolveInitialSettingsTab(initialTab);
    setTab((current) => (current === nextTab ? current : nextTab));
  }, [initialTab]);

  useEffect(() => {
    preloadSettingsTab(tab);
  }, [tab]);

  useEffect(() => {
    if (standaloneArkPulse) return;
    preloadCommonSettingsPanels();
    prefetchSettingsPageData(queryClient);
    const timer = window.setTimeout(preloadCommonSettingsPanels, 260);
    return () => window.clearTimeout(timer);
  }, [queryClient, standaloneArkPulse]);

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
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const mediaQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.media,
    queryFn: fetchSettingsMedia,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const modelsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.models,
    queryFn: fetchModels,
    staleTime: CORE_SETTINGS_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const updateStatusQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.updateStatus,
    queryFn: fetchSettingsUpdateStatus,
    enabled: updatesTabActive || backgroundSettingsDataEnabled,
    staleTime: 60_000,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: false,
    refetchOnWindowFocus: false,
  });
  const apiKeyQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.apiKey,
    queryFn: fetchSettingsApiKey,
    enabled: advancedTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive ? 10000 : false,
    refetchIntervalInBackground: advancedTabActive,
  });
  const settingsAutonomyQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.autonomySettings,
    queryFn: fetchSettingsAutonomy,
    enabled: advancedTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const settingsEvolutionQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.evolution,
    queryFn: fetchSettingsEvolution,
    enabled: advancedTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const settingsSentinelQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.sentinel,
    queryFn: fetchSettingsSentinel,
    enabled: advancedTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: advancedTabActive && autoRefresh ? REFRESH_MS : false,
    refetchIntervalInBackground: advancedTabActive && autoRefresh,
  });
  const tunnelQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.tunnelStatus,
    queryFn: fetchTunnelStatus,
    enabled: securityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
  });
  const tunnelProvidersQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.tunnelProviders,
    queryFn: fetchTunnelProviders,
    enabled: securityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
  });
  const securityStatusQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.securityStatus,
    queryFn: fetchSecurityStatus,
    enabled: securityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
  });
  const abuseReviewsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.securityAbuseReviews,
    queryFn: fetchSecurityAbuseReviews,
    enabled: securityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: securityTabActive && autoRefresh ? REFRESH_MS : false,
  });
  const abuseReviews = pickRecords(abuseReviewsQ.data, "reviews");
  const securityLogsQ = useQuery({
    queryKey: ["settings-security-logs-dialog"],
    queryFn: () => api.rawGet("/security/logs?limit=80"),
    enabled: tab === 4 && securityLogsDialogOpen,
    refetchInterval: securityLogsDialogOpen && autoRefresh ? REFRESH_MS : false,
  });
  const observabilityLogsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.observabilityLogs,
    queryFn: fetchSettingsObservabilityLogs,
    enabled: observabilityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: observabilityTabActive && autoRefresh ? REFRESH_MS : false,
  });
  const vaultSecretsQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.secrets,
    queryFn: fetchSettingsSecrets,
    enabled: securityTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: false,
  });
  const pulseQ = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.arkPulseLog,
    queryFn: fetchArkPulseLog,
    enabled: pulseTabActive || backgroundSettingsDataEnabled,
    staleTime: SETTINGS_BACKGROUND_STALE_TIME_MS,
    gcTime: SETTINGS_CACHE_GC_TIME_MS,
    refetchInterval: pulseTabActive
      ? pulsePollState
        ? 2000
        : autoRefresh
          ? REFRESH_MS
          : false
      : false,
  });
  const settings = asRecord(settingsQ.data);
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
  const modelsPayload = asRecord(modelsQ.data);
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
    model_privacy_current_chat_pii_policy: "raw_current_turn",
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

  function parseBoundedFloat(
    value: string,
    label: string,
    min: number,
    max: number,
  ): number {
    const trimmed = value.trim();
    if (!trimmed) throw new Error(`${label} is required.`);
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed) || parsed < min || parsed > max) {
      throw new Error(`${label} must be between ${min} and ${max}.`);
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
      smart_routing: toBool(settings.smart_routing),
      embeddings_provider: str(settings.embeddings_provider, "local-hf"),
      embeddings_model: str(settings.embeddings_model, LOCAL_EMBEDDINGS_MODEL),
      embeddings_base_url: str(settings.embeddings_base_url, ""),
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
        "raw_current_turn",
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
    setSavedFormSnapshot(snapshotSettingsForm(nextForm));

    setDirty(false);
    setError(null);
    setSuccess(null);
  }

  // Initialize form from backend once; keep defaults if backend is down.
  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.isSuccess) return;
    hydrateFromServer();
    setInitialized(true);
    setDirty(false);
  }, [initialized, settingsQ.isSuccess, settingsQ.dataUpdatedAt]);

  useEffect(() => {
    if (initialized) return;
    if (!settingsQ.data || !mediaQ.data) return;
    hydrateFromServer();
    setInitialized(true);
    setDirty(false);
  }, [initialized, settingsQ.data, mediaQ.data]); // eslint-disable-line react-hooks/exhaustive-deps

  // Safety: clear dirty once after hydration settles (handles race between effects)
  const hydrationDirtyCleared = useRef(false);
  useEffect(() => {
    if (initialized && !hydrationDirtyCleared.current) {
      hydrationDirtyCleared.current = true;
      setDirty(false);
    }
  }, [initialized]);

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
        smart_routing: form.smart_routing,
        embeddings_provider: form.embeddings_provider || "local-hf",
        embeddings_model:
          (form.embeddings_provider || "local-hf") === "local-hf"
            ? LOCAL_EMBEDDINGS_MODEL
            : form.embeddings_model || LOCAL_EMBEDDINGS_MODEL,
        embeddings_base_url: form.embeddings_base_url || null,
        embeddings_api_key: form.embeddings_api_key || null,

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
          form.model_privacy_current_chat_pii_policy || "raw_current_turn",
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
          { timeoutMs: 15000 },
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
  const selectedTunnelDescription = str(
    selectedTunnelProviderRecord?.description,
    "",
  ).trim();
  const selectedTunnelAvailable = toBool(
    selectedTunnelProviderRecord?.available,
  );
  const selectedTunnelConfigured = toBool(
    selectedTunnelProviderRecord?.configured,
  );
  const selectedTunnelMeta = getTunnelAccessMeta(selectedTunnelProviderRecord);
  const activeTunnelMeta = getTunnelAccessMeta(tunnel);
  const recommendedPublicTunnelProvider = useMemo(() => {
    const publicProviders = tunnelProviders.filter(
      (provider) => !getTunnelAccessMeta(provider).isPrivate,
    );
    return (
      publicProviders.find(
        (provider) => toBool(provider.available) && toBool(provider.configured),
      ) ||
      publicProviders.find((provider) => toBool(provider.available)) ||
      publicProviders[0] ||
      null
    );
  }, [tunnelProviders]);
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

  const securityLogs = pickRecords(securityLogsQ.data, "logs");
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
  const tunnelHasDetails =
    advancedTunnelConfigFields.length > 0 ||
    selectedTunnelDescription.length > 0 ||
    selectedTunnelHelp.length > 0 ||
    tunnelGuidanceText.length > 0;
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
      pickRecords(selectedPulseDetails, "doctor_findings").filter((f) =>
        isUserActionableDoctorFinding(f),
      ),
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
          "Use the recommended remediation under each issue, then run ArkPulse again.",
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
    ? "ArkPulse is currently running."
    : pulseEvents.length === 0
      ? pulseHistoryUnavailable
        ? "Earlier ArkPulse history is unavailable."
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
          "A previous ArkPulse payload exists, but this runtime could not decrypt it. New runs will appear normally."
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

  function securityEventTypeLabel(eventType: string): string {
    const normalized = (eventType || "").trim().toLowerCase();
    if (!normalized) return "Unknown";
    return normalized
      .replace(/_/g, " ")
      .replace(/\b\w/g, (m) => m.toUpperCase());
  }

  function securitySeverityAccent(sev: string): string {
    const s = (sev || "").trim().toLowerCase();
    if (s === "critical" || s === "high" || s === "error")
      return "var(--ui-rgba-248-113-113-900)";
    if (s === "medium" || s === "warn" || s === "warning")
      return "var(--ui-rgba-251-191-36-880)";
    if (s === "low") return "var(--ui-rgba-56-189-248-880)";
    if (s === "ok" || s === "info") return "var(--ui-rgba-52-211-153-880)";
    return "var(--ui-rgba-255-255-255-200)";
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
      remediation?: ArkPulseRemediationSpec | null;
      issueTitle: string;
      target: string;
      eventTimestamp?: string;
      findingIndex?: number;
    }) => {
      const body: ArkPulseRunFixRequest = {
        issue_title: payload.issueTitle,
        target: payload.target,
        event_timestamp: payload.eventTimestamp || undefined,
        finding_index: payload.findingIndex,
      };
      const fixCommand = payload.fixCommand.trim();
      if (fixCommand) {
        body.fix_command = fixCommand;
      }
      if (payload.remediation) {
        body.remediation = payload.remediation;
      }
      const out = asRecord(await api.rawPost("/arkpulse/fix", body));
      const status = str(out.status, "").toLowerCase();
      if (status === "error") {
        const errorText =
          str(out.error, "").trim() ||
          str(out.message, "").trim() ||
          "ArkPulse fix failed.";
        throw new Error(errorText);
      }
      return out;
    },
    onSuccess: async (raw) => {
      const message = str(raw.message, "").trim();
      const output = str(raw.output, "").trim();
      const mode = str(raw.mode, "").trim().toLowerCase();
      if (message && output) {
        setSuccess(`${message}\n\n${output}`);
      } else if (message) {
        setSuccess(message);
      } else {
        setSuccess("ArkPulse fix completed.");
      }
      const baselineEventId = latestPulseEventId;
      setSelectedPulseEvent(null);
      if (!pulseRunning) {
        setPulsePollState({
          baselineEventId,
          deadlineAt: Date.now() + 2 * 60 * 1000,
        });
        if (mode !== "app_restart") {
          try {
            await api.rawPost("/arkpulse/trigger", {});
          } catch (e) {
            setPulsePollState(null);
            setError(`Fix ran, but ArkPulse refresh failed: ${errMessage(e)}`);
          }
        }
      }
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
  const trustEvaluateMutation = useMutation({
    mutationFn: (payload: { action_kind: string; payload: unknown }) =>
      api.rawPost("/autonomy/trust/evaluate", payload),
  });
  const selectedTrustPreset =
    TRUST_APPROVAL_PRESETS.find((item) => item.id === trustPresetId) ??
    TRUST_APPROVAL_PRESETS[0];

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
      "ArkEvolve readiness thresholds saved.",
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
        ? "ArkSentinel turned off. Its signal switches will stay off after autonomy resumes until you turn them back on."
        : "ArkSentinel turned off. Its signal switches are off until you turn them back on.",
    );
    if (changed) {
      setSentinelDisableDialogOpen(false);
    }
  }

  async function submitSentinelInAppDisableDialog() {
    const changed = await updateSettingsSentinel(
      { watch_in_app: false },
      "ArkSentinel will ignore in-app AgentArk activity until you turn it back on.",
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
  const selectedSettingsHeaderTitle =
    selectedSettingsMeta.title || selectedSettingsNav?.label || "Settings";
  const arkPulseHeader = (
    <WorkspacePageHeader
      eyebrow="Ark Core"
      title="ArkPulse"
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
      {standaloneArkPulse ? arkPulseHeader : null}
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
                    ? "Stored ArkPulse history could not be loaded in this runtime."
                    : "No ArkPulse events yet."}
                </Typography>
                {renderSettingsInlineCard({
                  eyebrow: "ArkPulse",
                  title: "How this helps",
                  description:
                    "ArkPulse runs a health check for setup, integrations, safety, and runtime drift.",
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
                        Example: if notifications stop arriving, ArkPulse can
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
        setSuccess(str(out.message, "ArkPulse is already running."));
      } else {
        setSuccess(str(out.message, "ArkPulse check started."));
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

  const renderSearchProviderCredentialField = (
    provider: (typeof SEARCH_API_PROVIDER_OPTIONS)[number],
  ) => {
    const configured = toBool(settings[provider.configuredField]);
    const editing = Boolean(form[provider.editingField]);
    const clearPending = Boolean(form[provider.clearField]);
    const pendingValue = str(form[provider.keyField], "");
    const status = clearPending
      ? "delete pending"
      : configured
        ? "configured"
        : "not configured";
    const disabled = configured && !editing && !clearPending;
    const helperText = clearPending
      ? `The saved ${provider.label} key will be removed when you save settings.`
      : configured && !editing
        ? `${provider.label} already has a saved key. Use Edit to replace it or Delete to remove it.`
        : configured
          ? pendingValue.trim()
            ? `Save settings to replace the current ${provider.label} API key.`
            : `Leave blank to keep the current ${provider.label} API key, or enter a replacement.`
          : `Enter a ${provider.label} API key, then save settings.`;

    return (
      <Stack key={provider.id} spacing={0.75}>
        <TextField
          label={`${provider.label} API Key (${status})`}
          value={disabled ? "Saved key on file" : pendingValue}
          onChange={(e) =>
            setSearchProviderDraft(provider, {
              key: e.target.value,
              editing: true,
              clear: false,
            })
          }
          fullWidth
          size="small"
          type={disabled ? "text" : "password"}
          disabled={disabled}
          placeholder={
            configured && !editing
              ? "Saved key on file"
              : `Enter ${provider.label} API key`
          }
          helperText={helperText}
        />
        {configured ? (
          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{
              flexWrap: "wrap",
            }}
          >
            {!editing && !clearPending ? (
              <Button
                size="small"
                variant="outlined"
                onClick={() =>
                  setSearchProviderDraft(provider, {
                    key: "",
                    editing: true,
                    clear: false,
                  })
                }
              >
                Edit key
              </Button>
            ) : null}
            {editing ? (
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
                Cancel edit
              </Button>
            ) : null}
            {!clearPending ? (
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
                Delete integration
              </Button>
            ) : (
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
            )}
          </Stack>
        ) : null}
      </Stack>
    );
  };

  return (
    <Stack spacing={2}>
      {standaloneArkPulse ? (
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
          <SettingsNavigation tab={tab} onTabChange={setTab} />
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
                              ? "var(--ui-rgba-130-247-193-200)"
                              : s.tone === "warning"
                                ? "var(--ui-rgba-255-180-50-240)"
                                : "var(--ui-rgba-255-255-255-080)",
                          background:
                            s.tone === "success"
                              ? "var(--ui-rgba-130-247-193-060)"
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
                                ? "#82f7c1"
                                : s.tone === "warning"
                                  ? "var(--ui-rgba-255-180-50-900)"
                                  : "var(--ui-rgba-255-255-255-180)",
                            boxShadow:
                              s.tone === "success"
                                ? "0 0 6px var(--ui-rgba-130-247-193-320)"
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
                          borderColor: "var(--ui-rgba-130-247-193-220)",
                          color: "var(--ui-rgba-130-247-193-880)",
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
                    <Autocomplete
                      freeSolo
                      options={[
                        "UTC",
                        "America/New_York",
                        "America/Chicago",
                        "America/Denver",
                        "America/Los_Angeles",
                        "America/Phoenix",
                        "America/Toronto",
                        "America/Vancouver",
                        "Europe/London",
                        "Europe/Paris",
                        "Europe/Berlin",
                        "Asia/Dubai",
                        "Asia/Kolkata",
                        "Asia/Singapore",
                        "Asia/Tokyo",
                        "Australia/Sydney",
                      ]}
                      value={form.timezone || ""}
                      onChange={(_, v) => setField("timezone", String(v ?? ""))}
                      inputValue={form.timezone || ""}
                      onInputChange={(_, v) => setField("timezone", v)}
                      renderInput={(params) => (
                        <TextField
                          {...params}
                          label="Timezone"
                          placeholder="e.g. America/New_York"
                          fullWidth
                          size="small"
                        />
                      )}
                    />
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
              <Stack
                spacing={1.5}
                data-tour-target="settings-models"
                sx={{ minHeight: 0 }}
              >
                <Box sx={{ minHeight: 0 }}>
                  <Stack spacing={1.5}>
                    <Tabs
                      value={modelsSectionTab}
                      onChange={(_, value) =>
                        setModelsSectionTab(value as "pool" | "embeddings")
                      }
                      variant="scrollable"
                      scrollButtons="auto"
                      sx={{
                        minHeight: 0,
                        "& .MuiTabs-indicator": {
                          height: 2,
                        },
                      }}
                    >
                      <Tab value="pool" label="Model Pool" />
                      <Tab value="embeddings" label="Embeddings" />
                    </Tabs>

                    {modelsSectionTab === "pool" ? (
                      <Box sx={{ minHeight: 0 }}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Models",
                          title: "Model Pool",
                          description:
                            "Configure the models AgentArk uses for primary, fast, code, research, and fallback work.",
                          action: (
                            <Button
                              size="small"
                              variant="contained"
                              onClick={openAddModel}
                            >
                              Add Model
                            </Button>
                          ),
                        })}

                        <Stack
                          direction="row"
                          spacing={2}
                          sx={{
                            alignItems: "center",
                            mb: 1,
                          }}
                        >
                          <FormControlLabel
                            control={
                              <Switch
                                checked={form.smart_routing}
                                onChange={(e) =>
                                  setField("smart_routing", e.target.checked)
                                }
                              />
                            }
                            label="Smart Routing"
                          />
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            When off, the agent uses the primary model for
                            everything.
                          </Typography>
                        </Stack>

                        {modelsQ.isLoading && modelSlots.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading models...
                          </Typography>
                        ) : modelsRefreshIssue && modelSlots.length === 0 ? (
                          <Alert severity="warning">
                            Could not refresh model list right now. Please retry
                            in a moment.
                          </Alert>
                        ) : modelSlots.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            No models configured. Add a model to complete setup.
                          </Typography>
                        ) : (
                          <Stack spacing={1}>
                            {showingModelFallback ? (
                              <Alert severity="info">
                                Showing last known model list while refresh is
                                in progress.
                              </Alert>
                            ) : null}
                            <TableContainer className="table-shell settings-models-table-shell">
                              <Table size="small">
                                <TableHead>
                                  <TableRow>
                                    <TableCell>Label</TableCell>
                                    <TableCell>Role</TableCell>
                                    <TableCell>Provider</TableCell>
                                    <TableCell>Model</TableCell>
                                    <TableCell>Enabled</TableCell>
                                    <TableCell>API Key</TableCell>
                                    <TableCell align="right">Ops</TableCell>
                                  </TableRow>
                                </TableHead>
                                <TableBody>
                                  {modelSlots.map((slot) => {
                                    const id = str(slot.id, "");
                                    const enabled = toBool(slot.enabled);
                                    return (
                                      <TableRow key={id}>
                                        <TableCell>
                                          {str(slot.label, "-")}
                                        </TableCell>
                                        <TableCell>
                                          {str(slot.role, "-")}
                                        </TableCell>
                                        <TableCell>
                                          {str(slot.provider, "-")}
                                        </TableCell>
                                        <TableCell
                                          sx={{ wordBreak: "break-word" }}
                                        >
                                          {str(slot.model, "-")}
                                        </TableCell>
                                        <TableCell>
                                          {enabled ? "yes" : "no"}
                                        </TableCell>
                                        <TableCell>
                                          {toBool(slot.has_api_key)
                                            ? "configured"
                                            : "-"}
                                        </TableCell>
                                        <TableCell align="right">
                                          <RowOpsMenu
                                            actions={[
                                              {
                                                label: "Edit",
                                                onClick: () =>
                                                  openEditModel(slot),
                                              },
                                              {
                                                label: enabled
                                                  ? "Disable"
                                                  : "Enable",
                                                disabled:
                                                  toggleModelEnabledMutation.isPending,
                                                onClick: async () => {
                                                  setError(null);
                                                  try {
                                                    await toggleModelEnabledMutation.mutateAsync(
                                                      slot,
                                                    );
                                                  } catch (e) {
                                                    setError(errMessage(e));
                                                  }
                                                },
                                              },
                                              {
                                                label: "Delete",
                                                tone: "error",
                                                divider: true,
                                                disabled:
                                                  deleteModelMutation.isPending,
                                                onClick: async () => {
                                                  const ok = window.confirm(
                                                    "Delete this model slot?",
                                                  );
                                                  if (!ok) return;
                                                  setError(null);
                                                  try {
                                                    await deleteModelMutation.mutateAsync(
                                                      slot,
                                                    );
                                                  } catch (e) {
                                                    setError(errMessage(e));
                                                  }
                                                },
                                              },
                                            ]}
                                            ariaLabel="Model options"
                                          />
                                        </TableCell>
                                      </TableRow>
                                    );
                                  })}
                                </TableBody>
                              </Table>
                            </TableContainer>
                          </Stack>
                        )}
                      </Box>
                    ) : (
                      <Stack spacing={1.5}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Models",
                          title: "Embeddings",
                          description:
                            "Choose the backend used for local memory, retrieval, and document embeddings.",
                        })}

                        <Stack
                          direction="row"
                          spacing={1}
                          useFlexGap
                          sx={{
                            flexWrap: "wrap",
                          }}
                        >
                          <Chip
                            size="small"
                            variant="outlined"
                            label={
                              embeddingsProvider === "local-hf"
                                ? "Local Hugging Face"
                                : embeddingsProvider === "ollama"
                                  ? "External Ollama"
                                  : embeddingsDisabled
                                    ? "Disabled"
                                    : "External Provider"
                            }
                          />
                          <Chip
                            size="small"
                            variant="outlined"
                            label={
                              form.embeddings_model.trim() ||
                              "No model selected"
                            }
                          />
                          {embeddingsProvider === "openai-compatible" ? (
                            <Chip
                              size="small"
                              variant="outlined"
                              color={
                                form.embeddings_api_key.trim() ||
                                embeddingsHasApiKey
                                  ? "success"
                                  : "default"
                              }
                              label={
                                form.embeddings_api_key.trim() ||
                                embeddingsHasApiKey
                                  ? "API key configured"
                                  : "No API key"
                              }
                            />
                          ) : null}
                        </Stack>

                        {embeddingsStatus ? (
                          <Alert
                            severity={
                              /failed|unavailable|error/i.test(embeddingsStatus)
                                ? "error"
                                : /ready/i.test(embeddingsStatus)
                                  ? "success"
                                  : /download|initializ|configured|reachable/i.test(
                                        embeddingsStatus,
                                      )
                                    ? "info"
                                    : "warning"
                            }
                          >
                            {embeddingsStatus}
                          </Alert>
                        ) : null}

                        <Grid2 container spacing={1.5}>
                          <Grid2 size={{ xs: 12, md: 4 }}>
                            <TextField
                              label="Provider"
                              select
                              value={form.embeddings_provider}
                              onChange={(e) =>
                                setField("embeddings_provider", e.target.value)
                              }
                              fullWidth
                              size="small"
                            >
                              <MenuItem value="disabled">
                                Disabled
                              </MenuItem>
                              <MenuItem value="local-hf">
                                Local Hugging Face
                              </MenuItem>
                              <MenuItem value="openai-compatible">
                                External OpenAI-compatible
                              </MenuItem>
                              <MenuItem value="ollama">
                                External Ollama
                              </MenuItem>
                            </TextField>
                          </Grid2>
                          {!embeddingsIsLocal && !embeddingsDisabled ? (
                            <Grid2 size={{ xs: 12, md: 8 }}>
                              <TextField
                                label="Embedding Model"
                                value={form.embeddings_model}
                                onChange={(e) =>
                                  setField("embeddings_model", e.target.value)
                                }
                                fullWidth
                                size="small"
                                placeholder={
                                  embeddingsIsOllama
                                    ? "nomic-embed-text"
                                    : "text-embedding-3-small"
                                }
                                helperText={
                                  embeddingsIsOllama
                                    ? "Example: nomic-embed-text"
                                    : "Use the model name exposed by your external /embeddings provider."
                                }
                              />
                            </Grid2>
                          ) : null}

                          {embeddingsIsExternal ? (
                            <Grid2 size={{ xs: 12, md: 8 }}>
                              <TextField
                                label={
                                  embeddingsIsOllama
                                    ? "Base URL"
                                    : "Base URL (optional)"
                                }
                                value={form.embeddings_base_url}
                                onChange={(e) =>
                                  setField(
                                    "embeddings_base_url",
                                    e.target.value,
                                  )
                                }
                                fullWidth
                                size="small"
                                placeholder={
                                  embeddingsIsOllama
                                    ? "http://host.docker.internal:11434"
                                    : "https://api.openai.com/v1"
                                }
                                helperText={
                                  embeddingsIsOllama
                                    ? "Point this at a user-managed Ollama server. AgentArk does not bundle Ollama."
                                    : "Leave blank to use the provider default. Set this when using another OpenAI-compatible embeddings endpoint."
                                }
                              />
                            </Grid2>
                          ) : null}

                          {embeddingsProvider === "openai-compatible" ? (
                            <Grid2 size={{ xs: 12, md: 4 }}>
                              <TextField
                                label="API Key (optional)"
                                value={form.embeddings_api_key}
                                onChange={(e) =>
                                  setField("embeddings_api_key", e.target.value)
                                }
                                fullWidth
                                size="small"
                                type="password"
                                helperText={
                                  embeddingsHasApiKey
                                    ? "Leave blank to keep the current key."
                                    : "Only required if your external provider needs authentication."
                                }
                              />
                            </Grid2>
                          ) : null}
                        </Grid2>

                        {embeddingsIsLocal ? (
                          <Alert
                            severity="info"
                            icon={<InfoOutlinedIcon fontSize="inherit" />}
                          >
                            Local embeddings use the built-in default model{" "}
                            {LOCAL_EMBEDDINGS_MODEL} and initialize only when
                            dense retrieval is used. The model runs in the
                            AgentArk embeddings sidecar, isolated from the
                            main chat server, and no Ollama service is required.
                          </Alert>
                        ) : null}
                        {embeddingsDisabled ? (
                          <Alert
                            severity="info"
                            icon={<InfoOutlinedIcon fontSize="inherit" />}
                          >
                            Dense embeddings are disabled. Memory, document,
                            and action retrieval use lexical fallback until a
                            dense provider is enabled.
                          </Alert>
                        ) : null}
                      </Stack>
                    )}
                  </Stack>
                </Box>

                <Dialog
                  open={modelDialogOpen}
                  onClose={() => setModelDialogOpen(false)}
                  fullWidth
                  maxWidth="sm"
                >
                  <DialogTitle>
                    {modelEditingId ? "Edit Model" : "Add Model"}
                  </DialogTitle>
                  <DialogContent>
                    <Stack spacing={1.5} sx={{ mt: 1 }}>
                      <TextField
                        label="Label"
                        value={modelForm.label}
                        onChange={(e) =>
                          setModelForm((p) => ({ ...p, label: e.target.value }))
                        }
                        fullWidth
                      />
                      <TextField
                        label="Role"
                        select
                        value={modelForm.role}
                        onChange={(e) =>
                          setModelForm((p) => ({ ...p, role: e.target.value }))
                        }
                        fullWidth
                      >
                        <MenuItem value="primary">primary</MenuItem>
                        <MenuItem value="fast">fast</MenuItem>
                        <MenuItem value="code">code</MenuItem>
                        <MenuItem value="research">research</MenuItem>
                        <MenuItem value="fallback">fallback</MenuItem>
                      </TextField>
                      <TextField
                        label="Provider"
                        select
                        value={modelForm.provider}
                        onChange={(e) =>
                          setModelForm((p) => ({
                            ...p,
                            provider: e.target.value,
                          }))
                        }
                        fullWidth
                      >
                        <MenuItem value="">Select provider</MenuItem>
                        {modelForm.provider === "openai-subscription" ? (
                          <MenuItem
                            value="openai-subscription"
                            sx={{ display: "none" }}
                          >
                            openai-subscription
                          </MenuItem>
                        ) : null}
                        {MODEL_PROVIDER_OPTIONS.map((provider) => (
                          <MenuItem key={provider.value} value={provider.value}>
                            {provider.label}
                          </MenuItem>
                        ))}
                      </TextField>
                      <Autocomplete
                        freeSolo
                        options={modelOptions}
                        loading={discoverModelsQ.isFetching}
                        value={modelForm.model}
                        onChange={(_, v) =>
                          setModelForm((p) => ({
                            ...p,
                            model: String(v ?? ""),
                          }))
                        }
                        inputValue={modelForm.model}
                        onInputChange={(_, v) =>
                          setModelForm((p) => ({ ...p, model: v }))
                        }
                        renderOption={(props, option) => {
                          const name = modelOptionNames.get(option);
                          return (
                            <li {...props}>
                              <ListItemText
                                primary={name || option}
                                secondary={
                                  name && name !== option ? option : undefined
                                }
                              />
                            </li>
                          );
                        }}
                        renderInput={(params) => (
                          <TextField
                            {...params}
                            label="Model"
                            fullWidth
                            placeholder={
                              modelForm.provider === "openai-subscription"
                                ? "Choose or enter OpenAI model id"
                                : "Choose or enter model id"
                            }
                            helperText={
                              modelForm.provider === "openai-compatible" &&
                              !modelForm.base_url.trim()
                                ? "Set a Base URL in Advanced to auto-discover models, or type a model ID manually."
                                : discoverModelsQ.isFetching
                                  ? "Loading provider models. You can still type any model ID."
                                  : "You can type any model ID even if it is not listed."
                            }
                          />
                        )}
                      />
                      {modelForm.provider === "openai-subscription" ? (
                        <Stack spacing={1}>
                          <Alert severity="info">
                            Connect your OpenAI subscription with browser OAuth.
                            You can reconnect any time, especially if auth
                            expires.
                            <br />
                            <br />
                            <strong>First time?</strong> Enable device code auth
                            in your OpenAI account: go to{" "}
                            <a
                              href="https://chatgpt.com/settings/security"
                              target="_blank"
                              rel="noopener noreferrer"
                              style={{ color: "inherit" }}
                            >
                              chatgpt.com/settings/security
                            </a>{" "}
                            {"->"} toggle{" "}
                            <strong>"Enable device code authorization"</strong>{" "}
                            on.
                          </Alert>
                          <Stack direction="row" spacing={1}>
                            <Button
                              variant="contained"
                              size="small"
                              onClick={startOpenaiSubscriptionOAuth}
                              disabled={codexAuthBusy}
                            >
                              {codexAuthBusy
                                ? "Starting..."
                                : modelEditingId
                                  ? "Reconnect OAuth"
                                  : "Connect via Browser"}
                            </Button>
                            <Button
                              variant="outlined"
                              size="small"
                              onClick={checkOpenaiSubscriptionOAuthStatus}
                              disabled={codexAuthBusy}
                            >
                              Check Status
                            </Button>
                            <Button
                              variant="text"
                              size="small"
                              onClick={() => {
                                const authUrl = (
                                  openaiSubAuth?.authUrl || ""
                                ).trim();
                                if (!authUrl) return;
                                window.open(
                                  authUrl,
                                  "_blank",
                                  "noopener,noreferrer",
                                );
                              }}
                              disabled={
                                codexAuthBusy ||
                                !(openaiSubAuth?.authUrl || "").trim()
                              }
                            >
                              Open URL
                            </Button>
                          </Stack>
                          {(openaiSubAuth?.deviceCode || "").trim() ? (
                            <Stack
                              direction="row"
                              spacing={0.8}
                              sx={{
                                alignItems: "center",
                                minWidth: 0,
                              }}
                            >
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Device code:
                              </Typography>
                              <Typography
                                variant="caption"
                                component="code"
                                sx={{
                                  px: 0.8,
                                  py: 0.2,
                                  borderRadius: 1,
                                  bgcolor: "var(--ui-rgba-0-0-0-220)",
                                  fontFamily:
                                    "ui-monospace, SFMono-Regular, Menlo, monospace",
                                }}
                              >
                                {(openaiSubAuth?.deviceCode || "").trim()}
                              </Typography>
                              <IconButton
                                size="small"
                                onClick={async () => {
                                  try {
                                    await navigator.clipboard.writeText(
                                      (openaiSubAuth?.deviceCode || "").trim(),
                                    );
                                    setSuccess("Device code copied.");
                                  } catch {
                                    setError("Could not copy device code.");
                                  }
                                }}
                                aria-label="Copy device code"
                              >
                                <ContentCopyRoundedIcon fontSize="inherit" />
                              </IconButton>
                            </Stack>
                          ) : null}
                          {(openaiSubAuth?.authUrl || "").trim() ? (
                            <Stack
                              direction="row"
                              spacing={0.8}
                              sx={{
                                alignItems: "center",
                                minWidth: 0,
                              }}
                            >
                              <Link
                                href={(openaiSubAuth?.authUrl || "").trim()}
                                target="_blank"
                                rel="noopener noreferrer"
                                underline="hover"
                                sx={{
                                  fontSize: "0.75rem",
                                  wordBreak: "break-all",
                                  flex: 1,
                                  minWidth: 0,
                                }}
                              >
                                {(openaiSubAuth?.authUrl || "").trim()}
                              </Link>
                              <IconButton
                                size="small"
                                onClick={async () => {
                                  try {
                                    await navigator.clipboard.writeText(
                                      (openaiSubAuth?.authUrl || "").trim(),
                                    );
                                    setSuccess("OAuth URL copied.");
                                  } catch {
                                    setError("Could not copy URL.");
                                  }
                                }}
                                aria-label="Copy OAuth URL"
                              >
                                <ContentCopyRoundedIcon fontSize="inherit" />
                              </IconButton>
                            </Stack>
                          ) : null}
                          {openaiSubAuth &&
                          !openaiSubAuth.openedBrowser &&
                          (openaiSubAuth.authUrl || "").trim() ? (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "warning.main",
                              }}
                            >
                              Browser did not open automatically. Click "Open
                              URL" above to complete sign-in.
                            </Typography>
                          ) : null}
                          {openaiSubAuth?.running ? (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "info.main",
                              }}
                            >
                              Login is in progress. Finish auth in
                              browser/device flow, then click Check Status.
                            </Typography>
                          ) : null}
                          {openaiSubAuth?.message ? (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {openaiSubAuth.message}
                            </Typography>
                          ) : null}
                        </Stack>
                      ) : (
                        <Stack spacing={1}>
                          <TextField
                            label="API Key (optional)"
                            value={modelForm.api_key}
                            onChange={(e) => {
                              const nextValue = e.target.value;
                              setModelClearApiKey(false);
                              setModelForm((p) => ({
                                ...p,
                                api_key: nextValue,
                              }));
                            }}
                            fullWidth
                            type="password"
                            helperText={
                              modelEditingId
                                ? modelCanReuseExistingKey
                                  ? "Leave blank to keep the current key."
                                  : "Provider or base URL changed. Blank will not reuse the old key."
                                : undefined
                            }
                          />
                          {showClearSavedKeyAction ? (
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                alignItems: "center",
                                flexWrap: "wrap",
                              }}
                            >
                              <Chip
                                size="small"
                                color={
                                  modelClearSavedKeyPending
                                    ? "warning"
                                    : "success"
                                }
                                variant="outlined"
                                label={
                                  modelClearSavedKeyPending
                                    ? "Saved key will be removed"
                                    : "Saved key on file"
                                }
                              />
                              <Button
                                size="small"
                                variant="outlined"
                                color={
                                  modelClearSavedKeyPending
                                    ? "inherit"
                                    : "warning"
                                }
                                onClick={() => {
                                  setModelForm((p) => ({ ...p, api_key: "" }));
                                  setModelClearApiKey((prev) => !prev);
                                }}
                              >
                                {modelClearSavedKeyPending
                                  ? "Keep saved key"
                                  : "Clear saved key"}
                              </Button>
                            </Stack>
                          ) : null}
                          {modelNeedsReplacementKeyWarning ? (
                            <Alert
                              severity="warning"
                              icon={<InfoOutlinedIcon fontSize="inherit" />}
                            >
                              This edit changes the provider or base URL. The
                              previously saved key for this slot will not be
                              reused. Add a replacement key before saving, or
                              the slot will be saved without one.
                            </Alert>
                          ) : null}
                          {modelClearSavedKeyPending ? (
                            <Alert
                              severity="warning"
                              icon={<InfoOutlinedIcon fontSize="inherit" />}
                            >
                              The saved key for this slot will be removed when
                              you save. Runs may fail until you add a new key.
                            </Alert>
                          ) : null}
                        </Stack>
                      )}
                      <Accordion
                        expanded={modelAdvancedOpen}
                        onChange={(_, expanded) =>
                          setModelAdvancedOpen(expanded)
                        }
                        disableGutters
                      >
                        <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                          <Typography variant="body2">Advanced</Typography>
                        </AccordionSummary>
                        <AccordionDetails>
                          {[
                            "ollama",
                            "openrouter",
                            "openai-compatible",
                            "huggingface",
                          ].includes(modelForm.provider) ? (
                            <TextField
                              label={
                                modelForm.provider === "openai-compatible"
                                  ? "Base URL"
                                  : "Base URL (optional)"
                              }
                              value={modelForm.base_url}
                              onChange={(e) =>
                                setModelForm((p) => ({
                                  ...p,
                                  base_url: e.target.value,
                                }))
                              }
                              fullWidth
                              helperText={
                                modelForm.provider === "openrouter"
                                  ? `Example: ${OPENROUTER_DEFAULT_BASE_URL}`
                                  : modelForm.provider === "ollama"
                                    ? `Example: ${OLLAMA_DEFAULT_BASE_URL}`
                                    : modelForm.provider === "huggingface"
                                      ? "Default: https://api-inference.huggingface.co/v1 - use your HF token as the API key"
                                      : "Required for OpenAI-compatible providers."
                              }
                            />
                          ) : (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              No advanced provider settings for this model.
                            </Typography>
                          )}
                        </AccordionDetails>
                      </Accordion>
                      <FormControlLabel
                        control={
                          <Switch
                            checked={modelForm.enabled}
                            onChange={(e) =>
                              setModelForm((p) => ({
                                ...p,
                                enabled: e.target.checked,
                              }))
                            }
                          />
                        }
                        label="Enabled"
                      />
                      {modelConnectionTestResult ? (
                        <Alert
                          severity={
                            modelConnectionTestResult.ok ? "success" : "warning"
                          }
                        >
                          {modelConnectionTestResult.message}
                        </Alert>
                      ) : null}
                      {modelTestConnectionHint ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {modelTestConnectionHint}
                        </Typography>
                      ) : null}
                      <Stack
                        direction="row"
                        spacing={1}
                        sx={{
                          justifyContent: "flex-end",
                        }}
                      >
                        <Button onClick={() => setModelDialogOpen(false)}>
                          Cancel
                        </Button>
                        <Button
                          variant="outlined"
                          onClick={async () => {
                            setError(null);
                            setModelConnectionTestResult(null);
                            try {
                              await testModelConnectionMutation.mutateAsync();
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={
                            !canTestModelConnection ||
                            saveModelMutation.isPending ||
                            testModelConnectionMutation.isPending
                          }
                        >
                          {testModelConnectionMutation.isPending
                            ? "Testing..."
                            : "Test Connection"}
                        </Button>
                        <Button
                          variant="contained"
                          onClick={async () => {
                            setError(null);
                            setModelConnectivityWarning(null);
                            setModelConnectionTestResult(null);
                            try {
                              await saveModelMutation.mutateAsync();
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={
                            saveModelMutation.isPending ||
                            testModelConnectionMutation.isPending
                          }
                        >
                          {saveModelMutation.isPending ? "Saving..." : "Save"}
                        </Button>
                      </Stack>
                    </Stack>
                  </DialogContent>
                </Dialog>
              </Stack>
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
                      <TextField
                        label="SearXNG Base URL (self-hosted)"
                        value={form.search_searxng_base_url}
                        onChange={(e) =>
                          setField("search_searxng_base_url", e.target.value)
                        }
                        fullWidth
                        size="small"
                        placeholder="https://search.example.com"
                        helperText="Optional - requires your own SearXNG instance. AgentArk will call /search?format=json against this URL."
                      />
                    </Stack>
                  </Box>
                </Grid2>
              </Grid2>
            ) : null}

            {tab === 4 ? (
              <Grid2 container spacing={1.5}>
                <Grid2 size={{ xs: 12, lg: 12 }}>
                  <Box
                    sx={{
                      display: "grid",
                      gap: 2,
                      alignItems: "start",
                      gridTemplateColumns: {
                        xs: "minmax(0, 1fr)",
                        lg: "minmax(0, 1.45fr) minmax(320px, 1fr)",
                      },
                      gridTemplateAreas: {
                        xs: showInternalServiceSection
                          ? '"security" "abuse" "internal" "remote" "vault" "privacy"'
                          : '"security" "abuse" "remote" "vault" "privacy"',
                        lg: showInternalServiceSection
                          ? '"security internal" "abuse abuse" "remote vault" "privacy privacy"'
                          : '"security vault" "abuse abuse" "remote vault" "privacy privacy"',
                      },
                    }}
                  >
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "security" }}
                    >
                      <Stack spacing={1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Security & Master Password",
                          description:
                            "Protect operator access, control remote sign-in, and manage the primary instance password.",
                        })}
                        {securityStatusQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading security status...
                          </Typography>
                        ) : securityStatusQ.error ? (
                          <Alert severity="error">
                            {errMessage(securityStatusQ.error)}
                          </Alert>
                        ) : hasCustomMasterPassword ? (
                          <Stack spacing={1.1}>
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={1}
                            >
                              <Button
                                variant="contained"
                                size="large"
                                onClick={() => openPasswordDialog("change")}
                                disabled={passwordMutationPending}
                              >
                                Change Password
                              </Button>
                              <Button
                                color="error"
                                variant="outlined"
                                size="large"
                                onClick={() => openPasswordDialog("remove")}
                                disabled={passwordMutationPending}
                              >
                                Remove Password
                              </Button>
                            </Stack>
                          </Stack>
                        ) : null}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "abuse" }}
                    >
                      <Stack spacing={1.1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Abuse Review",
                          description:
                            "Resume or pause sources that repeatedly tripped the inbound semantic guard.",
                          action: (
                            <Chip
                              size="small"
                              color={abuseReviews.length > 0 ? "warning" : "success"}
                              variant="outlined"
                              label={`${abuseReviews.length} waiting`}
                            />
                          ),
                        })}
                        {abuseReviewsQ.isLoading ? (
                          <Typography variant="body2" sx={{ color: "text.secondary" }}>
                            Loading review queue...
                          </Typography>
                        ) : abuseReviewsQ.error ? (
                          <Alert severity="error">{errMessage(abuseReviewsQ.error)}</Alert>
                        ) : abuseReviews.length === 0 ? (
                          <Alert severity="success" sx={{ py: 0.25 }}>
                            No sources are paused or waiting for review.
                          </Alert>
                        ) : (
                          <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
                            <Table size="small" sx={{ tableLayout: "fixed", width: "100%" }}>
                              <TableHead>
                                <TableRow>
                                  <TableCell sx={{ width: "22%" }}>Status</TableCell>
                                  <TableCell sx={{ width: "24%" }}>Source</TableCell>
                                  <TableCell sx={{ width: "16%" }}>Trips</TableCell>
                                  <TableCell sx={{ width: "18%" }}>Updated</TableCell>
                                  <TableCell sx={{ width: "20%" }} align="right">
                                    Decision
                                  </TableCell>
                                </TableRow>
                              </TableHead>
                              <TableBody>
                                {abuseReviews.map((row, index) => {
                                  const sourceKeyHash = str(row.source_key_hash, "");
                                  const status = str(row.status, "");
                                  const source = str(row.channel_id, "channel");
                                  const identity = str(row.user_identity, "").trim();
                                  const updatedAt = humanTs(str(row.last_updated, ""));
                                  const pending = decideAbuseReviewMutation.isPending;
                                  return (
                                    <TableRow key={sourceKeyHash || `abuse-review-${index}`}>
                                      <TableCell>
                                        <Chip
                                          size="small"
                                          color={status === "paused" ? "error" : "warning"}
                                          variant="outlined"
                                          label={status === "paused" ? "Paused" : "Pending review"}
                                        />
                                      </TableCell>
                                      <TableCell sx={{ overflow: "hidden" }}>
                                        <Typography variant="body2" noWrap title={identity ? `${source} / ${identity}` : source}>
                                          {identity ? `${source} / ${identity}` : source}
                                        </Typography>
                                        <Typography variant="caption" sx={{ color: "text.secondary" }} noWrap title={sourceKeyHash}>
                                          {sourceKeyHash.slice(0, 12)}
                                        </Typography>
                                      </TableCell>
                                      <TableCell>{num(row.trip_count, 0)}</TableCell>
                                      <TableCell>
                                        <Typography variant="caption" title={updatedAt.tip}>
                                          {updatedAt.label}
                                        </Typography>
                                      </TableCell>
                                      <TableCell align="right">
                                        <Stack direction="row" spacing={0.75} sx={{ justifyContent: "flex-end" }}>
                                          <Button
                                            size="small"
                                            variant="outlined"
                                            disabled={pending || !sourceKeyHash}
                                            onClick={() =>
                                              decideAbuseReviewMutation.mutate({
                                                sourceKeyHash,
                                                decision: "reject",
                                              })
                                            }
                                          >
                                            Pause
                                          </Button>
                                          <Button
                                            size="small"
                                            variant="contained"
                                            disabled={pending || !sourceKeyHash}
                                            onClick={() =>
                                              decideAbuseReviewMutation.mutate({
                                                sourceKeyHash,
                                                decision: "approve",
                                              })
                                            }
                                          >
                                            Resume
                                          </Button>
                                        </Stack>
                                      </TableCell>
                                    </TableRow>
                                  );
                                })}
                              </TableBody>
                            </Table>
                          </TableContainer>
                        )}
                      </Stack>
                    </Box>

                    {showInternalServiceSection ? (
                      <Box
                        className="list-shell"
                        sx={{ minHeight: 0, gridArea: "internal" }}
                      >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Internal Service Credentials",
                          description: internalServiceDescription,
                          action: internalServiceRotationSupported ? (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{ flexWrap: "wrap" }}
                            >
                              <Chip size="small" label="Manual rotation" />
                              <Chip
                                size="small"
                                variant="outlined"
                                label="Restart required"
                              />
                            </Stack>
                          ) : undefined,
                        })}
                        {securityStatusQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            Loading internal credential status...
                          </Typography>
                        ) : securityStatusQ.error ? (
                          <Alert severity="error">
                            {errMessage(securityStatusQ.error)}
                          </Alert>
                        ) : internalServiceTokens.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            Internal executor and workspace credentials are not
                            available on this runtime.
                          </Typography>
                        ) : (
                          <Stack spacing={1.1}>
                            <Stack
                              divider={
                                <Divider
                                  flexItem
                                  sx={{ borderColor: "divider" }}
                                />
                              }
                              spacing={0}
                            >
                              {internalServiceTokens.map((row, index) => {
                                const item = asRecord(row);
                                const updatedAt = humanTs(
                                  str(item.updated_at, ""),
                                );
                                const managedByEnv = toBool(
                                  item.managed_by_env,
                                );
                                const configured = toBool(item.configured);
                                return (
                                  <Stack
                                    key={str(item.id, `token-${index}`)}
                                    direction={{ xs: "column", sm: "row" }}
                                    spacing={1}
                                    sx={{ py: 1 }}
                                  >
                                    <Stack
                                      spacing={0.35}
                                      sx={{ minWidth: 0, flex: 1 }}
                                    >
                                      <Typography
                                        variant="body2"
                                        sx={{ fontWeight: 600 }}
                                      >
                                        {str(item.label, "Internal service")}
                                      </Typography>
                                      <Typography
                                        variant="caption"
                                        sx={{ color: "text.secondary" }}
                                      >
                                        {managedByEnv
                                          ? `Managed by ${str(item.env_var, "environment configuration")}`
                                          : "Stored in the AgentArk config volume"}
                                      </Typography>
                                    </Stack>
                                    <Stack
                                      direction="row"
                                      spacing={0.75}
                                      useFlexGap
                                      sx={{
                                        flexWrap: "wrap",
                                        alignItems: "center",
                                      }}
                                    >
                                      <Chip
                                        size="small"
                                        color={
                                          configured ? "success" : "warning"
                                        }
                                        label={
                                          configured ? "Configured" : "Missing"
                                        }
                                      />
                                      <Chip
                                        size="small"
                                        variant="outlined"
                                        label={
                                          managedByEnv
                                            ? "Env managed"
                                            : `Updated ${updatedAt.label}`
                                        }
                                        title={
                                          managedByEnv
                                            ? undefined
                                            : updatedAt.tip
                                        }
                                      />
                                    </Stack>
                                  </Stack>
                                );
                              })}
                            </Stack>
                            {internalServiceRotationSupported ? (
                              <Alert severity="info">
                                Rotation rewrites both credentials together,
                                then restarts control, executor, and workspace
                                immediately. Active work can be interrupted
                                while the stack comes back.
                              </Alert>
                            ) : null}
                            {internalServiceRotationSupported ? (
                              <Stack
                                direction={{ xs: "column", sm: "row" }}
                                spacing={1}
                                useFlexGap
                                sx={{ flexWrap: "wrap" }}
                              >
                                <Button
                                  variant="outlined"
                                  color="warning"
                                  size="large"
                                  disabled={
                                    rotateInternalServiceTokensMutation.isPending ||
                                    !!restartNotice
                                  }
                                  onClick={openRotateInternalCredentialsDialog}
                                >
                                  {rotateInternalServiceTokensMutation.isPending
                                    ? "Rotating..."
                                    : "Rotate Internal Credentials"}
                                </Button>
                              </Stack>
                            ) : null}
                          </Stack>
                        )}
                      </Stack>
                      </Box>
                    ) : null}

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "remote" }}
                    >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Remote Access",
                          description:
                            "Only expose remote sign-in when you need it, and keep the access method and posture visible.",
                          action: (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                              }}
                            >
                              <Chip
                                size="small"
                                color={tunnelSummaryTone}
                                label={tunnelStateLabel}
                              />
                              <Chip
                                size="small"
                                variant="outlined"
                                label={tunnelAccessLabel}
                              />
                            </Stack>
                          ),
                        })}
                        {tunnelQ.isLoading || tunnelProvidersQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading tunnel settings...
                          </Typography>
                        ) : tunnelQ.error || tunnelProvidersQ.error ? (
                          <Alert severity="error">
                            {errMessage(
                              tunnelQ.error || tunnelProvidersQ.error,
                            )}
                          </Alert>
                        ) : (
                          <Stack spacing={1.1}>
                            <Alert
                              severity={tunnelSummaryTone}
                              sx={{
                                py: 0.25,
                                "& .MuiAlert-message": { width: "100%" },
                              }}
                            >
                              <Stack spacing={0.35}>
                                <Typography
                                  variant="body2"
                                  sx={{ fontWeight: 600 }}
                                >
                                  {tunnelPrimaryText}
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "inherit",
                                  }}
                                >
                                  {tunnelPrimaryDetail}
                                </Typography>
                              </Stack>
                            </Alert>
                            <TextField
                              label="Access method"
                              select
                              size="small"
                              fullWidth
                              value={
                                tunnelSelectedProviderId ||
                                serverSelectedTunnelProviderId
                              }
                              onChange={(e) => {
                                const next = e.target.value;
                                syncTunnelDraftFromPayload(
                                  tunnelProvidersPayload,
                                  next,
                                );
                              }}
                            >
                              {tunnelProviderOptions.map((provider) => {
                                const id = str(provider.id, "");
                                const label = str(
                                  provider.label,
                                  id || "Provider",
                                );
                                const available = toBool(provider.available);
                                return (
                                  <MenuItem key={id} value={id}>
                                    {available
                                      ? label
                                      : `${label} (not available)`}
                                  </MenuItem>
                                );
                              })}
                            </TextField>
                            {basicTunnelConfigFields.map((field) => {
                              const key = str(field.key, "");
                              const inputType = str(field.input_type, "text");
                              const options = Array.isArray(field.options)
                                ? field.options.filter(
                                    (value): value is string =>
                                      typeof value === "string",
                                  )
                                : [];
                              const value = tunnelDraftValues[key] ?? "";
                              const storedSecret =
                                inputType === "password" &&
                                selectedTunnelStoredSecretFields.includes(key);
                              const helperText =
                                inputType === "password" &&
                                storedSecret &&
                                !value.trim()
                                  ? "A value is already saved. Enter a new value only if you want to replace it."
                                  : undefined;
                              return (
                                <TextField
                                  key={key}
                                  label={str(field.label, key || "Field")}
                                  value={value}
                                  onChange={(e) =>
                                    setTunnelDraftValues((prev) => ({
                                      ...prev,
                                      [key]: e.target.value,
                                    }))
                                  }
                                  fullWidth
                                  size="small"
                                  required={toBool(field.required)}
                                  placeholder={
                                    str(field.placeholder, "") || undefined
                                  }
                                  type={
                                    inputType === "password"
                                      ? "password"
                                      : "text"
                                  }
                                  multiline={inputType === "textarea"}
                                  minRows={
                                    inputType === "textarea" ? 3 : undefined
                                  }
                                  select={inputType === "select"}
                                  helperText={helperText}
                                >
                                  {inputType === "select"
                                    ? options.map((option) => (
                                        <MenuItem key={option} value={option}>
                                          {option}
                                        </MenuItem>
                                      ))
                                    : null}
                                </TextField>
                              );
                            })}
                            {tunnelPanelNotice ? (
                              <Alert severity={tunnelPanelNotice.severity}>
                                {tunnelPanelNotice.text}
                              </Alert>
                            ) : null}
                            {str(tunnel.error, "").trim() ? (
                              <Alert severity="error">
                                {str(tunnel.error)}
                              </Alert>
                            ) : null}
                            {tunnelSetupChecks.length > 0
                              ? renderSettingsInlineCard({
                                  eyebrow: "Remote access",
                                  title: "Before remote access can start",
                                  description:
                                    "This checklist shows what is still missing, with the exact fix for each step.",
                                  tone: "info",
                                  children: (
                                    <Stack spacing={1}>
                                      {tunnelSetupChecks.map(
                                        (rawCheck, index) => {
                                          const check = asRecord(rawCheck);
                                          const status = str(
                                            check.status,
                                            "info",
                                          );
                                          const detail = str(check.detail, "");
                                          const remediation = str(
                                            check.remediation,
                                            "",
                                          ).trim();
                                          return (
                                            <Alert
                                              key={`${str(check.id, `check-${index}`)}-${index}`}
                                              severity={tunnelCheckAlertSeverity(
                                                status,
                                              )}
                                              sx={{
                                                py: 0.25,
                                                "& .MuiAlert-message": {
                                                  width: "100%",
                                                },
                                              }}
                                            >
                                              <Stack spacing={0.45}>
                                                <Stack
                                                  direction="row"
                                                  spacing={0.75}
                                                  useFlexGap
                                                  sx={{
                                                    alignItems: "center",
                                                    flexWrap: "wrap",
                                                  }}
                                                >
                                                  <Chip
                                                    size="small"
                                                    color={tunnelCheckChipColor(
                                                      status,
                                                    )}
                                                    label={tunnelCheckLabel(
                                                      status,
                                                    )}
                                                  />
                                                  <Typography
                                                    variant="body2"
                                                    sx={{ fontWeight: 600 }}
                                                  >
                                                    {str(
                                                      check.label,
                                                      "Setup step",
                                                    )}
                                                  </Typography>
                                                </Stack>
                                                {detail ? (
                                                  <Typography
                                                    variant="body2"
                                                    sx={{
                                                      color: "inherit",
                                                    }}
                                                  >
                                                    {detail}
                                                  </Typography>
                                                ) : null}
                                                {remediation ? (
                                                  <Typography
                                                    variant="caption"
                                                    sx={{
                                                      color: "inherit",
                                                      opacity: 0.85,
                                                    }}
                                                  >
                                                    {remediation}
                                                  </Typography>
                                                ) : null}
                                              </Stack>
                                            </Alert>
                                          );
                                        },
                                      )}
                                    </Stack>
                                  ),
                                })
                              : null}
                            {str(tunnel.url, "").trim() ? (
                              <TextField
                                label={getTunnelUrlFieldLabel(
                                  selectedTunnelMeta,
                                )}
                                value={str(tunnel.url)}
                                fullWidth
                                size="small"
                                slotProps={{
                                  input: { readOnly: true },
                                }}
                              />
                            ) : null}
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                              }}
                            >
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={handleTunnelProviderSave}
                                disabled={tunnelSaveMutation.isPending}
                              >
                                {tunnelSaveMutation.isPending
                                  ? "Saving..."
                                  : "Save"}
                              </Button>
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={handleTunnelProviderTest}
                                disabled={
                                  tunnelSaveMutation.isPending ||
                                  tunnelTestMutation.isPending
                                }
                              >
                                {tunnelTestMutation.isPending
                                  ? "Checking..."
                                  : "Check setup"}
                              </Button>
                              <Button
                                size="small"
                                variant="contained"
                                onClick={handleTunnelStart}
                                disabled={
                                  tunnelSaveMutation.isPending ||
                                  tunnelStartMutation.isPending ||
                                  toBool(tunnel.active) ||
                                  !selectedTunnelAvailable
                                }
                              >
                                {tunnelStartMutation.isPending
                                  ? "Starting..."
                                  : getTunnelStartButtonLabel(
                                      selectedTunnelMeta,
                                      hasCustomMasterPassword,
                                    )}
                              </Button>
                              <Button
                                size="small"
                                onClick={handleTunnelStop}
                                disabled={
                                  tunnelStopMutation.isPending ||
                                  !toBool(tunnel.active)
                                }
                              >
                                {tunnelStopMutation.isPending
                                  ? "Stopping..."
                                  : getTunnelStopButtonLabel(
                                      selectedTunnelMeta,
                                    )}
                              </Button>
                              <Button
                                size="small"
                                onClick={async () => {
                                  const url = str(tunnel.url, "");
                                  if (!url) return;
                                  await navigator.clipboard.writeText(url);
                                  setSuccess("Tunnel URL copied.");
                                }}
                                disabled={!str(tunnel.url, "").trim()}
                              >
                                Copy link
                              </Button>
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={() => {
                                  const url = str(tunnel.url, "").trim();
                                  if (!url) return;
                                  window.open(
                                    url,
                                    "_blank",
                                    "noopener,noreferrer",
                                  );
                                }}
                                disabled={!str(tunnel.url, "").trim()}
                              >
                                Open link
                              </Button>
                            </Stack>
                            {advancedTunnelConfigFields.length > 0 ? (
                              <Accordion
                                expanded={showTunnelAdvanced}
                                onChange={(_, expanded) =>
                                  setShowTunnelAdvanced(expanded)
                                }
                                disableGutters
                                sx={{
                                  background: "transparent",
                                  boxShadow: "none",
                                  border: "1px solid var(--ui-rgba-62-143-214-180)",
                                  borderRadius: 1,
                                }}
                              >
                                <AccordionSummary
                                  expandIcon={<ExpandMoreIcon />}
                                >
                                  <Typography
                                    variant="body2"
                                    sx={{ fontWeight: 600 }}
                                  >
                                    Advanced configuration
                                  </Typography>
                                </AccordionSummary>
                                <AccordionDetails sx={{ pt: 0 }}>
                                  <Stack spacing={1}>
                                    {advancedTunnelConfigFields.map((field) => {
                                      const key = str(field.key, "");
                                      const inputType = str(
                                        field.input_type,
                                        "text",
                                      );
                                      const value =
                                        tunnelDraftValues[key] ?? "";
                                      return (
                                        <TextField
                                          key={key}
                                          label={str(
                                            field.label,
                                            key || "Field",
                                          )}
                                          value={value}
                                          onChange={(e) =>
                                            setTunnelDraftValues((prev) => ({
                                              ...prev,
                                              [key]: e.target.value,
                                            }))
                                          }
                                          fullWidth
                                          size="small"
                                          required={toBool(field.required)}
                                          placeholder={
                                            str(field.placeholder, "") ||
                                            undefined
                                          }
                                          type={
                                            inputType === "password"
                                              ? "password"
                                              : "text"
                                          }
                                        />
                                      );
                                    })}
                                  </Stack>
                                </AccordionDetails>
                              </Accordion>
                            ) : null}
                            {hasCustomMasterPassword &&
                            getTunnelPanelWarning(selectedTunnelMeta) ? (
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {getTunnelPanelWarning(selectedTunnelMeta)}
                              </Typography>
                            ) : null}
                          </Stack>
                        )}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "vault" }}
                    >
                      <Stack spacing={1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Secrets vault",
                          description: vaultSummaryText,
                          info: "Save private keys and tokens here so AgentArk can use them without showing the raw value in normal screens.",
                          action: (
                            <Chip
                              size="small"
                              variant="outlined"
                              label={`${vaultSecrets.length} saved`}
                            />
                          ),
                        })}
                        {hasCustomMasterPassword ? (
                          <TextField
                            label="Master password for protected edits"
                            value={vaultPassword}
                            onChange={(e) => setVaultPassword(e.target.value)}
                            fullWidth
                            size="small"
                            type="password"
                          />
                        ) : (
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Using built-in local encryption. Set a custom
                            password only if you want password-protected sign-in
                            and remote access.
                          </Typography>
                        )}
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          spacing={1}
                        >
                          <Button
                            size="small"
                            onClick={async () => {
                              setError(null);
                              await queryClient.invalidateQueries({
                                queryKey: ["settings-secrets"],
                              });
                            }}
                            disabled={vaultSecretsQ.isLoading}
                          >
                            Refresh
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            onClick={openVaultEditor}
                          >
                            Add Custom Secret
                          </Button>
                        </Stack>

                        {vaultSecretsQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading secrets...
                          </Typography>
                        ) : vaultSecretsQ.error ? (
                          <Alert severity="error">
                            {errMessage(vaultSecretsQ.error)}
                          </Alert>
                        ) : vaultSecrets.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            No encrypted secrets stored yet.
                          </Typography>
                        ) : (
                          <TableContainer
                            className="table-shell"
                            sx={{ width: "100%", overflowX: "auto" }}
                          >
                            <Table
                              size="small"
                              sx={{ tableLayout: "fixed", width: "100%" }}
                            >
                              <TableHead>
                                <TableRow>
                                  <TableCell sx={{ width: "35%" }}>
                                    Key
                                  </TableCell>
                                  <TableCell sx={{ width: "20%" }}>
                                    Source
                                  </TableCell>
                                  <TableCell sx={{ width: "25%" }}>
                                    Value
                                  </TableCell>
                                  <TableCell
                                    sx={{ width: "20%" }}
                                    align="right"
                                  >
                                    Ops
                                  </TableCell>
                                </TableRow>
                              </TableHead>
                              <TableBody>
                                {vaultSecrets.map((row, idx) => {
                                  const key = str(row.key, "");
                                  const storageKey = str(row.storage_key, key);
                                  const displayKey = str(row.key, storageKey);
                                  const shownValue = str(row.masked, "");
                                  const source = str(row.source, "custom");
                                  const sourceLabel = str(row.source_label, "")
                                    .trim();
                                  const deletable = toBool(row.deletable);
                                  return (
                                    <TableRow key={`${storageKey}-${idx}`}>
                                      <TableCell
                                        sx={{
                                          fontFamily:
                                            "ui-monospace, SFMono-Regular, Menlo, monospace",
                                          fontSize: "0.8rem",
                                          overflow: "hidden",
                                          textOverflow: "ellipsis",
                                          whiteSpace: "nowrap",
                                        }}
                                        title={displayKey}
                                      >
                                        {displayKey}
                                      </TableCell>
                                      <TableCell sx={{ whiteSpace: "nowrap" }}>
                                        <Typography
                                          variant="body2"
                                        >
                                          {sourceLabel ||
                                            source.replace(/[-_]+/g, " ")}
                                        </Typography>
                                      </TableCell>
                                      <TableCell sx={{ overflow: "hidden" }}>
                                        <Typography
                                          variant="body2"
                                          title={shownValue}
                                          sx={{
                                            whiteSpace: "nowrap",
                                            overflow: "hidden",
                                            textOverflow: "ellipsis",
                                          }}
                                        >
                                          {shownValue || "-"}
                                        </Typography>
                                      </TableCell>
                                      <TableCell
                                        align="right"
                                        sx={{ whiteSpace: "nowrap" }}
                                      >
                                        <Stack
                                          direction="row"
                                          spacing={0.5}
                                          sx={{
                                            justifyContent: "flex-end",
                                          }}
                                        >
                                          {deletable ? (
                                            <Button
                                              size="small"
                                              color="error"
                                              sx={{
                                                minWidth: 72,
                                                whiteSpace: "nowrap",
                                              }}
                                              onClick={async () => {
                                                const ok = window.confirm(
                                                  `Delete secret '${displayKey}'?`,
                                                );
                                                if (!ok) return;
                                                const pw =
                                                  resolveVaultPasswordForSensitiveOps();
                                                if (pw === null) return;
                                                setError(null);
                                                try {
                                                  await deleteVaultSecretMutation.mutateAsync(
                                                    {
                                                      key: storageKey,
                                                      password: pw || undefined,
                                                    },
                                                  );
                                                } catch {
                                                  // handled by mutation onError
                                                }
                                              }}
                                              disabled={
                                                deleteVaultSecretMutation.isPending
                                              }
                                            >
                                              Delete
                                            </Button>
                                          ) : (
                                            <Typography
                                              variant="caption"
                                              sx={{
                                                color: "text.secondary",
                                              }}
                                            >
                                              Managed elsewhere
                                            </Typography>
                                          )}
                                        </Stack>
                                      </TableCell>
                                    </TableRow>
                                  );
                                })}
                              </TableBody>
                            </Table>
                          </TableContainer>
                        )}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "privacy" }}
                    >
                      <Stack spacing={1.5}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Model Privacy Boundary",
                          description:
                            "Control what sensitive content can enter model prompts, and whether chat can pause for one-time approval when read-only tools uncover person-linked data.",
                          action: (
                            <Chip
                              size="small"
                              color={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                                  ? "warning"
                                  : "default"
                              }
                              variant="outlined"
                              label={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                                  ? "Approval cards on"
                                  : "Approval cards off"
                              }
                            />
                          ),
                        })}
                        <Alert severity="info" sx={{ py: 0.25 }}>
                          Secrets are still never sent to the model. When
                          approval cards are enabled, chat pauses and shows
                          Approve/Reject buttons before the model can inspect
                          sensitive read-only tool results for a single request.
                        </Alert>
                        <Grid2 container spacing={1.5}>
                          <Grid2 size={{ xs: 12, md: 6 }}>
                            <TextField
                              fullWidth
                              select
                              size="small"
                              label="Retrieved context handling"
                              value={form.model_privacy_default_mode}
                              onChange={(e) =>
                                setField(
                                  "model_privacy_default_mode",
                                  e.target.value,
                                )
                              }
                              helperText="Applies to history, memories, tool output, documents, and helper-model prompts."
                            >
                              <MenuItem value="default_redact">
                                Default redact
                              </MenuItem>
                              <MenuItem value="zero_exposure">
                                Zero exposure
                              </MenuItem>
                              <MenuItem value="secrets_only">
                                Secrets only
                              </MenuItem>
                            </TextField>
                          </Grid2>
                          <Grid2 size={{ xs: 12, md: 6 }}>
                            <TextField
                              fullWidth
                              select
                              size="small"
                              label="Current chat handling"
                              value={form.model_privacy_current_chat_pii_policy}
                              onChange={(e) =>
                                setField(
                                  "model_privacy_current_chat_pii_policy",
                                  e.target.value,
                                )
                              }
                              helperText="Choose whether the active user message stays raw, gets masked, or is blocked when sensitive."
                            >
                              <MenuItem value="raw_current_turn">
                                Raw current turn
                              </MenuItem>
                              <MenuItem value="mask_chat_pii">
                                Mask chat PII
                              </MenuItem>
                              <MenuItem value="block_sensitive_chat">
                                Block sensitive chat
                              </MenuItem>
                            </TextField>
                          </Grid2>
                        </Grid2>
                        <FormControlLabel
                          control={
                            <Switch
                              checked={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                              }
                              onChange={(e) =>
                                setField(
                                  "model_privacy_request_scoped_sensitive_approval_enabled",
                                  e.target.checked,
                                )
                              }
                            />
                          }
                          label="Show approve/reject cards for sensitive read-only tool results"
                        />
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          Approvals are request-scoped only. Reject keeps the
                          data masked. Approve reveals non-secret sensitive
                          context for that single follow-up turn.
                        </Typography>
                      </Stack>
                    </Box>
                  </Box>
                </Grid2>

              </Grid2>
            ) : null}

            {tab === 5 ? (
              <Stack spacing={2.5}>
                {restartNotice
                  ? renderSettingsInlineCard({
                      eyebrow: "Restarting",
                      title: "AgentArk is coming back online",
                      description: restartNotice.text,
                      tone: "info",
                      action: (
                        <Chip
                          size="small"
                          icon={<AutorenewRoundedIcon />}
                          label={restartNotice.etaLabel}
                          color="info"
                          variant="outlined"
                        />
                      ),
                    })
                  : null}
                {/* -- Warning banner -- */}
                {renderSettingsInlineCard({
                  eyebrow: "Advanced",
                  title: "Use with care",
                  description:
                    "These controls can affect stability, security, or how the product behaves. Change them only if you understand the effect.",
                  fullWidthCopy: true,
                  tone: "warning",
                })}

                {/* -- System Controls group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <SettingsRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">
                        System Controls
                      </div>
                      <div className="adv-group-header-sub">
                        Core runtime and interface options.
                      </div>
                    </div>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Autonomy Pause
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Pause autonomous background work from this settings
                        surface. Scheduled reminders still fire.
                      </Typography>
                      {settingsAutonomyQ.error ? (
                        <Alert severity="error" sx={{ mt: 0.75 }}>
                          {errMessage(settingsAutonomyQ.error)}
                        </Alert>
                      ) : settingsAutonomyPaused ? (
                        <Alert severity="warning" sx={{ mt: 0.75 }}>
                          Autonomy is paused. ArkPulse, watchers, background
                          learning, suggestion scans, and proactive
                          optimizations stay paused until you resume it.
                        </Alert>
                      ) : null}
                    </Stack>
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{
                        alignItems: { xs: "stretch", sm: "center" },
                        flexShrink: 0,
                      }}
                    >
                      <Chip
                        size="small"
                        color={settingsAutonomyPaused ? "warning" : "success"}
                        label={settingsAutonomyModeLabel}
                      />
                      <Button
                        size="small"
                        color={settingsAutonomyPaused ? "success" : "warning"}
                        variant="outlined"
                        onClick={() => {
                          if (settingsAutonomyPaused) {
                            void handleResumeAutonomy();
                            return;
                          }
                          setError(null);
                          setSuccess(null);
                          setAutonomyPauseDialogOpen(true);
                        }}
                        disabled={
                          settingsAutonomyQ.isLoading ||
                          !!settingsAutonomyQ.error ||
                          settingsAutonomyMutation.isPending
                        }
                        sx={{ whiteSpace: "nowrap" }}
                      >
                        {settingsAutonomyMutation.isPending
                          ? settingsAutonomyPaused
                            ? "Resuming..."
                            : "Pausing..."
                          : settingsAutonomyPaused
                            ? "Resume autonomy"
                            : "Pause autonomy"}
                      </Button>
                    </Stack>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Restart AgentArk
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Restarts AgentArk to apply runtime and security changes.
                      </Typography>
                    </Stack>
                    <Button
                      size="small"
                      color="warning"
                      variant="outlined"
                      onClick={openRestartDialog}
                      disabled={restartMutation.isPending || !!restartNotice}
                      sx={{ whiteSpace: "nowrap" }}
                    >
                      Restart AgentArk
                    </Button>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Developer Mode
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Enables raw SKILL.md editing after you save. Keep off for
                        beginner-friendly forms.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={developerModeEnabled}
                          onChange={(e) => {
                            const next = e.target.checked;
                            setDeveloperModeEnabledState(next);
                            setError(null);
                            setSuccess(null);
                          }}
                        />
                      }
                      label={developerModeEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Guided Tour
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Re-run the onboarding walkthrough to review core
                        features.
                      </Typography>
                    </Stack>
                    <Button
                      size="small"
                      variant="outlined"
                      onClick={() => {
                        try {
                          window.localStorage.setItem(
                            "agentark.tour.completed",
                            "0",
                          );
                        } catch {}
                        const { startTour } = useUiStore.getState();
                        startTour();
                      }}
                      sx={{ whiteSpace: "nowrap" }}
                    >
                      Restart Tour
                    </Button>
                  </div>
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <StarRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">ArkSentinel</div>
                      <div className="adv-group-header-sub">
                        These switches decide where ArkSentinel learns from and
                        what kinds of follow-up it can suggest.
                      </div>
                    </div>
                  </div>
                  {settingsSentinelQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsSentinelQ.error)}
                    </Alert>
                  ) : null}
                  {settingsAutonomyPaused ? (
                    <Alert severity="warning" sx={{ mb: 1.5 }}>
                      ArkSentinel preferences stay saved here, but follow-up
                      scanning is paused until autonomy is active again.
                    </Alert>
                  ) : null}
                  {ADVANCED_SENTINEL_SIGNAL_OPTIONS.map((item) => {
                    const isMainSentinelSwitch = item.key === "enabled";
                    const storedEnabled =
                      settingsSentinel[item.key] == null
                        ? true
                        : toBool(settingsSentinel[item.key]);
                    const checked = isMainSentinelSwitch
                      ? storedEnabled
                      : settingsSentinelEnabled && storedEnabled;
                    return (
                      <div className="adv-row" key={item.key}>
                        <Stack spacing={0.2}>
                          <Typography
                            variant="body2"
                            sx={{ fontWeight: 600 }}
                          >
                            {item.label}
                          </Typography>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {item.description}
                          </Typography>
                        </Stack>
                        <FormControlLabel
                          control={
                            <Switch
                              checked={checked}
                              onChange={(event) => {
                                if (
                                  item.key === "enabled" &&
                                  !event.target.checked
                                ) {
                                  setError(null);
                                  setSuccess(null);
                                  setSentinelDisableDialogOpen(true);
                                  return;
                                }
                                if (
                                  item.key === "watch_in_app" &&
                                  !event.target.checked
                                ) {
                                  setError(null);
                                  setSuccess(null);
                                  setSentinelInAppDisableDialogOpen(true);
                                  return;
                                }
                                const nextChecked = event.target.checked;
                                const payload: JsonRecord =
                                  item.key === "enabled" && nextChecked
                                    ? {
                                        enabled: true,
                                        watch_in_app: true,
                                        watch_connected_services: true,
                                        infer_new_automations: true,
                                      }
                                    : ({
                                        [item.key]: nextChecked,
                                      } as JsonRecord);
                                void updateSettingsSentinel(
                                  payload,
                                  item.key === "enabled" && nextChecked
                                    ? "ArkSentinel and all signal switches are on."
                                    : nextChecked
                                      ? item.enabledMessage
                                      : item.disabledMessage,
                                );
                              }}
                              disabled={
                                settingsSentinelQ.isLoading ||
                                !!settingsSentinelQ.error ||
                                settingsSentinelMutation.isPending ||
                                (!isMainSentinelSwitch &&
                                  !settingsSentinelEnabled)
                              }
                            />
                          }
                          label={checked ? "On" : "Off"}
                          sx={{ mr: 0 }}
                        />
                      </div>
                    );
                  })}
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <AutorenewRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">ArkEvolve</div>
                      <div className="adv-group-header-sub">
                        Controls whether AgentArk learns from completed work and
                        tests reviewed improvements in the background.
                      </div>
                    </div>
                  </div>
                  {settingsEvolutionQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsEvolutionQ.error)}
                    </Alert>
                  ) : null}
                  {settingsAutonomyPaused ? (
                    <Alert severity="warning" sx={{ mb: 1.5 }}>
                      Self-evolve can stay on, but its background passes will
                      remain paused until autonomy is active again.
                    </Alert>
                  ) : null}
                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Self-evolve
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Controls heuristic reflection, consolidation, candidate
                        generation, and active canary experiments.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={settingsSelfEvolveEnabled}
                          onChange={(event) => {
                            if (event.target.checked) {
                              void handleEnableSelfEvolve();
                              return;
                            }
                            setError(null);
                            setSuccess(null);
                            setSelfEvolveDisableDialogOpen(true);
                          }}
                          disabled={
                            settingsEvolutionQ.isLoading ||
                            !!settingsEvolutionQ.error ||
                            settingsEvolutionMutation.isPending
                          }
                        />
                      }
                      label={settingsSelfEvolveEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>
                  <Accordion disableGutters sx={{ mt: 1 }}>
                    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                      <Stack spacing={0.2}>
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>
                          Readiness gates
                        </Typography>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          Human review and auto-run stay separate. Auto-run
                          requires stronger repeated evidence.
                        </Typography>
                      </Stack>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Alert severity="info" sx={{ borderRadius: 1, mb: 1.25 }}>
                        For normal use, leave these defaults alone. Lower values
                        make ArkEvolve suggest changes sooner; higher values make
                        it wait for more proof.
                      </Alert>
                      <Grid2 container spacing={1.25}>
                        {[
                          ["min_review_samples", "Review samples", "Runs needed before a suggestion can be approved"],
                          ["min_auto_samples", "Auto-run samples", "Runs needed before automatic use is even considered"],
                          ["min_review_success_rate_pct", "Review success %", "Minimum success rate for review"],
                          ["min_auto_success_rate_pct", "Auto-run success %", "Minimum success rate for auto-run"],
                          ["max_review_correction_rate_pct", "Review correction %", "Maximum correction rate for review"],
                          ["max_auto_correction_rate_pct", "Auto-run correction %", "Maximum correction rate for auto-run"],
                          ["min_candidate_review_confidence_pct", "Candidate confidence %", "Minimum confidence before review"],
                          ["max_review_trust_score", "Review risk score", "Highest trust risk allowed for review"],
                          ["max_auto_trust_score", "Auto-run risk score", "Highest trust risk allowed for auto-run"],
                        ].map(([key, label, helper]) => (
                          <Grid2 key={key} size={{ xs: 12, sm: 6, lg: 4 }}>
                            <TextField
                              fullWidth
                              size="small"
                              type="number"
                              label={label}
                              value={readinessPolicyDraft[key] ?? ""}
                              onChange={(event) =>
                                setReadinessPolicyDraft((draft) => ({
                                  ...draft,
                                  [key]: event.target.value,
                                }))
                              }
                              helperText={helper}
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                !!settingsEvolutionQ.error ||
                                settingsEvolutionMutation.isPending
                              }
                            />
                          </Grid2>
                        ))}
                        <Grid2 size={{ xs: 12 }}>
                          <Stack
                            direction={{ xs: "column", sm: "row" }}
                            spacing={1}
                            sx={{ justifyContent: "flex-end" }}
                          >
                            <Button
                              size="small"
                              color="inherit"
                              onClick={() =>
                                setReadinessPolicyDraft(
                                  readinessPolicyToDraft(settingsReadinessPolicy),
                                )
                              }
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                settingsEvolutionMutation.isPending
                              }
                            >
                              Reset
                            </Button>
                            <Button
                              size="small"
                              variant="contained"
                              onClick={() => void submitReadinessPolicyDraft()}
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                !!settingsEvolutionQ.error ||
                                settingsEvolutionMutation.isPending
                              }
                            >
                              Save gates
                            </Button>
                          </Stack>
                        </Grid2>
                      </Grid2>
                    </AccordionDetails>
                  </Accordion>
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <CheckCircleRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">
                        App Deploy Defaults
                      </div>
                      <div className="adv-group-header-sub">
                        Deployment defaults that should stay separate from
                        ArkSentinel and ArkEvolve.
                      </div>
                    </div>
                  </div>
                  {settingsEvolutionQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsEvolutionQ.error)}
                    </Alert>
                  ) : null}
                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Default app access guard
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        New app deploy and public-link flows start with the
                        access guard on unless a request explicitly overrides
                        it.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={settingsDefaultGuardEnabled}
                          onChange={(event) =>
                            void updateSettingsEvolution(
                              {
                                deploy_guard_default: event.target.checked,
                              },
                              event.target.checked
                                ? "New app deploys will start with the access guard on by default."
                                : "New app deploys will leave the access guard off by default.",
                            )
                          }
                          disabled={
                            settingsEvolutionQ.isLoading ||
                            !!settingsEvolutionQ.error ||
                            settingsEvolutionMutation.isPending
                          }
                        />
                      }
                      label={settingsDefaultGuardEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>
                </Box>

                {/* -- Permissions group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <span style={{ fontSize: 15 }}>&#128274;</span>
                    </div>
                    <div>
                      <div className="adv-group-header-title">Permissions</div>
                      <div className="adv-group-header-sub">
                        Action approval and auto-approve settings.
                      </div>
                    </div>
                  </div>

                  {/* Auto-Approve Skills */}
                  <Typography variant="body2" sx={{ fontWeight: 600, mb: 0.5 }}>
                    Auto-Approve Skills
                  </Typography>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      display: "block",
                      mb: 1.5,
                    }}
                  >
                    Select action-name overrides that can run without a separate
                    approval prompt. Dangerous actions stay approval-gated even
                    if typed manually.
                  </Typography>
                  {(() => {
                    const blockedEntries = findBlockedAutoApproveEntries(
                      form.auto_approve_csv,
                    );
                    const set = new Set(
                      sanitizeAutoApproveList(
                        parseCsvList(form.auto_approve_csv),
                      ),
                    );
                    const update = (name: string, checked: boolean) => {
                      const next = new Set(set);
                      if (checked) next.add(name);
                      else next.delete(name);
                      setField(
                        "auto_approve_csv",
                        sanitizeAutoApproveList(Array.from(next).sort()).join(
                          ", ",
                        ),
                      );
                    };
                    return (
                      <>
                        {blockedEntries.length > 0 ? (
                          <Alert severity="warning" sx={{ mb: 1.5 }}>
                            These actions always require approval and will be
                            ignored here:{" "}
                            {blockedEntries
                              .map((name) => `\`${name}\``)
                              .join(", ")}
                            .
                          </Alert>
                        ) : null}
                        <Grid2 container spacing={1}>
                          {AUTO_APPROVE_ACTION_OPTIONS.map((name) => {
                            const active = set.has(name);
                            return (
                              <Grid2 key={name} size={{ xs: 6, md: 4, lg: 3 }}>
                                <div
                                  className={`adv-skill-pill${active ? " active" : ""}`}
                                  onClick={() => update(name, !active)}
                                >
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      fontFamily: "'JetBrains Mono', monospace",
                                      fontSize: "0.7rem",
                                      letterSpacing: 0,
                                    }}
                                  >
                                    {name}
                                  </Typography>
                                  <Switch
                                    size="small"
                                    checked={active}
                                    onChange={(e) =>
                                      update(name, e.target.checked)
                                    }
                                  />
                                </div>
                              </Grid2>
                            );
                          })}
                        </Grid2>
                        <TextField
                          label="Custom (CSV)"
                          value={form.auto_approve_csv}
                          onChange={(e) =>
                            setField("auto_approve_csv", e.target.value)
                          }
                          fullWidth
                          size="small"
                          placeholder="comma separated action names"
                          helperText="Always blocked here: shell, file_write, code_execute, lan_discover, gmail_send, and similar sensitive actions."
                          sx={{ mt: 1.5 }}
                        />
                      </>
                    );
                  })()}
                </Box>

                {/* -- API Access group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <span style={{ fontSize: 15 }}>&#128273;</span>
                    </div>
                    <div>
                      <div className="adv-group-header-title">API Access</div>
                      <div className="adv-group-header-sub">
                        HTTP API key management.
                      </div>
                    </div>
                  </div>

                  {apiKeyQ.isLoading ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Loading API key...
                    </Typography>
                  ) : apiKeyQ.error ? (
                    <Alert severity="error">{errMessage(apiKeyQ.error)}</Alert>
                  ) : (
                    <Stack spacing={1.5}>
                      <Stack
                        direction="row"
                        spacing={2}
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap",
                        }}
                      >
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            flex: "1 1 auto",
                          }}
                        >
                          Used as{" "}
                          <code
                            style={{
                              background: "var(--ui-rgba-255-255-255-060)",
                              padding: "1px 5px",
                              borderRadius: 2,
                              fontSize: "0.72rem",
                              color: "var(--ui-rgba-244-245-247-900)",
                            }}
                          >
                            Authorization: Bearer &lt;key&gt;
                          </code>{" "}
                          for all HTTP requests.
                        </Typography>
                        <Chip
                          size="small"
                          color={
                            apiKeyRemainingSeconds > 0 ? "info" : "warning"
                          }
                          label={`Rotates in ${formatDurationClock(apiKeyRemainingSeconds)}`}
                        />
                      </Stack>
                      {apiKeyRotated ? (
                        <Chip
                          size="small"
                          color="success"
                          label="API key rotated automatically"
                        />
                      ) : null}
                      <TextField
                        label="Key"
                        value={
                          apiKeyRevealed
                            ? str(apiKeyPayload.key, "")
                            : str(apiKeyPayload.masked, "")
                        }
                        fullWidth
                        size="small"
                        slotProps={{
                          input: {
                            readOnly: true,
                            sx: {
                              fontFamily:
                                "'JetBrains Mono', 'Fira Code', monospace",
                              fontSize: "0.78rem",
                              letterSpacing: 0,
                            },
                          },
                        }}
                      />
                      {apiKeyIssuedAtUnix > 0
                        ? (() => {
                            const { label: issuedLabel, tip: issuedTip } =
                              humanTs(
                                new Date(
                                  apiKeyIssuedAtUnix * 1000,
                                ).toISOString(),
                              );
                            return (
                              <Tooltip title={issuedTip} placement="top">
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                    cursor: "default",
                                  }}
                                >
                                  Issued {issuedLabel}
                                </Typography>
                              </Tooltip>
                            );
                          })()
                        : null}
                      {apiKeyExpiresAtUnix > 0
                        ? (() => {
                            const { label: expiresLabel, tip: expiresTip } =
                              humanTs(
                                new Date(
                                  apiKeyExpiresAtUnix * 1000,
                                ).toISOString(),
                              );
                            return (
                              <Tooltip title={expiresTip} placement="top">
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                    cursor: "default",
                                  }}
                                >
                                  Expires {expiresLabel}
                                </Typography>
                              </Tooltip>
                            );
                          })()
                        : null}
                      <Stack direction="row" spacing={1}>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={() => setApiKeyRevealed((v) => !v)}
                        >
                          {apiKeyRevealed ? "Hide" : "Reveal"}
                        </Button>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={async () => {
                            const key = str(apiKeyPayload.key, "");
                            if (!key) return;
                            await navigator.clipboard.writeText(key);
                            setSuccess("API key copied.");
                          }}
                          disabled={!str(apiKeyPayload.key, "").trim()}
                        >
                          Copy
                        </Button>
                        <Button
                          size="small"
                          color="warning"
                          variant="outlined"
                          onClick={async () => {
                            const ok = window.confirm(
                              "Regenerate API key? Old key will stop working.",
                            );
                            if (!ok) return;
                            setError(null);
                            setSuccess(null);
                            try {
                              await regenerateApiKeyMutation.mutateAsync();
                              setApiKeyRevealed(true);
                              setSuccess("API key regenerated.");
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={regenerateApiKeyMutation.isPending}
                        >
                          Regenerate
                        </Button>
                      </Stack>
                    </Stack>
                  )}
                </Box>
              </Stack>
            ) : null}

            {tab === 14 ? (
              <Stack spacing={2.5}>
                <Alert
                  severity={
                    foreverLifecycleRules.length > 0 ? "warning" : "info"
                  }
                >
                  <Stack spacing={0.35}>
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      Data cleanup is enabled by default, but every cleanup
                      category can be disabled.
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "inherit",
                      }}
                    >
                      {foreverLifecycleRules.length > 0
                        ? `Forever is enabled for ${foreverLifecycleSummary}.`
                        : "Set any retention field below to 0 if you intentionally want to keep that data forever."}{" "}
                      Keeping rows forever or far beyond the defaults can
                      increase DB size, slow queries, and make the server feel
                      heavier over time.
                    </Typography>
                  </Stack>
                </Alert>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    <Stack
                      direction="row"
                      spacing={1}
                      useFlexGap
                      sx={{
                        flexWrap: "wrap",
                      }}
                    >
                      <Chip
                        size="small"
                        color={dataCleanupEnabled ? "success" : "default"}
                        label={
                          dataCleanupEnabled
                            ? "Cleanup active"
                            : "Cleanup paused"
                        }
                      />
                      <Chip
                        size="small"
                        variant="outlined"
                        label={
                          form.data_lifecycle_notifications_cleanup_enabled
                            ? "Notifications on"
                            : "Notifications off"
                        }
                      />
                      <Chip
                        size="small"
                        variant="outlined"
                        label={
                          form.data_lifecycle_logs_cleanup_enabled
                            ? "Logs & traces on"
                            : "Logs & traces off"
                        }
                      />
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.data_lifecycle_cleanup_enabled}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable data cleanup"
                    />
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Memory Behavior",
                      description:
                        "Retention controls for durable memory, audit history, staged candidates, and memory checks.",
                    })}
                    <Alert severity="info">
                      ArkMemory is the normal memory surface. These settings
                      only change how long memory records and evidence are
                      retained.
                    </Alert>
                    <Grid2 container spacing={1.5}>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Inactive memory rows (days)"
                          value={
                            form.data_lifecycle_experience_item_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_item_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Active memories are never auto-deleted. 0 keeps inactive rows too."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Memory ledger (days)"
                          value={form.data_lifecycle_recall_event_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_recall_event_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Audit history for memory changes."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Memory checks (days)"
                          value={form.data_lifecycle_recall_test_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_recall_test_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Generated checks for stored memory."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Staged candidates (days)"
                          value={
                            form.data_lifecycle_learning_candidate_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_learning_candidate_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Review queue and rejected candidates."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Experience runs (days)"
                          value={form.data_lifecycle_experience_run_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Session evidence used for learning."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Experience edges (days)"
                          value={
                            form.data_lifecycle_experience_edge_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_edge_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Lineage and supersedes links."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Inactive patterns (days)"
                          value={
                            form.data_lifecycle_procedural_pattern_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_procedural_pattern_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Active and draft learned patterns are never auto-deleted."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Notifications",
                      description:
                        "Set retention and cleanup cadence for in-product notifications stored by the system.",
                    })}
                    <FormControlLabel
                      control={
                        <Switch
                          checked={
                            form.data_lifecycle_notifications_cleanup_enabled
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notifications_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable notification cleanup"
                    />
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: notificationsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: notificationsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Retention (days)"
                          value={
                            form.data_lifecycle_notifications_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notifications_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps notifications forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Cleanup cadence (seconds)"
                          value={
                            form.data_lifecycle_notification_cleanup_interval_secs
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notification_cleanup_interval_secs",
                              e.target.value,
                            )
                          }
                          helperText="How often stale notifications are purged."
                          slotProps={{
                            htmlInput: { min: 300, step: 60 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Logs & Traces",
                      description:
                        "Retention windows for operational data. Use 0 only when you intentionally want to keep a category forever.",
                    })}
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.data_lifecycle_logs_cleanup_enabled}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_logs_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable logs, traces, task, and message cleanup"
                    />
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: logsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: logsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution traces (days)"
                          value={
                            form.data_lifecycle_execution_trace_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_trace_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all traces."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution runs (days)"
                          value={
                            form.data_lifecycle_execution_run_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all execution runs, checkpoints, and tool attempts."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Background sessions (days)"
                          value={
                            form.data_lifecycle_background_session_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_background_session_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps closed background sessions forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Browser sessions (days)"
                          value={
                            form.data_lifecycle_browser_session_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_browser_session_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps completed and failed browser sessions forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Automation runs (days)"
                          value={
                            form.data_lifecycle_automation_run_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_automation_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps automation history forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution proofs (days)"
                          value={
                            form.data_lifecycle_execution_proof_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_proof_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all proofs."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Operational logs (days)"
                          value={
                            form.data_lifecycle_operational_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_operational_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all operational logs."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security logs (days)"
                          value={
                            form.data_lifecycle_security_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Used by both housekeeping and idle cleanup."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Approval logs (days)"
                          value={
                            form.data_lifecycle_approval_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_approval_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all approval history."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Delegations (days)"
                          value={
                            form.data_lifecycle_swarm_delegation_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_swarm_delegation_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all delegation records."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="LLM usage (days)"
                          value={form.data_lifecycle_llm_usage_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_llm_usage_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all token/accounting usage."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Completed tasks (days)"
                          value={
                            form.data_lifecycle_terminal_task_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_terminal_task_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Recurring cron tasks are never purged."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Conversations (days)"
                          value={form.data_lifecycle_message_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_message_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all messages and conversation history."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Cleanup Cadence",
                      description:
                        "Configure how often housekeeping runs and when idle security cleanup is allowed to start.",
                    })}
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: logsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: logsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Housekeeping cadence (seconds)"
                          value={form.data_lifecycle_housekeeping_interval_secs}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_housekeeping_interval_secs",
                              e.target.value,
                            )
                          }
                          helperText="Used for trace, log, task, and message cleanup passes."
                          slotProps={{
                            htmlInput: { min: 300, step: 60 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security cleanup cadence (days)"
                          value={
                            form.data_lifecycle_security_cleanup_interval_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_cleanup_interval_days",
                              e.target.value,
                            )
                          }
                          helperText="How often the idle security-log cleanup may run."
                          slotProps={{
                            htmlInput: { min: 1, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security idle threshold (seconds)"
                          value={
                            form.data_lifecycle_security_cleanup_idle_threshold_secs
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_cleanup_idle_threshold_secs",
                              e.target.value,
                            )
                          }
                          helperText="Server must stay idle this long before the security sweep runs."
                          slotProps={{
                            htmlInput: { min: 60, step: 60 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>
              </Stack>
            ) : null}

            {tab === 25 ? (
              <Stack spacing={2.5}>
                {restartNotice
                  ? renderSettingsInlineCard({
                      eyebrow: "Restarting",
                      title: "AgentArk is coming back online",
                      description: restartNotice.text,
                      tone: "info",
                      action: (
                        <Chip
                          size="small"
                          icon={<AutorenewRoundedIcon />}
                          label={restartNotice.etaLabel}
                          color="info"
                          variant="outlined"
                        />
                      ),
                    })
                  : null}

                {renderSettingsInlineCard({
                  eyebrow: "Updates",
                  title: "Managed release updates",
                  description:
                    "Updating restarts AgentArk. Pending chats, running jobs, and in-flight approvals can be interrupted. Stored data and conversation history remain on this machine.",
                  tone: "warning",
                  fullWidthCopy: true,
                })}

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Updates",
                      title: "Release status",
                      description:
                        "Track the installed version and the latest tagged release without polling GitHub on every page refresh.",
                      action:
                        updateStatus?.state === "available" &&
                        updateStatus.apply_supported ? (
                          <Button
                            size="small"
                            variant="contained"
                            color="warning"
                            disabled={
                              updateAgentArkMutation.isPending ||
                              !!restartNotice
                            }
                            onClick={async () => {
                              const ok = window.confirm(
                                "Update AgentArk and restart now? Pending chats, running jobs, and in-flight approvals can be interrupted.",
                              );
                              if (!ok) return;
                              setError(null);
                              setSuccess(null);
                              setRestartNotice(null);
                              try {
                                await updateAgentArkMutation.mutateAsync();
                                void monitorRestartRecovery(
                                  UPDATE_NOTICE_DURATION_MS,
                                );
                              } catch (e) {
                                setError(errMessage(e));
                              }
                            }}
                          >
                            {updateAgentArkMutation.isPending
                              ? "Starting..."
                              : "Update and Restart"}
                          </Button>
                        ) : null,
                    })}

                    {updateStatusQ.isLoading && !updateStatus ? (
                      <Alert severity="info">
                        Checking the latest release.
                      </Alert>
                    ) : updateStatusQ.error ? (
                      <Alert severity="error">
                        {errMessage(updateStatusQ.error)}
                      </Alert>
                    ) : null}

                    <Grid2 container spacing={1.5}>
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          label="Installed version"
                          value={str(updateStatusQ.data?.version, "Unknown")}
                          disabled
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          label="Latest release"
                          value={str(
                            updateStatus?.latest_version,
                            "Unavailable",
                          )}
                          disabled
                        />
                      </Grid2>
                    </Grid2>

                    {updateStatus?.checked_at ? (
                      <Typography
                        variant="caption"
                        sx={{ color: "text.secondary" }}
                      >
                        Last checked {updateCheckedAtLabel}
                      </Typography>
                    ) : null}

                    {updateStatus?.release_url ? (
                      <Link
                        href={updateStatus.release_url}
                        target="_blank"
                        rel="noreferrer"
                        underline="hover"
                      >
                        Open release notes
                      </Link>
                    ) : null}

                    {(() => {
                      if (!updateStatus) {
                        return null;
                      }
                      if (updateStatus.state === "available") {
                        return (
                          <Alert severity="warning">
                            A newer tagged release is available.
                            {updateStatus.apply_supported
                              ? " Start the update here when you are ready for a restart."
                              : ` ${updateStatus.apply_message || "Update this deployment from the CLI instead."}`}
                          </Alert>
                        );
                      }
                      if (updateStatus.state === "current") {
                        return (
                          <Alert severity="success">
                            This installation is already on the latest tagged
                            release.
                          </Alert>
                        );
                      }
                      if (updateStatus.state === "unavailable") {
                        return (
                          <Alert severity="info">
                            Update status is unavailable for this deployment.
                            This is expected while release metadata is private
                            or temporarily unreachable.
                          </Alert>
                        );
                      }
                      return (
                        <Alert severity="info">
                          Checking release metadata.
                        </Alert>
                      );
                    })()}
                  </Stack>
                </Box>
              </Stack>
            ) : null}

            {tab === 6 ? (
              <Stack spacing={2.5}>
                <WorkspaceLazyPanel message="Loading observability...">
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
                </WorkspaceLazyPanel>
              </Stack>
            ) : null}

            {tab === 20 ? (
              <Box className="list-shell">
                <WorkspaceLazyPanel message="Loading channels integrations...">
                  <IntegrationsPanel
                    autoRefresh={autoRefresh}
                    embedded
                    mode="channels"
                  />
                </WorkspaceLazyPanel>
              </Box>
            ) : null}

            {tab === 21 ? (
              <Box className="list-shell">
                <WorkspaceLazyPanel message="Loading integrations...">
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
                  <WorkspaceLazyPanel message="Loading webhooks...">
                    <WebhooksPanel autoRefresh={autoRefresh} />
                  </WorkspaceLazyPanel>
                </Box>
                <Box className="list-shell">
                  <WorkspaceLazyPanel message="Loading custom API setup...">
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
                <WorkspaceLazyPanel message="Loading plugin SDK...">
                  <PluginSdkPanel autoRefresh={autoRefresh} embedded />
                </WorkspaceLazyPanel>
              </Box>
            ) : null}

            {tab === 26 ? (
              <WorkspaceLazyPanel message="Loading companion devices...">
                <CompanionDevicesPanel autoRefresh={autoRefresh} />
              </WorkspaceLazyPanel>
            ) : null}

            {tab === 11 ? (
              <WorkspaceLazyPanel message="Loading trace...">
                <TracePage autoRefresh={autoRefresh} />
              </WorkspaceLazyPanel>
            ) : null}

            {tab === 8 ? (
              <Box className="list-shell">
                <WorkspaceLazyPanel message="Loading MCP integrations...">
                  <IntegrationsPanel
                    autoRefresh={autoRefresh}
                    embedded
                    mode="mcp"
                  />
                </WorkspaceLazyPanel>
              </Box>
            ) : null}

            {tab === 12 ? (
              <WorkspaceLazyPanel message="Loading memory...">
                <MemoryPage
                  autoRefresh={autoRefresh}
                  projects={[]}
                  activeProjectId=""
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
                              ? "Stored ArkPulse history could not be loaded in this runtime."
                              : "No ArkPulse events yet."}
                          </Typography>
                          {renderSettingsInlineCard({
                            eyebrow: "ArkPulse",
                            title: "How this helps",
                            description:
                              "ArkPulse runs a health check for setup, integrations, safety, and runtime drift.",
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
                                  ArkPulse can point you to the broken setup
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
                ArkPulse Run
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
                {selectedPulseFindings.slice(0, 20).map((f, idx) => {
                  const fr = asRecord(f);
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
                              disabled={
                                !canRunFix || runPulseFixMutation.isPending
                              }
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
                                    findingIndex: idx,
                                  });
                                  setPulseFixResultsById((prev) => ({
                                    ...prev,
                                    [fixActionId]: {
                                      severity: "success",
                                      message:
                                        str(
                                          result.message,
                                          "ArkPulse diagnostic completed.",
                                        ).trim() ||
                                        "ArkPulse diagnostic completed.",
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
                This shows exactly what ArkPulse scanned, how long each phase
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
      {!standaloneArkPulse && (settingsQ.isLoading || mediaQ.isLoading) ? (
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
          }}
        >
          Loading settings...
        </Typography>
      ) : null}
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
              2. Watchers, external polling triggers, and ArkPulse health checks
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
        <DialogTitle>Turn off ArkSentinel?</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2} sx={{ mt: 0.5 }}>
            <Alert severity="warning">
              This stops ArkSentinel follow-up scanning in the background.
            </Alert>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              While ArkSentinel is off:
            </Typography>
            <Typography variant="body2">
              1. New follow-up suggestions from in-app and connected-app activity
              stop.
            </Typography>
            <Typography variant="body2">
              2. Routine-detection proposals stop appearing.
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              3. Existing preferences stay saved, but ArkSentinel stays off until
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
              : "Turn off ArkSentinel"}
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
              ArkSentinel will stop surfacing follow-ups from in-app chat and
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
      {settingsQ.error || mediaQ.error || modelsQ.error ? (
        <Alert severity="error">
          {errMessage(settingsQ.error || mediaQ.error || modelsQ.error)}
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
