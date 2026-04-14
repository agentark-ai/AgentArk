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
  MenuItem,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Tooltip,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
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

type IntegrationQuickstartPanelProps = {
  integrations: IntegrationItem[];
  autoRefresh: boolean;
  loading?: boolean;
  loadError?: string | null;
  onConfigureIntegration: (integration: IntegrationItem) => void;
  embedded?: boolean;
  mode?: "all" | "custom-apis-only";
};

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
  const [customApiOpen, setCustomApiOpen] = useState(false);
  const [customApiForm, setCustomApiForm] = useState<CustomApiForm>(defaultCustomApiForm());
  const customApisQ = useQuery({
    queryKey: ["integrations-quickstart-custom-apis"],
    queryFn: () => api.rawGet("/custom-apis"),
    enabled: showCustomApisOnly,
    refetchInterval: autoRefresh && showCustomApisOnly ? 8000 : false
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
  const customApis = useMemo(() => asRecords(asRecord(customApisQ.data).custom_apis), [customApisQ.data]);
  const sortedIntegrations = useMemo(() => {
    return [...integrations].sort((a, b) => {
      const rankDiff = connectorSortRank(a) - connectorSortRank(b);
      if (rankDiff !== 0) return rankDiff;
      const orderA = INTEGRATION_SORT_ORDER[a.id] ?? 50;
      const orderB = INTEGRATION_SORT_ORDER[b.id] ?? 50;
      if (orderA !== orderB) return orderA - orderB;
      return a.name.localeCompare(b.name);
    });
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
      letterSpacing: 0,
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
      letterSpacing: 0,
      textTransform: "uppercase"
    }
  } as const;

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
              <Typography variant="subtitle2">Prebuilt Connectors</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Connect Google Workspace, GitHub, Jira, Sentry, Notion, and other first-party integrations directly here.
              </Typography>
            </Box>
          ) : null}
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", sm: "center" }
            }}>
            <Box>
              <Typography variant="subtitle2">Available Connectors</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Use the dedicated Webhooks & APIs page for incoming event sources and imported custom tools.
              </Typography>
            </Box>
            <Chip size="small" label={`${sortedIntegrations.length} available`} sx={countChipSx} />
          </Stack>
          {loadError && sortedIntegrations.length === 0 ? (
            <Alert severity="error">Failed to load available integrations: {loadError}</Alert>
          ) : loading && sortedIntegrations.length === 0 ? (
            <Stack
              spacing={1.25}
              sx={{
                alignItems: "center",
                py: 4
              }}>
              <CircularProgress size={22} />
              <Typography variant="body2" sx={{
                color: "text.secondary"
              }}>
                Loading available integrations...
              </Typography>
            </Stack>
          ) : sortedIntegrations.length === 0 ? (
            <Alert severity="info">No prebuilt connectors are available yet. Refresh the page and try again.</Alert>
          ) : (
            <Grid2 container spacing={1.25}>
              {sortedIntegrations.map((integration) => {
                const state = connectorDisplayState(integration);
                const isConfigured =
                  state === "connected" || state === "starting" || state === "configured";
                return (
                  <Grid2 key={integration.id} size={{ xs: 12, md: 6, xl: 4 }}>
                    <Box sx={{ p: 1.5, borderRadius: 1.5, border: isConfigured ? "1px solid rgba(64,196,255,0.24)" : "1px solid rgba(112,153,201,0.16)", background: isConfigured ? "rgba(8,24,42,0.56)" : "rgba(7,17,32,0.6)", height: "100%" }}>
                      <Stack spacing={1.1} sx={{ height: "100%", justifyContent: "space-between" }}>
                        <Box>
                          <Stack
                            direction="row"
                            spacing={0.9}
                            sx={{
                              alignItems: "center",
                              mb: 0.75
                            }}>
                            <ChannelIcon name={integration.id || integration.name} size={20} />
                            <Typography variant="subtitle2">{integration.name}</Typography>
                          </Stack>
                          <Typography variant="body2" sx={{
                            color: "text.secondary"
                          }}>
                            {integration.description}
                          </Typography>
                        </Box>
                        <Stack
                          direction="row"
                          spacing={0.75}
                          sx={{
                            alignItems: "center",
                            justifyContent: "space-between"
                          }}>
                          <Stack direction="row" spacing={0.5} useFlexGap sx={{
                            flexWrap: "wrap"
                          }}>
                            <Chip size="small" label="Connector" sx={tagChipSx} />
                            {connectorStatusLabel(integration) ? (
                              <Chip size="small" label={connectorStatusLabel(integration)} sx={countChipSx} />
                            ) : null}
                          </Stack>
                          <Button
                            size="small"
                            variant={state === "available" ? "contained" : "outlined"}
                            sx={actionButtonSx}
                            onClick={() => onConfigureIntegration(integration)}
                          >
                            {connectorActionLabel(integration)}
                          </Button>
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
      ) : (
        <Box className="list-shell">
          <Stack spacing={1.5}>
            <Stack
              direction="row"
              sx={{
                alignItems: "center",
                justifyContent: "space-between"
              }}>
              <Box>
                <Typography variant="subtitle2">Custom APIs</Typography>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
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
      {showCustomApisOnly && customApis.length > 0 ? (
        <Box className="list-shell">
          <Stack spacing={1}>
            <Typography variant="subtitle2">Imported Custom APIs</Typography>
            <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
            <Table size="small" sx={{ minWidth: 720, "& td, & th": { borderColor: "rgba(112,153,201,0.12)", py: 0.75 } }}>
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
                        <Stack direction="row" spacing={0.5} sx={{
                          justifyContent: "flex-end"
                        }}>
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
            </TableContainer>
          </Stack>
        </Box>
      ) : null}
      <Dialog open={customApiOpen} onClose={() => setCustomApiOpen(false)} fullWidth maxWidth="lg">
        <DialogTitle>Import Custom API</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack direction="row" spacing={0.75} useFlexGap sx={{
              flexWrap: "wrap"
            }}>
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
                <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto", maxHeight: "none" }}>
                <Table size="small" sx={{ minWidth: 720, "& td, & th": { borderColor: "rgba(112,153,201,0.12)", py: 0.75 } }}>
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
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
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
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {operation.body_required ? "Requires request body." : "No request body required."}
                          </Typography>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
                </TableContainer>
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



