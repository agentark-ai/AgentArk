import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Autocomplete,
  Box,
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogTitle,
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
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { errMessage, str, toBool } from "./pageHelpers";
import { RowOpsMenu } from "./workspaceUiBits";
import {
  OLLAMA_DEFAULT_BASE_URL,
  OPENROUTER_DEFAULT_BASE_URL,
} from "./workspaceCore";
import {
  MODEL_PROVIDER_OPTIONS,
} from "./settingsConstants";

const LOCAL_EMBEDDINGS_MODEL = "BAAI/bge-small-en-v1.5";
const MODEL_ROLE_OPTIONS = ["primary", "fast", "code", "research", "fallback"];

function modelOptionLabel(value: unknown): string {
  const normalized = str(value, "").replace(/[_-]+/g, " ").trim();
  if (!normalized) return "-";
  return normalized.replace(/\b\w/g, (char) => char.toUpperCase());
}

type SettingsModelsPanelProps = {
  [key: string]: any;
};

export function SettingsModelsPanel({
  modelsSectionTab,
  setModelsSectionTab,
  renderSettingsSectionIntro,
  openAddModel,
  form,
  setField,
  modelsQ,
  modelSlots,
  modelsRefreshIssue,
  showingModelFallback,
  toggleModelEnabledMutation,
  setError,
  deleteModelMutation,
  openEditModel,
  embeddingsProvider,
  embeddingsDisabled,
  embeddingsHasApiKey,
  embeddingsStatus,
  embeddingsIsLocal,
  embeddingsIsOllama,
  embeddingsIsExternal,
  hiddenExternalEmbeddingsProvider,
  modelDialogOpen,
  setModelDialogOpen,
  modelForm,
  setModelForm,
  modelEditingId,
  modelCanReuseExistingKey,
  showClearSavedKeyAction,
  modelClearSavedKeyPending,
  setModelClearApiKey,
  modelNeedsReplacementKeyWarning,
  modelAdvancedOpen,
  setModelAdvancedOpen,
  modelConnectionTestResult,
  setModelConnectionTestResult,
  modelTestConnectionHint,
  testModelConnectionMutation,
  canTestModelConnection,
  saveModelMutation,
  setModelConnectivityWarning,
  openaiSubAuth,
  codexAuthBusy,
  startOpenaiSubscriptionOAuth,
  checkOpenaiSubscriptionOAuthStatus,
  discoverModelsQ,
  modelOptions,
  modelOptionNames,
  setSuccess,
}: SettingsModelsPanelProps) {
  return (
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
                                  {modelSlots.map((slot: any) => {
                                    const id = str(slot.id, "");
                                    const enabled = toBool(slot.enabled);
                                    return (
                                      <TableRow key={id}>
                                        <TableCell>
                                          {str(slot.label, "-")}
                                        </TableCell>
                                        <TableCell>
                                          {modelOptionLabel(slot.role)}
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
                            "Choose whether AgentArk uses the bundled local embeddings sidecar or lexical fallback.",
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
                              embeddingsDisabled
                                ? "Disabled"
                                : "Local Hugging Face"
                            }
                          />
                          <Chip
                            size="small"
                            variant="outlined"
                            label={
                              embeddingsDisabled
                                ? "Lexical fallback"
                                : LOCAL_EMBEDDINGS_MODEL
                            }
                          />
                        </Stack>

                        {hiddenExternalEmbeddingsProvider ? (
                          <Alert severity="warning">
                            An external embeddings provider is currently saved.
                            Settings now exposes only Local or Disabled until
                            provider-aware reindexing is built. Save this page
                            to replace it with the selected mode.
                          </Alert>
                        ) : null}

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
                              value={
                                form.embeddings_provider === "disabled"
                                  ? "disabled"
                                  : "local-hf"
                              }
                              onChange={(e) =>
                                setField("embeddings_provider", e.target.value)
                              }
                              fullWidth
                              size="small"
                            >
                              <MenuItem value="local-hf">
                                Local Hugging Face
                              </MenuItem>
                              <MenuItem value="disabled">
                                Disabled
                              </MenuItem>
                            </TextField>
                          </Grid2>
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
                          setModelForm((p: any) => ({ ...p, label: e.target.value }))
                        }
                        fullWidth
                      />
                      <TextField
                        label="Role"
                        select
                        value={modelForm.role}
                        onChange={(e) =>
                          setModelForm((p: any) => ({ ...p, role: e.target.value }))
                        }
                        fullWidth
                      >
                        {MODEL_ROLE_OPTIONS.map((role) => (
                          <MenuItem key={role} value={role}>
                            {modelOptionLabel(role)}
                          </MenuItem>
                        ))}
                      </TextField>
                      <TextField
                        label="Provider"
                        select
                        value={modelForm.provider}
                        onChange={(e) =>
                          setModelForm((p: any) => ({
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
                          setModelForm((p: any) => ({
                            ...p,
                            model: String(v ?? ""),
                          }))
                        }
                        inputValue={modelForm.model}
                        onInputChange={(_, v) =>
                          setModelForm((p: any) => ({ ...p, model: v }))
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
                              setModelForm((p: any) => ({
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
                                  setModelForm((p: any) => ({ ...p, api_key: "" }));
                                  setModelClearApiKey((prev: boolean) => !prev);
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
                                setModelForm((p: any) => ({
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
                              setModelForm((p: any) => ({
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
  );
}
