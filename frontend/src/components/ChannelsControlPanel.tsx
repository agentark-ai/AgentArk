import AddLinkRoundedIcon from "@mui/icons-material/AddLinkRounded";
import HubRoundedIcon from "@mui/icons-material/HubRounded";
import LinkRoundedIcon from "@mui/icons-material/LinkRounded";
import SignalCellularAltRoundedIcon from "@mui/icons-material/SignalCellularAltRounded";
import StorageRoundedIcon from "@mui/icons-material/StorageRounded";
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { formatUiDateTime } from "../lib/dateFormat";

export type ChannelConnectionState =
  | "connected"
  | "ready"
  | "connecting"
  | "syncing"
  | "disabled"
  | "missing_config"
  | "missing_token"
  | "error"
  | string;

export type ChannelItem = {
  id: string;
  name: string;
  kind: string;
  description?: string;
  status: ChannelConnectionState;
  enabled?: boolean;
  route_scope?: string;
  last_activity_at?: string;
  unread_count?: number;
  paired_with?: string;
  detail?: string;
};

export type ChannelCapability = {
  id: string;
  label: string;
  detail?: string;
};

export type ChannelsControlPanelProps = {
  channels: ChannelItem[];
  selectedChannelId?: string | null;
  capabilities?: ChannelCapability[];
  onSelectChannel?: (channelId: string) => void;
  onConnectChannel?: (channelId: string) => void | Promise<void>;
  onDisconnectChannel?: (channelId: string) => void | Promise<void>;
  onOpenWizard?: (channelId: string) => void;
  onSubmitQuickNote?: (channelId: string, note: string) => void | Promise<void>;
  className?: string;
};

function statusTone(status: ChannelConnectionState): "success" | "warning" | "error" | "info" | "default" {
  const value = String(status || "").toLowerCase();
  if (value === "connected" || value === "ready") return "success";
  if (value === "connecting" || value === "syncing") return "info";
  if (value === "missing_config" || value === "missing_token" || value === "disabled") return "warning";
  if (value === "error") return "error";
  return "default";
}

function statusLabel(status: ChannelConnectionState): string {
  const value = String(status || "").toLowerCase();
  if (value === "connected") return "Connected";
  if (value === "ready") return "Ready";
  if (value === "connecting") return "Connecting";
  if (value === "syncing") return "Syncing";
  if (value === "disabled") return "Disabled";
  if (value === "missing_config") return "Missing config";
  if (value === "missing_token") return "Missing token";
  if (value === "error") return "Error";
  return status || "Unknown";
}

function statusHint(channel: ChannelItem): string {
  if (channel.detail && channel.detail.trim()) return channel.detail.trim();
  if (!channel.enabled) return "Channel is available but disabled for agent dispatch.";
  if (String(channel.status).toLowerCase() === "missing_token") return "Add credentials before enabling delivery.";
  if (String(channel.status).toLowerCase() === "missing_config") return "Finish setup to make this route available.";
  return channel.description || "No additional details available.";
}

function formatDate(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "Never" });
}

function overviewCount(channels: ChannelItem[], predicate: (item: ChannelItem) => boolean): number {
  return channels.filter(predicate).length;
}

export function ChannelsControlPanel({
  channels,
  selectedChannelId,
  capabilities = [],
  onSelectChannel,
  onConnectChannel,
  onDisconnectChannel,
  onOpenWizard,
  onSubmitQuickNote,
  className
}: ChannelsControlPanelProps) {
  const selected = channels.find((channel) => channel.id === selectedChannelId) ?? channels[0] ?? null;
  const connectedCount = overviewCount(channels, (channel) => {
    const status = String(channel.status || "").toLowerCase();
    return status === "connected" || status === "ready";
  });
  const needsSetupCount = overviewCount(channels, (channel) => {
    const status = String(channel.status || "").toLowerCase();
    return status === "missing_config" || status === "missing_token";
  });
  const disabledCount = overviewCount(channels, (channel) => !channel.enabled);

  return (
    <Box className={className}>
      <Stack spacing={1.25}>
        <Box>
          <Typography variant="overline" className="workspace-shell-kicker">
            Channels
          </Typography>
          <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
            Channel routing and delivery readiness
          </Typography>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              maxWidth: 780
            }}>
            Keep channel connections visible, route-specific, and recoverable without leaving the OS workspace.
          </Typography>
        </Box>

        <Grid2 container spacing={1.25}>
          <Grid2 size={{ xs: 12, sm: 4 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Box>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Connected
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {connectedCount}
                    </Typography>
                  </Box>
                  <SignalCellularAltRoundedIcon fontSize="small" />
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
          <Grid2 size={{ xs: 12, sm: 4 }}>
            <Card className="workspace-side-card">
              <CardContent sx={{ p: 1.5 }}>
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Box>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Need setup
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {needsSetupCount}
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
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center"
                  }}>
                  <Box>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Disabled
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {disabledCount}
                    </Typography>
                  </Box>
                  <StorageRoundedIcon fontSize="small" />
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
                  <Stack
                    direction="row"
                    sx={{
                      justifyContent: "space-between",
                      alignItems: "center",
                      gap: 1
                    }}>
                    <Box>
                      <Typography variant="h6" sx={{ fontWeight: 650 }}>
                        Channel inventory
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Select a channel to inspect connection state, routing scope, and actions.
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={`${channels.length} channels`} />
                  </Stack>

                  <Divider />

                  {channels.length === 0 ? (
                    <Box sx={{ py: 4 }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                        No channels configured yet
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                          maxWidth: 560,
                          mb: 1.5
                        }}>
                        Add a channel to begin routing inbound messages and outbound delivery through the gateway.
                      </Typography>
                      <Button variant="contained" startIcon={<LinkRoundedIcon />} onClick={() => onOpenWizard?.("")}>
                        Add channel
                      </Button>
                    </Box>
                  ) : (
                    <Stack spacing={0.85}>
                      {channels.map((channel) => {
                        const selectedState = channel.id === selected?.id;
                        return (
                          <Box
                            key={channel.id}
                            className="action-row"
                            onClick={() => onSelectChannel?.(channel.id)}
                            role="button"
                            tabIndex={0}
                            sx={{
                              cursor: "pointer",
                              borderColor: selectedState ? "var(--ui-rgba-47-212-255-480)" : undefined,
                              background: selectedState ? "var(--ui-rgba-47-212-255-060)" : undefined
                            }}
                          >
                            <Stack spacing={0.75} sx={{ width: "100%" }}>
                              <Stack
                                direction="row"
                                sx={{
                                  justifyContent: "space-between",
                                  alignItems: "center",
                                  gap: 1
                                }}>
                                <Box sx={{ minWidth: 0 }}>
                                  <Typography variant="subtitle2" sx={{ fontWeight: 650 }} noWrap>
                                    {channel.name}
                                  </Typography>
                                  <Typography variant="caption" noWrap sx={{
                                    color: "text.secondary"
                                  }}>
                                    {channel.kind} {channel.route_scope ? `| ${channel.route_scope}` : ""}
                                  </Typography>
                                </Box>
                                <Stack
                                  direction="row"
                                  spacing={0.75}
                                  useFlexGap
                                  sx={{
                                    alignItems: "center",
                                    flexWrap: "wrap"
                                  }}>
                                  {typeof channel.unread_count === "number" && channel.unread_count > 0 ? (
                                    <Chip size="small" color="info" label={`${channel.unread_count} unread`} />
                                  ) : null}
                                  <Chip size="small" color={statusTone(channel.status)} label={statusLabel(channel.status)} />
                                </Stack>
                              </Stack>
                              <Typography variant="body2" sx={{
                                color: "text.secondary"
                              }}>
                                {statusHint(channel)}
                              </Typography>
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
                        Channel details
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Quick actions for the selected channel.
                      </Typography>
                    </Box>

                    <Stack direction="row" spacing={0.75} useFlexGap sx={{
                      flexWrap: "wrap"
                    }}>
                      <Chip size="small" variant="outlined" label={selected.kind} />
                      <Chip size="small" color={statusTone(selected.status)} label={statusLabel(selected.status)} />
                      {selected.route_scope ? <Chip size="small" variant="outlined" label={selected.route_scope} /> : null}
                    </Stack>

                    <Box>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        {statusHint(selected)}
                      </Typography>
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Last activity {formatDate(selected.last_activity_at)}
                      </Typography>
                    </Box>

                    <Stack direction="row" spacing={0.75} useFlexGap sx={{
                      flexWrap: "wrap"
                    }}>
                      <Button variant="contained" size="small" onClick={() => onConnectChannel?.(selected.id)}>
                        Connect
                      </Button>
                      <Button variant="outlined" size="small" onClick={() => onDisconnectChannel?.(selected.id)}>
                        Disconnect
                      </Button>
                      <Button variant="text" size="small" onClick={() => onOpenWizard?.(selected.id)}>
                        Open setup
                      </Button>
                    </Stack>

                    <Divider />

                    <Stack spacing={0.75}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Capability hints
                      </Typography>
                      {capabilities.length === 0 ? (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No channel capabilities supplied yet.
                        </Typography>
                      ) : (
                        capabilities.map((capability) => (
                          <Box key={capability.id} className="action-row">
                            <Stack spacing={0.25}>
                              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                                {capability.label}
                              </Typography>
                              {capability.detail ? (
                                <Typography variant="caption" sx={{
                                  color: "text.secondary"
                                }}>
                                  {capability.detail}
                                </Typography>
                              ) : null}
                            </Stack>
                          </Box>
                        ))
                      )}
                    </Stack>

                    <Divider />

                    <Stack spacing={1}>
                      <TextField
                        label="Quick note"
                        size="small"
                        fullWidth
                        placeholder="Leave a setup note or route hint"
                        onKeyDown={(event) => {
                          if (event.key !== "Enter") return;
                          const target = event.target as HTMLInputElement | HTMLTextAreaElement;
                          const value = target.value.trim();
                          if (!value) return;
                          onSubmitQuickNote?.(selected.id, value);
                          target.value = "";
                        }}
                      />
                      <Typography variant="caption" sx={{
                        color: "text.secondary"
                      }}>
                        Press Enter to submit a note for the selected channel.
                      </Typography>
                    </Stack>

                    <Box>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650, mb: 0.5 }}>
                        Delivery posture
                      </Typography>
                      <Stack direction="row" spacing={0.75} useFlexGap sx={{
                        flexWrap: "wrap"
                      }}>
                        <Chip size="small" icon={<HubRoundedIcon />} label={selected.enabled ? "Enabled" : "Disabled"} />
                        {selected.paired_with ? <Chip size="small" label={`Paired with ${selected.paired_with}`} /> : null}
                      </Stack>
                    </Box>
                  </Stack>
                ) : (
                  <Box sx={{ py: 4 }}>
                    <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                      No channel selected
                    </Typography>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Select a channel from the list to inspect connection state and actions.
                    </Typography>
                  </Box>
                )}
              </CardContent>
            </Card>
          </Grid2>
        </Grid2>
      </Stack>
    </Box>
  );
}



