import { Alert, Box, Button, Chip, Dialog, DialogActions, DialogContent, DialogTitle, Divider, Stack, TextField, Typography } from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import type { ExtensionPackView } from "../types";

type ExtensionPackMode = "all" | "integrations" | "messaging" | "connectors" | "channels";

function packKindFilter(mode: ExtensionPackMode): string | undefined {
  if (mode === "messaging" || mode === "channels") return "messaging_channel";
  if (mode === "integrations" || mode === "connectors") return "integration";
  return undefined;
}

function defaultSecretTemplate(pack: ExtensionPackView): string {
  if (pack.manifest.id === "slack_channel") {
    return JSON.stringify({ bot_token: "", default_channel_id: "", signing_secret: "" }, null, 2);
  }
  if (pack.manifest.id === "teams_channel") {
    return JSON.stringify(
      { service_url: "", access_token: "", bot_app_id: "", team_id: "", channel_id: "" },
      null,
      2
    );
  }
  if (pack.manifest.id === "whatsapp_channel") {
    return JSON.stringify(
      { mode: "cloud_api", access_token: "", phone_number_id: "", allowed_numbers: [""] },
      null,
      2
    );
  }
  const requiredSecrets = pack.manifest.auth.required_secrets || [];
  if (requiredSecrets.length > 0) {
    const payload = Object.fromEntries(
      requiredSecrets.map((key) => [key, key === "allowed_numbers" ? [""] : ""])
    );
    return JSON.stringify(payload, null, 2);
  }
  const mode = pack.manifest.auth.mode;
  if (mode === "basic") {
    return JSON.stringify({ username: "", password: "" }, null, 2);
  }
  if (mode === "api_key") {
    return JSON.stringify({ api_key: "" }, null, 2);
  }
  return "{}";
}

function cardAccent(status: string): { border: string; bg: string; chip: string } {
  if (status === "connected") {
    return { border: "rgba(76, 175, 80, 0.35)", bg: "rgba(76, 175, 80, 0.08)", chip: "#81c784" };
  }
  if (status === "needs_auth") {
    return { border: "rgba(255, 193, 7, 0.35)", bg: "rgba(255, 193, 7, 0.08)", chip: "#ffd54f" };
  }
  if (status === "draft") {
    return { border: "rgba(105, 226, 255, 0.3)", bg: "rgba(105, 226, 255, 0.08)", chip: "#69e2ff" };
  }
  if (status === "disabled") {
    return { border: "rgba(158, 158, 158, 0.35)", bg: "rgba(158, 158, 158, 0.08)", chip: "#b0bec5" };
  }
  if (status === "error") {
    return { border: "rgba(244, 67, 54, 0.35)", bg: "rgba(244, 67, 54, 0.08)", chip: "#ef9a9a" };
  }
  return { border: "rgba(105, 226, 255, 0.22)", bg: "rgba(255,255,255,0.02)", chip: "#90caf9" };
}

export function ExtensionPacksPanel({ mode = "all" }: { mode?: ExtensionPackMode }) {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [uploadDialogOpen, setUploadDialogOpen] = useState(false);
  const [scaffoldDialogOpen, setScaffoldDialogOpen] = useState(false);
  const [connectPack, setConnectPack] = useState<ExtensionPackView | null>(null);
  const [eventsPack, setEventsPack] = useState<ExtensionPackView | null>(null);
  const [linkUrl, setLinkUrl] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [uploadFile, setUploadFile] = useState<File | null>(null);
  const [scaffoldName, setScaffoldName] = useState("");
  const [scaffoldKind, setScaffoldKind] = useState(mode === "messaging" || mode === "channels" ? "messaging_channel" : "integration");
  const [scaffoldFeatures, setScaffoldFeatures] = useState("");
  const [scaffoldDocsUrl, setScaffoldDocsUrl] = useState("");
  const [scaffoldOpenapiUrl, setScaffoldOpenapiUrl] = useState("");
  const [scaffoldOpenapiText, setScaffoldOpenapiText] = useState("");
  const [scaffoldCurlText, setScaffoldCurlText] = useState("");
  const [connectionName, setConnectionName] = useState("Default connection");
  const [connectionSecretJson, setConnectionSecretJson] = useState("{}");
  const [connectError, setConnectError] = useState<string | null>(null);
  const [selectedConnectionId, setSelectedConnectionId] = useState<string | null>(null);

  const kind = packKindFilter(mode);
  const packsQ = useQuery({
    queryKey: ["extension-packs", kind || "all", search],
    queryFn: () =>
      api.getExtensionPacks({
        query: search.trim() || undefined,
        kind
      })
  });

  const installed = packsQ.data?.installed || [];
  const catalog = packsQ.data?.catalog || [];
  const emptyStateVisible =
    !packsQ.isLoading && !packsQ.isFetching && installed.length === 0 && catalog.length === 0;
  const connectDetailQ = useQuery({
    queryKey: ["extension-pack-detail", connectPack?.manifest.id],
    enabled: !!connectPack,
    queryFn: () => api.getExtensionPack(connectPack!.manifest.id)
  });
  const eventsQ = useQuery({
    queryKey: ["extension-pack-events", eventsPack?.manifest.id],
    enabled: !!eventsPack,
    queryFn: () => api.getExtensionPackEvents(eventsPack!.manifest.id, 25)
  });

  const installMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.installExtensionPack(payload),
    onSuccess: async (payload) => {
      setNotice({ kind: "success", text: `${payload.pack.manifest.name} installed.` });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });
  const uploadMutation = useMutation({
    mutationFn: (formData: FormData) => api.uploadExtensionPack(formData),
    onSuccess: async (payload) => {
      setNotice({ kind: "success", text: `${payload.pack.manifest.name} uploaded and installed.` });
      setUploadDialogOpen(false);
      setUploadFile(null);
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const scaffoldMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.scaffoldExtensionPack(payload),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: `${payload.pack.manifest.name} scaffolded as an unverified draft pack.`
      });
      setScaffoldDialogOpen(false);
      setScaffoldName("");
      setScaffoldFeatures("");
      setScaffoldDocsUrl("");
      setScaffoldOpenapiUrl("");
      setScaffoldOpenapiText("");
      setScaffoldCurlText("");
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const connectionMutation = useMutation({
    mutationFn: (payload: { packId: string; body: Record<string, unknown> }) =>
      api.upsertExtensionPackConnection(payload.packId, payload.body),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Connection saved." });
      setConnectPack(null);
      setConnectError(null);
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setConnectError(error.message)
  });

  const enableMutation = useMutation({
    mutationFn: (payload: { packId: string; enabled: boolean }) =>
      api.setExtensionPackEnabled(payload.packId, payload.enabled),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: `${payload.pack.manifest.name} ${payload.pack.enabled ? "enabled" : "disabled"}.`
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  useEffect(() => {
    if (!connectPack) return;
    const first = connectDetailQ.data?.connections?.[0];
    if (!first) return;
    if (!selectedConnectionId) {
      setSelectedConnectionId(first.connection.id);
      setConnectionName(first.connection.name || "Default connection");
    }
  }, [connectDetailQ.data, connectPack, selectedConnectionId]);

  const deleteMutation = useMutation({
    mutationFn: (packId: string) => api.deleteExtensionPack(packId, { remove_connections: true }),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Pack removed." });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const sectionTitle = useMemo(() => {
    if (mode === "messaging" || mode === "channels") return "Generic Channel Packs";
    if (mode === "integrations" || mode === "connectors") return "Custom Integrations";
    return "Generic Packs";
  }, [mode]);

  const sectionSubtitle = useMemo(() => {
    if (mode === "messaging" || mode === "channels") {
      return "Search installed packs, bundled defaults, upload a bundle, or scaffold a new messaging channel pack.";
    }
    if (mode === "integrations" || mode === "connectors") {
      return "Search installed integrations, bundled defaults, upload a bundle, or scaffold/import a custom integration from OpenAPI or curl.";
    }
    return "Search installed packs, bundled defaults, upload a bundle, or scaffold from OpenAPI/cURL when nothing exists yet.";
  }, [mode]);

  const emptyStateMessage = useMemo(() => {
    if (mode === "messaging" || mode === "channels") {
      return "No channel pack matched this search. Ask for a link or local path, upload a manifest/bundle, or scaffold a draft channel pack.";
    }
    if (mode === "integrations" || mode === "connectors") {
      return "No integration matched this search. Ask for a link or local path, upload a manifest/bundle, or scaffold a draft integration from docs/OpenAPI/cURL.";
    }
    return "No pack matched this search. Ask for a link or local path, upload a manifest/bundle, or scaffold a draft pack from docs/OpenAPI/cURL.";
  }, [mode]);

  async function openOauthConnect(pack: ExtensionPackView) {
    try {
      const payload = await api.getExtensionPackConnectUrl(pack.manifest.id);
      window.open(payload.url, "_blank", "noopener,noreferrer");
      setNotice({
        kind: "success",
        text: `Opened ${pack.manifest.name} sign-in in a new tab.`
      });
      void queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    } catch (error) {
      setNotice({
        kind: "error",
        text: error instanceof Error ? error.message : "Failed to open the connect URL."
      });
    }
  }

  async function testPack(pack: ExtensionPackView) {
    try {
      const detail = await api.getExtensionPack(pack.manifest.id);
      const connectionId =
        detail.connections.find((item) => item.state === "ready")?.connection.id ||
        detail.connections[0]?.connection.id;
      if (!connectionId) throw new Error("No saved connection was found for this pack.");
      const result = await api.testExtensionPackConnection(pack.manifest.id, connectionId);
      const status = String(result.result.status || "ok");
      setNotice({
        kind: status === "ok" ? "success" : "error",
        text:
          String(result.result.message || "") ||
          `${pack.manifest.name} test finished with status ${status}.`
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    } catch (error) {
      setNotice({
        kind: "error",
        text: error instanceof Error ? error.message : "Pack test failed."
      });
    }
  }

  function openConnectDialog(pack: ExtensionPackView) {
    setConnectPack(pack);
    setSelectedConnectionId(null);
    setConnectionName("Default connection");
    setConnectionSecretJson(defaultSecretTemplate(pack));
    setConnectError(null);
  }

  function renderPackCard(pack: ExtensionPackView, installedPack: boolean) {
    const accent = cardAccent(pack.status);
    return (
      <Box
        key={`${installedPack ? "installed" : "catalog"}-${pack.manifest.id}`}
        sx={{
          p: 1.4,
          borderRadius: "8px",
          border: `1px solid ${accent.border}`,
          background: accent.bg
        }}
      >
        <Stack spacing={1}>
          <Stack
            direction="row"
            spacing={1}
            sx={{
              alignItems: "center",
              justifyContent: "space-between"
            }}>
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                {pack.manifest.name}
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {pack.manifest.kind.replace(/_/g, " ")}
              </Typography>
            </Box>
            <Chip
              size="small"
              variant="outlined"
              label={pack.status.replace(/_/g, " ")}
              sx={{ color: accent.chip, borderColor: accent.chip }}
            />
          </Stack>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              lineHeight: 1.5
            }}>
            {pack.manifest.description || "No description provided."}
          </Typography>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Chip size="small" label={`${pack.feature_summaries.length} features`} variant="outlined" />
            <Chip size="small" label={pack.trust_level.replace(/_/g, " ")} variant="outlined" />
            <Chip size="small" label={pack.verification_status.replace(/_/g, " ")} variant="outlined" />
            <Chip size="small" label={pack.source_kind.replace(/_/g, " ")} variant="outlined" />
            {pack.manifest.draft ? <Chip size="small" label="draft" variant="outlined" /> : null}
          </Stack>
          {pack.verification_detail ? (
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {pack.verification_detail}
            </Typography>
          ) : null}
          {pack.status_detail ? (
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {pack.status_detail}
            </Typography>
          ) : null}
          {installedPack && pack.supports_webhook && pack.webhook_path ? (
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                wordBreak: "break-all"
              }}>
              Webhook: {`${window.location.origin}${pack.webhook_path}`}
            </Typography>
          ) : null}
          <Stack direction="row" spacing={1} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            {!installedPack ? (
              <Button
                size="small"
                variant="contained"
                onClick={() =>
                  installMutation.mutate({
                    pack_id: pack.manifest.id
                  })
                }
                disabled={installMutation.isPending}
              >
                Install
              </Button>
            ) : (
              <>
                {pack.needs_auth || pack.supports_connect_url ? (
                  <Button
                    size="small"
                    variant="contained"
                    onClick={() =>
                      pack.supports_connect_url ? void openOauthConnect(pack) : openConnectDialog(pack)
                    }
                  >
                    Connect
                  </Button>
                ) : null}
                <Button size="small" variant="outlined" onClick={() => void testPack(pack)}>
                  Test
                </Button>
                {pack.supports_webhook ? (
                  <Button size="small" variant="outlined" onClick={() => setEventsPack(pack)}>
                    Events
                  </Button>
                ) : null}
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() =>
                    enableMutation.mutate({
                      packId: pack.manifest.id,
                      enabled: !pack.enabled
                    })
                  }
                  disabled={enableMutation.isPending}
                >
                  {pack.enabled ? "Disable" : "Enable"}
                </Button>
                <Button
                  size="small"
                  color="error"
                  variant="outlined"
                  onClick={() => deleteMutation.mutate(pack.manifest.id)}
                  disabled={deleteMutation.isPending}
                >
                  Remove
                </Button>
              </>
            )}
          </Stack>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            {pack.feature_summaries.slice(0, 4).map((feature) => (
              <Chip
                key={`${pack.manifest.id}-${feature.id}`}
                size="small"
                label={feature.id}
                variant="outlined"
              />
            ))}
          </Stack>
        </Stack>
      </Box>
    );
  }

  return (
    <Stack spacing={1.5}>
      <Box className="list-shell" sx={{ p: 1.5 }}>
        <Stack spacing={1.2}>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "stretch", sm: "center" }
            }}>
            <Box>
              <Typography variant="subtitle2">{sectionTitle}</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {sectionSubtitle}
              </Typography>
            </Box>
            <Stack direction="row" spacing={1}>
              <Button size="small" variant="outlined" onClick={() => setLinkDialogOpen(true)}>
                Link or path
              </Button>
              <Button size="small" variant="outlined" onClick={() => setUploadDialogOpen(true)}>
                Upload bundle
              </Button>
              <Button size="small" variant="outlined" onClick={() => setScaffoldDialogOpen(true)}>
                Scaffold draft
              </Button>
            </Stack>
          </Stack>
          <TextField
            size="small"
            label="Search packs"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="microsoft, slack, notion, clickup..."
          />
          {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
          {packsQ.error ? (
            <Alert severity="error">
              {packsQ.error instanceof Error ? packsQ.error.message : "Failed to load extension packs."}
            </Alert>
          ) : null}
          {emptyStateVisible ? (
            <Alert severity="info">
              {emptyStateMessage}
            </Alert>
          ) : null}
          {packsQ.data?.not_found ? (
            <Stack spacing={0.75}>
              {packsQ.data.next_steps.map((step) => (
                <Typography key={step} variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  {step}
                </Typography>
              ))}
            </Stack>
          ) : null}
          {installed.length > 0 ? (
            <Stack spacing={1}>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Installed
              </Typography>
              <Stack spacing={1}>{installed.map((pack) => renderPackCard(pack, true))}</Stack>
            </Stack>
          ) : null}
          {catalog.length > 0 ? (
            <Stack spacing={1}>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Catalog
              </Typography>
              <Stack spacing={1}>{catalog.map((pack) => renderPackCard(pack, false))}</Stack>
            </Stack>
          ) : null}
        </Stack>
      </Box>
      <Dialog open={linkDialogOpen} onClose={() => setLinkDialogOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Install pack from link or local path</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Use this when you already have a manifest URL, raw manifest path, or local bundle path. Non-bundled sources install as unverified packs unless publisher verification succeeds.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Manifest URL"
              value={linkUrl}
              onChange={(event) => setLinkUrl(event.target.value)}
              placeholder="https://example.com/pack.json"
            />
            <TextField
              fullWidth
              size="small"
              label="Local manifest or bundle path"
              value={sourcePath}
              onChange={(event) => setSourcePath(event.target.value)}
              placeholder="C:\\packs\\clickup-pack.zip"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setLinkDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() =>
              installMutation.mutate(
                {
                  source_url: linkUrl.trim() || undefined,
                  source_path: sourcePath.trim() || undefined,
                  trust_unverified: true
                },
                {
                  onSuccess: () => {
                    setLinkDialogOpen(false);
                    setLinkUrl("");
                    setSourcePath("");
                  }
                }
              )
            }
            disabled={installMutation.isPending || (!linkUrl.trim() && !sourcePath.trim())}
          >
            Install
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={uploadDialogOpen} onClose={() => setUploadDialogOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Upload manifest or bundle</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Upload a manifest JSON/YAML file or a zip bundle containing one of the expected manifest names.
            </Typography>
            <Button variant="outlined" component="label">
              {uploadFile ? uploadFile.name : "Choose file"}
              <input
                hidden
                type="file"
                accept=".json,.yaml,.yml,.zip"
                onChange={(event) => setUploadFile(event.target.files?.[0] ?? null)}
              />
            </Button>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setUploadDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            disabled={uploadMutation.isPending || !uploadFile}
            onClick={() => {
              if (!uploadFile) return;
              const formData = new FormData();
              formData.append("file", uploadFile);
              formData.append("trust_unverified", "true");
              uploadMutation.mutate(formData);
            }}
          >
            Upload
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={scaffoldDialogOpen}
        onClose={() => setScaffoldDialogOpen(false)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Scaffold draft pack</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Draft packs are local and unverified by default. Start read-only when possible, then replace placeholder bindings after review.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Service name"
              value={scaffoldName}
              onChange={(event) => setScaffoldName(event.target.value)}
              placeholder="ClickUp"
            />
            <TextField
              fullWidth
              size="small"
              label="Pack kind"
              value={scaffoldKind}
              onChange={(event) => setScaffoldKind(event.target.value)}
              placeholder="integration"
            />
            <TextField
              fullWidth
              size="small"
              label="Desired features"
              value={scaffoldFeatures}
              onChange={(event) => setScaffoldFeatures(event.target.value)}
              placeholder="tasks.list, tasks.get, tasks.update"
              helperText="Comma-separated canonical or experimental feature IDs."
            />
            <TextField
              fullWidth
              size="small"
              label="Docs URL"
              value={scaffoldDocsUrl}
              onChange={(event) => setScaffoldDocsUrl(event.target.value)}
            />
            <Divider flexItem />
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Optional import source. If you provide an OpenAPI URL, OpenAPI text, or a sample curl command, the draft pack will be generated with executable HTTP bindings instead of placeholder bindings.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="OpenAPI URL"
              value={scaffoldOpenapiUrl}
              onChange={(event) => setScaffoldOpenapiUrl(event.target.value)}
              placeholder="https://api.example.com/openapi.json"
            />
            <TextField
              fullWidth
              multiline
              minRows={4}
              size="small"
              label="OpenAPI text"
              value={scaffoldOpenapiText}
              onChange={(event) => setScaffoldOpenapiText(event.target.value)}
              placeholder='{"openapi":"3.0.0", ...}'
            />
            <TextField
              fullWidth
              multiline
              minRows={3}
              size="small"
              label="Sample curl command"
              value={scaffoldCurlText}
              onChange={(event) => setScaffoldCurlText(event.target.value)}
              placeholder="curl https://api.example.com/v1/items -H 'Authorization: Bearer ...'"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setScaffoldDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() =>
              scaffoldMutation.mutate({
                name: scaffoldName.trim(),
                kind: scaffoldKind.trim(),
                docs_url: scaffoldDocsUrl.trim() || undefined,
                openapi_url: scaffoldOpenapiUrl.trim() || undefined,
                openapi_text: scaffoldOpenapiText.trim() || undefined,
                curl_text: scaffoldCurlText.trim() || undefined,
                desired_features: scaffoldFeatures
                  .split(",")
                  .map((value) => value.trim())
                  .filter(Boolean),
                binding_kind: "unsupported"
              })
            }
            disabled={scaffoldMutation.isPending || !scaffoldName.trim()}
          >
            Create draft
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={!!connectPack} onClose={() => setConnectPack(null)} maxWidth="sm" fullWidth>
        <DialogTitle>{connectPack ? `Connect ${connectPack.manifest.name}` : "Connect pack"}</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            {connectError ? <Alert severity="error">{connectError}</Alert> : null}
            {connectDetailQ.data?.connections?.length ? (
              <Stack spacing={1}>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Existing connections
                </Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  {connectDetailQ.data.connections.map((item) => (
                    <Button
                      key={item.connection.id}
                      size="small"
                      variant={selectedConnectionId === item.connection.id ? "contained" : "outlined"}
                      onClick={() => {
                        setSelectedConnectionId(item.connection.id);
                        setConnectionName(item.connection.name || "Default connection");
                      }}
                    >
                      {item.connection.name || item.connection.id}
                    </Button>
                  ))}
                  <Button
                    size="small"
                    variant={selectedConnectionId ? "outlined" : "contained"}
                    onClick={() => {
                      setSelectedConnectionId(null);
                      setConnectionName("Default connection");
                      setConnectionSecretJson(defaultSecretTemplate(connectPack!));
                    }}
                  >
                    New connection
                  </Button>
                </Stack>
              </Stack>
            ) : null}
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {
                "Save a connection secret as JSON. Examples: {\"api_key\":\"...\"} for API-key packs or {\"username\":\"...\",\"password\":\"...\"} for basic-auth packs."
              }
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Connection name"
              value={connectionName}
              onChange={(event) => setConnectionName(event.target.value)}
            />
            <TextField
              fullWidth
              multiline
              minRows={6}
              size="small"
              label="Secret JSON"
              value={connectionSecretJson}
              onChange={(event) => setConnectionSecretJson(event.target.value)}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConnectPack(null)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => {
              if (!connectPack) return;
              try {
                const parsedSecret = JSON.parse(connectionSecretJson);
                connectionMutation.mutate({
                  packId: connectPack.manifest.id,
                  body: {
                    connection_id: selectedConnectionId || undefined,
                    name: connectionName.trim() || "Default connection",
                    secret: parsedSecret
                  }
                });
              } catch {
                setConnectError("Secret JSON is invalid.");
              }
            }}
            disabled={connectionMutation.isPending}
          >
            Save connection
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={!!eventsPack} onClose={() => setEventsPack(null)} maxWidth="md" fullWidth>
        <DialogTitle>{eventsPack ? `${eventsPack.manifest.name} inbound events` : "Inbound events"}</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.25}>
            {eventsPack?.webhook_path ? (
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                  wordBreak: "break-all"
                }}>
                {`${window.location.origin}${eventsPack.webhook_path}`}
              </Typography>
            ) : null}
            {eventsQ.error ? (
              <Alert severity="error">
                {eventsQ.error instanceof Error ? eventsQ.error.message : "Failed to load pack events."}
              </Alert>
            ) : null}
            {!eventsQ.data?.items?.length ? (
              <Alert severity="info">No inbound events recorded for this pack yet.</Alert>
            ) : null}
            {eventsQ.data?.items?.map((event) => (
              <Box
                key={event.id}
                sx={{
                  p: 1.2,
                  borderRadius: "8px",
                  border: "1px solid rgba(255,255,255,0.08)",
                  background: "rgba(255,255,255,0.02)"
                }}
              >
                <Stack spacing={0.75}>
                  <Stack direction="row" spacing={0.75} useFlexGap sx={{
                    flexWrap: "wrap"
                  }}>
                    <Chip size="small" label={event.event_type} variant="outlined" />
                    <Chip size="small" label={event.status} variant="outlined" />
                    <Chip size="small" label={event.transport} variant="outlined" />
                  </Stack>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    {new Date(event.received_at).toLocaleString()}
                  </Typography>
                  {event.outcome ? (
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {event.outcome}
                    </Typography>
                  ) : null}
                  {event.response_preview ? (
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {event.response_preview}
                    </Typography>
                  ) : null}
                </Stack>
              </Box>
            ))}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setEventsPack(null)}>Close</Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}
