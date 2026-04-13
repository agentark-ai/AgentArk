import { Alert, Box, Button, Stack, TextField, Typography } from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../api/client";
type GoogleWorkspaceOAuthClientSettings = Record<string, unknown>;

function asErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message.trim()) return err.message;
  return "Request failed";
}

export function GoogleWorkspaceAdminPanel() {
  const queryClient = useQueryClient();
  const [credentialsJson, setCredentialsJson] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);

  const settingsQ = useQuery({
    queryKey: ["google-workspace-oauth-client-settings"],
    queryFn: () => api.rawGet("/integrations/google-workspace/oauth-client") as Promise<GoogleWorkspaceOAuthClientSettings>
  });

  const saveMutation = useMutation({
    mutationFn: async (payload: Record<string, unknown>) =>
      api.rawPost("/integrations/google-workspace/oauth-client", payload),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Google OAuth client saved." });
      setCredentialsJson("");
      setClientId("");
      setClientSecret("");
      await Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: ["google-workspace-oauth-client-settings"] }),
        queryClient.invalidateQueries({ queryKey: ["integrations"] })
      ]);
    },
    onError: (error) => {
      setNotice({ kind: "error", text: asErrorMessage(error) });
    }
  });

  const settings = settingsQ.data as GoogleWorkspaceOAuthClientSettings | undefined;
  const usingEnv = !!settings?.managed_externally;
  const hasSavedClient = settings?.source === "settings";
  const canSave = useMemo(() => {
    return !!credentialsJson.trim() || (!!clientId.trim() && !!clientSecret.trim());
  }, [clientId, clientSecret, credentialsJson]);

  const submit = async () => {
    setNotice(null);
    if (!canSave) {
      setNotice({
        kind: "error",
        text: "Paste the Google OAuth client JSON, or enter both client ID and client secret."
      });
      return;
    }
    await saveMutation.mutateAsync({
      credentials_json: credentialsJson.trim() || undefined,
      client_id: clientId.trim() || undefined,
      client_secret: clientSecret.trim() || undefined
    });
  };

  const clearSaved = async () => {
    setNotice(null);
    await saveMutation.mutateAsync({ clear: true });
  };

  return (
    <Stack spacing={2}>
      <Alert severity="info">
        Save one shared Google OAuth client here once. After that, Google Workspace sign-in from Integrations can go
        straight to Google without asking normal users for a credentials file.
      </Alert>
      {usingEnv ? (
        <Alert severity="warning">
          Google OAuth is currently coming from environment variables. Saved values here are kept encrypted, but the
          running instance will continue using the environment override until those variables are removed.
        </Alert>
      ) : null}
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {settingsQ.error ? <Alert severity="error">{asErrorMessage(settingsQ.error)}</Alert> : null}
      <Box className="list-shell" sx={{ minHeight: 0 }}>
        <Stack spacing={1.5}>
          <Box>
            <Typography variant="h6">Google OAuth</Typography>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Redirect URI: {String(settings?.redirect_uri || "http://localhost:8990/oauth/callback")}
            </Typography>
          </Box>
          <Stack direction={{ xs: "column", sm: "row" }} spacing={1} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Typography variant="body2">
              Status:{" "}
              <strong>{settings?.configured ? "Configured" : "Not configured"}</strong>
            </Typography>
            <Typography variant="body2" sx={{
              color: "text.secondary"
            }}>
              Source: {String(settings?.source_label || "Not configured")}
            </Typography>
            {settings?.client_id_hint ? (
              <Typography variant="body2" sx={{
                color: "text.secondary"
              }}>
                Client ID: {String(settings.client_id_hint)}
              </Typography>
            ) : null}
          </Stack>

          <TextField
            fullWidth
            multiline
            minRows={6}
            label="Google OAuth Client JSON"
            placeholder="Paste the OAuth client JSON from Google Cloud"
            value={credentialsJson}
            onChange={(event) => setCredentialsJson(event.target.value)}
            size="small"
          />
          <Stack direction="row" spacing={1} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Button component="label" variant="outlined" size="small">
              Upload Credentials JSON
              <input
                hidden
                type="file"
                accept="application/json,.json"
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (!file) return;
                  const reader = new FileReader();
                  reader.onload = () => {
                    const text = typeof reader.result === "string" ? reader.result : "";
                    setCredentialsJson(text);
                  };
                  reader.readAsText(file);
                  event.currentTarget.value = "";
                }}
              />
            </Button>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                alignSelf: "center"
              }}>
              Desktop or web OAuth client JSON from Google Cloud both work.
            </Typography>
          </Stack>

          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            Or enter the values directly if you do not want to paste the full JSON.
          </Typography>
          <TextField
            fullWidth
            size="small"
            label="Client ID"
            value={clientId}
            onChange={(event) => setClientId(event.target.value)}
            placeholder="1234567890-xxxxx.apps.googleusercontent.com"
          />
          <TextField
            fullWidth
            size="small"
            label="Client Secret"
            type="password"
            value={clientSecret}
            onChange={(event) => setClientSecret(event.target.value)}
            placeholder={settings?.secret_configured ? "Leave blank unless replacing it" : "GOCSPX-..."}
          />

          <Stack direction="row" spacing={1} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Button
              variant="contained"
              size="small"
              onClick={() => void submit()}
              disabled={saveMutation.isPending || !canSave}
            >
              {saveMutation.isPending ? "Saving..." : "Save Google OAuth"}
            </Button>
            <Button
              variant="outlined"
              color="warning"
              size="small"
              onClick={() => void clearSaved()}
              disabled={saveMutation.isPending || usingEnv || !hasSavedClient}
            >
              Clear saved client
            </Button>
          </Stack>
        </Stack>
      </Box>
    </Stack>
  );
}
