import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  IconButton,
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
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";

type JsonRecord = Record<string, unknown>;

type PluginSdkPanelProps = {
  autoRefresh: boolean;
  embedded?: boolean;
};

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function asRecords(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.map(asRecord) : [];
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function toBool(value: unknown): boolean {
  return value === true || value === "true" || value === 1;
}

function toStrings(value: unknown): string[] {
  return Array.isArray(value)
    ? value.map((item) => (typeof item === "string" ? item.trim() : "")).filter(Boolean)
    : [];
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string") return error;
  const record = asRecord(error);
  return str(record.error, str(record.message, "Request failed"));
}

function humanTs(value: string): string {
  return formatUiDateTime(value, { fallback: "-" });
}

const EMPTY_FORM = {
  id: "",
  name: "",
  base_url: "",
  enabled: true,
  auth_mode: "none",
  auth_header: "X-AgentArk-Plugin-Token",
  token: "",
  clear_token: false,
  subscribed_events: [] as string[]
};

export function PluginSdkPanel({ autoRefresh, embedded = false }: PluginSdkPanelProps) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [hasSavedToken, setHasSavedToken] = useState(false);
  const [form, setForm] = useState(EMPTY_FORM);

  const pluginsQ = useQuery({
    queryKey: ["settings-plugins"],
    queryFn: () => api.rawGet("/plugins"),
    refetchInterval: autoRefresh ? 8000 : false
  });
  const logsQ = useQuery({
    queryKey: ["settings-plugin-logs"],
    queryFn: () => api.rawGet("/plugins/logs?limit=50"),
    refetchInterval: autoRefresh ? 8000 : false
  });

  const createPlugin = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/plugins", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-plugins"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-plugin-logs"] });
    }
  });
  const updatePlugin = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPut(`/plugins/${encodeURIComponent(str(payload.id))}`, payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-plugins"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-plugin-logs"] });
    }
  });
  const deletePlugin = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/plugins/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-plugins"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-plugin-logs"] });
    }
  });
  const refreshPlugin = useMutation({
    mutationFn: (id: string) => api.rawPost(`/plugins/${encodeURIComponent(id)}/refresh`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-plugins"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-plugin-logs"] });
    }
  });
  const testPlugin = useMutation({
    mutationFn: (id: string) => api.rawPost(`/plugins/${encodeURIComponent(id)}/test`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-plugins"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-plugin-logs"] });
    }
  });

  const pluginsPayload = asRecord(pluginsQ.data);
  const plugins = useMemo(() => asRecords(pluginsPayload.plugins), [pluginsPayload]);
  const platformEvents = useMemo(
    () => toStrings(pluginsPayload.platform_events),
    [pluginsPayload]
  );
  const logs = useMemo(() => asRecords(asRecord(logsQ.data).logs), [logsQ.data]);

  function pluginToForm(plugin: JsonRecord) {
    return {
      id: str(plugin.id),
      name: str(plugin.name),
      base_url: str(plugin.base_url),
      enabled: plugin.enabled !== false,
      auth_mode: str(plugin.auth_mode, "none"),
      auth_header: str(plugin.auth_header, "X-AgentArk-Plugin-Token"),
      token: "",
      clear_token: false,
      subscribed_events: toStrings(plugin.subscribed_events)
    };
  }

  useEffect(() => {
    if (!editingId) return;
    const current = plugins.find((plugin) => str(plugin.id) === editingId);
    if (!current) return;
    setForm(pluginToForm(current));
    setHasSavedToken(toBool(current.token_configured));
  }, [editingId, plugins]);

  function resetForm() {
    setEditingId(null);
    setHasSavedToken(false);
    setForm(EMPTY_FORM);
  }

  function openCreateDialog() {
    setError(null);
    setSuccess(null);
    resetForm();
    setDialogOpen(true);
  }

  function openEditDialog(pluginId: string, tokenConfigured: boolean) {
    setError(null);
    setSuccess(null);
    setEditingId(pluginId);
    setHasSavedToken(tokenConfigured);
    setDialogOpen(true);
  }

  function closeDialog() {
    setDialogOpen(false);
    resetForm();
  }

  function setField(name: string, value: string | boolean | string[]) {
    setForm((current) => ({ ...current, [name]: value }));
  }

  function toggleSubscribedEvent(eventName: string) {
    setForm((current) => {
      const next = new Set(current.subscribed_events);
      if (next.has(eventName)) next.delete(eventName);
      else next.add(eventName);
      return {
        ...current,
        subscribed_events: Array.from(next)
      };
    });
  }

  async function handleSave() {
    setError(null);
    setSuccess(null);
    try {
      const payload: JsonRecord = {
        id: form.id || undefined,
        name: form.name.trim() || undefined,
        base_url: form.base_url.trim(),
        enabled: form.enabled,
        auth_mode: form.auth_mode,
        auth_header: form.auth_mode === "header" ? form.auth_header.trim() : undefined,
        token: form.token.trim() || undefined,
        clear_token: form.clear_token,
        subscribed_events: form.subscribed_events
      };
      if (!payload.base_url) {
        throw new Error("Base URL is required.");
      }
      if (form.auth_mode === "header" && !form.auth_header.trim()) {
        throw new Error("Header auth requires an auth header name.");
      }
      const response = editingId
        ? asRecord(await updatePlugin.mutateAsync(payload))
        : asRecord(await createPlugin.mutateAsync(payload));
      const savedPlugin = asRecord(response.plugin);
      const savedId = str(savedPlugin.id, editingId ?? form.id);
      if (savedId) {
        setEditingId(savedId);
      }
      setHasSavedToken(
        toBool(savedPlugin.token_configured) ||
          (!!form.token.trim() && !form.clear_token)
      );
      setForm(
        Object.keys(savedPlugin).length > 0
          ? pluginToForm(savedPlugin)
          : { ...form, id: savedId, token: "", clear_token: false }
      );
      if (editingId) {
        setSuccess("Plugin updated.");
      } else {
        setSuccess("Plugin installed. Run a test before closing if you want.");
      }
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleRefresh(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await refreshPlugin.mutateAsync(id);
      setSuccess("Plugin manifest refreshed.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleTest(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await testPlugin.mutateAsync(id);
      setSuccess("Plugin responded to ping.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleDelete(id: string) {
    setError(null);
    setSuccess(null);
    try {
      await deletePlugin.mutateAsync(id);
      if (editingId === id) closeDialog();
      setSuccess("Plugin removed.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const busy =
    createPlugin.isPending ||
    updatePlugin.isPending ||
    deletePlugin.isPending ||
    refreshPlugin.isPending ||
    testPlugin.isPending;

  return (
    <Stack spacing={2.5}>
      {!embedded ? (
        <Alert severity="info">
          Plugin SDK integrations are external HTTP services. Each plugin exposes
          <strong> </strong>
          <code>/agentark/manifest</code>, <code>/agentark/ping</code>,
          <strong> </strong>
          <code>/agentark/actions/&lt;name&gt;</code>, and
          <strong> </strong>
          <code>/agentark/events/&lt;name&gt;</code>. Tokens stay encrypted and are never
          sent to the model.
        </Alert>
      ) : null}

      {error ? <Alert severity="error">{error}</Alert> : null}
      {success ? <Alert severity="success">{success}</Alert> : null}

      <Box className="list-shell">
        <Stack
          direction={{ xs: "column", sm: "row" }}
          justifyContent="space-between"
          alignItems={{ xs: "flex-start", sm: "center" }}
          spacing={1}
        >
          {!embedded ? (
            <Box>
              <Typography variant="h6">Plugin SDK</Typography>
              <Typography variant="body2" color="text.secondary">
                Install third-party plugins and subscribe them to platform events like webhooks,
                approvals, and task outcomes.
              </Typography>
            </Box>
          ) : <Box sx={{ flex: 1 }} />}
          <Button variant="contained" onClick={openCreateDialog} disabled={busy} sx={{
              textTransform: "none",
              fontWeight: 600,
              px: 3,
            }}>
            Install plugin
          </Button>
        </Stack>
      </Box>

      <Dialog open={dialogOpen} onClose={busy ? undefined : closeDialog} fullWidth maxWidth="md">
        <DialogTitle>{editingId ? "Edit Plugin" : "Install Plugin"}</DialogTitle>
        <DialogContent dividers>
          <Stack direction={{ xs: "column", lg: "row" }} spacing={2} sx={{ pt: 0.5 }}>
            <Stack spacing={1.5} sx={{ flex: 1.3 }}>
              <TextField
                label="Base URL"
                placeholder="https://plugins.example.com/ops"
                value={form.base_url}
                onChange={(e) => setField("base_url", e.target.value)}
                fullWidth
              />
              <TextField
                label="Display Name"
                placeholder="Optional override"
                value={form.name}
                onChange={(e) => setField("name", e.target.value)}
                fullWidth
              />
              <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
                <TextField
                  select
                  label="Auth Mode"
                  value={form.auth_mode}
                  onChange={(e) => setField("auth_mode", e.target.value)}
                  fullWidth
                >
                  <MenuItem value="none">None</MenuItem>
                  <MenuItem value="bearer">Bearer token</MenuItem>
                  <MenuItem value="header">Custom header</MenuItem>
                </TextField>
                <TextField
                  label="Auth Header"
                  value={form.auth_header}
                  onChange={(e) => setField("auth_header", e.target.value)}
                  disabled={form.auth_mode !== "header"}
                  fullWidth
                  helperText={
                    form.auth_mode === "header"
                      ? "Header name used to inject the encrypted token."
                      : "Only used for custom header auth."
                  }
                />
              </Stack>
              <TextField
                label={hasSavedToken ? "API Token / Secret (leave blank to keep current)" : "API Token / Secret"}
                value={form.token}
                onChange={(e) => setField("token", e.target.value)}
                type="password"
                fullWidth
                helperText="Stored encrypted and only injected into plugin HTTP requests."
              />
              {hasSavedToken ? (
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={form.clear_token}
                      onChange={(e) => setField("clear_token", e.target.checked)}
                    />
                  }
                  label="Clear saved token on next save"
                />
              ) : null}
              <Stack direction="row" spacing={2} flexWrap="wrap">
                <FormControlLabel
                  control={
                    <Switch
                      checked={form.enabled}
                      onChange={(e) => setField("enabled", e.target.checked)}
                    />
                  }
                  label="Enabled"
                />
              </Stack>
            </Stack>

            <Stack spacing={1.25} sx={{ flex: 1 }}>
              <Typography variant="subtitle2">Subscribed Events</Typography>
              <Typography variant="caption" color="text.secondary">
                Plugins only receive the events you enable here.
              </Typography>
              <Box
                sx={{
                  border: "1px solid rgba(110, 160, 255, 0.18)",
                  borderRadius: 2,
                  p: 1.25,
                  minHeight: 160
                }}
              >
                <Stack spacing={0.6}>
                  {platformEvents.length ? (
                    platformEvents.map((eventName) => (
                      <FormControlLabel
                        key={eventName}
                        control={
                          <Checkbox
                            checked={form.subscribed_events.includes(eventName)}
                            onChange={() => toggleSubscribedEvent(eventName)}
                          />
                        }
                        label={eventName}
                      />
                    ))
                  ) : (
                    <Typography variant="body2" color="text.secondary">
                      Platform events will appear after the plugin settings payload loads.
                    </Typography>
                  )}
                </Stack>
              </Box>
            </Stack>
          </Stack>
          {!editingId ? (
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 1.5 }}>
              Save once to enable the plugin test button.
            </Typography>
          ) : null}
        </DialogContent>
        <DialogActions sx={{ px: 3, py: 2 }}>
          {editingId ? (
            <Button variant="contained" onClick={() => handleTest(editingId)} disabled={busy} sx={{
                textTransform: "none",
                fontWeight: 600,
                px: 3,
              }}>
              Test plugin
            </Button>
          ) : null}
          <Box sx={{ flex: 1 }} />
          <Button onClick={closeDialog} disabled={busy}>
            Cancel
          </Button>
          <Button variant="contained" onClick={handleSave} disabled={busy} sx={{
              textTransform: "none",
              fontWeight: 600,
              px: 3,
            }}>
            {editingId ? "Save Plugin" : "Install Plugin"}
          </Button>
        </DialogActions>
      </Dialog>

      <Box className="list-shell">
        <Stack spacing={1.5}>
          <Stack direction="row" justifyContent="space-between" alignItems="center">
            <Typography variant="h6">Installed Plugins</Typography>
            <Chip
              label={`${plugins.length} installed`}
              color={plugins.length ? "primary" : "default"}
              variant={plugins.length ? "filled" : "outlined"}
            />
          </Stack>

          {plugins.length > 0 ? (
            <Stack spacing={1.25}>
              {plugins.map((plugin) => {
                const manifest = asRecord(plugin.manifest);
                const actions = toStrings(plugin.registered_actions);
                const subscribed = toStrings(plugin.subscribed_events);
                const availableEvents = toStrings(plugin.available_events);
                const pluginId = str(plugin.id);
                return (
                  <Box
                    key={pluginId}
                    sx={{
                      border: "1px solid rgba(110, 160, 255, 0.18)",
                      borderRadius: 2,
                      p: 1.5
                    }}
                  >
                    <Stack direction={{ xs: "column", lg: "row" }} spacing={2} justifyContent="space-between">
                      <Stack spacing={0.8} sx={{ minWidth: 0 }}>
                        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
                          <Typography variant="subtitle1">{str(plugin.name, pluginId)}</Typography>
                          <Chip
                            size="small"
                            label={toBool(plugin.enabled) ? "Enabled" : "Disabled"}
                            color={toBool(plugin.enabled) ? "success" : "default"}
                          />
                          <Chip
                            size="small"
                            label={str(manifest.version, "unknown")}
                            variant="outlined"
                          />
                          {toBool(plugin.token_configured) ? (
                            <Chip size="small" label="Token saved" variant="outlined" />
                          ) : null}
                        </Stack>
                        <Typography variant="body2" color="text.secondary">
                          {str(plugin.description, str(manifest.description, "No description provided."))}
                        </Typography>
                        <Typography variant="caption" color="text.secondary">
                          Base URL: {str(plugin.base_url)}
                        </Typography>
                        <Typography variant="caption" color="text.secondary">
                          Last synced: {humanTs(str(plugin.last_synced_at || plugin.updated_at))}
                        </Typography>
                        {str(plugin.last_error) ? (
                          <Alert severity="warning" sx={{ mt: 0.5 }}>
                            {str(plugin.last_error)}
                          </Alert>
                        ) : null}
                        <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                          {actions.map((actionName) => (
                            <Chip key={actionName} size="small" label={actionName} />
                          ))}
                          {!actions.length ? <Chip size="small" label="No actions" variant="outlined" /> : null}
                        </Stack>
                        <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                          {subscribed.map((eventName) => (
                            <Chip key={`sub-${pluginId}-${eventName}`} size="small" label={`Subscribed: ${eventName}`} variant="outlined" />
                          ))}
                          {!subscribed.length ? (
                            <Chip size="small" label="No subscribed events" variant="outlined" />
                          ) : null}
                        </Stack>
                        <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                          {availableEvents.map((eventName) => (
                            <Chip key={`avail-${pluginId}-${eventName}`} size="small" label={`Supports: ${eventName}`} variant="outlined" />
                          ))}
                        </Stack>
                      </Stack>

                      <Stack direction="row" spacing={1} alignItems="flex-start" flexWrap="wrap">
                        <Button
                          variant="outlined"
                          onClick={() => {
                            openEditDialog(pluginId, toBool(plugin.token_configured));
                          }}
                          disabled={busy}
                        >
                          Edit
                        </Button>
                        <Button
                          variant="outlined"
                          onClick={() => handleRefresh(pluginId)}
                          disabled={busy}
                        >
                          Refresh
                        </Button>
                        <Button
                          variant="outlined"
                          onClick={() => handleTest(pluginId)}
                          disabled={busy}
                        >
                          Test
                        </Button>
                        <Button
                          color="error"
                          variant="outlined"
                          onClick={() => handleDelete(pluginId)}
                          disabled={busy}
                        >
                          Delete
                        </Button>
                        <Tooltip title="Copy runtime action names">
                          <span>
                            <IconButton
                              size="small"
                              onClick={async () => {
                                try {
                                  await navigator.clipboard.writeText(actions.join("\n"));
                                  setSuccess(`Copied ${actions.length} action name(s).`);
                                } catch {
                                  setError("Could not copy action names.");
                                }
                              }}
                              disabled={!actions.length}
                            >
                              <ContentCopyRoundedIcon fontSize="small" />
                            </IconButton>
                          </span>
                        </Tooltip>
                      </Stack>
                    </Stack>
                  </Box>
                );
              })}
            </Stack>
          ) : null}
        </Stack>
      </Box>

      <Box className="list-shell">
        <Stack spacing={1.5}>
          <Typography variant="h6">Plugin Activity</Typography>
          <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
          <Table size="small" sx={{ minWidth: 760 }}>
            <TableHead>
              <TableRow>
                <TableCell>Time</TableCell>
                <TableCell>Plugin</TableCell>
                <TableCell>Kind</TableCell>
                <TableCell>Subject</TableCell>
                <TableCell>Outcome</TableCell>
                <TableCell>Message</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {logs.length ? (
                logs.map((entry) => (
                  <TableRow key={str(entry.id)}>
                    <TableCell>{humanTs(str(entry.created_at))}</TableCell>
                    <TableCell>{str(entry.plugin_name, str(entry.plugin_id, "-"))}</TableCell>
                    <TableCell>{str(entry.kind, "-")}</TableCell>
                    <TableCell>{str(entry.subject, "-")}</TableCell>
                    <TableCell>{str(entry.outcome, "-")}</TableCell>
                    <TableCell sx={{ maxWidth: 420 }}>
                      <Typography variant="body2" color="text.secondary">
                        {str(entry.message, "-")}
                      </Typography>
                    </TableCell>
                  </TableRow>
                ))
              ) : (
                <TableRow>
                  <TableCell colSpan={6}>
                    <Typography variant="body2" color="text.secondary">
                      No plugin activity yet.
                    </Typography>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
          </TableContainer>
        </Stack>
      </Box>
    </Stack>
  );
}
