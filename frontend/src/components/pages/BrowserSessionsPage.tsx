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
const CHAT_COMPOSER_PREFILL_STORAGE_KEY = "agentark.chat.composerPrefill";
const CHAT_COMPOSER_PREFILL_EVENT = "agentark.chat.composer-prefill";

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

function browserProfileChatDraft(profile: BrowserProfile): string {
  const lines = [
    `Use the saved browser login profile "${profile.name}" for this browser task.`,
    `Profile id: ${profile.id}. Browser: ${profile.browser}.`,
  ];
  if (profile.target) {
    lines.push(`Saved target: ${profile.target}.`);
  }
  lines.push(
    "",
    "If the site asks for login, CAPTCHA, or 2FA, open a manual handoff for me and continue after I finish.",
    "",
    "Task: ",
  );
  return lines.join("\n");
}

function storeChatComposerPrefill(text: string): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(CHAT_COMPOSER_PREFILL_STORAGE_KEY, text);
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
  const profiles = useMemo(
    () => (profilesQ.data?.profiles || []).map(mapBrowserProfile),
    [profilesQ.data],
  );
  const activeCount = useMemo(
    () => sessions.filter((session) => !isTerminal(session)).length,
    [sessions],
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
    mutationFn: async (payload: { name: string; browser: string; managed: boolean }) =>
      api.createBrowserProfile(payload),
    onSuccess: async () => {
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
    storeChatComposerPrefill(browserProfileChatDraft(profile));
    navigateToChatComposer();
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
            {sessions.map((session) => {
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
                      Created {formatTimestamp(session.created_at)} - Updated{" "}
                      {formatTimestamp(session.updated_at)}
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
          </Stack>
        )}
      </Box>

      {error ? <Alert severity="error">{error}</Alert> : null}
    </WorkspacePageShell>
  );
}
