import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import { formatUiDateTimeMeta } from "../../lib/dateFormat";
import type { BrowserProfileRecord, BrowserSessionSummary } from "../../types";
import { BrowserProfilesPanel, type BrowserProfile } from "../BrowserProfilesPanel";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { errMessage } from "./pageHelpers";

const REFRESH_MS = 8000;
const SESSION_PAGE_SIZE = 8;
const CHAT_COMPOSER_PREFILL_STORAGE_KEY = "agentark.chat.composerPrefill";
const CHAT_COMPOSER_PREFILL_EVENT = "agentark.chat.composer-prefill";

type BrowserProfileChatPrefill = {
  text: string;
  browser_profile_context: {
    profile_id: string;
    profile_name: string;
    browser: string;
    target?: string;
    target_kind?: string;
    manual_handoff: boolean;
  };
};

type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

function browserSessionHandoffUrl(sessionId: string): string {
  return `/ui/browser-handoff/${encodeURIComponent(sessionId)}`;
}

function browserProfileChatPrefill(profile: BrowserProfile): BrowserProfileChatPrefill {
  return {
    text: `Browser profile: ${profile.name}\n\nTask: `,
    browser_profile_context: {
      profile_id: profile.id,
      profile_name: profile.name,
      browser: profile.browser,
      ...(profile.target ? { target: profile.target } : {}),
      ...(profile.target_kind ? { target_kind: profile.target_kind } : {}),
      manual_handoff: true,
    },
  };
}

function storeChatComposerPrefill(prefill: BrowserProfileChatPrefill): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(
      CHAT_COMPOSER_PREFILL_STORAGE_KEY,
      JSON.stringify(prefill),
    );
  } catch {
    // Ignore storage failures; navigation still works.
  }
}

function navigateToChatComposer(): void {
  if (typeof window === "undefined") return;
  const nextUrl = "/ui/chat";
  const current = `${window.location.pathname}${window.location.search}`;
  if (current !== nextUrl) {
    window.history.pushState(null, "", nextUrl);
  }
  window.dispatchEvent(new PopStateEvent("popstate"));
  window.dispatchEvent(new Event(CHAT_COMPOSER_PREFILL_EVENT));
}

function formatTimestamp(value?: string | null): string {
  return formatUiDateTimeMeta(value || "", { fallback: "-" }).label;
}

function statusLabel(status: string): string {
  const normalized = status.trim().toLowerCase();
  if (!normalized) return "-";
  if (normalized === "in_progress") return "Running";
  return normalized
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function statusColor(
  status: string,
): "success" | "warning" | "error" | "default" | "info" {
  const normalized = status.toLowerCase();
  if (normalized.includes("running") || normalized.includes("progress")) return "info";
  if (normalized.includes("paused") || normalized.includes("waiting")) return "warning";
  if (
    normalized.includes("failed") ||
    normalized.includes("interrupted") ||
    normalized.includes("cancelled") ||
    normalized.includes("canceled")
  ) {
    return "error";
  }
  if (normalized.includes("completed")) return "success";
  return "default";
}

function dotColor(status: string): string {
  const color = statusColor(status);
  if (color === "info") return "var(--ui-rgba-57-208-255-850)";
  if (color === "success") return "var(--ui-rgba-74-210-157-850)";
  if (color === "warning") return "var(--ui-rgba-255-191-130-850)";
  if (color === "error") return "var(--ui-rgba-255-100-100-850)";
  return "var(--ui-rgba-180-200-220-500)";
}

function isTerminal(session: BrowserSessionSummary): boolean {
  const status = session.status.toLowerCase();
  return (
    status.includes("completed") ||
    status.includes("failed") ||
    status.includes("interrupted") ||
    status.includes("cancelled") ||
    status.includes("canceled") ||
    status.includes("stopped")
  );
}

function sessionDetailLine(session: BrowserSessionSummary): string {
  return (
    session.summary ||
    session.question ||
    session.reason ||
    session.page_title ||
    session.page_url ||
    "Live browser session"
  );
}

function profileBrowserName(profile: BrowserProfileRecord): string {
  const metadata = profile.metadata || {};
  const browser =
    typeof metadata.browser === "string" && metadata.browser.trim()
      ? metadata.browser.trim()
      : "";
  return browser || profile.target_kind || "browser";
}

function profileStatus(profile: BrowserProfileRecord): BrowserProfile["status"] {
  if (profile.lock) return "locked";
  if (profile.last_error) return "error";
  const loginState = profile.login_state.trim().toLowerCase();
  if (loginState === "logged_in") return "available";
  if (loginState === "needs_mfa" || loginState === "expired") return "manual_login";
  if (profile.enabled === false) return "locked";
  return "available";
}

function profileTarget(profile: BrowserProfileRecord): string {
  return (
    profile.target_endpoint ||
    profile.target_profile_path ||
    profile.target_workspace ||
    profile.target_kind ||
    ""
  );
}

function profileDetail(profile: BrowserProfileRecord): string {
  if (profile.last_error) return profile.last_error;
  if (profile.login_note) return profile.login_note;
  if (profile.lock) {
    return profile.lock.reason || `Locked by ${profile.lock.owner}`;
  }
  if (profile.target_kind === "host") return profile.description || "Ready for real browser profile work.";
  return profile.description || "Ready for isolated browser work.";
}

function mapBrowserProfile(profile: BrowserProfileRecord): BrowserProfile {
  const metadata = profile.metadata || {};
  const managed =
    typeof metadata.managed === "boolean"
      ? metadata.managed
      : profile.target_kind !== "host";
  return {
    id: profile.id,
    name: profile.name,
    browser: profileBrowserName(profile),
    status: profileStatus(profile),
    target: profileTarget(profile),
    target_kind: profile.target_kind,
    managed,
    session_count: profile.recent_sessions?.length || 0,
    last_launch_at: profile.last_used_at || profile.login_checked_at || undefined,
    detail: profileDetail(profile),
  };
}

function RowOpsMenu({
  actions,
  ariaLabel = "Row actions",
}: {
  actions: RowMenuAction[];
  ariaLabel?: string;
}) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton
        size="small"
        aria-label={ariaLabel}
        onClick={(event) => {
          event.stopPropagation();
          setAnchorEl(event.currentTarget);
        }}
      >
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu
        anchorEl={anchorEl}
        open={open}
        onClose={closeMenu}
        onClick={(event) => event.stopPropagation()}
      >
        {actions.map((action, index) => (
          <MenuItem
            key={`${action.label}-${index}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={(event) => {
              event.stopPropagation();
              closeMenu();
              if (action.disabled) return;
              void action.onClick();
            }}
            sx={
              action.tone === "error"
                ? { color: "error.main" }
                : action.tone === "warning"
                  ? { color: "warning.main" }
                  : undefined
            }
          >
            {action.label}
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

export default function BrowserSessionsPage({
  autoRefresh,
}: {
  autoRefresh: boolean;
}) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(null);
  const [sessionPage, setSessionPage] = useState(0);

  const sessionsQ = useQuery({
    queryKey: ["browser-sessions"],
    queryFn: api.getBrowserSessions,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const profilesQ = useQuery({
    queryKey: ["browser-profiles"],
    queryFn: api.getBrowserProfiles,
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const sessions = useMemo(
    () => sessionsQ.data?.sessions || [],
    [sessionsQ.data],
  );
  const profiles = useMemo(() => {
    const activeByProfile = new Map<string, number>();
    for (const session of sessions) {
      const profileId = String(session.profile_id || "").trim();
      if (!profileId || isTerminal(session)) continue;
      activeByProfile.set(profileId, (activeByProfile.get(profileId) || 0) + 1);
    }
    return (profilesQ.data?.profiles || []).map((record) => {
      const mapped = mapBrowserProfile(record);
      const activeSessions = activeByProfile.get(record.id) || 0;
      return {
        ...mapped,
        status: activeSessions > 0 ? "running" : mapped.status,
        session_count: activeSessions,
      };
    });
  }, [profilesQ.data, sessions]);
  const activeCount = useMemo(
    () => sessions.filter((session) => !isTerminal(session)).length,
    [sessions],
  );
  const totalSessionPages = Math.max(1, Math.ceil(sessions.length / SESSION_PAGE_SIZE));
  const safeSessionPage = Math.min(sessionPage, totalSessionPages - 1);
  const visibleSessions = sessions.slice(
    safeSessionPage * SESSION_PAGE_SIZE,
    safeSessionPage * SESSION_PAGE_SIZE + SESSION_PAGE_SIZE,
  );

  const invalidate = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["browser-sessions"] }),
      queryClient.invalidateQueries({ queryKey: ["browser-profiles"] }),
      queryClient.invalidateQueries({ queryKey: ["autonomy-browser-sessions"] }),
    ]);
  };

  const actionMutation = useMutation({
    mutationFn: async ({
      kind,
      sessionId,
    }: {
      kind: "stop" | "delete";
      sessionId: string;
    }) => {
      if (kind === "stop") return api.stopBrowserSession(sessionId);
      return api.deleteBrowserSession(sessionId);
    },
    onSuccess: invalidate,
    onError: (err) => setError(errMessage(err)),
  });
  const profileMutation = useMutation({
    mutationFn: async (payload: { name: string; browser: string; managed: boolean; target_profile_path?: string }) =>
      api.createBrowserProfile(payload),
    onSuccess: async () => {
      await invalidate();
    },
    onError: (err) => setError(errMessage(err)),
  });
  const profileLaunchMutation = useMutation({
    mutationFn: async (profileId: string) => api.launchBrowserProfile(profileId),
    onSuccess: async (result) => {
      await invalidate();
      const session = result.session as BrowserSessionSummary | undefined;
      if (session?.id) {
        window.open(browserSessionHandoffUrl(session.id), "_blank", "noopener,noreferrer");
      }
    },
    onError: (err) => setError(errMessage(err)),
  });
  const profileCloseMutation = useMutation({
    mutationFn: async (profileId: string) => api.closeBrowserProfile(profileId),
    onSuccess: invalidate,
    onError: (err) => setError(errMessage(err)),
  });
  const profileDeleteMutation = useMutation({
    mutationFn: async (profileId: string) => api.deleteBrowserProfile(profileId),
    onSuccess: async (_result, profileId) => {
      if (selectedProfileId === profileId) setSelectedProfileId(null);
      await invalidate();
    },
    onError: (err) => setError(errMessage(err)),
  });

  const rowActions = (session: BrowserSessionSummary): RowMenuAction[] => {
    const actions: RowMenuAction[] = [];
    if (!isTerminal(session)) {
      actions.push({
        label: "Stop",
        tone: "warning",
        disabled: actionMutation.isPending,
        onClick: () =>
          actionMutation.mutate({ kind: "stop", sessionId: session.id }),
      });
    }
    actions.push({
      label: "Delete",
      tone: "error",
      divider: actions.length > 0,
      disabled: actionMutation.isPending,
      onClick: () => {
        const confirmed = window.confirm(
          "Delete this browser session? This closes the live browser and removes the saved session record.",
        );
        if (!confirmed) return;
        actionMutation.mutate({ kind: "delete", sessionId: session.id });
      },
    });
    return actions;
  };

  const useProfileInChat = (profile: BrowserProfile) => {
    storeChatComposerPrefill(browserProfileChatPrefill(profile));
    navigateToChatComposer();
  };
  const openProfileLive = (profileId: string) => {
    const liveSession = sessions.find(
      (session) => session.profile_id === profileId && !isTerminal(session),
    );
    if (liveSession) {
      window.open(browserSessionHandoffUrl(liveSession.id), "_blank", "noopener,noreferrer");
      return;
    }
    profileLaunchMutation.mutate(profileId);
  };
  const deleteProfile = (profileId: string) => {
    const profile = profiles.find((item) => item.id === profileId);
    const confirmed = window.confirm(
      `Delete browser profile "${profile?.name || profileId}"? This closes live browsers for it and removes saved browser state.`,
    );
    if (!confirmed) return;
    profileDeleteMutation.mutate(profileId);
  };

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Operations"
        title="Browser"
        description="Saved browser logins, live handoffs, diagnostics, and background browser runs."
      />

      <Box className="list-shell stat-strip">
        {[
          { label: "Sessions", value: sessions.length },
          { label: "Active", value: activeCount },
          { label: "Finished", value: sessions.length - activeCount },
          { label: "Login profiles", value: profiles.length },
        ].map((item) => (
          <div key={item.label} className="stat-strip-item">
            <span className="stat-strip-label">{item.label}</span>
            <span className="stat-strip-value">{item.value}</span>
          </div>
        ))}
      </Box>

      <Box className="list-shell">
        {profilesQ.error ? <Alert severity="error">{errMessage(profilesQ.error)}</Alert> : null}
        {profilesQ.isLoading ? (
          <Box sx={{ py: 4, textAlign: "center" }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Loading saved browser logins...
            </Typography>
          </Box>
        ) : (
          <BrowserProfilesPanel
            profiles={profiles}
            sessions={sessions
              .filter((session) => !isTerminal(session))
              .map((session) => ({
                id: session.id,
                status: session.status,
                title: session.task_description || session.page_title || session.id,
                url: session.page_url || undefined,
                profile_id: session.profile_id || undefined,
                profile_name: session.profile_name || undefined,
                updated_at: session.updated_at,
              }))}
            selectedProfileId={selectedProfileId}
            onSelectProfile={setSelectedProfileId}
            onUseProfileInChat={useProfileInChat}
            onLaunchProfile={(profileId) => profileLaunchMutation.mutate(profileId)}
            onStopProfile={(profileId) => profileCloseMutation.mutate(profileId)}
            onOpenManualLogin={openProfileLive}
            onDeleteProfile={deleteProfile}
            onCreateProfile={(payload) => profileMutation.mutate(payload)}
          />
        )}
      </Box>

      <Box className="list-shell">
        <Stack
          direction="row"
          sx={{ justifyContent: "space-between", alignItems: "center", mb: 1 }}
        >
          <Typography variant="h6">Browser Sessions</Typography>
          <Button size="small" onClick={() => void invalidate()}>
            Refresh
          </Button>
        </Stack>

        {sessionsQ.isLoading ? (
          <Box sx={{ py: 5, textAlign: "center" }}>
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              Loading browser sessions...
            </Typography>
          </Box>
        ) : sessionsQ.error ? (
          <Alert severity="error">{errMessage(sessionsQ.error)}</Alert>
        ) : sessions.length === 0 ? (
          <Box sx={{ py: 8, textAlign: "center" }}>
            <Typography variant="h6" sx={{ color: "text.secondary" }}>
              No browser sessions
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.5 }}>
              Browser work will appear here when a run needs a live handoff.
            </Typography>
          </Box>
        ) : (
          <Stack spacing={0.25}>
            {visibleSessions.map((session) => {
              const detailLine = sessionDetailLine(session);
              return (
                <Box
                  key={session.id}
                  sx={{
                    py: 1.15,
                    borderBottom: "1px solid",
                    borderColor: "divider",
                    display: "flex",
                    gap: 1,
                    alignItems: "flex-start",
                  }}
                >
                  <Box
                    sx={{
                      width: 7,
                      height: 7,
                      borderRadius: "50%",
                      flexShrink: 0,
                      mt: 0.85,
                      background: dotColor(session.status),
                    }}
                  />
                  <Box sx={{ minWidth: 0, flex: 1 }}>
                    <Stack
                      direction="row"
                      spacing={0.75}
                      sx={{ alignItems: "center", flexWrap: "wrap", minWidth: 0 }}
                    >
                      <Typography
                        variant="body2"
                        noWrap
                        sx={{ fontWeight: 700, minWidth: 160, flex: 1 }}
                        title={session.task_description}
                      >
                        {session.task_description || "Browser session"}
                      </Typography>
                      <Chip size="small" variant="outlined" label="Browser" />
                      {session.profile_name ? (
                        <Chip size="small" variant="outlined" label={session.profile_name} />
                      ) : null}
                      <Chip
                        size="small"
                        color={statusColor(session.status)}
                        variant="outlined"
                        label={statusLabel(session.status)}
                      />
                      <Button
                        size="small"
                        variant="outlined"
                        onClick={() =>
                          window.open(
                            browserSessionHandoffUrl(session.id),
                            "_blank",
                            "noopener,noreferrer",
                          )
                        }
                      >
                        Open
                      </Button>
                    </Stack>
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", display: "block", mt: 0.25 }}
                      noWrap
                      title={detailLine}
                    >
                      {detailLine}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{ color: "text.secondary", display: "block", mt: 0.2 }}
                    >
                      Updated {formatTimestamp(session.updated_at)}
                    </Typography>
                  </Box>
                  <Box sx={{ flexShrink: 0 }}>
                    <RowOpsMenu
                      actions={rowActions(session)}
                      ariaLabel="Browser session actions"
                    />
                  </Box>
                </Box>
              );
            })}
            {sessions.length > SESSION_PAGE_SIZE ? (
              <Stack
                direction="row"
                spacing={1}
                sx={{ alignItems: "center", justifyContent: "flex-end", pt: 1 }}
              >
                <Typography variant="caption" sx={{ color: "text.secondary", mr: "auto" }}>
                  Showing {safeSessionPage * SESSION_PAGE_SIZE + 1}-
                  {Math.min((safeSessionPage + 1) * SESSION_PAGE_SIZE, sessions.length)} of{" "}
                  {sessions.length}
                </Typography>
                <Button
                  size="small"
                  disabled={safeSessionPage === 0}
                  onClick={() => setSessionPage((page) => Math.max(0, page - 1))}
                >
                  Previous
                </Button>
                <Button
                  size="small"
                  disabled={safeSessionPage >= totalSessionPages - 1}
                  onClick={() =>
                    setSessionPage((page) => Math.min(totalSessionPages - 1, page + 1))
                  }
                >
                  Next
                </Button>
              </Stack>
            ) : null}
          </Stack>
        )}
      </Box>

      {error ? <Alert severity="error">{error}</Alert> : null}
    </WorkspacePageShell>
  );
}
