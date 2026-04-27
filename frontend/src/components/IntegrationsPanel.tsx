import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import type {
  CustomMessagingChannel,
  IntegrationAuthManifestField,
  GatewayChannelDescriptor,
  IntegrationConfigField,
  IntegrationItem,
  IntegrationSyncFeedItem,
  IntegrationSyncStatus
} from "../types";
import { ExtensionPacksPanel } from "./ExtensionPacksPanel";
import { IntegrationQuickstartPanel } from "./IntegrationQuickstartPanel";
import { IntegrationRoutingPanel } from "./IntegrationRoutingPanel";
import { PluginSdkPanel } from "./PluginSdkPanel";

const REFRESH_MS = 8000;
const OAUTH_SIGNAL_STORAGE_KEY = "agentark:oauth-callback";
const OAUTH_SIGNAL_CHANNEL = "agentark-oauth";

const CHANNEL_ICON_COLORS: Record<string, string> = {
  email: "#14B8A6",
  telegram: "#26A5E4",
  whatsapp: "#25D366",
  slack: "#4A154B",
  discord: "#5865F2",
  matrix: "#0DBD8B",
  teams: "#6264A7",
  google_chat: "#34A853",
  signal: "#3A76F0",
  imessage: "#147EFB",
  line: "#06C755",
  wechat: "#07C160",
  qq: "#12B7F5",
  "web search": "#69e2ff",
  google_workspace: "#4285F4",
  github: "#f0f0f0",
  jira: "#0052CC",
  sentry: "#362D59",
  notion: "#ffffff",
  linear: "#5E6AD2",
};

// Channel & integration SVG icon imports
import iconTelegram from "../assets/icons/telegram.svg";
import iconWhatsapp from "../assets/icons/whatsapp.svg";
import iconSlack from "../assets/icons/slack.svg";
import iconDiscord from "../assets/icons/discord.svg";
import iconMatrix from "../assets/icons/matrix.svg";
import iconTeams from "../assets/icons/teams.svg";
import iconGithub from "../assets/icons/github.svg";
import iconGoogle from "../assets/icons/google.svg";
import iconWebsearch from "../assets/icons/websearch.svg";
import icon1Password from "../assets/icons/1password.svg";
import iconGarmin from "../assets/icons/garmin.svg";
import iconGoogleAnalytics from "../assets/icons/googleanalytics.svg";
import iconGoogleMaps from "../assets/icons/googlemaps.svg";
import iconGoogleSearchConsole from "../assets/icons/googlesearchconsole.svg";
import iconNotion from "../assets/icons/notion.svg";
import iconShopify from "../assets/icons/shopify.svg";
import iconTwitter from "../assets/icons/twitter.svg";
import iconJira from "../assets/icons/jira.svg";
import iconSentry from "../assets/icons/sentry.svg";
import iconLinear from "../assets/icons/linear.svg";

const CHANNEL_ICON_MAP: Record<string, string> = {
  telegram: iconTelegram,
  whatsapp: iconWhatsapp,
  slack: iconSlack,
  discord: iconDiscord,
  matrix: iconMatrix,
  teams: iconTeams,
  google_chat: iconGoogle,
  github: iconGithub,
  google: iconGoogle,
  google_workspace: iconGoogle,
  "web search": iconWebsearch,
  "1password": icon1Password,
  onepassword: icon1Password,
  garmin: iconGarmin,
  "google analytics 4": iconGoogleAnalytics,
  google_analytics: iconGoogleAnalytics,
  "google places": iconGoogleMaps,
  google_places: iconGoogleMaps,
  "google search console": iconGoogleSearchConsole,
  google_search_console: iconGoogleSearchConsole,
  notion: iconNotion,
  "ordering & purchasing": iconShopify,
  shopify: iconShopify,
  "social analytics": iconTwitter,
  twitter: iconTwitter,
  jira: iconJira,
  sentry: iconSentry,
  linear: iconLinear,
};

export function ChannelIcon({ name, size = 20 }: { name: string; size?: number }) {
  const key = name.toLowerCase();
  const iconSrc = CHANNEL_ICON_MAP[key];
  if (iconSrc) {
    return (
      <Box
        component="img"
        src={iconSrc}
        alt={name}
        sx={{
          width: size,
          height: size,
          flexShrink: 0,
          // Simple-icons SVGs are black by default; invert to white for dark theme
          filter: "brightness(0) invert(1)",
          opacity: 0.85,
        }}
      />
    );
  }
  const color = CHANNEL_ICON_COLORS[key] || "var(--ui-rgba-180-200-225-600)";
  const letter = name.charAt(0).toUpperCase();
  return (
    <Box
      component="span"
      sx={{
        width: size,
        height: size,
        borderRadius: "5px",
        background: color,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
        fontSize: size * 0.55,
        fontWeight: 800,
        color: ["#ffffff", "#f0f0f0", "#25D366", "#69e2ff", "#0DBD8B"].includes(color) ? "var(--ui-rgba-0-0-0-850)" : "#fff",
        lineHeight: 1,
      }}
    >
      {letter}
    </Box>
  );
}

function ConnectorIcon({ id, name, size = 22 }: { id: string; name: string; size?: number }) {
  const key = id.toLowerCase();
  const color = CHANNEL_ICON_COLORS[key] || "var(--ui-rgba-108-156-212-300)";
  const letter = name.charAt(0).toUpperCase();
  return (
    <Box
      component="span"
      sx={{
        width: size,
        height: size,
        borderRadius: "6px",
        background: color,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
        fontSize: size * 0.5,
        fontWeight: 800,
        color: ["#ffffff", "#f0f0f0", "#25D366", "#69e2ff", "#0DBD8B"].includes(color) ? "var(--ui-rgba-0-0-0-850)" : "#fff",
        lineHeight: 1,
      }}
    >
      {letter}
    </Box>
  );
}
const GOOGLE_WORKSPACE_BUNDLES = [
  { id: "gmail", label: "Gmail" },
  { id: "calendar", label: "Calendar" },
  { id: "drive", label: "Drive" },
  { id: "docs", label: "Docs" },
  { id: "sheets", label: "Sheets" },
  { id: "chat", label: "Chat" },
  { id: "admin", label: "Admin" }
] as const;

const EMAIL_PROVIDER_OPTIONS = [
  { value: "auto", label: "Auto" },
  { value: "gmail", label: "Gmail" },
  { value: "google_workspace", label: "Google Workspace" },
  { value: "resend", label: "Resend" },
  { value: "postmark", label: "Postmark" },
  { value: "ses", label: "Amazon SES" },
  { value: "smtp", label: "SMTP" }
] as const;

const EMAIL_TRANSPORT_OPTIONS = [
  { value: "http", label: "HTTPS API" },
  { value: "smtp", label: "SMTP" }
] as const;

const EMAIL_AUTH_OPTIONS = [
  { value: "none", label: "Provider default" },
  { value: "bearer", label: "Bearer token" },
  { value: "header", label: "Header token" },
  { value: "basic", label: "Username + password" },
  { value: "aws_sigv4", label: "AWS SigV4" }
] as const;

const EMAIL_SMTP_SECURITY_OPTIONS = [
  { value: "starttls", label: "STARTTLS" },
  { value: "tls", label: "TLS / SMTPS" },
  { value: "none", label: "None" }
] as const;

const EMAIL_BUILTIN_PROVIDERS = new Set(["gmail", "google_workspace"]);
const EMAIL_EXTERNAL_PROVIDERS = new Set(["resend", "postmark", "ses", "smtp"]);

function emailProviderLabel(value: string): string {
  const normalized = (value || "").trim().toLowerCase();
  return EMAIL_PROVIDER_OPTIONS.find((option) => option.value === normalized)?.label || value || "Email";
}

function isEmailBuiltInProvider(value: string): boolean {
  return EMAIL_BUILTIN_PROVIDERS.has((value || "").trim().toLowerCase());
}

function isEmailExternalProvider(value: string): boolean {
  return EMAIL_EXTERNAL_PROVIDERS.has((value || "").trim().toLowerCase());
}

type JsonRecord = Record<string, unknown>;
type OAuthSignalPayload = {
  type?: string;
  service_id?: string;
  integration_id?: string;
  status?: string;
  detail?: string;
};

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
  env_csv: string;
  auth_type: McpAuthType;
  auth_header: string;
  auth_name: string;
  auth_token: string;
  auth_username: string;
  auth_password: string;
  auth_clear: boolean;
  tool_allowlist_csv: string;
  tool_blocklist_csv: string;
  resource_allowlist_csv: string;
  timeout_secs: string;
  max_response_bytes: string;
};

type ChannelSettingsForm = {
  email_provider: string;
  email_to_address: string;
  email_from_address: string;
  email_domain: string;
  email_transport_kind: string;
  email_http_base_url: string;
  email_http_send_path: string;
  email_smtp_host: string;
  email_smtp_port: string;
  email_smtp_security: string;
  email_auth_kind: string;
  email_auth_api_key: string;
  email_auth_header_name: string;
  email_auth_scheme: string;
  email_auth_basic_username: string;
  email_auth_basic_password: string;
  email_auth_aws_access_key_id: string;
  email_auth_aws_secret_access_key: string;
  email_auth_aws_session_token: string;
  email_auth_aws_region: string;
  email_auth_aws_service: string;
  telegram_enabled: boolean;
  telegram_bot_token: string;
  telegram_allowed_users_csv: string;
  slack_enabled: boolean;
  slack_bot_token: string;
  slack_signing_secret: string;
  slack_api_base_url: string;
  slack_default_channel_id: string;
  slack_default_thread_ts: string;
  slack_workspace_id: string;
  slack_workspace_name: string;
  slack_trust_policy: string;
  slack_allowed_senders_csv: string;
  discord_enabled: boolean;
  discord_bot_token: string;
  discord_webhook_url: string;
  discord_api_base_url: string;
  discord_default_channel_id: string;
  discord_default_thread_id: string;
  discord_guild_id: string;
  discord_application_id: string;
  matrix_enabled: boolean;
  matrix_homeserver_url: string;
  matrix_access_token: string;
  matrix_user_id: string;
  matrix_device_id: string;
  matrix_account_id: string;
  matrix_default_room_id: string;
  matrix_sync_timeout_ms: string;
  matrix_limit: string;
  matrix_user_agent: string;
  teams_enabled: boolean;
  teams_service_url: string;
  teams_access_token: string;
  teams_bot_app_id: string;
  teams_bot_name: string;
  teams_tenant_id: string;
  teams_team_id: string;
  teams_channel_id: string;
  teams_chat_id: string;
  teams_graph_base_url: string;
  teams_delivery_mode: "auto" | "bot_framework" | "graph";
  teams_timeout_secs: string;
  teams_user_agent: string;
  teams_trust_policy: string;
  teams_allowed_senders_csv: string;
  google_chat_enabled: boolean;
  google_chat_access_token: string;
  google_chat_verify_token: string;
  google_chat_api_base_url: string;
  google_chat_space: string;
  google_chat_thread_key: string;
  google_chat_app_id: string;
  google_chat_bot_name: string;
  google_chat_trust_policy: string;
  google_chat_allowed_senders_csv: string;
  signal_enabled: boolean;
  signal_bridge_token: string;
  signal_bridge_url: string;
  signal_default_recipient: string;
  signal_default_group_id: string;
  signal_trust_policy: string;
  signal_allowed_senders_csv: string;
  imessage_enabled: boolean;
  imessage_bridge_token: string;
  imessage_bridge_url: string;
  imessage_default_chat_id: string;
  imessage_default_handle: string;
  imessage_trust_policy: string;
  imessage_allowed_senders_csv: string;
  line_enabled: boolean;
  line_channel_access_token: string;
  line_channel_secret: string;
  line_api_base_url: string;
  line_default_target: string;
  line_user_agent: string;
  line_trust_policy: string;
  line_allowed_senders_csv: string;
  wechat_enabled: boolean;
  wechat_bridge_token: string;
  wechat_bridge_url: string;
  wechat_default_target_id: string;
  wechat_trust_policy: string;
  wechat_allowed_senders_csv: string;
  qq_enabled: boolean;
  qq_bridge_token: string;
  qq_bridge_url: string;
  qq_default_target_id: string;
  qq_trust_policy: string;
  qq_allowed_senders_csv: string;
  whatsapp_enabled: boolean;
  whatsapp_mode: "baileys" | "cloud_api";
  whatsapp_bridge_runtime: "embedded" | "external";
  whatsapp_access_token: string;
  whatsapp_app_secret: string;
  whatsapp_phone_number_id: string;
  whatsapp_verify_token: string;
  whatsapp_bridge_token: string;
  whatsapp_bridge_url: string;
  whatsapp_dm_policy: string;
  whatsapp_allowed_numbers_csv: string;
};

type MessagingChannelEnabledField =
  | "telegram_enabled"
  | "slack_enabled"
  | "discord_enabled"
  | "matrix_enabled"
  | "teams_enabled"
  | "google_chat_enabled"
  | "signal_enabled"
  | "imessage_enabled"
  | "line_enabled"
  | "wechat_enabled"
  | "qq_enabled"
  | "whatsapp_enabled";

type IntegrationSyncFormState = {
  enabled: boolean;
  poll_interval_minutes: string;
  importance_threshold_percent: string;
  notify_on_important: boolean;
  push_to_preferred_channel: boolean;
};

function defaultIntegrationSyncForm(): IntegrationSyncFormState {
  return {
    enabled: true,
    poll_interval_minutes: "30",
    importance_threshold_percent: "70",
    notify_on_important: true,
    push_to_preferred_channel: false
  };
}

function integrationSyncFormFromStatus(status?: IntegrationSyncStatus | null): IntegrationSyncFormState {
  if (!status) return defaultIntegrationSyncForm();
  return {
    enabled: !!status.enabled,
    poll_interval_minutes: String(Math.max(1, Math.round((status.poll_interval_secs || 300) / 60))),
    importance_threshold_percent: String(Math.round((status.importance_threshold || 0.7) * 100)),
    notify_on_important: !!status.notify_on_important,
    push_to_preferred_channel: !!status.push_to_preferred_channel
  };
}

function formatDateTime(value?: string | null): string {
  return formatUiDateTime(value, { fallback: "Never" });
}

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

type IntegrationCardState = "enabled" | "disabled";

function integrationIsLiveConnected(integration?: IntegrationItem | null): boolean {
  return !!integration && integration.status === "connected" && integration.enabled;
}

function integrationCardState(integration: IntegrationItem): IntegrationCardState {
  return integrationIsLiveConnected(integration) ? "enabled" : "disabled";
}

function integrationCardLabel(state: IntegrationCardState): string {
  return state === "enabled" ? "Connected" : "Disabled";
}

function integrationDialogStatusLabel(integration?: IntegrationItem | null): string {
  return integrationIsLiveConnected(integration) ? "Connected" : "Disabled";
}

function integrationCardCopy(integration: IntegrationItem): string {
  if (integrationIsLiveConnected(integration)) {
    const detail = str(integration.status_detail, "").trim();
    return detail || integration.description;
  }
  if (integration.status === "connected") {
    return "Connected credentials found, but this integration is currently disabled for agent use.";
  }
  if (integration.status === "error") {
    return "Disabled until the saved credentials are fixed.";
  }
  if (integration.status === "needs_auth" || integration.status === "starting") {
    return "Disabled until sign-in is completed.";
  }
  if (integration.status === "configured") {
    return "Disabled until a live connection is confirmed.";
  }
  return "Disabled until this integration is connected.";
}

function integrationCardAccent(state: IntegrationCardState): {
  border: string;
  background: string;
  hoverBorder: string;
  hoverBackground: string;
  chipBorder: string;
  chipColor: string;
} {
  if (state === "enabled") {
    return {
      border: "var(--ui-rgba-74-210-157-350)",
      background: "var(--ui-rgba-74-210-157-060)",
      hoverBorder: "var(--ui-rgba-74-210-157-550)",
      hoverBackground: "var(--ui-rgba-74-210-157-100)",
      chipBorder: "var(--ui-rgba-74-210-157-300)",
      chipColor: "var(--ui-rgba-74-210-157-900)"
    };
  }
  return {
    border: "var(--ui-rgba-255-180-50-280)",
    background: "var(--ui-rgba-255-180-50-050)",
    hoverBorder: "var(--ui-rgba-255-180-50-440)",
    hoverBackground: "var(--ui-rgba-255-180-50-080)",
    chipBorder: "var(--ui-rgba-255-180-50-240)",
    chipColor: "var(--ui-rgba-255-196-92-920)"
  };
}

function integrationCardDotColor(state: IntegrationCardState): string {
  return state === "enabled" ? "#4ad29d" : "var(--ui-rgba-255-180-50-850)";
}

type MessagingDisplayState = "off" | "checking" | "needs_setup" | "ready" | "error";

function messagingDisplayState(status: string, enabled: boolean): MessagingDisplayState {
  if (!enabled) return "off";
  const s = (status || "").toLowerCase();
  if (s === "checking" || s === "refreshing") return "checking";
  if (s === "connected" || s === "ready" || s === "configured") return "ready";
  if (s === "error" || s === "failed" || s === "invalid_token" || s === "unavailable") return "error";
  return "needs_setup";
}

function channelStatusColor(
  status: string,
  enabled: boolean
): "success" | "warning" | "error" | "info" | "default" {
  const display = messagingDisplayState(status, enabled);
  if (display === "ready") return "success";
  if (display === "error") return "error";
  if (display === "checking") return "info";
  if (display === "needs_setup") return "warning";
  return "default";
}

function channelStatusLabel(status: string, enabled: boolean): string {
  const display = messagingDisplayState(status, enabled);
  if (display === "ready") return "Channel ready";
  if (display === "error") return "Channel error";
  if (display === "checking") return "Checking";
  if (display === "needs_setup") return "Setup needed";
  return "";
}

function messagingWizardHint(status: string, enabled: boolean): string {
  const display = messagingDisplayState(status, enabled);
  if (display === "ready") return "Configured already. Open the wizard to update credentials, defaults, or targets.";
  if (display === "error") return "Open the wizard to fix the saved settings and reconnect.";
  if (display === "checking") return "A live connectivity check is in progress.";
  if (display === "needs_setup") return "Open the wizard to finish setup and save the required connection details.";
  return "Turn it on and complete the wizard to connect this channel.";
}

function gatewayChannelById(
  channels: GatewayChannelDescriptor[],
  id: string
): GatewayChannelDescriptor | null {
  return channels.find((channel) => channel.id === id) || null;
}

function gatewayChannelDetail(channel: GatewayChannelDescriptor | null, fallback: string): string {
  if (!channel) return fallback;
  if (str(channel.last_error, "").trim()) return str(channel.last_error, "");
  if (!channel.configured) return fallback;
  const parts: string[] = [];
  if ((channel.connected_account_count || 0) > 0) {
    parts.push(`${channel.connected_account_count} live account${channel.connected_account_count === 1 ? "" : "s"}`);
  } else if ((channel.account_count || 0) > 0) {
    parts.push(`${channel.account_count} account${channel.account_count === 1 ? "" : "s"} configured`);
  } else {
    parts.push("Configuration saved");
  }
  if ((channel.route_count || 0) > 0) {
    parts.push(`${channel.route_count} route${channel.route_count === 1 ? "" : "s"}`);
  }
  if (str(channel.delivery_mode, "").trim()) {
    parts.push(str(channel.delivery_mode, "").replace(/_/g, " "));
  }
  return parts.join(" | ");
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

function customMessagingCredentialFields(channel?: CustomMessagingChannel | null): IntegrationAuthManifestField[] {
  const mode = channel?.auth_manifest?.mode;
  if (!mode || typeof mode !== "object") return [];
  const fields = (mode as { fields?: unknown }).fields;
  return Array.isArray(fields)
    ? fields.filter((field): field is IntegrationAuthManifestField => isRecord(field) && typeof field.key === "string")
    : [];
}

function authFieldInputKind(field: IntegrationAuthManifestField): "text" | "password" | "textarea" {
  const input = field.input_type;
  const raw =
    typeof input === "string"
      ? input
      : input && typeof input === "object"
        ? str((input as Record<string, unknown>).kind, "")
        : "";
  const normalized = raw.trim().toLowerCase();
  if (normalized === "text" || normalized === "textarea") return normalized;
  return "password";
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

function parseOAuthSignalPayload(value: unknown): OAuthSignalPayload | null {
  if (typeof value === "string") {
    try {
      return parseOAuthSignalPayload(JSON.parse(value));
    } catch {
      return null;
    }
  }
  if (!isRecord(value)) return null;
  const payload = value as OAuthSignalPayload;
  const targetId = str(payload.integration_id ?? payload.service_id, "").trim();
  if (!targetId) return null;
  return payload;
}

function normalizeIntegrationId(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "calendar") return "google_calendar";
  return normalized;
}

function integrationDisplayName(id: string, integrations: IntegrationItem[]): string {
  const normalized = normalizeIntegrationId(id);
  return integrations.find((item) => item.id === normalized)?.name || normalized || "Integration";
}

function summarizeInlineNames(names: string[], emptyText: string): string {
  const cleaned = names.map((value) => value.trim()).filter(Boolean);
  if (cleaned.length === 0) return emptyText;
  const preview = cleaned.slice(0, 3).join(", ");
  return cleaned.length > 3 ? `${preview} +${cleaned.length - 3} more` : preview;
}

function parseWorkspaceBundleCsv(input: string): string[] {
  const seen = new Set<string>();
  for (const raw of (input || "").split(/[,\n\r;]+/g)) {
    const normalized = raw.trim().toLowerCase();
    if (!normalized) continue;
    const match = GOOGLE_WORKSPACE_BUNDLES.find((bundle) => bundle.id === normalized);
    if (match) seen.add(match.id);
  }
  return Array.from(seen.values());
}

function workspaceBundleCsv(input: string[]): string {
  return input.join(", ");
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
    env_csv: "",
    auth_type: "none",
    auth_header: "Authorization",
    auth_name: "",
    auth_token: "",
    auth_username: "",
    auth_password: "",
    auth_clear: false,
    tool_allowlist_csv: "",
    tool_blocklist_csv: "",
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
  mode?: "all" | "integrations" | "messaging" | "mcp" | "channels" | "connectors";
}) {
  const queryClient = useQueryClient();
  const showIntegrations = mode !== "mcp";
  const showCatalog = mode === "all" || mode === "integrations";
  const showMessagingOnly = mode === "messaging" || mode === "channels";
  const showMcp = mode === "all" || mode === "mcp";
  const showChannelsPage = mode === "channels";
  const showConnectorsPage = mode === "connectors";
  const shouldLoadConnectorCatalog = showCatalog || showConnectorsPage;
  const [active, setActive] = useState<IntegrationItem | null>(null);
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [googleWorkspaceHelpOpen, setGoogleWorkspaceHelpOpen] = useState(false);
  const [editingConnected, setEditingConnected] = useState(false);
  const [syncForm, setSyncForm] = useState<IntegrationSyncFormState>(defaultIntegrationSyncForm());
  const [syncDirty, setSyncDirty] = useState(false);
  const [syncExpanded, setSyncExpanded] = useState(false);
  const [syncNotice, setSyncNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
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
  const [expandedSection, setExpandedSection] = useState<string | false>("connected");
  const [sshKeyError, setSshKeyError] = useState<string | null>(null);
  const [sshConnError, setSshConnError] = useState<string | null>(null);
  const [showDisabledIntegrations, setShowDisabledIntegrations] = useState(false);
  const [oauthBusyId, setOauthBusyId] = useState<string | null>(null);
  const [oauthPendingId, setOauthPendingId] = useState<string | null>(null);
  const [customMessagingCredentialTarget, setCustomMessagingCredentialTarget] =
    useState<CustomMessagingChannel | null>(null);
  const [customMessagingCredentialValues, setCustomMessagingCredentialValues] =
    useState<Record<string, string>>({});
  const [channelsDirty, setChannelsDirty] = useState(false);
  const [emailSetupOpen, setEmailSetupOpen] = useState(false);
  const [telegramSetupOpen, setTelegramSetupOpen] = useState(false);
  const [slackSetupOpen, setSlackSetupOpen] = useState(false);
  const [discordSetupOpen, setDiscordSetupOpen] = useState(false);
  const [matrixSetupOpen, setMatrixSetupOpen] = useState(false);
  const [teamsSetupOpen, setTeamsSetupOpen] = useState(false);
  const [whatsAppSetupOpen, setWhatsAppSetupOpen] = useState(false);
  const [googleChatSetupOpen, setGoogleChatSetupOpen] = useState(false);
  const [signalSetupOpen, setSignalSetupOpen] = useState(false);
  const [imessageSetupOpen, setImessageSetupOpen] = useState(false);
  const [lineSetupOpen, setLineSetupOpen] = useState(false);
  const [wechatSetupOpen, setWechatSetupOpen] = useState(false);
  const [qqSetupOpen, setQqSetupOpen] = useState(false);
  const [channelForm, setChannelForm] = useState<ChannelSettingsForm>({
    email_provider: "auto",
    email_to_address: "",
    email_from_address: "",
    email_domain: "",
    email_transport_kind: "http",
    email_http_base_url: "",
    email_http_send_path: "",
    email_smtp_host: "",
    email_smtp_port: "587",
    email_smtp_security: "starttls",
    email_auth_kind: "none",
    email_auth_api_key: "",
    email_auth_header_name: "",
    email_auth_scheme: "",
    email_auth_basic_username: "",
    email_auth_basic_password: "",
    email_auth_aws_access_key_id: "",
    email_auth_aws_secret_access_key: "",
    email_auth_aws_session_token: "",
    email_auth_aws_region: "",
    email_auth_aws_service: "ses",
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
    slack_trust_policy: "open",
    slack_allowed_senders_csv: "",
    discord_enabled: false,
    discord_bot_token: "",
    discord_webhook_url: "",
    discord_api_base_url: "https://discord.com/api/v10",
    discord_default_channel_id: "",
    discord_default_thread_id: "",
    discord_guild_id: "",
    discord_application_id: "",
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
    teams_trust_policy: "open",
    teams_allowed_senders_csv: "",
    google_chat_enabled: false,
    google_chat_access_token: "",
    google_chat_verify_token: "",
    google_chat_api_base_url: "https://chat.googleapis.com",
    google_chat_space: "",
    google_chat_thread_key: "",
    google_chat_app_id: "",
    google_chat_bot_name: "",
    google_chat_trust_policy: "open",
    google_chat_allowed_senders_csv: "",
    signal_enabled: false,
    signal_bridge_token: "",
    signal_bridge_url: "http://127.0.0.1:9120",
    signal_default_recipient: "",
    signal_default_group_id: "",
    signal_trust_policy: "open",
    signal_allowed_senders_csv: "",
    imessage_enabled: false,
    imessage_bridge_token: "",
    imessage_bridge_url: "http://127.0.0.1:9130",
    imessage_default_chat_id: "",
    imessage_default_handle: "",
    imessage_trust_policy: "open",
    imessage_allowed_senders_csv: "",
    line_enabled: false,
    line_channel_access_token: "",
    line_channel_secret: "",
    line_api_base_url: "https://api.line.me",
    line_default_target: "",
    line_user_agent: "",
    line_trust_policy: "open",
    line_allowed_senders_csv: "",
    wechat_enabled: false,
    wechat_bridge_token: "",
    wechat_bridge_url: "http://127.0.0.1:9140",
    wechat_default_target_id: "",
    wechat_trust_policy: "open",
    wechat_allowed_senders_csv: "",
    qq_enabled: false,
    qq_bridge_token: "",
    qq_bridge_url: "http://127.0.0.1:9150",
    qq_default_target_id: "",
    qq_trust_policy: "open",
    qq_allowed_senders_csv: "",
    whatsapp_enabled: false,
    whatsapp_mode: "baileys",
    whatsapp_bridge_runtime: "embedded",
    whatsapp_access_token: "",
    whatsapp_app_secret: "",
    whatsapp_phone_number_id: "",
    whatsapp_verify_token: "",
    whatsapp_bridge_token: "",
    whatsapp_bridge_url: "",
    whatsapp_dm_policy: "pairing",
    whatsapp_allowed_numbers_csv: ""
  });
  // NOTE: Integrations are long-lived connectors. URL imports belong to Skills.

  const integrationsQ = useQuery({
    queryKey: ["integrations"],
    queryFn: api.getIntegrations,
    refetchInterval: REFRESH_MS,
    enabled: shouldLoadConnectorCatalog,
    placeholderData: (previous) => previous
  });
  const integrationSyncStatusQ = useQuery({
    queryKey: ["integration-sync-status"],
    queryFn: api.getIntegrationSyncStatus,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showCatalog
  });
  const integrationSyncFeedQ = useQuery({
    queryKey: ["integration-sync-feed"],
    queryFn: () => api.getIntegrationSyncFeed({ limit: 18 }),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showCatalog
  });
  const webhookSourcesQ = useQuery({
    queryKey: ["integrations-quickstart-webhooks"],
    queryFn: () => api.rawGet("/webhooks/sources"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showCatalog
  });
  const customApisQ = useQuery({
    queryKey: ["integrations-quickstart-custom-apis"],
    queryFn: () => api.rawGet("/custom-apis"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showCatalog
  });
  const pluginsSummaryQ = useQuery({
    queryKey: ["settings-plugins"],
    queryFn: () => api.rawGet("/plugins"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showCatalog
  });
  const settingsQ = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.rawGet("/settings"),
    refetchInterval: false,
    refetchOnWindowFocus: false,
    enabled: showIntegrations
  });
  const channelsQ = useQuery({
    queryKey: ["gateway-channels-integrations"],
    queryFn: api.getChannels,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showIntegrations
  });
  const customMessagingChannelsQ = useQuery({
    queryKey: ["custom-messaging-channels"],
    queryFn: api.listCustomMessagingChannels,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
    enabled: showChannelsPage
  });
  const senderVerificationQ = useQuery({
    queryKey: ["settings-sender-verification-inline"],
    queryFn: () => api.rawGet("/sender-verification"),
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
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-feed"] })
      ]);
    }
  });

  const disconnectMutation = useMutation({
    mutationFn: (id: string) => api.disconnectIntegration(id),
    onSuccess: async () => {
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-feed"] })
      ]);
    }
  });

  const enableMutation = useMutation({
    mutationFn: (id: string) => api.enableIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Integration enabled." });
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] })
      ]);
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => api.disableIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Integration disabled." });
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] })
      ]);
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const testMutation = useMutation({
    mutationFn: (id: string) => api.testIntegration(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Connection test passed." });
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] })
      ]);
    },
    onError: async (err) => {
      // Backend may auto-disable on failed test.
      setNotice({ kind: "error", text: asErrorMessage(err) });
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integrations"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] })
      ]);
    }
  });
  const saveSyncMutation = useMutation({
    mutationFn: ({ id, payload }: { id: string; payload: Record<string, unknown> }) =>
      api.updateIntegrationSync(id, payload),
    onSuccess: async () => {
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-feed"] })
      ]);
    }
  });
  const syncNowMutation = useMutation({
    mutationFn: (id: string) => api.runIntegrationSyncNow(id),
    onSuccess: async () => {
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] }),
        queryClient.invalidateQueries({ queryKey: ["integration-sync-feed"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications-count"] })
      ]);
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
  const approveSenderMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.rawPost("/sender-verification/approve", payload),
    onSuccess: async () => {
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["settings-sender-verification-inline"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications-count"] })
      ]);
    }
  });
  const revokeSenderMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.rawPost("/sender-verification/revoke", payload),
    onSuccess: async () => {
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["settings-sender-verification-inline"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications"] }),
        queryClient.invalidateQueries({ queryKey: ["notifications-count"] })
      ]);
    }
  });
  const saveCustomMessagingCredentialsMutation = useMutation({
    mutationFn: ({ id, values }: { id: string; values: Record<string, string> }) =>
      api.storeCustomMessagingChannelCredentials(id, values),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Custom channel credentials saved." });
      setCustomMessagingCredentialTarget(null);
      setCustomMessagingCredentialValues({});
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["custom-messaging-channels"] }),
        queryClient.invalidateQueries({ queryKey: ["gateway-channels-integrations"] })
      ]);
    },
    onError: (err) => setNotice({ kind: "error", text: asErrorMessage(err) })
  });
  const testCustomMessagingChannelMutation = useMutation({
    mutationFn: (id: string) => api.testCustomMessagingChannel(id),
    onSuccess: async (payload) => {
      const detail = payload.result?.detail || "Test completed.";
      setNotice({
        kind: payload.result?.ok ? "success" : "error",
        text: detail
      });
      await queryClient.invalidateQueries({ queryKey: ["custom-messaging-channels"] });
    },
    onError: (err) => setNotice({ kind: "error", text: asErrorMessage(err) })
  });
  const deleteCustomMessagingChannelMutation = useMutation({
    mutationFn: (id: string) => api.deleteCustomMessagingChannel(id),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Custom messaging channel removed." });
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["custom-messaging-channels"] }),
        queryClient.invalidateQueries({ queryKey: ["gateway-channels-integrations"] })
      ]);
    },
    onError: (err) => setNotice({ kind: "error", text: asErrorMessage(err) })
  });

  const integrations = shouldLoadConnectorCatalog
    ? (integrationsQ.data?.integrations || []).filter(
        (integration) => normalizeIntegrationId(integration.id) !== "moltbook",
      )
    : [];
  const integrationSyncStatuses = showCatalog ? integrationSyncStatusQ.data?.statuses || [] : [];
  const integrationSyncStatusById = useMemo(
    () =>
      Object.fromEntries(
        integrationSyncStatuses.map((status) => [status.integration_id, status] as const)
      ) as Record<string, IntegrationSyncStatus>,
    [integrationSyncStatuses]
  );
  const integrationSyncFeed = showCatalog ? integrationSyncFeedQ.data?.items || [] : [];
  const settings = asRecord(settingsQ.data);
  const emailSettings = asRecord(settings.email);
  const emailAuthSettings = asRecord(emailSettings.auth);
  const senderVerification = asRecord(senderVerificationQ.data);
  const senderVerificationSettings = asRecord(senderVerification.settings);
  const googleChatTrustSettings = asRecord(senderVerificationSettings.google_chat);
  const signalTrustSettings = asRecord(senderVerificationSettings.signal);
  const imessageTrustSettings = asRecord(senderVerificationSettings.imessage);
  const lineTrustSettings = asRecord(senderVerificationSettings.line);
  const slackTrustSettings = asRecord(senderVerificationSettings.slack);
  const teamsTrustSettings = asRecord(senderVerificationSettings.teams);
  const wechatTrustSettings = asRecord(senderVerificationSettings.wechat);
  const qqTrustSettings = asRecord(senderVerificationSettings.qq);
  const whatsappTrustSettings = asRecord(senderVerificationSettings.whatsapp);
  const pendingSenderRows = asRecords(senderVerification.pending);
  const approvedSenderRows = asRecords(senderVerification.approved);
  const gatewayChannels = showIntegrations ? channelsQ.data?.channels || [] : [];
  const waBridge = asRecord(waBridgeQ.data);
  const telegramStatus = asRecord(telegramStatusQ.data);
  const emailProviderSaved = str(emailSettings.provider, "auto").trim().toLowerCase() || "auto";
  const emailAvailableBackends = Array.from(new Set(
    asStringList(emailSettings.available_backends).map((backend) => backend.trim().toLowerCase()).filter(Boolean)
  ));
  const emailAvailableBackendLabels = emailAvailableBackends.map((backend) => emailProviderLabel(backend));
  const emailAutoResolvesTo = str(emailSettings.auto_resolves_to, "").trim().toLowerCase();
  const emailDeliveryReady = toBool(emailSettings.delivery_ready);
  const hasEmailApiKey = toBool(emailAuthSettings.has_api_key);
  const hasEmailBasicPassword = toBool(emailAuthSettings.has_basic_password);
  const hasEmailAwsSecretAccessKey = toBool(emailAuthSettings.has_aws_secret_access_key);
  const hasEmailAwsSessionToken = toBool(emailAuthSettings.has_aws_session_token);
  const telegramEnabledSaved = toBool(settings.telegram_enabled);
  const hasTelegramToken = toBool(settings.has_telegram_token);
  const telegramDeliveryReady = toBool(settings.telegram_delivery_ready);
  const hasSlackBotToken = toBool(settings.has_slack_bot_token);
  const hasSlackSigningSecret = toBool(settings.has_slack_signing_secret);
  const hasDiscordBotToken = toBool(settings.has_discord_bot_token);
  const hasMatrixAccessToken = toBool(settings.has_matrix_access_token);
  const hasTeamsAccessToken = toBool(settings.has_teams_access_token);
  const hasWhatsAppToken = toBool(settings.has_whatsapp_token);
  const hasWhatsAppAppSecret = toBool(settings.has_whatsapp_app_secret);
  const hasWhatsAppVerifyToken = toBool(settings.has_whatsapp_verify_token);
  const hasWhatsAppBridgeToken = toBool(settings.has_whatsapp_bridge_token);
  const hasGoogleChatAccessToken = toBool(settings.has_google_chat_access_token);
  const hasGoogleChatVerifyToken = toBool(settings.has_google_chat_verify_token);
  const hasSignalBridgeToken = toBool(settings.has_signal_bridge_token);
  const hasIMessageBridgeToken = toBool(settings.has_imessage_bridge_token);
  const hasLineAccessToken = toBool(settings.has_line_access_token);
  const hasLineChannelSecret = toBool(settings.has_line_channel_secret);
  const hasWeChatBridgeToken = toBool(settings.has_wechat_bridge_token);
  const hasQqBridgeToken = toBool(settings.has_qq_bridge_token);
  const emailSelectedProvider = (channelForm.email_provider || emailProviderSaved || "auto").trim().toLowerCase() || "auto";
  const emailUsesBuiltInProvider = isEmailBuiltInProvider(emailSelectedProvider);
  const emailUsesExternalProvider = isEmailExternalProvider(emailSelectedProvider);
  const emailSelectedTransportKind =
    (channelForm.email_transport_kind || "http").trim().toLowerCase() === "smtp" ? "smtp" : "http";
  const emailSelectedAuthKind = (channelForm.email_auth_kind || "none").trim().toLowerCase() || "none";
  const emailUsesSmtpTransport =
    emailSelectedProvider === "smtp" || (emailSelectedProvider === "ses" && emailSelectedTransportKind === "smtp");
  const emailRecipientConfigured = channelForm.email_to_address.trim().length > 0;
  const emailFromConfigured = channelForm.email_from_address.trim().length > 0;
  const emailApiKeyConfigured = hasEmailApiKey || channelForm.email_auth_api_key.trim().length > 0;
  const emailBasicPasswordConfigured =
    hasEmailBasicPassword || channelForm.email_auth_basic_password.trim().length > 0;
  const emailAwsSecretConfigured =
    hasEmailAwsSecretAccessKey || channelForm.email_auth_aws_secret_access_key.trim().length > 0;
  const emailBuiltInMailboxFallbackCount = emailAvailableBackends.filter((backend) => EMAIL_BUILTIN_PROVIDERS.has(backend)).length;
  const emailRecipientFallbackReady =
    emailBuiltInMailboxFallbackCount === 1 || (emailSelectedProvider === "auto" && EMAIL_BUILTIN_PROVIDERS.has(emailAutoResolvesTo));
  const emailRecipientReady =
    emailRecipientConfigured || emailUsesBuiltInProvider || emailRecipientFallbackReady;
  const emailDraftIssues = (() => {
    const issues: string[] = [];
    if (emailSelectedProvider === "auto") {
      if (emailAvailableBackends.length === 0) {
        issues.push("Connect Gmail or Google Workspace, or configure Resend, Postmark, SES, or SMTP.");
      } else if (emailAvailableBackends.length > 1) {
        issues.push("Auto cannot choose between multiple ready backends. Pick a provider explicitly.");
      }
      return issues;
    }
    if (emailUsesBuiltInProvider) {
      if (!emailAvailableBackends.includes(emailSelectedProvider)) {
        issues.push(`Connect ${emailProviderLabel(emailSelectedProvider)} in Integrations first.`);
      }
      return issues;
    }
    if (!emailFromConfigured) {
      issues.push("Add a From address on the domain you want AgentArk to send from.");
    }
    if (emailSelectedProvider === "resend" || emailSelectedProvider === "postmark") {
      if (!emailApiKeyConfigured) {
        issues.push(`Add the ${emailProviderLabel(emailSelectedProvider)} API key.`);
      }
    } else if (emailSelectedProvider === "ses") {
      if (emailUsesSmtpTransport) {
        if (!channelForm.email_smtp_host.trim()) {
          issues.push("Add the SES SMTP host.");
        }
        if (!channelForm.email_auth_basic_username.trim()) {
          issues.push("Add the SES SMTP username.");
        }
        if (!emailBasicPasswordConfigured) {
          issues.push("Add the SES SMTP password.");
        }
      } else {
        if (!channelForm.email_auth_aws_access_key_id.trim()) {
          issues.push("Add the AWS access key ID.");
        }
        if (!emailAwsSecretConfigured) {
          issues.push("Add the AWS secret access key.");
        }
        if (!channelForm.email_auth_aws_region.trim()) {
          issues.push("Add the AWS region.");
        }
      }
    } else if (emailSelectedProvider === "smtp") {
      if (!channelForm.email_smtp_host.trim()) {
        issues.push("Add the SMTP host.");
      }
      if (!channelForm.email_auth_basic_username.trim()) {
        issues.push("Add the SMTP username.");
      }
      if (!emailBasicPasswordConfigured) {
        issues.push("Add the SMTP password.");
      }
    }
    if (emailUsesExternalProvider && !emailRecipientReady) {
      issues.push("Set a recipient email, or keep exactly one connected Google mailbox available for fallback delivery.");
    }
    return issues;
  })();
  const telegramDraftTokenConfigured = channelForm.telegram_bot_token.trim().length > 0;
  const telegramTokenConfigured = hasTelegramToken || telegramDraftTokenConfigured;
  const whatsappTokenConfigured = hasWhatsAppToken || channelForm.whatsapp_access_token.trim().length > 0;
  const whatsappAppSecretConfigured =
    hasWhatsAppAppSecret || channelForm.whatsapp_app_secret.trim().length > 0;
  const whatsappVerifyTokenConfigured =
    hasWhatsAppVerifyToken || channelForm.whatsapp_verify_token.trim().length > 0;
  const whatsappExternalBridgeTokenConfigured =
    hasWhatsAppBridgeToken || channelForm.whatsapp_bridge_token.trim().length > 0;
  const whatsappEmbeddedBridgeSelected =
    channelForm.whatsapp_mode === "baileys" && channelForm.whatsapp_bridge_runtime === "embedded";
  const whatsappExternalBridgeSelected =
    channelForm.whatsapp_mode === "baileys" && channelForm.whatsapp_bridge_runtime === "external";
  const whatsappModeSummary =
    channelForm.whatsapp_mode === "cloud_api"
      ? "Cloud API"
      : whatsappExternalBridgeSelected
        ? "External bridge"
        : "Bundled bridge";
  const whatsappBridgeStatus = str(waBridge.status, "disconnected");
  const whatsappBridgeDetail = str(waBridge.detail, "");
  const whatsappBridgeError = str(waBridge.error, "");
  const whatsappBridgeWarning = str(waBridge.warning, "");
  const whatsappBridgeNumber = str(waBridge.number, "").trim();
  const whatsappBridgeInstalled = waBridge.installed !== false;
  const telegramProbeStatus = str(telegramStatus.status, "").trim().toLowerCase();
  const telegramProbeDetail = str(telegramStatus.detail, "").trim();
  const emailStatusLabel = emailDeliveryReady ? "Delivery ready" : "Setup needed";
  const emailStatusColor = emailDeliveryReady
    ? "success"
    : emailDraftIssues.length > 0
      ? "warning"
      : "info";
  const emailConnectionDetail = (() => {
    if (emailDeliveryReady) {
      const activeProviderLabel =
        emailProviderSaved === "auto" && emailAutoResolvesTo
          ? `Auto -> ${emailProviderLabel(emailAutoResolvesTo)}`
          : emailProviderLabel(emailProviderSaved);
      const recipientText = emailRecipientConfigured
        ? `Recipient ${channelForm.email_to_address.trim()}.`
        : emailUsesBuiltInProvider || emailRecipientFallbackReady
          ? "Recipient falls back to the connected Google mailbox."
          : "Set a recipient to choose where notification emails land.";
      const fromText = channelForm.email_from_address.trim()
        ? `From ${channelForm.email_from_address.trim()}.`
        : "";
      return [activeProviderLabel, "is ready.", recipientText, fromText].filter(Boolean).join(" ");
    }
    if (emailDraftIssues.length > 0) {
      return emailDraftIssues[0];
    }
    if (emailAvailableBackendLabels.length > 0) {
      return `Ready backends: ${summarizeInlineNames(emailAvailableBackendLabels, "No ready backends yet")}. Save to apply your selection.`;
    }
    return "Connect Gmail or Google Workspace, or configure a provider account you control.";
  })();
  const emailProviderHelperText = (() => {
    if (emailSelectedProvider === "auto" && emailAvailableBackends.length === 1) {
      return `Auto currently resolves to ${emailProviderLabel(emailAvailableBackends[0])}.`;
    }
    if (emailSelectedProvider === "auto" && emailAvailableBackends.length > 1) {
      return "More than one backend is ready. Pick one explicitly so AgentArk knows where to send.";
    }
    if (emailSelectedProvider === "gmail" || emailSelectedProvider === "google_workspace") {
      return `Use the connected ${emailProviderLabel(emailSelectedProvider)} mailbox for delivery.`;
    }
    return "Use a provider account you control for custom domains and branded email delivery.";
  })();
  const emailRecipientHelperText = emailRecipientConfigured
    ? "Notification emails will be delivered to this address."
    : emailUsesBuiltInProvider || emailRecipientFallbackReady
      ? "Leave blank when the connected Gmail or Google Workspace mailbox should receive the email."
      : "Set this for provider accounts you control. Leave blank only when one connected Google mailbox should receive the email.";
  const telegramConnectionStatusRaw = (() => {
    if (!telegramEnabledSaved) return "disabled";
    if (!hasTelegramToken) return "missing_token";
    if (!telegramProbeStatus) return "configured";
    if (telegramProbeStatus === "error") {
      const detail = telegramProbeDetail.toLowerCase();
      const likelyCredentialError =
        detail.includes("unauthorized") ||
        detail.includes("forbidden") ||
        detail.includes("invalid") ||
        detail.includes("not found") ||
        detail.includes("bot token");
      return likelyCredentialError ? "error" : "configured";
    }
    return telegramProbeStatus;
  })();
  const telegramConnectionDetail = (() => {
    if (!telegramEnabledSaved) return "Telegram is disabled.";
    if (!hasTelegramToken) {
      return telegramDraftTokenConfigured
        ? "Bot token entered locally. Save changes to apply it."
        : "Telegram bot token is not configured.";
    }
    if (telegramConnectionStatusRaw === "connected" && telegramProbeDetail) {
      return telegramProbeDetail;
    }
    if (telegramProbeStatus === "error" && telegramConnectionStatusRaw === "configured") {
      return telegramProbeDetail
        ? `Saved bot token is configured. Last live check failed: ${telegramProbeDetail}`
        : "Saved bot token is configured.";
    }
    if (telegramProbeDetail) return telegramProbeDetail;
    return telegramDeliveryReady
      ? "Saved bot token and delivery routing are configured."
      : "Saved bot token is configured.";
  })();
  const whatsappConnectionStatusRaw = (() => {
    if (!channelForm.whatsapp_enabled) return "disabled";
    if (channelForm.whatsapp_mode === "cloud_api") {
      return whatsappTokenConfigured &&
        whatsappAppSecretConfigured &&
        whatsappVerifyTokenConfigured &&
        channelForm.whatsapp_phone_number_id.trim()
        ? "ready"
        : "missing_config";
    }
    if (whatsappExternalBridgeSelected && !channelForm.whatsapp_bridge_url.trim()) {
      return "missing_config";
    }
    if (waBridgeQ.isFetching) return "checking";
    if (waBridgeQ.error) return "error";
    return whatsappBridgeStatus === "disabled" ? "missing_config" : whatsappBridgeStatus;
  })();
  const whatsappConnectionDetail = (() => {
    if (!channelForm.whatsapp_enabled) return "WhatsApp is disabled.";
    if (channelForm.whatsapp_mode === "cloud_api") {
      return whatsappConnectionStatusRaw === "ready"
        ? "Cloud API credentials are configured."
        : "Cloud API token, app secret, verify token, and phone number ID are required.";
    }
    if (whatsappExternalBridgeSelected) {
      if (!channelForm.whatsapp_bridge_url.trim()) {
        return "External bridge URL is required.";
      }
      if (!whatsappExternalBridgeTokenConfigured) {
        return "External bridge token is required for new setups. Leave the field blank only when keeping a saved token or preserving a legacy tokenless bridge.";
      }
      if (waBridgeQ.isFetching) return "Checking external bridge status...";
      if (waBridgeQ.error) return "External bridge status check failed.";
      if (whatsappBridgeStatus === "connected" && whatsappBridgeNumber) {
        return `External bridge connected as ${whatsappBridgeNumber}.`;
      }
      if (whatsappBridgeDetail) return whatsappBridgeDetail;
      if (whatsappBridgeError) return whatsappBridgeError;
      if (whatsappBridgeStatus === "connected") return "External bridge is connected.";
      if (whatsappBridgeWarning) return whatsappBridgeWarning;
      return "Save the external bridge URL and token, then make sure that bridge is reachable from AgentArk.";
    }
    if (waBridgeQ.isFetching) return "Checking bundled WhatsApp bridge...";
    if (!whatsappBridgeInstalled) {
      return whatsappBridgeError ||
        "Bundled WhatsApp bridge is not installed in this image. Use the full image or switch to an external bridge.";
    }
    if (waBridgeQ.error) return "Bundled WhatsApp bridge status check failed.";
    if (whatsappBridgeStatus === "connected" && whatsappBridgeNumber) {
      return `Bundled bridge connected as ${whatsappBridgeNumber}.`;
    }
    if (whatsappBridgeStatus === "qr") {
      return "Open WhatsApp > Linked Devices > Link a Device, then scan the QR.";
    }
    if (whatsappBridgeDetail) return whatsappBridgeDetail;
    if (whatsappBridgeError) return whatsappBridgeError;
    return "Save settings, then use the bundled bridge to pair with a QR code.";
  })();
  const slackGateway = gatewayChannelById(gatewayChannels, "slack");
  const discordGateway = gatewayChannelById(gatewayChannels, "discord");
  const matrixGateway = gatewayChannelById(gatewayChannels, "matrix");
  const teamsGateway = gatewayChannelById(gatewayChannels, "teams");
  const googleChatGateway = gatewayChannelById(gatewayChannels, "google_chat");
  const signalGateway = gatewayChannelById(gatewayChannels, "signal");
  const imessageGateway = gatewayChannelById(gatewayChannels, "imessage");
  const lineGateway = gatewayChannelById(gatewayChannels, "line");
  const wechatGateway = gatewayChannelById(gatewayChannels, "wechat");
  const qqGateway = gatewayChannelById(gatewayChannels, "qq");
  const slackConnectionStatusRaw = slackGateway?.status || (channelForm.slack_enabled ? "missing_config" : "disabled");
  const discordConnectionStatusRaw =
    discordGateway?.status || (channelForm.discord_enabled ? "missing_config" : "disabled");
  const matrixConnectionStatusRaw =
    matrixGateway?.status || (channelForm.matrix_enabled ? "missing_config" : "disabled");
  const teamsConnectionStatusRaw =
    teamsGateway?.status || (channelForm.teams_enabled ? "missing_config" : "disabled");
  const googleChatConnectionStatusRaw =
    googleChatGateway?.status || (channelForm.google_chat_enabled ? "missing_config" : "disabled");
  const signalConnectionStatusRaw =
    signalGateway?.status || (channelForm.signal_enabled ? "missing_config" : "disabled");
  const imessageConnectionStatusRaw =
    imessageGateway?.status || (channelForm.imessage_enabled ? "missing_config" : "disabled");
  const lineConnectionStatusRaw =
    lineGateway?.status || (channelForm.line_enabled ? "missing_config" : "disabled");
  const wechatConnectionStatusRaw =
    wechatGateway?.status || (channelForm.wechat_enabled ? "missing_config" : "disabled");
  const qqConnectionStatusRaw =
    qqGateway?.status || (channelForm.qq_enabled ? "missing_config" : "disabled");
  const slackConnectionDetail = gatewayChannelDetail(
    slackGateway,
    channelForm.slack_enabled ? "Bot token and signing secret are required." : "Not configured"
  );
  const discordConnectionDetail = gatewayChannelDetail(
    discordGateway,
    channelForm.discord_enabled ? "Bot token or webhook URL is required." : "Not configured"
  );
  const matrixConnectionDetail = gatewayChannelDetail(
    matrixGateway,
    channelForm.matrix_enabled ? "Homeserver URL, access token, and user ID are required." : "Not configured"
  );
  const teamsConnectionDetail = gatewayChannelDetail(
    teamsGateway,
    channelForm.teams_enabled ? "Service URL, access token, and bot app ID are required." : "Not configured"
  );
  const googleChatConnectionDetail = gatewayChannelDetail(
    googleChatGateway,
    channelForm.google_chat_enabled
      ? "Google Chat needs an access token to reply in linked spaces. Add a default space if you want AgentArk to send proactive updates there."
      : "Not configured"
  );
  const signalConnectionDetail = gatewayChannelDetail(
    signalGateway,
    channelForm.signal_enabled
      ? "Signal needs a bridge URL and bridge token. Add a default recipient or group if you want proactive updates without waiting for an inbound chat first."
      : "Not configured"
  );
  const imessageConnectionDetail = gatewayChannelDetail(
    imessageGateway,
    channelForm.imessage_enabled
      ? "iMessage needs a bridge URL and bridge token. Add a default chat or handle if you want proactive updates before someone messages AgentArk first."
      : "Not configured"
  );
  const lineConnectionDetail = gatewayChannelDetail(
    lineGateway,
    channelForm.line_enabled
      ? "LINE needs a channel access token and channel secret. Add a default target if you want proactive updates outside an active chat."
      : "Not configured"
  );
  const wechatConnectionDetail = gatewayChannelDetail(
    wechatGateway,
    channelForm.wechat_enabled
      ? "WeChat needs a bridge URL and bridge token. Add a default target if you want proactive updates before an inbound chat establishes a reply route."
      : "Not configured"
  );
  const qqConnectionDetail = gatewayChannelDetail(
    qqGateway,
    channelForm.qq_enabled
      ? "QQ needs a bridge URL and bridge token. Add a default target if you want proactive updates before an inbound chat establishes a reply route."
      : "Not configured"
  );
  const mcpServers = showMcp ? asRecords(asRecord(mcpQ.data).servers) : [];
  const sorted = useMemo(
    () =>
      [...integrations].sort((a, b) => {
        if (a.id === "google_workspace" && b.id !== "google_workspace") return -1;
        if (b.id === "google_workspace" && a.id !== "google_workspace") return 1;
        return a.name.localeCompare(b.name);
      }),
    [integrations]
  );
  const activeSyncStatus = active ? integrationSyncStatusById[active.id] || null : null;
  const syncSummaryLabel =
    activeSyncStatus && !activeSyncStatus.supported
      ? "Not available"
      : syncForm.enabled
        ? "Enabled"
        : "Disabled";
  const readyList = sorted.filter((i) => integrationCardState(i) === "enabled");
  const notReadyList = sorted.filter((i) => integrationCardState(i) !== "enabled");
  const webhookSources = useMemo(
    () => asRecords(asRecord(webhookSourcesQ.data).sources),
    [webhookSourcesQ.data]
  );
  const customApis = useMemo(
    () => asRecords(asRecord(customApisQ.data).custom_apis),
    [customApisQ.data]
  );
  const customMessagingChannels = useMemo(
    () => customMessagingChannelsQ.data?.custom_messaging_channels || [],
    [customMessagingChannelsQ.data]
  );
  const pluginSummaries = useMemo(
    () => asRecords(asRecord(pluginsSummaryQ.data).plugins),
    [pluginsSummaryQ.data]
  );
  const enabledWebhookSources = webhookSources.filter((source) => source.enabled !== false);
  const enabledCustomApis = customApis.filter((item) => item.enabled !== false);
  const enabledPluginSummaries = pluginSummaries.filter((plugin) => plugin.enabled !== false);
  const liveConnectionSummary = [
    readyList.length > 0
      ? {
          key: "apps",
          label: "Connected Apps",
          detail: summarizeInlineNames(
            readyList.map((item) => item.name),
            "No connected apps yet."
          ),
          badge: `${readyList.length} live`
        }
      : null,
    enabledWebhookSources.length > 0
      ? {
          key: "webhooks",
          label: "Webhooks",
          detail: summarizeInlineNames(
            enabledWebhookSources.map((item) => str(item.name, str(item.id))),
            "No webhooks enabled."
          ),
          badge: `${enabledWebhookSources.length} active`
        }
      : null,
    enabledCustomApis.length > 0
      ? {
          key: "custom-apis",
          label: "Custom APIs",
          detail: summarizeInlineNames(
            enabledCustomApis.map((item) => str(item.name, str(item.id))),
            "No custom APIs active."
          ),
          badge: `${enabledCustomApis.length} imported`
        }
      : null,
    enabledPluginSummaries.length > 0
      ? {
          key: "plugins",
          label: "Plugins",
          detail: summarizeInlineNames(
            enabledPluginSummaries.map((item) => str(item.name, str(item.id))),
            "No plugins enabled."
          ),
          badge: `${enabledPluginSummaries.length} active`
        }
      : null
  ].filter((item): item is { key: string; label: string; detail: string; badge: string } => Boolean(item));
  const sectionAccordionSx = {
    border: "1px solid var(--ui-rgba-112-153-201-140)",
    borderRadius: "8px",
    background: "var(--ui-rgba-8-18-34-700)",
    boxShadow: "none",
    "&:before": { display: "none" },
    "&.Mui-expanded": { mt: 0, mb: 0 },
    "& .MuiAccordionSummary-root": {
      minHeight: 56,
      px: 1.5
    },
    "& .MuiAccordionSummary-content": {
      my: 1
    },
    "& .MuiAccordionDetails-root": {
      pt: 0,
      px: 1.5,
      pb: 1.5
    }
  } as const;
  const sectionCountChipSx = {
    height: 22,
    borderRadius: 1,
    background: "var(--ui-rgba-14-25-43-920)",
    border: "1px solid var(--ui-rgba-112-153-201-160)",
    color: "var(--ui-rgba-173-192-214-900)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.64rem",
      fontWeight: 700,
      letterSpacing: 0,
      textTransform: "uppercase"
    }
  } as const;
  const sectionTagChipSx = {
    height: 22,
    borderRadius: 1,
    background: "var(--ui-rgba-14-25-43-950)",
    border: "1px solid var(--ui-rgba-112-153-201-180)",
    color: "var(--ui-rgba-198-214-235-820)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.63rem",
      fontWeight: 700,
      letterSpacing: 0,
      textTransform: "uppercase"
    }
  } as const;
  const connectorCardActionButtonSx = {
    minWidth: 0,
    width: "auto",
    maxWidth: "fit-content",
    alignSelf: "flex-start",
    flex: "0 0 auto",
    whiteSpace: "nowrap",
    borderRadius: 1.5,
    textTransform: "none",
    fontWeight: 700,
    boxShadow: "none"
  } as const;
  const dialogActionButtonSx = {
    minHeight: 32,
    px: 1.5,
    borderRadius: 1.5,
    textTransform: "none",
    fontWeight: 700,
    boxShadow: "none"
  } as const;
  const mcpSorted = useMemo(
    () => [...mcpServers].sort((a, b) => str(a.name, "").localeCompare(str(b.name, ""))),
    [mcpServers]
  );
  const sshKeyNames = asStringList(asRecord(sshKeysQ.data).keys).sort((a, b) => a.localeCompare(b));
  const sshConnectionsText = str(asRecord(sshConnectionsQ.data).connections, "");
  const sshConnectionNames = parseSshConnectionNames(sshConnectionsText);

  async function refreshIntegrationState(targetId?: string | null) {
    const normalizedTargetId = normalizeIntegrationId(str(targetId, ""));
    const refreshedIntegrations = shouldLoadConnectorCatalog ? await integrationsQ.refetch() : null;
    if (showIntegrations) {
      await Promise.allSettled([
        integrationSyncStatusQ.refetch(),
        integrationSyncFeedQ.refetch(),
        settingsQ.refetch(),
        channelsQ.refetch(),
        telegramStatusQ.refetch(),
        channelForm.whatsapp_enabled && channelForm.whatsapp_mode === "baileys"
          ? waBridgeQ.refetch()
          : Promise.resolve(null),
      ]);
    }
    const refreshedItems = refreshedIntegrations?.data?.integrations || [];
    const targetForActive = normalizedTargetId || active?.id || "";
    if (targetForActive) {
      const updatedActive = refreshedItems.find((item) => item.id === targetForActive);
      if (updatedActive) {
        setActive((current) =>
          current && current.id === targetForActive ? updatedActive : current
        );
      }
    }
    if (!normalizedTargetId) return;
    const updated = refreshedItems.find((item) => item.id === normalizedTargetId);
    if (updated) {
      const resolved =
        updated.status === "connected" ||
        updated.status === "error" ||
        (updated.status !== "needs_auth" && !str(updated.auth_url, "").trim());
      if (resolved) {
        setOauthPendingId((current) =>
          normalizeIntegrationId(str(current, "")) === normalizedTargetId ? null : current
        );
      }
    }
  }

  function openCustomMessagingCredentials(channel: CustomMessagingChannel) {
    const nextValues = Object.fromEntries(
      customMessagingCredentialFields(channel).map((field) => [field.key, ""])
    );
    setCustomMessagingCredentialValues(nextValues);
    setCustomMessagingCredentialTarget(channel);
  }

  function submitCustomMessagingCredentials() {
    const channel = customMessagingCredentialTarget;
    if (!channel) return;
    saveCustomMessagingCredentialsMutation.mutate({
      id: channel.id,
      values: customMessagingCredentialValues
    });
  }

  const needsPendingConnectionRefresh =
    !!oauthPendingId ||
    (telegramEnabledSaved &&
      hasTelegramToken &&
      ["checking", "missing_token"].includes(telegramConnectionStatusRaw)) ||
    (channelForm.whatsapp_enabled &&
      channelForm.whatsapp_mode === "baileys" &&
      !["connected", "ready", "disabled"].includes(whatsappConnectionStatusRaw));

  useEffect(() => {
    if (!showIntegrations) return;

    const handleSignal = (value: unknown) => {
      const payload = parseOAuthSignalPayload(value);
      if (!payload) return;
      const targetId = normalizeIntegrationId(str(payload.integration_id ?? payload.service_id, ""));
      const targetName = targetId
        ? integrationDisplayName(targetId, integrations)
        : "Connection";
      const status = str(payload.status, "").trim().toLowerCase();
      const detail = str(payload.detail, "").trim();
      if (status === "error") {
        setNotice({
          kind: "error",
          text: detail ? `${targetName} connection failed: ${detail}` : `${targetName} connection failed.`
        });
      } else if (detail) {
        setNotice({
          kind: "error",
          text: `${targetName}: ${detail}`
        });
      }
      try {
        window.localStorage.removeItem(OAUTH_SIGNAL_STORAGE_KEY);
      } catch {
        // Best effort.
      }
      if (targetId) {
        void refreshIntegrationState(targetId);
      }
    };

    const handleStorage = (event: StorageEvent) => {
      if (event.key !== OAUTH_SIGNAL_STORAGE_KEY || !event.newValue) return;
      handleSignal(event.newValue);
    };

    const handleWindowFocus = () => {
      void refreshIntegrationState(oauthPendingId);
    };

    const handleVisibility = () => {
      if (document.visibilityState === "visible") {
        void refreshIntegrationState(oauthPendingId);
      }
    };

    const existingSignal = (() => {
      try {
        return window.localStorage.getItem(OAUTH_SIGNAL_STORAGE_KEY);
      } catch {
        return null;
      }
    })();
    if (existingSignal) {
      handleSignal(existingSignal);
    }

    window.addEventListener("storage", handleStorage);
    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibility);

    let oauthChannel: BroadcastChannel | null = null;
    if ("BroadcastChannel" in window) {
      oauthChannel = new BroadcastChannel(OAUTH_SIGNAL_CHANNEL);
      oauthChannel.addEventListener("message", (event) => handleSignal(event.data));
    }

    return () => {
      window.removeEventListener("storage", handleStorage);
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibility);
      oauthChannel?.close();
    };
  }, [showIntegrations, integrations, oauthPendingId, active?.id, channelsQ, channelForm.whatsapp_enabled, channelForm.whatsapp_mode]);

  useEffect(() => {
    if (!notice || notice.kind !== "success") return;
    const timer = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    if (!showIntegrations || !needsPendingConnectionRefresh) return;
    const intervalId = window.setInterval(() => {
      void refreshIntegrationState(oauthPendingId);
    }, 3500);
    return () => window.clearInterval(intervalId);
  }, [showIntegrations, needsPendingConnectionRefresh, oauthPendingId, telegramConnectionStatusRaw, whatsappConnectionStatusRaw]);

  useEffect(() => {
    if (!showIntegrations || !settingsQ.data || channelsDirty) return;
    const next = asRecord(settingsQ.data);
    const emailSettings = asRecord(next.email);
    const emailTransport = asRecord(emailSettings.transport);
    const emailHttpTransport = asRecord(emailTransport.http);
    const emailSmtpTransport = asRecord(emailTransport.smtp);
    const emailAuthSettings = asRecord(emailSettings.auth);
    setChannelForm({
      email_provider: str(emailSettings.provider, "auto"),
      email_to_address: str(emailSettings.to_address, ""),
      email_from_address: str(emailSettings.from_address, ""),
      email_domain: str(emailSettings.domain, ""),
      email_transport_kind: str(emailTransport.kind, "http"),
      email_http_base_url: str(emailHttpTransport.base_url, ""),
      email_http_send_path: str(emailHttpTransport.send_path, ""),
      email_smtp_host: str(emailSmtpTransport.host, ""),
      email_smtp_port: str(emailSmtpTransport.port, "587"),
      email_smtp_security: str(emailSmtpTransport.security, "starttls"),
      email_auth_kind: str(emailAuthSettings.kind, "none"),
      email_auth_api_key: "",
      email_auth_header_name: str(emailAuthSettings.header_name, ""),
      email_auth_scheme: str(emailAuthSettings.scheme, ""),
      email_auth_basic_username: str(emailAuthSettings.basic_username, ""),
      email_auth_basic_password: "",
      email_auth_aws_access_key_id: str(emailAuthSettings.aws_access_key_id, ""),
      email_auth_aws_secret_access_key: "",
      email_auth_aws_session_token: "",
      email_auth_aws_region: str(emailAuthSettings.aws_region, ""),
      email_auth_aws_service: str(emailAuthSettings.aws_service, "ses"),
      telegram_enabled: toBool(next.telegram_enabled),
      telegram_bot_token: "",
      telegram_allowed_users_csv: asStringList(next.telegram_allowed_users).join(", "),
      slack_enabled: toBool(next.slack_enabled),
      slack_bot_token: "",
      slack_signing_secret: "",
      slack_api_base_url: str(next.slack_api_base_url, "https://slack.com/api"),
      slack_default_channel_id: str(next.slack_default_channel_id, ""),
      slack_default_thread_ts: str(next.slack_default_thread_ts, ""),
      slack_workspace_id: str(next.slack_workspace_id, ""),
      slack_workspace_name: str(next.slack_workspace_name, ""),
      slack_trust_policy: str(slackTrustSettings.policy, "open"),
      slack_allowed_senders_csv: asStringList(slackTrustSettings.allowed_senders).join(", "),
      discord_enabled: toBool(next.discord_enabled),
      discord_bot_token: "",
      discord_webhook_url: str(next.discord_webhook_url, ""),
      discord_api_base_url: str(next.discord_api_base_url, "https://discord.com/api/v10"),
      discord_default_channel_id: str(next.discord_default_channel_id, ""),
      discord_default_thread_id: str(next.discord_default_thread_id, ""),
      discord_guild_id: str(next.discord_guild_id, ""),
      discord_application_id: str(next.discord_application_id, ""),
      matrix_enabled: toBool(next.matrix_enabled),
      matrix_homeserver_url: str(next.matrix_homeserver_url, ""),
      matrix_access_token: "",
      matrix_user_id: str(next.matrix_user_id, ""),
      matrix_device_id: str(next.matrix_device_id, ""),
      matrix_account_id: str(next.matrix_account_id, ""),
      matrix_default_room_id: str(next.matrix_default_room_id, ""),
      matrix_sync_timeout_ms: str(next.matrix_sync_timeout_ms, "30000"),
      matrix_limit: str(next.matrix_limit, "100"),
      matrix_user_agent: str(next.matrix_user_agent, ""),
      teams_enabled: toBool(next.teams_enabled),
      teams_service_url: str(next.teams_service_url, ""),
      teams_access_token: "",
      teams_bot_app_id: str(next.teams_bot_app_id, ""),
      teams_bot_name: str(next.teams_bot_name, ""),
      teams_tenant_id: str(next.teams_tenant_id, ""),
      teams_team_id: str(next.teams_team_id, ""),
      teams_channel_id: str(next.teams_channel_id, ""),
      teams_chat_id: str(next.teams_chat_id, ""),
      teams_graph_base_url: str(next.teams_graph_base_url, "https://graph.microsoft.com/v1.0"),
      teams_delivery_mode: str(next.teams_delivery_mode, "auto") === "graph"
        ? "graph"
        : str(next.teams_delivery_mode, "auto") === "bot_framework"
          ? "bot_framework"
          : "auto",
      teams_timeout_secs: str(next.teams_timeout_secs, "15"),
      teams_user_agent: str(next.teams_user_agent, ""),
      teams_trust_policy: str(teamsTrustSettings.policy, "open"),
      teams_allowed_senders_csv: asStringList(teamsTrustSettings.allowed_senders).join(", "),
      google_chat_enabled: toBool(next.google_chat_enabled),
      google_chat_access_token: "",
      google_chat_verify_token: "",
      google_chat_api_base_url: str(next.google_chat_api_base_url, "https://chat.googleapis.com"),
      google_chat_space: str(next.google_chat_space, ""),
      google_chat_thread_key: str(next.google_chat_thread_key, ""),
      google_chat_app_id: str(next.google_chat_app_id, ""),
      google_chat_bot_name: str(next.google_chat_bot_name, ""),
      google_chat_trust_policy: str(googleChatTrustSettings.policy, "open"),
      google_chat_allowed_senders_csv: asStringList(googleChatTrustSettings.allowed_senders).join(", "),
      signal_enabled: toBool(next.signal_enabled),
      signal_bridge_token: "",
      signal_bridge_url: str(next.signal_bridge_url, "http://127.0.0.1:9120"),
      signal_default_recipient: str(next.signal_default_recipient, ""),
      signal_default_group_id: str(next.signal_default_group_id, ""),
      signal_trust_policy: str(signalTrustSettings.policy, "open"),
      signal_allowed_senders_csv: asStringList(signalTrustSettings.allowed_senders).join(", "),
      imessage_enabled: toBool(next.imessage_enabled),
      imessage_bridge_token: "",
      imessage_bridge_url: str(next.imessage_bridge_url, "http://127.0.0.1:9130"),
      imessage_default_chat_id: str(next.imessage_default_chat_id, ""),
      imessage_default_handle: str(next.imessage_default_handle, ""),
      imessage_trust_policy: str(imessageTrustSettings.policy, "open"),
      imessage_allowed_senders_csv: asStringList(imessageTrustSettings.allowed_senders).join(", "),
      line_enabled: toBool(next.line_enabled),
      line_channel_access_token: "",
      line_channel_secret: "",
      line_api_base_url: str(next.line_api_base_url, "https://api.line.me"),
      line_default_target: str(next.line_default_target, ""),
      line_user_agent: str(next.line_user_agent, ""),
      line_trust_policy: str(lineTrustSettings.policy, "open"),
      line_allowed_senders_csv: asStringList(lineTrustSettings.allowed_senders).join(", "),
      wechat_enabled: toBool(next.wechat_enabled),
      wechat_bridge_token: "",
      wechat_bridge_url: str(next.wechat_bridge_url, "http://127.0.0.1:9140"),
      wechat_default_target_id: str(next.wechat_default_target_id, ""),
      wechat_trust_policy: str(wechatTrustSettings.policy, "open"),
      wechat_allowed_senders_csv: asStringList(wechatTrustSettings.allowed_senders).join(", "),
      qq_enabled: toBool(next.qq_enabled),
      qq_bridge_token: "",
      qq_bridge_url: str(next.qq_bridge_url, "http://127.0.0.1:9150"),
      qq_default_target_id: str(next.qq_default_target_id, ""),
      qq_trust_policy: str(qqTrustSettings.policy, "open"),
      qq_allowed_senders_csv: asStringList(qqTrustSettings.allowed_senders).join(", "),
      whatsapp_enabled: toBool(next.whatsapp_enabled),
      whatsapp_mode: str(next.whatsapp_mode, "baileys") === "cloud_api" ? "cloud_api" : "baileys",
      whatsapp_bridge_runtime:
        str(next.whatsapp_bridge_runtime, "embedded") === "external" ? "external" : "embedded",
      whatsapp_access_token: "",
      whatsapp_app_secret: "",
      whatsapp_phone_number_id: str(next.whatsapp_phone_number_id, ""),
      whatsapp_verify_token: "",
      whatsapp_bridge_token: "",
      whatsapp_bridge_url: str(next.whatsapp_bridge_url, ""),
      whatsapp_dm_policy: str(whatsappTrustSettings.policy, str(next.whatsapp_dm_policy, "pairing")) || "pairing",
      whatsapp_allowed_numbers_csv: asStringList(whatsappTrustSettings.allowed_senders).join(", ")
    });
  }, [
    showIntegrations,
    settingsQ.data,
    settingsQ.dataUpdatedAt,
    channelsDirty,
    senderVerificationQ.data,
    googleChatTrustSettings,
    signalTrustSettings,
    imessageTrustSettings,
    lineTrustSettings,
    slackTrustSettings,
    teamsTrustSettings,
    wechatTrustSettings,
    qqTrustSettings,
    whatsappTrustSettings
  ]);

  useEffect(() => {
    if (!active || syncDirty) return;
    setSyncForm(integrationSyncFormFromStatus(activeSyncStatus));
  }, [active?.id, activeSyncStatus, syncDirty]);

  const setChannelField = <K extends keyof ChannelSettingsForm>(
    key: K,
    value: ChannelSettingsForm[K]
  ) => {
    setChannelsDirty(true);
    setChannelForm((prev) => ({ ...prev, [key]: value }));
  };

  const openEmailSetup = () => {
    setNotice(null);
    setEmailSetupOpen(true);
  };

  const openTelegramSetup = (enableIfDisabled = false) => {
    setNotice(null);
    if (enableIfDisabled && !channelForm.telegram_enabled) {
      setChannelField("telegram_enabled", true);
    }
    setTelegramSetupOpen(true);
  };

  const openSlackSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.slack_enabled) {
      setChannelField("slack_enabled", true);
    }
    setSlackSetupOpen(true);
  };

  const openDiscordSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.discord_enabled) {
      setChannelField("discord_enabled", true);
    }
    setDiscordSetupOpen(true);
  };

  const openMatrixSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.matrix_enabled) {
      setChannelField("matrix_enabled", true);
    }
    setMatrixSetupOpen(true);
  };

  const openTeamsSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.teams_enabled) {
      setChannelField("teams_enabled", true);
    }
    setTeamsSetupOpen(true);
  };

  const openGoogleChatSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.google_chat_enabled) {
      setChannelField("google_chat_enabled", true);
    }
    setGoogleChatSetupOpen(true);
  };

  const openSignalSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.signal_enabled) {
      setChannelField("signal_enabled", true);
    }
    setSignalSetupOpen(true);
  };

  const openIMessageSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.imessage_enabled) {
      setChannelField("imessage_enabled", true);
    }
    setImessageSetupOpen(true);
  };

  const openLineSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.line_enabled) {
      setChannelField("line_enabled", true);
    }
    setLineSetupOpen(true);
  };

  const openWeChatSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.wechat_enabled) {
      setChannelField("wechat_enabled", true);
    }
    setWechatSetupOpen(true);
  };

  const openQqSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.qq_enabled) {
      setChannelField("qq_enabled", true);
    }
    setQqSetupOpen(true);
  };

  const openWhatsAppSetup = (enableIfDisabled = false) => {
    if (enableIfDisabled && !channelForm.whatsapp_enabled) {
      setChannelField("whatsapp_enabled", true);
    }
    setWhatsAppSetupOpen(true);
  };

  const saveChannelDialog = async (onClose: () => void) => {
    try {
      await saveChannelsMutation.mutateAsync(channelForm);
      onClose();
    } catch {
      // Error alert handled by mutation + top-level notice.
    }
  };

  const disconnectChannel = async (
    label: string,
    enabledField: MessagingChannelEnabledField,
    onClose: () => void
  ) => {
    if (
      !window.confirm(
        `Disconnect ${label}? This will disable the channel and remove its saved configuration.`
      )
    ) {
      return;
    }
    const previousForm = channelForm;
    const wasDirty = channelsDirty;
    const nextForm = { ...channelForm, [enabledField]: false } as ChannelSettingsForm;
    setNotice(null);
    setChannelsDirty(true);
    setChannelForm(nextForm);
    try {
      await saveChannelsMutation.mutateAsync(nextForm);
      onClose();
    } catch {
      setChannelForm(previousForm);
      setChannelsDirty(wasDirty);
    }
  };

  const renderMessagingDialogActions = ({
    onClose,
    onDisconnect,
    disconnectVisible = false,
    disconnectLabel = "Disconnect"
  }: {
    onClose: () => void;
    onDisconnect?: () => void | Promise<void>;
    disconnectVisible?: boolean;
    disconnectLabel?: string;
  }) => (
    <DialogActions>
      {disconnectVisible ? (
        <Button
          color="warning"
          onClick={() => {
            void onDisconnect?.();
          }}
          disabled={saveChannelsMutation.isPending}
        >
          {disconnectLabel}
        </Button>
      ) : null}
      <Box sx={{ flex: 1 }} />
      <Button onClick={onClose} disabled={saveChannelsMutation.isPending}>
        Cancel
      </Button>
      <Button
        variant="contained"
        disabled={saveChannelsMutation.isPending || settingsQ.isLoading}
        onClick={() => {
          void saveChannelDialog(onClose);
        }}
      >
        {saveChannelsMutation.isPending ? "Saving..." : "Save"}
      </Button>
    </DialogActions>
  );

  const messagingSetups = [
    {
      id: "telegram",
      name: "Telegram",
      enabled: telegramEnabledSaved,
      status: telegramConnectionStatusRaw,
      detail: !telegramEnabledSaved
        ? "Turn Telegram on in the wizard, then add a bot token."
        : telegramConnectionDetail || (telegramTokenConfigured ? "Bot token saved. Finish setup to make delivery available." : "Add a bot token to finish setup."),
      actionLabel: channelForm.telegram_enabled ? "Open wizard" : "Turn on",
      open: () => openTelegramSetup(!channelForm.telegram_enabled)
    },
    {
      id: "whatsapp",
      name: "WhatsApp",
      enabled: channelForm.whatsapp_enabled,
      status: whatsappConnectionStatusRaw,
      detail: !channelForm.whatsapp_enabled
        ? "Turn WhatsApp on in the wizard, then choose a bundled bridge, an external bridge, or Cloud API."
        : whatsappConnectionDetail ||
          (channelForm.whatsapp_mode === "cloud_api"
            ? "Add the Cloud API token, app secret, verify token, and phone number ID to finish setup."
            : "Open the wizard to finish your bundled or external bridge setup."),
      actionLabel: channelForm.whatsapp_enabled ? "Open wizard" : "Turn on",
      open: () => openWhatsAppSetup(!channelForm.whatsapp_enabled)
    },
    {
      id: "slack",
      name: "Slack",
      enabled: channelForm.slack_enabled,
      status: slackConnectionStatusRaw,
      detail: !channelForm.slack_enabled
        ? "Turn Slack on in the wizard, then add the bot token and signing secret."
        : slackConnectionDetail,
      actionLabel: channelForm.slack_enabled ? "Open wizard" : "Turn on",
      open: () => openSlackSetup(!channelForm.slack_enabled)
    },
    {
      id: "discord",
      name: "Discord",
      enabled: channelForm.discord_enabled,
      status: discordConnectionStatusRaw,
      detail: !channelForm.discord_enabled
        ? "Turn Discord on in the wizard, then add a bot token or webhook URL."
        : discordConnectionDetail,
      actionLabel: channelForm.discord_enabled ? "Open wizard" : "Turn on",
      open: () => openDiscordSetup(!channelForm.discord_enabled)
    },
    {
      id: "matrix",
      name: "Matrix",
      enabled: channelForm.matrix_enabled,
      status: matrixConnectionStatusRaw,
      detail: !channelForm.matrix_enabled
        ? "Turn Matrix on in the wizard, then add the homeserver URL, access token, and user ID."
        : matrixConnectionDetail,
      actionLabel: channelForm.matrix_enabled ? "Open wizard" : "Turn on",
      open: () => openMatrixSetup(!channelForm.matrix_enabled)
    },
    {
      id: "teams",
      name: "Teams",
      enabled: channelForm.teams_enabled,
      status: teamsConnectionStatusRaw,
      detail: !channelForm.teams_enabled
        ? "Turn Teams on in the wizard, then add the service URL, access token, and bot app ID."
        : teamsConnectionDetail,
      actionLabel: channelForm.teams_enabled ? "Open wizard" : "Turn on",
      open: () => openTeamsSetup(!channelForm.teams_enabled)
    },
    {
      id: "google_chat",
      name: "Google Chat",
      enabled: channelForm.google_chat_enabled,
      status: googleChatConnectionStatusRaw,
      detail: !channelForm.google_chat_enabled
        ? "Turn Google Chat on, then add an access token. Add a default space only if you want proactive updates there."
        : googleChatConnectionDetail,
      actionLabel: channelForm.google_chat_enabled ? "Open wizard" : "Turn on",
      open: () => openGoogleChatSetup(!channelForm.google_chat_enabled)
    },
    {
      id: "signal",
      name: "Signal",
      enabled: channelForm.signal_enabled,
      status: signalConnectionStatusRaw,
      detail: !channelForm.signal_enabled
        ? "Turn Signal on, then add the bridge URL and token. Add a default recipient or group if you want proactive delivery."
        : signalConnectionDetail,
      actionLabel: channelForm.signal_enabled ? "Open wizard" : "Turn on",
      open: () => openSignalSetup(!channelForm.signal_enabled)
    },
    {
      id: "imessage",
      name: "iMessage",
      enabled: channelForm.imessage_enabled,
      status: imessageConnectionStatusRaw,
      detail: !channelForm.imessage_enabled
        ? "Turn iMessage on, then add the bridge URL and token. Add a default chat or handle if you want proactive delivery."
        : imessageConnectionDetail,
      actionLabel: channelForm.imessage_enabled ? "Open wizard" : "Turn on",
      open: () => openIMessageSetup(!channelForm.imessage_enabled)
    },
    {
      id: "line",
      name: "LINE",
      enabled: channelForm.line_enabled,
      status: lineConnectionStatusRaw,
      detail: !channelForm.line_enabled
        ? "Turn LINE on, then add the channel token and secret. Add a default target if you want proactive delivery outside an active chat."
        : lineConnectionDetail,
      actionLabel: channelForm.line_enabled ? "Open wizard" : "Turn on",
      open: () => openLineSetup(!channelForm.line_enabled)
    },
    {
      id: "wechat",
      name: "WeChat",
      enabled: channelForm.wechat_enabled,
      status: wechatConnectionStatusRaw,
      detail: !channelForm.wechat_enabled
        ? "Turn WeChat on, then add the bridge URL and token. Add a default target if you want proactive delivery."
        : wechatConnectionDetail,
      actionLabel: channelForm.wechat_enabled ? "Open wizard" : "Turn on",
      open: () => openWeChatSetup(!channelForm.wechat_enabled)
    },
    {
      id: "qq",
      name: "QQ",
      enabled: channelForm.qq_enabled,
      status: qqConnectionStatusRaw,
      detail: !channelForm.qq_enabled
        ? "Turn QQ on, then add the bridge URL and token. Add a default target if you want proactive delivery."
        : qqConnectionDetail,
      actionLabel: channelForm.qq_enabled ? "Open wizard" : "Turn on",
      open: () => openQqSetup(!channelForm.qq_enabled)
    }
  ];
  const messagingReadyCount = messagingSetups.filter(
    (setup) => messagingDisplayState(setup.status, setup.enabled) === "ready"
  ).length;
  const messagingAttentionCount = messagingSetups.length - messagingReadyCount;
  const connectedMessagingSetups = messagingSetups
    .map((setup) => ({
      ...setup,
      displayState: messagingDisplayState(setup.status, setup.enabled)
    }))
    .filter((setup) => setup.displayState === "ready");

  const persistChannelSettings = async (form: ChannelSettingsForm) => {
      const emailProviderForSave = form.email_provider.trim().toLowerCase() || "auto";
      const emailTransportKindForSave =
        form.email_transport_kind.trim().toLowerCase() === "smtp" ? "smtp" : "http";
      const emailAuthKindForSave = (() => {
        const selected = form.email_auth_kind.trim().toLowerCase() || "none";
        if (selected !== "none") return selected;
        if (emailProviderForSave === "ses" && emailTransportKindForSave === "smtp") {
          return "basic";
        }
        return "none";
      })();
      const emailAuthPayload: Record<string, unknown> = {
        kind: emailAuthKindForSave,
        header_name: form.email_auth_header_name.trim(),
        scheme: form.email_auth_scheme.trim(),
        basic_username: form.email_auth_basic_username.trim(),
        aws_access_key_id: form.email_auth_aws_access_key_id.trim(),
        aws_region: form.email_auth_aws_region.trim(),
        aws_service: form.email_auth_aws_service.trim()
      };
      if (form.email_auth_api_key.trim()) {
        emailAuthPayload.api_key = form.email_auth_api_key.trim();
      }
      if (form.email_auth_basic_password.trim()) {
        emailAuthPayload.basic_password = form.email_auth_basic_password.trim();
      }
      if (form.email_auth_aws_secret_access_key.trim()) {
        emailAuthPayload.aws_secret_access_key = form.email_auth_aws_secret_access_key.trim();
      }
      if (form.email_auth_aws_session_token.trim()) {
        emailAuthPayload.aws_session_token = form.email_auth_aws_session_token.trim();
      }
      const payload: Record<string, unknown> = {
        llm_provider: str(settings.llm_provider, ""),
        llm_model: str(settings.llm_model, ""),
        llm_base_url: settings.llm_base_url ?? null,
        llm_api_key: null,
        llm_fallback_provider: settings.llm_fallback_provider ?? null,
        llm_fallback_model: settings.llm_fallback_model ?? null,
        llm_fallback_base_url: settings.llm_fallback_base_url ?? null,
        llm_fallback_api_key: null,
        email: {
          provider: emailProviderForSave,
          to_address: form.email_to_address.trim(),
          from_address: form.email_from_address.trim(),
          domain: form.email_domain.trim(),
          transport: {
            kind: emailTransportKindForSave,
            http: {
              base_url: form.email_http_base_url.trim(),
              send_path: form.email_http_send_path.trim()
            },
            smtp: {
              host: form.email_smtp_host.trim(),
              port: Number(form.email_smtp_port) || 587,
              security: form.email_smtp_security.trim() || "starttls"
            }
          },
          auth: emailAuthPayload
        },
        telegram_enabled: !!form.telegram_enabled,
        telegram_bot_token: form.telegram_bot_token.trim() || null,
        telegram_allowed_users: parseTelegramUsers(form.telegram_allowed_users_csv),
        slack_enabled: !!form.slack_enabled,
        slack_bot_token: form.slack_bot_token.trim() || null,
        slack_signing_secret: form.slack_signing_secret.trim() || null,
        slack_api_base_url: form.slack_api_base_url.trim() || null,
        slack_default_channel_id: form.slack_default_channel_id.trim() || null,
        slack_default_thread_ts: form.slack_default_thread_ts.trim() || null,
        slack_workspace_id: form.slack_workspace_id.trim() || null,
        slack_workspace_name: form.slack_workspace_name.trim() || null,
        discord_enabled: !!form.discord_enabled,
        discord_bot_token: form.discord_bot_token.trim() || null,
        discord_webhook_url: form.discord_webhook_url.trim() || null,
        discord_api_base_url: form.discord_api_base_url.trim() || null,
        discord_default_channel_id: form.discord_default_channel_id.trim() || null,
        discord_default_thread_id: form.discord_default_thread_id.trim() || null,
        discord_guild_id: form.discord_guild_id.trim() || null,
        discord_application_id: form.discord_application_id.trim() || null,
        matrix_enabled: !!form.matrix_enabled,
        matrix_homeserver_url: form.matrix_homeserver_url.trim() || null,
        matrix_access_token: form.matrix_access_token.trim() || null,
        matrix_user_id: form.matrix_user_id.trim() || null,
        matrix_device_id: form.matrix_device_id.trim() || null,
        matrix_account_id: form.matrix_account_id.trim() || null,
        matrix_default_room_id: form.matrix_default_room_id.trim() || null,
        matrix_sync_timeout_ms: Number(form.matrix_sync_timeout_ms) || null,
        matrix_limit: Number(form.matrix_limit) || null,
        matrix_user_agent: form.matrix_user_agent.trim() || null,
        teams_enabled: !!form.teams_enabled,
        teams_service_url: form.teams_service_url.trim() || null,
        teams_access_token: form.teams_access_token.trim() || null,
        teams_bot_app_id: form.teams_bot_app_id.trim() || null,
        teams_bot_name: form.teams_bot_name.trim() || null,
        teams_tenant_id: form.teams_tenant_id.trim() || null,
        teams_team_id: form.teams_team_id.trim() || null,
        teams_channel_id: form.teams_channel_id.trim() || null,
        teams_chat_id: form.teams_chat_id.trim() || null,
        teams_graph_base_url: form.teams_graph_base_url.trim() || null,
        teams_delivery_mode: form.teams_delivery_mode,
        teams_timeout_secs: Number(form.teams_timeout_secs) || null,
        teams_user_agent: form.teams_user_agent.trim() || null,
        google_chat_enabled: !!form.google_chat_enabled,
        google_chat_access_token: form.google_chat_access_token.trim() || null,
        google_chat_verify_token: form.google_chat_verify_token.trim() || null,
        google_chat_api_base_url: form.google_chat_api_base_url.trim() || null,
        google_chat_space: form.google_chat_space.trim() || null,
        google_chat_thread_key: form.google_chat_thread_key.trim() || null,
        google_chat_app_id: form.google_chat_app_id.trim() || null,
        google_chat_bot_name: form.google_chat_bot_name.trim() || null,
        signal_enabled: !!form.signal_enabled,
        signal_bridge_token: form.signal_bridge_token.trim() || null,
        signal_bridge_url: form.signal_bridge_url.trim() || null,
        signal_default_recipient: form.signal_default_recipient.trim() || null,
        signal_default_group_id: form.signal_default_group_id.trim() || null,
        imessage_enabled: !!form.imessage_enabled,
        imessage_bridge_token: form.imessage_bridge_token.trim() || null,
        imessage_bridge_url: form.imessage_bridge_url.trim() || null,
        imessage_default_chat_id: form.imessage_default_chat_id.trim() || null,
        imessage_default_handle: form.imessage_default_handle.trim() || null,
        line_enabled: !!form.line_enabled,
        line_channel_access_token: form.line_channel_access_token.trim() || null,
        line_channel_secret: form.line_channel_secret.trim() || null,
        line_api_base_url: form.line_api_base_url.trim() || null,
        line_default_target: form.line_default_target.trim() || null,
        line_user_agent: form.line_user_agent.trim() || null,
        wechat_enabled: !!form.wechat_enabled,
        wechat_bridge_token: form.wechat_bridge_token.trim() || null,
        wechat_bridge_url: form.wechat_bridge_url.trim() || null,
        wechat_default_target_id: form.wechat_default_target_id.trim() || null,
        qq_enabled: !!form.qq_enabled,
        qq_bridge_token: form.qq_bridge_token.trim() || null,
        qq_bridge_url: form.qq_bridge_url.trim() || null,
        qq_default_target_id: form.qq_default_target_id.trim() || null,
        whatsapp_enabled: !!form.whatsapp_enabled,
        whatsapp_mode: form.whatsapp_mode,
        whatsapp_bridge_runtime:
          form.whatsapp_mode === "baileys" ? form.whatsapp_bridge_runtime : null,
        whatsapp_access_token: form.whatsapp_access_token.trim() || null,
        whatsapp_app_secret: form.whatsapp_app_secret.trim() || null,
        whatsapp_phone_number_id: form.whatsapp_phone_number_id.trim() || null,
        whatsapp_verify_token: form.whatsapp_verify_token.trim() || null,
        whatsapp_bridge_token:
          form.whatsapp_mode === "baileys" &&
          form.whatsapp_bridge_runtime === "external"
            ? form.whatsapp_bridge_token.trim() || null
            : null,
        whatsapp_bridge_url:
          form.whatsapp_mode === "baileys" &&
          form.whatsapp_bridge_runtime === "external"
            ? form.whatsapp_bridge_url.trim() || null
            : null,
        whatsapp_dm_policy: form.whatsapp_dm_policy.trim() || null,
        whatsapp_allowed_numbers: parseCsvList(form.whatsapp_allowed_numbers_csv)
      };
      await api.rawPost("/settings", payload);

      const senderVerificationPayload: Record<string, unknown> = {
        google_chat_policy: form.google_chat_trust_policy.trim() || "open",
        google_chat_allowed_senders: parseCsvList(form.google_chat_allowed_senders_csv),
        signal_policy: form.signal_trust_policy.trim() || "open",
        signal_allowed_senders: parseCsvList(form.signal_allowed_senders_csv),
        imessage_policy: form.imessage_trust_policy.trim() || "open",
        imessage_allowed_senders: parseCsvList(form.imessage_allowed_senders_csv),
        line_policy: form.line_trust_policy.trim() || "open",
        line_allowed_senders: parseCsvList(form.line_allowed_senders_csv),
        slack_policy: form.slack_trust_policy.trim() || "open",
        slack_allowed_senders: parseCsvList(form.slack_allowed_senders_csv),
        teams_policy: form.teams_trust_policy.trim() || "open",
        teams_allowed_senders: parseCsvList(form.teams_allowed_senders_csv),
        wechat_policy: form.wechat_trust_policy.trim() || "open",
        wechat_allowed_senders: parseCsvList(form.wechat_allowed_senders_csv),
        qq_policy: form.qq_trust_policy.trim() || "open",
        qq_allowed_senders: parseCsvList(form.qq_allowed_senders_csv)
      };

      if (form.whatsapp_enabled) {
        senderVerificationPayload.whatsapp_policy =
          form.whatsapp_dm_policy.trim() || "pairing";
        senderVerificationPayload.whatsapp_allowed_senders = parseCsvList(
          form.whatsapp_allowed_numbers_csv
        );
      }

      return api.rawPost("/sender-verification/settings", senderVerificationPayload);
  };

  const handleChannelsSaved = async (savedForm: ChannelSettingsForm) => {
    setNotice({ kind: "success", text: "Channel settings saved." });
    await Promise.allSettled([
      settingsQ.refetch(),
      channelsQ.refetch(),
      senderVerificationQ.refetch(),
      telegramStatusQ.refetch(),
      savedForm.whatsapp_enabled && savedForm.whatsapp_mode === "baileys"
        ? waBridgeQ.refetch()
        : Promise.resolve(null),
      queryClient.invalidateQueries({ queryKey: ["gateway-channels"] })
    ]);
    setChannelsDirty(false);
  };

  const saveChannelsMutation = useMutation({
    mutationFn: persistChannelSettings,
    onSuccess: async (_data, savedForm) => {
      await handleChannelsSaved(savedForm);
    },
    onError: (err) => {
      setNotice({ kind: "error", text: asErrorMessage(err) });
    }
  });

  const senderTrustBusy =
    approveSenderMutation.isPending ||
    revokeSenderMutation.isPending ||
    senderVerificationQ.isFetching;

  const renderSenderTrustSection = ({
    channel,
    configured,
    policyValue,
    onPolicyChange,
    allowedValue,
    onAllowedChange,
    allowedLabel,
    allowedHelper
  }: {
    channel: "google_chat" | "signal" | "imessage" | "line" | "slack" | "teams" | "whatsapp" | "wechat" | "qq";
    configured: boolean;
    policyValue: string;
    onPolicyChange: (value: string) => void;
    allowedValue: string;
    onAllowedChange: (value: string) => void;
    allowedLabel: string;
    allowedHelper: string;
  }) => {
    const pending = pendingSenderRows.filter((row) => str(row.channel) === channel);
    const approved = approvedSenderRows.filter((row) => str(row.channel) === channel);
    const channelNameMap: Record<string, string> = {
      google_chat: "Google Chat",
      signal: "Signal",
      imessage: "iMessage",
      line: "LINE",
      slack: "Slack",
      teams: "Teams",
      whatsapp: "WhatsApp",
      wechat: "WeChat",
      qq: "QQ"
    };
    const channelName = channelNameMap[channel] || channel;
    const formatSeen = (value: string) => {
      return formatUiDateTime(value, { fallback: "-" });
    };
    const senderLabel = (row: JsonRecord) => str(row.sender_label, str(row.sender_id, "-"));
    const scopeLabel = (row: JsonRecord) => str(row.scope_label, str(row.scope_id, "-"));

    return (
      <Box
        sx={{
          border: "1px solid var(--ui-rgba-110-160-255-180)",
          borderRadius: 2,
          p: 1.25,
          background: "var(--ui-rgba-10-18-32-500)"
        }}
      >
        <Stack spacing={1.25}>
          <Box>
            <Typography variant="subtitle2">Sender Trust</Typography>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Use pairing when unknown {channelName} senders should wait for operator approval before AgentArk acts.
            </Typography>
          </Box>
          <TextField
            select
            fullWidth
            size="small"
            label="Policy"
            value={policyValue}
            onChange={(e) => onPolicyChange(e.target.value)}
            disabled={!configured}
          >
            <MenuItem value="open">Open</MenuItem>
            <MenuItem value="pairing">Pairing</MenuItem>
          </TextField>
          <TextField
            fullWidth
            size="small"
            label={allowedLabel}
            value={allowedValue}
            onChange={(e) => onAllowedChange(e.target.value)}
            disabled={!configured}
            multiline
            minRows={2}
            helperText={configured ? allowedHelper : `${channelName} must be configured first.`}
          />
          {senderVerificationQ.error ? (
            <Alert severity="warning">
              Could not load sender approval state right now: {asErrorMessage(senderVerificationQ.error)}
            </Alert>
          ) : null}
          <Stack direction="row" spacing={1} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Chip size="small" variant="outlined" label={`${pending.length} pending`} />
            <Chip size="small" variant="outlined" label={`${approved.length} approved`} />
          </Stack>
          <Box>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                display: "block",
                mb: 0.75
              }}>
              Pending approvals
            </Typography>
            {pending.length === 0 ? (
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                No pending {channelName} senders.
              </Typography>
            ) : (
              <Stack spacing={0.9}>
                {pending.map((row) => (
                  <Box
                    key={str(row.key, `${channel}-${str(row.sender_id)}`)}
                    sx={{
                      border: "1px solid var(--ui-rgba-255-255-255-080)",
                      borderRadius: 1.5,
                      px: 1,
                      py: 0.9
                    }}
                  >
                    <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{
                      justifyContent: "space-between"
                    }}>
                      <Box sx={{ minWidth: 0 }}>
                        <Typography variant="body2">{senderLabel(row)}</Typography>
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          {str(row.sender_id, "-")}
                          {str(row.scope_id) ? ` | ${scopeLabel(row)}` : ""}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block"
                          }}>
                          Last seen {formatSeen(str(row.last_seen_at))}
                        </Typography>
                      </Box>
                      <Button
                        size="small"
                        variant="contained"
                        disabled={senderTrustBusy}
                        onClick={async () => {
                          try {
                            setNotice(null);
                            await approveSenderMutation.mutateAsync({
                              channel,
                              sender_id: str(row.sender_id),
                              sender_label: str(row.sender_label) || undefined,
                              scope_id: str(row.scope_id) || undefined,
                              scope_label: str(row.scope_label) || undefined,
                              conversation_id: str(row.conversation_id) || undefined,
                              approved_by: "integrations_panel"
                            });
                            setNotice({ kind: "success", text: `${channelName} sender approved.` });
                          } catch (err) {
                            setNotice({ kind: "error", text: asErrorMessage(err) });
                          }
                        }}
                      >
                        Approve
                      </Button>
                    </Stack>
                  </Box>
                ))}
              </Stack>
            )}
          </Box>
          <Box>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                display: "block",
                mb: 0.75
              }}>
              Approved senders
            </Typography>
            {approved.length === 0 ? (
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                No approved {channelName} senders yet.
              </Typography>
            ) : (
              <Stack spacing={0.9}>
                {approved.map((row) => (
                  <Box
                    key={str(row.key, `${channel}-${str(row.sender_id)}`)}
                    sx={{
                      border: "1px solid var(--ui-rgba-255-255-255-080)",
                      borderRadius: 1.5,
                      px: 1,
                      py: 0.9
                    }}
                  >
                    <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{
                      justifyContent: "space-between"
                    }}>
                      <Box sx={{ minWidth: 0 }}>
                        <Typography variant="body2">{senderLabel(row)}</Typography>
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          {str(row.sender_id, "-")}
                          {str(row.scope_id) ? ` | ${scopeLabel(row)}` : ""}
                        </Typography>
                      </Box>
                      <Button
                        size="small"
                        color="warning"
                        variant="outlined"
                        disabled={senderTrustBusy}
                        onClick={async () => {
                          try {
                            setNotice(null);
                            await revokeSenderMutation.mutateAsync({
                              channel,
                              sender_id: str(row.sender_id),
                              scope_id: str(row.scope_id) || undefined
                            });
                            setNotice({ kind: "success", text: `${channelName} sender revoked.` });
                          } catch (err) {
                            setNotice({ kind: "error", text: asErrorMessage(err) });
                          }
                        }}
                      >
                        Revoke
                      </Button>
                    </Stack>
                  </Box>
                ))}
              </Stack>
            )}
          </Box>
        </Stack>
      </Box>
    );
  };

  const setSyncField = <K extends keyof IntegrationSyncFormState>(
    key: K,
    value: IntegrationSyncFormState[K]
  ) => {
    setSyncDirty(true);
    setSyncForm((prev) => ({ ...prev, [key]: value }));
  };

  const saveActiveSyncSettings = async () => {
    if (!active) return;
    const pollIntervalMinutes = Math.max(1, Number(syncForm.poll_interval_minutes) || 5);
    const thresholdPercent = Math.min(100, Math.max(10, Number(syncForm.importance_threshold_percent) || 70));
    try {
      setSyncNotice(null);
      await saveSyncMutation.mutateAsync({
        id: active.id,
        payload: {
          enabled: syncForm.enabled,
          poll_interval_secs: pollIntervalMinutes * 60,
          importance_threshold: thresholdPercent / 100,
          notify_on_important: syncForm.notify_on_important,
          push_to_preferred_channel: syncForm.push_to_preferred_channel
        }
      });
      setSyncDirty(false);
      setSyncNotice({ kind: "success", text: "Background sync settings saved." });
    } catch (err) {
      setSyncNotice({ kind: "error", text: asErrorMessage(err) });
    }
  };

  const runActiveSyncNow = async () => {
    if (!active) return;
    try {
      setSyncNotice(null);
      await syncNowMutation.mutateAsync(active.id);
      setSyncNotice({ kind: "success", text: "Sync run queued." });
    } catch (err) {
      setSyncNotice({ kind: "error", text: asErrorMessage(err) });
    }
  };

  const openConfig = (integration: IntegrationItem) => {
    setActive(integration);
    setFormError(null);
    const nextValues: Record<string, string> = {};
    const configValues = asRecord(integration.config_values);
    for (const [key, value] of Object.entries(configValues)) {
      if (typeof value === "string") {
        nextValues[key] = value;
      } else if (typeof value === "number" || typeof value === "boolean") {
        nextValues[key] = String(value);
      }
    }
    setFormValues(nextValues);
    setConfigSuccess(false);
    setSyncForm(integrationSyncFormFromStatus(integrationSyncStatusById[integration.id] || null));
    setSyncDirty(false);
    setSyncExpanded(false);
    setSyncNotice(null);
    setGoogleWorkspaceHelpOpen(false);
  };

  const closeConfig = () => {
    setActive(null);
    setFormValues({});
    setFormError(null);
    setSaving(false);
    setConfigSuccess(false);
    setEditingConnected(false);
    setSyncForm(defaultIntegrationSyncForm());
    setSyncDirty(false);
    setSyncExpanded(false);
    setSyncNotice(null);
    setGoogleWorkspaceHelpOpen(false);
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
      env_csv: asStringList(transport.env_keys).join(", "),
      auth_type: authType,
      auth_header: str(auth.header, "Authorization"),
      auth_name: str(auth.name, ""),
      auth_token: "",
      auth_username: "",
      auth_password: "",
      auth_clear: false,
      tool_allowlist_csv: asStringList(server.tool_allowlist).join(", "),
      tool_blocklist_csv: asStringList(server.tool_blocklist).join(", "),
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
        tool_blocklist: parseCsvList(mcpForm.tool_blocklist_csv),
        timeout_secs: Math.floor(timeoutSecs),
        max_response_bytes: Math.floor(maxResponseBytes)
      };
      if (!mcpEditingId && mcpForm.id.trim()) payload.id = mcpForm.id.trim();

      if (mcpForm.transport_type === "http") {
        if (!mcpForm.url.trim()) throw new Error("HTTP URL is required.");
        const urlLower = mcpForm.url.trim().toLowerCase();
        if (!urlLower.startsWith("https://")) {
            throw new Error("MCP URL must start with https://");
        }
        payload.transport = { type: "http", url: mcpForm.url.trim() };
      } else {
        if (!mcpForm.command.trim()) throw new Error("Stdio command is required.");
        payload.transport = {
          type: "stdio",
          command: mcpForm.command.trim(),
          args: parseCsvList(mcpForm.args_csv),
          working_dir: mcpForm.working_dir.trim() || undefined,
          env_keys: parseCsvList(mcpForm.env_csv)
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

      // Warn if auth type set but no credentials provided (for new servers)
      if (!mcpEditingId && mcpForm.auth_type !== "none") {
          const hasCredential = mcpForm.auth_type === "basic"
              ? (mcpForm.auth_username.trim() || mcpForm.auth_password.trim())
              : mcpForm.auth_token.trim();
          if (!hasCredential) {
              throw new Error(`Auth type "${mcpForm.auth_type}" selected but no credentials provided. Add credentials or set auth to "none".`);
          }
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

  const saveActiveConfig = async (closeIfDone = true): Promise<IntegrationItem | null> => {
    if (!active) return null;
    const current = active;
    const fields = current.config_fields || [];
    for (const field of fields) {
      if (field.required && !(formValues[field.key] || "").trim()) {
        setFormError(`Missing required field: ${field.label}`);
        return null;
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
      await configureMutation.mutateAsync({ id: current.id, payload });
      const [refreshed] = await Promise.all([
        integrationsQ.refetch(),
        integrationSyncStatusQ.refetch(),
        integrationSyncFeedQ.refetch()
      ]);
      const refreshedItems = refreshed.data?.integrations || [];
      const updated = refreshedItems.find((item) => item.id === current.id) || current;
      setActive(updated);
      setConfigSuccess(true);
      setSaving(false);
      const needsOauthFollowup =
        updated.id === "google_workspace" ||
        updated.id === "gmail" ||
        updated.id === "google_calendar" ||
        updated.status === "needs_auth" ||
        !!str(updated.auth_url, "").trim();
      if (closeIfDone && !needsOauthFollowup) {
        setTimeout(() => {
          closeConfig();
        }, 850);
      }
      return updated;
    } catch (err) {
      setFormError(asErrorMessage(err));
      // Backend disables integration on failed validation; refresh to reflect it.
      await Promise.allSettled([integrationsQ.refetch(), integrationSyncStatusQ.refetch()]);
      setSaving(false);
      return null;
    }
  };

  const submitConfig = async () => {
    await saveActiveConfig(true);
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

  const startIntegrationAuth = async (
    integration: IntegrationItem,
    authWindow?: Window | null
  ) => {
    setNotice(null);
    setOauthBusyId(integration.id);
    try {
      let authUrl = str(integration.auth_url, "").trim();
      if (!authUrl) {
        const payload = await api.rawGet(`/integrations/${encodeURIComponent(integration.id)}/auth`);
        authUrl = extractAuthUrl(payload);
      }
      if (!authUrl) throw new Error("No OAuth URL is available yet. Configure this integration first.");
      setOauthPendingId(normalizeIntegrationId(integration.id));
      if (authWindow && !authWindow.closed) {
        authWindow.location.replace(authUrl);
        authWindow.focus();
      } else {
        window.open(authUrl, "_blank", "noopener,noreferrer");
      }
      setNotice({ kind: "success", text: "OAuth window opened. Finish sign-in and AgentArk will update this automatically." });
    } catch (err) {
      if (authWindow && !authWindow.closed) {
        authWindow.document.write(
          `<!doctype html><title>Sign-in failed</title><body style="font-family:system-ui;background:#0b1320;color:#dce7f7;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;padding:24px;"><div style="max-width:420px;background:#102236;border:1px solid var(--ui-rgba-112-153-201-180);border-radius:16px;padding:20px;"><h2 style="margin:0 0 12px;font-size:18px;">Sign-in could not start</h2><p style="margin:0;color:#a9bdd6;line-height:1.5;">${String(asErrorMessage(err))
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")}</p></div></body>`
        );
        authWindow.document.close();
      }
      setOauthPendingId(null);
      setNotice({ kind: "error", text: asErrorMessage(err) });
    } finally {
      setOauthBusyId(null);
      await queryClient.invalidateQueries({ queryKey: ["integrations"] });
    }
  };

  const continueWithOauth = async () => {
    if (!active) return;
    if (active.id === "google_workspace") {
      const configValues = asRecord(active.config_values);
      const configured = toBool(configValues.oauth_client_configured);
      const savedClientId = str(configValues.client_id, "").trim();
      const nextClientId = str(formValues.client_id, "").trim();
      const nextClientSecret = str(formValues.client_secret, "").trim();
      if (!configured && (!nextClientId || !nextClientSecret)) {
        setFormError("Enter the Google OAuth client ID and client secret first.");
        return;
      }
      if ((nextClientId && !nextClientSecret && nextClientId !== savedClientId) || (!nextClientId && nextClientSecret)) {
        setFormError("Enter both the Google OAuth client ID and client secret, or leave both unchanged.");
        return;
      }
    }
    const authWindow =
      typeof window !== "undefined"
        ? window.open("", "_blank", "width=540,height=760")
        : null;
    if (authWindow) {
      authWindow.document.write(
        "<!doctype html><title>Opening sign-in...</title><body style=\"font-family:system-ui;background:#0b1320;color:#dce7f7;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;\">Opening sign-in...</body>"
      );
      authWindow.document.close();
    }
    const integration =
      active.config_fields && active.config_fields.length > 0
        ? await saveActiveConfig(false)
        : active;
    if (!integration) {
      if (authWindow && !authWindow.closed) {
        authWindow.close();
      }
      return;
    }
    await startIntegrationAuth(integration, authWindow);
  };

  const activeNeedsOauth =
    !!active &&
    ((active.id === "google_workspace" && active.status !== "error") ||
      active.id === "gmail" ||
      active.id === "google_calendar" ||
      active.status === "needs_auth" ||
      !!str(active.auth_url, "").trim());
  const activeWorkspaceSecretConfigured =
    !!active &&
    active.id === "google_workspace" &&
    toBool(asRecord(active.config_values).client_secret_configured);
  const activeIsVerified = active?.status === "connected";
  const activeIsConfigured = active?.status === "configured";
  const activeHasSavedConfig = activeIsVerified || activeIsConfigured;

  const renderField = (field: IntegrationConfigField) => {
    const value = formValues[field.key] || "";
    if (active?.id === "google_workspace" && field.key === "service_bundles") {
      const selected = parseWorkspaceBundleCsv(value);
      return (
        <Stack key={field.key} spacing={1}>
          <Typography variant="subtitle2">{field.label}</Typography>
          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            Choose the Workspace services AgentArk should be allowed to use from this single Google consent flow.
          </Typography>
          <Grid2 container spacing={0.75}>
            {GOOGLE_WORKSPACE_BUNDLES.map((bundle) => {
              const checked = selected.includes(bundle.id);
              return (
                <Grid2 key={bundle.id} size={{ xs: 12, sm: 6 }}>
                  <Box
                    sx={{
                      border: "1px solid var(--ui-rgba-112-153-201-140)",
                      borderRadius: 1.5,
                      background: checked ? "var(--ui-rgba-15-68-110-180)" : "var(--ui-rgba-7-17-32-280)"
                    }}
                  >
                    <FormControlLabel
                      sx={{ m: 0, px: 1.1, py: 0.35, width: "100%" }}
                      control={
                        <Checkbox
                          size="small"
                          checked={checked}
                          onChange={() => {
                            const next = checked
                              ? selected.filter((item) => item !== bundle.id)
                              : [...selected, bundle.id];
                            setFormValues((prev) => ({
                              ...prev,
                              [field.key]: workspaceBundleCsv(next)
                            }));
                          }}
                        />
                      }
                      label={
                        <Stack spacing={0.15}>
                          <Typography variant="body2" sx={{ fontWeight: 600 }}>
                            {bundle.label}
                          </Typography>
                        </Stack>
                      }
                    />
                  </Box>
                </Grid2>
              );
            })}
          </Grid2>
          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            Checked bundles will be requested during Google consent. You can reconnect later to grant more.
          </Typography>
        </Stack>
      );
    }
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
        helperText={
          active?.id === "google_workspace" && field.key === "client_id"
            ? "Copy this from your Google OAuth client in Google Cloud."
            : active?.id === "google_workspace" && field.key === "client_secret"
              ? activeWorkspaceSecretConfigured
                ? "Leave blank to keep the saved secret. Paste a new one only when changing the client."
                : "Copy this from your Google OAuth client in Google Cloud."
              : undefined
        }
      />
    );
  };

  return (
    <Stack
      spacing={2}
      sx={embedded ? undefined : { p: { xs: 1, md: 1.5 }, height: "100%", overflow: "auto" }}
    >
      {!embedded ? (
        <>
          <Stack
            direction="row"
            sx={{
              alignItems: "center",
              justifyContent: "space-between"
            }}>
            <Typography variant="h6">
              {mode === "mcp"
                ? "MCP Servers"
                : mode === "channels"
                  ? "Messaging Channels"
                  : mode === "connectors"
                    ? "Integrations"
                    : mode === "messaging"
                      ? "Messaging Setup"
                      : "Integrations"}
            </Typography>
            <Stack direction="row" spacing={1} />
          </Stack>
          {mode === "channels" ? (
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Configure delivery transports and review live channel health. If something looks off, run ArkPulse for diagnostics.
            </Typography>
          ) : mode === "connectors" ? (
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Built-in connectors and user-added custom integrations the agent can use across chat, tasks, and automation.
            </Typography>
          ) : mode === "messaging" ? (
            <>
              <Typography
                variant="body2"
                sx={{
                  fontWeight: 700,
                  color: "#69e2ff"
                }}
              >
                Finish channel onboarding here. If you need a health check later, run ArkPulse.
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                This page is intentionally focused on messaging only: Telegram, WhatsApp, Slack, Discord, Matrix, and Teams.
              </Typography>
            </>
          ) : mode !== "mcp" ? (
            <>
              <Typography
                variant="body2"
                sx={{
                  fontWeight: 700,
                  color: "#69e2ff"
                }}
              >
                Built-in integrations and custom integrations are managed separately here.
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Use built-in integrations for first-party connectors and the custom integrations panel for pack-based installs.
              </Typography>
            </>
          ) : (
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Configure external MCP servers (HTTP or stdio), auth, allowlists, and refresh tools/resources.
            </Typography>
          )}
        </>
      ) : null}
      {showIntegrations && notice?.kind === "error" ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {showMcp && mcpNotice ? <Alert severity={mcpNotice.kind}>{mcpNotice.text}</Alert> : null}
      {showMcp && sshNotice ? <Alert severity={sshNotice.kind}>{sshNotice.text}</Alert> : null}
      {shouldLoadConnectorCatalog && integrationsQ.error ? (
        <Alert severity="error">
          Failed to load integrations:{" "}
          {integrationsQ.error instanceof Error ? integrationsQ.error.message : "Unknown error"}
        </Alert>
      ) : null}
      {showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "custom_integrations"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "custom_integrations" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Custom Integrations</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Install, connect, and manage pack-based integrations separately from the built-in connector catalog.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label="Pack-based" sx={sectionCountChipSx} />
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <ExtensionPacksPanel mode="integrations" />
          </AccordionDetails>
        </Accordion>
      ) : null}
      {showConnectorsPage ? (
        <>
          <IntegrationQuickstartPanel
            integrations={integrations}
            loading={integrationsQ.isLoading || (integrationsQ.isFetching && !integrationsQ.error)}
            loadError={integrationsQ.error instanceof Error ? integrationsQ.error.message : null}
            autoRefresh={autoRefresh}
            embedded
            onConfigureIntegration={openConfig}
          />
          {readyList.length > 0 ? (
            <Box className="list-shell" sx={{ mb: 1.5 }}>
              <Stack spacing={1.25}>
              <Stack
                direction={{ xs: "column", sm: "row" }}
                spacing={1}
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "flex-start", sm: "center" },
                  mb: 1.25
                }}>
                <Box>
                  <Typography variant="subtitle2">Connected</Typography>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    These integrations are live and available to the agent.
                  </Typography>
                </Box>
                <Chip size="small" variant="outlined" label={`${readyList.length} connected`} sx={sectionCountChipSx} />
              </Stack>
                <Grid2 container spacing={1}>
                  {readyList.map((integration) => {
                    const cardState = integrationCardState(integration);
                    const accent = integrationCardAccent(cardState);
                    const dotColor = integrationCardDotColor(cardState);
                    return (
                      <Grid2 key={`connected-${integration.id}`} size={{ xs: 12, sm: 6, md: 4, lg: 3 }}>
                        <Box
                          role="button"
                          tabIndex={0}
                          onClick={() => {
                            if (integration.config_fields && integration.config_fields.length > 0) {
                              openConfig(integration);
                            }
                          }}
                          onKeyDown={(e) => {
                            if ((e.key === "Enter" || e.key === " ") && integration.config_fields && integration.config_fields.length > 0) {
                              e.preventDefault();
                              openConfig(integration);
                            }
                          }}
                          sx={{
                            height: "100%",
                            p: 1.35,
                            borderRadius: "8px",
                            border: `1px solid ${accent.border}`,
                            background: accent.background,
                            cursor: "pointer",
                            transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                            "&:hover": {
                              borderColor: accent.hoverBorder,
                              background: accent.hoverBackground,
                              boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)"
                            }
                          }}
                        >
                          <Stack spacing={0.75}>
                            <Stack
                              direction="row"
                              spacing={1}
                              sx={{
                                alignItems: "center",
                                justifyContent: "space-between"
                              }}>
                              <Stack direction="row" spacing={0.75} sx={{
                                alignItems: "center"
                              }}>
                                <ConnectorIcon id={integration.id} name={integration.name} />
                                <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
                                  {integration.name}
                                </Typography>
                              </Stack>
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
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                                lineHeight: 1.45,
                                display: "-webkit-box",
                                WebkitLineClamp: 2,
                                WebkitBoxOrient: "vertical",
                                overflow: "hidden"
                              }}>
                              {integrationCardCopy(integration)}
                            </Typography>
                            <Stack
                              direction="row"
                              spacing={1}
                              sx={{
                                justifyContent: "space-between",
                                alignItems: "center"
                              }}>
                              <Chip
                                size="small"
                                label={integrationCardLabel(cardState)}
                                sx={{
                                  height: 20,
                                  fontSize: "0.68rem",
                                  fontWeight: 700,
                                  borderColor: accent.chipBorder,
                                  color: accent.chipColor
                                }}
                                variant="outlined"
                              />
                              <Button size="small" variant="text" sx={{ minWidth: 0 }} onClick={(e) => {
                                e.stopPropagation();
                                openConfig(integration);
                              }}>
                                Manage
                              </Button>
                            </Stack>
                          </Stack>
                        </Box>
                      </Grid2>
                    );
                  })}
                </Grid2>
              </Stack>
            </Box>
          ) : null}
          <ExtensionPacksPanel mode="connectors" />
        </>
      ) : null}
      {showCatalog ? (
        <Box className="list-shell">
          <Stack spacing={1.1}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" }
              }}>
              <Box>
                <Typography variant="subtitle2">Live Summary</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Current connected apps and active integration surfaces.
                </Typography>
              </Box>
              <Chip
                size="small"
                label={`${liveConnectionSummary.length} active groups`}
                sx={sectionCountChipSx}
              />
            </Stack>
            {liveConnectionSummary.length > 0 ? (
              <Grid2 container spacing={1}>
                {liveConnectionSummary.map((item) => (
                  <Grid2 key={item.key} size={{ xs: 12, sm: 6, lg: 3 }}>
                    <Box
                      sx={{
                        p: 1.15,
                        borderRadius: 1.5,
                        border: "1px solid var(--ui-rgba-74-210-157-180)",
                        background: "var(--ui-rgba-10-28-20-260)",
                        minHeight: 84
                      }}
                    >
                      <Stack spacing={0.65} sx={{ height: "100%" }}>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            alignItems: "center",
                            justifyContent: "space-between"
                          }}>
                          <Stack direction="row" spacing={0.8} sx={{
                            alignItems: "center"
                          }}>
                            <Box
                              sx={{
                                width: 8,
                                height: 8,
                                borderRadius: "50%",
                                background: "var(--ui-rgba-74-210-157-920)",
                                flexShrink: 0
                              }}
                            />
                            <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                              {item.label}
                            </Typography>
                          </Stack>
                          <Chip size="small" label={item.badge} sx={sectionCountChipSx} />
                        </Stack>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            lineHeight: 1.45,
                            display: "-webkit-box",
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: "vertical",
                            overflow: "hidden"
                          }}>
                          {item.detail}
                        </Typography>
                      </Stack>
                    </Box>
                  </Grid2>
                ))}
              </Grid2>
            ) : (
              <Typography variant="body2" sx={{
                color: "text.secondary"
              }}>
                No live integrations or automations are active yet.
              </Typography>
            )}
          </Stack>
        </Box>
      ) : null}
      {false && showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "connected"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "connected" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Active Connections</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Live integrations are shown here first. Expand this to manage every connected service.
                </Typography>
              </Box>
              <Chip size="small" label={`${readyList.length} connected`} sx={sectionCountChipSx} />
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
        <Box className="list-shell">
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", sm: "center" },
              mb: 1.25
            }}>
            <Box>
              <Typography variant="subtitle2">Connected Apps</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Live integrations are shown here first. Setup-heavy sections stay collapsed until you need them.
              </Typography>
            </Box>
            <Chip size="small" variant="outlined" label={`${readyList.length} connected`} />
          </Stack>
          {readyList.length > 0 ? (
            <Grid2 container spacing={1}>
              {readyList.map((integration) => {
                const cardState = integrationCardState(integration);
                const accent = integrationCardAccent(cardState);
                const dotColor = integrationCardDotColor(cardState);
                return (
                  <Grid2 key={`connected-${integration.id}`} size={{ xs: 12, sm: 6, md: 4, lg: 3 }}>
                    <Box
                      role="button"
                      tabIndex={0}
                      onClick={() => {
                        if (integration.config_fields && integration.config_fields.length > 0) {
                          openConfig(integration);
                        }
                      }}
                      onKeyDown={(e) => {
                        if ((e.key === "Enter" || e.key === " ") && integration.config_fields && integration.config_fields.length > 0) {
                          e.preventDefault();
                          openConfig(integration);
                        }
                      }}
                      sx={{
                        height: "100%",
                        p: 1.35,
                        borderRadius: "8px",
                        border: `1px solid ${accent.border}`,
                        background: accent.background,
                        cursor: "pointer",
                        transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                        "&:hover": {
                          borderColor: accent.hoverBorder,
                          background: accent.hoverBackground,
                          boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)"
                        }
                      }}
                    >
                      <Stack spacing={0.75}>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            alignItems: "center",
                            justifyContent: "space-between"
                          }}>
                          <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
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
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            lineHeight: 1.45,
                            display: "-webkit-box",
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: "vertical",
                            overflow: "hidden"
                          }}>
                          {integrationCardCopy(integration)}
                        </Typography>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            justifyContent: "space-between",
                            alignItems: "center"
                          }}>
                          <Chip
                            size="small"
                            label="Connected"
                            sx={{
                              height: 20,
                              fontSize: "0.68rem",
                              fontWeight: 700,
                              borderColor: accent.chipBorder,
                              color: accent.chipColor
                            }}
                            variant="outlined"
                          />
                          <Button size="small" variant="text" sx={{ minWidth: 0 }} onClick={(e) => {
                            e.stopPropagation();
                            openConfig(integration);
                          }}>
                            Manage
                          </Button>
                        </Stack>
                      </Stack>
                    </Box>
                  </Grid2>
                );
              })}
            </Grid2>
          ) : (
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              No integrations are connected yet.
            </Typography>
          )}
        </Box>
          </AccordionDetails>
        </Accordion>
      ) : null}
      {false && showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "plugins"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "plugins" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Plugin SDK</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Manage installed external plugins and their event subscriptions here.
                </Typography>
              </Box>
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <PluginSdkPanel autoRefresh={autoRefresh} embedded />
          </AccordionDetails>
        </Accordion>
      ) : null}
      {false && showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "activity"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "activity" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Recent Activity</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Background sync highlights important changes here when attention is needed.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label={`${integrationSyncFeed.length} recent`} />
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            {integrationSyncFeedQ.error ? (
              <Alert severity="warning">
                Could not load recent integration activity:{" "}
                {((integrationSyncFeedQ.error as Error | null)?.message) || "Unknown error"}
              </Alert>
            ) : integrationSyncFeed.length > 0 ? (
              <Table
                size="small"
                sx={{ "& td, & th": { borderColor: "var(--ui-rgba-112-153-201-120)", py: 0.75 } }}
              >
                <TableHead>
                  <TableRow>
                    <TableCell sx={{ fontWeight: 600, width: "18%" }}>Source</TableCell>
                    <TableCell sx={{ fontWeight: 600 }}>Update</TableCell>
                    <TableCell sx={{ fontWeight: 600, width: "12%" }}>Importance</TableCell>
                    <TableCell sx={{ fontWeight: 600, width: "16%" }}>Detected</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {integrationSyncFeed.map((item: IntegrationSyncFeedItem) => (
                    <TableRow key={item.id}>
                      <TableCell>
                        <Typography variant="body2">{item.integration_name}</Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            textTransform: "capitalize"
                          }}>
                          {item.kind.replace(/_/g, " ")}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2">{item.title}</Typography>
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          {item.summary}
                        </Typography>
                      </TableCell>
                      <TableCell>
                        <Chip
                          size="small"
                          color={item.important ? "warning" : "default"}
                          variant={item.important ? "filled" : "outlined"}
                          label={`${Math.round(item.importance * 100)}%`}
                        />
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2">{formatDateTime(item.detected_at)}</Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            textTransform: "capitalize"
                          }}>
                          {item.outcome.replace(/_/g, " ")}
                        </Typography>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <Typography variant="body2" sx={{
                color: "text.secondary"
              }}>
                No recent integration activity yet.
              </Typography>
            )}
          </AccordionDetails>
        </Accordion>
      ) : null}
      {showChannelsPage ? (
        <Box className="list-shell">
          {settingsQ.error ? (
            <Alert severity="error">
              Failed to load channels: {(settingsQ.error as Error)?.message || "Unknown error"}
            </Alert>
          ) : null}
          {connectedMessagingSetups.length > 0 ? (
            <>
              <Typography variant="subtitle2" sx={{ mb: 1 }}>
                Connected Channels
              </Typography>
              <Table size="small" sx={{ "& td, & th": { borderColor: "var(--ui-rgba-112-153-201-120)", py: 0.75 } }}>
                <TableHead>
                  <TableRow>
                    <TableCell sx={{ fontWeight: 600, width: "22%" }}>Channel</TableCell>
                    <TableCell sx={{ fontWeight: 600, width: "18%" }}>Status</TableCell>
                    <TableCell sx={{ fontWeight: 600 }}>Details</TableCell>
                    <TableCell sx={{ fontWeight: 600, width: "10%", textAlign: "right" }} />
                  </TableRow>
                </TableHead>
                <TableBody>
                  {connectedMessagingSetups.map((channel) => (
                    <TableRow key={channel.id}>
                      <TableCell>
                        <Stack direction="row" spacing={1} sx={{
                          alignItems: "center"
                        }}>
                          <ChannelIcon name={channel.name} />
                          <Typography variant="body2">{channel.name}</Typography>
                        </Stack>
                      </TableCell>
                      <TableCell>
                        <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                          <Box component="span" sx={{ width: 8, height: 8, borderRadius: "50%", flexShrink: 0, bgcolor: "var(--ui-rgba-74-210-157-850)" }} />
                          <Typography variant="body2" noWrap sx={{
                            color: "text.secondary"
                          }}>{channelStatusLabel(channel.status, channel.enabled)}</Typography>
                        </Box>
                      </TableCell>
                      <TableCell>
                        <Typography variant="body2" noWrap sx={{
                          color: "text.secondary"
                        }}>{channel.detail}</Typography>
                      </TableCell>
                      <TableCell align="right">
                        <Button size="small" variant="text" onClick={channel.open} sx={{ minWidth: 0 }}>
                          {channel.actionLabel || "Setup"}
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
              <Divider sx={{ my: 1.5, borderColor: "var(--ui-rgba-112-153-201-120)" }} />
            </>
          ) : null}
          <Stack spacing={1.25}>
            <Box>
              <Typography variant="subtitle2">Email Delivery</Typography>
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                Send AgentArk emails through Gmail, Google Workspace, or a provider account you control.
              </Typography>
            </Box>
            <Grid2 container spacing={1.25} sx={{ alignItems: "stretch" }}>
              <Grid2 size={{ xs: 12, md: 6, xl: 4 }} sx={{ display: "flex" }}>
                <Box
                  role="button"
                  tabIndex={0}
                  onClick={openEmailSetup}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      openEmailSetup();
                    }
                  }}
                  sx={{
                    height: "100%",
                    width: "100%",
                    p: 1.5,
                    borderRadius: 1.5,
                    border: emailDeliveryReady
                      ? "1px solid var(--ui-rgba-64-196-255-240)"
                      : "1px solid var(--ui-rgba-112-153-201-160)",
                    background: emailDeliveryReady ? "var(--ui-rgba-8-24-42-560)" : "var(--ui-rgba-7-17-32-600)",
                    cursor: "pointer",
                    transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                    "&:hover": {
                      borderColor: emailDeliveryReady
                        ? "var(--ui-rgba-64-196-255-360)"
                        : "var(--ui-rgba-112-153-201-260)",
                      background: emailDeliveryReady ? "var(--ui-rgba-8-28-48-660)" : "var(--ui-rgba-9-22-40-720)",
                      boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)"
                    }
                  }}
                >
                  <Stack spacing={1.1} sx={{ height: "100%", justifyContent: "space-between" }}>
                    <Box>
                      <Stack
                        direction="row"
                        spacing={0.9}
                        sx={{
                          alignItems: "center",
                          mb: 0.75
                        }}
                      >
                        <ChannelIcon name="Email" size={20} />
                        <Typography variant="subtitle2" noWrap>
                          Email Delivery
                        </Typography>
                      </Stack>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                          display: "-webkit-box",
                          WebkitLineClamp: 3,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden"
                        }}
                      >
                        {emailConnectionDetail}
                      </Typography>
                    </Box>
                    <Stack
                      direction="row"
                      spacing={1}
                      sx={{
                        justifyContent: "space-between",
                        alignItems: "center"
                      }}
                    >
                      <Stack direction="row" spacing={0.5} useFlexGap sx={{ flexWrap: "wrap" }}>
                        <Chip size="small" label="Email" sx={sectionTagChipSx} />
                        <Chip size="small" label={emailStatusLabel} sx={sectionCountChipSx} />
                        <Chip
                          size="small"
                          label={emailProviderLabel(emailSelectedProvider)}
                          sx={sectionTagChipSx}
                        />
                      </Stack>
                      <Button
                        size="small"
                        variant={emailDeliveryReady ? "outlined" : "contained"}
                        sx={connectorCardActionButtonSx}
                        onClick={(e) => {
                          e.stopPropagation();
                          openEmailSetup();
                        }}
                      >
                        {emailDeliveryReady ? "Open wizard" : "Set up"}
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              </Grid2>
            </Grid2>
          </Stack>
          <Divider sx={{ my: 1.5, borderColor: "var(--ui-rgba-112-153-201-120)" }} />
          <Stack spacing={1.25}>
            <Box>
              <Typography variant="subtitle2">Setup Wizards</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Connect each messaging channel here. Every inbound DM, group, room, or space becomes its own AgentArk conversation thread, while still sharing your global docs, memories, apps, tasks, and watchers.
              </Typography>
            </Box>
            {channelsQ.error ? (
              <Alert severity="error">
                Failed to load gateway channel health: {(channelsQ.error as Error)?.message || "Unknown error"}
              </Alert>
            ) : null}
            <Grid2 container spacing={1.25} sx={{
              alignItems: "stretch"
            }}>
              {messagingSetups.map((setup) => {
                const displayState = messagingDisplayState(setup.status, setup.enabled);
                const statusLabel = channelStatusLabel(setup.status, setup.enabled);
                const isConfigured = displayState !== "off";
                return (
                  <Grid2 key={setup.id} size={{ xs: 12, md: 6, xl: 4 }} sx={{ display: "flex" }}>
                    <Box
                      role="button"
                      tabIndex={0}
                      onClick={setup.open}
                      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setup.open(); } }}
                      sx={{
                        height: "100%",
                        width: "100%",
                        p: 1.5,
                        borderRadius: 1.5,
                        border: isConfigured ? "1px solid var(--ui-rgba-64-196-255-240)" : "1px solid var(--ui-rgba-112-153-201-160)",
                        background: isConfigured ? "var(--ui-rgba-8-24-42-560)" : "var(--ui-rgba-7-17-32-600)",
                        cursor: "pointer",
                        transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                        "&:hover": {
                          borderColor: isConfigured ? "var(--ui-rgba-64-196-255-360)" : "var(--ui-rgba-112-153-201-260)",
                          background: isConfigured ? "var(--ui-rgba-8-28-48-660)" : "var(--ui-rgba-9-22-40-720)",
                          boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)"
                        }
                      }}
                    >
                      <Stack spacing={1.1} sx={{ height: "100%", justifyContent: "space-between" }}>
                        <Box>
                          <Stack
                            direction="row"
                            spacing={0.9}
                            sx={{
                              alignItems: "center",
                              mb: 0.75
                            }}>
                            <ChannelIcon name={setup.name} size={20} />
                            <Typography variant="subtitle2" noWrap>
                              {setup.name}
                            </Typography>
                          </Stack>
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                              display: "-webkit-box",
                              WebkitLineClamp: 3,
                              WebkitBoxOrient: "vertical",
                              overflow: "hidden"
                            }}>
                            {setup.detail}
                          </Typography>
                        </Box>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            justifyContent: "space-between",
                            alignItems: "center"
                          }}>
                          <Stack direction="row" spacing={0.5} useFlexGap sx={{
                            flexWrap: "wrap"
                          }}>
                            <Chip size="small" label="Channel" sx={sectionTagChipSx} />
                            {statusLabel ? <Chip size="small" label={statusLabel} sx={sectionCountChipSx} /> : null}
                          </Stack>
                          <Button
                            size="small"
                            variant={displayState === "off" ? "contained" : "outlined"}
                            sx={connectorCardActionButtonSx}
                            onClick={(e) => { e.stopPropagation(); setup.open(); }}
                          >
                            {setup.actionLabel}
                          </Button>
                        </Stack>
                      </Stack>
                    </Box>
                  </Grid2>
                );
              })}
            </Grid2>
          </Stack>
          <Divider sx={{ my: 1.5, borderColor: "var(--ui-rgba-112-153-201-120)" }} />
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ justifyContent: "space-between", alignItems: { xs: "flex-start", sm: "center" } }}
            >
              <Box>
                <Typography variant="subtitle2">Custom Messaging Channels</Typography>
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  User-added delivery channels created from chat and secured through encrypted credential forms.
                </Typography>
              </Box>
              <Chip size="small" label={`${customMessagingChannels.length} custom`} sx={sectionCountChipSx} />
            </Stack>
            {customMessagingChannels.length > 0 ? (
              <Alert severity="info">
                Custom messaging channels send notifications to the endpoint
                configured for that channel. Store tokens and webhook secrets
                only through credential fields.
              </Alert>
            ) : null}
            {customMessagingChannelsQ.error ? (
              <Alert severity="error">
                Failed to load custom messaging channels: {asErrorMessage(customMessagingChannelsQ.error)}
              </Alert>
            ) : customMessagingChannelsQ.isLoading ? (
              <Typography variant="body2" sx={{ color: "text.secondary" }}>Loading custom channels...</Typography>
            ) : customMessagingChannels.length === 0 ? (
              <Alert severity="info">
                Ask AgentArk in chat to add a messaging channel from provider docs, a webhook, or an internal notification API.
              </Alert>
            ) : (
              <Grid2 container spacing={1.25} sx={{ alignItems: "stretch" }}>
                {customMessagingChannels.map((channel) => {
                  const fields = customMessagingCredentialFields(channel);
                  const ready = !!channel.configured;
                  const accent = integrationCardAccent(ready ? "enabled" : "disabled");
                  return (
                    <Grid2 key={channel.id} size={{ xs: 12, md: 6, xl: 4 }} sx={{ display: "flex" }}>
                      <Box
                        sx={{
                          width: "100%",
                          p: 1.5,
                          borderRadius: "8px",
                          border: `1px solid ${accent.border}`,
                          background: accent.background
                        }}
                      >
                        <Stack spacing={1}>
                          <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", alignItems: "center" }}>
                            <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", minWidth: 0 }}>
                              <ChannelIcon name={channel.name || channel.id} size={22} />
                              <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
                                {channel.name || channel.id}
                              </Typography>
                            </Stack>
                            <Chip
                              size="small"
                              label={ready ? "Ready" : channel.requires_auth ? "Needs credentials" : "Disabled"}
                              variant="outlined"
                              sx={{ height: 20, fontSize: "0.68rem", fontWeight: 700, borderColor: accent.chipBorder, color: accent.chipColor }}
                            />
                          </Stack>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                              lineHeight: 1.45,
                              display: "-webkit-box",
                              WebkitLineClamp: 2,
                              WebkitBoxOrient: "vertical",
                              overflow: "hidden"
                            }}
                          >
                            {channel.description || channel.runtime_channel_id}
                          </Typography>
                          {channel.last_test_message ? (
                            <Typography variant="caption" sx={{ color: "text.secondary" }} noWrap>
                              Last test: {channel.last_test_message}
                            </Typography>
                          ) : null}
                          <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", justifyContent: "flex-end" }}>
                            <Chip
                              size="small"
                              label="User-added"
                              variant="outlined"
                              sx={{ height: 20, fontSize: "0.68rem", fontWeight: 700 }}
                            />
                            <Chip
                              size="small"
                              label={channel.runtime_channel_id}
                              variant="outlined"
                              sx={{ height: 20, fontSize: "0.68rem", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}
                            />
                            {channel.docs_url ? (
                              <Button
                                size="small"
                                variant="text"
                                sx={connectorCardActionButtonSx}
                                onClick={() => window.open(channel.docs_url || "", "_blank", "noopener,noreferrer")}
                              >
                                Docs
                              </Button>
                            ) : null}
                            {fields.length > 0 ? (
                              <Button
                                size="small"
                                variant={ready ? "outlined" : "contained"}
                                sx={connectorCardActionButtonSx}
                                onClick={() => openCustomMessagingCredentials(channel)}
                              >
                                Credentials
                              </Button>
                            ) : null}
                            <Button
                              size="small"
                              variant="outlined"
                              sx={connectorCardActionButtonSx}
                              disabled={!ready || testCustomMessagingChannelMutation.isPending}
                              onClick={() => testCustomMessagingChannelMutation.mutate(channel.id)}
                            >
                              Test
                            </Button>
                            <CardActionsMenu
                              ariaLabel={`${channel.name || channel.id} options`}
                              actions={[
                                {
                                  label: "Delete",
                                  tone: "error",
                                  disabled: deleteCustomMessagingChannelMutation.isPending,
                                  onClick: () => {
                                    if (!window.confirm(`Delete custom messaging channel '${channel.name || channel.id}'?`)) return;
                                    deleteCustomMessagingChannelMutation.mutate(channel.id);
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
          </Stack>
        </Box>
      ) : null}
      {/* Messaging Channels accordion removed - dedicated "Messaging Channels" tab exists in sidebar */}
      {false ? (<Box>
        <Box className="list-shell">
          <Typography variant="subtitle2" sx={{ mb: 1 }}>
            Channels
          </Typography>
          {settingsQ.error ? (
            <Alert severity="error">
              Failed to load channels: {(settingsQ.error as Error)?.message || "Unknown error"}
            </Alert>
          ) : (
            <Table size="small" sx={{ "& td, & th": { borderColor: "var(--ui-rgba-112-153-201-120)", py: 0.75 } }}>
              <TableHead>
                <TableRow>
                  <TableCell sx={{ fontWeight: 600, width: "22%" }}>Channel</TableCell>
                  <TableCell sx={{ fontWeight: 600, width: "18%" }}>Status</TableCell>
                  <TableCell sx={{ fontWeight: 600 }}>Details</TableCell>
                  <TableCell sx={{ fontWeight: 600, width: "10%", textAlign: "right" }} />
                </TableRow>
              </TableHead>
              <TableBody>
                <TableRow>
                  <TableCell>Telegram</TableCell>
                  <TableCell>
                    <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                      <Box component="span" sx={{ width: 8, height: 8, borderRadius: "50%", flexShrink: 0, bgcolor: telegramEnabledSaved ? "var(--ui-rgba-74-210-157-850)" : "var(--ui-rgba-180-200-220-500)" }} />
                      <Typography variant="body2" noWrap sx={{
                        color: "text.secondary"
                      }}>{channelStatusLabel(telegramConnectionStatusRaw, telegramEnabledSaved)}</Typography>
                    </Box>
                  </TableCell>
                  <TableCell>
                    <Typography variant="body2" noWrap sx={{
                      color: "text.secondary"
                    }}>
                      {telegramConnectionDetail || (telegramTokenConfigured ? "Token set" : "Not configured")}
                    </Typography>
                  </TableCell>
                  <TableCell align="right">
                    <Button size="small" variant="text" onClick={() => openTelegramSetup(!channelForm.telegram_enabled)} sx={{ minWidth: 0 }}>
                      {channelForm.telegram_enabled ? "Setup" : "Enable"}
                    </Button>
                  </TableCell>
                </TableRow>
                <TableRow>
                  <TableCell>WhatsApp</TableCell>
                  <TableCell>
                    <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.75 }}>
                      <Box
                        component="span"
                        sx={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          flexShrink: 0,
                          bgcolor:
                            messagingDisplayState(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled) === "ready"
                              ? "var(--ui-rgba-74-210-157-850)"
                              : channelForm.whatsapp_enabled
                                ? "var(--ui-rgba-255-180-50-850)"
                                : "var(--ui-rgba-180-200-220-500)"
                        }}
                      />
                      <Typography variant="body2" noWrap sx={{
                        color: "text.secondary"
                      }}>{channelStatusLabel(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled)}</Typography>
                    </Box>
                  </TableCell>
                  <TableCell>
                    <Typography variant="body2" noWrap sx={{
                      color: "text.secondary"
                    }}>
                      {whatsappConnectionDetail || whatsappModeSummary}
                    </Typography>
                  </TableCell>
                  <TableCell align="right">
                    <Button size="small" variant="text" onClick={() => openWhatsAppSetup(!channelForm.whatsapp_enabled)} sx={{ minWidth: 0 }}>
                      {channelForm.whatsapp_enabled ? "Setup" : "Enable"}
                    </Button>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          )}
          <Divider sx={{ my: 1.5, borderColor: "var(--ui-rgba-112-153-201-120)" }} />
          <Stack spacing={1.25}>
            <Box>
              <Typography variant="subtitle2">Setup Wizards</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Onboard Slack, Discord, Matrix, and Teams here. If something looks off later, run ArkPulse for diagnostics.
              </Typography>
            </Box>
            {channelsQ.error ? (
              <Alert severity="error">
                Failed to load gateway channel health: {(channelsQ.error as Error)?.message || "Unknown error"}
              </Alert>
            ) : null}
            <Grid2 container spacing={1} sx={{
              alignItems: "stretch"
            }}>
              {messagingSetups.map((setup) => {
                const displayState = messagingDisplayState(setup.status, setup.enabled);
                const accent = integrationCardAccent(
                  displayState === "ready" ? "enabled" : "disabled"
                );
                const dotColor = integrationCardDotColor(
                  displayState === "ready" ? "enabled" : "disabled"
                );
                return (
                  <Grid2 key={setup.id} size={{ xs: 12, sm: 6, md: 4, lg: 3 }} sx={{ display: "flex" }}>
                    <Box
                      role="button"
                      tabIndex={0}
                      onClick={setup.open}
                      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setup.open(); } }}
                      sx={{
                        height: "100%",
                        width: "100%",
                        p: 1.35,
                        borderRadius: "8px",
                        border: `1px solid ${accent.border}`,
                        background: accent.background,
                        cursor: "pointer",
                        transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                        "&:hover": {
                          borderColor: accent.hoverBorder,
                          background: accent.hoverBackground,
                          boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)"
                        }
                      }}
                    >
                      <Stack spacing={0.75}>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            alignItems: "center",
                            justifyContent: "space-between"
                          }}>
                          <Stack direction="row" spacing={0.75} sx={{
                            alignItems: "center"
                          }}>
                            <ChannelIcon name={setup.name} size={22} />
                            <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
                              {setup.name}
                            </Typography>
                          </Stack>
                          <Box sx={{ width: 8, height: 8, borderRadius: "50%", background: dotColor, flex: "0 0 auto" }} />
                        </Stack>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            lineHeight: 1.45,
                            display: "-webkit-box",
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: "vertical",
                            overflow: "hidden"
                          }}>
                          {setup.detail}
                        </Typography>
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            justifyContent: "space-between",
                            alignItems: "center"
                          }}>
                          {channelStatusLabel(setup.status, setup.enabled) ? (
                            <Chip
                              size="small"
                              label={channelStatusLabel(setup.status, setup.enabled)}
                              sx={{ height: 20, fontSize: "0.68rem", fontWeight: 700, borderColor: accent.chipBorder, color: accent.chipColor }}
                              variant="outlined"
                            />
                          ) : <Box sx={{ flex: 1 }} />}
                          <Button size="small" variant="text" sx={{ minWidth: 0 }} onClick={(e) => { e.stopPropagation(); setup.open(); }}>
                            {setup.actionLabel}
                          </Button>
                        </Stack>
                      </Stack>
                    </Box>
                  </Grid2>
                );
              })}
            </Grid2>
          </Stack>
        </Box>
      </Box>) : null}
      {showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "sources"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "sources" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Prebuilt Connectors</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Connect Google Workspace, GitHub, Jira, Sentry, and other built-in integrations here.
                </Typography>
              </Box>
              <Chip size="small" label={`${integrations.length} available`} sx={sectionCountChipSx} />
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <IntegrationQuickstartPanel
              integrations={integrations}
              loading={integrationsQ.isLoading || integrationsQ.isFetching}
              autoRefresh={autoRefresh}
              embedded
              onConfigureIntegration={openConfig}
            />
          </AccordionDetails>
        </Accordion>
      ) : null}
      {showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "plugins"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "plugins" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Plugin SDK</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Manage installed external plugins and their event subscriptions here.
                </Typography>
              </Box>
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <PluginSdkPanel autoRefresh={autoRefresh} embedded />
          </AccordionDetails>
        </Accordion>
      ) : null}
      {showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "routing"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "routing" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Conversation Routing</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Keep channel-specific traffic pinned to the right agent only when you explicitly need it.
                </Typography>
              </Box>
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <IntegrationRoutingPanel autoRefresh={autoRefresh} />
          </AccordionDetails>
        </Accordion>
      ) : null}
      {/* Messaging onboarding section removed - duplicates Setup Wizards above */}
      {false && showCatalog ? (
        <Accordion
          disableGutters
          expanded={expandedSection === "catalog"}
          onChange={(_, expanded) => setExpandedSection(expanded ? "catalog" : false)}
          sx={sectionAccordionSx}
        >
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
                width: "100%"
              }}>
              <Box>
                <Typography variant="subtitle2">Integration Catalog</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Review everything available, including connectors that still need setup or attention.
                </Typography>
              </Box>
              <Stack direction="row" spacing={0.75} useFlexGap sx={{
                flexWrap: "wrap"
              }}>
                <Chip size="small" variant="outlined" label={`${readyList.length} ready`} />
                <Chip size="small" variant="outlined" label={`${notReadyList.length} need attention`} />
              </Stack>
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
        <Box className="list-shell">
          <Stack
            direction="row"
            sx={{
              justifyContent: "space-between",
              alignItems: "center",
              mb: 1.5
            }}>
            <Typography variant="subtitle2">
              Available Integrations
            </Typography>
          </Stack>
          <Grid2 container spacing={1}>
            {[...readyList, ...notReadyList].map((integration) => {
              const cardState = integrationCardState(integration);
              const isEnabled = cardState === "enabled";
              const accent = integrationCardAccent(cardState);
              const dotColor = integrationCardDotColor(cardState);
              return (
                <Grid2 key={integration.id} size={{ xs: 6, sm: 4, md: 3, lg: 2 }}>
                  <Box
                    role="button"
                    tabIndex={0}
                    onClick={() => {
                      if (cardState === "enabled") {
                        if (integration.config_fields && integration.config_fields.length > 0) {
                          openConfig(integration);
                        }
                      } else {
                        if (cardState === "disabled") {
                          enableMutation.mutate(integration.id);
                        } else {
                          openConfig(integration);
                        }
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        if (cardState === "enabled") {
                          if (integration.config_fields && integration.config_fields.length > 0) {
                            openConfig(integration);
                          }
                        } else {
                          if (cardState === "disabled") {
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
                      borderRadius: "8px",
                      border: `1px solid ${accent.border}`,
                      background: accent.background,
                      cursor: "pointer",
                      transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
                      opacity: cardState === "enabled" ? 1 : 0.86,
                      "&:hover": {
                        borderColor: accent.hoverBorder,
                        background: accent.hoverBackground,
                        boxShadow: "0 2px 12px var(--ui-rgba-0-0-0-200)",
                        opacity: 1
                      }
                    }}
                  >
                    <Stack spacing={0.5} sx={{ minHeight: 56 }}>
                      <Stack
                        direction="row"
                        spacing={0.5}
                        sx={{
                          alignItems: "center",
                          justifyContent: "space-between"
                        }}>
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
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          lineHeight: 1.3,
                          display: "-webkit-box",
                          WebkitLineClamp: 2,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden"
                        }}>
                        {integrationCardCopy(integration)}
                      </Typography>
                    </Stack>
                    <Chip
                      size="small"
                      label={integrationCardLabel(cardState)}
                      sx={{
                        mt: 0.8,
                        height: 20,
                        fontSize: "0.66rem",
                        fontWeight: 600,
                        borderColor: accent.chipBorder,
                        color: accent.chipColor
                      }}
                      variant="outlined"
                    />
                  </Box>
                </Grid2>
              );
            })}
          </Grid2>
        </Box>
          </AccordionDetails>
        </Accordion>
      ) : null}
      {showMcp ? (
        <Box className="list-shell">
        <Stack
          direction="row"
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
            mb: 1
          }}>
          <Typography variant="subtitle2">MCP Servers ({mcpSorted.length})</Typography>
          <Button size="small" variant="contained" onClick={openCreateMcp}>
            Add MCP Server
          </Button>
        </Stack>
        <Typography variant="caption" sx={{
          color: "text.secondary"
        }}>
          Manage external MCP servers (HTTP or stdio). Tools/resources hot-reload after create/update.
        </Typography>
        {mcpNotice ? <Alert sx={{ mt: 1 }} severity={mcpNotice.kind}>{mcpNotice.text}</Alert> : null}
        {mcpQ.error ? (
          <Alert sx={{ mt: 1 }} severity="error">
            Failed to load MCP servers: {mcpQ.error instanceof Error ? mcpQ.error.message : "Unknown error"}
          </Alert>
        ) : mcpSorted.length === 0 ? (
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              mt: 1
            }}>
            No MCP servers configured.
          </Typography>
        ) : (
          <Grid2 container spacing={1.5} sx={{ mt: 0.5 }}>
            {mcpSorted.map((server) => {
              const id = str(server.id, "");
              const warnings = asStringList(server.warnings);
              const lastError = str(server.last_error, "");
              const tools = asRecords(server.tools);
              const resources = asRecords(server.resources);
              return (
                <Grid2 key={id || str(server.name, Math.random().toString())} size={{ xs: 12, md: 6 }}>
                  <Box className="list-shell" sx={{ minHeight: 0 }}>
                    <Stack spacing={1}>
                      <Stack
                        direction="row"
                        sx={{
                          alignItems: "center",
                          justifyContent: "space-between"
                        }}>
                        <Typography variant="subtitle1" sx={{
                          fontWeight: 700
                        }}>
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
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        {str(server.description, "No description")}
                      </Typography>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        {transportSummary(server)}
                      </Typography>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Auth: {authSummary(server)} | Tools: {str(server.tool_count, "0")} | Resources: {str(server.resource_count, "0")}
                      </Typography>
                      {lastError ? <Alert severity="error">{lastError}</Alert> : null}
                      {warnings.length > 0 ? (
                        <Alert severity="warning">{warnings.slice(0, 2).join(" ")}</Alert>
                      ) : null}
                      {tools.length > 0 ? (
                        <Box sx={{ border: "1px solid", borderColor: "divider", borderRadius: 1, p: 1 }}>
                          <Typography variant="caption" sx={{ fontWeight: 700 }}>Reviewed tools</Typography>
                          <Stack spacing={0.75} sx={{ mt: 0.75 }}>
                            {tools.slice(0, 4).map((tool) => (
                              <Box key={str(tool.name)} sx={{ minWidth: 0 }}>
                                <Typography variant="caption" sx={{ fontWeight: 650 }}>{str(tool.name)}</Typography>
                                <Typography variant="caption" sx={{ color: "text.secondary", display: "block", overflowWrap: "anywhere" }}>
                                  {str(tool.description, "No description")}
                                </Typography>
                                <Typography component="pre" variant="caption" sx={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", m: 0, color: "text.secondary" }}>
                                  {JSON.stringify(tool.input_schema ?? {}, null, 2).slice(0, 700)}
                                </Typography>
                              </Box>
                            ))}
                          </Stack>
                        </Box>
                      ) : null}
                      {resources.length > 0 ? (
                        <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5 }}>
                          {resources.slice(0, 8).map((resource) => (
                            <Chip key={str(resource.uri)} size="small" variant="outlined" label={`Resource: ${str(resource.name, str(resource.uri))}`} />
                          ))}
                        </Stack>
                      ) : null}
                      <Stack direction="row" sx={{
                        justifyContent: "flex-end"
                      }}>
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
          <Stack
            direction="row"
            sx={{
              alignItems: "center",
              justifyContent: "space-between",
              mb: 1
            }}>
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
          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            Upload Ed25519 or ECDSA private keys, create named connection profiles, and run connectivity tests.
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
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
                mt: 1
              }}>
              No SSH keys or connections configured.
            </Typography>
          ) : (
            <Grid2 container spacing={1.5} sx={{ mt: 0.5 }}>
              {sshKeyNames.map((name) => (
                <Grid2 key={`key-${name}`} size={{ xs: 12, md: 6 }}>
                  <Box className="list-shell" sx={{ minHeight: 0 }}>
                    <Stack spacing={0.5}>
                      <Stack
                        direction="row"
                        sx={{
                          alignItems: "center",
                          justifyContent: "space-between"
                        }}>
                        <Typography variant="subtitle1" sx={{
                          fontWeight: 700
                        }}>{name}</Typography>
                        <Chip size="small" label="key" color="default" />
                      </Stack>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>SSH private key</Typography>
                      <Stack direction="row" sx={{
                        justifyContent: "flex-end"
                      }}>
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
                      <Stack
                        direction="row"
                        sx={{
                          alignItems: "center",
                          justifyContent: "space-between"
                        }}>
                        <Typography variant="subtitle1" sx={{
                          fontWeight: 700
                        }}>{name}</Typography>
                        <Chip size="small" label="connection" color="success" />
                      </Stack>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>SSH connection profile</Typography>
                      <Stack direction="row" sx={{
                        justifyContent: "flex-end"
                      }}>
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
      <Dialog
        open={!!customMessagingCredentialTarget}
        onClose={() => {
          setCustomMessagingCredentialTarget(null);
          setCustomMessagingCredentialValues({});
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {customMessagingCredentialTarget?.name || "Custom messaging channel"} Credentials
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Values are stored encrypted and are not written to chat.
            </Typography>
            {customMessagingCredentialTarget?.auth_manifest?.warning ? (
              <Alert severity="info">{customMessagingCredentialTarget.auth_manifest.warning}</Alert>
            ) : null}
            {customMessagingCredentialFields(customMessagingCredentialTarget).map((field) => {
              const inputKind = authFieldInputKind(field);
              return (
                <TextField
                  key={field.key}
                  fullWidth
                  size="small"
                  label={field.label || field.key}
                  value={customMessagingCredentialValues[field.key] || ""}
                  type={inputKind === "password" ? "password" : "text"}
                  multiline={inputKind === "textarea"}
                  minRows={inputKind === "textarea" ? 4 : undefined}
                  placeholder={field.placeholder || ""}
                  helperText={field.help || undefined}
                  required={field.required !== false}
                  onChange={(event) =>
                    setCustomMessagingCredentialValues((current) => ({
                      ...current,
                      [field.key]: event.target.value
                    }))
                  }
                />
              );
            })}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setCustomMessagingCredentialTarget(null);
              setCustomMessagingCredentialValues({});
            }}
            sx={dialogActionButtonSx}
          >
            Close
          </Button>
          <Button
            variant="contained"
            onClick={submitCustomMessagingCredentials}
            disabled={saveCustomMessagingCredentialsMutation.isPending}
            sx={dialogActionButtonSx}
          >
            {saveCustomMessagingCredentialsMutation.isPending ? "Saving..." : "Save credentials"}
          </Button>
        </DialogActions>
      </Dialog>
      {showIntegrations ? (
      <Dialog open={emailSetupOpen} onClose={() => setEmailSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Email Delivery</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {notice?.kind === "error" ? <Alert severity="error">{notice.text}</Alert> : null}
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Send AgentArk notification emails through a connected Google mailbox or a provider account you control.
            </Typography>
            <Stack direction="row" spacing={1} sx={{ alignItems: "center", flexWrap: "wrap" }}>
              <Chip
                size="small"
                label={emailStatusLabel}
                color={emailStatusColor}
                variant="outlined"
              />
              <Chip
                size="small"
                label={
                  emailProviderSaved === "auto" && emailAutoResolvesTo
                    ? `Auto -> ${emailProviderLabel(emailAutoResolvesTo)}`
                    : emailProviderLabel(emailProviderSaved)
                }
                variant="outlined"
              />
            </Stack>
            <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
              {emailAvailableBackendLabels.length > 0 ? (
                emailAvailableBackendLabels.map((label) => (
                  <Chip key={label} size="small" label={label} variant="outlined" sx={sectionCountChipSx} />
                ))
              ) : (
                <Chip size="small" label="No ready backends yet" variant="outlined" sx={sectionCountChipSx} />
              )}
            </Stack>
            <Alert severity={emailDeliveryReady ? "success" : emailDraftIssues.length > 0 ? "warning" : "info"}>
              {emailConnectionDetail}
            </Alert>
            <TextField
              select
              fullWidth
              size="small"
              label="Provider"
              value={channelForm.email_provider}
              onChange={(e) => setChannelField("email_provider", e.target.value)}
              helperText={emailProviderHelperText}
            >
              {EMAIL_PROVIDER_OPTIONS.map((option) => (
                <MenuItem key={option.value} value={option.value}>
                  {option.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              fullWidth
              size="small"
              type="email"
              label="Recipient Email"
              value={channelForm.email_to_address}
              onChange={(e) => setChannelField("email_to_address", e.target.value)}
              placeholder="you@example.com"
              helperText={emailRecipientHelperText}
            />
            <TextField
              fullWidth
              size="small"
              type="email"
              label="From Address"
              value={channelForm.email_from_address}
              onChange={(e) => setChannelField("email_from_address", e.target.value)}
              placeholder={
                emailUsesExternalProvider ? "notifications@example.com" : "Optional if the connected mailbox should send"
              }
              helperText={
                emailUsesExternalProvider
                  ? "Use an address on the domain you want AgentArk to send from."
                  : "Leave blank to use the connected mailbox unless you already configured a Google alias."
              }
            />
            <TextField
              fullWidth
              size="small"
              label="Verified Domain"
              value={channelForm.email_domain}
              onChange={(e) => setChannelField("email_domain", e.target.value)}
              placeholder="example.com"
              helperText="Optional for Gmail and Google Workspace. Use this for provider accounts that send from your own domain."
            />

            {emailUsesExternalProvider && !emailRecipientReady ? (
              <Alert severity="info" sx={{ py: 0.75 }}>
                Set a recipient email unless one connected Gmail or Google Workspace mailbox should receive these notifications.
              </Alert>
            ) : null}

            {emailSelectedProvider === "resend" || emailSelectedProvider === "postmark" ? (
              <>
                <Divider />
                <TextField
                  fullWidth
                  size="small"
                  type="password"
                  label="API Key"
                  value={channelForm.email_auth_api_key}
                  onChange={(e) => setChannelField("email_auth_api_key", e.target.value)}
                  placeholder={hasEmailApiKey ? "Configured (leave blank to keep)" : "Enter API key"}
                  helperText={
                    hasEmailApiKey
                      ? "Leave blank to keep the saved API key."
                      : `Required for ${emailProviderLabel(emailSelectedProvider)} delivery.`
                  }
                />
              </>
            ) : null}

            {emailSelectedProvider === "ses" ? (
              <>
                <Divider />
                <TextField
                  select
                  fullWidth
                  size="small"
                  label="Delivery Transport"
                  value={channelForm.email_transport_kind}
                  onChange={(e) => setChannelField("email_transport_kind", e.target.value)}
                  helperText="Use the HTTPS API for standard SES delivery, or SMTP when you want traditional relay credentials."
                >
                  {EMAIL_TRANSPORT_OPTIONS.map((option) => (
                    <MenuItem key={option.value} value={option.value}>
                      {option.label}
                    </MenuItem>
                  ))}
                </TextField>
                {emailSelectedTransportKind === "smtp" ? (
                  <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="SMTP Host"
                      value={channelForm.email_smtp_host}
                      onChange={(e) => setChannelField("email_smtp_host", e.target.value)}
                      placeholder="email-smtp.us-east-1.amazonaws.com"
                    />
                    <TextField
                      fullWidth
                      size="small"
                      label="SMTP Port"
                      value={channelForm.email_smtp_port}
                      onChange={(e) => setChannelField("email_smtp_port", e.target.value)}
                    />
                    <TextField
                      select
                      fullWidth
                      size="small"
                      label="Encryption"
                      value={channelForm.email_smtp_security}
                      onChange={(e) => setChannelField("email_smtp_security", e.target.value)}
                    >
                      {EMAIL_SMTP_SECURITY_OPTIONS.map((option) => (
                        <MenuItem key={option.value} value={option.value}>
                          {option.label}
                        </MenuItem>
                      ))}
                    </TextField>
                    <TextField
                      fullWidth
                      size="small"
                      label="SMTP Username"
                      value={channelForm.email_auth_basic_username}
                      onChange={(e) => setChannelField("email_auth_basic_username", e.target.value)}
                    />
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="SMTP Password"
                      value={channelForm.email_auth_basic_password}
                      onChange={(e) => setChannelField("email_auth_basic_password", e.target.value)}
                      placeholder={hasEmailBasicPassword ? "Configured (leave blank to keep)" : "Enter SMTP password"}
                      helperText={hasEmailBasicPassword ? "Leave blank to keep the saved SMTP password." : "Required for SES SMTP delivery."}
                    />
                  </Box>
                ) : (
                  <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
                    <TextField
                      fullWidth
                      size="small"
                      label="AWS Region"
                      value={channelForm.email_auth_aws_region}
                      onChange={(e) => setChannelField("email_auth_aws_region", e.target.value)}
                      placeholder="us-east-1"
                    />
                    <TextField
                      fullWidth
                      size="small"
                      label="Service"
                      value={channelForm.email_auth_aws_service}
                      onChange={(e) => setChannelField("email_auth_aws_service", e.target.value)}
                      helperText="Usually ses."
                    />
                    <TextField
                      fullWidth
                      size="small"
                      label="Access Key ID"
                      value={channelForm.email_auth_aws_access_key_id}
                      onChange={(e) => setChannelField("email_auth_aws_access_key_id", e.target.value)}
                    />
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="Secret Access Key"
                      value={channelForm.email_auth_aws_secret_access_key}
                      onChange={(e) => setChannelField("email_auth_aws_secret_access_key", e.target.value)}
                      placeholder={hasEmailAwsSecretAccessKey ? "Configured (leave blank to keep)" : "Enter secret access key"}
                      helperText={hasEmailAwsSecretAccessKey ? "Leave blank to keep the saved secret access key." : "Required for SES HTTPS delivery."}
                    />
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="Session Token"
                      value={channelForm.email_auth_aws_session_token}
                      onChange={(e) => setChannelField("email_auth_aws_session_token", e.target.value)}
                      placeholder={hasEmailAwsSessionToken ? "Configured (leave blank to keep)" : "Optional session token"}
                      helperText={hasEmailAwsSessionToken ? "Leave blank to keep the saved session token." : "Optional. Use it for temporary AWS credentials."}
                    />
                  </Box>
                )}
              </>
            ) : null}

            {emailSelectedProvider === "smtp" ? (
              <>
                <Divider />
                <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="SMTP Host"
                    value={channelForm.email_smtp_host}
                    onChange={(e) => setChannelField("email_smtp_host", e.target.value)}
                    placeholder="smtp.example.com"
                  />
                  <TextField
                    fullWidth
                    size="small"
                    label="SMTP Port"
                    value={channelForm.email_smtp_port}
                    onChange={(e) => setChannelField("email_smtp_port", e.target.value)}
                  />
                  <TextField
                    select
                    fullWidth
                    size="small"
                    label="Encryption"
                    value={channelForm.email_smtp_security}
                    onChange={(e) => setChannelField("email_smtp_security", e.target.value)}
                  >
                    {EMAIL_SMTP_SECURITY_OPTIONS.map((option) => (
                      <MenuItem key={option.value} value={option.value}>
                        {option.label}
                      </MenuItem>
                    ))}
                  </TextField>
                  <TextField
                    fullWidth
                    size="small"
                    label="SMTP Username"
                    value={channelForm.email_auth_basic_username}
                    onChange={(e) => setChannelField("email_auth_basic_username", e.target.value)}
                  />
                  <TextField
                    fullWidth
                    size="small"
                    type="password"
                    label="SMTP Password"
                    value={channelForm.email_auth_basic_password}
                    onChange={(e) => setChannelField("email_auth_basic_password", e.target.value)}
                    placeholder={hasEmailBasicPassword ? "Configured (leave blank to keep)" : "Enter SMTP password"}
                    helperText={hasEmailBasicPassword ? "Leave blank to keep the saved SMTP password." : "Required for SMTP delivery."}
                  />
                </Box>
              </>
            ) : null}

            {emailUsesExternalProvider ? (
              <Accordion
                disableGutters
                sx={{
                  border: "1px solid var(--ui-rgba-110-160-255-180)",
                  borderRadius: 1.5,
                  background: "var(--ui-rgba-8-24-42-320)",
                  "&:before": { display: "none" }
                }}
              >
                <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
                  <Box>
                    <Typography variant="subtitle2">Advanced Delivery Settings</Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Override auth or endpoint details only when your mail service expects something custom.
                    </Typography>
                  </Box>
                </AccordionSummary>
                <AccordionDetails sx={{ pt: 0 }}>
                  <Stack spacing={1.25}>
                    <TextField
                      select
                      fullWidth
                      size="small"
                      label="Auth Mode"
                      value={channelForm.email_auth_kind}
                      onChange={(e) => setChannelField("email_auth_kind", e.target.value)}
                      helperText="Provider default uses Bearer for Resend, header token for Postmark, AWS signing for SES HTTPS, and username/password for SMTP."
                    >
                      {EMAIL_AUTH_OPTIONS.map((option) => (
                        <MenuItem key={option.value} value={option.value}>
                          {option.label}
                        </MenuItem>
                      ))}
                    </TextField>
                    {!emailUsesSmtpTransport ? (
                      <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
                        <TextField
                          fullWidth
                          size="small"
                          label="API Base URL"
                          value={channelForm.email_http_base_url}
                          onChange={(e) => setChannelField("email_http_base_url", e.target.value)}
                          placeholder="Optional override"
                        />
                        <TextField
                          fullWidth
                          size="small"
                          label="Send Path"
                          value={channelForm.email_http_send_path}
                          onChange={(e) => setChannelField("email_http_send_path", e.target.value)}
                          placeholder="Optional override"
                        />
                      </Box>
                    ) : null}
                    {emailSelectedAuthKind === "bearer" || emailSelectedAuthKind === "header" ? (
                      <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
                        <TextField
                          fullWidth
                          size="small"
                          label="Header Name"
                          value={channelForm.email_auth_header_name}
                          onChange={(e) => setChannelField("email_auth_header_name", e.target.value)}
                          placeholder="Authorization"
                        />
                        <TextField
                          fullWidth
                          size="small"
                          label="Auth Scheme"
                          value={channelForm.email_auth_scheme}
                          onChange={(e) => setChannelField("email_auth_scheme", e.target.value)}
                          placeholder="Bearer"
                        />
                      </Box>
                    ) : null}
                  </Stack>
                </AccordionDetails>
              </Accordion>
            ) : null}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setEmailSetupOpen(false)
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={telegramSetupOpen} onClose={() => setTelegramSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Telegram Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            {notice?.kind === "error" ? <Alert severity="error">{notice.text}</Alert> : null}
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Add your Telegram bot token and optional allowed user IDs. This controls who can use your bot.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(telegramConnectionStatusRaw, telegramEnabledSaved)}
                color={channelStatusColor(telegramConnectionStatusRaw, telegramEnabledSaved)}
                variant="outlined"
              />
              <Button size="small" onClick={() => telegramStatusQ.refetch()} disabled={telegramStatusQ.isFetching}>
                {telegramStatusQ.isFetching ? "Checking..." : "Refresh Status"}
              </Button>
            </Stack>
            {telegramConnectionDetail ? (
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
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
        {renderMessagingDialogActions({
          onClose: () => setTelegramSetupOpen(false),
          disconnectVisible: channelForm.telegram_enabled,
          onDisconnect: () =>
            disconnectChannel("Telegram", "telegram_enabled", () => setTelegramSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={slackSetupOpen} onClose={() => setSlackSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Slack Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Save the bot token and signing secret, then point Slack Events API at `/webhook/slack`. Reply routing and channel runtime health will show up in Channels after the first live event.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(slackConnectionStatusRaw, channelForm.slack_enabled)}
                color={channelStatusColor(slackConnectionStatusRaw, channelForm.slack_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {slackConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.slack_enabled} onChange={(e) => setChannelField("slack_enabled", e.target.checked)} />}
              label={channelForm.slack_enabled ? "Slack enabled" : "Slack disabled"}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bot Token"
              value={channelForm.slack_bot_token}
              onChange={(e) => setChannelField("slack_bot_token", e.target.value)}
              placeholder={hasSlackBotToken ? "Configured (leave blank to keep)" : "xoxb-..."}
              helperText={hasSlackBotToken ? "Leave blank to keep the saved bot token." : "Required for outbound delivery."}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Signing Secret"
              value={channelForm.slack_signing_secret}
              onChange={(e) => setChannelField("slack_signing_secret", e.target.value)}
              placeholder={hasSlackSigningSecret ? "Configured (leave blank to keep)" : "Enter signing secret"}
              helperText={hasSlackSigningSecret ? "Leave blank to keep the saved signing secret." : "Required for signed webhook verification."}
            />
            <TextField fullWidth size="small" label="Default Channel ID" value={channelForm.slack_default_channel_id} onChange={(e) => setChannelField("slack_default_channel_id", e.target.value)} />
            <TextField fullWidth size="small" label="Default Thread TS" value={channelForm.slack_default_thread_ts} onChange={(e) => setChannelField("slack_default_thread_ts", e.target.value)} />
            <TextField fullWidth size="small" label="Workspace ID" value={channelForm.slack_workspace_id} onChange={(e) => setChannelField("slack_workspace_id", e.target.value)} />
            <TextField fullWidth size="small" label="Workspace Name" value={channelForm.slack_workspace_name} onChange={(e) => setChannelField("slack_workspace_name", e.target.value)} />
            <TextField fullWidth size="small" label="API Base URL" value={channelForm.slack_api_base_url} onChange={(e) => setChannelField("slack_api_base_url", e.target.value)} helperText="Leave the default unless you are targeting a proxy or test environment." />
            <Divider />
            {renderSenderTrustSection({
              channel: "slack",
              configured: channelForm.slack_enabled,
              policyValue: channelForm.slack_trust_policy,
              onPolicyChange: (value) => setChannelField("slack_trust_policy", value),
              allowedValue: channelForm.slack_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("slack_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted Sender IDs",
              allowedHelper: "Comma-separated Slack user IDs that should bypass pairing, for example U123ABC."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setSlackSetupOpen(false),
          disconnectVisible: channelForm.slack_enabled,
          onDisconnect: () => disconnectChannel("Slack", "slack_enabled", () => setSlackSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={discordSetupOpen} onClose={() => setDiscordSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Discord Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Discord now requires a live bot token plus at least one guild, channel, or thread scope for inbound handling. The webhook URL is optional and only helps with outbound thread posting.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(discordConnectionStatusRaw, channelForm.discord_enabled)}
                color={channelStatusColor(discordConnectionStatusRaw, channelForm.discord_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {discordConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.discord_enabled} onChange={(e) => setChannelField("discord_enabled", e.target.checked)} />}
              label={channelForm.discord_enabled ? "Discord enabled" : "Discord disabled"}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bot Token"
              value={channelForm.discord_bot_token}
              onChange={(e) => setChannelField("discord_bot_token", e.target.value)}
              placeholder={hasDiscordBotToken ? "Configured (leave blank to keep)" : "Enter bot token"}
              helperText={hasDiscordBotToken ? "Leave blank to keep the saved bot token." : "Required for gateway runtime and REST delivery."}
            />
            <TextField fullWidth size="small" label="Webhook URL" value={channelForm.discord_webhook_url} onChange={(e) => setChannelField("discord_webhook_url", e.target.value)} helperText="Optional. Useful for scoped thread delivery without the full bot runtime." />
            <TextField fullWidth size="small" label="Default Channel ID" value={channelForm.discord_default_channel_id} onChange={(e) => setChannelField("discord_default_channel_id", e.target.value)} />
            <TextField fullWidth size="small" label="Default Thread ID" value={channelForm.discord_default_thread_id} onChange={(e) => setChannelField("discord_default_thread_id", e.target.value)} />
            <TextField fullWidth size="small" label="Guild ID" value={channelForm.discord_guild_id} onChange={(e) => setChannelField("discord_guild_id", e.target.value)} />
            <TextField fullWidth size="small" label="Application ID" value={channelForm.discord_application_id} onChange={(e) => setChannelField("discord_application_id", e.target.value)} />
            <TextField fullWidth size="small" label="API Base URL" value={channelForm.discord_api_base_url} onChange={(e) => setChannelField("discord_api_base_url", e.target.value)} />
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setDiscordSetupOpen(false),
          disconnectVisible: channelForm.discord_enabled,
          onDisconnect: () =>
            disconnectChannel("Discord", "discord_enabled", () => setDiscordSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={matrixSetupOpen} onClose={() => setMatrixSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Matrix Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Configure a Matrix homeserver identity that the sync loop can poll continuously. After the first room event, AgentArk will remember the active reply destination automatically.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(matrixConnectionStatusRaw, channelForm.matrix_enabled)}
                color={channelStatusColor(matrixConnectionStatusRaw, channelForm.matrix_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {matrixConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.matrix_enabled} onChange={(e) => setChannelField("matrix_enabled", e.target.checked)} />}
              label={channelForm.matrix_enabled ? "Matrix enabled" : "Matrix disabled"}
            />
            <TextField fullWidth size="small" label="Homeserver URL" value={channelForm.matrix_homeserver_url} onChange={(e) => setChannelField("matrix_homeserver_url", e.target.value)} />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Access Token"
              value={channelForm.matrix_access_token}
              onChange={(e) => setChannelField("matrix_access_token", e.target.value)}
              placeholder={hasMatrixAccessToken ? "Configured (leave blank to keep)" : "Enter access token"}
              helperText={hasMatrixAccessToken ? "Leave blank to keep the saved access token." : "Required for sync and outbound delivery."}
            />
            <TextField fullWidth size="small" label="User ID" value={channelForm.matrix_user_id} onChange={(e) => setChannelField("matrix_user_id", e.target.value)} />
            <TextField fullWidth size="small" label="Default Room ID" value={channelForm.matrix_default_room_id} onChange={(e) => setChannelField("matrix_default_room_id", e.target.value)} />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="Device ID" value={channelForm.matrix_device_id} onChange={(e) => setChannelField("matrix_device_id", e.target.value)} />
              <TextField fullWidth size="small" label="Account ID" value={channelForm.matrix_account_id} onChange={(e) => setChannelField("matrix_account_id", e.target.value)} />
              <TextField fullWidth size="small" label="Sync Timeout (ms)" value={channelForm.matrix_sync_timeout_ms} onChange={(e) => setChannelField("matrix_sync_timeout_ms", e.target.value)} />
              <TextField fullWidth size="small" label="Batch Limit" value={channelForm.matrix_limit} onChange={(e) => setChannelField("matrix_limit", e.target.value)} />
            </Box>
            <TextField fullWidth size="small" label="User Agent" value={channelForm.matrix_user_agent} onChange={(e) => setChannelField("matrix_user_agent", e.target.value)} />
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setMatrixSetupOpen(false),
          disconnectVisible: channelForm.matrix_enabled,
          onDisconnect: () => disconnectChannel("Matrix", "matrix_enabled", () => setMatrixSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={teamsSetupOpen} onClose={() => setTeamsSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Teams Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Configure the Bot Framework endpoint and credentials, including the bot app ID used for JWT verification. The runtime will only accept signed inbound activities and only reply to trusted service URLs.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(teamsConnectionStatusRaw, channelForm.teams_enabled)}
                color={channelStatusColor(teamsConnectionStatusRaw, channelForm.teams_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {teamsConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.teams_enabled} onChange={(e) => setChannelField("teams_enabled", e.target.checked)} />}
              label={channelForm.teams_enabled ? "Teams enabled" : "Teams disabled"}
            />
            <TextField fullWidth size="small" label="Service URL" value={channelForm.teams_service_url} onChange={(e) => setChannelField("teams_service_url", e.target.value)} helperText="Must match the Bot Framework service URL used by inbound activities." />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Access Token"
              value={channelForm.teams_access_token}
              onChange={(e) => setChannelField("teams_access_token", e.target.value)}
              placeholder={hasTeamsAccessToken ? "Configured (leave blank to keep)" : "Enter access token"}
              helperText={hasTeamsAccessToken ? "Leave blank to keep the saved access token." : "Required for outbound Bot Framework or Graph delivery."}
            />
            <TextField fullWidth size="small" label="Bot App ID" value={channelForm.teams_bot_app_id} onChange={(e) => setChannelField("teams_bot_app_id", e.target.value)} helperText="Required for Bot Framework JWT validation." />
            <TextField fullWidth size="small" label="Bot Name" value={channelForm.teams_bot_name} onChange={(e) => setChannelField("teams_bot_name", e.target.value)} />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="Tenant ID" value={channelForm.teams_tenant_id} onChange={(e) => setChannelField("teams_tenant_id", e.target.value)} />
              <TextField fullWidth size="small" label="Chat ID" value={channelForm.teams_chat_id} onChange={(e) => setChannelField("teams_chat_id", e.target.value)} />
              <TextField fullWidth size="small" label="Team ID" value={channelForm.teams_team_id} onChange={(e) => setChannelField("teams_team_id", e.target.value)} />
              <TextField fullWidth size="small" label="Channel ID" value={channelForm.teams_channel_id} onChange={(e) => setChannelField("teams_channel_id", e.target.value)} />
            </Box>
            <TextField select fullWidth size="small" label="Delivery Mode" value={channelForm.teams_delivery_mode} onChange={(e) => setChannelField("teams_delivery_mode", e.target.value as ChannelSettingsForm["teams_delivery_mode"])}>
              <MenuItem value="auto">Auto</MenuItem>
              <MenuItem value="bot_framework">Bot Framework</MenuItem>
              <MenuItem value="graph">Graph</MenuItem>
            </TextField>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="Graph Base URL" value={channelForm.teams_graph_base_url} onChange={(e) => setChannelField("teams_graph_base_url", e.target.value)} />
              <TextField fullWidth size="small" label="Timeout (s)" value={channelForm.teams_timeout_secs} onChange={(e) => setChannelField("teams_timeout_secs", e.target.value)} />
            </Box>
            <TextField fullWidth size="small" label="User Agent" value={channelForm.teams_user_agent} onChange={(e) => setChannelField("teams_user_agent", e.target.value)} />
            <Divider />
            {renderSenderTrustSection({
              channel: "teams",
              configured: channelForm.teams_enabled,
              policyValue: channelForm.teams_trust_policy,
              onPolicyChange: (value) => setChannelField("teams_trust_policy", value),
              allowedValue: channelForm.teams_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("teams_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted Sender IDs",
              allowedHelper: "Comma-separated Teams user IDs or AAD object IDs that should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setTeamsSetupOpen(false),
          disconnectVisible: channelForm.teams_enabled,
          onDisconnect: () => disconnectChannel("Teams", "teams_enabled", () => setTeamsSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={googleChatSetupOpen} onClose={() => setGoogleChatSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Google Chat Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Use this when you want AgentArk to send and receive messages inside Google Chat. This is separate from Google Workspace data access. Each space or DM becomes its own AgentArk conversation thread, while still sharing your global docs, memories, apps, tasks, and watchers.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(googleChatConnectionStatusRaw, channelForm.google_chat_enabled)}
                color={channelStatusColor(googleChatConnectionStatusRaw, channelForm.google_chat_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {googleChatConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.google_chat_enabled} onChange={(e) => setChannelField("google_chat_enabled", e.target.checked)} />}
              label={channelForm.google_chat_enabled ? "Google Chat enabled" : "Google Chat disabled"}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Access Token"
              value={channelForm.google_chat_access_token}
              onChange={(e) => setChannelField("google_chat_access_token", e.target.value)}
              placeholder={hasGoogleChatAccessToken ? "Configured (leave blank to keep)" : "Enter access token"}
              helperText={hasGoogleChatAccessToken ? "Leave blank to keep the saved access token." : "Required for outbound messages and replies."}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Verification Token"
              value={channelForm.google_chat_verify_token}
              onChange={(e) => setChannelField("google_chat_verify_token", e.target.value)}
              placeholder={hasGoogleChatVerifyToken ? "Configured (leave blank to keep)" : "Optional verification token"}
              helperText="Optional. Use it if your Google Chat ingress includes a shared verification token."
            />
            <TextField fullWidth size="small" label="Default Space" value={channelForm.google_chat_space} onChange={(e) => setChannelField("google_chat_space", e.target.value)} helperText="Optional. Use a space like spaces/AAAA... if you want AgentArk to send proactive updates there." />
            <TextField fullWidth size="small" label="Thread Key" value={channelForm.google_chat_thread_key} onChange={(e) => setChannelField("google_chat_thread_key", e.target.value)} helperText="Optional. Example: weekly-digest. Use it when proactive updates should stay inside one Google Chat thread." />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="API Base URL" value={channelForm.google_chat_api_base_url} onChange={(e) => setChannelField("google_chat_api_base_url", e.target.value)} />
              <TextField fullWidth size="small" label="App ID" value={channelForm.google_chat_app_id} onChange={(e) => setChannelField("google_chat_app_id", e.target.value)} helperText="Optional. Helpful for matching logs and Google Chat app configuration." />
            </Box>
            <TextField fullWidth size="small" label="Bot Name" value={channelForm.google_chat_bot_name} onChange={(e) => setChannelField("google_chat_bot_name", e.target.value)} helperText="Optional display label for status screens." />
            <Divider />
            {renderSenderTrustSection({
              channel: "google_chat",
              configured: channelForm.google_chat_enabled,
              policyValue: channelForm.google_chat_trust_policy,
              onPolicyChange: (value) => setChannelField("google_chat_trust_policy", value),
              allowedValue: channelForm.google_chat_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("google_chat_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted Sender IDs",
              allowedHelper: "Use Google Chat sender IDs or app-mapped user IDs that should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setGoogleChatSetupOpen(false),
          disconnectVisible: channelForm.google_chat_enabled,
          onDisconnect: () =>
            disconnectChannel("Google Chat", "google_chat_enabled", () => setGoogleChatSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={signalSetupOpen} onClose={() => setSignalSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Signal Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Signal uses a companion bridge. Each Signal DM or group becomes its own AgentArk conversation thread, while AgentArk still keeps access to your shared docs, memories, apps, tasks, and watchers.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(signalConnectionStatusRaw, channelForm.signal_enabled)}
                color={channelStatusColor(signalConnectionStatusRaw, channelForm.signal_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {signalConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.signal_enabled} onChange={(e) => setChannelField("signal_enabled", e.target.checked)} />}
              label={channelForm.signal_enabled ? "Signal enabled" : "Signal disabled"}
            />
            <TextField fullWidth size="small" label="Bridge URL" value={channelForm.signal_bridge_url} onChange={(e) => setChannelField("signal_bridge_url", e.target.value)} helperText="Example: http://127.0.0.1:9120" />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bridge Token"
              value={channelForm.signal_bridge_token}
              onChange={(e) => setChannelField("signal_bridge_token", e.target.value)}
              placeholder={hasSignalBridgeToken ? "Configured (leave blank to keep)" : "Enter bridge token"}
              helperText={hasSignalBridgeToken ? "Leave blank to keep the saved bridge token." : "Required so only your bridge can talk to AgentArk."}
            />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="Default Recipient" value={channelForm.signal_default_recipient} onChange={(e) => setChannelField("signal_default_recipient", e.target.value)} helperText="Optional. Example: +14155551212. Add this if you want AgentArk to send proactive Signal messages without waiting for an inbound chat." />
              <TextField fullWidth size="small" label="Default Group ID" value={channelForm.signal_default_group_id} onChange={(e) => setChannelField("signal_default_group_id", e.target.value)} helperText="Optional. Use this when proactive updates should land in one Signal group by default." />
            </Box>
            <Divider />
            {renderSenderTrustSection({
              channel: "signal",
              configured: channelForm.signal_enabled,
              policyValue: channelForm.signal_trust_policy,
              onPolicyChange: (value) => setChannelField("signal_trust_policy", value),
              allowedValue: channelForm.signal_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("signal_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted Signal Senders",
              allowedHelper: "Use the sender IDs that your bridge reports for contacts who should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setSignalSetupOpen(false),
          disconnectVisible: channelForm.signal_enabled,
          onDisconnect: () =>
            disconnectChannel("Signal", "signal_enabled", () => setSignalSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={imessageSetupOpen} onClose={() => setImessageSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>iMessage Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              iMessage requires a companion bridge backed by an Apple device. Each chat becomes its own AgentArk conversation thread, while AgentArk still shares your global docs, memories, apps, tasks, and watchers across all channels.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(imessageConnectionStatusRaw, channelForm.imessage_enabled)}
                color={channelStatusColor(imessageConnectionStatusRaw, channelForm.imessage_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {imessageConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.imessage_enabled} onChange={(e) => setChannelField("imessage_enabled", e.target.checked)} />}
              label={channelForm.imessage_enabled ? "iMessage enabled" : "iMessage disabled"}
            />
            <TextField fullWidth size="small" label="Bridge URL" value={channelForm.imessage_bridge_url} onChange={(e) => setChannelField("imessage_bridge_url", e.target.value)} helperText="Example: http://127.0.0.1:9130" />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bridge Token"
              value={channelForm.imessage_bridge_token}
              onChange={(e) => setChannelField("imessage_bridge_token", e.target.value)}
              placeholder={hasIMessageBridgeToken ? "Configured (leave blank to keep)" : "Enter bridge token"}
              helperText={hasIMessageBridgeToken ? "Leave blank to keep the saved bridge token." : "Required so only your iMessage bridge can talk to AgentArk."}
            />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="Default Chat ID" value={channelForm.imessage_default_chat_id} onChange={(e) => setChannelField("imessage_default_chat_id", e.target.value)} helperText="Optional. Use this if your bridge exposes a stable chat ID for proactive delivery." />
              <TextField fullWidth size="small" label="Default Handle" value={channelForm.imessage_default_handle} onChange={(e) => setChannelField("imessage_default_handle", e.target.value)} helperText="Optional. Example: +14155551212 or name@icloud.com. Use it when proactive messages should open a specific chat." />
            </Box>
            <Divider />
            {renderSenderTrustSection({
              channel: "imessage",
              configured: channelForm.imessage_enabled,
              policyValue: channelForm.imessage_trust_policy,
              onPolicyChange: (value) => setChannelField("imessage_trust_policy", value),
              allowedValue: channelForm.imessage_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("imessage_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted iMessage Senders",
              allowedHelper: "Use the sender IDs or handles your bridge reports for contacts who should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setImessageSetupOpen(false),
          disconnectVisible: channelForm.imessage_enabled,
          onDisconnect: () =>
            disconnectChannel("iMessage", "imessage_enabled", () => setImessageSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={lineSetupOpen} onClose={() => setLineSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>LINE Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              LINE is a first-class messaging channel for AgentArk. Each DM, room, or group becomes its own AgentArk conversation thread, while still sharing your global docs, memories, apps, tasks, and watchers.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(lineConnectionStatusRaw, channelForm.line_enabled)}
                color={channelStatusColor(lineConnectionStatusRaw, channelForm.line_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {lineConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.line_enabled} onChange={(e) => setChannelField("line_enabled", e.target.checked)} />}
              label={channelForm.line_enabled ? "LINE enabled" : "LINE disabled"}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Channel Access Token"
              value={channelForm.line_channel_access_token}
              onChange={(e) => setChannelField("line_channel_access_token", e.target.value)}
              placeholder={hasLineAccessToken ? "Configured (leave blank to keep)" : "Enter channel access token"}
              helperText={hasLineAccessToken ? "Leave blank to keep the saved access token." : "Required for replies and push messages."}
            />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Channel Secret"
              value={channelForm.line_channel_secret}
              onChange={(e) => setChannelField("line_channel_secret", e.target.value)}
              placeholder={hasLineChannelSecret ? "Configured (leave blank to keep)" : "Enter channel secret"}
              helperText={hasLineChannelSecret ? "Leave blank to keep the saved secret." : "Required to verify LINE webhook signatures."}
            />
            <TextField fullWidth size="small" label="Default Target" value={channelForm.line_default_target} onChange={(e) => setChannelField("line_default_target", e.target.value)} helperText="Optional. Use a user, room, or group ID if you want proactive LINE updates outside an active chat thread." />
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 1 }}>
              <TextField fullWidth size="small" label="API Base URL" value={channelForm.line_api_base_url} onChange={(e) => setChannelField("line_api_base_url", e.target.value)} />
              <TextField fullWidth size="small" label="User Agent" value={channelForm.line_user_agent} onChange={(e) => setChannelField("line_user_agent", e.target.value)} helperText="Optional. Leave blank unless your proxy expects one." />
            </Box>
            <Divider />
            {renderSenderTrustSection({
              channel: "line",
              configured: channelForm.line_enabled,
              policyValue: channelForm.line_trust_policy,
              onPolicyChange: (value) => setChannelField("line_trust_policy", value),
              allowedValue: channelForm.line_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("line_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted LINE Senders",
              allowedHelper: "Use the LINE user IDs that should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setLineSetupOpen(false),
          disconnectVisible: channelForm.line_enabled,
          onDisconnect: () => disconnectChannel("LINE", "line_enabled", () => setLineSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={wechatSetupOpen} onClose={() => setWechatSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>WeChat Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              WeChat uses a bridge so each WeChat chat can stay in its own AgentArk conversation thread, while AgentArk still shares your global docs, memories, apps, tasks, and watchers across channels.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(wechatConnectionStatusRaw, channelForm.wechat_enabled)}
                color={channelStatusColor(wechatConnectionStatusRaw, channelForm.wechat_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {wechatConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.wechat_enabled} onChange={(e) => setChannelField("wechat_enabled", e.target.checked)} />}
              label={channelForm.wechat_enabled ? "WeChat enabled" : "WeChat disabled"}
            />
            <TextField fullWidth size="small" label="Bridge URL" value={channelForm.wechat_bridge_url} onChange={(e) => setChannelField("wechat_bridge_url", e.target.value)} />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bridge Token"
              value={channelForm.wechat_bridge_token}
              onChange={(e) => setChannelField("wechat_bridge_token", e.target.value)}
              placeholder={hasWeChatBridgeToken ? "Configured (leave blank to keep)" : "Enter bridge token"}
              helperText={hasWeChatBridgeToken ? "Leave blank to keep the saved bridge token." : "Required so only your WeChat bridge can talk to AgentArk."}
            />
            <TextField fullWidth size="small" label="Default Target ID" value={channelForm.wechat_default_target_id} onChange={(e) => setChannelField("wechat_default_target_id", e.target.value)} helperText="Optional. Add the target ID your bridge expects if you want proactive WeChat delivery before anyone messages AgentArk first." />
            <Divider />
            {renderSenderTrustSection({
              channel: "wechat",
              configured: channelForm.wechat_enabled,
              policyValue: channelForm.wechat_trust_policy,
              onPolicyChange: (value) => setChannelField("wechat_trust_policy", value),
              allowedValue: channelForm.wechat_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("wechat_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted WeChat Senders",
              allowedHelper: "Use the sender IDs your bridge reports for contacts who should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setWechatSetupOpen(false),
          disconnectVisible: channelForm.wechat_enabled,
          onDisconnect: () =>
            disconnectChannel("WeChat", "wechat_enabled", () => setWechatSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={qqSetupOpen} onClose={() => setQqSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>QQ Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              QQ uses a bridge so each QQ chat can stay in its own AgentArk conversation thread, while AgentArk still shares your global docs, memories, apps, tasks, and watchers across channels.
            </Typography>
            <Stack direction="row" spacing={1} sx={{
              alignItems: "center"
            }}>
              <Chip
                size="small"
                label={channelStatusLabel(qqConnectionStatusRaw, channelForm.qq_enabled)}
                color={channelStatusColor(qqConnectionStatusRaw, channelForm.qq_enabled)}
                variant="outlined"
              />
              <Button size="small" onClick={() => channelsQ.refetch()} disabled={channelsQ.isFetching}>
                {channelsQ.isFetching ? "Refreshing..." : "Refresh Status"}
              </Button>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {qqConnectionDetail}
            </Typography>
            <FormControlLabel
              control={<Switch checked={channelForm.qq_enabled} onChange={(e) => setChannelField("qq_enabled", e.target.checked)} />}
              label={channelForm.qq_enabled ? "QQ enabled" : "QQ disabled"}
            />
            <TextField fullWidth size="small" label="Bridge URL" value={channelForm.qq_bridge_url} onChange={(e) => setChannelField("qq_bridge_url", e.target.value)} />
            <TextField
              fullWidth
              size="small"
              type="password"
              label="Bridge Token"
              value={channelForm.qq_bridge_token}
              onChange={(e) => setChannelField("qq_bridge_token", e.target.value)}
              placeholder={hasQqBridgeToken ? "Configured (leave blank to keep)" : "Enter bridge token"}
              helperText={hasQqBridgeToken ? "Leave blank to keep the saved bridge token." : "Required so only your QQ bridge can talk to AgentArk."}
            />
            <TextField fullWidth size="small" label="Default Target ID" value={channelForm.qq_default_target_id} onChange={(e) => setChannelField("qq_default_target_id", e.target.value)} helperText="Optional. Add the target ID your bridge expects if you want proactive QQ delivery before anyone messages AgentArk first." />
            <Divider />
            {renderSenderTrustSection({
              channel: "qq",
              configured: channelForm.qq_enabled,
              policyValue: channelForm.qq_trust_policy,
              onPolicyChange: (value) => setChannelField("qq_trust_policy", value),
              allowedValue: channelForm.qq_allowed_senders_csv,
              onAllowedChange: (value) => setChannelField("qq_allowed_senders_csv", value),
              allowedLabel: "Always-Trusted QQ Senders",
              allowedHelper: "Use the sender IDs your bridge reports for contacts who should bypass pairing."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setQqSetupOpen(false),
          disconnectVisible: channelForm.qq_enabled,
          onDisconnect: () => disconnectChannel("QQ", "qq_enabled", () => setQqSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
      <Dialog open={whatsAppSetupOpen} onClose={() => setWhatsAppSetupOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>WhatsApp Setup</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
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
              <>
                <TextField
                  fullWidth
                  size="small"
                  select
                  label="Bridge Runtime"
                  value={channelForm.whatsapp_bridge_runtime}
                  onChange={(e) =>
                    setChannelField(
                      "whatsapp_bridge_runtime",
                      e.target.value === "external" ? "external" : "embedded"
                    )
                  }
                  disabled={!channelForm.whatsapp_enabled}
                >
                  <MenuItem value="embedded">Bundled bridge</MenuItem>
                  <MenuItem value="external">External bridge</MenuItem>
                </TextField>

                {whatsappEmbeddedBridgeSelected ? (
                  <Box className="metadata-box" sx={{ maxHeight: "none", p: 1.5 }}>
                    <Stack spacing={1}>
                      <Stack direction="row" spacing={1} sx={{
                        alignItems: "center"
                      }}>
                        <Typography variant="body2" sx={{
                          fontWeight: 700
                        }}>
                          Bundled bridge status
                        </Typography>
                        <Chip
                          size="small"
                          label={channelStatusLabel(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled)}
                          color={channelStatusColor(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled)}
                          variant="outlined"
                        />
                      </Stack>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        {whatsappConnectionDetail}
                      </Typography>
                      {!whatsappBridgeInstalled ? (
                        <Alert severity="warning" sx={{ py: 0.75 }}>
                          Full image required. This install does not include the bundled WhatsApp bridge.
                        </Alert>
                      ) : null}
                      {whatsappBridgeWarning ? (
                        <Alert severity="warning" sx={{ py: 0.75 }}>
                          {whatsappBridgeWarning}
                        </Alert>
                      ) : null}
                      {whatsappBridgeStatus === "qr" ? (
                        str(waBridge.qr, "").trim() ? (
                          <Box
                            component="img"
                            src={str(waBridge.qr, "")}
                            alt="WhatsApp QR code"
                            sx={{
                              width: 220,
                              height: 220,
                              borderRadius: 1,
                              border: "1px solid var(--surface-border)",
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
                ) : (
                  <Stack spacing={1.25}>
                    <Alert
                      severity={
                        whatsappConnectionStatusRaw === "ready" || whatsappConnectionStatusRaw === "connected"
                          ? "success"
                          : ["error", "unavailable"].includes(whatsappConnectionStatusRaw)
                            ? "error"
                            : "info"
                      }
                      sx={{ py: 0.75 }}
                    >
                      {whatsappConnectionDetail}
                    </Alert>
                    {whatsappBridgeWarning ? (
                      <Alert severity="warning" sx={{ py: 0.75 }}>
                        {whatsappBridgeWarning}
                      </Alert>
                    ) : null}
                    <TextField
                      fullWidth
                      size="small"
                      label="External Bridge URL"
                      value={channelForm.whatsapp_bridge_url}
                      onChange={(e) => setChannelField("whatsapp_bridge_url", e.target.value)}
                      placeholder="https://bridge.example.com"
                      helperText="AgentArk will send WhatsApp requests to this bridge instead of starting its bundled bridge."
                    />
                    <TextField
                      fullWidth
                      size="small"
                      type="password"
                      label="External Bridge Token"
                      value={channelForm.whatsapp_bridge_token}
                      onChange={(e) => setChannelField("whatsapp_bridge_token", e.target.value)}
                      placeholder={hasWhatsAppBridgeToken ? "Configured (leave blank to keep)" : "Enter bridge token"}
                      helperText={
                        hasWhatsAppBridgeToken
                          ? "Leave blank to keep the saved token. New external bridge setups require a token."
                          : "Required for new external bridge setups."
                      }
                    />
                  </Stack>
                )}
              </>
            ) : null}

            {channelForm.whatsapp_enabled && channelForm.whatsapp_mode === "cloud_api" ? (
              <>
                <Stack direction="row" spacing={1} sx={{
                  alignItems: "center"
                }}>
                  <Chip
                    size="small"
                    label={channelStatusLabel(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled)}
                    color={channelStatusColor(whatsappConnectionStatusRaw, channelForm.whatsapp_enabled)}
                    variant="outlined"
                  />
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
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
                  type="password"
                  label="App Secret"
                  value={channelForm.whatsapp_app_secret}
                  onChange={(e) => setChannelField("whatsapp_app_secret", e.target.value)}
                  placeholder={hasWhatsAppAppSecret ? "Configured (leave blank to keep)" : "Enter app secret"}
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
                  placeholder={hasWhatsAppVerifyToken ? "Configured (leave blank to keep)" : "Enter verify token"}
                />
              </>
            ) : null}

            <Divider />
            {renderSenderTrustSection({
              channel: "whatsapp",
              configured: channelForm.whatsapp_enabled,
              policyValue: channelForm.whatsapp_dm_policy,
              onPolicyChange: (value) => setChannelField("whatsapp_dm_policy", value),
              allowedValue: channelForm.whatsapp_allowed_numbers_csv,
              onAllowedChange: (value) => setChannelField("whatsapp_allowed_numbers_csv", value),
              allowedLabel: "Always-Trusted Numbers",
              allowedHelper: "Comma-separated phone numbers that should always bypass pairing. Dynamic approvals appear below."
            })}
          </Stack>
        </DialogContent>
        {renderMessagingDialogActions({
          onClose: () => setWhatsAppSetupOpen(false),
          disconnectVisible: channelForm.whatsapp_enabled,
          disconnectLabel: "Disconnect Channel",
          onDisconnect: () =>
            disconnectChannel("WhatsApp", "whatsapp_enabled", () => setWhatsAppSetupOpen(false))
        })}
      </Dialog>
      ) : null}
      {showIntegrations ? (
        <Dialog open={!!active} onClose={closeConfig} maxWidth="md" fullWidth>
          <DialogTitle sx={{ textTransform: "none" }}>{active?.name || "Configure integration"}</DialogTitle>
          <DialogContent sx={{ px: { xs: 2, md: 3 }, py: { xs: 1.5, md: 2.5 } }}>
            <Stack spacing={1.5} sx={{ pt: 0.5 }}>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              {active?.description}
            </Typography>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {integrationDialogStatusLabel(active)}
            </Typography>
            {activeHasSavedConfig && !editingConnected ? (
              <Stack spacing={1.5}>
                <Alert severity={activeIsVerified && active?.enabled ? "success" : "info"}>
                  {activeIsVerified
                    ? active?.enabled
                      ? "This integration is connected and active."
                      : "This integration is connected, but currently disabled for agent use."
                    : "Credentials are saved, but this integration remains disabled until a live connection is confirmed."}
                </Alert>
                <Stack direction="row" spacing={1} sx={{
                  alignItems: "center"
                }}>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={!!active?.enabled}
                        onChange={async () => {
                          try {
                            await api.rawPost(`/integrations/${active!.id}/${active!.enabled ? "disable" : "enable"}`, {});
                            await Promise.allSettled([
                              queryClient.invalidateQueries({ queryKey: ["integrations"] }),
                              queryClient.invalidateQueries({ queryKey: ["integration-sync-status"] })
                            ]);
                            setActive((prev) => prev ? { ...prev, enabled: !prev.enabled } : prev);
                          } catch (e) {
                            setFormError(asErrorMessage(e));
                          }
                        }}
                      />
                    }
                    label={active?.enabled ? "Enabled" : "Disabled"}
                  />
                  <Button variant="outlined" size="small" onClick={() => setEditingConnected(true)}>
                    Edit Configuration
                  </Button>
                </Stack>
              </Stack>
            ) : null}
            {activeHasSavedConfig && !active?.enabled && editingConnected ? (
              <Alert severity="info">
                {activeIsVerified
                  ? "This integration is connected, but currently disabled for agent use."
                  : "Credentials are saved, but this integration remains disabled until a live connection is confirmed."}
              </Alert>
            ) : null}
            {active?.status === "error" && active?.status_detail ? (
              <Alert severity="error">{active.status_detail}</Alert>
            ) : null}
            {active?.status === "starting" && active?.status_detail ? (
              <Alert severity="info">{active.status_detail}</Alert>
            ) : null}
            {active?.status === "not_configured" ? (
              <Alert severity="info">
                This integration is disabled until you connect it.
              </Alert>
            ) : null}
            {(!activeHasSavedConfig || editingConnected) && activeNeedsOauth ? (
              <Alert severity="info">
                {active?.id === "google_workspace"
                  ? "Enter the Google OAuth client ID and client secret, choose the Workspace bundles, then continue with Google."
                  : "Save your Google OAuth client credentials here, then click Connect to finish sign-in."}
              </Alert>
            ) : null}
            {(!activeHasSavedConfig || editingConnected) && active?.id === "google_workspace" ? (
              <Box
                sx={{
                  border: "1px solid var(--ui-rgba-112-153-201-120)",
                  borderRadius: 2,
                  p: 1.25,
                  background: "var(--ui-rgba-8-18-32-460)"
                }}
              >
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{
                    alignItems: "center",
                    justifyContent: "space-between"
                  }}>
                  <Box>
                    <Typography variant="subtitle2">Google OAuth setup</Typography>
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      Add the Google OAuth client here, then AgentArk will open the Google consent screen in your browser.
                    </Typography>
                  </Box>
                  <IconButton
                    size="small"
                    onClick={() => setGoogleWorkspaceHelpOpen((open) => !open)}
                    aria-label="How to get Google OAuth client credentials"
                  >
                    <InfoOutlinedIcon fontSize="small" />
                  </IconButton>
                </Stack>
                {googleWorkspaceHelpOpen ? (
                  <Alert severity="info" sx={{ mt: 1.25 }}>
                    Create or open a Google Cloud project, configure the OAuth consent screen, add yourself as a test user if the app is still in testing, then create an OAuth client and copy its client ID and client secret. Add the redirect URI <strong>http://localhost:8990/oauth/callback</strong>, and enable the Google APIs you want AgentArk to use such as Gmail API, Google Calendar API, Drive API, Docs API, Sheets API, Google Chat API, and Admin SDK. Google's official setup guide is at <strong>developers.google.com/accounts/docs/OAuth2Login</strong>.
                  </Alert>
                ) : null}
              </Box>
            ) : null}
            {(!activeHasSavedConfig || editingConnected)
              ? (active?.config_fields || []).map(renderField)
              : null}
            {active?.config_help ? (
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {active.config_help}
              </Typography>
            ) : null}
            {(!activeHasSavedConfig || editingConnected) && activeNeedsOauth ? (
              <Box
                sx={{
                  border: "1px solid var(--ui-rgba-64-196-255-240)",
                  borderRadius: 2,
                  p: 1.5,
                  background: "var(--ui-rgba-8-24-42-560)"
                }}
              >
                <Box>
                  <Typography variant="subtitle2">
                    {active?.id === "google_workspace"
                      ? "Browser Sign-In"
                      : "Finish Browser Sign-In"}
                  </Typography>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    {active?.id === "google_workspace"
                      ? "AgentArk will save the client ID, optional secret update, and selected bundles first, then open the Google consent screen."
                      : "AgentArk will save these credentials first, then open the provider sign-in flow."}
                  </Typography>
                </Box>
              </Box>
            ) : null}
            {active ? (
              <>
                <Divider sx={{ borderColor: "var(--ui-rgba-112-153-201-120)" }} />
                <Accordion
                  expanded={syncExpanded}
                  onChange={(_, expanded) => setSyncExpanded(expanded)}
                  disableGutters
                  sx={{
                    border: "1px solid var(--ui-rgba-110-160-255-180)",
                    borderRadius: 2,
                    background: "var(--ui-rgba-10-18-32-500)",
                    "&:before": { display: "none" }
                  }}
                >
                  <AccordionSummary
                    expandIcon={<ExpandMoreRoundedIcon />}
                    sx={{
                      px: 1.5,
                      py: 1,
                      minHeight: 0,
                      "& .MuiAccordionSummary-content": { my: 0 }
                    }}
                  >
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{
                        alignItems: { xs: "flex-start", sm: "center" },
                        justifyContent: "space-between",
                        width: "100%",
                        pr: 1
                      }}>
                      <Box>
                        <Typography variant="subtitle2">Background Sync</Typography>
                        <Typography variant="caption" sx={{
                          color: "text.secondary"
                        }}>
                          Defaults to ArkPulse cadence: every 30 minutes. Use a shorter interval only when this integration needs closer polling.
                        </Typography>
                      </Box>
                      <Chip
                        size="small"
                        variant="outlined"
                        color={syncSummaryLabel === "Enabled" ? "success" : "default"}
                        label={syncSummaryLabel}
                      />
                    </Stack>
                  </AccordionSummary>
                  <AccordionDetails sx={{ px: 1.5, pt: 0, pb: 1.5 }}>
                  <Stack spacing={1.25}>
                    {integrationSyncStatusQ.error ? (
                      <Alert severity="warning">
                        Could not load background sync state right now: {asErrorMessage(integrationSyncStatusQ.error)}
                      </Alert>
                    ) : null}
                    {activeSyncStatus && !activeSyncStatus.supported ? (
                      <Alert severity="info">
                        This connector does not expose a reliable background feed yet. Use webhooks or watchers for proactive behavior.
                      </Alert>
                    ) : (
                      <>
                        {activeSyncStatus && !activeSyncStatus.integration_enabled ? (
                          <Alert severity="info">
                            This integration is disabled. Background sync is saved, but polling stays paused until the connector is enabled again.
                          </Alert>
                        ) : null}
                        {activeSyncStatus && !activeSyncStatus.connected ? (
                          <Alert severity="info">
                            Connect this integration first. Sync preferences can be saved now, but polling starts only after credentials work.
                          </Alert>
                        ) : null}
                        <FormControlLabel
                          control={
                            <Switch
                              checked={syncForm.enabled}
                              onChange={(e) => setSyncField("enabled", e.target.checked)}
                            />
                          }
                          label="Enable background sync"
                        />
                        <Grid2 container spacing={1.25}>
                          <Grid2 size={{ xs: 12, sm: 6 }}>
                            <TextField
                              fullWidth
                              size="small"
                              label="Poll every (minutes)"
                              type="number"
                              value={syncForm.poll_interval_minutes}
                              onChange={(e) => setSyncField("poll_interval_minutes", e.target.value)}
                              slotProps={{
                                htmlInput: { min: 1, max: 1440 }
                              }}
                            />
                          </Grid2>
                          <Grid2 size={{ xs: 12, sm: 6 }}>
                            <TextField
                              fullWidth
                              size="small"
                              label="Important threshold (0-100)"
                              type="number"
                              value={syncForm.importance_threshold_percent}
                              onChange={(e) =>
                                setSyncField("importance_threshold_percent", e.target.value)
                              }
                              slotProps={{
                                htmlInput: { min: 10, max: 100 }
                              }}
                            />
                          </Grid2>
                        </Grid2>
                        <FormControlLabel
                          control={
                            <Switch
                              checked={syncForm.notify_on_important}
                              onChange={(e) =>
                                setSyncField("notify_on_important", e.target.checked)
                              }
                            />
                          }
                          label="Notify when something important is detected"
                        />
                        <FormControlLabel
                          control={
                            <Switch
                              checked={syncForm.push_to_preferred_channel}
                              onChange={(e) =>
                                setSyncField("push_to_preferred_channel", e.target.checked)
                              }
                            />
                          }
                          label="Also push important updates to the preferred channel"
                        />
                        {activeSyncStatus ? (
                          <Grid2 container spacing={1}>
                            <Grid2 size={{ xs: 12, sm: 6 }}>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                Last sync
                              </Typography>
                              <Typography variant="body2">
                                {formatDateTime(activeSyncStatus.last_sync_at)}
                              </Typography>
                            </Grid2>
                            <Grid2 size={{ xs: 12, sm: 6 }}>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                Last detected item
                              </Typography>
                              <Typography variant="body2">
                                {formatDateTime(activeSyncStatus.last_item_at)}
                              </Typography>
                            </Grid2>
                            <Grid2 size={{ xs: 12, sm: 6 }}>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                Recent captured items
                              </Typography>
                              <Typography variant="body2">{activeSyncStatus.recent_item_count}</Typography>
                            </Grid2>
                            <Grid2 size={{ xs: 12, sm: 6 }}>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                Feed type
                              </Typography>
                              <Typography variant="body2" sx={{ textTransform: "capitalize" }}>
                                {activeSyncStatus.sync_kind.replace(/_/g, " ")}
                              </Typography>
                            </Grid2>
                          </Grid2>
                        ) : null}
                        {activeSyncStatus?.last_error ? (
                          <Alert severity="warning">{activeSyncStatus.last_error}</Alert>
                        ) : null}
                        {syncNotice ? <Alert severity={syncNotice.kind}>{syncNotice.text}</Alert> : null}
                        <Stack direction="row" spacing={1}>
                          <Button
                            variant="outlined"
                            onClick={() => void runActiveSyncNow()}
                            disabled={!active || syncNowMutation.isPending || !activeSyncStatus?.supported}
                          >
                            {syncNowMutation.isPending ? "Syncing..." : "Sync now"}
                          </Button>
                          <Button
                            variant="contained"
                            onClick={() => void saveActiveSyncSettings()}
                            disabled={!active || saveSyncMutation.isPending || !activeSyncStatus?.supported}
                          >
                            {saveSyncMutation.isPending ? "Saving..." : "Save sync settings"}
                          </Button>
                        </Stack>
                      </>
                    )}
                  </Stack>
                  </AccordionDetails>
                </Accordion>
              </>
            ) : null}
            {formError ? <Alert severity="error">{formError}</Alert> : null}
            {configSuccess ? (
              <Alert severity="success">
                {activeNeedsOauth
                  ? active?.id === "google_workspace"
                    ? "Workspace setup saved. Continue with Google to finish authorization."
                    : "Credentials saved. Click Connect to finish authorization."
                  : activeIsConfigured
                    ? "Credentials saved. AgentArk will confirm live connectivity when this connector is first used."
                    : "API keys validated and saved."}
              </Alert>
            ) : null}
          </Stack>
          </DialogContent>
          <DialogActions sx={{ px: 3, pb: 2 }}>
          {activeHasSavedConfig ? (
            <Button
              color="warning"
              variant="outlined"
              onClick={disconnectActive}
              disabled={saving || disconnectMutation.isPending}
            >
              Disconnect
            </Button>
          ) : null}
          <Button onClick={closeConfig} disabled={saving} size="small" sx={dialogActionButtonSx}>
            Close
          </Button>
          {(!activeHasSavedConfig || editingConnected) && activeNeedsOauth ? (
            <Button
              variant="contained"
              onClick={() => {
                void continueWithOauth();
              }}
              disabled={saving || oauthBusyId === active?.id}
              size="small"
              sx={dialogActionButtonSx}
            >
              {oauthBusyId === active?.id
                ? "Opening..."
                : active?.id === "google_workspace"
                  ? "Continue with Google"
                  : "Save & Connect"}
            </Button>
          ) : null}
          {(!activeHasSavedConfig || editingConnected) && !(active?.id === "google_workspace" && activeNeedsOauth) ? (
            <Button
              variant={activeNeedsOauth ? "outlined" : "contained"}
              onClick={submitConfig}
              disabled={
                saving ||
                active?.status === "starting" ||
                !(active?.config_fields?.length)
              }
              size="small"
              sx={dialogActionButtonSx}
            >
              {saving
                ? "Saving..."
                : str(active?.configure_button, "").trim()
                  ? str(active?.configure_button, "").trim()
                : activeNeedsOauth
                  ? "Save Only"
                  : active?.id === "google_workspace"
                    ? "Save Workspace Bundles"
                    : "Validate & Enable"}
              </Button>
          ) : null}
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
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              Use Ed25519 or ECDSA OpenSSH private keys. RSA and legacy `id_rsa` keys are not supported.
            </Typography>
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
                slotProps={{
                  input: { readOnly: true }
                }}
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
                  <Stack spacing={1}>
                    <TextField
                      fullWidth
                      size="small"
                      label="HTTPS URL"
                      placeholder="https://example.com/mcp"
                      value={mcpForm.url}
                      onChange={(e) => setMcpForm((p) => ({ ...p, url: e.target.value }))}
                    />
                    <Alert severity="warning">
                      HTTP MCP servers must use public HTTPS. Private hosts, localhost, metadata endpoints, and redirects are rejected.
                    </Alert>
                  </Stack>
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
                  <Grid2 size={{ xs: 12 }}>
                    <TextField
                      label="Environment Variables"
                      fullWidth
                      value={mcpForm.env_csv}
                      onChange={(e) => setMcpForm((p) => ({ ...p, env_csv: e.target.value }))}
                      placeholder="KEY1=value1, KEY2=value2"
                      helperText="Comma-separated KEY=VALUE pairs. Values are stored encrypted."
                      multiline
                      minRows={2}
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
                  label="Tool Blocklist"
                  fullWidth
                  value={mcpForm.tool_blocklist_csv}
                  onChange={(e) => setMcpForm((p) => ({ ...p, tool_blocklist_csv: e.target.value }))}
                  placeholder="tool_to_block1, tool_to_block2"
                  helperText="Comma-separated tool names to exclude. Takes precedence over allowlist."
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
