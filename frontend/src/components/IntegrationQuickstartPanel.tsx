import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import { ChannelIcon } from "./IntegrationsPanel";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Grid2,
  IconButton,
  MenuItem,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Tooltip,
  Typography
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../api/client";
import type { IntegrationItem } from "../types";

type JsonRecord = Record<string, unknown>;

type OperationDraft = {
  id: string;
  name: string;
  method: string;
  path: string;
  description: string;
  read_only: boolean;
  enabled: boolean;
  body_required: boolean;
  parameters: unknown[];
  default_headers: Record<string, string>;
  default_query: Record<string, string>;
};

type CustomApiForm = {
  name: string;
  description: string;
  base_url: string;
  source_mode: "openapi_text" | "openapi_url" | "curl";
  openapi_url: string;
  openapi_text: string;
  curl_text: string;
  auth_mode: string;
  auth_header: string;
  auth_name: string;
  auth_username: string;
  secret: string;
  enabled: boolean;
  operations: OperationDraft[];
  notes: string[];
};

type WebhookForm = {
  name: string;
  provider: string;
  auth_mode: string;
  match_mode: string;
  secret: string;
  enabled: boolean;
  require_approval: boolean;
  notify_on_queued: boolean;
  notify_on_success: boolean;
  notify_on_failure: boolean;
  output_target: string;
  output_channel: string;
  instruction: string;
};

type IntegrationQuickstartPanelProps = {
  integrations: IntegrationItem[];
  autoRefresh: boolean;
  loading?: boolean;
  loadError?: string | null;
  onConfigureIntegration: (integration: IntegrationItem) => void;
  embedded?: boolean;
  mode?: "all" | "custom-apis-only";
};

const FEATURED_PREBUILT = ["google_workspace", "github", "jira", "sentry", "notion"];
const INTEGRATION_SORT_ORDER: Record<string, number> = {
  google_workspace: 0, github: 1, "1password": 2, notion: 3, jira: 4, sentry: 5, linear: 6,
  google_analytics: 10, google_search_console: 11, garmin: 12, shopify: 13, social_analytics: 14,
  google_places: 99,
};

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function asRecords(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.map(asRecord) : [];
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function toBool(value: unknown): boolean {
  return value === true || value === "true" || value === 1;
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return str(asRecord(error).error, "Request failed");
}

function defaultWebhookForm(provider = "github"): WebhookForm {
  return {
    name:
      provider === "github"
        ? "GitHub Events"
        : provider === "sentry"
          ? "Sentry Alerts"
          : provider === "gitlab"
            ? "GitLab Events"
            : "Incoming Webhook",
    provider,
    auth_mode: provider === "github" ? "hmac_sha256" : "header_token",
    match_mode: provider === "sentry" ? "failures_only" : "all",
    secret: "",
    enabled: true,
    require_approval: false,
    notify_on_queued: false,
    notify_on_success: true,
    notify_on_failure: true,
    output_target: "preferred",
    output_channel: "",
    instruction:
      provider === "github"
        ? "When CI or deployment events arrive, inspect the change or failure and take the next safe action."
        : provider === "sentry"
          ? "When alerts arrive, summarize impact, identify likely cause, and take the next safe action."
          : "Analyze this event and take the next safe action. If it is only informational, summarize it briefly and stop."
  };
}

function defaultCustomApiForm(): CustomApiForm {
  return {
    name: "",
    description: "",
    base_url: "",
    source_mode: "openapi_text",
    openapi_url: "",
    openapi_text: "",
    curl_text: "",
    auth_mode: "none",
    auth_header: "",
    auth_name: "",
    auth_username: "",
    secret: "",
    enabled: true,
    operations: [],
    notes: []
  };
}

function webhookEndpoint(id: string): string {
  if (typeof window === "undefined") {
    return `/webhook/inbound/${encodeURIComponent(id)}`;
  }
  return `${window.location.origin}/webhook/inbound/${encodeURIComponent(id)}`;
}

function completionChannelOptions(integrations: IntegrationItem[]): Array<{ id: string; label: string }> {
  const items = [{ id: "preferred", label: "Preferred channel" }];
  const seen = new Set<string>(["preferred"]);
  const push = (id: string, label: string) => {
    const key = id.trim().toLowerCase();
    if (!key || seen.has(key)) return;
    seen.add(key);
    items.push({ id: key, label });
  };
  const labelFallbacks: Record<string, string> = {
    telegram: "Telegram",
    slack: "Slack",
    discord: "Discord",
    matrix: "Matrix",
    teams: "Teams",
    whatsapp: "WhatsApp"
  };
  integrations.forEach((item) => {
    if (item.status !== "connected") return;
    push(item.id, item.name || labelFallbacks[item.id.trim().toLowerCase()] || item.id);
  });
  if (integrations.some((item) => item.id === "google_workspace" && item.status === "connected")) {
    push("email", "Email");
  }
  return items;
}

type ConnectorDisplayState =
  | "connected"
  | "starting"
  | "configured"
  | "needs_auth"
  | "error"
  | "available";

function connectorDisplayState(integration: IntegrationItem): ConnectorDisplayState {
  if (integration.status === "error") return "error";
  if (integration.status === "starting") return "starting";
  if (integration.status === "configured") return "configured";
  if (integration.status === "needs_auth") return "needs_auth";
  if (integration.status === "disabled") {
    return "configured";
  }
  if (integration.status === "connected") return integration.enabled ? "connected" : "configured";
  return "available";
}

function connectorSortRank(integration: IntegrationItem): number {
  const state = connectorDisplayState(integration);
  if (state === "connected") return 0;
  if (state === "starting") return 1;
  if (state === "configured") return 2;
  if (state === "needs_auth") return 3;
  if (state === "error") return 4;
  return 5;
}

function connectorActionLabel(integration: IntegrationItem): string {
  const state = connectorDisplayState(integration);
  if (state === "connected") return "Manage";
  if (state === "starting") return "Manage";
  if (state === "configured") return "Manage";
  if (state === "needs_auth") return "Resume";
  if (state === "error") return "Fix";
  return "Connect";
}

function connectorStatusLabel(integration: IntegrationItem): string | null {
  const state = connectorDisplayState(integration);
  if (state === "connected") return "Connected";
  if (state === "starting") return "Starting";
  if (state === "configured") return "Configured";
  if (state === "needs_auth") return "Needs sign-in";
  if (state === "error") return "Needs attention";
  return null;
}

function parseOperationDraft(value: unknown): OperationDraft {
  const row = asRecord(value);
  return {
    id: str(row.id),
    name: str(row.name),
    method: str(row.method),
    path: str(row.path),
    description: str(row.description),
    read_only: toBool(row.read_only),
    enabled: row.enabled !== false,
    body_required: toBool(row.body_required),
    parameters: Array.isArray(row.parameters) ? row.parameters : [],
    default_headers: asRecord(row.default_headers) as Record<string, string>,
    default_query: asRecord(row.default_query) as Record<string, string>
  };
}

export function IntegrationQuickstartPanel({
  integrations,
  autoRefresh,
  loading = false,
  loadError = null,
  onConfigureIntegration,
  embedded = false,
  mode = "all"
}: IntegrationQuickstartPanelProps) {
  const showCustomApisOnly = mode === "custom-apis-only";
  const queryClient = useQueryClient();
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [prebuiltOpen, setPrebuiltOpen] = useState(false);
  const [webhookOpen, setWebhookOpen] = useState(false);
  const [customApiOpen, setCustomApiOpen] = useState(false);
  const [createdWebhookId, setCreatedWebhookId] = useState<string | null>(null);
  const [webhookForm, setWebhookForm] = useState<WebhookForm>(defaultWebhookForm());
  const [customApiForm, setCustomApiForm] = useState<CustomApiForm>(defaultCustomApiForm());

  const webhooksQ = useQuery({
    queryKey: ["integrations-quickstart-webhooks"],
    queryFn: () => api.rawGet("/webhooks/sources"),
    refetchInterval: autoRefresh ? 8000 : false
  });
  const customApisQ = useQuery({
    queryKey: ["integrations-quickstart-custom-apis"],
    queryFn: () => api.rawGet("/custom-apis"),
    refetchInterval: autoRefresh ? 8000 : false
  });

  const previewCustomApi = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/custom-apis/preview", payload)
  });
  const createCustomApi = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/custom-apis", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations-quickstart-custom-apis"] });
    }
  });
  const deleteCustomApi = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/custom-apis/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations-quickstart-custom-apis"] });
    }
  });
  const testCustomApi = useMutation({
    mutationFn: (id: string) => api.rawPost(`/custom-apis/${encodeURIComponent(id)}/test`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations-quickstart-custom-apis"] });
    }
  });
  const createWebhook = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/webhooks/sources", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations-quickstart-webhooks"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
    }
  });
  const testWebhook = useMutation({
    mutationFn: (id: string) => api.rawPost(`/webhooks/sources/${encodeURIComponent(id)}/test`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["integrations-quickstart-webhooks"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    }
  });

  const webhookSources = useMemo(() => asRecords(asRecord(webhooksQ.data).sources), [webhooksQ.data]);
  const customApis = useMemo(() => asRecords(asRecord(customApisQ.data).custom_apis), [customApisQ.data]);
  const channelOptions = useMemo(() => completionChannelOptions(integrations), [integrations]);
  const featuredIntegrations = useMemo(() => {
    const featured = FEATURED_PREBUILT
      .map((id) => integrations.find((item) => item.id === id))
      .filter((item): item is IntegrationItem => Boolean(item));
    return featured.length > 0 ? featured : integrations.slice(0, 6);
  }, [integrations]);
  const actionButtonSx = {
    minWidth: 0,
    width: "auto",
    maxWidth: "fit-content",
    alignSelf: "flex-start",
    flex: "0 0 auto",
    whiteSpace: "nowrap",
  } as const;
  const tagChipSx = {
    height: 22,
    borderRadius: 1,
    background: "rgba(14, 25, 43, 0.95)",
    border: "1px solid rgba(112,153,201,0.18)",
    color: "rgba(198,214,235,0.82)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.63rem",
      fontWeight: 700,
      letterSpacing: "0.08em",
      textTransform: "uppercase"
    }
  } as const;
  const countChipSx = {
    height: 22,
    borderRadius: 1,
    background: "rgba(14, 25, 43, 0.92)",
    border: "1px solid rgba(112,153,201,0.16)",
    color: "rgba(173,192,214,0.9)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.64rem",
      fontWeight: 700,
      letterSpacing: "0.08em",
      textTransform: "uppercase"
    }
  } as const;

  async function copyText(value: string, successText: string) {
    try {
      await navigator.clipboard.writeText(value);
      setNotice({ kind: "success", text: successText });
    } catch {
      setNotice({ kind: "error", text: "Clipboard copy failed." });
    }
  }

  async function handleWebhookSave() {
    setNotice(null);
    try {
      const response = asRecord(
        await createWebhook.mutateAsync({
          name: webhookForm.name.trim(),
          provider: webhookForm.provider,
          auth_mode: webhookForm.auth_mode,
          match_mode: webhookForm.match_mode,
          secret: webhookForm.secret.trim() || undefined,
          enabled: webhookForm.enabled,
          require_approval: webhookForm.require_approval,
          notify_on_queued: webhookForm.notify_on_queued,
          notify_on_success: webhookForm.notify_on_success,
          notify_on_failure: webhookForm.notify_on_failure,
          output_target: webhookForm.output_target,
          output_channel:
            webhookForm.output_target === "channel" ? webhookForm.output_channel.trim() : undefined,
          instruction: webhookForm.instruction.trim()
        })
      );
      const source = asRecord(response.source);
      const sourceId = str(source.id);
      setCreatedWebhookId(sourceId || null);
      setNotice({ kind: "success", text: "Webhook created. Copy the endpoint and run a test." });
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  async function handleWebhookTest(id: string) {
    setNotice(null);
    try {
      await testWebhook.mutateAsync(id);
      setNotice({ kind: "success", text: "Synthetic webhook test queued." });
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  async function handlePreviewCustomApi() {
    setNotice(null);
    try {
      const response = asRecord(
        await previewCustomApi.mutateAsync({
          name: customApiForm.name.trim() || undefined,
          base_url: customApiForm.base_url.trim() || undefined,
          openapi_url:
            customApiForm.source_mode === "openapi_url"
              ? customApiForm.openapi_url.trim() || undefined
              : undefined,
          openapi_text:
            customApiForm.source_mode === "openapi_text"
              ? customApiForm.openapi_text
              : undefined,
          curl_text:
            customApiForm.source_mode === "curl" ? customApiForm.curl_text : undefined
        })
      );
      const preview = asRecord(response.preview);
      setCustomApiForm((current) => ({
        ...current,
        name: str(preview.suggested_name, current.name),
        base_url: str(preview.base_url, current.base_url),
        auth_mode: str(preview.auth_mode, current.auth_mode),
        auth_header: str(preview.auth_header),
        auth_name: str(preview.auth_name),
        auth_username: str(preview.auth_username),
        operations: asRecords(preview.operations).map(parseOperationDraft),
        notes: Array.isArray(preview.notes) ? preview.notes.map((item) => str(item)).filter(Boolean) : []
      }));
      setNotice({ kind: "success", text: "API schema parsed. Review the imported endpoints before saving." });
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  async function handleSaveCustomApi() {
    setNotice(null);
    try {
      await createCustomApi.mutateAsync({
        name: customApiForm.name.trim(),
        description: customApiForm.description.trim(),
        base_url: customApiForm.base_url.trim(),
        enabled: customApiForm.enabled,
        auth_mode: customApiForm.auth_mode,
        auth_header: customApiForm.auth_header.trim() || undefined,
        auth_name: customApiForm.auth_name.trim() || undefined,
        auth_username: customApiForm.auth_username.trim() || undefined,
        secret: customApiForm.secret.trim() || undefined,
        operations: customApiForm.operations
      });
      setNotice({ kind: "success", text: "Custom API imported. The selected endpoints are now available as tools." });
      setCustomApiOpen(false);
      setCustomApiForm(defaultCustomApiForm());
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  async function handleDeleteCustomApi(id: string) {
    setNotice(null);
    try {
      await deleteCustomApi.mutateAsync(id);
      setNotice({ kind: "success", text: "Custom API removed." });
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  async function handleTestCustomApi(id: string) {
    setNotice(null);
    try {
      await testCustomApi.mutateAsync(id);
      setNotice({ kind: "success", text: "Custom API test completed." });
    } catch (error) {
      setNotice({ kind: "error", text: errMessage(error) });
    }
  }

  return (
    <Stack spacing={2}>
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}

      {!showCustomApisOnly ? (
      <Box className="list-shell">
        <Stack spacing={1.5}>
          {!embedded ? (
            <Box>
              <Typography variant="subtitle2">Add Connections</Typography>
              <Typography variant="caption" color="text.secondary">
                Start here for prebuilt connectors, incoming webhooks, and imported APIs. Secrets stay encrypted and never go to the model.
              </Typography>
            </Box>
          ) : null}
          <Grid2 container spacing={1.25}>
            <Grid2 size={{ xs: 12, md: 6, lg: 4 }}>
              <Box sx={{ p: 1.5, borderRadius: 1.5, border: "1px solid rgba(112,153,201,0.16)", background: "rgba(7,17,32,0.6)", height: "100%" }}>
                <Stack spacing={1.25} sx={{ height: "100%", justifyContent: "space-between" }}>
                  <Box>
                    <Typography variant="subtitle2">Prebuilt Connector</Typography>
                    <Typography variant="body2" color="text.secondary">
                      Pick Google Workspace, GitHub, Jira, Sentry, Notion, Slack, and other ready-made integrations, then connect with OAuth or API keys.
                    </Typography>
                  </Box>
                  <Stack spacing={1}>
                    <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                      {featuredIntegrations.slice(0, 4).map((integration) => (
                        <Chip
                          key={integration.id}
                          size="small"
                          label={integration.name}
                          onClick={() => onConfigureIntegration(integration)}
                          clickable
                          sx={tagChipSx}
                        />
                      ))}
                    </Stack>
                    <Button variant="contained" sx={actionButtonSx} onClick={() => setPrebuiltOpen(true)}>
                      Open connectors
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            </Grid2>
            <Grid2 size={{ xs: 12, md: 6, lg: 4 }}>
              <Box sx={{ p: 1.5, borderRadius: 1.5, border: "1px solid rgba(112,153,201,0.16)", background: "rgba(7,17,32,0.6)", height: "100%" }}>
                <Stack spacing={1.25} sx={{ height: "100%", justifyContent: "space-between" }}>
                  <Box>
                    <Typography variant="subtitle2">Incoming Webhook</Typography>
                    <Typography variant="body2" color="text.secondary">
                      Choose a GitHub, Sentry, GitLab, or generic template, copy the endpoint, and let AgentArk act when events arrive.
                    </Typography>
                  </Box>
                  <Stack direction="row" alignItems="center" justifyContent="space-between">
                    <Chip size="small" label={`${webhookSources.length} configured`} sx={countChipSx} />
                    <Button variant="contained" sx={actionButtonSx} onClick={() => {
                      setWebhookForm(defaultWebhookForm("github"));
                      setCreatedWebhookId(null);
                      setWebhookOpen(true);
                    }}>
                      New webhook
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            </Grid2>
            <Grid2 size={{ xs: 12, md: 6, lg: 4 }}>
              <Box sx={{ p: 1.5, borderRadius: 1.5, border: "1px solid rgba(112,153,201,0.16)", background: "rgba(7,17,32,0.6)", height: "100%" }}>
                <Stack spacing={1.25} sx={{ height: "100%", justifyContent: "space-between" }}>
                  <Box>
                    <Typography variant="subtitle2">Custom API</Typography>
                    <Typography variant="body2" color="text.secondary">
                      Import approved API endpoints as tools the agent can use safely.
                    </Typography>
                  </Box>
                  <Stack direction="row" alignItems="center" justifyContent="space-between">
                    <Chip size="small" label={`${customApis.length} imported`} sx={countChipSx} />
                    <Button variant="contained" sx={actionButtonSx} onClick={() => {
                      setCustomApiForm(defaultCustomApiForm());
                      setCustomApiOpen(true);
                    }}>
                      Import API
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            </Grid2>
          </Grid2>
        </Stack>
      </Box>
      ) : (
        <Box className="list-shell">
          <Stack spacing={1.5}>
            <Stack direction="row" alignItems="center" justifyContent="space-between">
              <Box>
                <Typography variant="subtitle2">Custom APIs</Typography>
                <Typography variant="caption" color="text.secondary">
                  Import approved API endpoints as tools the agent can use safely. Secrets stay encrypted.
                </Typography>
              </Box>
              <Button variant="contained" sx={actionButtonSx} onClick={() => {
                setCustomApiForm(defaultCustomApiForm());
                setCustomApiOpen(true);
              }}>
                Import API
              </Button>
            </Stack>
          </Stack>
        </Box>
      )}

      {!showCustomApisOnly && webhookSources.length > 0 ? (
        <Box className="list-shell">
          <Stack spacing={1}>
            <Typography variant="subtitle2">Incoming Webhooks</Typography>
            <Table size="small" sx={{ "& td, & th": { borderColor: "rgba(112,153,201,0.12)", py: 0.75 } }}>
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>Provider</TableCell>
                  <TableCell>Trigger</TableCell>
                  <TableCell>Endpoint</TableCell>
                  <TableCell align="right">Ops</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {webhookSources.slice(0, 6).map((source) => {
                  const sourceId = str(source.id);
                  const endpoint = webhookEndpoint(sourceId);
                  return (
                    <TableRow key={sourceId}>
                      <TableCell>{str(source.name, sourceId)}</TableCell>
                      <TableCell>{str(source.provider, "generic")}</TableCell>
                      <TableCell>{str(source.match_mode, "all").replace(/_/g, " ")}</TableCell>
                      <TableCell sx={{ fontFamily: "monospace", fontSize: "0.76rem" }}>{endpoint}</TableCell>
                      <TableCell align="right">
                        <Stack direction="row" spacing={0.5} justifyContent="flex-end">
                          <Tooltip title="Copy endpoint">
                            <IconButton size="small" onClick={() => copyText(endpoint, "Webhook endpoint copied.")}>
                              <ContentCopyRoundedIcon fontSize="inherit" />
                            </IconButton>
                          </Tooltip>
                          <Button size="small" variant="text" onClick={() => handleWebhookTest(sourceId)}>
                            Test
                          </Button>
                        </Stack>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </Stack>
        </Box>
      ) : null}

      {customApis.length > 0 ? (
        <Box className="list-shell">
          <Stack spacing={1}>
            <Typography variant="subtitle2">Imported Custom APIs</Typography>
            <Table size="small" sx={{ "& td, & th": { borderColor: "rgba(112,153,201,0.12)", py: 0.75 } }}>
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>Base URL</TableCell>
                  <TableCell>Actions</TableCell>
                  <TableCell>Last Test</TableCell>
                  <TableCell align="right">Ops</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {customApis.map((item) => {
                  const config = asRecord(item);
                  const configId = str(config.id);
                  return (
                    <TableRow key={configId}>
                      <TableCell>{str(config.name, configId)}</TableCell>
                      <TableCell sx={{ fontFamily: "monospace", fontSize: "0.76rem" }}>{str(config.base_url)}</TableCell>
                      <TableCell>{String(Number(config.action_count) || 0)}</TableCell>
                      <TableCell>{str(config.last_test_outcome, "-")}</TableCell>
                      <TableCell align="right">
                        <Stack direction="row" spacing={0.5} justifyContent="flex-end">
                          <Button
                            size="small"
                            variant="text"
                            disabled={!str(config.test_action_name)}
                            onClick={() => handleTestCustomApi(configId)}
                          >
                            Test
                          </Button>
                          <Button size="small" color="error" variant="text" onClick={() => handleDeleteCustomApi(configId)}>
                            Remove
                          </Button>
                        </Stack>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </Stack>
        </Box>
      ) : null}

      <Dialog open={prebuiltOpen} onClose={() => setPrebuiltOpen(false)} fullWidth maxWidth="sm">
        <DialogTitle>Add Integration</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1}>
            <Typography variant="body2" color="text.secondary">
              Choose a connector, then finish auth, scopes, and triggers in its setup dialog.
            </Typography>
            {loadError && integrations.length === 0 ? (
              <Alert severity="error">Failed to load available integrations: {loadError}</Alert>
            ) : loading && integrations.length === 0 ? (
              <Stack spacing={1.25} alignItems="center" sx={{ py: 4 }}>
                <CircularProgress size={22} />
                <Typography variant="body2" color="text.secondary">
                  Loading available integrations...
                </Typography>
              </Stack>
            ) : integrations.length === 0 ? (
              <Alert severity="info">No integrations are available yet. Refresh the page and try again.</Alert>
            ) : (
              [...integrations].sort((a, b) => {
                const rankDiff = connectorSortRank(a) - connectorSortRank(b);
                if (rankDiff !== 0) return rankDiff;
                const orderA = INTEGRATION_SORT_ORDER[a.id] ?? 50;
                const orderB = INTEGRATION_SORT_ORDER[b.id] ?? 50;
                if (orderA !== orderB) return orderA - orderB;
                return a.name.localeCompare(b.name);
              }).map((integration) => {
                const state = connectorDisplayState(integration);
                const isConfigured =
                  state === "connected" || state === "starting" || state === "configured";
                return (
                  <Box
                    key={integration.id}
                    sx={{
                      p: 1.25,
                      borderRadius: 1.25,
                      border: isConfigured ? "1px solid rgba(64, 196, 255, 0.24)" : "1px solid rgba(112,153,201,0.16)",
                      background: isConfigured ? "rgba(8, 24, 42, 0.56)" : "rgba(7,17,32,0.45)"
                    }}
                  >
                    <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={1}>
                      <Box sx={{ minWidth: 0 }}>
                        <Stack direction="row" spacing={0.75} alignItems="center">
                          <ChannelIcon name={integration.id || integration.name} size={20} />
                          <Typography variant="subtitle2" noWrap>{integration.name}</Typography>
                        </Stack>
                        <Typography variant="caption" color="text.secondary">
                          {integration.description}
                        </Typography>
                      </Box>
                      <Stack direction="row" spacing={0.75} alignItems="center" sx={{ flexShrink: 0 }}>
                        {connectorStatusLabel(integration) ? (
                          <Chip size="small" label={connectorStatusLabel(integration)} sx={countChipSx} />
                        ) : null}
                        <Button
                          size="small"
                          variant={state === "available" ? "contained" : "outlined"}
                          sx={actionButtonSx}
                          onClick={() => {
                            setPrebuiltOpen(false);
                            onConfigureIntegration(integration);
                          }}
                        >
                          {connectorActionLabel(integration)}
                        </Button>
                      </Stack>
                    </Stack>
                  </Box>
                );
              })
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setPrebuiltOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>

      <Dialog open={webhookOpen} onClose={() => setWebhookOpen(false)} fullWidth maxWidth="md">
        <DialogTitle>Incoming Webhook</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
              {["github", "sentry", "gitlab", "generic"].map((template) => (
                <Chip
                  key={template}
                  label={template === "generic" ? "Generic" : template.charAt(0).toUpperCase() + template.slice(1)}
                  color={webhookForm.provider === template ? "primary" : "default"}
                  variant={webhookForm.provider === template ? "filled" : "outlined"}
                  onClick={() => {
                    setWebhookForm(defaultWebhookForm(template));
                    setCreatedWebhookId(null);
                  }}
                />
              ))}
            </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
              <TextField label="Name" fullWidth value={webhookForm.name} onChange={(e) => setWebhookForm((current) => ({ ...current, name: e.target.value }))} />
              <TextField select label="Template" fullWidth value={webhookForm.provider} onChange={(e) => {
                setWebhookForm(defaultWebhookForm(e.target.value));
                setCreatedWebhookId(null);
              }}>
                <MenuItem value="github">GitHub</MenuItem>
                <MenuItem value="sentry">Sentry</MenuItem>
                <MenuItem value="gitlab">GitLab</MenuItem>
                <MenuItem value="generic">Generic</MenuItem>
              </TextField>
            </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
              <TextField select label="Auth" fullWidth value={webhookForm.auth_mode} onChange={(e) => setWebhookForm((current) => ({ ...current, auth_mode: e.target.value }))}>
                <MenuItem value="header_token">Header token</MenuItem>
                <MenuItem value="hmac_sha256">HMAC SHA-256</MenuItem>
                <MenuItem value="bearer_token">Bearer token</MenuItem>
                <MenuItem value="none">None</MenuItem>
              </TextField>
              <TextField select label="Trigger" fullWidth value={webhookForm.match_mode} onChange={(e) => setWebhookForm((current) => ({ ...current, match_mode: e.target.value }))}>
                <MenuItem value="all">All events</MenuItem>
                <MenuItem value="failures_only">Failures only</MenuItem>
                <MenuItem value="changes_only">Changes only</MenuItem>
              </TextField>
            </Stack>
            <TextField
              label="Secret / Token"
              type="password"
              fullWidth
              value={webhookForm.secret}
              onChange={(e) => setWebhookForm((current) => ({ ...current, secret: e.target.value }))}
              helperText="Stored encrypted and only used to verify incoming events."
            />
            <TextField
              label="Autonomous Instruction"
              fullWidth
              multiline
              minRows={3}
              value={webhookForm.instruction}
              onChange={(e) => setWebhookForm((current) => ({ ...current, instruction: e.target.value }))}
            />
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
              <TextField
                select
                label="Completion Delivery"
                fullWidth
                value={webhookForm.output_target}
                onChange={(e) => setWebhookForm((current) => ({ ...current, output_target: e.target.value }))}
              >
                <MenuItem value="preferred">Preferred channel</MenuItem>
                <MenuItem value="channel">Specific channel</MenuItem>
              </TextField>
              <TextField
                select
                label="Completion Channel"
                fullWidth
                value={webhookForm.output_channel}
                disabled={webhookForm.output_target !== "channel"}
                onChange={(e) => setWebhookForm((current) => ({ ...current, output_channel: e.target.value }))}
              >
                {channelOptions.filter((item) => item.id !== "preferred").map((item) => (
                  <MenuItem key={item.id} value={item.id}>{item.label}</MenuItem>
                ))}
              </TextField>
            </Stack>
            <Stack direction="row" spacing={2} flexWrap="wrap" useFlexGap>
              <FormControlLabel control={<Switch checked={webhookForm.enabled} onChange={(e) => setWebhookForm((current) => ({ ...current, enabled: e.target.checked }))} />} label="Enabled" />
              <FormControlLabel control={<Switch checked={webhookForm.require_approval} onChange={(e) => setWebhookForm((current) => ({ ...current, require_approval: e.target.checked }))} />} label="Require approval" />
              <FormControlLabel control={<Switch checked={webhookForm.notify_on_success} onChange={(e) => setWebhookForm((current) => ({ ...current, notify_on_success: e.target.checked }))} />} label="Notify on success" />
              <FormControlLabel control={<Switch checked={webhookForm.notify_on_failure} onChange={(e) => setWebhookForm((current) => ({ ...current, notify_on_failure: e.target.checked }))} />} label="Notify on failure" />
            </Stack>
            {createdWebhookId ? (
              <Alert severity="success" action={
                <Stack direction="row" spacing={1}>
                  <Button color="inherit" size="small" onClick={() => void copyText(webhookEndpoint(createdWebhookId), "Webhook endpoint copied.")}>
                    Copy endpoint
                  </Button>
                  <Button color="inherit" size="small" onClick={() => void handleWebhookTest(createdWebhookId)}>
                    Run test
                  </Button>
                </Stack>
              }>
                Endpoint ready: <Box component="span" sx={{ fontFamily: "monospace" }}>{webhookEndpoint(createdWebhookId)}</Box>
              </Alert>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setWebhookOpen(false)}>Close</Button>
          <Button
            variant="contained"
            onClick={() => void handleWebhookSave()}
            disabled={
              createWebhook.isPending ||
              !webhookForm.name.trim() ||
              (webhookForm.output_target === "channel" && !webhookForm.output_channel.trim())
            }
          >
            {createWebhook.isPending ? "Saving..." : "Save Webhook"}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog open={customApiOpen} onClose={() => setCustomApiOpen(false)} fullWidth maxWidth="lg">
        <DialogTitle>Import Custom API</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
              <Chip label="OpenAPI text" color={customApiForm.source_mode === "openapi_text" ? "primary" : "default"} variant={customApiForm.source_mode === "openapi_text" ? "filled" : "outlined"} onClick={() => setCustomApiForm((current) => ({ ...current, source_mode: "openapi_text" }))} />
              <Chip label="OpenAPI URL" color={customApiForm.source_mode === "openapi_url" ? "primary" : "default"} variant={customApiForm.source_mode === "openapi_url" ? "filled" : "outlined"} onClick={() => setCustomApiForm((current) => ({ ...current, source_mode: "openapi_url" }))} />
              <Chip label="Sample curl" color={customApiForm.source_mode === "curl" ? "primary" : "default"} variant={customApiForm.source_mode === "curl" ? "filled" : "outlined"} onClick={() => setCustomApiForm((current) => ({ ...current, source_mode: "curl" }))} />
            </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
              <TextField label="Name" fullWidth value={customApiForm.name} onChange={(e) => setCustomApiForm((current) => ({ ...current, name: e.target.value }))} />
              <TextField label="Base URL override" fullWidth value={customApiForm.base_url} onChange={(e) => setCustomApiForm((current) => ({ ...current, base_url: e.target.value }))} />
            </Stack>
            {customApiForm.source_mode === "openapi_url" ? (
              <TextField label="OpenAPI URL" fullWidth value={customApiForm.openapi_url} onChange={(e) => setCustomApiForm((current) => ({ ...current, openapi_url: e.target.value }))} />
            ) : null}
            {customApiForm.source_mode === "openapi_text" ? (
              <TextField label="OpenAPI Document" fullWidth multiline minRows={10} value={customApiForm.openapi_text} onChange={(e) => setCustomApiForm((current) => ({ ...current, openapi_text: e.target.value }))} />
            ) : null}
            {customApiForm.source_mode === "curl" ? (
              <TextField label="Sample curl" fullWidth multiline minRows={6} value={customApiForm.curl_text} onChange={(e) => setCustomApiForm((current) => ({ ...current, curl_text: e.target.value }))} />
            ) : null}
            <Button variant="contained" onClick={() => void handlePreviewCustomApi()} disabled={previewCustomApi.isPending}>
              {previewCustomApi.isPending ? "Analyzing..." : "Discover Endpoints"}
            </Button>
            {customApiForm.notes.length > 0 ? (
              <Alert severity="info">
                {customApiForm.notes.join(" ")}
              </Alert>
            ) : null}
            {customApiForm.operations.length > 0 ? (
              <>
                <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
                  <TextField select label="Auth" fullWidth value={customApiForm.auth_mode} onChange={(e) => setCustomApiForm((current) => ({ ...current, auth_mode: e.target.value }))}>
                    <MenuItem value="none">None</MenuItem>
                    <MenuItem value="bearer">Bearer token</MenuItem>
                    <MenuItem value="api_key_header">API key header</MenuItem>
                    <MenuItem value="api_key_query">API key query</MenuItem>
                    <MenuItem value="oauth2">OAuth token</MenuItem>
                    <MenuItem value="basic">Basic auth</MenuItem>
                  </TextField>
                  <TextField label="Auth header" fullWidth value={customApiForm.auth_header} onChange={(e) => setCustomApiForm((current) => ({ ...current, auth_header: e.target.value }))} />
                  <TextField label="Auth name" fullWidth value={customApiForm.auth_name} onChange={(e) => setCustomApiForm((current) => ({ ...current, auth_name: e.target.value }))} />
                </Stack>
                <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
                  <TextField label="Username" fullWidth value={customApiForm.auth_username} onChange={(e) => setCustomApiForm((current) => ({ ...current, auth_username: e.target.value }))} />
                  <TextField label="Token / Secret" type="password" fullWidth value={customApiForm.secret} onChange={(e) => setCustomApiForm((current) => ({ ...current, secret: e.target.value }))} helperText="Stored encrypted and injected only into API requests." />
                </Stack>
                <Table size="small" sx={{ "& td, & th": { borderColor: "rgba(112,153,201,0.12)", py: 0.75 } }}>
                  <TableHead>
                    <TableRow>
                      <TableCell>Import</TableCell>
                      <TableCell>Endpoint</TableCell>
                      <TableCell>Access</TableCell>
                      <TableCell>Notes</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {customApiForm.operations.map((operation, index) => (
                      <TableRow key={`${operation.id}-${index}`}>
                        <TableCell padding="checkbox">
                          <Checkbox
                            checked={operation.enabled}
                            onChange={(e) => setCustomApiForm((current) => ({
                              ...current,
                              operations: current.operations.map((item, itemIndex) => itemIndex === index ? { ...item, enabled: e.target.checked } : item)
                            }))}
                          />
                        </TableCell>
                        <TableCell>
                          <Typography variant="body2">{operation.name || `${operation.method} ${operation.path}`}</Typography>
                          <Typography variant="caption" color="text.secondary">
                            {operation.method} {operation.path}
                          </Typography>
                        </TableCell>
                        <TableCell>
                          <TextField
                            select
                            size="small"
                            value={operation.read_only ? "read" : "write"}
                            onChange={(e) => setCustomApiForm((current) => ({
                              ...current,
                              operations: current.operations.map((item, itemIndex) => itemIndex === index ? { ...item, read_only: e.target.value === "read" } : item)
                            }))}
                          >
                            <MenuItem value="read">Read-only</MenuItem>
                            <MenuItem value="write">Write enabled</MenuItem>
                          </TextField>
                        </TableCell>
                        <TableCell>
                          <Typography variant="caption" color="text.secondary">
                            {operation.body_required ? "Requires request body." : "No request body required."}
                          </Typography>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
                <FormControlLabel control={<Switch checked={customApiForm.enabled} onChange={(e) => setCustomApiForm((current) => ({ ...current, enabled: e.target.checked }))} />} label="Enable imported actions immediately" />
              </>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCustomApiOpen(false)}>Close</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveCustomApi()}
            disabled={
              createCustomApi.isPending ||
              customApiForm.operations.length === 0 ||
              !customApiForm.name.trim() ||
              !customApiForm.base_url.trim() ||
              ((customApiForm.auth_mode === "bearer" ||
                customApiForm.auth_mode === "api_key_header" ||
                customApiForm.auth_mode === "api_key_query" ||
                customApiForm.auth_mode === "oauth2" ||
                customApiForm.auth_mode === "basic") &&
                !customApiForm.secret.trim()) ||
              (customApiForm.auth_mode === "basic" && !customApiForm.auth_username.trim())
            }
          >
            {createCustomApi.isPending ? "Importing..." : "Import API"}
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}
