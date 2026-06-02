import ChatBubbleOutlineRoundedIcon from "@mui/icons-material/ChatBubbleOutlineRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import LoginRoundedIcon from "@mui/icons-material/LoginRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import {
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import { useMemo, useState } from "react";
import { formatUiDateTime } from "../lib/dateFormat";
import { humanizeStatusLabel } from "../lib/displayLabels";

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
  target_kind?: string;
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
  onDeleteProfile?: (profileId: string) => void | Promise<void>;
  onCreateProfile?: (payload: { name: string; browser: string; managed: boolean; target_profile_path?: string }) => void | Promise<void>;
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
  return humanizeStatusLabel(status, "Unknown");
}

function statusRailClassName(status: BrowserProfile["status"]): string {
  const value = String(status || "").toLowerCase();
  if (value === "available" || value === "running" || value === "locked" || value === "manual_login" || value === "error") {
    return `browser-profile-row-rail browser-profile-row-rail--${value}`;
  }
  return "browser-profile-row-rail browser-profile-row-rail--default";
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
  onDeleteProfile,
  onCreateProfile,
  onSetDefaultProfile,
  className
}: BrowserProfilesPanelProps) {
  const selected = profiles.find((profile) => profile.id === selectedProfileId) ?? profiles[0] ?? null;
  const [draftName, setDraftName] = useState("");
  const [draftProfilePath, setDraftProfilePath] = useState("");
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const activeSessionsByProfile = useMemo(() => {
    const byProfile = new Map<string, BrowserSessionSummary[]>();
    for (const session of sessions) {
      if (!session.profile_id) continue;
      const existing = byProfile.get(session.profile_id) ?? [];
      existing.push(session);
      byProfile.set(session.profile_id, existing);
    }
    return byProfile;
  }, [sessions]);

  const resetDraft = () => {
    setDraftName("");
    setDraftProfilePath("");
  };

  const submitProfile = () => {
    const name = draftName.trim();
    if (!name || !onCreateProfile) return;
    const targetProfilePath = draftProfilePath.trim();
    void Promise.resolve(
      onCreateProfile({
        name,
        browser: "chrome",
        managed: false,
        target_profile_path: targetProfilePath || undefined,
      }),
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

        <Box className="browser-profile-panel-shell">
          <Stack
            direction={{ xs: "column", md: "row" }}
            className="browser-profile-panel-head"
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "flex-start", md: "center" },
              gap: 1
            }}
          >
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="h6" sx={{ fontWeight: 650 }}>
                Saved browser identities
              </Typography>
              <Typography variant="body2" sx={{ color: "text.secondary", maxWidth: 860 }}>
                Reusable login context for browser tasks that need cookies, sessions, or manual handoff.
              </Typography>
            </Box>
            <Chip size="small" variant="outlined" label={`${profiles.length} identities`} />
          </Stack>

          {profiles.length === 0 ? (
            <Box className="browser-profile-empty-state">
              <Typography variant="subtitle1" sx={{ fontWeight: 650, mb: 0.5 }}>
                No saved browser logins yet
              </Typography>
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
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
            <Box className="browser-profile-row-list">
              {profiles.map((profile) => {
                const selectedState = profile.id === selected?.id;
                const profileSessions = activeSessionsByProfile.get(profile.id) ?? [];
                const profileRunning = profileSessions.length > 0;
                const effectiveStatus = profileRunning ? "running" : profile.status;
                return (
                  <Box
                    key={profile.id}
                    className={`browser-profile-row${selectedState ? " is-selected" : ""}`}
                    onClick={() => onSelectProfile?.(profile.id)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        onSelectProfile?.(profile.id);
                      }
                    }}
                    role="button"
                    tabIndex={0}
                  >
                    <span className={statusRailClassName(effectiveStatus)} aria-hidden="true" />
                    <Box className="browser-profile-row-main">
                      <Stack
                        direction="row"
                        spacing={0.85}
                        sx={{ alignItems: "center", flexWrap: "wrap", minWidth: 0 }}
                      >
                        <Typography className="browser-profile-row-name" variant="subtitle2" noWrap>
                          {profile.name}
                        </Typography>
                        {profile.default ? <Chip size="small" label="Default" color="info" /> : null}
                        <Chip size="small" color={statusTone(effectiveStatus)} label={statusLabel(effectiveStatus)} />
                        <Chip
                          size="small"
                          variant="outlined"
                          label={profile.managed ? "Sandbox profile" : "Real browser profile"}
                        />
                      </Stack>
                      <Typography className="browser-profile-row-target" variant="caption" noWrap>
                        {profile.browser}
                        {profile.target ? ` / ${profile.target}` : ""}
                      </Typography>
                      <Typography className="browser-profile-row-detail" variant="body2">
                        {profile.detail || "No login context notes yet."}
                      </Typography>
                    </Box>

                    <Box className="browser-profile-row-meta">
                      <span className="browser-profile-row-meta-label">Last launch</span>
                      <span className="browser-profile-row-meta-value">{formatDate(profile.last_launch_at)}</span>
                      <span className="browser-profile-row-meta-label">Sessions</span>
                      <span className="browser-profile-row-meta-value">
                        {profileRunning ? `${profileSessions.length} live` : "Idle"}
                      </span>
                    </Box>

                    <Stack
                      direction="row"
                      spacing={0.65}
                      useFlexGap
                      className="browser-profile-row-actions"
                      onClick={(event) => event.stopPropagation()}
                    >
                      {onLaunchProfile && !profileRunning ? (
                        <Button
                          variant="contained"
                          size="small"
                          startIcon={<PlayArrowRoundedIcon fontSize="small" />}
                          onClick={() => onLaunchProfile(profile.id)}
                        >
                          Launch browser
                        </Button>
                      ) : null}
                      {onOpenManualLogin && profileRunning ? (
                        <Button
                          variant="outlined"
                          size="small"
                          startIcon={<OpenInNewRoundedIcon fontSize="small" />}
                          onClick={() => onOpenManualLogin(profile.id)}
                        >
                          Open browser
                        </Button>
                      ) : null}
                      {onStopProfile && profileRunning ? (
                        <Button
                          variant="outlined"
                          size="small"
                          startIcon={<CloseRoundedIcon fontSize="small" />}
                          onClick={() => onStopProfile(profile.id)}
                        >
                          Close and save
                        </Button>
                      ) : null}
                      {onUseProfileInChat ? (
                        <Button
                          variant="text"
                          size="small"
                          startIcon={<ChatBubbleOutlineRoundedIcon fontSize="small" />}
                          onClick={() => onUseProfileInChat(profile)}
                        >
                          Use in Chat
                        </Button>
                      ) : null}
                      {onSetDefaultProfile ? (
                        <Button
                          variant={profile.default ? "contained" : "outlined"}
                          size="small"
                          disabled={profile.default}
                          onClick={() => onSetDefaultProfile(profile.id)}
                        >
                          {profile.default ? "Default" : "Set default"}
                        </Button>
                      ) : null}
                      {onDeleteProfile ? (
                        <Button
                          variant="text"
                          color="error"
                          size="small"
                          startIcon={<DeleteOutlineRoundedIcon fontSize="small" />}
                          onClick={() => onDeleteProfile(profile.id)}
                        >
                          Delete
                        </Button>
                      ) : null}
                    </Stack>
                  </Box>
                );
              })}
            </Box>
          )}
        </Box>
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
              label="Chrome profile folder"
              placeholder="Leave blank for a dedicated AgentArk folder"
              helperText="Optional. Use a separate folder unless you know the selected browser profile is closed."
              size="small"
              value={draftProfilePath}
              onChange={(event) => setDraftProfilePath(event.target.value)}
            />
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



