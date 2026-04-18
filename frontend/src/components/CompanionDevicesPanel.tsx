import AddLinkRoundedIcon from "@mui/icons-material/AddLinkRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ComputerRoundedIcon from "@mui/icons-material/ComputerRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import DevicesRoundedIcon from "@mui/icons-material/DevicesRounded";
import HomeRoundedIcon from "@mui/icons-material/HomeRounded";
import PhoneAndroidRoundedIcon from "@mui/icons-material/PhoneAndroidRounded";
import PhoneIphoneRoundedIcon from "@mui/icons-material/PhoneIphoneRounded";
import SensorsRoundedIcon from "@mui/icons-material/SensorsRounded";
import WarningAmberRoundedIcon from "@mui/icons-material/WarningAmberRounded";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Divider,
  FormControlLabel,
  IconButton,
  MenuItem,
  Stack,
  TextField,
  Tooltip,
  Typography
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type JSX } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import type {
  CompanionCapabilityDescriptor,
  CompanionCommandRecord,
  CompanionDeviceRecord,
  CompanionPairingSession,
  CompanionPreset
} from "../types";

function errMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error || "Request failed");
}

function statusTone(status: string): "success" | "warning" | "error" | "info" | "default" {
  const value = status.toLowerCase();
  if (value === "online") return "success";
  if (value === "paired" || value === "idle") return "info";
  if (value === "pairing" || value === "busy") return "warning";
  if (value === "revoked" || value === "offline" || value === "error") return "error";
  return "default";
}

function presetIcon(presetId: string, platform: string): JSX.Element {
  const value = `${presetId} ${platform}`.toLowerCase();
  if (value.includes("ios") || value.includes("iphone")) return <PhoneIphoneRoundedIcon fontSize="small" />;
  if (value.includes("android")) return <PhoneAndroidRoundedIcon fontSize="small" />;
  if (value.includes("desktop") || value.includes("windows") || value.includes("linux") || value.includes("mac")) {
    return <ComputerRoundedIcon fontSize="small" />;
  }
  if (value.includes("server") || value.includes("home")) return <HomeRoundedIcon fontSize="small" />;
  if (value.includes("pi") || value.includes("iot")) return <SensorsRoundedIcon fontSize="small" />;
  return <DevicesRoundedIcon fontSize="small" />;
}

function labelForCapability(capabilities: CompanionCapabilityDescriptor[], id: string): string {
  const found = capabilities.find((capability) => capability.id === id);
  if (found) return found.label;
  return id
    .split(/[._:-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function compactJson(value: unknown): string {
  if (value == null) return "{}";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return "{}";
  }
}

function boundedJsonPreview(value: unknown, depth = 0): unknown {
  if (value == null || typeof value !== "object") return value;
  if (depth >= 3) return Array.isArray(value) ? `[${value.length} items]` : "{...}";
  if (Array.isArray(value)) {
    return value.slice(0, 8).map((item) => boundedJsonPreview(item, depth + 1));
  }
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .slice(0, 16)
      .map(([key, item]) => [key, boundedJsonPreview(item, depth + 1)])
  );
}

function safeJsonPreview(value: unknown): string {
  return compactJson(boundedJsonPreview(value));
}

function shortValue(value?: string | null): string {
  if (!value) return "none";
  return value.length > 18 ? `${value.slice(0, 18)}...` : value;
}

function defaultPresetCapabilities(
  preset: CompanionPreset | null,
  capabilities: CompanionCapabilityDescriptor[]
): string[] {
  if (!preset) return [];
  const capabilityMap = new Map(capabilities.map((capability) => [capability.id, capability]));
  const saferDefaults = preset.capability_ids.filter((id) => capabilityMap.get(id)?.risk !== "high");
  return saferDefaults.length ? saferDefaults : [];
}

function parseJsonObject(raw: string): Record<string, unknown> {
  const trimmed = raw.trim();
  if (!trimmed) return {};
  const parsed = JSON.parse(trimmed) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Arguments must be a JSON object.");
  }
  return parsed as Record<string, unknown>;
}

function latestSession(sessions: CompanionPairingSession[]): CompanionPairingSession | null {
  return sessions[0] ?? null;
}

export function CompanionDevicesPanel({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [notice, setNotice] = useState<{ kind: "success" | "info" | "warning" | "error"; text: string } | null>(null);
  const [selectedPresetId, setSelectedPresetId] = useState("ios");
  const [draftName, setDraftName] = useState("My iPhone");
  const [draftCapabilities, setDraftCapabilities] = useState<string[]>([]);
  const [trustedUnattested, setTrustedUnattested] = useState(false);
  const [customCapability, setCustomCapability] = useState("");
  const [pairingPayload, setPairingPayload] = useState<Record<string, unknown> | null>(null);
  const [selectedDeviceId, setSelectedDeviceId] = useState<string | null>(null);
  const [commandCapability, setCommandCapability] = useState("");
  const [commandAction, setCommandAction] = useState("");
  const [commandArgs, setCommandArgs] = useState("{}");
  const [rotatedToken, setRotatedToken] = useState<string | null>(null);

  const presetsQ = useQuery({
    queryKey: ["companion-presets"],
    queryFn: api.getCompanionPresets
  });
  const protocolQ = useQuery({
    queryKey: ["companion-protocol"],
    queryFn: api.getCompanionProtocol
  });
  const devicesQ = useQuery({
    queryKey: ["companion-devices"],
    queryFn: api.getCompanionDevices,
    refetchInterval: autoRefresh ? 8000 : false
  });
  const auditQ = useQuery({
    queryKey: ["companion-audit"],
    queryFn: () => api.getCompanionAudit(80),
    refetchInterval: autoRefresh ? 15000 : false
  });

  const presets = presetsQ.data?.presets ?? [];
  const capabilities = presetsQ.data?.capabilities ?? [];
  const devices = devicesQ.data?.devices ?? [];
  const sessions = devicesQ.data?.pairing_sessions ?? [];
  const pendingApprovals = devicesQ.data?.pending_approvals ?? [];
  const selectedPreset = presets.find((preset) => preset.id === selectedPresetId) ?? presets[0] ?? null;
  const selectedDevice = devices.find((device) => device.id === selectedDeviceId) ?? devices[0] ?? null;
  const capabilityMap = useMemo(
    () => new Map(capabilities.map((capability) => [capability.id, capability])),
    [capabilities]
  );

  const commandsQ = useQuery({
    queryKey: ["companion-commands", selectedDevice?.id],
    queryFn: () => api.getCompanionCommands(selectedDevice?.id || ""),
    enabled: Boolean(selectedDevice?.id),
    refetchInterval: autoRefresh && selectedDevice?.id ? 8000 : false
  });

  useEffect(() => {
    if (!selectedPreset) return;
    setDraftCapabilities((current) =>
      current.length ? current : defaultPresetCapabilities(selectedPreset, capabilities)
    );
    if (!draftName.trim()) setDraftName(selectedPreset.label);
  }, [selectedPreset, capabilities, draftName]);

  useEffect(() => {
    if (!selectedDevice) {
      if (selectedDeviceId !== null) setSelectedDeviceId(null);
      return;
    }
    if (!selectedDeviceId || !devices.some((device) => device.id === selectedDeviceId)) {
      setSelectedDeviceId(selectedDevice.id);
    }
  }, [devices, selectedDevice, selectedDeviceId]);

  useEffect(() => {
    const caps = selectedDevice?.token_capabilities ?? selectedDevice?.granted_capabilities ?? [];
    if (!caps.length) {
      setCommandCapability("");
      return;
    }
    if (!commandCapability || !caps.includes(commandCapability)) {
      setCommandCapability(caps[0]);
    }
  }, [selectedDevice, commandCapability]);

  const selectedCapabilityRisk = capabilityMap.get(commandCapability)?.risk ?? "high";
  const selectedHighRiskCapabilities = draftCapabilities.filter(
    (id) => (capabilityMap.get(id)?.risk ?? "high") === "high"
  );
  const hasHighRiskDraftCapability = selectedHighRiskCapabilities.length > 0;
  const bundledMobileNeedsAttestation =
    hasHighRiskDraftCapability && (selectedPreset?.id === "ios" || selectedPreset?.id === "android");
  useEffect(() => {
    if (!hasHighRiskDraftCapability || bundledMobileNeedsAttestation) {
      setTrustedUnattested(false);
    }
  }, [hasHighRiskDraftCapability, bundledMobileNeedsAttestation]);
  const pairingBlockedByAttestation =
    hasHighRiskDraftCapability && (bundledMobileNeedsAttestation || !trustedUnattested);
  const overview = devicesQ.data?.overview;
  const latestPairing = latestSession(sessions);
  const protocol = protocolQ.data;

  const refreshAll = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["companion-devices"] }),
      queryClient.invalidateQueries({ queryKey: ["companion-audit"] }),
      queryClient.invalidateQueries({ queryKey: ["companion-commands"] })
    ]);
  };

  const createPairingMutation = useMutation({
    mutationFn: async () => {
      if (!selectedPreset) throw new Error("Choose a device type first.");
      const name = draftName.trim();
      if (!name) throw new Error("Enter a device name.");
      return api.createCompanionPairingSession({
        display_name: name,
        preset_id: selectedPreset.id,
        platform: selectedPreset.platform,
        capabilities: draftCapabilities,
        trusted_unattested: trustedUnattested && !bundledMobileNeedsAttestation
      });
    },
    onSuccess: async (response) => {
      setPairingPayload(response.pairing_payload ?? null);
      setNotice({ kind: "success", text: "Pairing code created. Approve it after the device claims the session." });
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const approvePairingMutation = useMutation({
    mutationFn: (sessionId: string) => api.approveCompanionPairingSession(sessionId),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Pairing approved. The companion receives its scoped token through WebSocket claim." });
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const createCommandMutation = useMutation({
    mutationFn: async () => {
      if (!selectedDevice) throw new Error("Select a companion device first.");
      if (!commandCapability) throw new Error("Choose a capability.");
      if (!commandAction.trim()) throw new Error("Enter a typed action id.");
      const args = parseJsonObject(commandArgs);
      return api.createCompanionCommand(selectedDevice.id, {
        capability: commandCapability,
        action: commandAction.trim(),
        requested_scopes: [commandCapability],
        arguments: args,
        actor: "companion-devices-ui"
      });
    },
    onSuccess: async (response) => {
      const command = response.command as CompanionCommandRecord | undefined;
      setNotice({
        kind: command?.status === "approval_required" ? "warning" : "success",
        text:
          command?.status === "approval_required"
            ? "Command is waiting for fresh approval before dispatch."
            : "Command queued for the companion device."
      });
      setCommandAction("");
      setCommandArgs("{}");
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const approveCommandMutation = useMutation({
    mutationFn: ({ commandId, approved }: { commandId: string; approved: boolean }) =>
      api.approveCompanionCommand(commandId, {
        approved,
        reason: approved ? "Approved from Companion Devices UI." : "Denied from Companion Devices UI."
      }),
    onSuccess: async (_, vars) => {
      setNotice({ kind: vars.approved ? "success" : "info", text: vars.approved ? "Command approved and queued." : "Command denied." });
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const revokeMutation = useMutation({
    mutationFn: (deviceId: string) => api.revokeCompanionDevice(deviceId),
    onSuccess: async () => {
      setNotice({ kind: "success", text: "Companion device revoked." });
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const rotateMutation = useMutation({
    mutationFn: (device: CompanionDeviceRecord) =>
      api.rotateCompanionToken(device.id, {
        requested_scopes: device.token_capabilities ?? device.granted_capabilities ?? []
      }),
    onSuccess: async (response) => {
      setRotatedToken(response.rotation?.device_token ?? null);
      setNotice({ kind: "warning", text: "Device token rotated. Store the new token in the companion keychain." });
      await refreshAll();
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const copyPairingPayload = async () => {
    if (!pairingPayload) return;
    await navigator.clipboard?.writeText(JSON.stringify(pairingPayload, null, 2));
    setNotice({ kind: "info", text: "Pairing payload copied." });
  };

  const addCustomCapability = () => {
    const normalized = customCapability.trim().toLowerCase();
    if (!normalized) return;
    const id = normalized.startsWith("custom.") || normalized.startsWith("custom:")
      ? normalized
      : `custom.${normalized.replace(/[^a-z0-9_.:-]+/g, "_")}`;
    setDraftCapabilities((current) => Array.from(new Set([...current, id])).sort());
    setCustomCapability("");
  };

  const commandCapabilities = selectedDevice?.token_capabilities ?? selectedDevice?.granted_capabilities ?? [];
  const commandRows = commandsQ.data?.commands ?? [];
  const metricItems = [
    { label: "Total", value: overview?.total ?? devices.length, tone: "info" },
    {
      label: "Online",
      value: overview?.online ?? devices.filter((device) => device.state === "online").length,
      tone: "good"
    },
    {
      label: "Pairing",
      value: overview?.pending_pairing ?? sessions.filter((session) => ["pending", "claimed"].includes(session.status)).length,
      tone: "warn"
    },
    { label: "Approvals", value: overview?.pending_approvals ?? pendingApprovals.length, tone: "warn" }
  ];
  const presetCapabilityIds = selectedPreset?.capability_ids ?? [];

  return (
    <Stack spacing={1.5} sx={{ width: "100%", minWidth: 0, overflowX: "clip" }}>
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {rotatedToken ? (
        <Alert severity="warning" onClose={() => setRotatedToken(null)}>
          New persistent device token: <code>{rotatedToken}</code>
        </Alert>
      ) : null}

      <Box className="list-shell stat-strip" sx={{ minWidth: 0 }}>
        {metricItems.map((item) => (
          <Box key={item.label} className="stat-strip-item" data-tone={item.tone}>
            <Typography className="stat-strip-label">{item.label}</Typography>
            <Typography className="stat-strip-value">{item.value}</Typography>
          </Box>
        ))}
      </Box>

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "minmax(0, 1fr)", xl: "minmax(340px, 420px) minmax(0, 1fr)" },
          gap: 1.5,
          alignItems: "start",
          minWidth: 0
        }}
      >
        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.35}>
            <Stack direction="row" spacing={1} sx={{ alignItems: "flex-start", minWidth: 0 }}>
              <Box
                sx={{
                  width: 34,
                  height: 34,
                  borderRadius: 1,
                  display: "grid",
                  placeItems: "center",
                  color: "primary.main",
                  bgcolor: "rgba(91, 164, 255, 0.1)",
                  flex: "0 0 auto"
                }}
              >
                <AddLinkRoundedIcon fontSize="small" />
              </Box>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Pair Device
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary", lineHeight: 1.45 }}>
                  Short-lived code, device claim, then explicit approval.
                </Typography>
              </Box>
            </Stack>

            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: { xs: "minmax(0, 1fr)", sm: "minmax(0, 1fr) minmax(0, 1fr)" },
                gap: 1
              }}
            >
              <TextField
                select
                size="small"
                label="Device type"
                value={selectedPresetId}
                onChange={(event) => {
                  const next = event.target.value;
                  const preset = presets.find((item) => item.id === next);
                  setSelectedPresetId(next);
                  setDraftCapabilities(defaultPresetCapabilities(preset ?? null, capabilities));
                  setTrustedUnattested(false);
                  setDraftName(preset?.label ?? "");
                  setPairingPayload(null);
                }}
              >
                {presets.map((preset) => (
                  <MenuItem key={preset.id} value={preset.id}>
                    {preset.label}
                  </MenuItem>
                ))}
              </TextField>
              <TextField
                size="small"
                label="Device name"
                value={draftName}
                onChange={(event) => setDraftName(event.target.value)}
              />
            </Box>

            <Box sx={{ minWidth: 0 }}>
              <Stack
                direction={{ xs: "column", sm: "row" }}
                spacing={0.75}
                sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between", mb: 0.75 }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                    Grants
                  </Typography>
                  <Typography variant="caption" sx={{ color: "text.secondary" }}>
                    High-risk grants still need approval for sensitive actions.
                  </Typography>
                </Box>
                <Chip size="small" variant="outlined" label={`${draftCapabilities.length} selected`} />
              </Stack>
              <Box
                sx={{
                  display: "grid",
                  gridTemplateColumns: { xs: "minmax(0, 1fr)", sm: "repeat(2, minmax(0, 1fr))" },
                  gap: 0.5,
                  minWidth: 0
                }}
              >
                {presetCapabilityIds.map((id) => {
                  const risk = capabilityMap.get(id)?.risk ?? "low";
                  return (
                    <FormControlLabel
                      key={id}
                      sx={{
                        m: 0,
                        minWidth: 0,
                        alignItems: "flex-start",
                        "& .MuiCheckbox-root": { p: 0.75 },
                        "& .MuiFormControlLabel-label": { minWidth: 0, width: "100%" }
                      }}
                      control={
                        <Checkbox
                          size="small"
                          checked={draftCapabilities.includes(id)}
                          onChange={(event) =>
                            setDraftCapabilities((current) =>
                              event.target.checked
                                ? Array.from(new Set([...current, id])).sort()
                                : current.filter((value) => value !== id)
                            )
                          }
                        />
                      }
                      label={
                        <Stack
                          direction="row"
                          spacing={0.5}
                          sx={{ alignItems: "center", minWidth: 0, py: 0.45 }}
                        >
                          <Typography
                            component="span"
                            variant="body2"
                            sx={{ minWidth: 0, overflowWrap: "anywhere", lineHeight: 1.3 }}
                          >
                            {labelForCapability(capabilities, id)}
                          </Typography>
                          {risk === "high" ? <Chip size="small" color="warning" label="Approval" /> : null}
                        </Stack>
                      }
                    />
                  );
                })}
              </Box>
            </Box>

            {selectedPreset?.id === "custom" ? (
              <Stack spacing={1}>
                <Divider />
                <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                  <TextField
                    size="small"
                    label="Custom capability id"
                    value={customCapability}
                    onChange={(event) => setCustomCapability(event.target.value)}
                    helperText="Use a structured id such as custom.greenhouse_sensor."
                    fullWidth
                  />
                  <Button variant="outlined" onClick={addCustomCapability} sx={{ minWidth: 96 }}>
                    Add
                  </Button>
                </Stack>
                {protocol ? (
                  <Box
                    sx={{
                      border: "1px solid",
                      borderColor: "divider",
                      borderRadius: 1,
                      p: 1,
                      bgcolor: "rgba(255, 255, 255, 0.02)",
                      minWidth: 0
                    }}
                  >
                    <Stack spacing={0.75}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Protocol {protocol.protocol_version}
                      </Typography>
                      <Typography variant="body2" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                        Claim at {protocol.websocket_path}, store the scoped token, then send pulse and command results.
                      </Typography>
                      <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5 }}>
                        {protocol.messages.slice(0, 7).map((message) => (
                          <Chip key={message} size="small" variant="outlined" label={message} />
                        ))}
                      </Stack>
                    </Stack>
                  </Box>
                ) : null}
              </Stack>
            ) : null}

            <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5 }}>
              {draftCapabilities.map((id) => (
                <Chip key={id} size="small" label={labelForCapability(capabilities, id)} />
              ))}
            </Stack>

            {hasHighRiskDraftCapability ? (
              <Alert severity={bundledMobileNeedsAttestation ? "warning" : "info"}>
                {bundledMobileNeedsAttestation
                  ? "High-risk iOS and Android grants require verified platform attestation before approval."
                  : "High-risk grants without platform attestation require an audited trusted-unattested override."}
                {!bundledMobileNeedsAttestation ? (
                  <FormControlLabel
                    sx={{ display: "flex", mt: 0.5, ml: 0 }}
                    control={
                      <Checkbox
                        size="small"
                        checked={trustedUnattested}
                        onChange={(event) => setTrustedUnattested(event.target.checked)}
                      />
                    }
                    label="Trust this unattested device for the selected high-risk grants"
                  />
                ) : null}
              </Alert>
            ) : null}

            <Button
              fullWidth
              variant="contained"
              onClick={() => createPairingMutation.mutate()}
              disabled={createPairingMutation.isPending || pairingBlockedByAttestation}
            >
              {createPairingMutation.isPending ? "Creating..." : "Create Pairing Code"}
            </Button>

            {pairingPayload ? (
              <Box
                sx={{
                  border: "1px solid",
                  borderColor: "divider",
                  borderRadius: 1,
                  p: 1,
                  bgcolor: "rgba(255, 255, 255, 0.02)",
                  minWidth: 0
                }}
              >
                <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center", gap: 1 }}>
                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                    Pairing Payload
                  </Typography>
                  <Tooltip title="Copy pairing payload">
                    <IconButton size="small" onClick={copyPairingPayload} aria-label="Copy pairing payload">
                      <ContentCopyRoundedIcon fontSize="small" />
                    </IconButton>
                  </Tooltip>
                </Stack>
                <Typography
                  component="pre"
                  variant="caption"
                  sx={{
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                    maxHeight: 190,
                    overflow: "auto",
                    m: 0
                  }}
                >
                  {JSON.stringify(pairingPayload, null, 2)}
                </Typography>
                <Alert severity="warning" sx={{ mt: 1 }}>
                  One-time pairing secret. Approve only after the expected device claims it.
                </Alert>
              </Box>
            ) : null}

            {latestPairing ? (
              <Box className="action-row">
                <Stack spacing={0.75}>
                  <Stack direction="row" spacing={1} sx={{ justifyContent: "space-between", gap: 1 }}>
                    <Box sx={{ minWidth: 0 }}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650, overflowWrap: "anywhere" }}>
                        {latestPairing.display_name}
                      </Typography>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>
                        Latest session - {latestPairing.status}
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={latestPairing.status} />
                  </Stack>
                  {latestPairing.claimed_device_public_key ? (
                    <Typography variant="caption" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                      Claimed identity: {shortValue(latestPairing.claimed_device_public_key)}
                    </Typography>
                  ) : null}
                  <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5 }}>
                    {latestPairing.attestation?.verified ? (
                      <Chip size="small" color="success" label="Attested" />
                    ) : (
                      <Chip size="small" variant="outlined" label="Unattested" />
                    )}
                    {latestPairing.trusted_unattested ? (
                      <Chip size="small" color="warning" label="Trusted unattested" />
                    ) : null}
                  </Stack>
                  <Button
                    variant="outlined"
                    onClick={() => approvePairingMutation.mutate(latestPairing.id)}
                    disabled={approvePairingMutation.isPending || latestPairing.status !== "claimed"}
                  >
                    Approve Claimed Pairing
                  </Button>
                </Stack>
              </Box>
            ) : null}
          </Stack>
        </Box>

        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Devices
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Paired identity, granted capabilities, token scope, and last pulse.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label={`${devices.length} device${devices.length === 1 ? "" : "s"}`} />
            </Stack>
            {devicesQ.error ? <Alert severity="error">{errMessage(devicesQ.error)}</Alert> : null}
            {devices.length === 0 ? (
              <Alert severity="info">No companion devices are paired yet.</Alert>
            ) : (
              <Stack spacing={0.85}>
                {devices.map((device) => {
                  const selected = device.id === selectedDevice?.id;
                  return (
                    <Box
                      key={device.id}
                      component="button"
                      type="button"
                      className="action-row"
                      onClick={() => setSelectedDeviceId(device.id)}
                      sx={{
                        width: "100%",
                        color: "inherit",
                        font: "inherit",
                        textAlign: "left",
                        cursor: "pointer",
                        borderColor: selected ? "primary.main" : undefined,
                        boxShadow: selected ? "0 0 0 1px rgba(91, 164, 255, 0.18)" : undefined,
                        "&:focus-visible": { outline: "none", boxShadow: "var(--button-focus-ring)" }
                      }}
                    >
                      <Stack spacing={0.8} sx={{ width: "100%", minWidth: 0 }}>
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          spacing={1}
                          sx={{ alignItems: { xs: "stretch", sm: "flex-start" }, justifyContent: "space-between" }}
                        >
                          <Stack direction="row" spacing={1} sx={{ alignItems: "flex-start", minWidth: 0 }}>
                            <Box sx={{ pt: 0.15, flex: "0 0 auto" }}>{presetIcon(device.preset_id, device.platform)}</Box>
                            <Box sx={{ minWidth: 0 }}>
                              <Typography variant="subtitle2" sx={{ fontWeight: 650, overflowWrap: "anywhere" }}>
                                {device.display_name}
                              </Typography>
                              <Typography variant="caption" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                                {device.platform} - Last seen {formatUiDateTime(device.last_seen_at, { fallback: "never" })}
                              </Typography>
                            </Box>
                          </Stack>
                          <Chip size="small" color={statusTone(device.state)} label={device.state} sx={{ alignSelf: "flex-start" }} />
                        </Stack>
                        <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5 }}>
                          {device.attestation?.verified ? (
                            <Chip size="small" color="success" label="Attested" />
                          ) : device.trusted_unattested ? (
                            <Chip size="small" color="warning" label="Trusted unattested" />
                          ) : null}
                          {(device.granted_capabilities ?? []).map((id) => (
                            <Chip key={id} size="small" variant="outlined" label={labelForCapability(capabilities, id)} />
                          ))}
                        </Stack>
                      </Stack>
                    </Box>
                  );
                })}
              </Stack>
            )}
          </Stack>
        </Box>
      </Box>

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "minmax(0, 1fr)", xl: "minmax(340px, 420px) minmax(0, 1fr)" },
          gap: 1.5,
          alignItems: "start",
          minWidth: 0
        }}
      >
        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Typed Command
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Queue adapter actions for the selected device.
                </Typography>
              </Box>
              {selectedDevice ? <Chip size="small" variant="outlined" label={selectedDevice.display_name} /> : null}
            </Stack>
            {selectedDevice ? (
              <>
                <Alert
                  severity={selectedCapabilityRisk === "high" ? "warning" : "info"}
                  icon={selectedCapabilityRisk === "high" ? <WarningAmberRoundedIcon /> : <CheckCircleRoundedIcon />}
                >
                  {selectedCapabilityRisk === "high"
                    ? "Fresh approval required before dispatch."
                    : "Scope validation runs before dispatch."}
                </Alert>
                <TextField
                  select
                  size="small"
                  label="Capability"
                  value={commandCapability}
                  onChange={(event) => setCommandCapability(event.target.value)}
                >
                  {commandCapabilities.map((id) => (
                    <MenuItem key={id} value={id}>
                      {labelForCapability(capabilities, id)}
                    </MenuItem>
                  ))}
                </TextField>
                <TextField
                  size="small"
                  label="Typed action id"
                  value={commandAction}
                  onChange={(event) => setCommandAction(event.target.value)}
                  helperText="Use adapter action ids such as capture_photo or run_shortcut."
                />
                <TextField
                  size="small"
                  label="Arguments JSON"
                  value={commandArgs}
                  onChange={(event) => setCommandArgs(event.target.value)}
                  multiline
                  minRows={4}
                  spellCheck={false}
                />
                <Stack direction={{ xs: "column", sm: "row" }} spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                  <Button
                    variant="contained"
                    onClick={() => createCommandMutation.mutate()}
                    disabled={createCommandMutation.isPending || !commandCapabilities.length}
                  >
                    {createCommandMutation.isPending ? "Queueing..." : "Queue Command"}
                  </Button>
                  <Button
                    variant="outlined"
                    onClick={() => rotateMutation.mutate(selectedDevice)}
                    disabled={rotateMutation.isPending || selectedDevice.state === "revoked"}
                  >
                    Rotate Token
                  </Button>
                  <Button
                    variant="outlined"
                    color="error"
                    onClick={() => revokeMutation.mutate(selectedDevice.id)}
                    disabled={revokeMutation.isPending || selectedDevice.state === "revoked"}
                  >
                    Revoke Device
                  </Button>
                </Stack>
              </>
            ) : (
              <Alert severity="info">Select a paired device to queue typed commands.</Alert>
            )}
          </Stack>
        </Box>

        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Approvals
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  High-risk commands waiting for an operator decision.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label={`${pendingApprovals.length} waiting`} />
            </Stack>
            {pendingApprovals.length === 0 ? (
              <Alert severity="info">No high-risk companion commands are waiting.</Alert>
            ) : (
              <Stack spacing={0.85}>
                {pendingApprovals.map((command) => (
                  <Box key={command.id} className="action-row">
                    <Stack spacing={0.75} sx={{ width: "100%", minWidth: 0 }}>
                      <Stack
                        direction={{ xs: "column", sm: "row" }}
                        sx={{ justifyContent: "space-between", gap: 1, alignItems: { xs: "stretch", sm: "flex-start" } }}
                      >
                        <Box sx={{ minWidth: 0 }}>
                          <Typography variant="subtitle2" sx={{ fontWeight: 650, overflowWrap: "anywhere" }}>
                            {command.action}
                          </Typography>
                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                            {labelForCapability(capabilities, command.capability)} -{" "}
                            {formatUiDateTime(command.requested_at, { fallback: "unknown" })}
                          </Typography>
                        </Box>
                        <Chip size="small" color="warning" label="Fresh approval" sx={{ alignSelf: "flex-start" }} />
                      </Stack>
                      <Typography
                        component="pre"
                        variant="caption"
                        sx={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", m: 0 }}
                      >
                        {safeJsonPreview(command.arguments)}
                      </Typography>
                      <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
                        <Button
                          size="small"
                          variant="contained"
                          onClick={() => approveCommandMutation.mutate({ commandId: command.id, approved: true })}
                        >
                          Approve
                        </Button>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={() => approveCommandMutation.mutate({ commandId: command.id, approved: false })}
                        >
                          Deny
                        </Button>
                      </Stack>
                    </Stack>
                  </Box>
                ))}
              </Stack>
            )}
          </Stack>
        </Box>
      </Box>

      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "minmax(0, 1fr)", xl: "repeat(2, minmax(0, 1fr))" },
          gap: 1.5,
          alignItems: "start",
          minWidth: 0
        }}
      >
        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.25}>
            <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
              Command History
            </Typography>
            {selectedDevice ? (
              commandRows.length ? (
                <Stack spacing={0.75}>
                  {commandRows.slice(0, 8).map((command) => (
                    <Box key={command.id} className="action-row">
                      <Stack spacing={0.5} sx={{ width: "100%", minWidth: 0 }}>
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          sx={{ justifyContent: "space-between", gap: 1, alignItems: { xs: "stretch", sm: "flex-start" } }}
                        >
                          <Typography variant="subtitle2" sx={{ fontWeight: 650, overflowWrap: "anywhere" }}>
                            {command.action}
                          </Typography>
                          <Chip size="small" variant="outlined" label={command.status} sx={{ alignSelf: "flex-start" }} />
                        </Stack>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          {labelForCapability(capabilities, command.capability)} -{" "}
                          {formatUiDateTime(command.requested_at, { fallback: "unknown" })}
                        </Typography>
                        {command.result_preview || command.error ? (
                          <Typography
                            variant="body2"
                            sx={{ color: command.error ? "error.main" : "text.secondary", overflowWrap: "anywhere" }}
                          >
                            {command.error || command.result_preview}
                          </Typography>
                        ) : null}
                      </Stack>
                    </Box>
                  ))}
                </Stack>
              ) : (
                <Alert severity="info">No commands for the selected device.</Alert>
              )
            ) : (
              <Alert severity="info">Select a device to see command history.</Alert>
            )}
          </Stack>
        </Box>

        <Box className="list-shell" sx={{ p: 1.5, minWidth: 0 }}>
          <Stack spacing={1.25}>
            <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
              Audit
            </Typography>
            {(auditQ.data?.events ?? []).length === 0 ? (
              <Alert severity="info">No companion-device audit events yet.</Alert>
            ) : (
              <Stack spacing={0.75}>
                {(auditQ.data?.events ?? []).slice(0, 10).map((event) => (
                  <Box key={event.id} className="action-row">
                    <Stack spacing={0.4} sx={{ width: "100%", minWidth: 0 }}>
                      <Stack
                        direction={{ xs: "column", sm: "row" }}
                        sx={{ justifyContent: "space-between", gap: 1, alignItems: { xs: "stretch", sm: "flex-start" } }}
                      >
                        <Typography variant="subtitle2" sx={{ fontWeight: 650, overflowWrap: "anywhere" }}>
                          {event.event_type}
                        </Typography>
                        <Chip size="small" variant="outlined" label={event.decision} sx={{ alignSelf: "flex-start" }} />
                      </Stack>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>
                        {formatUiDateTime(event.timestamp, { fallback: "unknown" })}
                      </Typography>
                      <Typography variant="body2" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                        {event.reason}
                      </Typography>
                      {event.event_hash ? (
                        <Typography variant="caption" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                          Audit hash: {shortValue(event.event_hash)}
                        </Typography>
                      ) : null}
                    </Stack>
                  </Box>
                ))}
              </Stack>
            )}
          </Stack>
        </Box>
      </Box>
    </Stack>
  );
}
