import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
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
import { humanizeMachineLabel, humanizeStatusLabel } from "../lib/displayLabels";
import type { IntegrationItem } from "../types";

type JsonRecord = Record<string, unknown>;

type WebhooksPanelProps = {
  autoRefresh: boolean;
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

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string") return error;
  const record = asRecord(error);
  return str(record.error, str(record.message, "Request failed"));
}

function humanTs(value: string): string {
  return formatUiDateTime(value, { fallback: "-" });
}

function integrationLabel(id: string, integrations: IntegrationItem[]): string {
  const normalized = id.trim().toLowerCase();
  if (!normalized) return "-";
  const found = integrations.find((item) => item.id.trim().toLowerCase() === normalized);
  if (found?.name) return found.name;
  const fallbacks: Record<string, string> = {
    email: "Email",
    telegram: "Telegram",
    slack: "Slack",
    discord: "Discord",
    matrix: "Matrix",
    teams: "Teams",
    whatsapp: "WhatsApp"
  };
  return fallbacks[normalized] || normalized;
}

function completionTargetSummary(source: JsonRecord, integrations: IntegrationItem[]): string {
  const target = str(source.output_target, "none");
  if (target === "preferred") return "Preferred channel";
  if (target === "channel") {
    const channel = str(source.output_channel);
    return channel ? integrationLabel(channel, integrations) : "Specific channel";
  }
  return "Off";
}

function notificationSummary(source: JsonRecord): string {
  const enabled = [
    toBool(source.notify_on_queued) ? "queued" : "",
    toBool(source.notify_on_success) ? "success" : "",
    toBool(source.notify_on_failure) ? "failure" : ""
  ].filter(Boolean);
  return enabled.length ? enabled.join(", ") : "off";
}

function completionChannelOptions(integrations: IntegrationItem[]): Array<{ id: string; label: string }> {
  const options: Array<{ id: string; label: string }> = [];
  const seen = new Set<string>();
  const labelFallbacks: Record<string, string> = {
    telegram: "Telegram",
    slack: "Slack",
    discord: "Discord",
    matrix: "Matrix",
    teams: "Teams",
    whatsapp: "WhatsApp"
  };
  const push = (id: string, label: string) => {
    const normalized = id.trim().toLowerCase();
    if (!normalized || seen.has(normalized)) return;
    seen.add(normalized);
    options.push({ id: normalized, label });
  };
  integrations.forEach((item) => {
    if (item.status !== "connected") return;
    push(item.id, item.name || labelFallbacks[item.id.trim().toLowerCase()] || item.id);
  });
  if (integrations.some((item) => item.id === "google_workspace" && item.status === "connected")) {
    push("email", "Email");
  }
  return options;
}

function defaultAuthMode(provider: string): string {
  return provider === "github" ? "hmac_sha256" : "header_token";
}

function defaultEventHeader(provider: string): string {
  if (provider === "github") return "X-GitHub-Event";
  if (provider === "gitlab") return "X-Gitlab-Event";
  if (provider === "sentry") return "Sentry-Hook-Resource";
  return "X-Event-Type";
}

function defaultSecretHeader(provider: string, authMode: string): string {
  if (authMode === "bearer_token") return "Authorization";
  if (authMode === "hmac_sha256") return provider === "github" ? "X-Hub-Signature-256" : "X-AgentArk-Signature";
  if (authMode === "none") return "";
  return provider === "gitlab" ? "X-Gitlab-Token" : "X-AgentArk-Webhook-Secret";
}

const EMPTY_FORM = {
  id: "",
  name: "",
  provider: "generic",
  enabled: true,
  auth_mode: "header_token",
  match_mode: "all",
  event_header: "X-Event-Type",
  secret_header: "X-AgentArk-Webhook-Secret",
  secret: "",
  clear_secret: false,
  allow_duplicate: false,
  require_approval: false,
  dedupe_window_secs: 900,
  notify_on_queued: false,
  notify_on_success: true,
  notify_on_failure: true,
  output_target: "none",
  output_channel: "",
  instruction: "Analyze this event and take the next safe action. If it is only informational, summarize it briefly and stop."
};

export function WebhooksPanel({ autoRefresh }: WebhooksPanelProps) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [hasSavedSecret, setHasSavedSecret] = useState(false);
  const [form, setForm] = useState(EMPTY_FORM);

  const sourcesQ = useQuery({
    queryKey: ["settings-webhook-sources"],
    queryFn: () => api.rawGet("/webhooks/sources"),
    refetchInterval: autoRefresh ? 8000 : false
  });
  const eventsQ = useQuery({
    queryKey: ["settings-webhook-events"],
    queryFn: () => api.rawGet("/webhooks/events?limit=40"),
    refetchInterval: autoRefresh ? 8000 : false
  });
  const integrationsQ = useQuery({
    queryKey: ["webhook-output-integrations"],
    queryFn: () => api.getIntegrations(),
    refetchInterval: false
  });

  const createSource = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/webhooks/sources", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-sources"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    }
  });
  const updateSource = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPut(`/webhooks/sources/${encodeURIComponent(str(payload.id))}`, payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-sources"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
    }
  });
  const deleteSource = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/webhooks/sources/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-sources"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
    }
  });
  const testSource = useMutation({
    mutationFn: (id: string) => api.rawPost(`/webhooks/sources/${encodeURIComponent(id)}/test`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-sources"] });
      await queryClient.invalidateQueries({ queryKey: ["settings-webhook-events"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    }
  });

  const sources = useMemo(() => asRecords(asRecord(sourcesQ.data).sources), [sourcesQ.data]);
  const events = useMemo(() => asRecords(asRecord(eventsQ.data).events), [eventsQ.data]);
  const integrations = useMemo(() => integrationsQ.data?.integrations || [], [integrationsQ.data]);
  const channelOptions = useMemo(() => completionChannelOptions(integrations), [integrations]);

  function sourceToForm(source: JsonRecord) {
    const provider = str(source.provider, "generic");
    const authMode = str(source.auth_mode, defaultAuthMode(provider));
    return {
      id: str(source.id),
      name: str(source.name),
      provider,
      enabled: toBool(source.enabled),
      auth_mode: authMode,
      match_mode: str(source.match_mode, "all"),
      event_header: str(source.event_header, defaultEventHeader(provider)),
      secret_header: str(
        source.secret_header,
        defaultSecretHeader(provider, authMode)
      ),
      secret: "",
      clear_secret: false,
      allow_duplicate: toBool(source.allow_duplicate),
      require_approval: toBool(source.require_approval),
      dedupe_window_secs: Number(source.dedupe_window_secs) || 900,
      notify_on_queued: toBool(source.notify_on_queued),
      notify_on_success: source.notify_on_success !== false,
      notify_on_failure: source.notify_on_failure !== false,
      output_target: str(source.output_target, "none"),
      output_channel: str(source.output_channel),
      instruction: str(source.instruction, EMPTY_FORM.instruction)
    };
  }

  useEffect(() => {
    if (!editingId) return;
    const current = sources.find((source) => str(source.id) === editingId);
    if (!current) return;
    setForm(sourceToForm(current));
    setHasSavedSecret(toBool(current.secret_configured));
  }, [editingId, sources]);

  function resetForm() {
    setEditingId(null);
    setHasSavedSecret(false);
    setForm(EMPTY_FORM);
  }

  function openCreateDialog() {
    setError(null);
    setSuccess(null);
    resetForm();
    setDialogOpen(true);
  }

  function openEditDialog(sourceId: string, secretConfigured: boolean) {
    setError(null);
    setSuccess(null);
    setEditingId(sourceId);
    setHasSavedSecret(secretConfigured);
    setDialogOpen(true);
  }

  function closeDialog() {
    setDialogOpen(false);
    resetForm();
  }

  function setField(name: string, value: string | boolean | number) {
    setForm((current) => ({ ...current, [name]: value }));
  }

  async function handleSave() {
    setError(null);
    setSuccess(null);
    try {
      const payload: JsonRecord = {
        id: form.id || undefined,
        name: form.name.trim(),
        provider: form.provider,
        enabled: form.enabled,
        auth_mode: form.auth_mode,
        match_mode: form.match_mode,
        event_header: form.event_header.trim(),
        secret_header: form.secret_header.trim(),
        secret: form.secret.trim(),
        clear_secret: form.clear_secret,
        allow_duplicate: form.allow_duplicate,
        require_approval: form.require_approval,
        dedupe_window_secs: Number(form.dedupe_window_secs) || 900,
        notify_on_queued: form.notify_on_queued,
        notify_on_success: form.notify_on_success,
        notify_on_failure: form.notify_on_failure,
        output_target: form.output_target,
        output_channel: form.output_target === "channel" ? form.output_channel.trim() : undefined,
        instruction: form.instruction.trim()
      };
      const response = editingId
        ? asRecord(await updateSource.mutateAsync(payload))
        : asRecord(await createSource.mutateAsync(payload));
      const savedSource = asRecord(response.source);
      const savedId = str(savedSource.id, editingId ?? form.id);
      if (savedId) {
        setEditingId(savedId);
      }
      setHasSavedSecret(
        toBool(savedSource.secret_configured) ||
          (form.auth_mode !== "none" && !!form.secret.trim() && !form.clear_secret)
      );
      setForm(
        Object.keys(savedSource).length > 0
          ? sourceToForm(savedSource)
          : { ...form, id: savedId, secret: "", clear_secret: false }
      );
      if (editingId) {
        setSuccess("Webhook source updated.");
      } else {
        setSuccess("Webhook source created. Run a test before closing if you want.");
      }
    } catch (err) {
      setError(errMessage(err));
    }
  }

  const busy = createSource.isPending || updateSource.isPending || deleteSource.isPending || testSource.isPending;
  const missingCompletionChannel = form.output_target === "channel" && !form.output_channel.trim();

  return (
    <Stack spacing={2.5}>
      <Alert severity="info">
        Add inbound webhook sources here. Secrets are stored encrypted, never echoed back in normal UI responses, and not passed to the LLM. Matched events create autonomous work without waiting for chat, can notify on queued/succeeded/failed states, and can push completion output to the preferred or source-specific channel.
      </Alert>
      {error ? <Alert severity="error">{error}</Alert> : null}
      {success ? <Alert severity="success">{success}</Alert> : null}
      <Box className="list-shell">
        <Stack
          direction={{ xs: "column", sm: "row" }}
          spacing={1}
          sx={{
            justifyContent: "space-between",
            alignItems: { xs: "flex-start", sm: "center" }
          }}>
          <Box>
            <Typography variant="h6">Webhook Sources</Typography>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Create inbound webhook sources in a guided popup, then run a synthetic test without leaving the editor.
            </Typography>
          </Box>
          <Button variant="contained" onClick={openCreateDialog} disabled={busy}>
            New Webhook Source
          </Button>
        </Stack>
      </Box>
      <Dialog open={dialogOpen} onClose={busy ? undefined : closeDialog} fullWidth maxWidth="md">
        <DialogTitle>{editingId ? "Edit Webhook Source" : "New Webhook Source"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
          <TextField label="Name" size="small" fullWidth value={form.name} onChange={(e) => setField("name", e.target.value)} />
          <TextField
            select
            label="Provider"
            size="small"
            fullWidth
            value={form.provider}
            onChange={(e) => {
              const provider = e.target.value;
              const authMode = defaultAuthMode(provider);
              setForm((current) => ({
                ...current,
                provider,
                auth_mode: current.id ? current.auth_mode : authMode,
                event_header: defaultEventHeader(provider),
                secret_header: defaultSecretHeader(provider, current.id ? current.auth_mode : authMode)
              }));
            }}
          >
            <MenuItem value="generic">Generic</MenuItem>
            <MenuItem value="github">GitHub</MenuItem>
            <MenuItem value="gitlab">GitLab</MenuItem>
            <MenuItem value="sentry">Sentry</MenuItem>
            <MenuItem value="pagerduty">PagerDuty</MenuItem>
          </TextField>
        </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
          <TextField
            select
            label="Auth Mode"
            size="small"
            fullWidth
            value={form.auth_mode}
            onChange={(e) => {
              const authMode = e.target.value;
              setForm((current) => ({
                ...current,
                auth_mode: authMode,
                secret_header: defaultSecretHeader(current.provider, authMode)
              }));
            }}
          >
            <MenuItem value="header_token">Header token</MenuItem>
            <MenuItem value="hmac_sha256">HMAC SHA-256</MenuItem>
            <MenuItem value="bearer_token">Bearer token</MenuItem>
            <MenuItem value="none">None</MenuItem>
          </TextField>
          <TextField select label="Match Mode" size="small" fullWidth value={form.match_mode} onChange={(e) => setField("match_mode", e.target.value)}>
            <MenuItem value="all">All events</MenuItem>
            <MenuItem value="failures_only">Failures only</MenuItem>
            <MenuItem value="changes_only">Changes only</MenuItem>
          </TextField>
        </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
          <TextField label="Event Header" size="small" fullWidth value={form.event_header} onChange={(e) => setField("event_header", e.target.value)} />
          <TextField label="Secret / Signature Header" size="small" fullWidth value={form.secret_header} onChange={(e) => setField("secret_header", e.target.value)} disabled={form.auth_mode === "none"} />
        </Stack>
            <TextField
          label={hasSavedSecret ? "Secret (leave blank to keep current)" : "Secret"}
          size="small"
          fullWidth
          type="password"
          value={form.secret}
          onChange={(e) => setField("secret", e.target.value)}
          helperText={hasSavedSecret ? "Stored encrypted. Leave blank to keep the saved value." : "Stored encrypted and used only for webhook verification."}
          disabled={form.auth_mode === "none"}
        />
            {hasSavedSecret && form.auth_mode !== "none" ? (
              <FormControlLabel control={<Switch checked={form.clear_secret} onChange={(e) => setField("clear_secret", e.target.checked)} />} label="Clear saved secret on next save" />
            ) : null}
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
          <TextField
            label="Dedupe Window (seconds)"
            size="small"
            type="number"
            fullWidth
            value={form.dedupe_window_secs}
            onChange={(e) => setField("dedupe_window_secs", Number(e.target.value) || 900)}
          />
          <Stack
            direction="row"
            spacing={1.5}
            useFlexGap
            sx={{
              alignItems: "center",
              flexWrap: "wrap",
              minHeight: 40
            }}>
            <FormControlLabel control={<Switch checked={form.enabled} onChange={(e) => setField("enabled", e.target.checked)} />} label="Enabled" />
            <FormControlLabel control={<Switch checked={form.require_approval} onChange={(e) => setField("require_approval", e.target.checked)} />} label="Require approval" />
            <FormControlLabel control={<Switch checked={form.allow_duplicate} onChange={(e) => setField("allow_duplicate", e.target.checked)} />} label="Allow duplicates" />
          </Stack>
        </Stack>
            <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
          <TextField
            select
            label="Completion Delivery"
            size="small"
            fullWidth
            value={form.output_target}
            onChange={(e) => setField("output_target", e.target.value)}
            helperText="Preferred uses the existing preferred notification channel. Specific channel pushes only when the webhook task completes."
          >
            <MenuItem value="none">Off</MenuItem>
            <MenuItem value="preferred">Preferred channel</MenuItem>
            <MenuItem value="channel">Specific channel</MenuItem>
          </TextField>
          <TextField
            select
            label="Completion Channel"
            size="small"
            fullWidth
            value={form.output_channel}
            onChange={(e) => setField("output_channel", e.target.value)}
            disabled={form.output_target !== "channel"}
            helperText={form.output_target === "channel" ? "Available messaging/integration destinations for completion pushes." : "Enable Specific channel delivery first."}
          >
            <MenuItem value="">Select channel</MenuItem>
            {form.output_channel && !channelOptions.some((option) => option.id === form.output_channel) ? (
              <MenuItem value={form.output_channel}>{form.output_channel}</MenuItem>
            ) : null}
            {channelOptions.map((option) => (
              <MenuItem key={option.id} value={option.id}>
                {option.label}
              </MenuItem>
            ))}
          </TextField>
        </Stack>
            <Stack
              direction="row"
              spacing={1.5}
              useFlexGap
              sx={{
                alignItems: "center",
                flexWrap: "wrap",
                minHeight: 40
              }}>
          <FormControlLabel control={<Switch checked={form.notify_on_queued} onChange={(e) => setField("notify_on_queued", e.target.checked)} />} label="Notify on queued" />
          <FormControlLabel control={<Switch checked={form.notify_on_success} onChange={(e) => setField("notify_on_success", e.target.checked)} />} label="Notify on success" />
          <FormControlLabel control={<Switch checked={form.notify_on_failure} onChange={(e) => setField("notify_on_failure", e.target.checked)} />} label="Notify on failure" />
        </Stack>
            <TextField
          label="Autonomous Instruction"
          fullWidth
          multiline
          minRows={4}
          value={form.instruction}
          onChange={(e) => setField("instruction", e.target.value)}
          helperText="This is what the agent sees after the webhook is normalized. Put operator intent here, not secrets."
        />
            {!editingId ? (
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Save once to enable synthetic test runs for this source.
              </Typography>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions sx={{ px: 3, py: 2 }}>
          {editingId ? (
            <Button
              variant="contained"
              onClick={async () => {
                try {
                  setError(null);
                  setSuccess(null);
                  await testSource.mutateAsync(editingId);
                  setSuccess("Synthetic test event queued.");
                } catch (err) {
                  setError(errMessage(err));
                }
              }}
              disabled={busy}
            >
              Run test
            </Button>
          ) : null}
          <Box sx={{ flex: 1 }} />
          <Button onClick={closeDialog} disabled={busy}>
            Cancel
          </Button>
          <Button variant="contained" onClick={handleSave} disabled={busy || !form.name.trim() || missingCompletionChannel}>
            {editingId ? "Save Source" : "Create Source"}
          </Button>
        </DialogActions>
      </Dialog>
      <Stack spacing={1.5}>
        <Typography variant="h6">Configured Sources</Typography>
        {sourcesQ.error ? <Alert severity="error">{errMessage(sourcesQ.error)}</Alert> : null}
        {sources.length > 0 ? (
          sources.map((source) => {
            const sourceId = str(source.id);
            const ingestPath = str(source.ingest_path);
            const ingestUrl = typeof window === "undefined" ? ingestPath : `${window.location.origin}${ingestPath}`;
            return (
              <Box key={sourceId} sx={{ p: 1.5, borderRadius: 2, border: "1px solid var(--ui-rgba-120-180-255-180)", background: "var(--ui-rgba-8-20-38-600)" }}>
                <Stack direction={{ xs: "column", md: "row" }} spacing={1.5} sx={{
                  justifyContent: "space-between"
                }}>
                  <Stack spacing={0.6}>
                    <Stack
                      direction="row"
                      spacing={1}
                      useFlexGap
                      sx={{
                        alignItems: "center",
                        flexWrap: "wrap"
                      }}>
                      <Typography variant="subtitle1">{str(source.name)}</Typography>
                      <Chip size="small" label={toBool(source.enabled) ? "Enabled" : "Disabled"} color={toBool(source.enabled) ? "success" : "default"} />
                      <Chip size="small" variant="outlined" label={str(source.provider, "generic")} />
                      <Chip size="small" variant="outlined" color={toBool(source.secret_configured) ? "success" : "warning"} label={toBool(source.secret_configured) ? "Secret saved" : "No secret"} />
                    </Stack>
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {ingestPath}
                    </Typography>
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      Last activity: {str(source.last_received_at) ? `${humanTs(str(source.last_received_at))} (${str(source.last_outcome, "unknown")})` : "No deliveries yet"}
                    </Typography>
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      Notifications: {notificationSummary(source)} | Completion: {completionTargetSummary(source, integrations)}
                    </Typography>
                  </Stack>
                  <Stack
                    direction="row"
                    spacing={1}
                    useFlexGap
                    sx={{
                      alignItems: "center",
                      flexWrap: "wrap"
                    }}>
                    <Tooltip title="Copy webhook URL">
                      <IconButton size="small" onClick={async () => { await navigator.clipboard.writeText(ingestUrl); setSuccess("Webhook URL copied."); }}>
                        <ContentCopyRoundedIcon fontSize="small" />
                      </IconButton>
                    </Tooltip>
                    <Button size="small" variant="outlined" onClick={() => openEditDialog(sourceId, toBool(source.secret_configured))}>Edit</Button>
                    <Button size="small" variant="outlined" onClick={async () => { try { setError(null); setSuccess(null); await testSource.mutateAsync(sourceId); setSuccess("Synthetic test event queued."); } catch (err) { setError(errMessage(err)); } }} disabled={busy}>Run test</Button>
                    <Button size="small" color="error" variant="outlined" onClick={async () => { if (!window.confirm(`Delete webhook source '${str(source.name)}'?`)) return; try { setError(null); setSuccess(null); await deleteSource.mutateAsync(sourceId); if (editingId === sourceId) closeDialog(); setSuccess("Webhook source deleted."); } catch (err) { setError(errMessage(err)); } }} disabled={busy}>Delete</Button>
                  </Stack>
                </Stack>
              </Box>
            );
          })
        ) : null}
      </Stack>
      <Divider />
      <Stack spacing={1}>
        <Typography variant="h6">Recent Event Deliveries</Typography>
        <Typography variant="caption" sx={{
          color: "text.secondary"
        }}>
          This is the ingress view. Tasks and Trace show what the agent actually executed after a webhook matched.
        </Typography>
        {eventsQ.error ? <Alert severity="error">{errMessage(eventsQ.error)}</Alert> : null}
        <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
        <Table size="small" sx={{ minWidth: 760 }}>
          <TableHead>
            <TableRow>
              <TableCell>When</TableCell>
              <TableCell>Source</TableCell>
              <TableCell>Event</TableCell>
              <TableCell>Outcome</TableCell>
              <TableCell>Details</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {events.length === 0 ? (
              <TableRow>
                <TableCell colSpan={5}>
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>No webhook deliveries yet.</Typography>
                </TableCell>
              </TableRow>
            ) : (
              events.map((event) => (
                <TableRow key={str(event.id)}>
                  <TableCell>{humanTs(str(event.received_at))}</TableCell>
                  <TableCell>{str(event.source_name)}</TableCell>
                  <TableCell>
                    <Stack spacing={0.2}>
                      <Typography variant="body2">{humanizeMachineLabel(str(event.event_type, "webhook"))}</Typography>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>{str(event.subject)}</Typography>
                    </Stack>
                  </TableCell>
                  <TableCell>
                    <Chip size="small" label={humanizeStatusLabel(str(event.outcome))} color={str(event.outcome) === "queued" ? "success" : str(event.outcome) === "auth_failed" ? "error" : "default"} />
                  </TableCell>
                  <TableCell>
                    <Stack spacing={0.2}>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>{str(event.message)}</Typography>
                      {str(event.task_id) ? <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>Task: {str(event.task_id)}</Typography> : null}
                      {str(event.payload_excerpt) ? <Typography variant="caption" sx={{ whiteSpace: "pre-wrap", color: "text.secondary" }}>{str(event.payload_excerpt)}</Typography> : null}
                    </Stack>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
        </TableContainer>
      </Stack>
    </Stack>
  );
}
