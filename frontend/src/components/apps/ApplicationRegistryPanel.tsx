import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  Link,
  Stack,
  TextField,
  Tooltip,
  Typography
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../api/client";

const REFRESH_MS = 8000;

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  const obj = asRecord(value);
  const raw = obj[key];
  if (!Array.isArray(raw)) return [];
  return raw.filter((item): item is JsonRecord => !!item && typeof item === "object" && !Array.isArray(item));
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function bool(value: unknown): boolean {
  return value === true;
}

function errMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Request failed.";
}

async function copyText(value: string): Promise<void> {
  if (!value.trim()) return;
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  throw new Error("Clipboard is not available in this browser session.");
}

function stateDotColor(state: string): string {
  const normalized = state.trim().toLowerCase();
  if (normalized === "running") return "var(--ui-rgba-74-210-157-850)";
  if (normalized === "failed") return "var(--ui-rgba-255-100-100-850)";
  if (normalized === "stopped" || normalized === "completed") return "var(--ui-rgba-255-191-130-850)";
  return "var(--ui-rgba-180-200-220-500)";
}

export function ApplicationRegistryPanel({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const registryQ = useQuery({
    queryKey: ["application-launchers"],
    queryFn: () => api.rawGet("/api/applications"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const [modelById, setModelById] = useState<Record<string, string>>({});
  const [bannerError, setBannerError] = useState<string | null>(null);
  const [bannerSuccess, setBannerSuccess] = useState<string | null>(null);

  const launchMutation = useMutation({
    mutationFn: ({ id, mode, model }: { id: string; mode: "launch" | "config"; model?: string }) =>
      api.rawPost(`/api/applications/${encodeURIComponent(id)}/launch`, {
        mode,
        model: model?.trim() || undefined
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["application-launchers"] });
    }
  });

  const stopMutation = useMutation({
    mutationFn: ({ id }: { id: string }) => api.rawPost(`/api/applications/${encodeURIComponent(id)}/stop`, {}),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["application-launchers"] });
    }
  });

  const applications = pickRecords(registryQ.data, "applications");
  const runtime = asRecord(asRecord(registryQ.data).runtime);
  const ollamaReady = bool(runtime.ollama_cli_available) && bool(runtime.ollama_reachable);
  const runtimeDetail = str(runtime.detail, "");
  const runtimeVersion = str(runtime.ollama_version, "");
  const runtimeBaseUrl = str(runtime.ollama_base_url, "");
  const runtimeSource = str(runtime.ollama_base_url_source, "");
  const runtimeInDocker = bool(runtime.docker_runtime);

  useEffect(() => {
    if (applications.length === 0) return;
    setModelById((prev) => {
      const next = { ...prev };
      let changed = false;
      for (const app of applications) {
        const id = str(app.id, "");
        if (!id || next[id] != null) continue;
        const models = Array.isArray(app.recommended_models)
          ? app.recommended_models.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
          : [];
        next[id] = models[0] || "";
        changed = true;
      }
      return changed ? next : prev;
    });
  }, [applications]);

  useEffect(() => {
    if (!bannerSuccess) return;
    const timer = window.setTimeout(() => setBannerSuccess(null), 3500);
    return () => window.clearTimeout(timer);
  }, [bannerSuccess]);

  const sortedApplications = useMemo(
    () =>
      [...applications].sort((a, b) => {
        const aState = str(asRecord(a.runtime).state, "idle");
        const bState = str(asRecord(b.runtime).state, "idle");
        if (aState === "running" && bState !== "running") return -1;
        if (bState === "running" && aState !== "running") return 1;
        return str(a.label).localeCompare(str(b.label));
      }),
    [applications]
  );

  return (
    <Box className="list-shell">
      <Stack spacing={1.25}>
        <Box>
          <Typography variant="h6">External Launchers</Typography>
          <Typography variant="caption" sx={{
            color: "text.secondary"
          }}>
            AgentArk-managed launchers for optional external terminal tools through Ollama Launch. These are companion tools, not AgentArk modes.
          </Typography>
        </Box>

        {registryQ.error ? <Alert severity="error">{errMessage(registryQ.error)}</Alert> : null}
        {bannerError ? <Alert severity="error">{bannerError}</Alert> : null}
        {bannerSuccess ? <Alert severity="success">{bannerSuccess}</Alert> : null}

        <Alert severity={ollamaReady ? "success" : "warning"}>
          <Stack spacing={0.5}>
            <Typography variant="body2" sx={{
              fontWeight: 700
            }}>
              {ollamaReady ? "Ollama launch runtime is ready." : "Ollama launch runtime needs attention."}
            </Typography>
            <Typography variant="body2">{runtimeDetail || "No Ollama runtime details available yet."}</Typography>
            <Stack direction="row" spacing={1} useFlexGap sx={{
              flexWrap: "wrap"
            }}>
              <Chip size="small" label={bool(runtime.ollama_cli_available) ? "CLI installed" : "CLI missing"} color={bool(runtime.ollama_cli_available) ? "success" : "warning"} />
              <Chip size="small" label={bool(runtime.ollama_reachable) ? "Runtime reachable" : "Runtime unreachable"} color={bool(runtime.ollama_reachable) ? "success" : "warning"} />
              {runtimeVersion ? <Chip size="small" label={runtimeVersion} variant="outlined" /> : null}
              {runtimeBaseUrl ? <Chip size="small" label={`Host: ${runtimeBaseUrl}`} variant="outlined" /> : null}
              {runtimeSource ? <Chip size="small" label={`Source: ${runtimeSource}`} variant="outlined" /> : null}
              {runtimeInDocker ? <Chip size="small" label="Docker runtime" variant="outlined" /> : null}
            </Stack>
          </Stack>
        </Alert>

        {sortedApplications.length === 0 ? (
          <Typography variant="body2" sx={{
            color: "text.secondary"
          }}>
            No built-in application launchers are registered.
          </Typography>
        ) : (
          <Stack spacing={1.25}>
            {sortedApplications.map((app) => {
              const id = str(app.id, "");
              const label = str(app.label, id);
              const runtimeInfo = asRecord(app.runtime);
              const state = str(runtimeInfo.state, "idle");
              const currentMode = str(runtimeInfo.mode, "");
              const currentCommand = str(runtimeInfo.command, "");
              const currentMessage = str(runtimeInfo.message, "");
              const currentModel = str(runtimeInfo.model, "");
              const currentLogs = Array.isArray(runtimeInfo.logs)
                ? runtimeInfo.logs.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
                : [];
              const supportsConfig = bool(app.supports_config);
              const running = state === "running";
              const recommendedModels = Array.isArray(app.recommended_models)
                ? app.recommended_models.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
                : [];
              const aliases = Array.isArray(app.aliases)
                ? app.aliases.filter((value): value is string => typeof value === "string" && value.trim().length > 0)
                : [];
              const modelValue = modelById[id] ?? "";
              const runtimeLaunchCommand = modelValue.trim()
                ? `ollama launch ${id} --model ${modelValue.trim()}`
                : str(app.runtime_launch_command) || `ollama launch ${id}`;
              const runtimeConfigCommand = supportsConfig
                ? str(app.runtime_config_command) || `ollama launch ${id} --config`
                : "";
                const hostOllamaBaseUrl = runtimeBaseUrl || "http://host.docker.internal:11434";
                const hostLaunchCommand = runtimeInDocker
                  ? modelValue.trim()
                    ? `docker exec -it -e OLLAMA_HOST=${hostOllamaBaseUrl} agentark ollama launch ${id} --model ${modelValue.trim()}`
                    : str(app.host_launch_command) || runtimeLaunchCommand
                  : runtimeLaunchCommand;
                const hostConfigCommand = supportsConfig
                  ? runtimeInDocker
                    ? str(app.host_config_command) || `docker exec -it -e OLLAMA_HOST=${hostOllamaBaseUrl} agentark ollama launch ${id} --config`
                    : runtimeConfigCommand
                  : "";
              const appBusy =
                (launchMutation.isPending && launchMutation.variables?.id === id) ||
                (stopMutation.isPending && stopMutation.variables?.id === id);

              return (
                <Box key={id} sx={{ width: "100%", px: 0, py: 1.15, borderBottom: "1px solid", borderColor: "divider", transition: "background 0.15s ease", "&:hover": { background: "var(--ui-rgba-57-208-255-040)" } }}>
                  <Stack spacing={0.75}>
                    {/* Line 1: dot + name ... status text */}
                    <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                      <Stack direction="row" sx={{ alignItems: "center", gap: 1, minWidth: 0 }}>
                        <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: stateDotColor(state) }} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>{label}</Typography>
                        <Chip size="small" label="External" variant="outlined" sx={{ height: 20 }} />
                      </Stack>
                      <Stack direction="row" sx={{ alignItems: "center", gap: 1, flexShrink: 0 }}>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>{state || "idle"}</Typography>
                        {currentMode ? <Typography variant="caption" sx={{ color: "text.secondary" }}>{currentMode}</Typography> : null}
                        {currentModel ? <Typography variant="caption" sx={{ color: "text.secondary" }}>{currentModel}</Typography> : null}
                        <Link href={str(app.docs_url)} target="_blank" rel="noreferrer" underline="hover" sx={{ fontSize: "0.75rem" }}>Docs</Link>
                        <Tooltip title="These launchers are terminal-first tools. For the full interactive experience, copy the command into your own terminal." arrow>
                          <Typography variant="caption" sx={{ color: "text.secondary", cursor: "help" }}>Terminal-first</Typography>
                        </Tooltip>
                      </Stack>
                    </Stack>

                    {/* Line 2: tagline / description */}
                    {str(app.tagline) || str(app.description) ? (
                      <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                        {str(app.tagline)}{str(app.tagline) && str(app.description) ? " — " : ""}{str(app.description)}
                      </Typography>
                    ) : null}

                    {/* Line 3: aliases */}
                    {aliases.length > 0 ? (
                      <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                        Aliases: {aliases.join(", ")}
                      </Typography>
                    ) : null}

                    {/* Model override + action buttons */}
                    <Stack direction={{ xs: "column", md: "row" }} spacing={1} sx={{ alignItems: { xs: "stretch", md: "center" }, pl: "15px" }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Model override (optional)"
                        value={modelValue}
                        placeholder={str(app.model_hint)}
                        onChange={(event) =>
                          setModelById((prev) => ({
                            ...prev,
                            [id]: event.target.value
                          }))
                        }
                      />
                      <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", flexShrink: 0 }}>
                        <Button
                          size="small"
                          variant="outlined"
                          disabled={appBusy}
                          onClick={async () => {
                            try {
                              await copyText(hostLaunchCommand);
                              setBannerError(null);
                              setBannerSuccess(`${label} ${runtimeInDocker ? "Docker" : "launch"} command copied.`);
                            } catch (error) {
                              setBannerSuccess(null);
                              setBannerError(errMessage(error));
                            }
                          }}
                        >
                          {runtimeInDocker ? "Copy Docker" : "Copy Launch"}
                        </Button>
                        {supportsConfig ? (
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={appBusy}
                            onClick={async () => {
                              try {
                                await copyText(hostConfigCommand);
                                setBannerError(null);
                                setBannerSuccess(`${label} ${runtimeInDocker ? "Docker config" : "config"} command copied.`);
                              } catch (error) {
                                setBannerSuccess(null);
                                setBannerError(errMessage(error));
                              }
                            }}
                          >
                            {runtimeInDocker ? "Copy Docker Config" : "Copy Config"}
                          </Button>
                        ) : null}
                        <Button
                          size="small"
                          variant="contained"
                          disabled={appBusy || !ollamaReady || running}
                          onClick={async () => {
                            setBannerError(null);
                            setBannerSuccess(null);
                            try {
                              await launchMutation.mutateAsync({
                                id,
                                mode: "launch",
                                model: modelValue
                              });
                              setBannerSuccess(`${label} launch started.`);
                            } catch (error) {
                              setBannerError(errMessage(error));
                            }
                          }}
                        >
                          Run in Runtime
                        </Button>
                        {supportsConfig ? (
                          <Button
                            size="small"
                            variant="outlined"
                            disabled={appBusy || !ollamaReady || running}
                            onClick={async () => {
                              setBannerError(null);
                              setBannerSuccess(null);
                              try {
                              await launchMutation.mutateAsync({
                                id,
                                mode: "config",
                                model: undefined
                              });
                                setBannerSuccess(`${label} config launch started.`);
                              } catch (error) {
                                setBannerError(errMessage(error));
                              }
                            }}
                          >
                            Run Config
                          </Button>
                        ) : null}
                        <Button
                          size="small"
                          color="warning"
                          disabled={appBusy || !running}
                          onClick={async () => {
                            setBannerError(null);
                            setBannerSuccess(null);
                            try {
                              await stopMutation.mutateAsync({ id });
                              setBannerSuccess(`${label} stopped.`);
                            } catch (error) {
                              setBannerError(errMessage(error));
                            }
                          }}
                        >
                          Stop
                        </Button>
                      </Stack>
                    </Stack>

                    {recommendedModels.length > 0 ? (
                      <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", pl: "15px" }}>
                        {recommendedModels.map((model) => (
                          <Chip
                            key={`${id}-${model}`}
                            size="small"
                            label={model}
                            clickable
                            onClick={() =>
                              setModelById((prev) => ({
                                ...prev,
                                [id]: model
                              }))
                            }
                          />
                        ))}
                      </Stack>
                    ) : null}

                    <Box className="metadata-box micro-surface" sx={{ p: 1.1, ml: "15px", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}>
                      <Typography className="micro-surface-kicker">Runtime</Typography>
                      <Typography className="micro-surface-title">
                        {runtimeInDocker ? "Launch commands" : "Launch command"}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block",
                          mt: 0.75
                        }}>
                        {runtimeInDocker ? "Docker host command" : "Launch command"}
                      </Typography>
                      <Typography variant="body2" sx={{ wordBreak: "break-all" }}>
                        {hostLaunchCommand}
                      </Typography>
                      {supportsConfig ? (
                        <>
                          <Divider sx={{ my: 0.75 }} />
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            {runtimeInDocker ? "Docker host config command" : "Config command"}
                          </Typography>
                          <Typography variant="body2" sx={{ wordBreak: "break-all" }}>
                            {hostConfigCommand}
                          </Typography>
                        </>
                      ) : null}
                      {runtimeInDocker ? (
                        <>
                          <Divider sx={{ my: 0.75 }} />
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            In-container command
                          </Typography>
                          <Typography variant="body2" sx={{ wordBreak: "break-all" }}>
                            {runtimeLaunchCommand}
                          </Typography>
                          {supportsConfig ? (
                            <>
                              <Divider sx={{ my: 0.75 }} />
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                In-container config command
                              </Typography>
                              <Typography variant="body2" sx={{ wordBreak: "break-all" }}>
                                {runtimeConfigCommand}
                              </Typography>
                            </>
                          ) : null}
                        </>
                      ) : null}
                    </Box>

                    {currentMessage || currentCommand || currentLogs.length > 0 ? (
                      <Box className="metadata-box micro-surface" sx={{ p: 1.1, ml: "15px" }}>
                        <Stack spacing={0.6}>
                          <Box className="micro-surface-head" sx={{ mb: 0.2 }}>
                            <Typography className="micro-surface-kicker">Runtime</Typography>
                            <Typography className="micro-surface-title">Live runtime detail</Typography>
                          </Box>
                          {currentMessage ? (
                            <Typography className="micro-surface-copy">
                              {currentMessage}
                            </Typography>
                          ) : null}
                          {currentCommand ? (
                            <Typography variant="caption" sx={{ wordBreak: "break-all", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", color: "var(--ui-rgba-191-223-255-840)" }}>
                              {currentCommand}
                            </Typography>
                          ) : null}
                          {currentLogs.length > 0 ? (
                            <Box className="micro-surface-scroll">
                              <Stack spacing={0.35}>
                                {currentLogs.map((line, index) => (
                                  <Typography
                                    key={`${id}-log-${index}`}
                                    variant="caption"
                                    sx={{
                                      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                                      whiteSpace: "pre-wrap",
                                      wordBreak: "break-word"
                                    }}
                                  >
                                    {line}
                                  </Typography>
                                ))}
                              </Stack>
                            </Box>
                          ) : null}
                        </Stack>
                      </Box>
                    ) : null}
                  </Stack>
                </Box>
              );
            })}
          </Stack>
        )}
      </Stack>
    </Box>
  );
}
