import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  Grid2,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography
} from "@mui/material";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { IntegrationConfigField, IntegrationItem } from "../types";

const REFRESH_MS = 8000;

type JsonRecord = Record<string, unknown>;

type McpTransportType = "http" | "stdio";
type McpAuthType = "none" | "bearer" | "basic" | "header" | "query";

type McpServerForm = {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  resources_enabled: boolean;
  transport_type: McpTransportType;
  url: string;
  command: string;
  args_csv: string;
  working_dir: string;
  auth_type: McpAuthType;
  auth_header: string;
  auth_name: string;
  auth_token: string;
  auth_username: string;
  auth_password: string;
  auth_clear: boolean;
  tool_allowlist_csv: string;
  resource_allowlist_csv: string;
  timeout_secs: string;
  max_response_bytes: string;
};

type ChannelSettingsForm = {
  search_primary: string;
  search_fallback1: string;
  search_fallback2: string;
  search_serper_key: string;
  search_searxng_url: string;
  search_brave_key: string;
  telegram_enabled: boolean;
  telegram_bot_token: string;
  telegram_allowed_users_csv: string;
  whatsapp_enabled: boolean;
  whatsapp_mode: "baileys" | "cloud_api";
  whatsapp_access_token: string;
  whatsapp_phone_number_id: string;
  whatsapp_verify_token: string;
  whatsapp_bridge_url: string;
  whatsapp_dm_policy: string;
  whatsapp_allowed_numbers_csv: string;
};

function asErrorMessage(err: unknown): string {
  if (!(err instanceof Error)) return "Request failed";
  try {
    const parsed = JSON.parse(err.message) as { error?: string };
    if (parsed.error && parsed.error.trim()) return parsed.error;
  } catch {
    // Best effort - return raw error if not JSON.
  }
  return err.message;
}

function statusColor(status: IntegrationItem["status"]): "success" | "warning" | "error" | "default" {
  if (status === "connected") return "success";
  if (status === "needs_auth") return "warning";
  if (status === "error") return "error";
  return "default";
}

function channelStatusColor(status: string): "success" | "warning" | "error" | "info" | "default" {
  const s = (status || "").toLowerCase();
  if (s === "connected" || s === "ready") return "success";
  if (s === "qr" || s === "connecting" || s === "syncing" || s === "checking") return "info";
  if (s === "missing_token" || s === "missing_config" || s === "disabled") return "warning";
  if (s === "error" || s === "failed") return "error";
  return "default";
}

function channelStatusLabel(status: string): string {
  const s = (status || "").toLowerCase();
  if (s === "qr") return "Waiting for QR scan";
  if (s === "connecting") return "Connecting";
  if (s === "syncing") return "Syncing";
  if (s === "connected") return "Connected";
  if (s === "ready") return "Ready";
  if (s === "disabled") return "Disabled";
  if (s === "missing_token") return "Missing token";
  if (s === "missing_config") return "Missing config";
  if (s === "checking") return "Checking";
  if (s === "error") return "Error";
  return status || "Unknown";
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function asRecords(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord);
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const v = value.trim().toLowerCase();
    return v === "true" || v === "1" || v === "yes";
  }
  return false;
}

function extractAuthUrl(payload: unknown): string {
  if (typeof payload === "string") {
    const trimmed = payload.trim();
    return /^https?:\/\//i.test(trimmed) ? trimmed : "";
  }
  const root = asRecord(payload);
  const direct = str(root.auth_url, "").trim();
  if (direct) return direct;
  const url = str(root.url, "").trim();
  if (url) return url;
  const nested = asRecord(root.data);
  const nestedAuthUrl = str(nested.auth_url, "").trim();
  if (nestedAuthUrl) return nestedAuthUrl;
  const nestedUrl = str(nested.url, "").trim();
  return nestedUrl;
}

function parseCsvList(input: string): string[] {
  return (input || "")
    .split(/[,\n]/g)
    .map((s) => s.trim())
    .filter(Boolean);
}

function parseTelegramUsers(input: string): number[] {
  const raw = parseCsvList(input);
  const users: number[] = [];
  for (const item of raw) {
    const n = Number(item);
    if (!Number.isFinite(n)) throw new Error(`Invalid Telegram user ID: '${item}'`);
    users.push(Math.trunc(n));
  }
  return users;
}

function asStringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((v) => str(v, "").trim()).filter(Boolean);
}

function defaultMcpForm(): McpServerForm {
  return {
    id: "",
    name: "",
    description: "",
    enabled: true,
    resources_enabled: false,
    transport_type: "http",
    url: "",
    command: "",
    args_csv: "",
    working_dir: "",
    auth_type: "none",
    auth_header: "Authorization",
    auth_name: "",
    auth_token: "",
    auth_username: "",
    auth_password: "",
    auth_clear: false,
    tool_allowlist_csv: "",
    resource_allowlist_csv: "",
    timeout_secs: "15",
    max_response_bytes: "1048576"
  };
}

function transportSummary(server: JsonRecord): string {
  const transport = asRecord(server.transport);
  const t = str(transport.type, "http");
  if (t === "stdio") {
    return `stdio: ${str(transport.command, "(command)")}`;
  }
  return `http: ${str(transport.url, "(url)")}`;
}

function authSummary(server: JsonRecord): string {
  const auth = asRecord(server.auth);
  const authType = str(auth.auth_type, "none");
  const hasAuth = toBool(auth.has_auth);
  return `${authType}${hasAuth ? " (configured)" : ""}`;
}

function parseSshConnectionNames(value: string): string[] {
  const lines = (value || "").split(/\r?\n/);
  const names: string[] = [];
  for (const line of lines) {
    const m = line.match(/^\s*-\s*([^\s(]+)\s*\(/);
    if (m && m[1]) names.push(m[1].trim());
  }
  return names;
}

type CardMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

function CardActionsMenu({ actions, ariaLabel = "Actions" }: { actions: CardMenuAction[]; ariaLabel?: string }) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton size="small" aria-label={ariaLabel} onClick={(e) => setAnchorEl(e.currentTarget)}>
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu anchorEl={anchorEl} open={open} onClose={closeMenu}>
        {actions.map((action, idx) => (
          <MenuItem
            key={`${action.label}-${idx}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={() => {
              closeMenu();
              if (action.disabled) return;
              void action.onClick();
            }}
            sx={
              action.tone === "error"
                ? { color: "error.main" }
                : action.tone === "warning"
                  ? { color: "warning.main" }
                  : undefined
            }
          >
            {action.label}
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

export function IntegrationsPanel({
  autoRefresh,
  embedded = false,
  mode = "all"
}: {
  autoRefresh: boolean;
  embedded?: boolean;
  mode?: "all" | "integrations" | "mcp";
}) {
  const queryClient = useQueryClient();
  const showIntegrations = mode !== "mcp";
  const showMcp = mode !== "integrations";
  const [active, setActive] = useState<IntegrationItem | null>(null);
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [configSuccess, setConfigSuccess] = useState(false);
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const [mcpEditingId, setMcpEditingId] = useState<string | null>(null);
  const [mcpForm, setMcpForm] = useState<McpServerForm>(defaultMcpForm());
  const [mcpHasStoredAuth, setMcpHasStoredAuth] = useState(false);
  const [mcpError, setMcpError] = useState<string | null>(null);
  const [mcpNotice, setMcpNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [sshNotice, setSshNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [sshKeyDialogOpen, setSshKeyDialogOpen] = useState(false);
  const [sshConnDialogOpen, setSshConnDialogOpen] = useState(false);
  const [sshTestDialogOpen, setSshTestDialogOpen] = useState(false);
  const [sshKeyName, setSshKeyName] = useState("");
  const [sshKeyPem, setSshKeyPem] = useState("");
  const [sshConnName, setSshConnName] = useState("");
  const [sshHost, setSshHost] = useState("");
  const [sshPort, setSshPort] = useState("22");
  const [sshUsername, setSshUsername] = useState("");
  const [sshConnKeyName, setSshConnKeyName] = useState("");
  const [sshTestConnName, setSshTestConnName] = useState("");
  const [sshTestOutput, setSshTestOutput] = useState("");
  const [sshKeyError, setSshKeyError] = useState<string | null>(null);
  const [sshConnError, setSshConnError] = useState<string | null>(null);
  const [showDisabledIntegrations, setShowDisabledIntegrations] = useState(false);
  const [oauthBusyId, setOauthBusyId] = useState<string | null>(null);
  const [channelsDirty, setChannelsDirty] = useState(false);
  const [searchSetupOpen, setSearchSetupOpen] = useState(false);
  const [telegramSetupOpen, setTelegramSetupOpen] = useState(false);
  const [whatsAppSetupOpen, setWhatsAppSetupOpen] = useState(false);
  const [channelForm, setChannelForm] = useState<ChannelSettingsForm>({
    search_primary: "playwright",
    search_fallback1: "duckduckgo",
    search_fallback2: "none",
    search_serper_key: "",
    search_searxng_url: "",
    search_brave_key: "",
    telegram_enabled: false,
    telegram_bot_token: "",
    telegram_allowed_users_csv: "",
    whatsapp_enabled: false,
    whatsapp_mode: "baileys",
    whatsapp_access_token: "",
    whatsapp_phone_number_id: "",
    whatsapp_verify_token: "",
    whatsapp_bridge_url: "",
    whatsapp_dm_policy: "all",
    whatsapp_allowed_numbers_csv: ""
  });
  // NOTE: Integrations are long-lived connectors. URL imports belong to Skills.

  const integrationsQ = useQuery({
    queryKey: ["integrations"],
    queryFn: api.getIntegrations,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showIntegrations
  });
  const settingsQ = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.rawGet("/settings"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showIntegrations
  });
  const waBridgeQ = useQuery({
    queryKey: ["wa-bridge-status", "integrations-panel"],
    queryFn: () => api.rawGet("/api/whatsapp-bridge/status"),
    enabled:
      showIntegrations &&
      channelForm.whatsapp_enabled &&
      channelForm.whatsapp_mode === "baileys",
    refetchInterval:
      autoRefresh &&
      channelForm.whatsapp_enabled &&
      channelForm.whatsapp_mode === "baileys"
        ? 5000
        : false
  });
  const telegramStatusQ = useQuery({
    queryKey: ["telegram-status", "integrations-panel"],
    queryFn: () => api.rawGet("/api/telegram/status"),
    enabled: showIntegrations,
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const mcpQ = useQuery({
    queryKey: ["mcp-servers"],
    queryFn: () => api.rawGet("/mcp/servers?include_details=true"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showMcp
  });
  const sshKeysQ = useQuery({
    queryKey: ["ssh-keys"],
    queryFn: () => api.rawGet("/ssh/keys"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showMcp
  });
  const sshConnectionsQ = useQuery({
    queryKey: ["ssh-connections"],
    queryFn: () => api.rawGet("/ssh/connections"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showMcp
  });

  const configureMutation = useMutation({
    mutationFn: ({ id, payload }: { id: string; payload: Record<string, unknown> }) =>
      api.configureIntegration(id, payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    }
  });

  const disconnectMutation = useMutation({
    mutationFn: (id: string) => api.disconnectIntegration(id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    }
  });

  const enableMutation = useMutation({
    mutationFn: (id: string) => api.enableIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Integration enabled." });
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => api.disableIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Integration disabled." });
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const testMutation = useMutation({
    mutationFn: (id: string) => api.testIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Connection test passed." });
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    },
    onError: async (err) => {
      // Backend may auto-disable on failed test.
      setNotice({ kind: "error", text: asErrorMessage(err) });
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    }
  });

  const waLogoutMutation = useMutation({
    mutationFn: () => api.rawPost("/api/whatsapp-bridge/logout", {}),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "WhatsApp pairing cleared." });
      await queryClient.invalidateQueries({ queryKey: ["wa-bridge-status", "integrations-panel"] });
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const integrations = showIntegrations ? integrationsQ.data?.integrations || [] : [];
  const settings = asRecord(settingsQ.data);
  const waBridge = asRecord(waBridgeQ.data);
  const telegramStatus = asRecord(telegramStatusQ.data);
  const hasTelegramToken = toBool(settings.has_telegram_token);
  const hasWhatsAppToken = toBool(settings.has_whatsapp_token);
  const searchSerperConfigured = toBool(settings.search_serper_configured);
  const searchBraveConfigured = toBool(settings.search_brave_configured);
  const telegramTokenConfigured = hasTelegramToken || channelForm.telegram_bot_token.trim().length > 0;
  const whatsappTokenConfigured = hasWhatsAppToken || channelForm.whatsapp_access_token.trim().length > 0;
  const telegramConnectionStatusRaw = telegramStatusQ.isFetching
    ? "checking"
    : str(telegramStatus.status, channelForm.telegram_enabled ? "ready" : "disabled");
  const telegramConnectionDetail = str(telegramStatus.detail, "");
  const whatsappConnectionStatusRaw = (() => {
    if (!channelForm.whatsapp_enabled) return "disabled";
    if (channelForm.whatsapp_mode === "cloud_api") {
      return whatsappTokenConfigured && channelForm.whatsapp_phone_number_id.trim() ? "ready" : "missing_config";
    }
    if (waBridgeQ.isFetching) return "checking";
    if (waBridgeQ.error) return "error";
    return str(waBridge.status, "disconnected");
  })();
  const whatsappConnectionDetail =
    channelForm.whatsapp_mode === "cloud_api"
      ? whatsappConnectionStatusRaw === "ready"
        ? "Cloud API credentials are configured."
        : "Cloud API token and phone number ID are required."
      : str(waBridge.error, "");
  const mcpServers = showMcp ? asRecords(asRecord(mcpQ.data).servers) : [];
  const sorted = useMemo(
    () => [...integrations].sort((a, b) => a.name.localeCompare(b.name)),
    [integrations]
  );
  const enabledList = sorted.filter((i) => i.enabled);
  const disabledList = sorted.filter((i) => !i.enabled);
  const mcpSorted = useMemo(
    () => [...mcpServers].sort((a, b) => str(a.name, "").localeCompare(str(b.name, ""))),
    [mcpServers]
  );
  const sshKeyNames = asStringList(asRecord(sshKeysQ.data).keys).sort((a, b) => a.localeCompare(b));
  const sshConnectionsText = str(asRecord(sshConnectionsQ.data).connections, "");
  const sshConnectionNames = parseSshConnectionNames(sshConnectionsText);

  useEffect(() => {
    if (!showIntegrations || !settingsQ.data || channelsDirty) return;
    const next = asRecord(settingsQ.data);
    setChannelForm({
      search_primary: str(next.search_primary, "playwright"),
      search_fallback1: str(next.search_fallback1, "duckduckgo"),
      search_fallback2: str(next.search_fallback2, "none"),
      search_serper_key: "",
      search_searxng_url: str(next.search_searxng_url, ""),
      search_brave_key: "",
      telegram_enabled: toBool(next.telegram_enabled),
      telegram_bot_token: "",
      telegram_allowed_users_csv: asStringList(next.telegram_allowed_users).join(", "),
      whatsapp_enabled: toBool(next.whatsapp_enabled),
      whatsapp_mode: str(next.whatsapp_mode, "baileys") === "cloud_api" ? "cloud_api" : "baileys",
      whatsapp_access_token: "",
      whatsapp_phone_number_id: str(next.whatsapp_phone_number_id, ""),
      whatsapp_verify_token: "",
      whatsapp_bridge_url: str(next.whatsapp_bridge_url, ""),
      whatsapp_dm_policy: str(next.whatsapp_dm_policy, "all") || "all",
      whatsapp_allowed_numbers_csv: asStringList(next.whatsapp_allowed_numbers).join(", ")
    });
  }, [showIntegrations, settingsQ.data, settingsQ.dataUpdatedAt, channelsDirty]);

  const setChannelField = <K extends keyof ChannelSettingsForm>(
    key: K,
    value: ChannelSettingsForm[K]
  ) => {
    setChannelsDirty(true);
    setChannelForm((prev) => ({ ...prev, [key]: value }));
  };

  const openTelegramSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.telegram_enabled) {
      setChannelField("telegram_enabled", true);
    }
    setTelegramSetupOpen(true);
  };

  const openWhatsAppSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.whatsapp_enabled) {
      setChannelField("whatsapp_enabled", true);
    }
    setWhatsAppSetupOpen(true);
  };

  const openSearchSetup = () => {
    setSearchSetupOpen(true);
  };

  const saveChannelsMutation = useMutation({
    mutationFn: async () => {
      const payload: Record<string, unknown> = {
        llm_provider: str(settings.llm_provider, "ollama"),
        llm_model: str(settings.llm_model, ""),
        llm_base_url: settings.llm_base_url ?? null,
        llm_api_key: null,
        llm_fallback_provider: settings.llm_fallback_provider ?? null,
        llm_fallback_model: settings.llm_fallback_model ?? null,
        llm_fallback_base_url: settings.llm_fallback_base_url ?? null,
        llm_fallback_api_key: null,
        search_primary: channelForm.search_primary.trim() || null,
        search_fallback1: channelForm.search_fallback1.trim() || null,
        search_fallback2: channelForm.search_fallback2.trim() || null,
        search_serper_key: channelForm.search_serper_key.trim() || null,
        search_searxng_url: channelForm.search_searxng_url.trim() || null,
        search_brave_key: channelForm.search_brave_key.trim() || null,
        telegram_enabled: !!channelForm.telegram_enabled,
        telegram_bot_token: channelForm.telegram_bot_token.trim() || null,
        telegram_allowed_users: parseTelegramUsers(channelForm.telegram_allowed_users_csv),
        whatsapp_enabled: !!channelForm.whatsapp_enabled,
        whatsapp_mode: channelForm.whatsapp_mode,
        whatsapp_access_token: channelForm.whatsapp_access_token.trim() || null,
        whatsapp_phone_number_id: channelForm.whatsapp_phone_number_id.trim() || null,
        whatsapp_verify_token: channelForm.whatsapp_verify_token.trim() || null,
        whatsapp_bridge_url: channelForm.whatsapp_bridge_url.trim() || null,
        whatsapp_dm_policy: channelForm.whatsapp_dm_policy.trim() || null,
        whatsapp_allowed_numbers: parseCsvList(channelForm.whatsapp_allowed_numbers_csv)
      };
      return api.rawPost("/settings", payload);
    },
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Channel settings saved." });
      setChannelsDirty(false);
      setChannelForm((prev) => ({
        ...prev,
        search_serper_key: "",
        search_brave_key: "",
        telegram_bot_token: "",
        whatsapp_access_token: "",
        whatsapp_verify_token: ""
      }));
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const openConfig = (integration: IntegrationItem) => {
    setActive(integration);
    setFormError(null);
    setFormValues({});
    setConfigSuccess(false);
  };

  const closeConfig = () => {
    setActive(null);
    setFormValues({});
    setFormError(null);
    setSaving(false);
    setConfigSuccess(false);
  };

  const openCreateMcp = () => {
    setMcpDialogOpen(true);
    setMcpEditingId(null);
    setMcpForm(defaultMcpForm());
    setMcpHasStoredAuth(false);
    setMcpError(null);
  };

  const openEditMcp = (server: JsonRecord) => {
    const transport = asRecord(server.transport);
    const auth = asRecord(server.auth);
    const transportType = str(transport.type, "http") === "stdio" ? "stdio" : "http";
    const authTypeRaw = str(auth.auth_type, "none");
    const authType: McpAuthType = ["none", "bearer", "basic", "header", "query"].includes(authTypeRaw)
      ? (authTypeRaw as McpAuthType)
      : "none";

    setMcpEditingId(str(server.id, ""));
    setMcpDialogOpen(true);
    setMcpError(null);
    setMcpHasStoredAuth(toBool(auth.has_auth));
    setMcpForm({
      id: str(server.id, ""),
      name: str(server.name, ""),
      description: str(server.description, ""),
      enabled: toBool(server.enabled),
      resources_enabled: toBool(server.resources_enabled),
      transport_type: transportType,
      url: str(transport.url, ""),
      command: str(transport.command, ""),
      args_csv: asStringList(transport.args).join(", "),
      working_dir: str(transport.working_dir, ""),
      auth_type: authType,
      auth_header: str(auth.header, "Authorization"),
      auth_name: str(auth.name, ""),
      auth_token: "",
      auth_username: "",
      auth_password: "",
      auth_clear: false,
      tool_allowlist_csv: asStringList(server.tool_allowlist).join(", "),
      resource_allowlist_csv: asStringList(server.resource_allowlist).join(", "),
      timeout_secs: str(server.timeout_secs, "15"),
      max_response_bytes: str(server.max_response_bytes, "1048576")
    });
  };

  const closeMcpDialog = () => {
    setMcpDialogOpen(false);
    setMcpEditingId(null);
    setMcpForm(defaultMcpForm());
    setMcpHasStoredAuth(false);
    setMcpError(null);
  };

  const syncAfterMcpMutation = async () => {
    await queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
    await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
  };

  const syncAfterSshMutation = async () => {
    await queryClient.invalidateQueries({ queryKey: ["ssh-keys"] });
    await queryClient.invalidateQueries({ queryKey: ["ssh-connections"] });
  };

  const saveMcpMutation = useMutation({
    mutationFn: async () => {
      const name = mcpForm.name.trim();
      if (!name) throw new Error("Server name is required.");

      const timeoutSecs = Number(mcpForm.timeout_secs);
      if (!Number.isFinite(timeoutSecs) || timeoutSecs <= 0) {
        throw new Error("Timeout must be a positive number.");
      }
      const maxResponseBytes = Number(mcpForm.max_response_bytes);
      if (!Number.isFinite(maxResponseBytes) || maxResponseBytes <= 0) {
        throw new Error("Max response bytes must be a positive number.");
      }

      const payload: Record<string, unknown> = {
        name,
        description: mcpForm.description.trim() || undefined,
        enabled: mcpForm.enabled,
        resources_enabled: mcpForm.resources_enabled,
        tool_allowlist: parseCsvList(mcpForm.tool_allowlist_csv),
        resource_allowlist: parseCsvList(mcpForm.resource_allowlist_csv),
        timeout_secs: Math.floor(timeoutSecs),
        max_response_bytes: Math.floor(maxResponseBytes)
      };
      if (!mcpEditingId && mcpForm.id.trim()) payload.id = mcpForm.id.trim();

      if (mcpForm.transport_type === "http") {
        if (!mcpForm.url.trim()) throw new Error("HTTP URL is required.");
        payload.transport = { type: "http", url: mcpForm.url.trim() };
      } else {
        if (!mcpForm.command.trim()) throw new Error("Stdio command is required.");
        payload.transport = {
          type: "stdio",
          command: mcpForm.command.trim(),
          args: parseCsvList(mcpForm.args_csv),
          working_dir: mcpForm.working_dir.trim() || undefined
        };
      }

      if (mcpForm.auth_type === "none") {
        payload.auth = { type: "none" };
      } else if (mcpForm.auth_type === "bearer") {
        payload.auth = {
          type: "bearer",
          header: mcpForm.auth_header.trim() || undefined,
          token: mcpForm.auth_token.trim() || undefined,
          clear: mcpForm.auth_clear
        };
      } else if (mcpForm.auth_type === "basic") {
        payload.auth = {
          type: "basic",
          username: mcpForm.auth_username.trim() || undefined,
          password: mcpForm.auth_password.trim() || undefined,
          clear: mcpForm.auth_clear
        };
      } else if (mcpForm.auth_type === "header") {
        const nameValue = mcpForm.auth_name.trim();
        if (!nameValue) throw new Error("Header name is required for header auth.");
        payload.auth = {
          type: "header",
          name: nameValue,
          value: mcpForm.auth_token.trim() || undefined,
          clear: mcpForm.auth_clear
        };
      } else if (mcpForm.auth_type === "query") {
        const nameValue = mcpForm.auth_name.trim();
        if (!nameValue) throw new Error("Query parameter name is required for query auth.");
        payload.auth = {
          type: "query",
          name: nameValue,
          value: mcpForm.auth_token.trim() || undefined,
          clear: mcpForm.auth_clear
        };
      }

      if (mcpEditingId) {
        await api.rawPut(`/mcp/servers/${encodeURIComponent(mcpEditingId)}`, payload);
      } else {
        await api.rawPost("/mcp/servers", payload);
      }
    },
    onSuccess: async () => {
      setMcpNotice({ kind: "success", text: mcpEditingId ? "MCP server updated." : "MCP server created." });
      closeMcpDialog();
      await syncAfterMcpMutation();
    },
    onError: (err) => {
      setMcpError(asErrorMessage(err));
    }
  });

  const deleteMcpMutation = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/mcp/servers/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      setMcpNotice({ kind: "success", text: "MCP server deleted." });
      await syncAfterMcpMutation();
    },
    onError: (err) => {
      setMcpNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const refreshMcpMutation = useMutation({
    mutationFn: (id: string) => api.rawPost(`/mcp/servers/${encodeURIComponent(id)}/refresh`, {}),
    onSuccess: async () => {
      setMcpNotice({ kind: "success", text: "MCP refresh queued." });
      await syncAfterMcpMutation();
    },
    onError: (err) => {
      setMcpNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const openSshKeyDialog = () => {
    setSshKeyName("");
    setSshKeyPem("");
    setSshKeyError(null);
    setSshKeyDialogOpen(true);
  };
  const closeSshKeyDialog = () => {
    setSshKeyDialogOpen(false);
    setSshKeyError(null);
  };
  const openSshConnDialog = () => {
    setSshConnName("");
    setSshHost("");
    setSshPort("22");
    setSshUsername("");
    setSshConnKeyName("");
    setSshConnError(null);
    setSshConnDialogOpen(true);
  };
  const closeSshConnDialog = () => {
    setSshConnDialogOpen(false);
    setSshConnError(null);
  };
  const openSshTestDialog = () => {
    setSshTestConnName("");
    setSshTestOutput("");
    setSshTestDialogOpen(true);
  };
  const closeSshTestDialog = () => {
    setSshTestDialogOpen(false);
    setSshTestOutput("");
  };

  const uploadSshKeyMutation = useMutation({
    mutationFn: () =>
      api.rawPost("/ssh/keys", {
        name: sshKeyName.trim(),
        pem_content: sshKeyPem
      }),
    onSuccess: async () => {
      setSshNotice({ kind: "success", text: "SSH key uploaded." });
      closeSshKeyDialog();
      await syncAfterSshMutation();
    },
    onError: (err) => setSshKeyError(asErrorMessage(err))
  });

  const removeSshKeyMutation = useMutation({
    mutationFn: (name: string) => api.rawDelete(`/ssh/keys/${encodeURIComponent(name)}`),
    onSuccess: async () => {
      setSshNotice({ kind: "success", text: "SSH key removed." });
      await syncAfterSshMutation();
    },
    onError: (err) => setSshNotice({ kind: "error", text: asErrorMessage(err) })
  });

  const addSshConnectionMutation = useMutation({
    mutationFn: () =>
      api.rawPost("/ssh/connections", {
        name: sshConnName.trim(),
        host: sshHost.trim(),
        port: Number(sshPort) || 22,
        username: sshUsername.trim(),
        key_name: sshConnKeyName.trim()
      }),
    onSuccess: async () => {
      setSshNotice({ kind: "success", text: "SSH connection saved." });
      closeSshConnDialog();
      await syncAfterSshMutation();
    },
    onError: (err) => setSshConnError(asErrorMessage(err))
  });

  const removeSshConnectionMutation = useMutation({
    mutationFn: (name: string) => api.rawDelete(`/ssh/connections/${encodeURIComponent(name)}`),
    onSuccess: async () => {
      setSshNotice({ kind: "success", text: "SSH connection removed." });
      await syncAfterSshMutation();
    },
    onError: (err) => setSshNotice({ kind: "error", text: asErrorMessage(err) })
  });

  const testSshMutation = useMutation({
    mutationFn: () => api.rawPost("/ssh/test", { connection: sshTestConnName.trim() }),
    onSuccess: (out) => {
      const payload = asRecord(out);
      setSshTestOutput(str(payload.output, str(payload.status, "ok")));
      setSshNotice({ kind: "success", text: "SSH test succeeded." });
    },
    onError: (err) => {
      setSshNotice({ kind: "error", text: asErrorMessage(err) });
      setSshTestOutput("");
    }
  });

  const submitConfig = async () => {
    if (!active) return;
    const fields = active.config_fields || [];
    for (const field of fields) {
      if (field.required && !(formValues[field.key] || "").trim()) {
        setFormError(`Missing required field: ${field.label}`);
        return;
      }
    }

    setSaving(true);
    setFormError(null);
    try {
      const payload: Record<string, unknown> = {};
      for (const field of fields) {
        const value = formValues[field.key];
        if (value != null && value.trim() !== "") {
          payload[field.key] = value;
        }
      }
      await configureMutation.mutateAsync({ id: active.id, payload });
      await integrationsQ.refetch();
      setConfigSuccess(true);
      setTimeout(() => {
        closeConfig();
      }, 850);
    } catch (err) {
      setFormError(asErrorMessage(err));
      // Backend disables integration on failed validation; refresh to reflect it.
      await integrationsQ.refetch();
      setSaving(false);
    }
  };

  const disconnectActive = async () => {
    if (!active) return;
    if (!window.confirm(`Disconnect ${active.name}?`)) return;
    setSaving(true);
    setFormError(null);
    try {
      await disconnectMutation.mutateAsync(active.id);
      closeConfig();
    } catch (err) {
      setFormError(asErrorMessage(err));
      setSaving(false);
    }
  };

  const startIntegrationAuth = async (integration: IntegrationItem) => {
    setNotice(null);
    setOauthBusyId(integration.id);
    try {
      let authUrl = str(integration.auth_url, "").trim();
      if (!authUrl) {
        const payload = await api.rawGet(`/integrations/${encodeURIComponent(integration.id)}/auth`);
        authUrl = extractAuthUrl(payload);
      }
      if (!authUrl) throw new Error("No OAuth URL is available yet. Configure this integration first.");
      window.open(authUrl, "_blank", "noopener,noreferrer");
      setNotice({ kind: "success", text: "OAuth window opened. Finish sign-in, then run Test." });
    } catch (err) {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    } finally {
      setOauthBusyId(null);
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    }
  };

  const renderField = (field: IntegrationConfigField) => {
    const value = formValues[field.key] || "";
    if (field.input_type === "select") {
      return (
        <TextField
          key={field.key}
          fullWidth
          select
          label={field.label}
          value={value}
          onChange={(e) =>
            setFormValues((prev) => ({
              ...prev,
              [field.key]: e.target.value
            }))
          }
          required={field.required}
          size="small"
        >
          {(field.options || []).map((opt) => (
            <MenuItem key={opt} value={opt}>
              {opt}
            </MenuItem>
          ))}
        </TextField>
      );
    }

    return (
      <TextField
        key={field.key}
        fullWidth
        multiline={field.input_type === "textarea"}
        minRows={field.input_type === "textarea" ? 4 : undefined}
        label={field.label}
        type={field.input_type === "password" ? "password" : "text"}
        placeholder={field.placeholder || ""}
        value={value}
        onChange={(e) =>
          setFormValues((prev) => ({
            ...prev,
            [field.key]: e.target.value
          }))
        }
        required={field.required}
        size="small"
      />
    );
  };

  return (
    <Stack spacing={2} sx={embedded ? undefined : { p: 1, height: "100%", overflow: "auto" }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography variant="h6">
          {mode === "mcp" ? "MCP Servers" : mode === "integrations" ? "Integrations" : "Integrations"}
        </Typography>
        <Stack direction="row" spacing={1} />
      </Stack>
      {mode !== "mcp" ? (
        <>
          <Typography
            variant="body2"
            sx={{
              fontWeight: 700,
              color: "#69e2ff"
            }}
          >
            These are pre-built integrations. You can always chat with the agent to build any custom integration on your own.
          </Typography>
          <Typography variant="caption" color="text.secondary">
            Integrations are long-lived connectors (auth + config). To import skills from a URL, use the Skills tab.
          </Typography>
        </>
      ) : (
        <Typography variant="caption" color="text.secondary">
          Configure external MCP servers (HTTP or stdio), auth, allowlists, and refresh tools/resources.
        </Typography>
      )}

      {showIntegrations && notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {showMcp && mcpNotice ? <Alert severity={mcpNotice.kind}>{mcpNotice.text}</Alert> : null}
      {showMcp && sshNotice ? <Alert severity={sshNotice.kind}>{sshNotice.text}</Alert> : null}

      {showIntegrations && integrationsQ.error ? (
        <Alert severity="error">
          Failed to load integrations:{" "}
          {integrationsQ.error instanceof Error ? integrationsQ.error.message : "Unknown error"}
        </Alert>
      ) : null}

      {showIntegrations ? (
        <Box className="list-shell">
          <Typography variant="subtitle2" sx={{ mb: 1 }}>
            Channels
          </Typography>
          {settingsQ.error ? (
            <Alert severity="error">
              Failed to load channels: {settingsQ.error instanceof Error ? settingsQ.error.message : "Unknown error"}
            </Alert>
          ) : (
            <Grid2 container spacing={1.5}>
              <Grid2 size={{ xs: 12 }}>
                <Box className="list-shell" sx={{ minHeight: 0 }}>
                  <Stack spacing={1.25}>
                    <Typography variant="subtitle1" fontWeight={700}>
                      Web Search
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      Primary: {channelForm.search_primary || "-"} | Fallback 1: {channelForm.search_fallback1 || "-"} | Fallback 2: {channelForm.search_fallback2 || "-"}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      Serper key configured: {searchSerperConfigured ? "yes" : "no"} | Brave key configured: {searchBraveConfigured ? "yes" : "no"}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      Primary fails over to fallbacks in order. Playwright requires no API key.
                    </Typography>
                    <Stack direction="row" spacing={1}>
                      <Button size="small" variant="contained" onClick={openSearchSetup}>
                        Manage
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Box className="list-shell" sx={{ minHeight: 0, height: "100%" }}>
                  <Stack spacing={1.25}>
                    <Typography variant="subtitle1" fontWeight={700}>
                      Telegram
                    </Typography>
                    <Stack direction="row" spacing={1} alignItems="center">
                      <Typography variant="body2" color="text.secondary">
                        Connection:
                      </Typography>
                      <Chip
                        size="small"
                        label={channelStatusLabel(telegramConnectionStatusRaw)}
                        color={channelStatusColor(telegramConnectionStatusRaw)}
                        variant="outlined"
                      />
                    </Stack>
                    <Typography variant="caption" color="text.secondary">
                      Token configured: {telegramTokenConfigured ? "yes" : "no"}
                    </Typography>
                    {telegramConnectionDetail ? (
                      <Typography variant="caption" color="text.secondary">
                        {telegramConnectionDetail}
                      </Typography>
                    ) : null}
                    <Typography variant="caption" color="text.secondary">
                      Connect your bot token and allowed user IDs so only approved users can message the agent.
                    </Typography>
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        variant="contained"
                        onClick={() => openTelegramSetup(!channelForm.telegram_enabled)}
                      >
                        {channelForm.telegram_enabled ? "Manage" : "Enable"}
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <Box className="list-shell" sx={{ minHeight: 0, height: "100%" }}>
                  <Stack spacing={1.25}>
                    <Typography variant="subtitle1" fontWeight={700}>
                      WhatsApp
                    </Typography>
                    <Stack direction="row" spacing={1} alignItems="center">
                      <Typography variant="body2" color="text.secondary">
                        Connection:
                      </Typography>
                      <Chip
                        size="small"
                        label={channelStatusLabel(whatsappConnectionStatusRaw)}
                        color={channelStatusColor(whatsappConnectionStatusRaw)}
                        variant="outlined"
                      />
                    </Stack>
                    <Typography variant="caption" color="text.secondary">
                      Mode: {channelForm.whatsapp_mode === "cloud_api" ? "Enterprise Cloud API" : "Scan QR"}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      Token configured (Enterprise Cloud API): {whatsappTokenConfigured ? "yes" : "no"}
                    </Typography>
                    {whatsappConnectionDetail ? (
                      <Typography variant="caption" color="text.secondary">
                        {whatsappConnectionDetail}
                      </Typography>
                    ) : null}
                    <Typography variant="caption" color="text.secondary">
                      Choose between quick QR pairing or Meta Cloud API enterprise setup.
                    </Typography>
                    <Stack direction="row" spacing={1}>
                      <Button
                        size="small"
                        variant="contained"
                        onClick={() => openWhatsAppSetup(!channelForm.whatsapp_enabled)}
                      >
                        {channelForm.whatsapp_enabled ? "Manage" : "Enable"}
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>
            </Grid2>
          )}
        </Box>
      ) : null}

      {showIntegrations ? (
        <Box className="list-shell">
          <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mb: 1.5 }}>
            <Typography variant="subtitle2">
              Integrations ({enabledList.length} active, {disabledList.length} available)
            </Typography>
          </Stack>
          <Grid2 container spacing={1}>
            {[...enabledList, ...disabledList].map((integration) => {
              const isEnabled = integration.enabled;
              const sc = statusColor(integration.status);
              const dotColor = sc === "success" ? "#4ad29d" : sc === "error" ? "rgba(255,88,88,0.85)" : sc === "warning" ? "rgba(255,180,50,0.85)" : "rgba(255,255,255,0.25)";
              return (
                <Grid2 key={integration.id} size={{ xs: 6, sm: 4, md: 3, lg: 2 }}>
                  <Box
                    role="button"
                    tabIndex={0}
                    onClick={() => {
                      if (isEnabled) {
                        if (integration.config_fields && integration.config_fields.length > 0) {
                          openConfig(integration);
                        }
                      } else {
                        if (integration.status === "connected") {
                          enableMutation.mutate(integration.id);
                        } else {
                          openConfig(integration);
                        }
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        if (isEnabled) {
                          if (integration.config_fields && integration.config_fields.length > 0) {
                            openConfig(integration);
                          }
                        } else {
                          if (integration.status === "connected") {
                            enableMutation.mutate(integration.id);
                          } else {
                            openConfig(integration);
                          }
                        }
                      }
                    }}
                    sx={{
                      height: "100%",
                      p: 1.2,
                      borderRadius: "10px",
                      border: isEnabled ? "1px solid rgba(74,210,157,0.35)" : "1px solid rgba(108,156,212,0.18)",
                      background: isEnabled ? "rgba(74,210,157,0.06)" : "rgba(6,15,29,0.5)",
                      cursor: "pointer",
                      transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                      opacity: isEnabled ? 1 : 0.75,
                      "&:hover": {
                        borderColor: isEnabled ? "rgba(74,210,157,0.55)" : "rgba(47,212,255,0.4)",
                        background: isEnabled ? "rgba(74,210,157,0.1)" : "rgba(47,212,255,0.06)",
                        boxShadow: "0 2px 12px rgba(0,0,0,0.2)",
                        opacity: 1
                      }
                    }}
                  >
                    <Stack spacing={0.5} sx={{ minHeight: 56 }}>
                      <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={0.5}>
                        <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700, fontSize: "0.82rem" }}>
                          {integration.name}
                        </Typography>
                        <Box
                          sx={{
                            width: 8,
                            height: 8,
                            borderRadius: "50%",
                            background: dotColor,
                            flex: "0 0 auto"
                          }}
                        />
                      </Stack>
                      <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.3, display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden" }}>
                        {isEnabled ? (integration.status_detail || integration.description) : integration.description}
                      </Typography>
                    </Stack>
                    <Chip
                      size="small"
                      label={isEnabled ? "Enabled" : "Disabled"}
                      sx={{
                        mt: 0.8,
                        height: 20,
                        fontSize: "0.66rem",
                        fontWeight: 600,
                        borderColor: isEnabled ? "rgba(74,210,157,0.3)" : "rgba(255,255,255,0.1)",
                        color: isEnabled ? "rgba(74,210,157,0.9)" : "rgba(180,200,225,0.5)"
                      }}
                      variant="outlined"
                    />
                  </Box>
                </Grid2>
              );
            })}
          </Grid2>
        </Box>
      ) : null}

      {showMcp ? (
        <Box className="list-shell">
        <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 1 }}>
          <Typography variant="subtitle2">MCP Servers ({mcpSorted.length})</Typography>
          <Button size="small" variant="contained" onClick={openCreateMcp}>
            Add MCP Server
          </Button>
        </Stack>
        <Typography variant="caption" color="text.secondary">
          Manage external MCP servers (HTTP or stdio). Tools/resources hot-reload after create/update.
        </Typography>
        {mcpNotice ? <Alert sx={{ mt: 1 }} severity={mcpNotice.kind}>{mcpNotice.text}</Alert> : null}
        {mcpQ.error ? (
          <Alert sx={{ mt: 1 }} severity="error">
            Failed to load MCP servers: {mcpQ.error instanceof Error ? mcpQ.error.message : "Unknown error"}
          </Alert>
        ) : mcpSorted.length === 0 ? (
          <Typography sx={{ mt: 1 }} variant="body2" color="text.secondary">
            No MCP servers configured.
          </Typography>
        ) : (
          <Grid2 container spacing={1.5} sx={{ mt: 0.5 }}>
            {mcpSorted.map((server) => {
              const id = str(server.id, "");
              const warnings = asStringList(server.warnings);
              const lastError = str(server.last_error, "");
              return (
                <Grid2 key={id || str(server.name, Math.random().toString())} size={{ xs: 12, md: 6 }}>
                  <Box className="list-shell" sx={{ minHeight: 0 }}>
                    <Stack spacing={1}>
                      <Stack direction="row" alignItems="center" justifyContent="space-between">
                        <Typography variant="subtitle1" fontWeight={700}>
                          {str(server.name, "(unnamed)")}
                        </Typography>
                        <Stack direction="row" spacing={0.5}>
                          <Chip
                            size="small"
                            label={toBool(server.enabled) ? "enabled" : "disabled"}
                            color={toBool(server.enabled) ? "success" : "default"}
                          />
                          <Chip
                            size="small"
                            label={toBool(server.resources_enabled) ? "resources on" : "resources off"}
                            color={toBool(server.resources_enabled) ? "warning" : "default"}
                          />
                        </Stack>
                      </Stack>
                      <Typography variant="body2" color="text.secondary">
                        {str(server.description, "No description")}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        {transportSummary(server)}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Auth: {authSummary(server)} | Tools: {str(server.tool_count, "0")} | Resources: {str(server.resource_count, "0")}
                      </Typography>
                      {lastError ? <Alert severity="error">{lastError}</Alert> : null}
                      {warnings.length > 0 ? (
                        <Alert severity="warning">{warnings.slice(0, 2).join(" ")}</Alert>
                      ) : null}
                      <Stack direction="row" justifyContent="flex-end">
                        <CardActionsMenu
                          ariaLabel={`${str(server.name, "MCP server")} options`}
                          actions={[
                            {
                              label: "Edit",
                              onClick: () => openEditMcp(server)
                            },
                            {
                              label: "Refresh",
                              disabled: refreshMcpMutation.isPending || !id,
                              onClick: () => refreshMcpMutation.mutate(id)
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              disabled: deleteMcpMutation.isPending || !id,
                              onClick: () => {
                                if (!id) return;
                                if (!window.confirm(`Delete MCP server '${str(server.name, id)}'?`)) return;
                                deleteMcpMutation.mutate(id);
                              }
                            }
                          ]}
                        />
                      </Stack>
                    </Stack>
                  </Box>
                </Grid2>
              );
            })}
          </Grid2>
        )}
      </Box>
      ) : null}

      {showMcp ? (
        <Box className="list-shell">
          <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 1 }}>
            <Typography variant="subtitle2">SSH Access ({sshKeyNames.length} keys, {sshConnectionNames.length} connections)</Typography>
            <Stack direction="row" spacing={1}>
              <Button size="small" variant="outlined" onClick={openSshTestDialog} disabled={sshConnectionNames.length === 0}>
                Test
              </Button>
              <Button size="small" variant="contained" onClick={openSshKeyDialog}>
                Add SSH Key
              </Button>
              <Button size="small" variant="contained" onClick={openSshConnDialog} disabled={sshKeyNames.length === 0}>
                Add Connection
              </Button>
            </Stack>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            Upload private keys, create named connection profiles, and run connectivity tests.
          </Typography>
          {sshNotice ? <Alert sx={{ mt: 1 }} severity={sshNotice.kind}>{sshNotice.text}</Alert> : null}
          {sshKeysQ.error ? (
            <Alert sx={{ mt: 1 }} severity="error">
              Failed to load SSH keys: {sshKeysQ.error instanceof Error ? sshKeysQ.error.message : "Unknown error"}
            </Alert>
          ) : null}
          {sshConnectionsQ.error ? (
            <Alert sx={{ mt: 1 }} severity="error">
              Failed to load SSH connections: {sshConnectionsQ.error instanceof Error ? sshConnectionsQ.error.message : "Unknown error"}
            </Alert>
          ) : null}

          {sshKeyNames.length === 0 && sshConnectionNames.length === 0 ? (
            <Typography sx={{ mt: 1 }} variant="body2" color="text.secondary">
              No SSH keys or connections configured.
            </Typography>
          ) : (
            <Grid2 container spacing={1.5} sx={{ mt: 0.5 }}>
              {sshKeyNames.map((name) => (
                <Grid2 key={`key-${name}`} size={{ xs: 12, md: 6 }}>
                  <Box className="list-shell" sx={{ minHeight: 0 }}>
                    <Stack spacing={0.5}>
                      <Stack direction="row" alignItems="center" justifyContent="space-between">
                        <Typography variant="subtitle1" fontWeight={700}>{name}</Typography>
                        <Chip size="small" label="key" color="default" />
                      </Stack>
                      <Typography variant="caption" color="text.secondary">SSH private key</Typography>
                      <Stack direction="row" justifyContent="flex-end">
                        <CardActionsMenu
                          ariaLabel={`${name} options`}
                          actions={[
                            {
                              label: "Delete",
                              tone: "error",
                              disabled: removeSshKeyMutation.isPending,
                              onClick: () => {
                                if (!window.confirm(`Remove SSH key '${name}'?`)) return;
                                setSshNotice(null);
                                removeSshKeyMutation.mutate(name);
                              }
                            }
                          ]}
                        />
                      </Stack>
                    </Stack>
                  </Box>
                </Grid2>
              ))}
              {sshConnectionNames.map((name) => (
                <Grid2 key={`conn-${name}`} size={{ xs: 12, md: 6 }}>
                  <Box className="list-shell" sx={{ minHeight: 0 }}>
                    <Stack spacing={0.5}>
                      <Stack direction="row" alignItems="center" justifyContent="space-between">
                        <Typography variant="subtitle1" fontWeight={700}>{name}</Typography>
                        <Chip size="small" label="connection" color="success" />
                      </Stack>
                      <Typography variant="caption" color="text.secondary">SSH connection profile</Typography>
                      <Stack direction="row" justifyContent="flex-end">
                        <CardActionsMenu
                          ariaLabel={`${name} options`}
                          actions={[
                            {
                              label: "Test",
                              onClick: () => {
                                setSshTestConnName(name);
                                setSshTestOutput("");
                                setSshTestDialogOpen(true);
                              }
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              disabled: removeSshConnectionMutation.isPending,
                              onClick: () => {
                                if (!window.confirm(`Remove SSH connection '${name}'?`)) return;
                                setSshNotice(null);
                                removeSshConnectionMutation.mutate(name);
                              }
                            }
                          ]}
                        />
                      </Stack>
                    </Stack>
                  </Box>
                </Grid2>
              ))}
            </Grid2>
          )}
        </Box>
      ) : null}

      {showIntegrations ? (
      <Dialog open={searchSetupOpen} onClose={() => setSearchSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Web Search Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" color="text.secondary">
              Choose search priority and optional API providers. Playwright works without any API key.
            </Typography>
            <TextField
              fullWidth
              size="small"
              select
              label="Primary"
              value={channelForm.search_primary}
              onChange={(e) => setChannelField("search_primary", e.target.value)}
            >
              <MenuItem value="playwright">playwright</MenuItem>
              <MenuItem value="serper">serper</MenuItem>
              <MenuItem value="searxng">searxng</MenuItem>
              <MenuItem value="brave_api">brave_api</MenuItem>
              <MenuItem value="duckduckgo">duckduckgo</MenuItem>
            </TextField>
            <TextField
              fullWidth
              size="small"
              select
              label="Fallback 1"
              value={channelForm.search_fallback1}
              onChange={(e) => setChannelField("search_fallback1", e.target.value)}
            >
              <MenuItem value="none">none</MenuItem>
              <MenuItem value="playwright">playwright</MenuItem>
              <MenuItem value="serper">serper</MenuItem>
              <MenuItem value="searxng">searxng</MenuItem>
              <MenuItem value="brave_api">brave_api</MenuItem>
              <MenuItem value="duckduckgo">duckduckgo</MenuItem>
            </TextField>
            <TextField
              fullWidth
              size="small"
              select
              label="Fallback 2"
              value={channelForm.search_fallback2}
              onChange={(e) => setChannelField("search_fallback2", e.target.value)}
            >
              <MenuItem value="none">none</MenuItem>
              <MenuItem value="playwright">playwright</MenuItem>
              <MenuItem value="serper">serper</MenuItem>
              <MenuItem value="searxng">searxng</MenuItem>
              <MenuItem value="brave_api">brave_api</MenuItem>
              <MenuItem value="duckduckgo">duckduckgo</MenuItem>
            </TextField>
            {[channelForm.search_primary, channelForm.search_fallback1, channelForm.search_fallback2].includes("serper") ? (
              <TextField
                fullWidth
                size="small"
                type="password"
                label={`Serper API Key (${searchSerperConfigured ? "configured" : "not configured"})`}
                value={channelForm.search_serper_key}
                onChange={(e) => setChannelField("search_serper_key", e.target.value)}
                placeholder={searchSerperConfigured ? "Leave blank to keep current key" : "Enter Serper API key"}
              />
            ) : null}
            {[channelForm.search_primary, channelForm.search_fallback1, channelForm.search_fallback2].includes("searxng") ? (
              <TextField
                fullWidth
                size="small"
                label="SearXNG URL"
                value={channelForm.search_searxng_url}
                onChange={(e) => setChannelField("search_searxng_url", e.target.value)}
                placeholder="http://localhost:8080"
              />
            ) : null}
            {[channelForm.search_primary, channelForm.search_fallback1, channelForm.search_fallback2].includes("brave_api") ? (
              <TextField
                fullWidth
                size="small"
                type="password"
                label={`Brave Search API Key (${searchBraveConfigured ? "configured" : "not configured"})`}
                value={channelForm.search_brave_key}
                onChange={(e) => setChannelField("search_brave_key", e.target.value)}
                placeholder={searchBraveConfigured ? "Leave blank to keep current key" : "Enter Brave Search API key"}
              />
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSearchSetupOpen(false)} disabled={saveChannelsMutation.isPending}>
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={saveChannelsMutation.isPending || settingsQ.isLoading}
            onClick={async () => {
              try {
                await saveChannelsMutation.mutateAsync();
                setSearchSetupOpen(false);
              } catch {
                // Error alert handled by mutation + top-level notice.
              }
            }}
          >
            {saveChannelsMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showIntegrations ? (
      <Dialog open={telegramSetupOpen} onClose={() => setTelegramSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Telegram Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" color="text.secondary">
              Add your Telegram bot token and optional allowed user IDs. This controls who can use your bot.
            </Typography>
            <Stack direction="row" spacing={1} alignItems="center">
              <Chip
                size="small"
                label={channelStatusLabel(telegramConnectionStatusRaw)}
                color={channelStatusColor(telegramConnectionStatusRaw)}
                variant="outlined"
              />
              <Button size="small" onClick={() => telegramStatusQ.refetch()} disabled={telegramStatusQ.isFetching}>
                {telegramStatusQ.isFetching ? "Checking..." : "Refresh Status"}
              </Button>
            </Stack>
            {telegramConnectionDetail ? (
              <Typography variant="caption" color="text.secondary">
                {telegramConnectionDetail}
              </Typography>
            ) : null}
            <FormControlLabel
              control={
                <Switch
                  checked={channelForm.telegram_enabled}
                  onChange={(e) => setChannelField("telegram_enabled", e.target.checked)}
                />
              }
              label={channelForm.telegram_enabled ? "Telegram enabled" : "Telegram disabled"}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bot Token"
              value={channelForm.telegram_bot_token}
              onChange={(e) => setChannelField("telegram_bot_token", e.target.value)}
              placeholder={hasTelegramToken ? "Configured (leave blank to keep)" : "Enter bot token"}
              helperText={
                hasTelegramToken
                  ? "Leave blank to keep existing token."
                  : "Get token from @BotFather in Telegram."
              }
            />
            <TextField
              fullWidth
              size="small"
              label="Allowed User IDs (comma separated)"
              value={channelForm.telegram_allowed_users_csv}
              onChange={(e) => setChannelField("telegram_allowed_users_csv", e.target.value)}
              placeholder="123456789, 987654321"
              helperText="Optional, but recommended for private access control."
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setTelegramSetupOpen(false)} disabled={saveChannelsMutation.isPending}>
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={saveChannelsMutation.isPending || settingsQ.isLoading}
            onClick={async () => {
              try {
                await saveChannelsMutation.mutateAsync();
                setTelegramSetupOpen(false);
              } catch {
                // Error alert handled by mutation + top-level notice.
              }
            }}
          >
            {saveChannelsMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showIntegrations ? (
      <Dialog open={whatsAppSetupOpen} onClose={() => setWhatsAppSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>WhatsApp Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" color="text.secondary">
              Choose how you want to connect WhatsApp: quick QR pairing or Enterprise Cloud API.
            </Typography>
            <FormControlLabel
              control={
                <Switch
                  checked={channelForm.whatsapp_enabled}
                  onChange={(e) => setChannelField("whatsapp_enabled", e.target.checked)}
                />
              }
              label={channelForm.whatsapp_enabled ? "WhatsApp enabled" : "WhatsApp disabled"}
            />

            <TextField
              fullWidth
              size="small"
              select
              label="Connection Mode"
              value={channelForm.whatsapp_mode}
              onChange={(e) =>
                setChannelField("whatsapp_mode", e.target.value === "cloud_api" ? "cloud_api" : "baileys")
              }
              disabled={!channelForm.whatsapp_enabled}
            >
              <MenuItem value="baileys">Scan QR (Recommended)</MenuItem>
              <MenuItem value="cloud_api">Enterprise Cloud API</MenuItem>
            </TextField>

            {channelForm.whatsapp_enabled && channelForm.whatsapp_mode === "baileys" ? (
              <Box className="metadata-box" sx={{ maxHeight: "none", p: 1.5 }}>
                <Stack spacing={1}>
                  <Stack direction="row" spacing={1} alignItems="center">
                    <Typography variant="body2" fontWeight={700}>
                      Bridge status
                    </Typography>
                    <Chip
                      size="small"
                      label={channelStatusLabel(whatsappConnectionStatusRaw)}
                      color={channelStatusColor(whatsappConnectionStatusRaw)}
                      variant="outlined"
                    />
                  </Stack>
                  <Typography variant="caption" color="text.secondary">
                    {waBridgeQ.isLoading
                      ? "Checking WhatsApp bridge status..."
                      : waBridgeQ.error
                        ? "Bridge unreachable. Ensure bridge is running."
                        : str(waBridge.status, "disconnected") === "connected"
                          ? `Connected as ${str(waBridge.number, "-")}`
                          : str(waBridge.status, "disconnected") === "qr"
                            ? "Open WhatsApp > Linked Devices > Link a Device, then scan the QR."
                            : `Current status: ${str(waBridge.status, "disconnected")}`}
                  </Typography>
                  {str(waBridge.status, "disconnected") === "qr" ? (
                    str(waBridge.qr, "").trim() ? (
                      <Box
                        component="img"
                        src={str(waBridge.qr, "")}
                        alt="WhatsApp QR code"
                        sx={{
                          width: 220,
                          height: 220,
                          borderRadius: 1,
                          border: "1px solid rgba(108, 156, 212, 0.35)",
                          background: "#fff",
                          p: 0.75
                        }}
                      />
                    ) : (
                      <Alert severity="info" sx={{ py: 0.75 }}>
                        QR is being generated. Click Refresh if it does not appear.
                      </Alert>
                    )
                  ) : null}
                  <Stack direction="row" spacing={1}>
                    <Button
                      size="small"
                      onClick={async () => {
                        await queryClient.invalidateQueries({ queryKey: ["wa-bridge-status", "integrations-panel"] });
                      }}
                    >
                      Refresh
                    </Button>
                    <Button
                      size="small"
                      color="warning"
                      disabled={waLogoutMutation.isPending}
                      onClick={async () => {
                        if (!window.confirm("Disconnect WhatsApp and clear current pairing?")) return;
                        try {
                          await waLogoutMutation.mutateAsync();
                        } catch {
                          // Error alert handled by mutation + top-level notice.
                        }
                      }}
                    >
                      {waLogoutMutation.isPending ? "Disconnecting..." : "Disconnect"}
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            ) : null}

            {channelForm.whatsapp_enabled && channelForm.whatsapp_mode === "cloud_api" ? (
              <>
                <Stack direction="row" spacing={1} alignItems="center">
                  <Chip
                    size="small"
                    label={channelStatusLabel(whatsappConnectionStatusRaw)}
                    color={channelStatusColor(whatsappConnectionStatusRaw)}
                    variant="outlined"
                  />
                  <Typography variant="caption" color="text.secondary">
                    {whatsappConnectionDetail}
                  </Typography>
                </Stack>
                <TextField
                  fullWidth
                  size="small"
                  type="password"
                  label="Cloud API Access Token"
                  value={channelForm.whatsapp_access_token}
                  onChange={(e) => setChannelField("whatsapp_access_token", e.target.value)}
                  placeholder={hasWhatsAppToken ? "Configured (leave blank to keep)" : "Enter access token"}
                />
                <TextField
                  fullWidth
                  size="small"
                  label="Phone Number ID"
                  value={channelForm.whatsapp_phone_number_id}
                  onChange={(e) => setChannelField("whatsapp_phone_number_id", e.target.value)}
                />
                <TextField
                  fullWidth
                  size="small"
                  label="Webhook Verify Token"
                  value={channelForm.whatsapp_verify_token}
                  onChange={(e) => setChannelField("whatsapp_verify_token", e.target.value)}
                />
              </>
            ) : null}

            <TextField
              fullWidth
              size="small"
              select
              label="DM Policy"
              value={channelForm.whatsapp_dm_policy}
              onChange={(e) => setChannelField("whatsapp_dm_policy", e.target.value)}
              disabled={!channelForm.whatsapp_enabled}
            >
              <MenuItem value="pairing">pairing (recommended)</MenuItem>
              <MenuItem value="open">open</MenuItem>
            </TextField>
            <TextField
              fullWidth
              size="small"
              label="Allowed Numbers (comma separated, optional)"
              value={channelForm.whatsapp_allowed_numbers_csv}
              onChange={(e) => setChannelField("whatsapp_allowed_numbers_csv", e.target.value)}
              placeholder="+15551234567, +15557654321"
              disabled={!channelForm.whatsapp_enabled}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setWhatsAppSetupOpen(false)} disabled={saveChannelsMutation.isPending}>
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={saveChannelsMutation.isPending || settingsQ.isLoading}
            onClick={async () => {
              try {
                await saveChannelsMutation.mutateAsync();
                setWhatsAppSetupOpen(false);
              } catch {
                // Error alert handled by mutation + top-level notice.
              }
            }}
          >
            {saveChannelsMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showIntegrations ? (
      <Dialog open={!!active} onClose={closeConfig} maxWidth="sm" fullWidth>
        <DialogTitle sx={{ textTransform: "none" }}>{active?.name || "Configure integration"}</DialogTitle>
        <DialogContent>
          <Stack spacing={1.25} sx={{ pt: 1 }}>
            <Typography variant="body2" color="text.secondary">
              {active?.description}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {active?.status === "connected" ? "Connected" : "Not configured"}
            </Typography>
            {!active?.enabled ? (
              <Alert severity="info">
                This integration is disabled. Enter credentials and validate to enable it.
              </Alert>
            ) : null}
            {(active?.config_fields || []).map(renderField)}
            {active?.config_help ? (
              <Typography variant="caption" color="text.secondary">
                {active.config_help}
              </Typography>
            ) : null}
            {formError ? <Alert severity="error">{formError}</Alert> : null}
            {configSuccess ? (
              <Alert severity="success">API keys validated and saved.</Alert>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          {active?.status === "connected" ? (
            <Button
              color="warning"
              variant="outlined"
              onClick={disconnectActive}
              disabled={saving || disconnectMutation.isPending}
            >
              Disconnect
            </Button>
          ) : null}
          <Button onClick={closeConfig} disabled={saving}>
            Close
          </Button>
          <Button variant="contained" onClick={submitConfig} disabled={saving || !(active?.config_fields?.length)}>
            {saving ? "Validating..." : "Validate & Enable"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showMcp ? (
      <Dialog open={sshKeyDialogOpen} onClose={closeSshKeyDialog} maxWidth="sm" fullWidth>
        <DialogTitle>Add SSH Key</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {sshKeyError ? <Alert severity="error">{sshKeyError}</Alert> : null}
            <TextField
              fullWidth
              size="small"
              label="Key name"
              value={sshKeyName}
              onChange={(e) => setSshKeyName(e.target.value)}
              placeholder="e.g. my-server-key"
            />
            <TextField
              fullWidth
              multiline
              minRows={6}
              size="small"
              label="Private key PEM"
              placeholder={"-----BEGIN OPENSSH PRIVATE KEY-----"}
              value={sshKeyPem}
              onChange={(e) => setSshKeyPem(e.target.value)}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeSshKeyDialog}>Cancel</Button>
          <Button
            variant="contained"
            disabled={uploadSshKeyMutation.isPending || !sshKeyName.trim() || !sshKeyPem.trim()}
            onClick={() => {
              setSshKeyError(null);
              uploadSshKeyMutation.mutate();
            }}
          >
            {uploadSshKeyMutation.isPending ? "Uploading..." : "Upload Key"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showMcp ? (
      <Dialog open={sshConnDialogOpen} onClose={closeSshConnDialog} maxWidth="sm" fullWidth>
        <DialogTitle>Add SSH Connection</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {sshConnError ? <Alert severity="error">{sshConnError}</Alert> : null}
            <TextField
              fullWidth
              size="small"
              label="Connection name"
              value={sshConnName}
              onChange={(e) => setSshConnName(e.target.value)}
              placeholder="e.g. production-server"
            />
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Host"
                  value={sshHost}
                  onChange={(e) => setSshHost(e.target.value)}
                  placeholder="e.g. 192.168.1.100"
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Port"
                  value={sshPort}
                  onChange={(e) => setSshPort(e.target.value)}
                />
              </Grid2>
            </Grid2>
            <TextField
              fullWidth
              size="small"
              label="Username"
              value={sshUsername}
              onChange={(e) => setSshUsername(e.target.value)}
              placeholder="e.g. root"
            />
            <TextField
              fullWidth
              size="small"
              select
              label="SSH Key"
              value={sshConnKeyName}
              onChange={(e) => setSshConnKeyName(e.target.value)}
              helperText="Select an uploaded SSH key."
            >
              {sshKeyNames.map((k) => (
                <MenuItem key={k} value={k}>{k}</MenuItem>
              ))}
            </TextField>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeSshConnDialog}>Cancel</Button>
          <Button
            variant="contained"
            disabled={
              addSshConnectionMutation.isPending ||
              !sshConnName.trim() ||
              !sshHost.trim() ||
              !sshUsername.trim() ||
              !sshConnKeyName.trim()
            }
            onClick={() => {
              setSshConnError(null);
              addSshConnectionMutation.mutate();
            }}
          >
            {addSshConnectionMutation.isPending ? "Saving..." : "Save Connection"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showMcp ? (
      <Dialog open={sshTestDialogOpen} onClose={closeSshTestDialog} maxWidth="sm" fullWidth>
        <DialogTitle>Test SSH Connection</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <TextField
              fullWidth
              size="small"
              select
              label="Connection"
              value={sshTestConnName}
              onChange={(e) => setSshTestConnName(e.target.value)}
            >
              {sshConnectionNames.map((n) => (
                <MenuItem key={n} value={n}>{n}</MenuItem>
              ))}
            </TextField>
            {sshTestOutput ? (
              <TextField
                fullWidth
                multiline
                minRows={4}
                size="small"
                label="Test output"
                value={sshTestOutput}
                InputProps={{ readOnly: true }}
              />
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeSshTestDialog}>Close</Button>
          <Button
            variant="contained"
            disabled={testSshMutation.isPending || !sshTestConnName.trim()}
            onClick={() => {
              setSshNotice(null);
              setSshTestOutput("");
              testSshMutation.mutate();
            }}
          >
            {testSshMutation.isPending ? "Testing..." : "Run Test"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}

      {showMcp ? (
      <Dialog open={mcpDialogOpen} onClose={closeMcpDialog} maxWidth="md" fullWidth>
        <DialogTitle>{mcpEditingId ? "Edit MCP Server" : "Add MCP Server"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {mcpError ? <Alert severity="error">{mcpError}</Alert> : null}
            {mcpHasStoredAuth ? (
              <Alert severity="info">
                Stored credentials exist for this server. Leave secret fields blank to keep them, or enable clear auth.
              </Alert>
            ) : null}
            {!mcpEditingId ? (
              <TextField
                fullWidth
                size="small"
                label="Server ID (optional)"
                value={mcpForm.id}
                onChange={(e) => setMcpForm((p) => ({ ...p, id: e.target.value }))}
                helperText="If blank, an ID is auto-generated."
              />
            ) : null}
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Name"
                  value={mcpForm.name}
                  onChange={(e) => setMcpForm((p) => ({ ...p, name: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <Stack direction="row" spacing={1}>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={mcpForm.enabled}
                        onChange={(e) => setMcpForm((p) => ({ ...p, enabled: e.target.checked }))}
                      />
                    }
                    label="Enabled"
                  />
                </Stack>
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Description (optional)"
                  value={mcpForm.description}
                  onChange={(e) => setMcpForm((p) => ({ ...p, description: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  select
                  label="Transport"
                  value={mcpForm.transport_type}
                  onChange={(e) => setMcpForm((p) => ({ ...p, transport_type: e.target.value as McpTransportType }))}
                >
                  <MenuItem value="http">http</MenuItem>
                  <MenuItem value="stdio">stdio</MenuItem>
                </TextField>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={mcpForm.resources_enabled}
                      onChange={(e) => setMcpForm((p) => ({ ...p, resources_enabled: e.target.checked }))}
                    />
                  }
                  label="Enable resources"
                />
              </Grid2>
              {mcpForm.transport_type === "http" ? (
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="HTTP URL"
                    placeholder="https://example.com/mcp"
                    value={mcpForm.url}
                    onChange={(e) => setMcpForm((p) => ({ ...p, url: e.target.value }))}
                  />
                </Grid2>
              ) : (
                <>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Command"
                      placeholder="npx"
                      value={mcpForm.command}
                      onChange={(e) => setMcpForm((p) => ({ ...p, command: e.target.value }))}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Args (comma separated)"
                      placeholder="-y, @modelcontextprotocol/server-filesystem, C:\\data"
                      value={mcpForm.args_csv}
                      onChange={(e) => setMcpForm((p) => ({ ...p, args_csv: e.target.value }))}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Working directory (optional)"
                      value={mcpForm.working_dir}
                      onChange={(e) => setMcpForm((p) => ({ ...p, working_dir: e.target.value }))}
                    />
                  </Grid2>
                </>
              )}
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  select
                  label="Auth type"
                  value={mcpForm.auth_type}
                  onChange={(e) => setMcpForm((p) => ({ ...p, auth_type: e.target.value as McpAuthType }))}
                >
                  <MenuItem value="none">none</MenuItem>
                  <MenuItem value="bearer">bearer</MenuItem>
                  <MenuItem value="basic">basic</MenuItem>
                  <MenuItem value="header">header</MenuItem>
                  <MenuItem value="query">query</MenuItem>
                </TextField>
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={mcpForm.auth_clear}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_clear: e.target.checked }))}
                    />
                  }
                  label="Clear stored auth"
                />
              </Grid2>
              {mcpForm.auth_type === "bearer" ? (
                <>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Auth header"
                      value={mcpForm.auth_header}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_header: e.target.value }))}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="Bearer token (optional)"
                      value={mcpForm.auth_token}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_token: e.target.value }))}
                    />
                  </Grid2>
                </>
              ) : null}
              {mcpForm.auth_type === "basic" ? (
                <>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="Username (optional)"
                      value={mcpForm.auth_username}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_username: e.target.value }))}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="Password (optional)"
                      value={mcpForm.auth_password}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_password: e.target.value }))}
                    />
                  </Grid2>
                </>
              ) : null}
              {mcpForm.auth_type === "header" || mcpForm.auth_type === "query" ? (
                <>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label={mcpForm.auth_type === "header" ? "Header name" : "Query parameter name"}
                      value={mcpForm.auth_name}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_name: e.target.value }))}
                    />
                  </Grid2>
                  <Grid2 size={{ xs: 12, md: 6 }}>
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="Value (optional)"
                      value={mcpForm.auth_token}
                      onChange={(e) => setMcpForm((p) => ({ ...p, auth_token: e.target.value }))}
                    />
                  </Grid2>
                </>
              ) : null}
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Tool allowlist (comma separated)"
                  value={mcpForm.tool_allowlist_csv}
                  onChange={(e) => setMcpForm((p) => ({ ...p, tool_allowlist_csv: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Resource allowlist (comma separated)"
                  value={mcpForm.resource_allowlist_csv}
                  onChange={(e) => setMcpForm((p) => ({ ...p, resource_allowlist_csv: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Timeout (seconds)"
                  value={mcpForm.timeout_secs}
                  onChange={(e) => setMcpForm((p) => ({ ...p, timeout_secs: e.target.value }))}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Max response bytes"
                  value={mcpForm.max_response_bytes}
                  onChange={(e) => setMcpForm((p) => ({ ...p, max_response_bytes: e.target.value }))}
                />
              </Grid2>
            </Grid2>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeMcpDialog}>Cancel</Button>
          <Button
            variant="contained"
            disabled={saveMcpMutation.isPending}
            onClick={() => {
              setMcpError(null);
              saveMcpMutation.mutate();
            }}
          >
            {saveMcpMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </DialogActions>
      </Dialog>
      ) : null}
    </Stack>
  );
}
