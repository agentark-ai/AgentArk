import ChatBubbleOutlineRoundedIcon from "@mui/icons-material/ChatBubbleOutlineRounded";
import LoginRoundedIcon from "@mui/icons-material/LoginRounded";
import PhonelinkRoundedIcon from "@mui/icons-material/PhonelinkRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  MenuItem,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMemo, useState } from "react";
import { formatUiDateTime } from "../lib/dateFormat";

export type BrowserSessionSummary = {
  id: string;
  status: "active" | "waiting" | "completed" | "failed" | string;
  title?: string;
  url?: string;
  profile_id?: string | null;
  profile_name?: string | null;
  updated_at?: string;
};

export type BrowserProfile = {
  id: string;
  name: string;
  browser: string;
  status: "available" | "running" | "locked" | "error" | "manual_login" | string;
  default?: boolean;
  target?: string;
  managed?: boolean;
  session_count?: number;
  last_launch_at?: string;
  detail?: string;
};

export type BrowserProfilesPanelProps = {
  profiles: BrowserProfile[];
  sessions?: BrowserSessionSummary[];
  selectedProfileId?: string | null;
  onSelectProfile?: (profileId: string) => void;
  onLaunchProfile?: (profileId: string) => void | Promise<void>;
  onStopProfile?: (profileId: string) => void | Promise<void>;
  onOpenManualLogin?: (profileId: string) => void | Promise<void>;
  onUseProfileInChat?: (profile: BrowserProfile) => void | Promise<void>;
  onCreateProfile?: (payload: { name: string; browser: string; managed: boolean }) => void | Promise<void>;
  onSetDefaultProfile?: (profileId: string) => void | Promise<void>;
  className?: string;
};

function statusTone(status: BrowserProfile["status"]): "success" | "warning" | "error" | "info" | "default" {
  const value = String(status || "").toLowerCase();
  if (value === "available") return "success";
  if (value === "running") return "info";
  if (value === "manual_login" || value === "locked") return "warning";
  if (value === "error") return "error";
  return "default";
}

function statusLabel(status: BrowserProfile["status"]): string {
  const value = String(status || "").toLowerCase();
  if (value === "available") return "Available";
  if (value === "running") return "Running";
  if (value === "locked") return "Locked";
  if (value === "manual_login") return "Manual login";
  if (value === "error") return "Error";
  return status || "Unknown";
}

function formatDate(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "Never" });
}

export function BrowserProfilesPanel({
  profiles,
  sessions = [],
  selectedProfileId,
  onSelectProfile,
  onLaunchProfile,
  onStopProfile,
  onOpenManualLogin,
  onUseProfileInChat,
  onCreateProfile,
  onSetDefaultProfile,
  className
}: BrowserProfilesPanelProps) {
  const selected = profiles.find((profile) => profile.id === selectedProfileId) ?? profiles[0] ?? null;
  const [draftName, setDraftName] = useState("");
  const [draftBrowser, setDraftBrowser] = useState("chrome");
  const [draftManaged, setDraftManaged] = useState(true);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const stats = useMemo(() => {
    const running = profiles.filter((profile) => String(profile.status).toLowerCase() === "running").length;
    const locked = profiles.filter((profile) => String(profile.status).toLowerCase() === "locked").length;
    const managed = profiles.filter((profile) => profile.managed).length;
    return { running, locked, managed };
  }, [profiles]);

  const resetDraft = () => {
    setDraftName("");
    setDraftBrowser("chrome");
    setDraftManaged(true);
  };

  const submitProfile = () => {
    const name = draftName.trim();
    if (!name || !onCreateProfile) return;
    void Promise.resolve(
      onCreateProfile({ name, browser: draftBrowser, managed: draftManaged }),
    );
    resetDraft();
    setCreateDialogOpen(false);
  };

  return (
    <Box className={className}>
      <Stack spacing={1.25}>
        <Stack
          direction={{ xs: "column", sm: "row" }}
          spacing={1}
          sx={{
            justifyContent: "space-between",
            alignItems: { xs: "flex-start", sm: "center" }
          }}
        >
          <Box>
            <Typography variant="overline" className="workspace-shell-kicker">
              Browser
            </Typography>
            <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
              Saved browser logins and handoff state
            </Typography>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
                maxWidth: 840
              }}>
              Save separate browser identities for accounts, customer sites, and login-required automation.
            </Typography>
          </Box>
          {onCreateProfile ? (
            <Button
              variant="contained"
              size="small"
              startIcon={<LoginRoundedIcon fontSize="small" />}
              onClick={() => setCreateDialogOpen(true)}
            >
              Add login profile
            </Button>
          ) : null}
        </Stack>

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
                      Running
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.running}
                    </Typography>
                  </Box>
                  <PlayArrowRoundedIcon fontSize="small" />
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
                      Locked
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.locked}
                    </Typography>
                  </Box>
                  <LoginRoundedIcon fontSize="small" />
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
                      Reusable
                    </Typography>
                    <Typography variant="h5" sx={{ fontWeight: 700 }}>
                      {stats.managed}
                    </Typography>
                  </Box>
                  <PhonelinkRoundedIcon fontSize="small" />
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
                        Saved browser identities
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Reusable login context for browser tasks that need cookies, sessions, or manual handoff.
                      </Typography>
                    </Box>
                    <Chip size="small" variant="outlined" label={`${profiles.length} identities`} />
                  </Stack>
                  <Divider />

                  {profiles.length === 0 ? (
                    <Box sx={{ py: 4 }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                        No saved browser logins yet
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Add one when a site or account needs its own cookies, login state, or manual 2FA handoff.
                      </Typography>
                      {onCreateProfile ? (
                        <Button
                          size="small"
                          variant="outlined"
                          sx={{ mt: 1.25 }}
                          onClick={() => setCreateDialogOpen(true)}
                        >
                          Add login profile
                        </Button>
                      ) : null}
                    </Box>
                  ) : (
                    <Stack spacing={0.85}>
                      {profiles.map((profile) => {
                        const selectedState = profile.id === selected?.id;
                        return (
                          <Box
                            key={profile.id}
                            className="action-row"
                            onClick={() => onSelectProfile?.(profile.id)}
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
                                    {profile.name}
                                  </Typography>
                                  <Typography variant="caption" noWrap sx={{
                                    color: "text.secondary"
                                  }}>
                                    {profile.browser} {profile.target ? `| ${profile.target}` : ""}
                                  </Typography>
                                </Box>
                                <Stack direction="row" spacing={0.75} useFlexGap sx={{
                                  flexWrap: "wrap"
                                }}>
                                  {profile.default ? <Chip size="small" label="Default" color="info" /> : null}
                                  <Chip size="small" color={statusTone(profile.status)} label={statusLabel(profile.status)} />
                                </Stack>
                              </Stack>
                              <Typography variant="body2" sx={{
                                color: "text.secondary"
                              }}>
                                {profile.detail || "No login context notes yet."}
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
                        Login profile details
                      </Typography>
                      <Typography variant="body2" sx={{
                        color: "text.secondary"
                      }}>
                        Use this saved identity from Chat when a browser task needs cookies, login, CAPTCHA, or 2FA.
                      </Typography>
                    </Box>

                    <Stack direction="row" spacing={0.75} useFlexGap sx={{
                      flexWrap: "wrap"
                    }}>
                      <Chip size="small" variant="outlined" label={selected.browser} />
                      <Chip size="small" color={statusTone(selected.status)} label={statusLabel(selected.status)} />
                      {selected.managed ? <Chip size="small" variant="outlined" label="Reusable login" /> : null}
                    </Stack>

                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Last launch {formatDate(selected.last_launch_at)}.
                    </Typography>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Active sessions: {selected.session_count || 0}
                    </Typography>

                    <Box
                      sx={{
                        border: "1px solid",
                        borderColor: "divider",
                        borderRadius: 1,
                        p: 1,
                        bgcolor: "var(--ui-rgba-47-212-255-045)",
                      }}
                    >
                      <Stack spacing={0.85}>
                        <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                          Next step
                        </Typography>
                        <Typography variant="body2" sx={{ color: "text.secondary" }}>
                          Start from Chat, describe the site and job, and AgentArk can reuse this saved login profile or pause for a handoff when the site asks for you.
                        </Typography>
                        {onUseProfileInChat ? (
                          <Box>
                            <Button
                              variant="contained"
                              size="small"
                              startIcon={<ChatBubbleOutlineRoundedIcon fontSize="small" />}
                              onClick={() => onUseProfileInChat(selected)}
                            >
                              Start browser task
                            </Button>
                          </Box>
                        ) : null}
                      </Stack>
                    </Box>

                    {onLaunchProfile || onStopProfile || onOpenManualLogin ? (
                      <Stack direction="row" spacing={0.75} useFlexGap sx={{
                        flexWrap: "wrap"
                      }}>
                        {onLaunchProfile ? (
                          <Button variant="contained" size="small" onClick={() => onLaunchProfile(selected.id)}>
                            Launch
                          </Button>
                        ) : null}
                        {onStopProfile ? (
                          <Button variant="outlined" size="small" onClick={() => onStopProfile(selected.id)}>
                            Stop
                          </Button>
                        ) : null}
                        {onOpenManualLogin ? (
                          <Button variant="text" size="small" onClick={() => onOpenManualLogin(selected.id)}>
                            Open login handoff
                          </Button>
                        ) : null}
                      </Stack>
                    ) : null}

                    {onSetDefaultProfile ? (
                      <Button variant="outlined" size="small" onClick={() => onSetDefaultProfile(selected.id)}>
                        Use by default
                      </Button>
                    ) : null}
                  </Stack>
                ) : (
                  <Box sx={{ py: 4 }}>
                    <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                      No login profile selected
                    </Typography>
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      Select a saved login profile to manage launch and handoff flow.
                    </Typography>
                  </Box>
                )}
              </CardContent>
            </Card>

            <Card className="workspace-side-card" sx={{ mt: 1.25 }}>
              <CardContent sx={{ p: 1.5 }}>
                <Stack spacing={1.1}>
                  <Typography variant="h6" sx={{ fontWeight: 650 }}>
                    Active sessions
                  </Typography>
                  {sessions.length === 0 ? (
                    <Typography variant="body2" sx={{
                      color: "text.secondary"
                    }}>
                      No browser sessions are active right now.
                    </Typography>
                  ) : (
                    <Stack spacing={0.75}>
                      {sessions.map((session) => (
                        <Box key={session.id} className="action-row">
                          <Stack spacing={0.25}>
                            <Typography variant="body2" sx={{ fontWeight: 600 }}>
                              {session.title || session.id}
                            </Typography>
                            <Typography variant="caption" sx={{
                              color: "text.secondary"
                            }}>
                              {session.status}
                              {session.profile_name ? ` | ${session.profile_name}` : ""}
                              {session.url ? ` | ${session.url}` : ""}
                            </Typography>
                          </Stack>
                        </Box>
                      ))}
                    </Stack>
                  )}
                </Stack>
              </CardContent>
            </Card>
          </Grid2>
        </Grid2>
      </Stack>
      <Dialog
        open={createDialogOpen}
        onClose={() => {
          setCreateDialogOpen(false);
          resetDraft();
        }}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>Add login profile</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.2} sx={{ pt: 0.25 }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Use a login profile to keep cookies, saved sessions, and browser state separate for repeat browser tasks.
            </Typography>
            <TextField
              autoFocus
              label="Profile name"
              placeholder="Work Gmail, Client Shopify, Research sandbox"
              helperText="Name the account, site, or purpose this browser identity is for."
              size="small"
              value={draftName}
              onChange={(event) => setDraftName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") submitProfile();
              }}
            />
            <TextField
              select
              label="Browser"
              size="small"
              value={draftBrowser}
              onChange={(event) => setDraftBrowser(event.target.value)}
            >
              <MenuItem value="chrome">Chrome</MenuItem>
              <MenuItem value="chromium">Chromium</MenuItem>
              <MenuItem value="firefox">Firefox</MenuItem>
              <MenuItem value="edge">Edge</MenuItem>
            </TextField>
            <TextField
              select
              label="Keep session managed"
              helperText="Managed login profiles are saved for future agent runs; unmanaged sessions are for throwaway browser work."
              size="small"
              value={draftManaged ? "yes" : "no"}
              onChange={(event) => setDraftManaged(event.target.value === "yes")}
            >
              <MenuItem value="yes">Yes</MenuItem>
              <MenuItem value="no">No</MenuItem>
            </TextField>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setCreateDialogOpen(false);
              resetDraft();
            }}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={!draftName.trim() || !onCreateProfile}
            onClick={submitProfile}
          >
            Add login profile
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}



