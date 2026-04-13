import {
  Alert,
  Box,
  Button,
  Chip,
  MenuItem,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type Dispatch, type SetStateAction, useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";

type JsonRecord = Record<string, unknown>;

type SenderVerificationPanelProps = {
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

function csv(values: string[]): string {
  return values.join(", ");
}

function parseCsv(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function channelLabel(channel: string): string {
  switch (channel) {
    case "slack":
      return "Slack";
    case "teams":
      return "Teams";
    case "whatsapp":
      return "WhatsApp";
    default:
      return channel || "-";
  }
}

function senderDisplay(row: JsonRecord): string {
  return str(row.sender_label, str(row.sender_id, "-"));
}

function scopeDisplay(row: JsonRecord): string {
  return str(row.scope_label, str(row.scope_id, "-"));
}

type ChannelForm = {
  policy: string;
  allowed: string;
};

const EMPTY_CHANNEL_FORM: ChannelForm = {
  policy: "open",
  allowed: ""
};

export function SenderVerificationPanel({ autoRefresh }: SenderVerificationPanelProps) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [slack, setSlack] = useState<ChannelForm>(EMPTY_CHANNEL_FORM);
  const [teams, setTeams] = useState<ChannelForm>(EMPTY_CHANNEL_FORM);
  const [whatsapp, setWhatsapp] = useState<ChannelForm>({ policy: "pairing", allowed: "" });

  const overviewQ = useQuery({
    queryKey: ["settings-sender-verification"],
    queryFn: () => api.rawGet("/sender-verification"),
    refetchInterval: autoRefresh ? 8000 : false
  });

  const saveSettings = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/sender-verification/settings", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-sender-verification"] });
    }
  });
  const approveSender = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/sender-verification/approve", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-sender-verification"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
    }
  });
  const revokeSender = useMutation({
    mutationFn: (payload: JsonRecord) => api.rawPost("/sender-verification/revoke", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings-sender-verification"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications"] });
      await queryClient.invalidateQueries({ queryKey: ["notifications-count"] });
    }
  });

  const payload = asRecord(overviewQ.data);
  const settings = asRecord(payload.settings);
  const slackSettings = asRecord(settings.slack);
  const teamsSettings = asRecord(settings.teams);
  const whatsappSettings = asRecord(settings.whatsapp);
  const pending = useMemo(() => asRecords(payload.pending), [payload]);
  const approved = useMemo(() => asRecords(payload.approved), [payload]);

  useEffect(() => {
    if (dirty) return;
    setSlack({
      policy: str(slackSettings.policy, "open"),
      allowed: csv(toStrings(slackSettings.allowed_senders))
    });
    setTeams({
      policy: str(teamsSettings.policy, "open"),
      allowed: csv(toStrings(teamsSettings.allowed_senders))
    });
    setWhatsapp({
      policy: str(whatsappSettings.policy, "pairing"),
      allowed: csv(toStrings(whatsappSettings.allowed_senders))
    });
  }, [dirty, slackSettings, teamsSettings, whatsappSettings]);

  function updateChannel(
    setter: Dispatch<SetStateAction<ChannelForm>>,
    field: keyof ChannelForm,
    value: string
  ) {
    setDirty(true);
    setter((current) => ({ ...current, [field]: value }));
  }

  async function handleSave() {
    setError(null);
    setSuccess(null);
    try {
      const payload: JsonRecord = {
        slack_policy: slack.policy,
        slack_allowed_senders: parseCsv(slack.allowed),
        teams_policy: teams.policy,
        teams_allowed_senders: parseCsv(teams.allowed)
      };
      if (toBool(whatsappSettings.configured)) {
        payload.whatsapp_policy = whatsapp.policy;
        payload.whatsapp_allowed_senders = parseCsv(whatsapp.allowed);
      }
      await saveSettings.mutateAsync(payload);
      setDirty(false);
      setSuccess("Sender verification settings saved.");
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleApprove(row: JsonRecord) {
    setError(null);
    setSuccess(null);
    try {
      await approveSender.mutateAsync({
        channel: str(row.channel),
        sender_id: str(row.sender_id),
        sender_label: str(row.sender_label) || undefined,
        scope_id: str(row.scope_id) || undefined,
        scope_label: str(row.scope_label) || undefined,
        conversation_id: str(row.conversation_id) || undefined,
        approved_by: "settings_ui"
      });
      setSuccess(`Approved ${senderDisplay(row)}.`);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function handleRevoke(row: JsonRecord) {
    setError(null);
    setSuccess(null);
    try {
      await revokeSender.mutateAsync({
        channel: str(row.channel),
        sender_id: str(row.sender_id),
        scope_id: str(row.scope_id) || undefined
      });
      setSuccess(`Revoked ${senderDisplay(row)}.`);
    } catch (e) {
      setError(errMessage(e));
    }
  }

  const busy =
    saveSettings.isPending || approveSender.isPending || revokeSender.isPending || overviewQ.isFetching;

  return (
    <Stack spacing={2.5}>
      <Alert severity="info">
        Transport signatures confirm Slack, Teams, and WhatsApp webhooks came from the platform. This
        page decides which human senders are trusted to trigger AgentArk work.
      </Alert>

      {error ? <Alert severity="error">{error}</Alert> : null}
      {success ? <Alert severity="success">{success}</Alert> : null}

      <Box className="list-shell">
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1.5}>
          <Box>
            <Typography variant="h6">Sender Trust Policies</Typography>
            <Typography variant="body2" color="text.secondary">
              Use `pairing` when a channel should stop unknown senders until an operator approves them.
            </Typography>
          </Box>
          <Button variant="contained" onClick={handleSave} disabled={busy || overviewQ.isLoading}>
            Save Policies
          </Button>
        </Stack>

        <Stack spacing={1.5}>
          {[
            {
              key: "slack",
              title: "Slack",
              form: slack,
              setForm: setSlack,
              configured: toBool(slackSettings.configured),
              helper: "Always-trusted sender IDs, usually Slack user IDs such as U123ABC."
            },
            {
              key: "teams",
              title: "Teams",
              form: teams,
              setForm: setTeams,
              configured: toBool(teamsSettings.configured),
              helper: "Always-trusted sender IDs, usually Teams IDs or AAD object IDs."
            },
            {
              key: "whatsapp",
              title: "WhatsApp",
              form: whatsapp,
              setForm: setWhatsapp,
              configured: toBool(whatsappSettings.configured),
              helper: "Allowed numbers stay as a hard allowlist; dynamic approvals appear below."
            }
          ].map((item) => (
            <Box key={item.key} className="integration-card">
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1.5}
                justifyContent="space-between"
                alignItems={{ xs: "flex-start", md: "center" }}
                mb={1.5}
              >
                <Box>
                  <Typography variant="subtitle1">{item.title}</Typography>
                  <Typography variant="body2" color="text.secondary">
                    {item.helper}
                  </Typography>
                </Box>
                <Chip
                  size="small"
                  color={item.configured ? "success" : "default"}
                  label={item.configured ? "Configured" : "Not configured"}
                />
              </Stack>
              <Stack direction={{ xs: "column", lg: "row" }} spacing={1.5}>
                <TextField
                  select
                  label="Policy"
                  value={item.form.policy}
                  onChange={(event) => updateChannel(item.setForm, "policy", event.target.value)}
                  sx={{ minWidth: 180 }}
                  disabled={item.key === "whatsapp" && !item.configured}
                >
                  <MenuItem value="open">Open</MenuItem>
                  <MenuItem value="pairing">Pairing</MenuItem>
                </TextField>
                <TextField
                  label="Always-Trusted Sender IDs"
                  value={item.form.allowed}
                  onChange={(event) => updateChannel(item.setForm, "allowed", event.target.value)}
                  multiline
                  minRows={2}
                  fullWidth
                  disabled={item.key === "whatsapp" && !item.configured}
                  helperText={item.helper}
                />
              </Stack>
            </Box>
          ))}
        </Stack>
      </Box>

      <Box className="list-shell">
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Box>
            <Typography variant="h6">Pending Sender Approvals</Typography>
            <Typography variant="body2" color="text.secondary">
              New senders that hit a paired channel stop here until an operator approves them.
            </Typography>
          </Box>
          <Chip size="small" label={`${pending.length} pending`} />
        </Stack>
        {pending.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No pending senders.
          </Typography>
        ) : (
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Channel</TableCell>
                <TableCell>Sender</TableCell>
                <TableCell>Scope</TableCell>
                <TableCell>Seen</TableCell>
                <TableCell>Attempts</TableCell>
                <TableCell>Preview</TableCell>
                <TableCell align="right">Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {pending.map((row) => (
                <TableRow key={str(row.key)}>
                  <TableCell>{channelLabel(str(row.channel))}</TableCell>
                  <TableCell>
                    <Stack spacing={0.2}>
                      <span>{senderDisplay(row)}</span>
                      <Typography variant="caption" color="text.secondary">
                        {str(row.sender_id, "-")}
                      </Typography>
                    </Stack>
                  </TableCell>
                  <TableCell>{scopeDisplay(row)}</TableCell>
                  <TableCell>
                    <Stack spacing={0.2}>
                      <span>{humanTs(str(row.last_seen_at))}</span>
                      <Typography variant="caption" color="text.secondary">
                        First seen {humanTs(str(row.first_seen_at))}
                      </Typography>
                    </Stack>
                  </TableCell>
                  <TableCell>{String(row.occurrences ?? 1)}</TableCell>
                  <TableCell>
                    <Typography variant="body2" sx={{ maxWidth: 320 }}>
                      {str(row.message_preview, "-")}
                    </Typography>
                  </TableCell>
                  <TableCell align="right">
                    <Button size="small" variant="contained" onClick={() => handleApprove(row)} disabled={busy}>
                      Approve
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </Box>

      <Box className="list-shell">
        <Stack direction="row" justifyContent="space-between" alignItems="center" mb={1}>
          <Box>
            <Typography variant="h6">Approved Senders</Typography>
            <Typography variant="body2" color="text.secondary">
              These senders can trigger AgentArk on paired channels until they are revoked.
            </Typography>
          </Box>
          <Chip size="small" label={`${approved.length} approved`} />
        </Stack>
        {approved.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No approved senders yet.
          </Typography>
        ) : (
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Channel</TableCell>
                <TableCell>Sender</TableCell>
                <TableCell>Scope</TableCell>
                <TableCell>Approved</TableCell>
                <TableCell>By</TableCell>
                <TableCell align="right">Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {approved.map((row) => (
                <TableRow key={str(row.key)}>
                  <TableCell>{channelLabel(str(row.channel))}</TableCell>
                  <TableCell>
                    <Stack spacing={0.2}>
                      <span>{senderDisplay(row)}</span>
                      <Typography variant="caption" color="text.secondary">
                        {str(row.sender_id, "-")}
                      </Typography>
                    </Stack>
                  </TableCell>
                  <TableCell>{scopeDisplay(row)}</TableCell>
                  <TableCell>{humanTs(str(row.approved_at))}</TableCell>
                  <TableCell>{str(row.approved_by, "-")}</TableCell>
                  <TableCell align="right">
                    <Button size="small" color="warning" onClick={() => handleRevoke(row)} disabled={busy}>
                      Revoke
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </Box>
    </Stack>
  );
}
