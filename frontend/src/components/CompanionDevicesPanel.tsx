import AddLinkRoundedIcon from "@mui/icons-material/AddLinkRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ComputerRoundedIcon from "@mui/icons-material/ComputerRounded";
import ContentCopyRoundedIcon from "@mui/icons-material/ContentCopyRounded";
import DevicesRoundedIcon from "@mui/icons-material/DevicesRounded";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import HomeRoundedIcon from "@mui/icons-material/HomeRounded";
import LaunchRoundedIcon from "@mui/icons-material/LaunchRounded";
import PhoneAndroidRoundedIcon from "@mui/icons-material/PhoneAndroidRounded";
import PhoneIphoneRoundedIcon from "@mui/icons-material/PhoneIphoneRounded";
import SensorsRoundedIcon from "@mui/icons-material/SensorsRounded";
import WarningAmberRoundedIcon from "@mui/icons-material/WarningAmberRounded";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  IconButton,
  LinearProgress,
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
import { humanizeMachineLabel, humanizeStatusLabel } from "../lib/displayLabels";
import { sessionNeedsPairingPoll } from "./companionPairing";
import type {
  CompanionCapabilityDescriptor,
  CompanionCommandRecord,
  CompanionDevicesResponse,
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

function recordString(record: Record<string, unknown> | null, key: string): string {
  const value = record?.[key];
  return typeof value === "string" ? value : "";
}

function recordBool(record: Record<string, unknown> | null, key: string): boolean {
  return record?.[key] === true;
}

function companionWebUrl(sessionId: string, code: string, wsUrl: string): string {
  const origin =
    wsUrl.startsWith("wss://")
      ? `https://${wsUrl.slice("wss://".length).split("/")[0]}`
      : wsUrl.startsWith("ws://")
        ? `http://${wsUrl.slice("ws://".length).split("/")[0]}`
        : typeof window !== "undefined"
          ? window.location.origin
          : "";
  if (!origin) return "";
  const url = new URL("/companion/web", origin);
  if (sessionId) url.searchParams.set("session_id", sessionId);
  if (code) url.searchParams.set("code", code);
  if (wsUrl) url.searchParams.set("ws", wsUrl);
  return url.toString();
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

type PairingWizardStep = "tunnel" | "pairing" | "approve" | "connected";

export function CompanionDevicesPanel({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [notice, setNotice] = useState<{ kind: "success" | "info" | "warning" | "error"; text: string } | null>(null);
  const [selectedPresetId, setSelectedPresetId] = useState("ios");
  const [draftName, setDraftName] = useState("My Device");
  const [draftCapabilities, setDraftCapabilities] = useState<string[]>([]);
  const [trustedUnattested, setTrustedUnattested] = useState(false);
  const [customCapability, setCustomCapability] = useState("");
  const [pairingPayload, setPairingPayload] = useState<Record<string, unknown> | null>(null);
  const [selectedDeviceId, setSelectedDeviceId] = useState<string | null>(null);
  const [commandCapability, setCommandCapability] = useState("");
  const [commandAction, setCommandAction] = useState("");
  const [commandArgs, setCommandArgs] = useState("{}");
  const [rotatedToken, setRotatedToken] = useState<string | null>(null);
  const [pairingDialogOpen, setPairingDialogOpen] = useState(false);
  const [pairingStep, setPairingStep] = useState<PairingWizardStep>("tunnel");
  const activePairingSessionId = recordString(pairingPayload, "session_id");

  const presetsQ = useQuery({
    queryKey: ["companion-presets"],
    queryFn: api.getCompanionPresets
  });
  const protocolQ = useQuery({
    queryKey: ["companion-protocol"],
    queryFn: api.getCompanionProtocol
  });
  const connectivityQ = useQuery({
    queryKey: ["companion-connectivity"],
    queryFn: api.getCompanionConnectivity,
    refetchInterval: autoRefresh ? 10000 : false
  });
  const devicesQ = useQuery({
    queryKey: ["companion-devices"],
    queryFn: api.getCompanionDevices,
    refetchInterval: (query) =>
      sessionNeedsPairingPoll(query.state.data as CompanionDevicesResponse | undefined, activePairingSessionId)
        ? 2500
        : autoRefresh
          ? 8000
          : false
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
  const connectivity = connectivityQ.data ?? null;
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
    const declared = selectedDevice?.declared_commands ?? [];
    if (declared.length) {
      const stillValid = declared.some(
        (command) => command.capability === commandCapability && command.action === commandAction
      );
      if (!stillValid) {
        setCommandCapability(declared[0].capability);
        setCommandAction(declared[0].action);
      }
      return;
    }
    const caps = selectedDevice?.token_capabilities ?? selectedDevice?.granted_capabilities ?? [];
    if (!caps.length) {
      setCommandCapability("");
      return;
    }
    if (!commandCapability || !caps.includes(commandCapability)) {
      setCommandCapability(caps[0]);
    }
  }, [selectedDevice, commandCapability, commandAction]);

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
  const latestPairing = latestSession(sessions);
  const protocol = protocolQ.data;

  const refreshAll = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["companion-connectivity"] }),
      queryClient.invalidateQueries({ queryKey: ["companion-devices"] }),
      queryClient.invalidateQueries({ queryKey: ["companion-audit"] }),
      queryClient.invalidateQueries({ queryKey: ["companion-commands"] })
    ]);
  };

  const startCompanionTunnelMutation = useMutation({
    mutationFn: api.startCompanionTunnel,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["companion-connectivity"] });
      setNotice({ kind: "success", text: "Companion tunnel is ready. Use the generated link on the selected device." });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const stopCompanionTunnelMutation = useMutation({
    mutationFn: api.stopCompanionTunnel,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["companion-connectivity"] });
      setNotice({ kind: "info", text: "Companion tunnel disabled." });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const createPairingMutation = useMutation({
    mutationFn: async () => {
      if (!selectedPreset) throw new Error("Choose a device type first.");
      const name = draftName.trim() || selectedPreset.label || "Companion device";
      const grants = draftCapabilities.length
        ? draftCapabilities
        : defaultPresetCapabilities(selectedPreset, capabilities);
      return api.createCompanionPairingSession({
        display_name: name,
        preset_id: selectedPreset.id,
        platform: selectedPreset.platform,
        capabilities: grants,
        trusted_unattested: false
      });
    },
    onSuccess: async (response) => {
      setPairingPayload(response.pairing_payload ?? null);
      setPairingStep("approve");
      setPairingDialogOpen(true);
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

  const declaredCommandRows = selectedDevice?.declared_commands ?? [];
  const activeDeclaredCommandId =
    declaredCommandRows.find(
      (command) => command.capability === commandCapability && command.action === commandAction
    )?.id ?? "";
  const commandCapabilities = selectedDevice?.token_capabilities ?? selectedDevice?.granted_capabilities ?? [];
  const commandRows = commandsQ.data?.commands ?? [];
  const presetCapabilityIds = selectedPreset?.capability_ids ?? [];
  const pairingSessionId = activePairingSessionId;
  const pairingCode = recordString(pairingPayload, "code");
  const pairingWebSocketPath =
    recordString(pairingPayload, "websocket_path") || protocol?.websocket_path || "/companion/ws";
  const pairingExpiresAt = recordString(pairingPayload, "expires_at");
  const companionTunnelWsUrl = recordString(connectivity, "websocket_url");
  const companionTunnelActive = recordBool(connectivity, "tunnel_active");
  const companionTunnelEnabled = recordBool(connectivity, "tunnel_companion_enabled");
  const companionTunnelError = recordString(connectivity, "error");
  const companionTunnelReady = companionTunnelActive && companionTunnelEnabled && Boolean(companionTunnelWsUrl);
  const recommendedCompanionWsUrl = companionTunnelWsUrl;
  const webCompanionUrl = companionWebUrl(pairingSessionId, pairingCode, recommendedCompanionWsUrl);
  const activePairingSession = pairingSessionId
    ? sessions.find((session) => session.id === pairingSessionId) ?? null
    : null;
  const activePairingStatus = activePairingSession?.status.toLowerCase() ?? "";
  const activePairingClaimed = activePairingStatus === "claimed";
  const activePairingCompleted = activePairingStatus === "completed";
  const activePairingInProgress = Boolean(pairingSessionId && !activePairingCompleted);
  const connectedPairingDevice =
    (activePairingSession?.metadata?.device_id
      ? devices.find((device) => device.id === activePairingSession.metadata?.device_id)
      : null) ??
    devices.find(
      (device) =>
        device.display_name === (activePairingSession?.display_name || draftName) &&
        device.platform === (activePairingSession?.platform || selectedPreset?.platform)
    ) ??
    null;
  const pairingStepIndex = { tunnel: 0, pairing: 1, approve: 2, connected: 3 }[pairingStep];
  const pairingStepProgress = ((pairingStepIndex + 1) / 4) * 100;
  const primaryPairingButtonLabel = activePairingInProgress
    ? "Resume pairing"
    : devices.length
      ? "Pair another"
      : "Pair device";
  const phoneDevices = devices.filter((device) => {
    const value = `${device.preset_id} ${device.platform}`.toLowerCase();
    return value.includes("ios") || value.includes("iphone") || value.includes("android");
  });
  const phoneOnlineCount = phoneDevices.filter((device) => device.state.toLowerCase() === "online").length;

  useEffect(() => {
    if (!pairingDialogOpen) return;
    if (activePairingCompleted) {
      setPairingStep("connected");
    } else if (activePairingClaimed) {
      setPairingStep("approve");
    }
  }, [activePairingClaimed, activePairingCompleted, pairingDialogOpen]);

  const openPairingDialog = () => {
    if (!selectedPreset && presets[0]) {
      setSelectedPresetId(presets[0].id);
      setDraftCapabilities(defaultPresetCapabilities(presets[0], capabilities));
      setDraftName((current) => current.trim() || presets[0].label || "Companion device");
    } else if (selectedPreset && !draftCapabilities.length) {
      setDraftCapabilities(defaultPresetCapabilities(selectedPreset, capabilities));
    }
    setPairingStep(activePairingCompleted ? "connected" : activePairingInProgress ? "approve" : "tunnel");
    setPairingDialogOpen(true);
  };

  const handleDeviceTypeChange = (nextPresetId: string) => {
    const preset = presets.find((item) => item.id === nextPresetId) ?? null;
    setSelectedPresetId(nextPresetId);
    setDraftCapabilities(defaultPresetCapabilities(preset, capabilities));
    setDraftName(preset?.label ?? "Companion device");
    setTrustedUnattested(false);
    setPairingPayload(null);
    setPairingStep("tunnel");
  };

  const startTunnelAndContinue = async () => {
    try {
      await startCompanionTunnelMutation.mutateAsync();
      setPairingStep("pairing");
    } catch {
      // Mutation onError surfaces the failure in the panel notice.
    }
  };

  const copyWebCompanionLink = async () => {
    if (!webCompanionUrl) return;
    await navigator.clipboard?.writeText(webCompanionUrl);
    setNotice({ kind: "success", text: "Web companion link copied." });
  };

  return (
    <Stack className="companion-devices-panel" spacing={1.25}>
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {rotatedToken ? (
        <Alert severity="warning" onClose={() => setRotatedToken(null)}>
          New persistent device token: <code>{rotatedToken}</code>
        </Alert>
      ) : null}

      <Box className="settings-inline-card companion-status-summary">
        <Stack
          direction={{ xs: "column", md: "row" }}
          spacing={1.25}
          sx={{ alignItems: { xs: "stretch", md: "center" }, justifyContent: "space-between" }}
        >
          <Stack direction="row" spacing={1} sx={{ alignItems: "flex-start", minWidth: 0 }}>
            <Box className="companion-pairing-start-icon">
              <PhoneIphoneRoundedIcon fontSize="small" />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <Typography className="settings-inline-card-kicker">Companion status</Typography>
              <Typography className="settings-inline-card-title">Notifications and approvals only</Typography>
              <Typography className="settings-inline-card-description">
                iPhone and Android companions do not read SMS, photos, calls, or app data.
              </Typography>
            </Box>
          </Stack>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", justifyContent: { xs: "flex-start", md: "flex-end" } }}>
            <Chip
              size="small"
              color={phoneOnlineCount > 0 ? "success" : "default"}
              variant={phoneOnlineCount > 0 ? "filled" : "outlined"}
              label={`${phoneOnlineCount} phone online`}
            />
            <Chip
              size="small"
              color={pendingApprovals.length > 0 ? "warning" : "default"}
              variant="outlined"
              label={`${pendingApprovals.length} approval${pendingApprovals.length === 1 ? "" : "s"}`}
            />
          </Stack>
        </Stack>
      </Box>

      <Box className="settings-inline-card companion-pairing-start">
        <Stack
          direction={{ xs: "column", md: "row" }}
          spacing={1.25}
          sx={{ alignItems: { xs: "stretch", md: "center" }, justifyContent: "space-between" }}
        >
          <Stack direction="row" spacing={1} sx={{ alignItems: "flex-start", minWidth: 0 }}>
            <Box className="companion-pairing-start-icon">
              <DevicesRoundedIcon fontSize="small" />
            </Box>
            <Box sx={{ minWidth: 0 }}>
              <Typography className="settings-inline-card-kicker">Companion pairing</Typography>
              <Typography className="settings-inline-card-title">Pair a notification and approval companion</Typography>
              <Typography className="settings-inline-card-description">
                Use this when AgentArk should notify your phone or ask for approvals.
              </Typography>
            </Box>
          </Stack>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", justifyContent: { xs: "flex-start", md: "flex-end" } }}>
            <Chip
              size="small"
              color={companionTunnelReady ? "success" : "default"}
              variant={companionTunnelReady ? "filled" : "outlined"}
              label={companionTunnelReady ? "Tunnel ready" : "Tunnel needed"}
            />
            <Button variant="contained" startIcon={<DevicesRoundedIcon />} onClick={openPairingDialog}>
              {primaryPairingButtonLabel}
            </Button>
          </Stack>
        </Stack>
      </Box>

      <Dialog
        open={pairingDialogOpen}
        onClose={() => setPairingDialogOpen(false)}
        fullWidth
        maxWidth="sm"
        className="companion-flow-dialog"
      >
        <DialogTitle>
          <Stack spacing={0.45}>
            <Typography variant="h6" sx={{ fontWeight: 700, lineHeight: 1.2 }}>
              Pair companion device
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              You can close this flow anytime; the session remains visible here until it expires or completes.
            </Typography>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.4}>
            <Box>
              <Stack direction="row" sx={{ justifyContent: "space-between", gap: 1, mb: 0.75 }}>
                {["Device", "Link", "Approve", "Done"].map((label, index) => (
                  <Typography
                    key={label}
                    variant="caption"
                    sx={{
                      color: index <= pairingStepIndex ? "primary.main" : "text.secondary",
                      fontWeight: index === pairingStepIndex ? 700 : 500
                    }}
                  >
                    {label}
                  </Typography>
                ))}
              </Stack>
              <LinearProgress variant="determinate" value={pairingStepProgress} />
            </Box>

            {pairingStep === "tunnel" ? (
              <Stack spacing={1.2}>
                <Box className="companion-dialog-step-grid">
                  <TextField
                    select
                    size="small"
                    label="Device type"
                    value={selectedPresetId}
                    onChange={(event) => handleDeviceTypeChange(event.target.value)}
                    disabled={!presets.length}
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
                <Alert severity="info">
                  The device must be able to open the generated AgentArk link. Same Wi-Fi is fine if the link is reachable; if your network blocks the tunnel, switch the phone to cellular or use Tailscale/custom domain access.
                </Alert>
                <Box className="companion-flow-status">
                  <Stack direction="row" spacing={1} sx={{ alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                    <Box sx={{ minWidth: 0 }}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Companion tunnel
                      </Typography>
                      <Typography className="companion-connect-url">
                        {companionTunnelReady ? companionTunnelWsUrl : "No active companion tunnel yet"}
                      </Typography>
                    </Box>
                    <Chip
                      size="small"
                      color={companionTunnelReady ? "success" : "default"}
                      variant={companionTunnelReady ? "filled" : "outlined"}
                      label={companionTunnelReady ? "Ready" : "Not ready"}
                    />
                  </Stack>
                  {companionTunnelError ? (
                    <Typography variant="caption" sx={{ color: "error.main", overflowWrap: "anywhere" }}>
                      {companionTunnelError}
                    </Typography>
                  ) : null}
                </Box>
              </Stack>
            ) : null}

            {pairingStep === "pairing" ? (
              <Stack spacing={1.2}>
                <Box className="companion-flow-status">
                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                    {selectedPreset?.label ?? "Companion device"}
                  </Typography>
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    {draftName.trim() || "Companion device"}
                  </Typography>
                  <Stack direction="row" useFlexGap sx={{ flexWrap: "wrap", gap: 0.5, mt: 1 }}>
                    {(draftCapabilities.length ? draftCapabilities : defaultPresetCapabilities(selectedPreset, capabilities)).map((id) => (
                      <Chip key={id} size="small" variant="outlined" label={labelForCapability(capabilities, id)} />
                    ))}
                    {!draftCapabilities.length && defaultPresetCapabilities(selectedPreset, capabilities).length === 0 ? (
                      <Chip size="small" variant="outlined" label="No default grants" />
                    ) : null}
                  </Stack>
                </Box>
                <Alert severity="info">
                  AgentArk will create a short-lived pairing link. Open it on the device, let it claim the session, then approve the claim here.
                </Alert>
              </Stack>
            ) : null}

            {pairingStep === "approve" ? (
              <Stack spacing={1.2}>
                <Box className="companion-flow-status">
                  <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ justifyContent: "space-between", gap: 1 }}>
                    <Box sx={{ minWidth: 0 }}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Open this link on the device
                      </Typography>
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>
                        Expires {formatUiDateTime(pairingExpiresAt, { fallback: "soon" })}
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={humanizeStatusLabel(activePairingSession?.status ?? "pending")} sx={{ alignSelf: "flex-start" }} />
                  </Stack>
                  <Typography className="companion-connect-url">{webCompanionUrl || "Create the pairing link first"}</Typography>
                </Box>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
                  <Button
                    size="small"
                    variant="contained"
                    startIcon={<ContentCopyRoundedIcon />}
                    onClick={copyWebCompanionLink}
                    disabled={!webCompanionUrl}
                  >
                    Copy link
                  </Button>
                  <Button
                    size="small"
                    variant="outlined"
                    startIcon={<LaunchRoundedIcon />}
                    onClick={() => window.open(webCompanionUrl, "_blank", "noopener,noreferrer")}
                    disabled={!webCompanionUrl}
                  >
                    Open
                  </Button>
                </Stack>
                {activePairingClaimed ? (
                  <Alert severity="success">
                    Device claimed the session. Approve it here only if the identity matches the device you just opened.
                  </Alert>
                ) : activePairingStatus === "approved" ? (
                  <Alert severity="info">Approved. Waiting for the companion page to finish and store its token.</Alert>
                ) : (
                  <Alert severity="info">Waiting for the device to open the link and claim this pairing session.</Alert>
                )}
                {activePairingSession?.claimed_device_public_key ? (
                  <Typography variant="caption" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
                    Claimed identity: {shortValue(activePairingSession.claimed_device_public_key)}
                  </Typography>
                ) : null}
              </Stack>
            ) : null}

            {pairingStep === "connected" ? (
              <Stack spacing={1.2}>
                <Alert severity="success" icon={<CheckCircleRoundedIcon />}>
                  Companion device connected.
                </Alert>
                <Box className="companion-flow-status">
                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                    {connectedPairingDevice?.display_name ?? activePairingSession?.display_name ?? draftName}
                  </Typography>
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    {connectedPairingDevice
                      ? `${connectedPairingDevice.platform} - ${connectedPairingDevice.state}`
                      : "Pairing completed"}
                  </Typography>
                </Box>
              </Stack>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setPairingDialogOpen(false)}>Close</Button>
          {pairingStep === "tunnel" ? (
            companionTunnelReady ? (
              <Button variant="contained" onClick={() => setPairingStep("pairing")}>
                Continue
              </Button>
            ) : (
              <Button
                variant="contained"
                onClick={startTunnelAndContinue}
                disabled={startCompanionTunnelMutation.isPending}
              >
                {startCompanionTunnelMutation.isPending ? "Opening tunnel..." : "Open companion tunnel"}
              </Button>
            )
          ) : null}
          {pairingStep === "pairing" ? (
            <>
              <Button onClick={() => setPairingStep("tunnel")}>Back</Button>
              <Button
                variant="contained"
                onClick={() => createPairingMutation.mutate()}
                disabled={createPairingMutation.isPending || !selectedPreset || !companionTunnelReady}
              >
                {createPairingMutation.isPending ? "Creating..." : "Create pairing link"}
              </Button>
            </>
          ) : null}
          {pairingStep === "approve" ? (
            <Button
              variant="contained"
              onClick={() => activePairingSession && approvePairingMutation.mutate(activePairingSession.id)}
              disabled={!activePairingClaimed || approvePairingMutation.isPending}
            >
              {approvePairingMutation.isPending ? "Approving..." : "Approve in AgentArk"}
            </Button>
          ) : null}
        </DialogActions>
      </Dialog>

      <Box className="settings-inline-card companion-guide">
        <Stack
          direction={{ xs: "column", md: "row" }}
          spacing={1}
          sx={{ alignItems: { xs: "stretch", md: "flex-start" }, justifyContent: "space-between" }}
        >
          <Box sx={{ minWidth: 0 }}>
            <Typography className="settings-inline-card-kicker">Connection flow</Typography>
            <Typography className="settings-inline-card-title">
              Pair a phone or helper device without giving it broad access
            </Typography>
            <Typography className="settings-inline-card-description">
              Create a short-lived code, let the device claim it, approve the claimed identity, then send only typed commands that fit the granted scopes.
            </Typography>
          </Box>
          <Chip size="small" variant="outlined" label={protocol?.protocol_version ?? "agentark-companion-v1"} />
        </Stack>
        <Box className="companion-guide-steps">
          {[
            ["1", "Create code", "Choose the device type and the grants it should receive."],
            ["2", "Claim on device", "Open the companion app and enter the session id plus code."],
            ["3", "Approve identity", "Approve only after the expected device appears as claimed."],
            ["4", "Use safely", "Low-risk commands run; high-risk commands wait in Approvals."]
          ].map(([step, title, body]) => (
            <Box key={step} className="companion-guide-step">
              <span className="companion-step-index">{step}</span>
              <Box sx={{ minWidth: 0 }}>
                <Typography className="companion-guide-step-title">{title}</Typography>
                <Typography className="companion-guide-step-body">{body}</Typography>
              </Box>
            </Box>
          ))}
        </Box>
        <Box className="companion-connect-card">
          <Box sx={{ minWidth: 0 }}>
            <Typography className="companion-guide-step-title">iPhone connection URL</Typography>
            <Typography className="companion-guide-step-body">
              Start a companion tunnel to connect from the same Wi-Fi, a VPS install, or anywhere else without editing Docker files.
            </Typography>
            <Typography className="companion-connect-url">
              {companionTunnelReady ? companionTunnelWsUrl : "Start companion tunnel to generate a wss:// URL"}
            </Typography>
            {companionTunnelError ? (
              <Typography variant="caption" sx={{ color: "error.main", overflowWrap: "anywhere" }}>
                {companionTunnelError}
              </Typography>
            ) : null}
          </Box>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", justifyContent: { xs: "flex-start", md: "flex-end" } }}>
            <Button
              size="small"
              variant="contained"
              onClick={() => startCompanionTunnelMutation.mutate()}
              disabled={startCompanionTunnelMutation.isPending}
            >
              {startCompanionTunnelMutation.isPending ? "Starting..." : companionTunnelReady ? "Refresh tunnel" : "Start companion tunnel"}
            </Button>
            <Button
              size="small"
              variant="outlined"
              onClick={async () => {
                await navigator.clipboard?.writeText(recommendedCompanionWsUrl);
                setNotice({ kind: "success", text: "Companion WebSocket URL copied." });
              }}
              disabled={!recommendedCompanionWsUrl || !companionTunnelReady}
            >
              Copy URL
            </Button>
            {companionTunnelEnabled ? (
              <Button
                size="small"
                variant="outlined"
                onClick={() => stopCompanionTunnelMutation.mutate()}
                disabled={stopCompanionTunnelMutation.isPending}
              >
                {stopCompanionTunnelMutation.isPending ? "Stopping..." : "Disable"}
              </Button>
            ) : null}
          </Stack>
        </Box>
      </Box>

      <Box className="companion-devices-grid">
        <Box className="settings-inline-card companion-panel companion-panel-pair">
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
                  bgcolor: "var(--ui-rgba-91-164-255-100)",
                  flex: "0 0 auto"
                }}
              >
                <AddLinkRoundedIcon fontSize="small" />
              </Box>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Create pairing code
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary", lineHeight: 1.45 }}>
                  Select the companion type, grant only what it needs, then send the code to that device.
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
                    Low-risk grants are selected by default. High-risk custom or desktop grants require explicit approval and audit.
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
                      bgcolor: "var(--ui-rgba-255-255-255-020)",
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
                  bgcolor: "var(--ui-rgba-255-255-255-020)",
                  minWidth: 0
                }}
              >
                <Stack direction="row" sx={{ justifyContent: "space-between", alignItems: "center", gap: 1 }}>
                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                    Enter these in the companion app
                  </Typography>
                  <Tooltip title="Copy pairing payload">
                    <IconButton size="small" onClick={copyPairingPayload} aria-label="Copy pairing payload">
                      <ContentCopyRoundedIcon fontSize="small" />
                    </IconButton>
                  </Tooltip>
                </Stack>
                <Box className="companion-pairing-fields">
                  <Box className="companion-pairing-field">
                    <Typography className="companion-pairing-label">WebSocket path</Typography>
                    <Typography className="companion-pairing-value">{pairingWebSocketPath}</Typography>
                  </Box>
                  <Box className="companion-pairing-field">
                    <Typography className="companion-pairing-label">Session id</Typography>
                    <Typography className="companion-pairing-value">{pairingSessionId || "unknown"}</Typography>
                  </Box>
                  <Box className="companion-pairing-field">
                    <Typography className="companion-pairing-label">Pairing code</Typography>
                    <Typography className="companion-pairing-value">{pairingCode || "unknown"}</Typography>
                  </Box>
                  <Box className="companion-pairing-field">
                    <Typography className="companion-pairing-label">Expires</Typography>
                    <Typography className="companion-pairing-value">
                      {formatUiDateTime(pairingExpiresAt, { fallback: "unknown" })}
                    </Typography>
                  </Box>
                </Box>
                <Typography variant="caption" sx={{ display: "block", color: "text.secondary", mt: 1 }}>
                  No Xcode path: open the web companion link in iPhone Safari, tap Claim pairing, then approve the claimed device here.
                </Typography>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", mt: 1 }}>
                  <Button
                    size="small"
                    variant="contained"
                    onClick={async () => {
                      await navigator.clipboard?.writeText(webCompanionUrl);
                      setNotice({ kind: "success", text: "Web companion link copied. Open it on the iPhone." });
                    }}
                    disabled={!webCompanionUrl}
                  >
                    Copy Web Companion Link
                  </Button>
                  <Button
                    size="small"
                    variant="outlined"
                    onClick={() => window.open(webCompanionUrl, "_blank", "noopener,noreferrer")}
                    disabled={!webCompanionUrl}
                  >
                    Open Web Companion
                  </Button>
                </Stack>
                {webCompanionUrl ? (
                  <Typography className="companion-connect-url" sx={{ mt: 1 }}>
                    {webCompanionUrl}
                  </Typography>
                ) : null}
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
                  This is a one-time secret. Approve only after the expected device claims this exact session.
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
                        Latest session - {humanizeStatusLabel(latestPairing.status)}
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={humanizeStatusLabel(latestPairing.status)} />
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

        <Box className="settings-inline-card companion-panel companion-panel-devices">
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
                  Paired companions and last seen status.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label={`${devices.length} device${devices.length === 1 ? "" : "s"}`} />
            </Stack>
            {devicesQ.error ? <Alert severity="error">{errMessage(devicesQ.error)}</Alert> : null}
            {devices.length === 0 ? (
              <Alert severity="info">
                No companion devices are paired yet. Create a pairing code, enter it in the companion app, then approve the claimed device here.
              </Alert>
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
                        boxShadow: selected ? "0 0 0 1px var(--ui-rgba-91-164-255-180)" : undefined,
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

        <Accordion className="companion-advanced" disableGutters defaultExpanded={pendingApprovals.length > 0}>
          <AccordionSummary expandIcon={<ExpandMoreRoundedIcon />}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={0.75}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between", width: "100%" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                  Advanced
                </Typography>
                <Typography variant="caption" sx={{ color: "text.secondary" }}>
                  Command testing, pending approvals, history, and audit.
                </Typography>
              </Box>
              {pendingApprovals.length > 0 ? (
                <Chip size="small" color="warning" label={`${pendingApprovals.length} waiting`} />
              ) : null}
            </Stack>
          </AccordionSummary>
          <AccordionDetails>
            <Stack className="companion-advanced-stack" spacing={1.25}>
        <Box className="settings-inline-card companion-panel companion-panel-command">
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Command test
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Send a declared command to the selected device.
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
                {declaredCommandRows.length ? (
                  <TextField
                    select
                    size="small"
                    label="Command"
                    value={activeDeclaredCommandId}
                    onChange={(event) => {
                      const next = declaredCommandRows.find((command) => command.id === event.target.value);
                      if (!next) return;
                      setCommandCapability(next.capability);
                      setCommandAction(next.action);
                    }}
                    helperText="Commands come from the selected companion's latest declaration."
                  >
                    {declaredCommandRows.map((command) => (
                      <MenuItem key={command.id} value={command.id}>
                        {command.label || command.action}
                      </MenuItem>
                    ))}
                  </TextField>
                ) : null}
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
                {declaredCommandRows.length ? (
                  <Chip size="small" variant="outlined" label={commandAction || "No command selected"} sx={{ alignSelf: "flex-start" }} />
                ) : (
                  <TextField
                    size="small"
                    label="Action id"
                    value={commandAction}
                    onChange={(event) => setCommandAction(event.target.value)}
                    helperText="Use a declared adapter action id such as notifications.show."
                  />
                )}
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
              <Alert severity="info">
                Select a paired device first. Commands are structured JSON actions, not free-form remote control.
              </Alert>
            )}
          </Stack>
        </Box>

        <Box className="settings-inline-card companion-panel companion-panel-approvals">
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
              <Alert severity="info">
                No high-risk companion commands are waiting. Sensitive commands will appear here before they can run.
              </Alert>
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
        <Box className="settings-inline-card companion-panel companion-panel-history">
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Command History
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Recent queued actions and returned results.
                </Typography>
              </Box>
              {selectedDevice ? <Chip size="small" variant="outlined" label={`${commandRows.length} command${commandRows.length === 1 ? "" : "s"}`} /> : null}
            </Stack>
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
                            {humanizeMachineLabel(command.action)}
                          </Typography>
                          <Chip size="small" variant="outlined" label={humanizeStatusLabel(command.status)} sx={{ alignSelf: "flex-start" }} />
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
                <Alert severity="info">No commands for the selected device yet. Queue a typed command to test dispatch.</Alert>
              )
            ) : (
              <Alert severity="info">Select a device to see the commands AgentArk has sent to it.</Alert>
            )}
          </Stack>
        </Box>

        <Box className="settings-inline-card companion-panel companion-panel-audit">
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{ alignItems: { xs: "stretch", sm: "center" }, justifyContent: "space-between" }}
            >
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="h6" sx={{ fontWeight: 650, lineHeight: 1.2 }}>
                  Audit
                </Typography>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  Decisions, command approvals, and event hashes.
                </Typography>
              </Box>
              <Chip size="small" variant="outlined" label={`${(auditQ.data?.events ?? []).length} event${(auditQ.data?.events ?? []).length === 1 ? "" : "s"}`} />
            </Stack>
            {(auditQ.data?.events ?? []).length === 0 ? (
              <Alert severity="info">
                No companion-device audit events yet. Pairing, approvals, token changes, and command decisions will be recorded here.
              </Alert>
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
            </Stack>
          </AccordionDetails>
        </Accordion>
      </Box>
    </Stack>
  );
}
