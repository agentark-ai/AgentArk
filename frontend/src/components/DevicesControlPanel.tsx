import AddLinkRoundedIcon from "@mui/icons-material/AddLinkRounded";
import DevicesRoundedIcon from "@mui/icons-material/DevicesRounded";
import LocationOnRoundedIcon from "@mui/icons-material/LocationOnRounded";
import MonitorRoundedIcon from "@mui/icons-material/MonitorRounded";
import NotificationsActiveRoundedIcon from "@mui/icons-material/NotificationsActiveRounded";
import PhoneAndroidRoundedIcon from "@mui/icons-material/PhoneAndroidRounded";
import ScreenshotMonitorRoundedIcon from "@mui/icons-material/ScreenshotMonitorRounded";
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Grid2,
  MenuItem,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import { useMemo, useState, type JSX } from "react";
import { formatUiDateTime } from "../lib/dateFormat";

export type DeviceCapability =
  | "camera"
  | "screen"
  | "screen_recording"
  | "location"
  | "sms"
  | "notifications"
  | "remote_run"
  | string;

export type DeviceNode = {
  id: string;
  name: string;
  platform: string;
  status: "online" | "idle" | "busy" | "pairing" | "offline" | "error" | string;
  capabilities: DeviceCapability[];
  last_seen_at?: string;
  paired_at?: string;
  location?: string;
  model?: string;
  owner?: string;
  detail?: string;
};

export type DevicesControlPanelProps = {
  nodes: DeviceNode[];
  selectedNodeId?: string | null;
  onSelectNode?: (nodeId: string) => void;
  onPairNode?: (payload: { name: string; platform: string; capabilities: DeviceCapability[] }) => void | Promise<void>;
  onUnpairNode?: (nodeId: string) => void | Promise<void>;
  onSendCommand?: (nodeId: string, command: string) => void | Promise<void>;
  onRefreshNode?: (nodeId: string) => void | Promise<void>;
  className?: string;
};

const DEVICE_CAPABILITY_OPTIONS: Array<{ value: DeviceCapability; label: string }> = [
  { value: "camera", label: "Camera" },
  { value: "screen", label: "Screen capture" },
  { value: "screen_recording", label: "Screen recording" },
  { value: "location", label: "Location" },
  { value: "sms", label: "SMS" },
  { value: "notifications", label: "Notifications" },
  { value: "remote_run", label: "Remote run" },
];

function statusTone(status: DeviceNode["status"]): "success" | "warning" | "error" | "info" | "default" {
  const value = String(status || "").toLowerCase();
  if (value === "online") return "success";
  if (value === "idle") return "info";
  if (value === "busy" || value === "pairing") return "warning";
  if (value === "error" || value === "offline") return "error";
  return "default";
}

function statusLabel(status: DeviceNode["status"]): string {
  const value = String(status || "").toLowerCase();
  if (value === "online") return "Online";
  if (value === "idle") return "Idle";
  if (value === "busy") return "Busy";
  if (value === "pairing") return "Pairing";
  if (value === "offline") return "Offline";
  if (value === "error") return "Error";
  return status || "Unknown";
}

function capabilityIcon(kind: DeviceCapability): JSX.Element {
  const value = String(kind || "").toLowerCase();
  if (value.includes("camera")) return <MonitorRoundedIcon fontSize="small" />;
  if (value.includes("screen")) return <ScreenshotMonitorRoundedIcon fontSize="small" />;
  if (value.includes("location")) return <LocationOnRoundedIcon fontSize="small" />;
  if (value.includes("sms")) return <PhoneAndroidRoundedIcon fontSize="small" />;
  if (value.includes("notification")) return <NotificationsActiveRoundedIcon fontSize="small" />;
  return <DevicesRoundedIcon fontSize="small" />;
}

function capabilityLabel(kind: DeviceCapability): string {
  const normalized = String(kind || "").trim().toLowerCase();
  const preset = DEVICE_CAPABILITY_OPTIONS.find((option) => option.value === normalized);
  if (preset) return preset.label;
  return normalized
    .split(/[_\s-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatDate(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "Never" });
}

export function DevicesControlPanel({
  nodes,
  selectedNodeId,
  onSelectNode,
  onPairNode,
  onUnpairNode,
  onSendCommand,
  onRefreshNode,
  className
}: DevicesControlPanelProps) {
  const selected = nodes.find((node) => node.id === selectedNodeId) ?? nodes[0] ?? null;
  const [draftName, setDraftName] = useState("");
  const [draftPlatform, setDraftPlatform] = useState("android");
  const [draftCapabilities, setDraftCapabilities] = useState<DeviceCapability[]>(["notifications"]);
  const [command, setCommand] = useState("");

  const stats = useMemo(() => {
    const online = nodes.filter((node) => String(node.status).toLowerCase() === "online").length;
    const pairing = nodes.filter((node) => String(node.status).toLowerCase() === "pairing").length;
    const capable = nodes.filter((node) => (node.capabilities || []).length > 0).length;
    return { online, pairing, capable };
  }, [nodes]);

  return (
    <Box className={className}>
      <Stack spacing={1.25}>
        <Box>
          <Typography variant="overline" className="workspace-shell-kicker">
            Devices
          </Typography>
          <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
            Companion nodes and capability grants
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 840 }}>
            Use this panel for paired devices, capability scope, and the operational state of each node.
          </Typography>
        </Box>

        <Grid2 container spacing={1.25}>
          <Grid2 size={{ xs: 12, sm: 4 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center">
                  <Box>
                    <Typography variant="body2" color="text.secondary">
                      Online
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.online}
                    </Typography>
                  </Box>
                  <DevicesRoundedIcon fontSize="small" />
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
          <Grid2 size={{ xs: 12, sm: 4 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center">
                  <Box>
                    <Typography variant="body2" color="text.secondary">
                      Pairing
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.pairing}
                    </Typography>
                  </Box>
                  <AddLinkRoundedIcon fontSize="small" />
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
          <Grid2 size={{ xs: 12, sm: 4 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack direction="row" justifyContent="space-between" alignItems="center">
                  <Box>
                    <Typography variant="body2" color="text.secondary">
                      Capability-ready
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.capable}
                    </Typography>
                  </Box>
                  <NotificationsActiveRoundedIcon fontSize="small" />
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
        </Grid2>

        <Grid2 container spacing={1.25}>
          <Grid2 size={{ xs: 12, lg: 7 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack spacing={1.2}>
                  <Stack direction="row" justifyContent="space-between" alignItems="center" gap={1}>
                    <Box>
                      <Typography variant="h6" sx={{ fontWeight: 650 }}>
                        Node inventory
                      </Typography>
                      <Typography variant="body2" color="text.secondary">
                        Operational state for paired companion devices.
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={`${nodes.length} nodes`} />
                  </Stack>
                  <Divider />

                  {nodes.length === 0 ? (
                    <Box sx={{ py: 4 }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                        No devices paired yet
                      </Typography>
                      <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 560 }}>
                        Pair a node to expose camera, screen, location, SMS, notifications, or remote run capabilities.
                      </Typography>
                    </Box>
                  ) : (
                    <Stack spacing={0.85}>
                      {nodes.map((node) => {
                        const selectedState = node.id === selected?.id;
                        return (
                          <Box
                            key={node.id}
                            className="action-row"
                            onClick={() => onSelectNode?.(node.id)}
                            role="button"
                            tabIndex={0}
                            sx={{
                              cursor: "pointer",
                              borderColor: selectedState ? "rgba(47,212,255,0.48)" : undefined,
                              background: selectedState ? "rgba(47,212,255,0.06)" : undefined
                            }}
                          >
                            <Stack spacing={0.75} sx={{ width: "100%" }}>
                              <Stack direction="row" justifyContent="space-between" alignItems="center" gap={1}>
                                <Box sx={{ minWidth: 0 }}>
                                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }} noWrap>
                                    {node.name}
                                  </Typography>
                                  <Typography variant="caption" color="text.secondary" noWrap>
                                    {node.platform} {node.model ? `| ${node.model}` : ""}
                                  </Typography>
                                </Box>
                                <Chip size="small" color={statusTone(node.status)} label={statusLabel(node.status)} />
                              </Stack>
                              <Typography variant="body2" color="text.secondary">
                                {node.detail || node.location || "No node detail supplied yet."}
                              </Typography>
                              <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                                {(node.capabilities || []).slice(0, 4).map((capability) => (
                                  <Chip
                                    key={capability}
                                    size="small"
                                    variant="outlined"
                                    icon={capabilityIcon(capability)}
                                    label={capabilityLabel(capability)}
                                  />
                                ))}
                              </Stack>
                            </Stack>
                          </Box>
                        );
                      })}
                    </Stack>
                  )}
                </Stack>
              </CardContent>
            </Card>
          </Grid2>

          <Grid2 size={{ xs: 12, lg: 5 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                {selected ? (
                  <Stack spacing={1.2}>
                    <Box>
                      <Typography variant="h6" sx={{ fontWeight: 650 }}>
                        Node details
                      </Typography>
                      <Typography variant="body2" color="text.secondary">
                        Manage pairing and command execution for the selected node.
                      </Typography>
                    </Box>

                    <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                      <Chip size="small" variant="outlined" label={selected.platform} />
                      <Chip size="small" color={statusTone(selected.status)} label={statusLabel(selected.status)} />
                      {selected.owner ? <Chip size="small" variant="outlined" label={selected.owner} /> : null}
                    </Stack>

                    <Typography variant="body2" color="text.secondary">
                      Last seen {formatDate(selected.last_seen_at)}
                    </Typography>

                    <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                      <Button variant="contained" size="small" onClick={() => onRefreshNode?.(selected.id)}>
                        Refresh
                      </Button>
                      <Button variant="outlined" size="small" onClick={() => onUnpairNode?.(selected.id)}>
                        Unpair
                      </Button>
                    </Stack>

                    <Divider />

                    <Stack spacing={1}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Send command
                      </Typography>
                      <TextField
                        label="Command"
                        size="small"
                        fullWidth
                        value={command}
                        onChange={(event) => setCommand(event.target.value)}
                        placeholder="capture screen, send location, run system command"
                      />
                      <Button
                        variant="outlined"
                        onClick={() => {
                          const value = command.trim();
                          if (!value) return;
                          onSendCommand?.(selected.id, value);
                          setCommand("");
                        }}
                      >
                        Send
                      </Button>
                    </Stack>
                  </Stack>
                ) : (
                  <Box sx={{ py: 4 }}>
                    <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                      No node selected
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                      Select a device to inspect its capabilities and operational state.
                    </Typography>
                  </Box>
                )}
              </CardContent>
            </Card>

            <Card className="workspace-side-card" sx={{ mt: 1.25 }}>
              <CardContent sx={{ p: 1.5 }}>
                <Stack spacing={1.1}>
                  <Typography variant="h6" sx={{ fontWeight: 650 }}>
                    Pair node
                  </Typography>
                  <Typography variant="body2" color="text.secondary">
                    This currently registers a device record in AgentArk. A real iPhone/macOS pairing
                    handshake is not implemented yet.
                  </Typography>
                  <TextField label="Node name" size="small" value={draftName} onChange={(event) => setDraftName(event.target.value)} />
                  <TextField
                    select
                    label="Platform"
                    size="small"
                    value={draftPlatform}
                    onChange={(event) => setDraftPlatform(event.target.value)}
                  >
                    <MenuItem value="android">Android</MenuItem>
                    <MenuItem value="ios">iOS</MenuItem>
                    <MenuItem value="macos">macOS</MenuItem>
                    <MenuItem value="headless">Headless</MenuItem>
                    <MenuItem value="desktop">Desktop</MenuItem>
                  </TextField>
                  <TextField
                    select
                    label="Capabilities"
                    size="small"
                    value={draftCapabilities}
                    onChange={(event) =>
                      setDraftCapabilities(
                        (Array.isArray(event.target.value)
                          ? event.target.value
                          : [event.target.value]
                        )
                          .map((value) => String(value).trim())
                          .filter(Boolean)
                      )
                    }
                    helperText="Choose the capabilities this companion device would expose."
                    SelectProps={{
                      multiple: true,
                      renderValue: (selected) => (
                        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5 }}>
                          {(selected as string[]).map((value) => (
                            <Chip key={value} size="small" label={capabilityLabel(value)} />
                          ))}
                        </Box>
                      )
                    }}
                  >
                    {DEVICE_CAPABILITY_OPTIONS.map((option) => (
                      <MenuItem key={option.value} value={option.value}>
                        {option.label}
                      </MenuItem>
                    ))}
                  </TextField>
                  <Typography variant="caption" color="text.secondary">
                    Capabilities are metadata right now. They do not activate real camera, SMS, or remote-run access by
                    themselves.
                  </Typography>
                  <Button
                    variant="contained"
                    onClick={() => {
                      const name = draftName.trim();
                      if (!name) return;
                      onPairNode?.({ name, platform: draftPlatform, capabilities: draftCapabilities });
                      setDraftName("");
                    }}
                  >
                    Pair node
                  </Button>
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
        </Grid2>
      </Stack>
    </Box>
  );
}
