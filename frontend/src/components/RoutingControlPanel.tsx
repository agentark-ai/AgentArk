import AddRoundedIcon from "@mui/icons-material/AddRounded";
import GroupRoundedIcon from "@mui/icons-material/GroupRounded";
import RouteRoundedIcon from "@mui/icons-material/RouteRounded";
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Grid as Grid2,
  MenuItem,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import { useMemo, useState } from "react";
import { formatUiDateTime } from "../lib/dateFormat";

export type RouteTarget = {
  id: string;
  label: string;
  kind?: string;
  detail?: string;
};

export type RouteRule = {
  id: string;
  name: string;
  match: string;
  scope: "per_channel" | "global" | string;
  route_to?: string;
  channel?: string;
  agent?: string;
  broadcast_group?: string;
  enabled: boolean;
  priority?: number;
  last_matched_at?: string;
  description?: string;
};

export type BroadcastGroup = {
  id: string;
  name: string;
  members: string[];
  description?: string;
};

export type RoutingControlPanelProps = {
  routes: RouteRule[];
  targets: RouteTarget[];
  broadcastGroups?: BroadcastGroup[];
  selectedRouteId?: string | null;
  onSelectRoute?: (routeId: string) => void;
  onToggleRoute?: (routeId: string, enabled: boolean) => void | Promise<void>;
  onSaveRoute?: (route: Partial<RouteRule>) => void | Promise<void>;
  onDeleteRoute?: (routeId: string) => void | Promise<void>;
  onCreateGroup?: (name: string) => void | Promise<void>;
  className?: string;
};

function formatDate(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "Never" });
}

function scopeLabel(scope: RouteRule["scope"]): string {
  if (scope === "per_channel") return "Per channel";
  if (scope === "global") return "Global";
  return scope || "Custom";
}

export function RoutingControlPanel({
  routes,
  targets,
  broadcastGroups = [],
  selectedRouteId,
  onSelectRoute,
  onToggleRoute,
  onSaveRoute,
  onDeleteRoute,
  onCreateGroup,
  className
}: RoutingControlPanelProps) {
  const selected = routes.find((route) => route.id === selectedRouteId) ?? routes[0] ?? null;
  const [draftName, setDraftName] = useState("");
  const [draftMatch, setDraftMatch] = useState("");
  const [draftScope, setDraftScope] = useState<RouteRule["scope"]>("per_channel");
  const [draftTarget, setDraftTarget] = useState("");
  const [draftGroup, setDraftGroup] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const routeStats = useMemo(() => {
    const enabled = routes.filter((route) => route.enabled).length;
    const global = routes.filter((route) => route.scope === "global").length;
    const channelScoped = routes.filter((route) => route.scope === "per_channel").length;
    return { enabled, global, channelScoped };
  }, [routes]);

  return (
    <Box className={className}>
      <Stack spacing={1.25}>
        <Box>
          <Typography variant="overline" className="workspace-shell-kicker">
            Conversation Routing
          </Typography>
          <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
            Keep the right channel tied to the right agent
          </Typography>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              maxWidth: 840
            }}>
            Use this only when messages from a specific channel, account, or thread should always go to the same agent
            or broadcast target. Most users can leave this empty.
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
                      Active routes
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {routeStats.enabled}
                    </Typography>
                  </Box>
                  <RouteRoundedIcon fontSize="small" />
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
                      Channel scoped
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {routeStats.channelScoped}
                    </Typography>
                  </Box>
                  <GroupRoundedIcon fontSize="small" />
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
                      Global routes
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {routeStats.global}
                    </Typography>
                  </Box>
                  <AddRoundedIcon fontSize="small" />
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
                        Routing rules
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Visible rules with explicit targets and scope.
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={`${routes.length} rules`} />
                  </Stack>
                  <Divider />

                  {routes.length === 0 ? (
                    <Box sx={{ py: 4 }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                        No routes defined
                      </Typography>
                      <Typography
                        variant="body2"
                        sx={{
                          color: "text.secondary",
                          maxWidth: 560,
                          mb: 1.5
                        }}>
                        Add a rule only if you need a channel, account, or broadcast group to always use the same agent.
                      </Typography>
                    </Box>
                  ) : (
                    <Stack spacing={0.85}>
                      {routes.map((route) => {
                        const selectedState = route.id === selected?.id;
                        return (
                          <Box
                            key={route.id}
                            className="action-row"
                            onClick={() => onSelectRoute?.(route.id)}
                            role="button"
                            tabIndex={0}
                            sx={{
                              cursor: "pointer",
                              borderColor: selectedState ? "rgba(47,212,255,0.48)" : undefined,
                              background: selectedState ? "rgba(47,212,255,0.06)" : undefined
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
                                    {route.name}
                                  </Typography>
                                  <Typography variant="caption" noWrap sx={{
                                    color: "text.secondary"
                                  }}>
                                    {route.match}
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
                                  <Chip
                                    size="small"
                                    color={route.enabled ? "success" : "default"}
                                    label={route.enabled ? "Enabled" : "Disabled"}
                                  />
                                  <Chip size="small" variant="outlined" label={scopeLabel(route.scope)} />
                                </Stack>
                              </Stack>
                              <Typography variant="body2" sx={{
                                color: "text.secondary"
                              }}>
                                {route.description || "No description provided."}
                              </Typography>
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>
                                Target {route.route_to || route.agent || route.broadcast_group || route.channel || "unset"} | Last
                                matched {formatDate(route.last_matched_at)}
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
                {!advancedOpen ? (
                  <Stack spacing={1.2}>
                    <Box>
                      <Typography variant="h6" sx={{ fontWeight: 650 }}>
                        When you actually need this
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Leave routing empty unless a specific channel, account, or thread must always use the same
                        agent.
                      </Typography>
                    </Box>
                    <Stack spacing={0.75}>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Example: send Slack support messages to a Support Agent.
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Example: keep Discord moderation traffic with a Moderation Agent.
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Example: fan out urgent alerts to more than one agent with a broadcast group.
                      </Typography>
                    </Stack>
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      This is an expert/admin tool. Most workspaces do not need any rules here.
                    </Typography>
                    <Button variant="outlined" onClick={() => setAdvancedOpen(true)}>
                      Show advanced routing tools
                    </Button>
                  </Stack>
                ) : (
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
                          Advanced routing tools
                        </Typography>
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          Build a rule or edit the selected one.
                        </Typography>
                      </Box>
                      <Button size="small" variant="text" onClick={() => setAdvancedOpen(false)}>
                        Hide
                      </Button>
                    </Stack>

                    {selected ? (
                      <Stack spacing={1}>
                        <TextField
                          label="Rule name"
                          size="small"
                          value={selected.name}
                          onChange={(event) => onSaveRoute?.({ id: selected.id, name: event.target.value })}
                        />
                        <TextField
                          label="When this applies"
                          size="small"
                          value={selected.match}
                          onChange={(event) => onSaveRoute?.({ id: selected.id, match: event.target.value })}
                          helperText="Examples: channel:web, guild:team-a, thread:12345"
                        />
                        <TextField
                          select
                          label="How widely this applies"
                          size="small"
                          value={selected.scope}
                          onChange={(event) => onSaveRoute?.({ id: selected.id, scope: event.target.value })}
                        >
                          <MenuItem value="per_channel">Per channel</MenuItem>
                          <MenuItem value="global">Global</MenuItem>
                        </TextField>
                        <TextField
                          label="Send matching messages to"
                          size="small"
                          value={selected.route_to || selected.agent || selected.broadcast_group || selected.channel || ""}
                          onChange={(event) => onSaveRoute?.({ id: selected.id, route_to: event.target.value })}
                          helperText="Channel, agent, or broadcast-group target"
                        />
                        <Stack direction="row" spacing={0.75} useFlexGap sx={{
                          flexWrap: "wrap"
                        }}>
                          <Button
                            variant="contained"
                            size="small"
                            onClick={() => onToggleRoute?.(selected.id, !selected.enabled)}
                          >
                            {selected.enabled ? "Disable" : "Enable"}
                          </Button>
                          <Button variant="outlined" size="small" onClick={() => onDeleteRoute?.(selected.id)}>
                            Delete
                          </Button>
                        </Stack>
                      </Stack>
                    ) : (
                      <Stack spacing={1}>
                        <TextField
                          label="Rule name"
                          size="small"
                          value={draftName}
                          onChange={(event) => setDraftName(event.target.value)}
                        />
                        <TextField
                          label="When this applies"
                          size="small"
                          value={draftMatch}
                          onChange={(event) => setDraftMatch(event.target.value)}
                          helperText="Use structured keys that the main app translates into channel, account, or thread matches."
                        />
                        <TextField
                          select
                          label="How widely this applies"
                          size="small"
                          value={draftScope}
                          onChange={(event) => setDraftScope(event.target.value as RouteRule["scope"])}
                        >
                          <MenuItem value="per_channel">Per channel</MenuItem>
                          <MenuItem value="global">Global</MenuItem>
                        </TextField>
                        <TextField
                          select
                          label="Send matching messages to"
                          size="small"
                          value={draftTarget}
                          onChange={(event) => setDraftTarget(event.target.value)}
                        >
                          <MenuItem value="">Select a target</MenuItem>
                          {targets.map((target) => (
                            <MenuItem key={target.id} value={target.id}>
                              {target.label}
                            </MenuItem>
                          ))}
                        </TextField>
                        <Button
                          variant="contained"
                          onClick={() =>
                            onSaveRoute?.({
                              name: draftName,
                              match: draftMatch,
                              scope: draftScope,
                              route_to: draftTarget,
                              enabled: true
                            })
                          }
                        >
                          Create route
                        </Button>
                      </Stack>
                    )}

                    <Divider />

                    <Stack spacing={1}>
                      <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>
                        Broadcast groups
                      </Typography>
                      {broadcastGroups.length === 0 ? (
                        <Typography variant="body2" sx={{
                          color: "text.secondary"
                        }}>
                          No groups yet. Use a group only when one event needs to go to multiple agents or devices.
                        </Typography>
                      ) : (
                        <Stack spacing={0.75}>
                          {broadcastGroups.map((group) => (
                            <Box key={group.id} className="action-row">
                              <Stack spacing={0.25}>
                                <Typography variant="body2" sx={{ fontWeight: 600 }}>
                                  {group.name}
                                </Typography>
                                <Typography variant="caption" sx={{
                                  color: "text.secondary"
                                }}>
                                  {group.members.length} members{group.description ? ` | ${group.description}` : ""}
                                </Typography>
                              </Stack>
                            </Box>
                          ))}
                        </Stack>
                      )}

                      <Stack direction="row" spacing={1}>
                        <TextField
                          label="New group"
                          size="small"
                          fullWidth
                          value={draftGroup}
                          onChange={(event) => setDraftGroup(event.target.value)}
                        />
                        <Button
                          variant="outlined"
                          onClick={() => {
                            const value = draftGroup.trim();
                            if (!value) return;
                            onCreateGroup?.(value);
                            setDraftGroup("");
                          }}
                        >
                          Create
                        </Button>
                      </Stack>
                    </Stack>
                  </Stack>
                )}
              </CardContent>
            </Card>
          </Grid2>
        </Grid2>
      </Stack>
    </Box>
  );
}
